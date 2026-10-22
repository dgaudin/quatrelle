//! What the window asks of the desktop (`Services`): the clock and the system's style and fonts,
//! files and web addresses opened with the system's apps, the clipboard, files dragged out of
//! the window, notifications, and video frames.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use quatrelle_engine::store::location;
use quatrelle_ui_egui::{ReminderAction, Services, VideoFrame};

#[cfg(windows)]
use crate::toast;
#[cfg(all(unix, not(target_os = "macos")))]
use crate::xdnd;
use crate::{appearance, display, self_update, video};

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
    pub(crate) waker: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl Services for Desktop {
    fn system_style(&self) -> quatrelle_ui_egui::theme::SystemStyle {
        self.style.get()
    }
    fn clock(&self) -> (i64, i32) {
        let now = chrono::Local::now();
        (now.timestamp(), now.offset().local_minus_utc())
    }
    fn pick_files(&mut self) -> Vec<PathBuf> {
        rfd::FileDialog::new().set_title("Attach files").pick_files().unwrap_or_default()
    }
    fn save_path(&mut self, suggested: &str) -> Option<PathBuf> {
        let mut d = rfd::FileDialog::new().set_title("Save as").set_file_name(suggested);
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
        d.save_file()
    }
    fn open_path(&mut self, path: &Path) -> Result<(), String> {
        shell_open(path.as_os_str())
    }
    fn pending_update(&mut self) -> Option<String> {
        self.updater.as_ref().and_then(self_update::Updater::ready)
    }
    fn restart_to_update(&mut self) -> bool {
        self.updater.as_ref().is_some_and(self_update::Updater::restart_after_close)
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
        rfd::FileDialog::new().set_title("Save attachments to").pick_folder()
    }
    fn pick_rules_file(&mut self) -> Option<PathBuf> {
        rfd::FileDialog::new().set_title("Import rules").add_filter("Quatrelle rules", &["json"]).add_filter("All files", &["*"]).pick_file()
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
        rfd::FileDialog::new()
            .set_title("Insert pictures")
            .add_filter("Pictures", &["png", "jpg", "jpeg", "gif", "webp", "bmp"])
            .pick_files()
            .unwrap_or_default()
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
                    Ok(h) => h,
                    Err(e) => {
                        log::warn!("reminder notification: {e}");
                        return;
                    }
                };
                handle.wait_for_action(|action| {
                    let what = match action {
                        "snooze" => ReminderAction::Snooze,
                        "dismiss" => ReminderAction::Dismiss,
                        "default" => ReminderAction::Open,
                        // Closed, or timed out by the desktop: the window still shows it.
                        _ => return,
                    };
                    answers.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((key.clone(), what));
                    if let Some(wake) = &waker {
                        wake();
                    }
                });
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
    fn take_reminder_actions(&mut self) -> Vec<(String, ReminderAction)> {
        std::mem::take(&mut *self.reminder_answers.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
    }
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
