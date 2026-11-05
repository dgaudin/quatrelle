// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Quatrelle's egui front end: the familiar three-pane mail layout (folders, message list,
//! reading pane) with a simplified ribbon, search box, inline compose and account dialogs.
//!
//! The UI is thin: it reads [`Session`] state and changes it through commands (by id), so the
//! toolbar, menus, shortcuts and agents all behave the same. The network sits behind
//! [`Backend`] and platform services (file dialogs, opening files, the clock) behind
//! [`Services`], so nothing here depends on threads, sockets or a windowing crate.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

/// An interface string in the current language (see [`i18n`]).
macro_rules! tl {
    ($s:expr) => {
        $crate::i18n::t($s)
    };
}

/// A translated template with `{name}` values filled in: `tf!("Saved {path}", path = p.display())`.
macro_rules! tf {
    ($s:expr $(, $k:ident = $v:expr)* $(,)?) => {
        $crate::i18n::tfmt($s, &[$((stringify!($k), &$v.to_string())),*])
    };
}

mod app;
mod dialogs;
pub mod i18n;
mod look;
mod panes;
mod reading;
mod rich_edit;

mod services;
mod spellcheck;
mod state;

// Each part by its own name, wherever its file sits (`crate::list`, `quatrelle_ui_egui::theme`).
use dialogs::pim_dialogs;
pub use look::thumbs::VideoFrame;
pub use look::{bidi, craft_fonts, icons, motion, theme};
use look::{thumbs, widgets};
pub use panes::backstage;
use panes::{calendar, list, nav, notes, organize, people, reminders, tasks};

use std::collections::HashMap;
use std::path::PathBuf;

use egui::{Align, Color32, Key, Layout, Margin, RichText, Sense, Ui, vec2};
use quatrelle_engine::model::{MessageKey, SyncStatus};
use quatrelle_engine::{CalMode, Module, ReadingPane, SearchScope, Session};
use serde_json::{Value, json};

pub(crate) use app::{SearchDrop, junk_menu, meeting_browser, meeting_profile, scope_label};
pub use app::{WindowPlace, after_open, alt_code_pages, alt_codes};
pub use dialogs::Dialog;
use icons::Icon;
pub use services::{
    Backend, BrowserProfile, FileAsk, Lane, NullBackend, NullServices, ReminderAction, Services, TrayAction, TrayIcon, UpdateState, Work,
};
pub use state::UiState;
use theme::Tokens;

/// Whether the message being written shows in the reading pane: when it isn't popped out, or
/// where windows of its own can't open (the browser).
/// How the app reads HTML from the clipboard, set once at start (the clipboard is the
/// platform's; a build without one has none).
static CLIPBOARD_HTML: std::sync::OnceLock<fn() -> Option<String>> = std::sync::OnceLock::new();

/// Tell the editor how to read HTML from the clipboard, so text pasted from Outlook, Word or a
/// web page keeps its look.
pub fn set_clipboard_html(read: fn() -> Option<String>) {
    let _ = CLIPBOARD_HTML.set(read);
}

/// HTML on the clipboard (`None` when there is none).
pub(crate) fn clipboard_html() -> Option<String> {
    CLIPBOARD_HTML.get().and_then(|read| read())
}

/// How the app puts HTML on the clipboard (with the text for apps that take text alone), set
/// once at start: Copy and Cut in the editor then keep the look, for Outlook or Word to paste.
static CLIPBOARD_HTML_WRITER: std::sync::OnceLock<fn(&str, &str)> = std::sync::OnceLock::new();

/// Tell the editor how to put HTML and text on the clipboard together.
pub fn set_clipboard_html_writer(write: fn(&str, &str)) {
    let _ = CLIPBOARD_HTML_WRITER.set(write);
}

/// Put `html` on the clipboard, with `text` for apps that take text alone; `false` when the app
/// can't (the caller then copies the text the usual way).
pub(crate) fn write_clipboard_html(html: &str, text: &str) -> bool {
    match CLIPBOARD_HTML_WRITER.get() {
        Some(write) => {
            write(html, text);
            true
        }
        None => false,
    }
}

pub(crate) fn compose_inline(c: &quatrelle_engine::Compose, ctx: &egui::Context) -> bool {
    !c.window || ctx.embed_viewports()
}

