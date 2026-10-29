//! Quatrelle desktop app.
//!
//! Usage: `quatrelle [--version] [--sample] [--data <folder>] [--light | --dark] [--cmd <id>[=<json>]]… [mailto:… | file]…`
//!
//! `--sample` opens a separate, local-only sample mailbox (no server needed). `--data` (or the
//! `QUATRELLE_DATA` environment variable) chooses where mail is cached. `--cmd` runs engine
//! commands at start-up (`--cmd 'list.open={"key":"sample:1:1:INBOX"}'`); `ui.file` (or
//! `ui.settings`), `ui.openExport`, `ui.options` and `ui.about` open the File page at that page;
//! `ui.addAccount` (optionally `={"email":…,"advanced":true,"graph":true}` or `"pop":true`), `ui.newEvent`
//! (optionally `={"at":"YYYY-MM-DDTHH:MM","summary":…,"guests":…,"rooms":…,"scheduling":true,"finder":true}`)
//! and `ui.newContact` open those dialogs, `ui.rules` the Rules dialog; `ui.folderSize`
//! (optionally `={"account":…}`) opens Folder Size over the File page.
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

/// See `Cargo.toml`: memory freed is given back to the system rather than kept by malloc.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[global_allocator]
static ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

use quatrelle_ui_egui::backstage::{Backstage, Page};
use std::path::PathBuf;
use std::sync::Arc;

use quatrelle_engine::Session;
use quatrelle_engine::model::dates::Clock;
use quatrelle_engine::model::{Event, Job};
use quatrelle_engine::store::{Store, location};
use quatrelle_net::worker::{Credentials, Pool};
use quatrelle_ui_egui::{Backend, MailApp, Services};

#[cfg(target_os = "macos")]
mod apple_events;
mod args;
mod desktop;
mod display;
mod graphics;
mod logfile;
mod portable;
mod screenshot;
mod secrets;
mod self_update;
mod single_instance;
#[cfg(windows)]
mod toast;
mod video;
#[cfg(test)]
mod windows_manifest;
#[cfg(all(unix, not(target_os = "macos")))]
mod xdnd;

use desktop::{Desktop, drag_out, read_clipboard, system_fonts};
use desktop::{appearance, keychain::Keychain};

struct PoolBackend(Pool);

impl Backend for PoolBackend {
    fn submit(&mut self, job: Job) {
        self.0.submit(job);
    }
    fn poll(&mut self) -> Vec<Event> {
        self.0.poll()
    }
}

/// The window icon: the committed app icon (`cargo xtask icons`), or egui's default if it can't be decoded.
fn app_icon() -> egui::IconData {
    const PNG: &[u8] = include_bytes!("../../../assets/app-icon/hicolor/64x64/apps/ai.storyteller.quatrelle.png");
    match image::load_from_memory_with_format(PNG, image::ImageFormat::Png) {
        Ok(img) => {
            let img = img.into_rgba8();
            egui::IconData { width: img.width(), height: img.height(), rgba: img.into_raw() }
        }
        Err(_) => egui::IconData::default(),
    }
}

struct App {
    app: MailApp,
    state_path: Option<PathBuf>,
    /// The window's GPU device, until its errors are only logged (`graphics::log_errors`).
    gpu: Option<eframe::wgpu::Device>,
    frames: u32,
    /// Answering later launches on the same data folder (`single_instance.rs`).
    instance: Option<single_instance::Server>,
    /// Documents macOS sent over since the last frame (`apple_events.rs`).
    #[cfg(target_os = "macos")]
    finder: fmv_macos_events::Inbox,
    native_wayland: bool,
    /// Installs a downloaded update as Quatrelle closes.
    updater: Option<self_update::Updater>,
    /// The settings as last written, and when they were last looked at ([`App::keep_state`]).
    saved: Vec<u8>,
    looked: std::time::Instant,
    /// `QUATRELLE_SCREENSHOT`: the window saved as a picture, then closed.
    screenshot: Option<screenshot::Shot>,
    /// Frames that don't wait for the screen kept to 60 a second (`graphics::Pacing`).
    pacing: graphics::Pacing,
}

/// How often changed settings are written while Quatrelle runs.
const KEEP_STATE_EVERY: std::time::Duration = std::time::Duration::from_secs(5);

