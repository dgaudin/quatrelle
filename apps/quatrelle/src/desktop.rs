//! What the window asks of the desktop (`Services`): the clock and the system's style and fonts,
//! files and web addresses opened with the system's apps, the clipboard, files dragged out of
//! the window, notifications, and video frames.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use quatrelle_engine::store::location;
use quatrelle_ui_egui::{BrowserProfile, ReminderAction, Services, VideoFrame};

#[cfg(windows)]
use crate::toast;
#[cfg(all(unix, not(target_os = "macos")))]
use crate::xdnd;
use crate::{display, self_update, video};

pub(crate) mod appearance;
pub(crate) mod browsers;
pub(crate) mod keychain;
pub(crate) mod profiles;
pub(crate) mod tray;

#[derive(Default)]
pub(crate) struct Desktop {
    /// Where video frames come from (looked up once).
    pub(crate) frame_source: Option<Option<video::Source>>,
    /// Video frames being made or made, by file.
    pub(crate) frames: std::collections::HashMap<PathBuf, Arc<std::sync::Mutex<Option<VideoFrame>>>>,
    /// The data files' folder in use, and the app folder whose data files can be moved (`None`
    /// when the place was chosen at start: `--data`, `--sample`, `QUATRELLE_DATA`).
    pub(crate) data: Option<PathBuf>,
    pub(crate) base: Option<PathBuf>,
    /// How a move carried out at this start went (told once).
    pub(crate) moved: Option<Result<PathBuf, String>>,
    /// The desktop's light or dark style and accent colour, kept up to date.
    pub(crate) style: appearance::Watch,
    /// Portable mode (`portable.rs`): Quatrelle's name isn't written to Windows' registry.
    #[cfg(windows)]
    pub(crate) portable: bool,
    /// Test builds' updater (`self_update.rs`), when this one has it.
    pub(crate) updater: Option<self_update::Updater>,
    /// What was pressed on reminder notifications (the reminder's key, and what), left by the
    /// threads that wait on them for the window to take; and the window's waker, so it draws
    /// (and answers) as soon as one comes.
    pub(crate) reminder_answers: Arc<std::sync::Mutex<Vec<(String, ReminderAction)>>>,
    #[cfg(all(unix, not(target_os = "macos")))]
    pub(crate) waker: Option<Arc<dyn Fn() + Send + Sync>>,
    /// The reminder notifications up, by reminder: taken back when the reminder is answered
    /// elsewhere or its time passes, and as Quatrelle closes (nothing would answer them then).
    #[cfg(all(unix, not(target_os = "macos")))]
    pub(crate) reminder_notes: ReminderNotes,
    /// The folder of the last file picked or saved: the next file dialog opens there (attaching
    /// several files from one folder, saving attachments beside each other), as figcraft's do.
    pub(crate) last_dir: Option<PathBuf>,
    /// Quatrelle's icon in the notification area, the envelope on it while new mail is unread.
    pub(crate) tray: tray::Tray,
}

impl Desktop {
    /// A file dialog titled `title`, open at the folder last used when it's still there.
    fn dialog(&self, title: &str) -> rfd::FileDialog {
        let d = rfd::FileDialog::new().set_title(title);
        match self.last_dir.as_ref().filter(|d| d.is_dir()) {
            Some(dir) => d.set_directory(dir),
            None => d,
        }
    }

    /// Notes where a dialog's answer was (a file's folder, or the folder picked); passes it on.
    fn used<T: AsRef<Path>>(&mut self, picked: Option<T>, is_folder: bool) -> Option<T> {
        if let Some(p) = &picked {
            let p = p.as_ref();
            self.last_dir = if is_folder { Some(p.to_path_buf()) } else { p.parent().map(Path::to_path_buf) }.or(self.last_dir.take());
        }
        picked
    }
}

