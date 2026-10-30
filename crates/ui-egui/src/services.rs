//! What the window needs from where it runs: the network side (`Backend`) and the platform's services.

use std::path::PathBuf;

use quatrelle_engine::model::{Event, Job};

use crate::VideoFrame;
use crate::theme;

/// The network side: the desktop app plugs in `quatrelle-net`'s worker pool.
pub trait Backend {
    fn submit(&mut self, job: Job);
    fn poll(&mut self) -> Vec<Event>;
}

/// No network (the sample mailbox, tests).
pub struct NullBackend;

impl Backend for NullBackend {
    fn submit(&mut self, _job: Job) {}
    fn poll(&mut self) -> Vec<Event> {
        Vec::new()
    }
}

/// What was done on a reminder's desktop notification ([`Services::notify_reminder`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReminderAction {
    /// Its "Snooze" button: five minutes more.
    Snooze,
    /// Its "Dismiss" button.
    Dismiss,
    /// The notification itself was clicked: bring the window forward, the reminder with it.
    Open,
}

/// Quatrelle's icon in the notification area, as [`Services::show_tray`] is told to show it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrayIcon {
    /// Shown at all: always while closing the window leaves Quatrelle running, or while new mail
    /// is unread.
    pub shown: bool,
    /// New messages unread: the icon carries the red dot (the envelope).
    pub new: usize,
    /// What hovering it says under Quatrelle's name (empty: nothing more).
    pub tip: String,
    /// Its menu: bring the window back, and quit.
    pub open: String,
    pub quit: String,
}

/// A profile of a meeting browser ([`Services::browser_profiles`]): each keeps its own sign-ins.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BrowserProfile {
    /// Its folder in the browser's data (`Default`, `Profile 1`…): what the browser is told.
    pub dir: String,
    /// The name the browser shows for it.
    pub name: String,
    /// The Google account signed in to it (empty: none).
    pub account: String,
}

/// What was done with the icon in the notification area ([`Services::take_tray_action`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    /// Clicked, or its menu's Open: the window comes back (on the Inbox, for new mail).
    Open,
    /// Its menu's Exit: Quatrelle closes, asking first as File › Exit does.
    Quit,
}

/// Where a build that updates itself is with the next version (`apps/quatrelle/src/self_update.rs`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum UpdateState {
    #[default]
    None,
    /// Being downloaded: the version (`0.25.0`), the bytes so far and the size (0 if unknown).
    Downloading { version: String, done: u64, total: u64 },
    /// Downloaded and checked: installed as Quatrelle closes. `admin`: Windows asks for an
    /// administrator (the MSI installs for everyone).
    Ready { version: String, admin: bool },
}

impl UpdateState {
    /// The version waiting to be installed.
    pub fn ready(&self) -> Option<&str> {
        match self {
            UpdateState::Ready { version, .. } => Some(version),
            _ => None,
        }
    }
}