impl App {
    /// Write the settings when they've changed: a reminder dismissed or a sender blocked isn't
    /// lost if Quatrelle is ended without closing (a crash, a power cut, a forced quit).
    fn keep_state(&mut self, ctx: Option<&egui::Context>, force: bool) {
        let since = self.looked.elapsed();
        if !force && since < KEEP_STATE_EVERY {
            // Something may have changed in this frame: look again when it's time, even if
            // nothing else draws a frame by then.
            if let Some(ctx) = ctx {
                ctx.request_repaint_after(KEEP_STATE_EVERY.saturating_sub(since));
            }
            return;
        }
        self.looked = std::time::Instant::now();
        let Some(p) = &self.state_path else { return };
        let Ok(bytes) = serde_json::to_vec_pretty(&self.app.saved_state()) else { return };
        if bytes != self.saved {
            match quatrelle_engine::store::write_atomic(p, &bytes) {
                Ok(()) => self.saved = bytes,
                Err(e) => log::warn!("the settings weren't saved: {e}"),
            }
        }
    }
}

impl eframe::App for App {
    // Every frame, the window minimized or hidden too (when eframe runs no `ui`): started again,
    // this window comes forward instead of a second one, and opens what that launch handed over.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pacing.frame(ctx);
        if let Some(handed) = self.instance.as_ref().and_then(single_instance::Server::take) {
            single_instance::show(ctx, self.native_wayland);
            for h in &handed {
                self.app.open_handed(h);
            }
        }
        #[cfg(target_os = "macos")]
        apple_events::deliver(&self.finder, &mut self.app, ctx);
        // Mail, notifications, reminders and an update ready keep going while the window is
        // minimized or hidden, and a close from the taskbar or the Dock then is asked about here,
        // as `ui` doesn't run.
        self.app.logic(ctx);
        // On a Mac a close nothing held open quits through AppKit (`apple_events::quit`); last, so
        // it sees whether a question held the window.
        #[cfg(target_os = "macos")]
        MailApp::quit_instead_of_close(ctx, apple_events::quit);
    }
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.frames = self.frames.saturating_add(1);
        if self.frames >= graphics::STARTUP_FRAMES
            && let Some(device) = self.gpu.take()
        {
            graphics::log_errors(&device);
        }
        self.app.ui(ui);
        self.keep_state(Some(ui.ctx()), false);
        if let Some(path) = self.app.take_drag_out() {
            drag_out(frame, path);
        }
        // Its picture saved, the window closes for good, not into the notification area.
        if self.screenshot.is_some() {
            self.app.ui.quitting = true;
        }
        if let Some(shot) = &mut self.screenshot
            && shot.step(ui.ctx(), self.frames)
        {
            // The window wouldn't close by itself: end here, the state kept as closing keeps it.
            log::warn!("the window didn't close once its picture was saved: ending");
            self.on_exit();
            std::process::exit(0);
        }
    }
    fn on_exit(&mut self) {
        if let Some(server) = &self.instance {
            server.close();
        }
        // The address book's latest changes (it's written a minute at most after each).
        self.app.session.save_contacts();
        // A test build downloaded and checked: installed now that Quatrelle closes (unless it was
        // kept for later); a download still going stops, to carry on next time.
        if let Some(u) = &self.updater {
            u.closing();
        }
        self.keep_state(None, true);
        // Issue #4: attachments handed to other apps, and pictures made of them, go with
        // Quatrelle when it's portable or keeps an account online only (one still open in an
        // app stays until that app lets it go).
        let s = &self.app.session;
        if s.portable || s.accounts.iter().any(|a| a.config.keeps_online()) {
            quatrelle_engine::store::clear_scratch();
        }
        // macOS: end here, before eframe tears the window down. AccessKit's macOS adapter
        // subclasses the window's content view and puts its class back on teardown; AppKit's
        // Touch Bar support observes that view, so in the next display cycle it throws ("not
        // registered") and quitting ends in a crash report (DeckCraft and GridCraft found). Every
        // way of quitting comes here, and nothing after it is needed: the window's state is
        // saved, the log is unbuffered and the data files keep what they committed.
        #[cfg(target_os = "macos")]
        std::process::exit(0);
    }
}