/// The reminder notifications up, by reminder.
#[cfg(all(unix, not(target_os = "macos")))]
#[derive(Default)]
pub(crate) struct ReminderNotes(Arc<std::sync::Mutex<std::collections::HashMap<String, Arc<notify_rust::NotificationHandle>>>>);

#[cfg(all(unix, not(target_os = "macos")))]
impl Drop for ReminderNotes {
    /// The reminder notifications go with the window: left up, their buttons would answer
    /// nothing (and the next start tells of what's still due again). A second at most.
    fn drop(&mut self) {
        let up: Vec<_> = std::mem::take(&mut *self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner)).into_values().collect();
        if up.is_empty() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new().name("reminder-close".into()).spawn(move || {
            for note in up {
                async_io::block_on(note.close_async());
            }
            let _ = tx.send(());
        });
        if spawned.is_ok() {
            let _ = rx.recv_timeout(std::time::Duration::from_secs(1));
        }
    }
}

impl Services for Desktop {
    fn system_style(&self) -> quatrelle_ui_egui::theme::SystemStyle {
        self.style.get()
    }
    fn clock(&self) -> (i64, i32) {
        let now = chrono::Local::now();
        (now.timestamp(), now.offset().local_minus_utc())
    }
    fn zone(&self) -> Option<String> {
        iana_time_zone::get_timezone().ok()
    }
    fn pick_files(&mut self) -> Vec<PathBuf> {
        let files = self.dialog("Attach files").pick_files().unwrap_or_default();
        self.used(files.first(), false);
        files
    }
    fn save_path(&mut self, suggested: &str) -> Option<PathBuf> {
        let mut d = self.dialog("Save as").set_file_name(suggested);
        // Save as type: the suggested name's kind first, so the system keeps its extension.
        if let Some((_, ext)) = suggested.rsplit_once('.').filter(|(_, e)| !e.is_empty() && e.len() <= 8) {
            let kind = match ext.to_ascii_lowercase().as_str() {
                "pst" => "Outlook data file".to_string(),
                "eml" => "Email message".to_string(),
                "pdf" => "PDF document".to_string(),
                "ics" => "Calendar".to_string(),
                "vcf" => "Contact card".to_string(),
                "txt" => "Text".to_string(),
                "json" => "Quatrelle rules".to_string(),
                "html" | "htm" => "Web page".to_string(),
                e => format!("{} file", e.to_ascii_uppercase()),
            };
            d = d.add_filter(kind, &[ext]).add_filter("All files", &["*"]);
        }
        let saved = d.save_file();
        self.used(saved, false)
    }
    fn open_path(&mut self, path: &Path) -> Result<(), String> {
        shell_open(path.as_os_str())
    }
    fn update_state(&mut self) -> quatrelle_ui_egui::UpdateState {
        self.updater.as_ref().map(self_update::Updater::state).unwrap_or_default()
    }
    fn restart_to_update(&mut self) -> bool {
        self.updater.as_ref().is_some_and(self_update::Updater::restart_after_close)
    }
    fn keep_update_for_later(&mut self) {
        if let Some(u) = &self.updater {
            u.keep_for_later();
        }
    }
    fn data_folder(&self) -> Option<(PathBuf, bool)> {
        self.data.clone().map(|d| (d, self.base.is_some()))
    }
    fn take_data_moved(&mut self) -> Option<Result<PathBuf, String>> {
        self.moved.take()
    }
    fn pending_data_move(&self) -> Option<PathBuf> {
        self.base.as_deref().and_then(location::pending_move)
    }
    fn move_data_folder(&mut self, to: Option<&Path>) -> Result<Option<PathBuf>, String> {
        let base = self.base.as_deref().ok_or("the data folder was chosen when Quatrelle started")?;
        match to {
            Some(to) => location::schedule_move(base, to).map(Some),
            None => {
                location::cancel_move(base);
                Ok(None)
            }
        }
    }
    fn craft_app(&mut self, command: &str) -> Option<PathBuf> {
        let exe = if cfg!(windows) { format!("{command}.exe") } else { command.to_string() };
        // On the PATH (the CLI installers and Linux packages put it there)…
        if let Some(paths) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&paths) {
                let p = dir.join(&exe);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
        // …or where each platform's installer puts apps.
        let title = match command {
            "pdfcraft" => "PdfCraft",
            "wordcraft" => "WordCraft",
            "gridcraft" => "GridCraft",
            "deckcraft" => "DeckCraft",
            "photocraft" => "PhotoCraft",
            "filmcraft" => "FilmCraft",
            "soundcraft" => "SoundCraft",
            _ => return None,
        };
        let mut places: Vec<PathBuf> = Vec::new();
        if cfg!(windows) {
            for var in ["ProgramFiles", "LOCALAPPDATA"] {
                if let Some(base) = std::env::var_os(var) {
                    let base = PathBuf::from(base);
                    places.push(base.join(title).join(&exe));
                    places.push(base.join("Programs").join(title).join(&exe));
                }
            }
        } else if cfg!(target_os = "macos") {
            places.push(PathBuf::from(format!("/Applications/{title}.app")));
            // Installed for one user (an account that can't write /Applications).
            if let Some(home) = std::env::var_os("HOME") {
                places.push(PathBuf::from(home).join("Applications").join(format!("{title}.app")));
            }
        } else {
            places.push(PathBuf::from(format!("/opt/{command}/bin/{command}")));
        }
        places.into_iter().find(|p| p.exists())
    }
    fn open_with(&mut self, app: &Path, file: &Path) -> Result<(), String> {
        let mut cmd = if cfg!(target_os = "macos") && app.extension().is_some_and(|e| e == "app") {
            let mut c = std::process::Command::new("open");
            c.arg("-a").arg(app).arg(file);
            c
        } else {
            let mut c = std::process::Command::new(app);
            c.arg(file);
            c
        };
        cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
    }
    fn cjk_font(&mut self, lang: &str) -> Option<(Vec<u8>, u32)> {
        // The language's own font first (Chinese, Japanese and Korean draw shared characters
        // differently), then any that has the characters at all.
        let (windows, macos, noto_index): (&[&str], &[&str], u32) = match lang {
            "ja" => (&["YuGothM.ttc", "msgothic.ttc"], &["ヒラギノ角ゴシック W3.ttc", "Hiragino Sans GB.ttc"], 0),
            "ko" => (&["malgun.ttf", "gulim.ttc"], &["AppleSDGothicNeo.ttc"], 1),
            "zh-hant" => (&["msjh.ttc", "mingliu.ttc"], &["PingFang.ttc"], 3),
            _ => (&["msyh.ttc", "simsun.ttc"], &["PingFang.ttc", "Hiragino Sans GB.ttc"], 2),
        };
        let mut candidates: Vec<(String, u32)> = Vec::new();
        if cfg!(target_os = "windows") {
            let dir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
            for f in windows.iter().chain(["msyh.ttc", "YuGothM.ttc", "malgun.ttf"].iter()) {
                candidates.push((format!("{dir}\\Fonts\\{f}"), 0));
            }
        } else if cfg!(target_os = "macos") {
            for f in macos.iter().chain(["PingFang.ttc", "AppleSDGothicNeo.ttc"].iter()) {
                candidates.push((format!("/System/Library/Fonts/{f}"), 0));
            }
        } else {
            // Noto Sans CJK collections hold Japanese, Korean, Simplified and Traditional faces.
            for p in [
                "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
            ] {
                candidates.push((p.to_string(), noto_index));
            }
            candidates.push(("/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf".into(), 0));
        }
        candidates.iter().find_map(|(p, i)| std::fs::read(p).ok().map(|b| (b, *i)))
    }
    fn typeface(&mut self, family: &str) -> Option<(Vec<u8>, Option<Vec<u8>>)> {
        let (regular, bold) = appearance::typeface_files(family)?;
        let regular = std::fs::read(regular).ok()?;
        Some((regular, bold.and_then(|b| std::fs::read(b).ok())))
    }
    fn rtl_font(&mut self) -> Option<Vec<u8>> {
        // Fonts with both Arabic and Hebrew first.
        let candidates: &[&str] = if cfg!(target_os = "windows") {
            &["C:\\Windows\\Fonts\\tahoma.ttf", "C:\\Windows\\Fonts\\arial.ttf"]
        } else if cfg!(target_os = "macos") {
            &["/System/Library/Fonts/Supplemental/Arial.ttf", "/Library/Fonts/Arial.ttf", "/System/Library/Fonts/Supplemental/Tahoma.ttf"]
        } else {
            &[
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/TTF/DejaVuSans.ttf",
                "/usr/share/fonts/truetype/noto/NotoSansArabic-Regular.ttf",
                "/usr/share/fonts/noto/NotoSansArabic-Regular.ttf",
            ]
        };
        candidates.iter().find_map(|p| std::fs::read(p).ok())
    }
    fn emoji_font(&mut self) -> Option<(Vec<u8>, u32)> {
        let (path, index) = appearance::emoji_font_file()?;
        let bytes = std::fs::read(&path).ok()?;
        log::info!("colour emoji from {}", path.display());
        Some((bytes, index))
    }
    fn can_drag_out(&self) -> bool {
        // Elsewhere, under X11 only: a Wayland window isn't an X window. A window on XWayland is
        // one, though `WAYLAND_DISPLAY` is set there too (testing it turned drag-out off on every
        // Wayland desktop): the window's own choice says.
        cfg!(any(target_os = "windows", target_os = "macos")) || display::choose_from_env() == display::Choice::X11
    }
    fn video_frame(&mut self, path: &Path) -> VideoFrame {
        if let Some(slot) = self.frames.get(path) {
            return slot.lock().map(|g| g.clone().unwrap_or(VideoFrame::Pending)).unwrap_or(VideoFrame::Unavailable);
        }
        let source = self.frame_source.get_or_insert_with(video::source).clone();
        let Some(source) = source else { return VideoFrame::Unavailable };
        let slot = Arc::new(std::sync::Mutex::new(None));
        self.frames.insert(path.to_path_buf(), slot.clone());
        let video = path.to_path_buf();
        let spawned = std::thread::Builder::new().name("video-thumbnail".into()).spawn(move || {
            let png = video::frame(&source, &video);
            if let Ok(mut g) = slot.lock() {
                *g = Some(png.map_or(VideoFrame::Unavailable, VideoFrame::Ready));
            }
        });
        if spawned.is_err() {
            return VideoFrame::Unavailable;
        }
        VideoFrame::Pending
    }
    fn pick_folder(&mut self) -> Option<PathBuf> {
        let folder = self.dialog("Save attachments to").pick_folder();
        self.used(folder, true)
    }
    fn pick_certificate(&mut self) -> Option<PathBuf> {
        let file =
            self.dialog("Import certificate").add_filter("Certificate with its key", &["p12", "pfx"]).add_filter("All files", &["*"]).pick_file();
        self.used(file, false)
    }
    fn pick_rules_file(&mut self) -> Option<PathBuf> {
        let file = self.dialog("Import rules").add_filter("Quatrelle rules", &["json"]).add_filter("All files", &["*"]).pick_file();
        self.used(file, false)
    }
    fn pick_data_file(&mut self) -> Option<PathBuf> {
        let mut d = rfd::FileDialog::new();
        // Where the data files are (a removed account's stays there), as Outlook opens its own.
        if let Some(dir) = self.data.as_ref().filter(|d| d.is_dir()) {
            d = d.set_directory(dir);
        }
        d.set_title("Open data file")
            .add_filter("Data files", &["pst", "quatrelle"])
            .add_filter("Outlook data files", &["pst"])
            .add_filter("Quatrelle data files", &["quatrelle"])
            .pick_file()
    }
    fn pick_pictures(&mut self) -> Vec<PathBuf> {
        let files =
            self.dialog("Insert pictures").add_filter("Pictures", &["png", "jpg", "jpeg", "gif", "webp", "bmp"]).pick_files().unwrap_or_default();
        self.used(files.first(), false);
        files
    }
    fn clipboard_picture(&mut self) -> Option<Vec<u8>> {
        let img = read_clipboard(|c| c.get_image().ok())?;
        let (w, h) = (u32::try_from(img.width).ok()?, u32::try_from(img.height).ok()?);
        let rgba = image::RgbaImage::from_raw(w, h, img.bytes.into_owned())?;
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(rgba).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
        Some(png)
    }
    fn open_url(&mut self, url: &str) -> Result<(), String> {
        // A network share (`file://server/share/…`, as the reader keeps one): Windows opens it by
        // its own name (`\\server\share\…`, Explorer for a folder); Linux, the BSDs and
        // macOS by `smb://`, which their file managers (GVfs, KIO, Finder) know.
        if let Some(rest) = url.strip_prefix("file://") {
            let Some((_, unc)) = quatrelle_engine::mime::share_parts(url) else { return Err("not a network share".into()) };
            #[cfg(windows)]
            {
                let _ = rest;
                return shell_open(std::ffi::OsStr::new(&unc));
            }
            #[cfg(all(unix, not(target_os = "macos")))]
            {
                let _ = unc;
                return show_share(format!("smb://{rest}"));
            }
            #[cfg(not(any(windows, all(unix, not(target_os = "macos")))))]
            {
                let _ = unc;
                return shell_open(std::ffi::OsStr::new(&format!("smb://{rest}")));
            }
        }
        // Web addresses go to Windows' URL handler: ShellExec_RunDLL (right for files) started
        // and quietly showed nothing, so sign-in pages never reached the browser.
        #[cfg(windows)]
        if url.starts_with("https://") || url.starts_with("http://") || url.starts_with("mailto:") {
            return std::process::Command::new("rundll32.exe")
                .arg("url.dll,FileProtocolHandler")
                .arg(url)
                .spawn()
                .map(drop)
                .map_err(|e| e.to_string());
        }
        shell_open(std::ffi::OsStr::new(url))
    }
    fn browsers(&mut self) -> Vec<(String, String)> {
        browsers::found().into_iter().map(|b| (b.name.to_string(), b.program.display().to_string())).collect()
    }
    fn browser_profiles(&mut self, program: &str) -> Vec<BrowserProfile> {
        // Which browser the program is says where its data is; a program not found has none.
        let Some(browser) = browsers::found().into_iter().find(|b| b.program.display().to_string() == program) else { return Vec::new() };
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
        let env = |v: &str| std::env::var_os(v).filter(|p| !p.is_empty()).map(PathBuf::from);
        profiles::user_data_dir(browser.name, profiles::Platform::HERE, home.as_deref(), env)
            .map(|dir| profiles::read_profiles(&dir))
            .unwrap_or_default()
    }
    fn open_app_window(&mut self, program: &str, url: &str) -> Result<(), String> {
        browsers::launch(Path::new(program), url)
    }
    fn notify(&mut self, title: &str, body: &str) {
        let (title, body) = (title.to_string(), body.to_string());
        #[cfg(windows)]
        let portable = self.portable;
        // Off the UI thread: the notification service can be slow, or missing.
        std::thread::spawn(move || {
            let mut n = notify_rust::Notification::new();
            n.summary(&title).body(&body).appname("Quatrelle");
            // Windows names a toast after its app id (PowerShell's when none is given). A
            // portable copy writes nothing to the registry: it uses an installed Quatrelle's.
            #[cfg(windows)]
            if (!portable && toast::registered()) || (portable && toast::installed()) {
                n.app_id(toast::APP_ID);
            }
            if let Err(e) = n.show() {
                log::warn!("notification: {e}");
            }
        });
    }
    /// A reminder on the desktop. On Linux (and the BSDs) it's one that stays until answered,
    /// sounding the alarm, with Snooze and Dismiss buttons and a click that brings the window
    /// forward: a thread waits on what's pressed and leaves it for the window. Elsewhere, a
    /// plain notification (Windows' toasts have no buttons without unsafe code; macOS's none).
    fn notify_reminder(&mut self, key: &str, title: &str, body: &str, snooze: &str, dismiss: &str) {
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let (key, title, body, snooze, dismiss) = (key.to_string(), title.to_string(), body.to_string(), snooze.to_string(), dismiss.to_string());
            let answers = self.reminder_answers.clone();
            let waker = self.waker.clone();
            let notes = self.reminder_notes.0.clone();
            let spawned = std::thread::Builder::new().name("reminder-notification".into()).spawn(move || {
                let mut n = notify_rust::Notification::new();
                n.summary(&title)
                    .body(&body)
                    .appname("Quatrelle")
                    .urgency(notify_rust::Urgency::Critical)
                    .timeout(notify_rust::Timeout::Never)
                    .hint(notify_rust::Hint::SoundName("alarm-clock-elapsed".into()))
                    // The notification itself (its label is the title: a desktop that shows
                    // one says what opens).
                    .action("default", &title)
                    .action("snooze", &snooze)
                    .action("dismiss", &dismiss);
                let handle = match n.show() {
                    Ok(h) => Arc::new(h),
                    Err(e) => {
                        log::warn!("reminder notification: {e}");
                        return;
                    }
                };
                // Kept, so it can be taken back; told again (after a snooze), the one before goes.
                let before = notes.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(key.clone(), handle.clone());
                if let Some(before) = before {
                    async_io::block_on(before.close_async());
                }
                async_io::block_on(handle.wait_for_action_async(|response| {
                    let what = match response {
                        notify_rust::NotificationResponse::Action(a) if a == "snooze" => ReminderAction::Snooze,
                        notify_rust::NotificationResponse::Action(a) if a == "dismiss" => ReminderAction::Dismiss,
                        notify_rust::NotificationResponse::Default => ReminderAction::Open,
                        // Closed, or timed out by the desktop: the window still shows it.
                        _ => return,
                    };
                    answers.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((key.clone(), what));
                    if let Some(wake) = &waker {
                        wake();
                    }
                }));
                // Answered or closed: no longer up (unless told again meanwhile).
                let mut notes = notes.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                if notes.get(&key).is_some_and(|n| Arc::ptr_eq(n, &handle)) {
                    notes.remove(&key);
                }
            });
            if let Err(e) = spawned {
                log::warn!("reminder notification: {e}");
            }
        }
        #[cfg(not(all(unix, not(target_os = "macos"))))]
        {
            let _ = (key, snooze, dismiss);
            self.notify(title, body);
        }
    }
    fn show_tray(&mut self, icon: &quatrelle_ui_egui::TrayIcon) {
        self.tray.show(icon);
    }
    fn take_tray_action(&mut self) -> Option<quatrelle_ui_egui::TrayAction> {
        self.tray.take_action()
    }
    fn tray_shown(&self) -> bool {
        self.tray.shown()
    }
    fn take_reminder_actions(&mut self) -> Vec<(String, ReminderAction)> {
        std::mem::take(&mut *self.reminder_answers.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
    }
    /// The notification closed, off the window's thread (a call on the session bus); its
    /// waiting thread then ends.
    #[cfg(all(unix, not(target_os = "macos")))]
    fn withdraw_reminder(&mut self, key: &str) {
        let Some(note) = self.reminder_notes.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(key) else { return };
        let spawned = std::thread::Builder::new().name("reminder-close".into()).spawn(move || async_io::block_on(note.close_async()));
        if let Err(e) = spawned {
            log::warn!("reminder notification: {e}");
        }
    }
}

