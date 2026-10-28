//! The settings the user changes (View tab, File › Options), saved by the app.

use super::*;

/// Mail kept on this computer by default: what came in the last two years (the owner's choice;
/// Outlook offers the same range).
pub const KEEP_DAYS: u32 = 730;

/// How far the message read can be zoomed, in percent.
pub const READING_ZOOM: std::ops::RangeInclusive<u16> = 50..=400;

/// Settings the user changes from the View tab (saved by the app).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prefs {
    pub reading_pane: ReadingPane,
    /// Group the list by conversation.
    pub conversations: bool,
    /// Mark messages read in the reading pane at all (when off, only opening one in its own
    /// window does).
    pub mark_read_on_open: bool,
    pub read_in_pane: ReadInPane,
    pub notes_order: NotesOrder,
    /// Calendars left out of the calendar (`account/calendar id`), as Outlook's unticked ones.
    pub hidden_calendars: BTreeSet<String>,
    /// Calendars this computer has seen (`account/calendar id`): one seen for the first time
    /// that is someone else's in a group, or a public folder's, comes unticked.
    pub known_calendars: BTreeSet<String>,
    /// Day and week show each calendar in a column of its own (Outlook's side-by-side view)
    /// rather than all of them together.
    pub calendars_side_by_side: bool,
    /// Months shown one under another in the calendar's side pane (1 to 6).
    pub calendar_months: u8,
    /// The address book Select Names opens with (Outlook's "Show this address list first"):
    /// `all` (everyone written to or heard from, and contacts), `people` (every contact), or
    /// one address book (`book:<account>:<book id>`).
    pub address_book: String,
    /// Mail received in the last this many days kept on this computer, whole (0: all of it), as
    /// Outlook's "Mail to keep offline" in Cached Exchange Mode; older mail is read on the server.
    pub keep_days: u32,
    /// With `ReadInPane::Shown`: after it's been shown this many seconds (0: at once).
    pub read_after_secs: u32,
    /// Show the first line of text under each message.
    pub show_preview: bool,
    /// Tighter rows.
    pub compact: bool,
    /// Cards, a table, or whichever suits where the reading pane is.
    pub list_layout: ListLayout,
    /// The month and the coming appointments beside the mail (Outlook's To-Do bar).
    pub todo_bar: bool,
    /// Ask before deleting permanently.
    pub confirm_permanent_delete: bool,
    /// Desktop notifications for new mail in an Inbox.
    pub notify: bool,
    /// An envelope in the notification area while mail that arrived in an Inbox is unread, as
    /// Outlook shows one.
    pub envelope: bool,
    /// The Reminders window opens in a window of its own, over every other, when a reminder
    /// comes (the main window minimized or behind others too), as Outlook's does.
    pub reminders_on_top: bool,
    /// The categories offered, in order.
    pub categories: Vec<quatrelle_model::organize::Category>,
    /// Split the Inbox into Focused and Other.
    pub focused_inbox: bool,
    /// Senders the user moved: `true` always Focused, `false` always Other.
    pub focus_overrides: std::collections::BTreeMap<String, bool>,
    /// Show HTML mail formatted (`false`: as plain text).
    pub show_html: bool,
    /// The message read shown bigger or smaller (Ctrl+wheel over it, or the status bar's zoom,
    /// as Outlook's), in percent: 100 as written ([`Prefs::reading_scale`]).
    pub reading_zoom: u16,
    /// Senders whose pictures from the web load without asking (lower-case addresses).
    pub trusted_senders: BTreeSet<String>,
    /// Servers whose network shares (`\\server\share` links) open without asking (lower
    /// case): a share on any other is opened only once the user says so, as one on a stranger's
    /// server can ask for their sign-in.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub trusted_share_servers: BTreeSet<String>,
    /// Junk Email: senders (a lower-case address, or `@domain`) whose mail goes to Junk as it
    /// arrives ([`junk`]).
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub blocked_senders: BTreeSet<String>,
    /// Junk Email: senders whose mail is never junk (taken back out of Junk; pictures load).
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub safe_senders: BTreeSet<String>,
    /// Folders the user pinned to Favorites, in order. `None` until the user changes them: the
    /// Inbox, Sent Items and Drafts of a single account are shown.
    pub favorites: Option<Vec<FolderRef>>,
    /// New messages open in a window of their own (replies start in the reading pane).
    pub new_mail_window: bool,
    /// How the message list is arranged (View › Arrange by), and turned the other way round.
    pub arrange: Arrange,
    pub reverse_sort: bool,
    /// Work offline (Send / Receive › Work Offline): no account connects; kept across starts.
    pub work_offline: bool,
    /// Ignored conversations (Delete › Ignore): their messages' Message-IDs; replies to any of
    /// them go to Deleted Items as they arrive.
    pub ignored: BTreeSet<String>,
    /// The folder pane minimized to a narrow strip (View › Folder pane, or its ‹ button).
    pub folder_pane_minimized: bool,
    /// What's collapsed in the folder pane, kept from one start to the next as Outlook keeps it:
    /// `favorites`, `account:<id>`, `folder:<account>/<path>`.
    #[serde(default)]
    pub collapsed: BTreeSet<String>,
    /// The folder pane's order of mailboxes, moved by the user ([`Session::pane_sections`]):
    /// account ids, and shared mailboxes as `<account>|<address>`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pane_order: Vec<String>,
    /// When flagged messages are due (Follow Up › Today, Tomorrow…): Message-ID → local day.
    /// IMAP has nowhere for it, so it's kept here, as Outlook keeps it for IMAP accounts.
    #[serde(default)]
    pub follow_up: BTreeMap<String, i64>,
    /// Words the writer added to the dictionary (the spelling check keeps them).
    #[serde(default)]
    pub spelling: Vec<String>,
    /// Underline the words the dictionaries don't know as the message is written (Outlook's
    /// "Check spelling as you type").
    #[serde(default = "yes")]
    pub spell_check: bool,
    /// Put the `-- ` line (RFC 3676's signature delimiter) before the signature, as some mail
    /// apps mark it. Off: the signature goes in as Outlook adds it, with nothing before it.
    #[serde(default)]
    pub signature_dashes: bool,
    /// Quick Steps: actions done to the selection in one go ([`quick_steps`]).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub quick_steps: Vec<quick_steps::QuickStep>,
    /// Play animations: hovers and selections easing, panes sliding, notices sliding in (off:
    /// everything changes at once).
    pub animations: bool,
    /// Search Folders: saved searches shown as folders ([`search_folders`]).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub search_folders: Vec<search_folders::SearchFolder>,
    /// Reminders answered ([`Session::due_reminders`]), by occurrence
    /// (`<account>|<uid>|<occurrence>`): 0 dismissed, else snoozed until that Unix time.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub reminders: BTreeMap<String, i64>,
    /// Where the search box looks unless told (its menu, as Outlook's; Current Mailbox at first).
    pub search_scope: SearchScope,
    /// The last searches, newest first (the search box's list; cleared from it).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub recent_searches: Vec<String>,
    /// Photos go with where they were taken (the place a phone writes into them). Off, as at
    /// first: it's taken out of every JPEG and PNG attached as the message is sent.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub keep_photo_location: bool,
    /// Links from a message open where they really go: out of redirect wrappers, without the
    /// tags that say who clicked ([`mime::clean_link`]). On at first.
    pub clean_links: bool,
    /// Ask a signer's certificate's authority whether it was withdrawn (OCSP or its CRL), as
    /// signed mail is shown. Off at first: the authority learns whose mail is being read.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub check_revocation: bool,
    /// The Webex personal room the appointment window's Webex Meeting button puts in
    /// (`https://<site>.webex.com/meet/<name>`); empty until it's set in Options.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub webex_room: String,
    /// The language model asked about mail (File › Options › Assistant): an OpenAI-compatible
    /// server's address (`…/v1`; empty: none) and the model; its key is in the keychain.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub llm_url: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub llm_model: String,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            reading_pane: ReadingPane::Right,
            conversations: false,
            mark_read_on_open: true,
            read_in_pane: ReadInPane::Shown,
            notes_order: NotesOrder::Modified,
            hidden_calendars: BTreeSet::new(),
            known_calendars: BTreeSet::new(),
            calendars_side_by_side: false,
            calendar_months: 2,
            address_book: "all".into(),
            keep_days: KEEP_DAYS,
            read_after_secs: 0,
            show_preview: true,
            compact: false,
            list_layout: ListLayout::Auto,
            todo_bar: true,
            confirm_permanent_delete: true,
            favorites: None,
            notify: true,
            envelope: true,
            reminders_on_top: true,
            categories: quatrelle_model::organize::default_categories(),
            focused_inbox: false,
            focus_overrides: std::collections::BTreeMap::new(),
            show_html: true,
            reading_zoom: 100,
            trusted_senders: BTreeSet::new(),
            trusted_share_servers: BTreeSet::new(),
            blocked_senders: BTreeSet::new(),
            safe_senders: BTreeSet::new(),
            new_mail_window: true,
            arrange: Arrange::Date,
            reverse_sort: false,
            work_offline: false,
            ignored: BTreeSet::new(),
            follow_up: BTreeMap::new(),
            reminders: BTreeMap::new(),
            search_folders: Vec::new(),
            animations: true,
            search_scope: SearchScope::Mailbox,
            recent_searches: Vec::new(),
            keep_photo_location: false,
            clean_links: true,
            check_revocation: false,
            webex_room: String::new(),
            llm_url: String::new(),
            llm_model: String::new(),
            quick_steps: Vec::new(),
            folder_pane_minimized: false,
            collapsed: BTreeSet::new(),
            spelling: Vec::new(),
            spell_check: true,
            signature_dashes: false,
            pane_order: Vec::new(),
        }
    }
}