/// jemalloc purges freed pages only when the thread that freed them allocates again, so a sync
/// thread that ends, or a window left alone, kept them; its background thread purges them after
/// ten seconds unused. One is enough.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn give_back_freed_memory() {
    let _ = tikv_jemalloc_ctl::max_background_threads::write(1);
    match tikv_jemalloc_ctl::background_thread::write(true) {
        Ok(()) => log::info!("memory: freed memory goes back to the system"),
        Err(e) => log::warn!("memory: freed memory stays with the app (no background thread: {e})"),
    }
    // What the memory is, in the log now and then: what's in use, and the rest of what the
    // process holds (pictures drawn, fonts, the program itself).
    let _ = std::thread::Builder::new().name("memory".into()).spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(60));
        loop {
            log_memory();
            std::thread::sleep(std::time::Duration::from_secs(10 * 60));
        }
    });
}

/// One line on the memory: the process's resident size, and the allocator's share of it.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn log_memory() {
    use tikv_jemalloc_ctl::{epoch, stats};
    if epoch::advance().is_err() {
        return;
    }
    let mb = |r: Result<usize, tikv_jemalloc_ctl::Error>| r.map_or(0, |b| b >> 20);
    let rss = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("VmRSS:")).and_then(|l| l.split_whitespace().nth(1)).and_then(|k| k.parse::<usize>().ok()))
        .map_or(0, |kb| kb >> 10);
    log::info!(
        "memory: {rss} MB resident; the allocator's {} MB resident, {} MB in pages used, {} MB in use; {} MB on the graphics card",
        mb(stats::resident::read()),
        mb(stats::active::read()),
        mb(stats::allocated::read()),
        graphics_memory() >> 10
    );
}

/// What the graphics driver holds for this process (KiB), as the kernel tells it for each of its
/// clients (`/proc/self/fdinfo`, `drm-total-*`): an integrated GPU's memory is the computer's.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn graphics_memory() -> u64 {
    let Ok(fds) = std::fs::read_dir("/proc/self/fdinfo") else { return 0 };
    let mut clients = std::collections::HashMap::new();
    for fd in fds.flatten().take(4096) {
        let Ok(info) = std::fs::read_to_string(fd.path()) else { continue };
        let Some(client) = info.lines().find_map(|l| l.strip_prefix("drm-client-id:")).map(|c| c.trim().to_string()) else { continue };
        let kib: u64 = info.lines().filter(|l| l.starts_with("drm-total-")).filter_map(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok()).sum();
        clients.insert(client, kib);
    }
    clients.values().sum()
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn give_back_freed_memory() {}

/// The system libraries' malloc (glibc's: the graphics driver's, X11's) keeps an arena for each
/// thread that ever allocated, each holding on to what it freed: 25 of them held 54 MB in an
/// Exchange user's Quatrelle, none of it Quatrelle's own (that's jemalloc's). Two are enough.
/// glibc reads `MALLOC_ARENA_MAX` only as a process starts, so Quatrelle starts again with it,
/// once, before anything else; one set already is left as it is, and the programs Quatrelle
/// opens inherit it.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn fewer_malloc_arenas() {
    use std::os::unix::process::CommandExt;
    if std::env::var_os("MALLOC_ARENA_MAX").is_some() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    // Returns only when it couldn't start again: then it goes on as it is.
    let _ = std::process::Command::new(exe).args(std::env::args_os().skip(1)).env("MALLOC_ARENA_MAX", "2").exec();
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn fewer_malloc_arenas() {}