/// A network share (`smb://server/share/…`) in the desktop's file manager, which mounts it
/// (asking for a password when the server wants one): a folder opened, a file shown in its
/// folder (not run). Through the file managers' common D-Bus interface (Nautilus, Dolphin,
/// Nemo…), on a thread of its own as starting one can take a moment; without one, the system's
/// handler for `smb://` (GNOME declares none: it went nowhere).
#[cfg(all(unix, not(target_os = "macos")))]
fn show_share(uri: String) -> Result<(), String> {
    let file = uri.rsplit('/').next().is_some_and(|last| last.contains('.'));
    std::thread::Builder::new()
        .name("open-share".into())
        .spawn(move || {
            let method = if file { "ShowItems" } else { "ShowFolders" };
            let shown = zbus::blocking::Connection::session().and_then(|bus| {
                bus.call_method(
                    Some("org.freedesktop.FileManager1"),
                    "/org/freedesktop/FileManager1",
                    Some("org.freedesktop.FileManager1"),
                    method,
                    &(vec![uri.as_str()], ""),
                )
                .map(drop)
            });
            if let Err(e) = shown {
                log::info!("no file manager answered for {uri} ({e}): the system's handler opens it");
                if let Err(e) = open::that_detached(&uri) {
                    log::warn!("{uri} couldn't be opened: {e}");
                }
            }
        })
        .map(drop)
        .map_err(|e| e.to_string())
}

