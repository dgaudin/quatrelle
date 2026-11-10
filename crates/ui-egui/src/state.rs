// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The window's own state, beside the session's: what the panes, dialogs and windows keep
//! between frames.

use super::*;

/// UI-only state.
pub struct UiState {
    /// Where the main window, the last message window and the last compose window were.
    pub places: std::collections::BTreeMap<String, WindowPlace>,
    /// Closing was confirmed though it loses online-only accounts' unsent mail (issue #4); and
    /// the window is to close on the next frame.
    pub closing_anyway: bool,
    /// The window's GPU device is gone: nothing is drawn any more, and Quatrelle closes
    /// ([`MailApp::graphics_lost`]).
    pub graphics_lost: bool,
    pub close_now: bool,
    /// The window was closed while Compact Now had a data file to itself: it closes once that's
    /// done (stopped then, the file would be gone through whole at the next start).
    pub(crate) close_after_compact: bool,
    /// The close asked for quits (File › Exit, the icon's Exit, a script): it goes ahead rather
    /// than leaving Quatrelle in the notification area.
    pub quitting: bool,
    /// The main window was put away in the notification area (its close button): it comes back
    /// for a question as Quatrelle quits.
    pub(crate) put_away: bool,
    /// When the close was last decided (the input's time): [`MailApp::logic`] decides it, and
    /// `ui` in the same pass doesn't again.
    pub(crate) close_checked: Option<f64>,
    /// Where the updater is, from [`Services::update_state`].
    pub update: UpdateState,
    /// The ready version whose bar was put away (Later), and the one the system was told of.
    pub(crate) update_later: Option<String>,
    pub(crate) update_told: Option<String>,
    /// What to do with the update as Quatrelle closes was answered: closing asks no more.
    pub(crate) update_answered: bool,
    /// Restart now was chosen: the new version opens once installed, if the close goes ahead.
    pub(crate) restart_wanted: bool,
    /// A question about closing is (or was) on screen; put away, the close is off.
    pub(crate) close_asked: bool,
    /// Calendar reminders come due ([`reminders`]).
    pub reminders: reminders::State,
    pub tokens: Tokens,
    /// The platform has files (temporary ones for other apps): not in the browser.
    pub has_files: bool,
    /// The browsers Teams meetings can open in (shown name, program), asked of the system as the
    /// File page opens (its Options say which one), and forgotten as it closes.
    pub(crate) browsers: Option<Vec<(String, String)>>,
    /// The profiles of the browser chosen for meetings (its program, and them), read as the File
    /// page opens and again when another browser is chosen; forgotten as it closes.
    pub(crate) meeting_profiles: Option<(String, Vec<BrowserProfile>)>,
    /// Where the data is kept and whether it can be moved; where it moves at the next start.
    pub data_folder: Option<(PathBuf, bool)>,
    pub pending_move: Option<PathBuf>,
    /// `None` follows the system.
    pub dark: Option<bool>,
    /// The accent colour is the desktop's (when it has one) rather than Quatrelle's.
    pub system_accent: bool,
    /// The ribbon folded away to its tabs (Ctrl+F1, or its ⌃), as Outlook's.
    pub ribbon_collapsed: bool,
    /// The light or dark style last asked of the window's title bar (`None` inside: the system's).
    pub(super) window_theme: Option<Option<bool>>,
    pub tab: RibbonTab,
    pub search_text: String,
    pub(super) focus_search: bool,
    /// The search box has the keyboard focus (the Search tab stays while it does).
    pub(super) search_focused: bool,
    /// Put the search box's cursor at the end when it next takes the focus.
    pub(super) search_cursor_end: bool,
    /// When to search for what's typed (a moment after the last key, so typing stays quick).
    pub(super) search_due: Option<f64>,
    /// The search box's recent searches or advanced search, when open.
    pub(super) search_drop: SearchDrop,
    /// Why the message being written couldn't be sent or saved, shown over it until dismissed.
    pub compose_alert: Option<String>,
    /// The attachment shown in place of its message's text (a click on its chip), as Outlook
    /// previews attachments in the reading pane.
    pub attachment_view: Option<(MessageKey, usize)>,
    /// The messages shown formatted or as plain text for themselves, not by the setting.
    pub(crate) shown_as: crate::reading::ShownAs,
    /// The preview a plain click on a chip waits to show, and when: once the double-click time
    /// is out, as a double-click only opens the file (a click's preview showed behind it).
    pub attachment_click: Option<(MessageKey, usize, f64)>,
    /// The attachments selected in a message's strip (a click picks one, Ctrl+click adds or
    /// removes one, Ctrl+A takes them all), and whether the strip has the keyboard: it was the
    /// last thing clicked, so Ctrl+A selects attachments rather than mail.
    pub attachment_selected: Option<(MessageKey, Vec<usize>)>,
    pub attachments_active: bool,
    /// The pointer was over an attachment chip in the last frame drawn.
    pub attachment_hovered: bool,
    /// Where the ribbon was last drawn (a click on the Search tab's buttons takes the focus
    /// from the box, and mustn't close the tab).
    pub(super) ribbon_rect: Option<egui::Rect>,
    /// The ribbon's buttons from this one on go without their labels: it didn't fit the window
    /// ([`widgets::RibbonFit`]).
    pub(super) ribbon_icons_from: usize,
    /// The ribbon's groups as they fit the window, per module and tab (`"Mail/Home"`): which
    /// are drawn compact, and their columns ([`widgets::GroupFit`]).
    pub(super) ribbon_groups: HashMap<String, RibbonGroups>,
    pub dialog: Option<Dialog>,
    /// The pictures of the messages shown: attachment thumbnails, pictures in HTML, a picture or
    /// a PDF's pages at the pane's size ([`reading::Pictures`]).
    pub(crate) pictures: reading::Pictures,
    /// Video attachments' thumbnails, and the frame each was last shown in
    /// (`reading/attachments.rs`).
    pub(super) video_thumbs: HashMap<(MessageKey, usize), (Option<egui::TextureHandle>, u64)>,
    /// Web and carried pictures as shown (texture, own size), by address, and the frame each was
    /// last shown in (the oldest go when there are too many).
    pub(super) shown_pictures: HashMap<String, (Option<(egui::TextureHandle, egui::Vec2)>, u64)>,
    /// The heights of HTML messages' table cells as last drawn, for their rows' alignment.
    pub(super) cell_heights: reading::CellHeights,
    /// The notice on show: its words, whether it is an error, when it appeared, and whether Undo is on offer.
    pub toast: Option<(String, bool, f64, bool)>,
    pub(super) seen_notices: usize,
    pub(super) scroll_to_focus: bool,
    /// A text field on the File page had the keyboard at the end of the last frame: Esc leaves it
    /// rather than the page (egui takes the focus away on Esc before the shortcuts see it).
    pub(super) backstage_typing: bool,
    /// How many message rows the list shows at once: a page, for Page Up and Page Down.
    pub(super) list_page: u64,
    /// A message source's window to bring to the front (asked for again while open).
    pub(super) raise_source: Option<MessageKey>,
    /// The message sources shown as their header only (by key).
    pub(super) source_headers: std::collections::HashSet<MessageKey>,
    /// The named font families the UI uses are registered (`install_fonts`).
    pub(super) fonts_ready: bool,
    /// [`MailApp::logic`] ran for this frame already (the app calls it before each `ui`; a
    /// caller that only draws, the tests', gets it from `ui`).
    pub(super) logic_ran: bool,
    /// When the textures were last told in the log (session clock, seconds).
    pub(crate) textures_told: i64,
    /// The messages being written that have been shown, to focus a new one's first field once.
    pub(crate) compose_shown: std::collections::HashSet<u64>,
    /// The editors of the messages being written other than the one acting now (`editor`).
    pub(crate) editors: HashMap<u64, rich_edit::Editor>,
    /// The message whose window a dialog for messages belongs to (Save changes, Address Book).
    pub(crate) dialog_for: Option<u64>,
    /// The system's file dialog open, and what asked for it (`app/file_dialogs.rs`).
    pub(crate) files: app::FileDialogs,
    /// The appointments and meetings open in windows of their own, each with its number, and
    /// the last number given.
    pub(crate) event_windows: Vec<(u64, Box<pim_dialogs::EventForm>)>,
    pub(crate) event_window_ids: u64,
    /// The body's cursor (characters), where pictures go.
    /// The message editor's state (selection, undo, what's typed next).
    pub(crate) editor: Option<rich_edit::Editor>,
    /// The editors of the blocks of text of the originals under replies being written, and the
    /// one with the cursor (its message, its place): the ribbon and pictures go to it.
    pub(crate) quote_editors: reading::QuoteEditors,
    pub(crate) quote_focus: Option<(u64, usize)>,
    /// A selection that goes across the text being written and its original's blocks, and what
    /// it took out, to undo (`reading/compose/across.rs`).
    pub(crate) across: reading::AcrossState,
    /// The contacts fields on screen (To, Cc, Bcc, Replies to, Select Names' rows), by field, and
    /// the send held for the names still being resolved.
    pub(crate) people: dialogs::people_field::PeopleFields,
    /// Installed Craft apps (command, path), found at start-up.
    pub(crate) craft_apps: Vec<(&'static str, PathBuf)>,
    /// The interface language setting (`auto` or a language code).
    pub language: String,
    /// The fonts installed, kept to add a CJK fallback later (and for PDF thumbnails).
    pub(crate) fonts: thumbs::Fonts,
    /// A fallback font for Arabic and Hebrew (when the main font hasn't them).
    pub(super) rtl_font: Option<Vec<u8>>,
    /// The system was asked for its colour emoji font (`look/emoji.rs`), once.
    pub(super) emoji_asked: bool,
    /// The language whose CJK font is loaded, and the one replies quote their original in.
    pub(super) cjk_loaded: Option<&'static str>,
    pub(super) quoted_in: Option<&'static str>,
    /// Shown text has Chinese characters none of the fonts has (`reading::han_missing`).
    pub(crate) han_missing: bool,
    /// The app's name in the window title (`Quatrelle`, or `Quatrelle – Sample mailbox`).
    pub title_base: String,
    /// This page is a message's own window (the browser): only that message is shown.
    pub page_message: Option<MessageKey>,
    pub(crate) title_shown: String,
    /// What the icon in the notification area was last shown as.
    pub(super) tray_shown: TrayIcon,
    /// Raise the main window on the next frame.
    pub(super) focus_main: bool,
    /// The Webex card is to be scrolled to as File › Options draws it (read once by the card).
    pub(crate) webex_scroll: std::cell::Cell<bool>,
    /// Video attachments written out for the platform to take a frame from, and the frames.
    pub(crate) video_files: HashMap<(MessageKey, usize), PathBuf>,
    pub(crate) video_frames: HashMap<PathBuf, VideoFrame>,
    /// The work behind them, on the pictures lane: the files written, the frames decoded.
    pub(crate) videos: reading::Videos,
    /// The open message's body on its way back from the reading lane.
    pub(crate) bodies: reading::Bodies,
    /// Long message texts laid out by what's in view: their parts' heights.
    pub(crate) long_text: HashMap<egui::Id, reading::LongText>,
    /// What's selected of the message texts shown, and whether one has the keyboard.
    pub(crate) text_selection: reading::TextSelection,
    /// Find in the message: the reading pane's bar and the message windows'.
    pub(crate) find: reading::Finds,
    /// The File page, while it's open.
    pub backstage: Option<backstage::Backstage>,
    /// The month the calendar's small month shows (`year * 12 + month - 1`; `None`: the day on
    /// screen's).
    pub(crate) mini_month: Option<i64>,
    /// The hours were scrolled to the working day once.
    /// The day and week views have been scrolled to the working day once (each has its own scroll).
    pub(crate) cal_scrolled: [bool; 2],
    pub(crate) people_query: String,
    /// The address book People shows (account, book id); `None`: all of them.
    pub(crate) people_book: Option<(String, String)>,
    /// The message list's rows and where each starts, kept while the list stays the same.
    pub(crate) list_rows: Option<list::ListRows>,
    /// The rows drawn whose preview is still to be worked out from their source.
    pub(crate) want_previews: Vec<MessageKey>,
    /// How People lays contacts out (Outlook's Change View).
    pub people_view: people::PeopleView,
    /// The People list's columns and sort.
    pub people_list: people::ListSettings,
    /// The message table's column widths, as dragged (see `list::ColumnWidths`).
    pub list_columns: list::ColumnWidths,
    /// Notes: the search, the note selected in the grid, and the notes open in windows.
    pub(crate) notes_query: String,
    /// The calendar's search (issue #33): what's typed, the switches, what it found.
    pub(crate) cal_search: calendar::CalSearch,
    /// How far the calendar's month grip has been dragged since it last added or took a month.
    pub(crate) months_drag: f32,
    /// An appointment being dragged to another time ([`calendar::Drag`]).
    pub(crate) cal_drag: Option<calendar::Drag>,
    /// The appointment or empty half hour selected in the calendar's grid ([`calendar::Pick`]).
    pub(crate) cal_pick: Option<calendar::Pick>,
    pub(crate) note_selected: Option<String>,
    /// Tasks: the search, the one picked and its form, the task being typed in.
    pub(crate) tasks: tasks::TasksUi,
    pub(crate) note_windows: Vec<notes::NoteWindow>,
    /// The person whose card is shown (uid).
    pub(crate) person: Option<String>,
    /// The invitations of the messages shown (the reading pane's, those in windows), each worked out once per calendar change.
    pub(crate) invite: Vec<(MessageKey, u64, Option<quatrelle_engine::Invite>)>,
    /// A file to drag out of the window (the app starts the system drag).
    pub(super) drag_out: Option<PathBuf>,
    /// The platform can drag files out.
    pub(crate) can_drag_out: bool,
    /// Typefaces a message being written asked for, read from the system at the next frame.
    pub(crate) load_typefaces: Vec<String>,
}