fn main() -> eframe::Result {
    fewer_malloc_arenas();
    // `args()` panics on an argument that isn't Unicode (a file name in another encoding).
    let args: Vec<String> = std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        say_out(&args::version_line());
        return Ok(());
    }
    // Asking how to start it prints that, and no window opens.
    if args.iter().any(|a| matches!(a.as_str(), "--help" | "-h" | "/?")) {
        say_out(args::USAGE);
        return Ok(());
    }
    let cmd_args = args.clone();
    // `mailto:` links and mail files: Quatrelle as the system's mail app, or Open with (`args.rs`).
    let handed = args::handed(&args);
    let sample = args.iter().any(|a| a == "--sample");
    // Text pasted from Outlook, Word or a web page keeps its look.
    quatrelle_ui_egui::set_clipboard_html(|| read_clipboard(|c| c.get().html().ok()).filter(|h| !h.trim().is_empty()));
    // And text copied from a message keeps its look for Outlook or Word to paste.
    quatrelle_ui_egui::set_clipboard_html_writer(|html, text| {
        if let Ok(mut c) = arboard::Clipboard::new() {
            let _ = c.set_html(html.to_string(), Some(text.to_string()));
        }
    });
    // Each account is a data file in Documents\Quatrelle Files (as Outlook keeps .pst files in
    // Outlook Files); the account list and the app's state stay in the app folder. The data files'
    // folder can be moved (File › Options) when it's the usual one: a move asked for is carried
    // out now, before anything opens them.
    let chosen = sample || args.iter().any(|a| a == "--data") || quatrelle_engine::store::env_data_dir().is_some();
    // Portable (issue #4): `portable.txt` beside the app keeps everything in QuatrelleData there.
    let portable = if chosen { portable::Portable::default() } else { portable::current().clone() };
    let base = (!chosen && portable.dir.is_none()).then(location::base_root).flatten();
    let data = args
        .iter()
        .position(|a| a == "--data")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .or_else(|| portable.dir.clone())
        .or_else(quatrelle_engine::store::default_root);
    let root = data.map(|d| if sample { d.join("sample") } else { d });
    // One Quatrelle per data folder: started again, it brings the running one forward and leaves,
    // before it moves, logs or opens anything there (`single_instance.rs`).
    let instance = match root.as_deref().map(|r| single_instance::claim(r, &handed)) {
        Some(single_instance::Claim::Shown) => {
            say(if !handed.is_empty() {
                "Quatrelle is already running: what it was started with opens there"
            } else if args.iter().any(|a| a == "--cmd") {
                "Quatrelle is already running: its window is brought forward (--cmd is run only by the first)"
            } else {
                "Quatrelle is already running: its window is brought forward"
            });
            return Ok(());
        }
        Some(single_instance::Claim::Primary(server)) => Some(server),
        _ => None,
    };
    let mut moved = match base.as_deref().map(location::apply_pending_move) {
        Some(Ok(Some(to))) => Some(Ok(to)),
        Some(Err(e)) => Some(Err(e)),
        _ => None,
    };
    // The sample mailbox stays a folder of files; anything else keeps its data files beside it
    // (`--data`) or in Documents.
    let files =
        (!sample).then(|| root.as_ref().map(|r| base.as_deref().map_or_else(|| r.join(location::FILES_FOLDER), location::files_dir))).flatten();
    // Diagnostics go to `quatrelle.log` in the data folder.
    let echo = cfg!(debug_assertions) || args.iter().any(|a| a == "--log");
    if let Some(path) = logfile::start(root.as_deref(), echo) {
        log::info!("{} started; log at {}", args::version_line(), path.display());
    }
    give_back_freed_memory();
    if let Some(dir) = &portable.dir {
        log::info!("portable: everything is kept in {}", dir.display());
    }
    if let Some(why) = &portable.unwritable {
        log::warn!("{why}");
        say(why);
    }

    let (now, offset) = Desktop::default().clock();
    let store = root.as_ref().and_then(|r| {
        let opened = match &files {
            Some(f) => Store::open_packed(r, f).or_else(|e| {
                // Folders from before that couldn't come into data files stay as they are.
                moved = Some(Err(e.to_string()));
                Store::open(r)
            }),
            None => Store::open(r),
        };
        match opened {
            Ok(s) => Some(s),
            Err(e) => {
                say(&format!("the mail cache can't be opened ({e}); running without one"));
                None
            }
        }
    });
    if sample
        && let Some(s) = &store
        && let Err(e) = quatrelle_engine::sample::seed_if_empty_at(s, now, i64::from(offset))
    {
        say(&format!("the sample mailbox couldn't be created: {e}"));
    }
    let mut session = Session::open(store.clone(), Clock::new(now, offset));
    let desktop_data = store.as_ref().and_then(Store::data_files_dir);
    session.oauth_available = quatrelle_net::oauth::available();
    // S/MIME signers are trusted through the roots this computer trusts.
    session.set_trust_roots(quatrelle_net::trust_roots());
    session.set_card_modules(quatrelle_net::card_modules());
    session.portable = portable.dir.is_some();
    // A release build's commit, so File › About can tell its test build from a later one.
    session.build_sha = option_env!("QUATRELLE_BUILD_SHA").filter(|s| !s.trim().is_empty()).map(str::to_string);
    let state_path = root.as_ref().map(|r| r.join("ui.json"));
    // Test builds update themselves (`self_update.rs`); not the sample mailbox's run, unless it's
    // trying the updater out.
    let updater =
        root.clone().filter(|_| !sample || self_update::trying_out()).and_then(|r| self_update::Updater::start(r, session.build_sha.clone()));

    // The main window opens where it was, at the size it was, maximized if it was.
    let main_place: Option<quatrelle_ui_egui::WindowPlace> = state_path
        .as_ref()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v.get("windows")?.get("main").cloned())
        .and_then(|m| serde_json::from_value(m).ok());
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(if sample { "Quatrelle – Sample mailbox" } else { "Quatrelle" })
            // Matches the desktop entry (StartupWMClass) and icon name in packaging/linux.
            .with_app_id("ai.storyteller.quatrelle")
            .with_inner_size([1320.0, 820.0])
            .with_min_inner_size([760.0, 480.0])
            .with_icon(Arc::new(app_icon())),
        ..Default::default()
    };
    let mut options = match main_place {
        Some(p) => eframe::NativeOptions { viewport: p.apply(options.viewport), ..options },
        None => options,
    };
    graphics::configure(&mut options);
    // Linux and the BSDs: through XWayland on a Wayland desktop, so attachments drag in and out.
    #[cfg(all(unix, not(target_os = "macos")))]
    if display::choose_from_env() == display::Choice::X11 {
        options.event_loop_builder = Some(Box::new(|b| {
            use winit::platform::x11::EventLoopBuilderExtX11;
            b.with_x11();
        }));
    }
    // A window that can't open says why, rather than the app leaving without a word (or
    // staying in the background); a panic on the way is caught for that too.
    let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run(options, sample, store, session, desktop_data, base, moved, state_path, (cmd_args, handed), instance, updater)
    }));
    let failed = match ran {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(e)) => e.to_string(),
        Err(p) => p
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| p.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "it stopped unexpectedly".into()),
    };
    log::error!("the window couldn't open: {failed}");
    say(&format!("the window couldn't open: {failed}"));
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("Quatrelle")
        .set_description(cant_open(&failed))
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
    std::process::exit(1)
}