/// Open a file or a web address with the system's choice of app, as Explorer would.
///
/// On Windows this goes through `rundll32 shell32.dll,ShellExec_RunDLL`, a process of its own:
/// opening from Quatrelle (directly, or through the hidden PowerShell the `open` crate starts)
/// silently did nothing for apps such as Acrobat, and a file no app opens gets Windows' "How do
/// you want to open this file?", as Outlook does.
fn shell_open(target: &std::ffi::OsStr) -> Result<(), String> {
    #[cfg(windows)]
    {
        // A folder opens in Explorer: ShellExec_RunDLL quietly does nothing with one (Options ›
        // Data files › Open folder never opened).
        if std::path::Path::new(target).is_dir() {
            return std::process::Command::new("explorer.exe").arg(target).spawn().map(drop).map_err(|e| e.to_string());
        }
        std::process::Command::new("rundll32.exe").arg("shell32.dll,ShellExec_RunDLL").arg(target).spawn().map(drop).map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        open::that_detached(target).map_err(|e| e.to_string())
    }
}

/// The system's UI font, if installed (never bundled).
pub(crate) fn system_fonts() -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    let candidates: &[(&str, &str)] = if cfg!(target_os = "windows") {
        &[("C:\\Windows\\Fonts\\segoeui.ttf", "C:\\Windows\\Fonts\\segoeuib.ttf")]
    } else if cfg!(target_os = "macos") {
        &[("/System/Library/Fonts/Supplemental/Arial.ttf", "/System/Library/Fonts/Supplemental/Arial Bold.ttf")]
    } else {
        // The desktop's own interface font, when it has a bold of its own.
        #[cfg(all(unix, not(target_os = "macos")))]
        if let Some((r, b)) = appearance::desktop_font_files()
            && let (Ok(reg), Ok(bold)) = (std::fs::read(r), std::fs::read(b))
        {
            return (Some(reg), Some(bold));
        }
        &[
            ("/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf", "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf"),
            ("/usr/share/fonts/noto/NotoSans-Regular.ttf", "/usr/share/fonts/noto/NotoSans-Bold.ttf"),
            ("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"),
        ]
    };
    for (r, b) in candidates {
        if let Ok(reg) = std::fs::read(r) {
            return (Some(reg), std::fs::read(b).ok());
        }
    }
    (None, None)
}