impl Prefs {
    /// The message read's zoom as a factor (1.0 as written), within [`READING_ZOOM`] whatever
    /// the settings file says.
    pub fn reading_scale(&self) -> f32 {
        f32::from(self.reading_zoom.clamp(*READING_ZOOM.start(), *READING_ZOOM.end())) / 100.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReadingPane {
    #[default]
    Right,
    Bottom,
    Off,
}

/// How the message list lays out its messages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ListLayout {
    /// A table when the reading pane is at the bottom or off (the list is wide then), else
    /// cards, as Outlook switches.
    #[default]
    Auto,
    /// Each message on lines of its own: the sender, the subject, a preview.
    Cards,
    /// One line per message, in columns that sort the list when their heading is clicked.
    Table,
}

/// How notes are listed (Outlook's Arrange by for notes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NotesOrder {
    /// The last changed first.
    #[default]
    Modified,
    /// A to Z by their first line.
    Title,
    /// By colour, then A to Z.
    Color,
}

/// When a message shown in the reading pane is marked read (with `Prefs::mark_read_on_open`),
/// as Outlook's Reading Pane options have it. Opened in its own window, it always is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReadInPane {
    /// When it's shown (after `Prefs::read_after_secs`).
    #[default]
    Shown,
    /// When another message is chosen (or the folder left).
    Left,
}

/// A setting that was on before it existed.
fn yes() -> bool {
    true
}

impl ListLayout {
    pub const ALL: [ListLayout; 3] = [ListLayout::Auto, ListLayout::Cards, ListLayout::Table];

    pub fn code(self) -> &'static str {
        match self {
            ListLayout::Auto => "auto",
            ListLayout::Cards => "cards",
            ListLayout::Table => "table",
        }
    }

    pub fn from_code(s: &str) -> Option<ListLayout> {
        ListLayout::ALL.into_iter().find(|l| l.code() == s)
    }
}