/// Tell standard error (and the log). `eprintln!` panics when standard error is a broken pipe,
/// which a launch from another program can leave it as (PdfCraft and GridCraft found).
fn say(what: &str) {
    log::warn!("{what}");
    let _ = std::io::Write::write_fmt(&mut std::io::stderr(), format_args!("quatrelle: {what}\n"));
}

/// Tell standard output, as `println!` would without its panic on a closed pipe.
fn say_out(what: &str) {
    let _ = std::io::Write::write_fmt(&mut std::io::stdout(), format_args!("{what}\n"));
}

/// What the message box says when the window can't open.
fn cant_open(why: &str) -> String {
    let (help, log) = if cfg!(windows) {
        (
            "Updating it may help, or starting Quatrelle with the environment variable WGPU_FORCE_FALLBACK_ADAPTER=1, which draws it with \
             Windows' software renderer.",
            "%APPDATA%\\Quatrelle\\quatrelle.log",
        )
    } else {
        ("Updating it may help.", "quatrelle.log, in Quatrelle's data folder")
    };
    format!("Quatrelle's window couldn't open:\n{why}\n\nThis is usually the graphics driver. {help}\n\nThe details are in {log}.")
}

/// Open the window and run the app until it closes.
#[allow(clippy::too_many_arguments)]
fn run(
    options: eframe::NativeOptions,
    sample: bool,
    store: Option<Store>,
    session: Session,
    desktop_data: Option<PathBuf>,
    base: Option<PathBuf>,
    moved: Option<Result<PathBuf, String>>,
    state_path: Option<PathBuf>,
    (cmd_args, handed): (Vec<String>, Vec<String>),
    instance: Option<single_instance::Server>,
    updater: Option<self_update::Updater>,
) -> eframe::Result {
    // The guard stays here, outside the closure, so the handlers live as long as the loop.
    #[cfg(target_os = "macos")]
    let (_finder_guard, finder) = apple_events::listen();
    let pacing = graphics::Pacing::new(options.wgpu_options.surface.present_mode);
    eframe::run_native(
        "Quatrelle",
        options,
        Box::new(move |cc| {
            let args = cmd_args;
            let app_portable = session.portable;
            let backend: Box<dyn Backend> = match store {
                Some(store) => {
                    // Portable: passwords and sign-ins in memory only, never the system's keychain.
                    let creds: Arc<dyn Credentials> =
                        if app_portable { Arc::new(quatrelle_net::worker::MemoryCredentials::default()) } else { Arc::new(Keychain) };
                    let mut pool = Pool::new(store, creds);
                    // Folders read on two connections of their own: the main one stays free
                    // for what someone is waiting on (see `SyncOptions::parallel`).
                    pool.opts.parallel = 2;
                    let ctx = cc.egui_ctx.clone();
                    pool.set_waker(Arc::new(move || ctx.request_repaint()));
                    Box::new(PoolBackend(pool))
                }
                None => Box::new(quatrelle_ui_egui::NullBackend),
            };
            let ctx = cc.egui_ctx.clone();
            let style = appearance::Watch::start(move || ctx.request_repaint());
            // The updater wakes the window as the update downloads and is ready.
            if let Some(u) = &updater {
                let ctx = cc.egui_ctx.clone();
                u.set_waker(Arc::new(move || ctx.request_repaint()));
            }
            // A Wayland window can't bring itself forward (it asks for attention instead), nor
            // hide for the icon in the notification area to bring it back.
            let native_wayland = cfg!(all(unix, not(target_os = "macos")))
                && std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty())
                && display::choose_from_env() != display::Choice::X11;
            let mut app = MailApp::new(
                session,
                backend,
                Box::new(Desktop {
                    data: desktop_data,
                    base,
                    moved,
                    style,
                    #[cfg(windows)]
                    portable: app_portable,
                    updater: updater.clone(),
                    // A reminder answered from its notification wakes the window to act on it.
                    #[cfg(all(unix, not(target_os = "macos")))]
                    waker: Some(Arc::new({
                        let ctx = cc.egui_ctx.clone();
                        move || ctx.request_repaint()
                    })),
                    // The icon clicked wakes the window to come back (on the Inbox for new mail).
                    tray: desktop::tray::Tray::new(
                        Arc::new({
                            let ctx = cc.egui_ctx.clone();
                            move || ctx.request_repaint()
                        }),
                        !native_wayland,
                    ),
                    ..Default::default()
                }),
            );
            if sample {
                app.ui.title_base = "Quatrelle – Sample mailbox".into();
            }
            let (regular, bold) = system_fonts();
            // PDFs (print, save as PDF) embed the same font.
            if let Some(r) = &regular {
                app.session.print_fonts = Some((r.clone(), bold.clone()));
            }
            app.install_fonts(&cc.egui_ctx, regular, bold);
            if let Some(v) = state_path.as_ref().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()) {
                app.restore_state(&v);
            }
            if args.iter().any(|a| a == "--light") {
                app.ui.dark = Some(false);
            } else if args.iter().any(|a| a == "--dark") {
                app.ui.dark = Some(true);
            }
            for c in args.windows(2).filter(|w| w.first().is_some_and(|a| a == "--cmd")).filter_map(|w| w.get(1)) {
                let (id, params) = c.split_once('=').unwrap_or((c.as_str(), "{}"));
                match id {
                    "ui.addAccount" => {
                        let p: serde_json::Value = serde_json::from_str(params).unwrap_or_default();
                        let mut d = quatrelle_ui_egui::Dialog::add_account_with(
                            p.get("email").and_then(|v| v.as_str()).unwrap_or_default(),
                            p.get("advanced").and_then(|v| v.as_bool()).unwrap_or(false),
                        );
                        if let quatrelle_ui_egui::Dialog::AddAccount(f) = &mut d {
                            f.graph = p.get("graph").and_then(|v| v.as_bool()).unwrap_or(false);
                            f.pop = p.get("pop").and_then(|v| v.as_bool()).unwrap_or(false);
                        }
                        app.ui.dialog = Some(d);
                    }
                    "ui.file" | "ui.settings" => app.ui.backstage = Some(Backstage::new(Page::Info)),
                    "ui.options" => app.ui.backstage = Some(Backstage::new(Page::Options)),
                    "ui.openExport" => app.ui.backstage = Some(Backstage::new(Page::Open)),
                    "ui.about" => app.ui.backstage = Some(Backstage::new(Page::About)),
                    // Optionally `={"at":"YYYY-MM-DDTHH:MM","summary":…,"guests":"a@x, …","rooms":"Room <r@x>, …","scheduling":true,"finder":true,"room":"r@x" (its day shown in the Room Finder),"repeat":RRULE,"recurrence":true,"attendees":true (Select Attendees and Resources open)}`.
                    "ui.newEvent" => {
                        let p: serde_json::Value = serde_json::from_str(params).unwrap_or_default();
                        let text = |k: &str| p.get(k).and_then(|v| v.as_str()).map(str::to_string);
                        let at = text("at").and_then(|x| quatrelle_engine::parse_local(&x, &app.session.clock)).map(|(t, _)| t);
                        let mut d = quatrelle_ui_egui::Dialog::new_event(&app.session, at);
                        if let quatrelle_ui_egui::Dialog::Event(f) = &mut d {
                            if let Some(s) = text("summary") {
                                f.summary = s;
                            }
                            if let Some(g) = text("guests") {
                                f.guests = g;
                            }
                            for r in quatrelle_engine::model::addr::parse_list(&text("rooms").unwrap_or_default()).unwrap_or_default() {
                                f.add_room(&r.name, &r.email);
                            }
                            f.scheduling = p.get("scheduling").and_then(|v| v.as_bool()).unwrap_or(false);
                            if p.get("finder").and_then(|v| v.as_bool()).unwrap_or(false) {
                                f.finder = Some(Default::default());
                            }
                            if let Some(r) = text("room") {
                                let shown = (r.split('@').next().unwrap_or_default().to_string(), r);
                                f.finder.get_or_insert_with(Default::default).shown = Some(shown);
                            }
                            if let Some(r) = text("repeat") {
                                f.repeat = r;
                            }
                            if p.get("recurrence").and_then(|v| v.as_bool()).unwrap_or(false) {
                                f.open_recurrence();
                            }
                            if p.get("attendees").and_then(|v| v.as_bool()).unwrap_or(false) {
                                f.inviting = true;
                                f.select_attendees();
                            }
                        }
                        app.ui.dialog = Some(d);
                    }
                    "ui.newContact" => app.ui.dialog = Some(quatrelle_ui_egui::Dialog::new_contact()),
                    "ui.folderSize" => {
                        let p: serde_json::Value = serde_json::from_str(params).unwrap_or_default();
                        let first = app.session.accounts.first().map(|a| a.config.id.clone());
                        if let Some(id) = p.get("account").and_then(|v| v.as_str()).map(str::to_string).or(first) {
                            app.run("account.folderSizes", serde_json::json!({"account": id}));
                            app.ui.backstage = Some(Backstage::new(Page::Info));
                            app.ui.dialog = Some(quatrelle_ui_egui::Dialog::folder_size(&app.session, &id));
                        }
                    }
                    "ui.rules" => app.ui.dialog = Some(quatrelle_ui_egui::Dialog::Rules),
                    _ => match serde_json::from_str(params) {
                        Ok(p) => app.run(id, p),
                        Err(e) => say(&format!("--cmd {id}: bad JSON: {e}")),
                    },
                }
            }
            for h in &handed {
                app.open_handed(h);
            }
            let gpu = cc.wgpu_render_state.as_ref().map(|rs| rs.device.clone());
            if let Some(server) = &instance {
                server.connect(&cc.egui_ctx);
            }
            Ok(Box::new(App {
                app,
                state_path,
                gpu,
                frames: 0,
                instance,
                #[cfg(target_os = "macos")]
                finder: {
                    apple_events::repaint_on_arrival(&finder, &cc.egui_ctx);
                    // ⌘Q asks before what's being written is lost; without the menu it still
                    // runs, quitting as winit's does.
                    if let Err(e) = apple_events::install_menu(&cc.egui_ctx) {
                        log::warn!("the app menu wasn't put in place, so Quit won't ask first: {e}");
                    }
                    finder
                },
                native_wayland,
                updater,
                saved: Vec::new(),
                looked: std::time::Instant::now(),
                screenshot: screenshot::Shot::wanted(),
                pacing,
            }))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The allocator gives freed memory back by itself (Linux on x86-64).
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    fn freed_memory_goes_back_to_the_system() {
        give_back_freed_memory();
        assert_eq!(tikv_jemalloc_ctl::background_thread::read().ok(), Some(true));
        assert_eq!(tikv_jemalloc_ctl::max_background_threads::read().ok(), Some(1));
    }
    use quatrelle_ui_egui::VideoFrame;

    /// eframe is used without its default features, and copy and paste are one of them since
    /// 0.37: the app had no clipboard (nothing pasted anywhere) until it was named.
    #[test]
    fn the_window_has_a_clipboard() {
        let manifest = include_str!("../Cargo.toml");
        let eframe = manifest.lines().find(|l| l.trim_start().starts_with("eframe")).unwrap_or("");
        assert!(eframe.contains("\"clipboard\""), "eframe without its clipboard feature: {eframe}");
    }

    /// A shortcut that names an MSI `<Icon>` shows a blank icon once an update deletes the
    /// installer's cached copy, and a taskbar pin (a copy of the shortcut) is never repaired:
    /// shortcuts take the icon embedded in quatrelle.exe, and package.ps1 refuses an exe without
    /// one. Only Apps and features (`ARPPRODUCTICON`) uses the MSI `<Icon>`.
    #[test]
    fn windows_shortcuts_use_the_icon_embedded_in_the_exe() {
        let wxs = include_str!("../../../packaging/windows/quatrelle.wxs");
        let shortcuts: Vec<&str> = wxs.split("<Shortcut ").skip(1).filter_map(|s| s.split('>').next()).collect();
        assert!(!shortcuts.is_empty(), "quatrelle.wxs has no shortcut");
        for shortcut in shortcuts {
            assert!(!shortcut.contains("Icon="), "a shortcut names an MSI <Icon>: {shortcut}");
            assert!(shortcut.contains("Advertise=\"no\""), "an advertised shortcut shows the MSI's icon: {shortcut}");
        }
        assert!(wxs.contains("<Property Id=\"ARPPRODUCTICON\""), "Apps and features keeps its icon");
        let script = include_str!("../../../packaging/windows/package.ps1");
        assert!(script.contains("ExtractIconEx") && script.contains("has no embedded icon"), "package.ps1 doesn't refuse an exe without an icon");
    }

    /// A window that can't open says why, in plain sentences (no gaps left from the source).
    #[test]
    fn a_window_that_cant_open_says_why() {
        let m = cant_open("no graphics adapter");
        assert!(m.starts_with("Quatrelle's window couldn't open:\nno graphics adapter\n"), "{m}");
        assert!(m.contains("quatrelle.log") && !m.contains("  "), "{m}");
    }

    #[test]
    fn video_thumbnails_are_made_off_the_ui_thread() {
        let Some(ffmpeg) = video::find_tool("ffmpeg", &[]) else { return };
        let dir = std::env::temp_dir().join(format!("quatrelle-desktop-video-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let clip = dir.join("clip.mp4");
        let made = std::process::Command::new(&ffmpeg)
            .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=duration=2:size=320x240:rate=10", "-pix_fmt", "yuv420p"])
            .arg(&clip)
            .status();
        if !made.is_ok_and(|s| s.success()) {
            return;
        }
        // The system's thumbnailer, as when ffmpeg isn't installed.
        let mut d = Desktop { frame_source: Some(video::system()), ..Default::default() };
        let end = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            match d.video_frame(&clip) {
                VideoFrame::Pending if std::time::Instant::now() < end => std::thread::sleep(std::time::Duration::from_millis(50)),
                f => {
                    assert!(matches!(f, VideoFrame::Ready(_)) || video::system().is_none(), "{f:?}");
                    break;
                }
            }
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