/// Hand a file to the system's drag and drop (Windows and macOS; elsewhere egui has no way to).
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub(crate) fn drag_out(frame: &eframe::Frame, path: PathBuf) {
    const PNG: &[u8] = include_bytes!("../../../assets/app-icon/hicolor/64x64/apps/ai.storyteller.quatrelle.png");
    let options = drag::Options { skip_animatation_on_cancel_or_failure: false, mode: drag::DragMode::Copy };
    // The drag crate is native code; a failure there must not take the app down.
    let started = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        drag::start_drag(frame, drag::DragItem::Files(vec![path]), drag::Image::Raw(PNG.to_vec()), |_, _| {}, options)
    }));
    match started {
        Ok(Ok(())) => {}
        Ok(Err(e)) => log::warn!("drag out: {e}"),
        Err(_) => log::warn!("drag out failed"),
    }
}

/// Hand a file to X11's drag and drop (XDND), on a thread of its own while the button is held.
#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) fn drag_out(frame: &eframe::Frame, path: PathBuf) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let own = match frame.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Xlib(h)) => u32::try_from(h.window).ok(),
        Ok(RawWindowHandle::Xcb(h)) => Some(h.window.get()),
        _ => None,
    };
    let Some(own) = own else {
        log::info!("drag out: not an X11 window");
        return;
    };
    let spawned = std::thread::Builder::new().name("drag-out".into()).spawn(move || {
        if let Err(e) = xdnd::drag(None, own, &path) {
            log::warn!("drag out: {e}");
        }
    });
    if let Err(e) = spawned {
        log::warn!("drag out: {e}");
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
pub(crate) fn drag_out(_frame: &eframe::Frame, _path: PathBuf) {}

/// How long a clipboard read may wait on the app that copied (Linux and the BSDs).
#[cfg(all(unix, not(target_os = "macos")))]
const CLIPBOARD_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// Read the clipboard. On Linux and the BSDs the read runs on a thread of its own and gives up
/// after [`CLIPBOARD_WAIT`]: a Wayland or X11 read waits on the app that copied, with no time
/// limit of its own, so an app that hangs would hang Quatrelle's window too (FigCraft found).
#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) fn read_clipboard<T: Send + 'static>(read: fn(&mut arboard::Clipboard) -> Option<T>) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("clipboard".into())
        .spawn(move || {
            let _ = tx.send(arboard::Clipboard::new().ok().and_then(|mut c| read(&mut c)));
        })
        .ok()?;
    rx.recv_timeout(CLIPBOARD_WAIT).ok().flatten()
}