/// Platform services the UI needs.
pub trait Services {
    /// Unix seconds and the local UTC offset in seconds.
    fn clock(&self) -> (i64, i32);
    /// The local time zone's IANA name (`Europe/Berlin`), when the system says.
    fn zone(&self) -> Option<String> {
        None
    }
    /// Files to attach (an empty list when cancelled).
    fn pick_files(&mut self) -> Vec<PathBuf>;
    /// Where to save a file (`None` when cancelled).
    fn save_path(&mut self, suggested: &str) -> Option<PathBuf>;
    /// Open a file with the system's default app.
    fn open_path(&mut self, path: &std::path::Path) -> Result<(), String>;
    /// Where a build that updates itself is with the next version. The default: nowhere.
    fn update_state(&mut self) -> UpdateState {
        UpdateState::None
    }
    /// Install the waiting version as Quatrelle closes, then open it. False when there's none.
    fn restart_to_update(&mut self) -> bool {
        false
    }
    /// Leave the waiting version for later: not installed as Quatrelle closes this time.
    fn keep_update_for_later(&mut self) {}
    /// Show a desktop notification (new mail). The default does nothing.
    fn notify(&mut self, _title: &str, _body: &str) {}
    /// Tell of a calendar reminder (`key`: the reminder's, for answering it): a notification
    /// that stays until answered, with `snooze` and `dismiss` buttons where the desktop has
    /// them; what was pressed comes back through [`Services::take_reminder_actions`]. The
    /// default is a plain notification.
    fn notify_reminder(&mut self, _key: &str, title: &str, body: &str, _snooze: &str, _dismiss: &str) {
        self.notify(title, body);
    }
    /// What was done on reminder notifications since the last call: (the reminder's key, what).
    fn take_reminder_actions(&mut self) -> Vec<(String, ReminderAction)> {
        Vec::new()
    }
    /// Take back a reminder's notification: it was answered elsewhere (the Reminders window,
    /// another notification) or its time has passed, so its buttons have nothing left to do.
    /// The default does nothing.
    fn withdraw_reminder(&mut self, _key: &str) {}
    /// Show Quatrelle's icon in the notification area as `icon` says, or take it away. The
    /// default does nothing.
    fn show_tray(&mut self, _icon: &TrayIcon) {}
    /// What was done with the icon since the last call.
    fn take_tray_action(&mut self) -> Option<TrayAction> {
        None
    }
    /// Whether the icon is in a notification area now, there to bring the window back: closing
    /// the window then leaves Quatrelle running. The default: no.
    fn tray_shown(&self) -> bool {
        false
    }
    /// Where an installed Craft app is (`pdfcraft`, `wordcraft`…), if it is.
    fn craft_app(&mut self, _command: &str) -> Option<PathBuf> {
        None
    }
    /// Open a file with a given app.
    fn open_with(&mut self, _app: &std::path::Path, _file: &std::path::Path) -> Result<(), String> {
        Err("not available".into())
    }
    /// A rules file to import (`None` when cancelled, or where there's no file picker).
    fn pick_rules_file(&mut self) -> Option<PathBuf> {
        None
    }
    /// A certificate file (`.p12`, `.pfx`) to import (`None` when cancelled, or where there's no
    /// file picker).
    fn pick_certificate(&mut self) -> Option<PathBuf> {
        None
    }
    /// A folder to save into (`None` when cancelled).
    fn pick_folder(&mut self) -> Option<PathBuf> {
        None
    }
    /// An installed font for a Chinese, Japanese or Korean interface language (`ja`, `ko`,
    /// `zh-hans`, `zh-hant`): its bytes and the index in a collection.
    fn cjk_font(&mut self, _lang: &str) -> Option<(Vec<u8>, u32)> {
        None
    }
    /// A Chinese, Japanese or Korean font is wanted now (the interface is in that language, or
    /// shown text has characters no font has): where fonts are downloaded (the browser), fetch it.
    fn want_font(&mut self, _lang: &str) {}
    /// A font asked for is still on its way ([`Services::cjk_font`] has it later).
    fn font_pending(&self) -> bool {
        false
    }
    /// An installed font with Arabic and Hebrew, for when the main font hasn't them.
    fn rtl_font(&mut self) -> Option<Vec<u8>> {
        None
    }
    /// The system's colour emoji font (Noto Color Emoji, Segoe UI Emoji, Apple Color Emoji):
    /// its bytes and the face index in a collection, for emoji drawn as the pictures they are
    /// (`look/emoji.rs`). Asked once. `None` when there's none: emoji stay line drawings.
    fn emoji_font(&mut self) -> Option<(Vec<u8>, u32)> {
        None
    }
    /// An installed typeface by name (`Arial`, `Calibri`), for the message editor to draw text
    /// set in it: its regular file, and its bold when there is one. `None` when the system
    /// hasn't it (nor, on Linux, a font standing in for it).
    fn typeface(&mut self, _family: &str) -> Option<(Vec<u8>, Option<Vec<u8>>)> {
        None
    }
    /// A picture on the clipboard, as PNG bytes (`None` when there is none).
    fn clipboard_picture(&mut self) -> Option<Vec<u8>> {
        None
    }
    /// Pictures to insert (an empty list when cancelled). The default offers the file picker.
    fn pick_pictures(&mut self) -> Vec<PathBuf> {
        self.pick_files()
    }
    /// Open a web address in the browser. The default can't.
    fn open_url(&mut self, _url: &str) -> Result<(), String> {
        Err("not available".into())
    }
    /// The Chromium-family browsers installed, for meetings: (the name shown, the program).
    fn browsers(&mut self) -> Vec<(String, String)> {
        Vec::new()
    }
    /// The profiles of the browser `program` (one of [`Self::browsers`]), as it lists them. The
    /// default has none.
    fn browser_profiles(&mut self, _program: &str) -> Vec<BrowserProfile> {
        Vec::new()
    }
    /// Open a web address in a browser `program` as an app window of its own, in its `profile`
    /// folder (`None`: the one the browser used last). The default can't.
    fn open_app_window(&mut self, _program: &str, _profile: Option<&str>, _url: &str) -> Result<(), String> {
        Err("not available".into())
    }
    /// Open a message (its key) in a browser window of its own, as web mail does; `false` when windows are
    /// windows here.
    fn open_message_window(&mut self, _key: &str) -> bool {
        false
    }
    /// Close this page (a message's own window, when it's done).
    fn close_page(&mut self) {}
    /// An Outlook data file to open (`None` when cancelled).
    fn pick_data_file(&mut self) -> Option<PathBuf> {
        None
    }
    /// The folder Quatrelle keeps its data in, and whether it can be moved from here (not when
    /// it was chosen at start, or in the browser).
    fn data_folder(&self) -> Option<(PathBuf, bool)> {
        None
    }
    /// How a move carried out at this start went: the new folder, or why nothing moved.
    fn take_data_moved(&mut self) -> Option<Result<PathBuf, String>> {
        None
    }
    /// Where the data moves when Quatrelle next starts, if a move was asked for.
    fn pending_data_move(&self) -> Option<PathBuf> {
        None
    }
    /// Ask for the data to move into `to` when Quatrelle next starts (`None` forgets the move);
    /// the folder it will be in.
    fn move_data_folder(&mut self, _to: Option<&std::path::Path>) -> Result<Option<PathBuf>, String> {
        Err("not available".into())
    }
    /// A frame of a video file for its thumbnail. Called each frame until it isn't
    /// [`VideoFrame::Pending`], so slow work belongs on another thread.
    fn video_frame(&mut self, _path: &std::path::Path) -> VideoFrame {
        VideoFrame::Unavailable
    }
    /// Whether files can be dragged out of the window to other apps (see
    /// [`MailApp::take_drag_out`]).
    fn can_drag_out(&self) -> bool {
        false
    }
    /// Whether files are handed over as downloads (the browser, which has no files) instead of
    /// saved through [`Services::save_path`].
    fn downloads(&self) -> bool {
        false
    }
    /// Hand `bytes` to the user as a download named `name`.
    fn download(&mut self, _name: &str, _bytes: &[u8]) {}
    /// Start choosing files without waiting (the browser): `purpose` is `attach` or `picture`,
    /// and the files arrive through [`Services::take_picked`]. `false` (the default) means
    /// [`Services::pick_files`] answers at once.
    fn pick_later(&mut self, _purpose: &str) -> bool {
        false
    }
    /// Files chosen since the last call: (purpose, name, bytes).
    fn take_picked(&mut self) -> Vec<(String, String, Vec<u8>)> {
        Vec::new()
    }
    /// The desktop's light or dark style and accent colour, as last read (called every frame,
    /// so reading them belongs on another thread). The default says nothing.
    fn system_style(&self) -> theme::SystemStyle {
        theme::SystemStyle::default()
    }
}

/// Services for tests: a fixed clock, no dialogs.
pub struct NullServices(pub i64);

impl Services for NullServices {
    fn clock(&self) -> (i64, i32) {
        (self.0, 0)
    }
    fn pick_files(&mut self) -> Vec<PathBuf> {
        Vec::new()
    }
    fn save_path(&mut self, _: &str) -> Option<PathBuf> {
        None
    }
    fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
        Err("not available".into())
    }
}