/// Put message `id`'s editor at hand (`st.editor`), keeping the one there for later.
pub(crate) fn use_editor(st: &mut UiState, id: u64) {
    if st.editor.as_ref().is_some_and(|e| e.id == id) {
        return;
    }
    if let Some(e) = st.editor.take() {
        st.editors.insert(e.id, e);
    }
    st.editor = st.editors.remove(&id);
}

/// How a ribbon tab's groups fit the window, from the frame before ([`widgets::GroupFit`]).
#[derive(Clone, Debug, PartialEq)]
pub struct RibbonGroups {
    /// Groups from this one on are drawn compact (`usize::MAX`: not known yet, all large).
    pub compact_from: usize,
    /// Each group's column widths when compact.
    pub columns: Vec<Vec<f32>>,
    /// How far past the window's right edge the last group reached in that frame (at most
    /// zero when the ribbon fits).
    pub overflow: f32,
}

impl Default for RibbonGroups {
    /// Not measured yet: every group large.
    fn default() -> Self {
        RibbonGroups { compact_from: usize::MAX, columns: Vec::new(), overflow: 0.0 }
    }
}

impl RibbonGroups {
    /// The fit of a tab drawn this frame, as the tests look at it.
    pub fn of(ui: &UiState, module: Module, tab: RibbonTab) -> Option<&RibbonGroups> {
        ui.ribbon_groups.get(&format!("{module:?}/{tab:?}"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RibbonTab {
    Home,
    SendReceive,
    View,
    /// While searching, as Outlook's Search tab: the scope, terms to narrow it, Close Search.
    Search,
}

/// Something a pane asks for; run after drawing so panes only borrow what they read.
pub enum Action {
    Run(&'static str, Value),
    /// Close Quatrelle (File › Exit), asking first as any close does.
    Quit,
    /// Close Quatrelle though online-only accounts hold unsent mail or changes (issue #4).
    CloseAnyway,
    /// Close, install the downloaded version, and open it.
    RestartToUpdate,
    /// Closing, with the update asked about: installed as Quatrelle closes (true), or kept for
    /// later (false).
    UpdateOnClose(bool),
    Open(MessageKey),
    Dialog(Dialog),
    /// An action from one message's window: done to that message (several can be written at
    /// once, each in a window of its own).
    ForCompose(u64, Box<Action>),
    /// A command from an appointment's window (its number): the window closes once it's done,
    /// or says why it couldn't be.
    ForEvent(u64, &'static str, Value),
    /// A Webex meeting asked for from an appointment's window (its number), with
    /// `webex.createMeeting`'s params: the window waits for the answer, or says why not.
    ForWebex(u64, Value),
    OpenAttachment(usize),
    SaveAttachment(usize),
    AttachFiles,
    /// Attach Item, open on a kind of item.
    AttachItem(quatrelle_engine::ItemKind),
    FocusSearch,
    /// Make a note and write on it.
    NewNote,
    /// Show a task in Tasks (from its reminder).
    OpenTask(String),
    /// Search every folder for this (shown in the box, with the Search tab).
    Search(String),
    SetDark(Option<bool>),
    SetSystemAccent(bool),
    /// Print the open message (as a PDF opened in the system's viewer).
    Print,
    /// Save the open message as a PDF, or as its `.eml` source.
    ExportPdf,
    SaveMessage,
    /// Open an attachment in a Craft app (its command name).
    OpenInCraft(usize, &'static str),
    /// Save every attachment of the open message into a folder.
    SaveAll,
    /// Save some attachments of the open message (by index) into a folder.
    SaveAttachments(Vec<usize>),
    /// The messages selected in the list, as `.eml` files (one where asked; several in a folder).
    SaveSelected,
    /// These messages as `.eml` files (Save As on those selected, once the place is said).
    SaveMessages(Vec<MessageKey>),
    /// The rules to a file, or rules from one (Outlook's Export and Import Rules).
    ExportRules,
    ImportRules,
    /// Pick a `.p12` / `.pfx` file for an account's certificate, then ask its password.
    ImportCertificate(String),
    /// The browser: open the account's certificate it keeps, with its password, for this page.
    UnlockCertificate(String),
    /// Choose a certificate on a smart card (or in the system's keychain) for an account.
    UseSmartCard(String),
    /// Sign an account in again: the browser for OAuth accounts, else the password dialog.
    SignInAgain(String),
    /// Insert a picture at the cursor of the message being written (from a file).
    InsertPicture,
    /// A picture into the signature being made (a logo), from a file.
    SignaturePicture,
    /// The account's signature at the cursor of the message being written.
    InsertSignature,
    /// Paste a picture from the clipboard, if there is one.
    PastePicture,
    /// Put a picture pasted (a screenshot: its PNG) in the message being written.
    PasteImage(Vec<u8>),
    /// Choose the interface language (`auto` or a code).
    SetLanguage(String),
    /// Open the signature editor for an account (`insert`: from a message, putting it in).
    EditSignature {
        account: String,
        insert: bool,
    },
    /// A link in a message was clicked (web pages open in the browser; `mailto:` starts a message;
    /// a network share asks first, unless its server is trusted).
    OpenUrl(String),
    /// A meeting's link was clicked in an event or meeting (its Join button, its notes): a Teams
    /// meeting joins in a browser's app window, as File › Options › Applications says; any other
    /// link opens as [`Action::OpenUrl`] does.
    OpenMeeting(String),
    /// File › Options › Applications' "Create a profile for meetings": the meeting browser opens
    /// Teams in Quatrelle's own profile, which it makes, for the user to sign in to once; once it
    /// has opened, that profile is the one meetings open in.
    MakeMeetingProfile,
    /// A network share's link the user said to open ([`Dialog::OpenShare`]).
    OpenShare(String),
    /// Run a command for the open dialog: it closes on success and shows the error otherwise.
    Submit(&'static str, Value),
    /// Server settings' Save with a domain typed and not added: `account.trustDomain`'s params,
    /// then `account.update`'s. A domain refused leaves the dialog open, saying why, nothing saved.
    SaveWithDomain(Value, Value),
    /// The open dialog's OK once its field's names are settled (Add calendar, Open another mailbox).
    DialogOk,
    /// Bring the main window to the front (after acting from a message window).
    FocusMain,
    /// Open the File page at a page.
    Backstage(backstage::Page),
    /// Open File › Options, scrolled to the Webex card.
    WebexOptions,
    /// Read a typeface the message being written uses (Arial, Calibri…) from the system, so
    /// the editor draws it as it will be read.
    LoadTypeface(String),
    /// An attachment is being dragged out of the window: hand it to the system as a file.
    DragOut(usize),
    /// Choose an Outlook data file and open it.
    OpenDataFile,
    /// Choose an existing data file for the account being added (its mail comes in first).
    PickAccountDataFile,
    /// An action from a message's own window, on that message (its attachments, Save As, Print).
    InWindow(MessageKey, Box<Action>),
    /// Choose where to write an account's mail as an Outlook data file.
    ExportDataFile(String),
    /// Show a folder in the system's file manager.
    OpenFolder(PathBuf),
    /// Choose a folder to move Quatrelle's data into (at the next start).
    MoveDataFolder,
    /// Forget a move asked for.
    CancelDataMove,
    /// Say why something can't be done now, as a command's refusal is said (a message whose
    /// text came first: its attachments are still coming).
    Refuse(String),
}

pub struct MailApp {
    pub session: Session,
    pub backend: Box<dyn Backend>,
    pub services: Box<dyn Services>,
    pub ui: UiState,
}

impl MailApp {
    pub fn new(mut session: Session, backend: Box<dyn Backend>, mut services: Box<dyn Services>) -> MailApp {
        // Mail kept as Outlook's Cached Exchange Mode keeps it, from the start (the saved
        // choice, if any, follows with the rest of the preferences).
        session.apply_sync_prefs();
        // The spelling dictionaries, read once in the background (nothing in the browser), with
        // the words added to them so far.
        spellcheck::start();
        spellcheck::set_personal(session.prefs.spelling.clone());
        let seen = session.notices.len();
        let has_files = !services.downloads();
        let (data_folder, pending_move) = (services.data_folder(), services.pending_data_move());
        let mut session = session;
        // The window reads the address book a slice a frame: the session never reads it whole.
        session.interactive = true;
        // A search's slices keep to a share of each frame, by the window's clock (the engine
        // reads none: std's panics in the browser, web-time's runs there too).
        let started = web_time::Instant::now();
        session.set_search_clock(Some(quatrelle_engine::SearchClock { budget: SEARCH_FRAME_SHARE, now: Box::new(move || started.elapsed()) }));
        session.set_zone(services.zone().as_deref());
        match services.take_data_moved() {
            Some(Ok(to)) => session.notice(tf!("Your mail is now kept in {folder}", folder = to.display().to_string()), false),
            Some(Err(e)) => session.notice(tf!("Your mail couldn't be moved, so it stays where it was: {error}", error = e), true),
            None => {}
        }
        let craft_apps: Vec<(&'static str, PathBuf)> = ["pdfcraft", "wordcraft", "gridcraft", "deckcraft", "photocraft", "filmcraft", "soundcraft"]
            .into_iter()
            .filter_map(|c| services.craft_app(c).map(|p| (c, p)))
            .collect();
        let mut app = MailApp {
            session,
            backend,
            services,
            ui: UiState {
                tokens: Tokens::light(),
                closing_anyway: false,
                close_now: false,
                quitting: false,
                put_away: false,
                close_checked: None,
                update: UpdateState::None,
                update_later: None,
                update_told: None,
                update_answered: false,
                restart_wanted: false,
                close_asked: false,
                reminders: reminders::State::default(),
                has_files,
                browsers: None,
                meeting_profiles: None,
                data_folder,
                pending_move,
                dark: None,
                system_accent: true,
                ribbon_collapsed: false,
                window_theme: None,
                tab: RibbonTab::Home,
                search_text: String::new(),
                focus_search: false,
                search_focused: false,
                search_cursor_end: false,
                search_due: None,
                search_drop: SearchDrop::None,
                compose_alert: None,
                attachment_view: None,
                attachment_click: None,
                attachment_selected: None,
                attachments_active: false,
                attachment_hovered: false,
                ribbon_rect: None,
                ribbon_icons_from: usize::MAX,
                ribbon_groups: HashMap::new(),
                dialog: None,
                pictures: reading::Pictures::default(),
                video_thumbs: HashMap::new(),
                shown_pictures: HashMap::new(),
                cell_heights: reading::CellHeights::default(),
                toast: None,
                seen_notices: seen,
                scroll_to_focus: false,
                list_page: 10,
                raise_source: None,
                source_headers: Default::default(),
                fonts_ready: false,
                logic_ran: false,
                textures_told: 0,
                compose_shown: Default::default(),
                editors: HashMap::new(),
                dialog_for: None,
                files: Default::default(),
                event_windows: Vec::new(),
                event_window_ids: 0,
                editor: None,
                people: Default::default(),
                craft_apps: Vec::new(),
                language: i18n::AUTO.into(),
                fonts: (None, None),
                rtl_font: None,
                emoji_asked: false,
                cjk_loaded: None,
                quoted_in: None,
                han_missing: false,
                title_base: "Quatrelle".into(),
                places: Default::default(),
                page_message: None,
                title_shown: String::new(),
                tray_shown: TrayIcon::default(),
                focus_main: false,
                webex_scroll: std::cell::Cell::new(false),
                video_files: HashMap::new(),
                video_frames: HashMap::new(),
                backstage: None,
                mini_month: None,
                cal_scrolled: [false; 2],
                people_query: String::new(),
                people_book: None,
                list_rows: None,
                want_previews: Vec::new(),
                people_view: people::PeopleView::default(),
                people_list: people::ListSettings::default(),
                list_columns: list::ColumnWidths::default(),
                notes_query: String::new(),
                cal_search: calendar::CalSearch::default(),
                months_drag: 0.0,
                cal_drag: None,
                cal_pick: None,
                note_edit: None,
                tasks: Default::default(),
                note_focus: false,
                person: None,
                invite: Vec::new(),
                drag_out: None,
                can_drag_out: false,
                load_typefaces: Vec::new(),
            },
        };
        app.ui.craft_apps = craft_apps;
        app.ui.can_drag_out = app.services.can_drag_out();
        app
    }

    /// A file the user started dragging out of the window, for the app to hand to the system's
    /// drag and drop (egui can't).
    pub fn take_drag_out(&mut self) -> Option<PathBuf> {
        self.ui.drag_out.take()
    }

    /// Register the UI fonts, preferring an installed system font (call before the first frame).
    pub fn install_fonts(&mut self, ctx: &egui::Context, regular: Option<Vec<u8>>, bold: Option<Vec<u8>>) {
        self.ui.fonts = (regular.clone().map(std::sync::Arc::new), bold.clone().map(std::sync::Arc::new));
        bidi::set_coverage(regular.as_deref());
        if !bidi::covers_rtl() {
            self.ui.rtl_font = self.services.rtl_font();
        }
        theme::install_fonts_with(ctx, regular, bold, Vec::new(), self.ui.rtl_font.clone());
        // Emoji as pictures, from the system's colour emoji font (kept by egui across font changes).
        if !std::mem::replace(&mut self.ui.emoji_asked, true) {
            look::emoji::install(ctx, &mut *self.services);
        }
        self.ui.fonts_ready = true;
    }

    /// Draw in the chosen language. Chinese, Japanese and Korean fonts are always there as
    /// fallbacks (the language menu names them in their own script, and mail may be in them),
    /// the chosen language's own first: they draw shared characters differently.
    fn apply_language(&mut self, ctx: &egui::Context) {
        let lang = i18n::Lang::from_pref(&self.ui.language);
        i18n::set_current(lang);
        self.sync_quote_words();
        let first = if lang.cjk() { lang.code() } else { "zh-hans" };
        // Shown text with Chinese characters no font has (the browser has only the fonts it
        // brought; Simplified Chinese's is too large to bring, so it's fetched when wanted).
        if std::mem::take(&mut self.ui.han_missing) {
            self.services.want_font("zh-hans");
            self.ui.cjk_loaded = None;
        }
        if lang.code() == "zh-hans" {
            self.services.want_font("zh-hans");
        }
        if self.ui.cjk_loaded != Some(first) {
            self.ui.cjk_loaded = Some(first);
            // Korean has its own font; Simplified Chinese's covers Japanese kana too.
            let second = if first == "ko" { "zh-hans" } else { "ko" };
            let mut cjk: Vec<(Vec<u8>, u32)> = Vec::new();
            for code in [first, second] {
                if let Some(f) = self.services.cjk_font(code)
                    && !cjk.iter().any(|(b, i)| *i == f.1 && b.len() == f.0.len())
                {
                    cjk.push(f);
                }
            }
            if !cjk.is_empty() {
                let (r, b) = (self.ui.fonts.0.as_ref().map(|f| f.to_vec()), self.ui.fonts.1.as_ref().map(|f| f.to_vec()));
                theme::install_fonts_with(ctx, r, b, cjk, self.ui.rtl_font.clone());
                ctx.request_repaint();
            } else if self.services.font_pending() {
                // Asked again once it has come.
                self.ui.cjk_loaded = None;
            } else if lang.cjk() && !(craft_fonts::has_script("Jpan") || craft_fonts::has_script("Hans")) {
                // (Otherwise the built-in fallbacks, installed with the other fonts, draw them.)
                log::warn!("no Chinese/Japanese/Korean font found: those characters can't be drawn");
            }
        }
    }

    /// Replies quote their original's header in the interface's language (commands given at
    /// start-up run before the first frame sets it).
    fn sync_quote_words(&mut self) {
        let lang = i18n::Lang::from_pref(&self.ui.language);
        if self.ui.quoted_in.replace(lang.code()) != Some(lang.code()) {
            self.session.quote_words = i18n::quote_words(lang);
            self.session.conflict_words = i18n::conflict_words(lang);
        }
    }

    /// Run a command, reporting failures in a toast. Send first waits for the names its fields
    /// are still resolving ([`dialogs::people_field::PeopleFields::gate_send`]; `logic` runs it
    /// once they're settled, or says why it can't go).
    pub fn run(&mut self, id: &str, params: Value) {
        if id == "compose.send" {
            let now = self.ui.close_checked.unwrap_or(0.0);
            if self.ui.people.gate_send(&mut self.session, &params, now) {
                return;
            }
        }
        // Everyone put in Bcc sets To and Cc anew, which would forget what a field left out.
        if id == "compose.hideRecipients" && self.session.compose.as_ref().is_some_and(|c| self.ui.people.has_losses(c.id)) {
            self.ui.compose_alert = Some(dialogs::people_field::left_out_stop());
            return;
        }
        self.run_now(id, params);
    }

    /// Run a command now, whatever its fields are still resolving.
    pub(crate) fn run_now(&mut self, id: &str, params: Value) {
        use quatrelle_engine::CmdError;
        self.sync_quote_words();
        match self.session.run(id, &params) {
            // Held to be looked at first: said in the message's window when it has one.
            Ok(v) if id == "compose.send" && v.get("check").is_some() => {
                let found = serde_json::from_value(v["check"].clone()).unwrap_or_default();
                self.ui.dialog_for = self.session.compose.as_ref().filter(|c| c.window).map(|c| c.id);
                self.ui.dialog = Some(Dialog::SendCheck(found));
            }
            Ok(_) => {}
            // Not from an account that can send: said over the message, as why it didn't go.
            Err(CmdError::Disabled(_))
                if id == "compose.send"
                    && let Some(b) = self.session.compose_send_block() =>
            {
                self.ui.compose_alert = Some(reading::send_block_text(b));
            }
            // Not possible now (a button that does nothing to do): logged, never shown.
            Err(CmdError::Disabled(m)) => log::info!("{id}: {m}"),
            // What went wrong with the message being written is said over it, in its own
            // window if it has one, as Outlook does (not in the main window's corner).
            Err(CmdError::Params(m) | CmdError::Failed(m)) if id.starts_with("compose.") && self.session.compose.is_some() => {
                self.ui.compose_alert = Some(m);
            }
            Err(CmdError::Params(m) | CmdError::Failed(m)) => self.session.notice(m, true),
            Err(e) => self.session.notice(e.to_string(), true),
        }
    }

    /// A `mailto:` link or a mail file the system handed Quatrelle (as its mail app, or through
    /// Open with): a new message to write, the message shown, or the events or people added and
    /// shown. Nothing is sent and nothing is attached.
    pub fn open_handed(&mut self, what: &str) {
        use quatrelle_engine::CmdError;
        let mailto = quatrelle_engine::model::mailto::is_mailto(what);
        let r = if mailto { self.session.run("mail.mailto", &json!({"url": what})) } else { self.session.run("file.open", &json!({"path": what})) };
        let name = what.rsplit(['/', '\\']).next().unwrap_or(what).to_string();
        // How many the answer says were added or left out (none when it doesn't say).
        let tally = |answer: &Value, field: &str| answer[field].as_u64().unwrap_or(0);
        match r {
            Ok(_) if mailto => self.ui.backstage = None,
            Ok(v) => {
                self.ui.backstage = None;
                let left_out = tally(&v, "skipped");
                let left_out =
                    if left_out > 0 { format!(" {}", tf!("Left out because they couldn't be read: {n}", n = left_out)) } else { String::new() };
                match v.get("kind").and_then(Value::as_str) {
                    Some("calendar") => {
                        self.run("view.calendar", json!({}));
                        let added = tf!("Added to your calendar from {file}: {n}", file = name, n = tally(&v, "added"));
                        self.session.notice(format!("{added}{left_out}"), false);
                    }
                    Some("contacts") => {
                        self.run("view.people", json!({}));
                        let added = tf!("Added to your contacts from {file}: {n}", file = name, n = tally(&v, "added"));
                        self.session.notice(format!("{added}{left_out}"), false);
                    }
                    _ => self.ui.dialog = Some(Dialog::preview(&v, &self.session.clock)),
                }
            }
            Err(CmdError::Disabled(_)) if mailto => {
                self.session.notice(tl!("Add an account first: a new message needs an account to send from.").to_string(), true)
            }
            Err(CmdError::Disabled(_)) => {
                self.session.notice(tl!("Add an account first: events and people are kept with an account.").to_string(), true)
            }
            Err(e) => self.session.notice(tf!("Couldn't open {file}: {error}", file = name, error = e.to_string()), true),
        }
    }

    /// Take in what the workers sent; `true` when a server-side search was answered (the list
    /// changed, so the frame is drawn again at once rather than at the next idle tick).
    fn pump(&mut self) -> bool {
        let (now, offset) = self.services.clock();
        let before = self.session.clock.offset;
        self.session.set_clock(now, offset);
        // The offset changed (summer time, or the system's zone did): which zone it is, again.
        if offset != before {
            self.session.set_zone(self.services.zone().as_deref());
        }
        let mut answered = false;
        let events = self.backend.poll();
        // A folder changed several times since the last look (a sync saves a folder more than
        // once, Download all after each message) is read again once, for its last change: each
        // was a reload of the whole folder on the window's thread.
        let mut last: HashMap<(&str, &str), usize> = HashMap::new();
        for (i, e) in events.iter().enumerate() {
            if let quatrelle_engine::model::Event::FolderChanged { account, folder } = e {
                last.insert((account.as_str(), folder.as_str()), i);
            }
        }
        let skip: std::collections::HashSet<usize> = events
            .iter()
            .enumerate()
            .filter_map(|(i, e)| match e {
                quatrelle_engine::model::Event::FolderChanged { account, folder } => {
                    (last.get(&(account.as_str(), folder.as_str())) != Some(&i)).then_some(i)
                }
                _ => None,
            })
            .collect();
        drop(last);
        for (i, e) in events.into_iter().enumerate() {
            if skip.contains(&i) {
                continue;
            }
            answered |= matches!(e, quatrelle_engine::model::Event::Found { .. });
            let merging = app::Merging::of(&e);
            self.session.apply(e);
            if let Some(m) = merging {
                m.done();
            }
        }
        // Sign-in pages go to the browser.
        for url in std::mem::take(&mut self.session.open_urls) {
            if let Err(e) = self.services.open_url(&url) {
                self.session.notice(tf!("The sign-in page couldn't be opened: {error}", error = e), true);
            }
        }
        // Files chosen in the browser's picker.
        for (purpose, name, data) in self.services.take_picked() {
            if let Some(account) = purpose.strip_prefix("certificate:") {
                self.ui.dialog = Some(Dialog::Certificate { account: account.to_string(), path: name, data: Some(data), password: String::new() });
            } else if purpose == "picture" {
                let size = rich_edit::own_size(&data);
                self.insert_picture(json!({"name": name, "data": data}), size);
            } else {
                self.run("compose.attach", json!({"name": name, "data": data}));
            }
        }
        // Snoozed mail that is due comes back.
        self.session.tick();
        answered
    }

    fn flush_jobs(&mut self) {
        for j in self.session.take_jobs() {
            self.backend.submit(j);
        }
    }

    /// Preferences and theme for the app to save.
    pub fn saved_state(&self) -> Value {
        json!({ "prefs": self.session.prefs, "dark": self.ui.dark, "systemAccent": self.ui.system_accent, "ribbonCollapsed": self.ui.ribbon_collapsed, "language": self.ui.language, "windows": self.ui.places, "peopleView": self.ui.people_view, "peopleList": self.ui.people_list, "listColumns": self.ui.list_columns, "envelope": self.session.envelope_keys() })
    }

    pub fn restore_state(&mut self, v: &Value) {
        if let Some(p) = v.get("prefs").and_then(|p| serde_json::from_value::<quatrelle_engine::Prefs>(p.clone()).ok()) {
            let offline = p.work_offline;
            self.session.prefs = p;
            self.session.prefs.carry_over_webex_rooms();
            self.session.follow_public_favorites();
            spellcheck::set_personal(self.session.prefs.spelling.clone());
            self.session.note_new_calendars();
            self.session.refresh_list();
            // How much mail to keep, before any account connects.
            self.session.apply_sync_prefs();
            // Still working offline from last time: before any account connects.
            if offline {
                self.session.set_work_offline(true);
            }
        }
        // New mail still unread as Quatrelle closed: the envelope shows again.
        if let Some(keys) = v.get("envelope").and_then(|k| serde_json::from_value(k.clone()).ok()) {
            self.session.restore_envelope(keys);
        }
        self.ui.dark = v.get("dark").and_then(Value::as_bool);
        self.ui.system_accent = v.get("systemAccent").and_then(Value::as_bool).unwrap_or(true);
        self.ui.ribbon_collapsed = v.get("ribbonCollapsed").and_then(Value::as_bool).unwrap_or(false);
        if let Some(pv) = v.get("peopleView").and_then(|w| serde_json::from_value(w.clone()).ok()) {
            self.ui.people_view = pv;
        }
        if let Some(c) = v.get("listColumns").and_then(|w| serde_json::from_value::<list::ColumnWidths>(w.clone()).ok()) {
            self.ui.list_columns = c.clamped();
        }
        if let Some(pl) = v.get("peopleList").and_then(|w| serde_json::from_value(w.clone()).ok()) {
            self.ui.people_list = pl;
        }
        if let Some(w) = v.get("windows").and_then(|w| serde_json::from_value(w.clone()).ok()) {
            self.ui.places = w;
        }
        if let Some(l) = v.get("language").and_then(Value::as_str).filter(|l| *l == i18n::AUTO || i18n::Lang::from_code(l).is_some()) {
            self.ui.language = l.to_string();
        }
    }
}

/// The list's title in the interface language.
pub(crate) fn view_title(s: &Session) -> String {
    if let Some(name) = s.open_search_folder_name() {
        return name.to_string();
    }
    match &s.search {
        Some((q, _)) => tf!("Results for “{query}”", query = q),
        None => i18n::tc("folder", &s.view_title()).to_string(),
    }
}

/// Whether this is the first frame of a window: its place is asked for once, at its opening, and
/// left to the window manager from then on. A place handed in at every frame made a window the
/// window manager had just placed jump between the two places, frame after frame (GNOME put a
/// compose window at one spot, the app asked for another, and the window flashed between them).
fn first_frame(ctx: &egui::Context, viewport: egui::ViewportId) -> bool {
    let id = egui::Id::unique(("first-frame", viewport));
    let seen = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    if !seen {
        ctx.data_mut(|d| d.insert_temp(id, true));
    }
    !seen
}

/// `QUATRELLE_TRACE_WINDOWS=1`: what a window of the app says about itself — its outer and inner
/// rectangles, its monitor, its scale — logged each time it changes, with the window's id, so a
/// window that the window manager and the app seem to fight over (one that jumps between two
/// places, flashes, or comes in two) can be followed in `quatrelle.log` frame by frame.
pub fn trace_window(ui: &Ui, what: &str) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*ON.get_or_init(|| std::env::var_os("QUATRELLE_TRACE_WINDOWS").is_some_and(|v| !v.is_empty())) {
        return;
    }
    let ctx = ui.ctx();
    let id = ctx.viewport_id();
    let v = ui.input(|i| i.viewport().clone());
    let rect = |r: Option<egui::Rect>| r.map(|r| format!("{:.0},{:.0} {:.0}x{:.0}", r.min.x, r.min.y, r.width(), r.height()));
    let say = format!(
        "outer {} inner {} monitor {:?} scale {:.2} maximized {:?} minimized {:?} focused {:?}",
        rect(v.outer_rect).unwrap_or_else(|| "-".into()),
        rect(v.inner_rect).unwrap_or_else(|| "-".into()),
        v.monitor_size.map(|m| format!("{:.0}x{:.0}", m.x, m.y)),
        v.native_pixels_per_point.unwrap_or(1.0),
        v.maximized,
        v.minimized,
        v.focused,
    );
    let key = egui::Id::unique(("trace-window", id));
    let last: Option<String> = ctx.data(|d| d.get_temp(key));
    if last.as_deref() != Some(say.as_str()) {
        log::info!("window {what} {id:?}: {say}");
        ctx.data_mut(|d| d.insert_temp(key, say));
    }
}

/// Messages the search index adds per call at most: the window indexes a slice each frame and
/// draws between them, so a big mailbox's first search never freezes it. What binds is the time
/// ([`SEARCH_FRAME_SHARE`]); this only caps a frame of quick ones.
const SEARCH_INDEX_BUDGET: usize = 4096;

/// The share of a frame a search's work takes (the rest draws it), by the window's clock: a
/// message is begun only while one as long as the frame's longest so far still fits. Counted in
/// messages (500 a frame), a frame that met long downloaded bodies took half a second.
const SEARCH_FRAME_SHARE: web_time::Duration = web_time::Duration::from_millis(6);

/// Messages the address book (recipient suggestions, Select Names, the Focused Inbox) takes in
/// per frame while it is built, the window drawing between slices: counting a summary is a
/// fraction of a microsecond, reading a folder from disk the slice's real cost, so a slice is
/// a folder or so (a 550,000-message mailbox is built in a hundred frames, a second or two).
const CONTACTS_BUDGET: usize = 5_000;

#[cfg(test)]
mod tests;