/// Read the clipboard (Windows' and macOS's reads don't wait on the app that copied).
#[cfg(not(all(unix, not(target_os = "macos"))))]
pub(crate) fn read_clipboard<T>(read: fn(&mut arboard::Clipboard) -> Option<T>) -> Option<T> {
    read(&mut arboard::Clipboard::new().ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The next dialog opens where the last answer was: a file's folder, or the folder picked;
    /// a cancelled dialog leaves it, and a folder that's gone isn't offered.
    #[test]
    fn dialogs_open_where_the_last_one_was() {
        let root = std::env::temp_dir().join(format!("quatrelle-dialogs-{}", std::process::id()));
        let (docs, out) = (root.join("Docs"), root.join("Out"));
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::create_dir_all(&out).unwrap();
        let mut d = Desktop::default();
        assert_eq!(d.used(Some(docs.join("plan.pdf")), false), Some(docs.join("plan.pdf")));
        assert_eq!(d.last_dir.as_deref(), Some(docs.as_path()));
        d.used(None::<PathBuf>, false);
        assert_eq!(d.last_dir.as_deref(), Some(docs.as_path()), "cancelled: kept");
        d.used(Some(out.clone()), true);
        assert_eq!(d.last_dir.as_deref(), Some(out.as_path()));
        let _ = d.dialog("Attach files");
        std::fs::remove_dir_all(&root).unwrap();
        assert!(d.last_dir.as_ref().is_some_and(|p| !p.is_dir()), "gone: the dialog opens where the system says");
        let _ = d.dialog("Attach files");
    }
}
