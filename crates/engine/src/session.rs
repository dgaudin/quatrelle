//! The session: accounts, the current view and its message list, the open message, the message
//! being written, preferences, and the queue of jobs for the network layer.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use quatrelle_mime::{self as mime, OutAttachment, Parsed, ReplyMode};
use quatrelle_model::contacts::{Contact, ContactBook};
use quatrelle_model::dates::{Clock, DateGroup};
use quatrelle_model::job::{Secret, Vacation};
use quatrelle_model::search::{Doc, Index, Query, summary_text};
use quatrelle_model::thread::{Thread, threads};
use quatrelle_model::{
    AccountConfig, EmailAddress, Event, Flag, Folder, FolderRole, Job, MessageKey, Op, Summary, SyncStatus, addr, sort_folders, sort_newest_first,
};
use quatrelle_store::{FolderState, OutboxItem, Store};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{CmdError, CmdResult, Notice, Registry};
use quatrelle_model::search::FolderIndex;
use quatrelle_model::vcard::Photo;

mod accounts;
mod changing;
mod events;
pub mod invite;
mod listing;
mod mailboxes;
mod opening;
mod organize;
mod outbox;
mod pictures;
pub mod pim;
mod prefs;
pub mod scheduling;
mod searching;
mod security;
mod writing;
// The list's, the settings', the message being written's and organizing mail's own parts,
// named here as before.
pub use listing::{ARRANGE_HEADINGS, Arrange, FolderRef, ListFilter, OUTBOX, Row, View, arrange_list};
use listing::{Preview, person, size_group};
use mailboxes::{Contacts, bare_name, directory_key};
pub use mailboxes::{DirectoryAnswer, FolderSizes, Sizes, fill_name};
pub use organize::{RulesScope, junk, quick_steps, search_folders};
pub use prefs::{
    ListLayout, MEETINGS_PROFILE, NotesOrder, PictureLoading, Prefs, READING_ZOOM, RECENT_EMOJI, ReadInPane, ReadingPane, is_profile_dir,
};
use searching::SearchIndex;
pub use security::{CardSearch, KnownCert, Lock, RevocationCheck, SMIME_DOC, Seal, Security, SignedBy, SmimeSettings};
use writing::NameCheck;
pub use writing::{
    AssistKind, Assistant, Compose, Filing, ItemChoice, ItemKind, LlmModels, NAMES_NOT_FOUND, NameChoice, Rewrite, SendCheck, Sender, add_pictures,
    style_signature,
};

/// One account and what the session knows about it.
#[derive(Clone, Debug, PartialEq)]
pub struct AccountState {
    pub config: AccountConfig,
    pub folders: Vec<Folder>,
    pub status: SyncStatus,
    /// Space used on the server and its limit in bytes, once reported.
    pub quota: Option<(u64, u64)>,
    /// The automatic replies as last read from the server.
    pub vacation: VacationState,
}

/// What is known of an account's automatic replies.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum VacationState {
    #[default]
    Unknown,
    /// Asked the server.
    Loading,
    Loaded(Vacation),
    /// The server offers none, or reading or changing them failed.
    Failed(String),
}

impl AccountState {
    /// This account's own folder for a role (not one in another mailbox opened here).
    pub fn folder(&self, role: FolderRole) -> Option<&Folder> {
        self.folders.iter().find(|f| f.role == role && f.mailbox.is_none() && self.config.folder_owner(&f.path).is_none())
    }

    /// The mailbox a folder path is in (`None`: this account's own), whether the server marks
    /// its folders (`mailbox`, Exchange) or names them after the address (Graph's `/users`).
    pub fn folder_mailbox(&self, path: &str) -> Option<String> {
        self.folders.iter().find(|f| f.path == path).and_then(|f| f.mailbox.clone()).or_else(|| self.config.folder_owner(path).map(str::to_string))
    }

    /// The folder for a role in the mailbox `path` is in: another mailbox's own (its Deleted
    /// Items for mail deleted there), else this account's.
    pub fn folder_beside(&self, path: &str, role: FolderRole) -> Option<&Folder> {
        if let Some(owner) = self.folder_mailbox(path) {
            return self
                .folders
                .iter()
                .find(|f| f.mailbox.as_deref() == Some(owner.as_str()) && f.mailbox_role == role)
                .or_else(|| self.folders.iter().find(|f| f.role == role && self.config.folder_owner(&f.path) == Some(owner.as_str())));
        }
        self.folder(role)
    }

    /// A local-only account (the sample mailbox): no server, changes apply to the cache alone.
    pub fn is_local(&self) -> bool {
        self.config.is_local()
    }

    /// An Outlook data file opened here (its "address" is the file's name).
    pub fn is_data_file(&self) -> bool {
        self.is_local() && !addr::is_valid_email(&self.config.email)
    }
}

/// File › About › Check for updates.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum UpdateCheck {
    #[default]
    NotAsked,
    Checking,
    /// This is the newest (the newest release's tag, when there is one).
    UpToDate(String),
    /// A newer one is out.
    Available(quatrelle_model::job::Release),
    Failed(String),
}

/// The newest release `releases` has for a build of `version` made from commit `build_sha`:
/// `Some` when it's newer. A newer version is; the same version at a later test build is when
/// this build is one of that version's earlier test builds (by its commit). A build made from
/// no release (a developer's), ahead or the same, is up to date.
pub fn newer_release<'a>(
    version: &str,
    build_sha: Option<&str>,
    releases: &'a [quatrelle_model::job::Release],
) -> Option<&'a quatrelle_model::job::Release> {
    let parse = |v: &str| -> Option<(u32, u32, u32)> {
        let mut p = v.split('.').map(|n| n.parse::<u32>().ok());
        Some((p.next()??, p.next()??, p.next()??))
    };
    let current = parse(version)?;
    let newest = releases.iter().filter_map(|r| parse(&r.version).map(|v| (v, r))).max_by_key(|(v, r)| (*v, r.build))?;
    match newest.0.cmp(&current) {
        std::cmp::Ordering::Greater => Some(newest.1),
        std::cmp::Ordering::Less => None,
        std::cmp::Ordering::Equal => {
            let sha = build_sha.map(str::trim).filter(|s| s.len() >= 7)?;
            let mine = releases.iter().find(|r| !r.commit.is_empty() && (r.commit.starts_with(sha) || sha.starts_with(&r.commit)))?;
            (mine.version == newest.1.version && mine.build < newest.1.build).then_some(newest.1)
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchScope {
    /// Every folder of every account (Outlook's All Mailboxes).
    All,
    /// Every folder of the account on view (Outlook's Current Mailbox, its default); every
    /// account's from a unified view.
    #[default]
    Mailbox,
    /// The folder on view and the folders inside it.
    Subfolders,
    /// The folder (or unified view) on screen.
    Current,
}

/// A search asked of the servers ([`Session::search_server`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerSearch {
    /// Which search it is (the jobs' `search`), so a newer one stops it.
    pub id: u64,
    pub query: String,
    /// Accounts still searching.
    pub pending: usize,
    /// What the servers found (in their folders' lists now), shown whether or not the words are
    /// in what's downloaded here.
    pub found: HashSet<MessageKey>,
    /// Why an account couldn't search.
    pub errors: Vec<String>,
}

/// A mailbox in the folder pane: an account, or a shared mailbox one opens ([`Session::pane_sections`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneSection {
    /// The account's id, or `<account>|<address>` for a shared mailbox.
    pub key: String,
    pub account: String,
    /// A shared mailbox: its top folder's path.
    pub mailbox: Option<String>,
}

/// A change the user made that Undo can put back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Undo {
    /// Messages moved (deleted to Deleted Items, archived, moved): found again in `to` by their
    /// Message-ID (a move gives them new keys) and moved back where each came from.
    Move { account: String, to: String, back: Vec<(String, String)> },
    /// Flags or keywords changed: the changes that put them back.
    Ops { account: String, ops: Vec<Op> },
    /// An appointment moved (or one occurrence of it, `occurrence`): where it was, and whether
    /// its guests were told (they're told of its return the same way).
    EventMove { uid: String, occurrence: Option<i64>, start: i64, end: i64, send: bool },
}

/// Changes Undo remembers.
const UNDO_DEPTH: usize = 20;

/// The most messages open in their own windows at once.
pub const MAX_WINDOWS: usize = 8;

/// The message open in the reading pane (or in a window of its own).
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    pub key: MessageKey,
    pub parsed: Option<Parsed>,
    pub error: Option<String>,
    /// The other messages of its conversation in this list, oldest first.
    pub thread: Vec<MessageKey>,
    /// Who it says it's from doesn't add up: a lookalike of a domain the user deals with, or
    /// another address shown in the sender's name.
    pub warning: Option<quatrelle_model::lookalike::SenderWarning>,
    /// Signed or encrypted (S/MIME), and how that went.
    pub security: Option<Security>,
}

/// Outlook's "Download all": the messages still to bring onto this computer, the ones asked for
/// (a batch at a time) and not answered yet, and how many are done.
struct DownloadAll {
    keys: Vec<MessageKey>,
    next: usize,
    in_flight: HashSet<MessageKey>,
    done: usize,
    failed: usize,
}

/// Message sources asked for at once by "Download all": the worker fetches them in order, and
/// each answer asks for the next, so a mailbox of any size doesn't flood the job queue.
const FETCH_BATCH: usize = 32;

/// File for the Replied/Forwarded flags still owed by replies and forwards in the Outbox.
const PENDING_MARKS: &str = "pending-marks.json";

/// A local account with more messages than this keeps the counts it saved when it opens, rather
/// than reading every folder to count them.
const RECOUNT_MAX: u64 = 50_000;

/// Message summaries kept in memory at most, besides the folders on screen. A summary takes about
/// 1.4 KB (its recipients, references and preview): a search of everything or the address book
/// reads every folder, and at 300,000 a 210,000-message Exchange mailbox stayed in memory whole
/// (300 MB) for the rest of the day. A folder read again comes from the cache file in milliseconds.
const CACHE_MESSAGES: usize = 50_000;

/// Body words kept for search at most (bytes): enough for the messages a search looks at again.
const BODY_WORDS_BYTES: usize = 48 << 20;

/// How many command panics [`Session::run`] has caught in this process: none is the only right
/// number, so tests that drive the whole window check it (a caught panic is otherwise only a
/// line in the log).
pub static PANICS_CAUGHT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub struct Session {
    pub store: Option<Store>,
    pub accounts: Vec<AccountState>,
    pub view: View,
    pub filter: ListFilter,
    /// The message last opened: it stays in a filtered list (Unread) until another is opened or
    /// the filter or folder changes, as in Outlook — marked read as it opened, it used to vanish
    /// from under the pointer, the reading pane emptied with it.
    pub(crate) filter_keep: Option<MessageKey>,
    /// The active search (query text and scope).
    pub search: Option<(String, SearchScope)>,
    /// The search's index is still being built: the results so far are shown while it goes on, a
    /// slice per call, so the window stays responsive.
    pub search_pending: bool,
    /// Messages the search index adds per call (`usize::MAX`: no limit).
    search_budget: usize,
    /// The messages waiting for a reply, worked out once for a search call and the list it
    /// shows (it walks every folder), forgotten when they're done.
    awaiting_memo: Option<HashSet<MessageKey>>,
    /// What the server found for the search on screen: Exchange's own index answers for the
    /// messages whose body was never downloaded, and those are merged with the local answer the
    /// way Outlook does.
    found: Option<(String, Vec<Summary>)>,
    /// A folder's mail older than what's kept here, read on the server (Cached Exchange Mode's
    /// "more on the server"): the account, the folder, the messages, and whether the server
    /// has more. Held in memory only, shown after the folder's own list.
    pub(crate) older: Option<(String, String, Vec<Summary>, bool)>,
    /// The query (and scope) the server was asked about: once per search, however many slices
    /// the local index takes.
    server_asked: Option<(String, SearchScope)>,
    /// The Search Folder last opened ([`Session::open_search_folder_name`]: while the search is
    /// still its own).
    pub search_folder: Option<String>,
    /// The search box's query asked of the servers at the user's request (Search the server, at
    /// the end of the results: mail past what's kept here), beyond what `ask_server`
    /// asks on its own.
    pub server_search: Option<ServerSearch>,
    /// Server searches asked so far (each one's id).
    searches: u64,
    /// The recent searches while an account is kept online only (issue #4): in memory only,
    /// since what's searched for can tell what the mail says.
    recent_in_memory: Vec<String>,
    /// Outlook's "Download all": the messages still to bring onto this computer.
    download_all: Option<DownloadAll>,
    /// A folder asked to show more of its older mail than it keeps (`folder.more`), until the
    /// sync it started ends.
    pub showing_more: Option<(String, String)>,
    /// Accounts whose password is to be asked for, one at a time (`NeedsPassword`), and those
    /// already asked while this runs.
    pub ask_password: Vec<String>,
    asked_password: HashSet<String>,
    /// Pictures the directory keeps for addresses, by lowercased address (`None`: there is
    /// none, or the server keeps none).
    person_photos: BTreeMap<String, Option<Photo>>,
    /// Addresses a picture was asked for (even when the answer was "none").
    photo_asked: HashSet<String>,
    /// The messages on screen, newest first: shared with the folders' cached lists (a message
    /// changed there is copied, this one staying as it was until the list is made again).
    pub list: Vec<Arc<Summary>>,
    /// Conversations over `list` (when [`Prefs::conversations`] is on).
    pub threads: Vec<Thread>,
    pub selection: Vec<MessageKey>,
    pub focus: Option<MessageKey>,
    pub reading: Option<Reading>,
    /// Messages opened in their own windows (double-click), oldest first.
    pub windows: Vec<Reading>,
    /// The message being written that commands act on (`compose.*`).
    pub compose: Option<Compose>,
    /// Other messages being written, each in a window of its own (Outlook keeps every one
    /// open): starting another sets the current one aside here rather than losing it.
    /// [`Session::activate_compose`] brings one back to act on.
    pub composes: Vec<Compose>,
    /// Replies written in the reading pane and left for another message, as Outlook leaves them:
    /// kept in Drafts, and back in the pane when their message is read again (oldest first).
    pub set_aside: Vec<Compose>,
    /// The messages a reply waits for, in Drafts or set aside, by `Message-ID`
    /// ([`Session::has_draft_reply`]).
    pub(crate) drafted: HashSet<String>,
    /// The words the original's header is written with under a reply, in the interface's
    /// language (the app sets them; English without it).
    pub quote_words: mime::QuoteWords,
    pub prefs: Prefs,
    pub clock: Clock,
    pub notices: Vec<Notice>,
    /// What the last changes can be put back to, newest last (Undo, Ctrl+Z).
    pub undo: Vec<Undo>,
    /// Bumped whenever `list` changes.
    pub revision: u64,
    /// Bumped whenever a calendar or address book changes.
    pub pim_revision: u64,
    /// Messages waiting in the Outbox, per account (oldest first).
    pub outbox: Vec<(String, OutboxItem)>,
    /// Held-back Outbox messages already asked to be sent when their time came.
    flushed: BTreeSet<u64>,
    /// Accounts whose server has been asked for its rules since it connected (once is enough).
    rules_asked: std::collections::BTreeSet<String>,
    /// Accounts whose rules were read on coming online: a look for new mail (every half minute)
    /// goes back to Syncing and then Online without reading them again.
    rules_online: std::collections::BTreeSet<String>,
    /// The public folders each Exchange account's sync was last told to follow
    /// ([`Session::follow_public_favorites`]).
    followed_public: HashMap<String, Vec<String>>,
    /// The last reason each account's server gave for refusing its rules that was told this
    /// session: the same one isn't told again on every try.
    rules_refused_told: HashMap<String, String>,
    jobs: Vec<Job>,
    pub(crate) cache: HashMap<(String, String), FolderState<Arc<Summary>>>,
    /// The cached folders, the least recently used first.
    pub(crate) cache_order: Vec<(String, String)>,
    /// Accounts whose other people's appointments are set aside (`pim/parking.rs`), and since
    /// when the calendar isn't shown.
    parked: HashSet<String>,
    away_from_calendar: Option<i64>,
    /// Other people's calendars read again while theirs were set aside, by account: what was
    /// read wins over what was set aside.
    read_aside: HashMap<String, HashSet<String>>,
    /// Previews worked out from sources for list rows that came without one
    /// ([`Session::fill_previews`]).
    previews: HashMap<MessageKey, Preview>,
    /// Flags owed to originals, keyed by the Outbox id of the reply or forward that earns them:
    /// set when the server accepts that message. Saved to disk so a restart keeps them.
    pending_marks: BTreeMap<u64, (MessageKey, Flag)>,
    /// Summaries kept cached at most ([`CACHE_MESSAGES`]; tests make it small).
    pub(crate) cache_limit: usize,
    /// The address book (recipient suggestions, Select Names, the Focused Inbox): kept on disk,
    /// and only the mail new in a folder read into it (see `session/mailboxes/contacts.rs`).
    contacts: Contacts,
    /// The domains the user deals with ([`Session::known_domains`]), and the address book's
    /// revision they were counted from.
    known_domains: Option<(u64, BTreeSet<String>)>,
    /// S/MIME: each account's settings, the roots trusted for email, decrypted messages (in
    /// memory only: the MIME entity inside, and whether it was authenticated), the ones being
    /// decrypted and why others couldn't be, and messages waiting to be signed or encrypted.
    smime: HashMap<String, SmimeSettings>,
    trust_roots: Vec<Vec<u8>>,
    opened: HashMap<MessageKey, (Vec<u8>, bool)>,
    opening: HashSet<MessageKey>,
    decrypt_errors: HashMap<MessageKey, String>,
    protecting: HashMap<u64, security::Protecting>,
    revocation: security::Revocations,
    cards: security::CardSearch,
    /// The highest UID seen per Inbox, so only mail that arrives later is announced.
    pub(crate) seen_uid: HashMap<(String, String), u32>,
    /// New mail to announce (desktop notifications), oldest first.
    pub new_mail: Vec<Summary>,
    /// New mail still unread, for the envelope in the notification area.
    pub(crate) envelope: mailboxes::Envelope,
    /// Inbox rules, run on mail as it arrives (`rules.json`).
    pub rules: Vec<quatrelle_model::organize::Rule>,
    /// The rules the accounts' servers run that Quatrelle didn't make (Outlook's own, the
    /// server's web page, a Sieve script): listed, never changed from here. Read when an account
    /// connects, and after every change of ours.
    pub foreign_rules: Vec<quatrelle_model::organize::ForeignRule>,
    /// Snoozed messages and when they come back (`snoozes.json`).
    pub snoozes: Vec<quatrelle_model::organize::Snooze>,
    /// The Other tab of a Focused/Other Inbox is showing.
    pub focus_other: bool,
    /// Pictures from the web, by address: downloaded only after the user asked.
    pub pictures: HashMap<String, Picture>,
    /// Messages whose pictures the user asked to see this time.
    pictures_for: BTreeSet<MessageKey>,
    /// Server settings looked up per address (lower case): `None` while looking.
    pub discovered: HashMap<String, Option<(Option<AccountConfig>, String)>>,
    /// Accounts waiting for a data file to come in before they connect (their start).
    held: HashMap<String, Job>,
    /// The account whose data file is being compacted (Compact Now), and the last one
    /// compacted here with its size before and after.
    pub compacting: Option<String>,
    pub compacted: Option<(String, u64, u64)>,
    /// Folder Size: each of an account's folders' sizes here and on its server.
    pub sizes: Option<FolderSizes>,
    /// What the language model was last asked about the message being read.
    pub assistant: Option<Assistant>,
    /// The models the language model's server offers (File › Options › Assistant).
    pub llm_models: Option<LlmModels>,
    /// OAuth providers this copy can sign in with (set by the app).
    pub oauth_available: Vec<String>,
    /// Running portable (set by the app): everything is kept beside it, passwords only in memory.
    pub portable: bool,
    /// Only JMAP accounts can be added (the browser, which can't open IMAP or SMTP connections).
    pub jmap_only: bool,
    /// The fonts PDFs embed (regular, bold): the system's, set by the app.
    pub print_fonts: Option<(Vec<u8>, Option<Vec<u8>>)>,
    /// Pages to show in the browser (OAuth sign-in), oldest first.
    pub open_urls: Vec<String>,
    /// A vote on its way: the message and the choice.
    pub vote_sending: Option<(MessageKey, String)>,
    /// Each account's calendars and address books.
    pub pim: HashMap<String, quatrelle_model::pim::Pim>,
    /// The rooms each account offers for meetings, once asked for (Outlook's Room Finder).
    pub rooms: HashMap<String, scheduling::Fetched<scheduling::RoomFinder>>,
    /// When people and rooms are busy, by (account, address, from, to), once asked for (the
    /// Scheduling Assistant's free/busy).
    pub free_busy: HashMap<(String, String, i64, i64), scheduling::Fetched<quatrelle_model::scheduling::Availability>>,
    /// Mail, Calendar or People.
    pub module: pim::Module,
    pub cal_mode: pim::CalMode,
    /// The local day the calendar shows (`None`: today).
    pub cal_day: Option<i64>,
    index: Option<SearchIndex>,
    /// The message in the reading pane to mark read, and when (`ReadInPane::Shown` with a wait).
    read_due: Option<(MessageKey, i64)>,
    /// The message to mark read once another is chosen (`ReadInPane::Left`).
    read_on_leave: Option<MessageKey>,
    /// Each downloaded message's words for search (its text and attachment names), read from
    /// the cache once: the index is rebuilt whenever mail changes, and parsing every message
    /// again each time made typing in the search box hang.
    body_words: HashMap<MessageKey, Arc<str>>,
    /// Their size, kept under [`BODY_WORDS_BYTES`].
    body_words_bytes: usize,
    registry: Arc<Registry>,
    next_id: u64,
    seed: u64,
    /// The commit this build was made from (release builds: `QUATRELLE_BUILD_SHA`), for
    /// telling its test build from a later one of the same version.
    pub build_sha: Option<String>,
    /// File › About's update check.
    pub update: UpdateCheck,
}

const ACCOUNT_COLORS: u8 = 8;

impl Session {
    /// A session over a cache. Remote accounts get a `Start` job (password from the keychain).
    pub fn open(store: Option<Store>, clock: Clock) -> Session {
        let mut s = Session {
            store,
            accounts: Vec::new(),
            view: View::Empty,
            filter: ListFilter::All,
            filter_keep: None,
            search: None,
            search_pending: false,
            search_budget: usize::MAX,
            awaiting_memo: None,
            found: None,
            older: None,
            server_asked: None,
            search_folder: None,
            server_search: None,
            searches: 0,
            recent_in_memory: Vec::new(),
            download_all: None,
            showing_more: None,
            ask_password: Vec::new(),
            asked_password: HashSet::new(),
            person_photos: BTreeMap::new(),
            photo_asked: HashSet::new(),
            list: Vec::new(),
            threads: Vec::new(),
            selection: Vec::new(),
            focus: None,
            reading: None,
            windows: Vec::new(),
            compose: None,
            composes: Vec::new(),
            set_aside: Vec::new(),
            drafted: HashSet::new(),
            quote_words: mime::QuoteWords::default(),
            prefs: Prefs::default(),
            clock,
            notices: Vec::new(),
            undo: Vec::new(),
            revision: 0,
            pim_revision: 0,
            outbox: Vec::new(),
            flushed: BTreeSet::new(),
            rules_asked: std::collections::BTreeSet::new(),
            rules_online: std::collections::BTreeSet::new(),
            followed_public: HashMap::new(),
            rules_refused_told: HashMap::new(),
            jobs: Vec::new(),
            cache: HashMap::new(),
            cache_order: Vec::new(),
            previews: HashMap::new(),
            parked: HashSet::new(),
            away_from_calendar: None,
            read_aside: HashMap::new(),
            pending_marks: BTreeMap::new(),
            cache_limit: CACHE_MESSAGES,
            contacts: Contacts::default(),
            known_domains: None,
            smime: HashMap::new(),
            trust_roots: Vec::new(),
            opened: HashMap::new(),
            opening: HashSet::new(),
            decrypt_errors: HashMap::new(),
            protecting: HashMap::new(),
            revocation: Default::default(),
            cards: Default::default(),
            seen_uid: HashMap::new(),
            new_mail: Vec::new(),
            envelope: Default::default(),
            rules: Vec::new(),
            foreign_rules: Vec::new(),
            snoozes: Vec::new(),
            focus_other: false,
            pictures: HashMap::new(),
            pictures_for: BTreeSet::new(),
            discovered: HashMap::new(),
            held: HashMap::new(),
            compacting: None,
            compacted: None,
            sizes: None,
            assistant: None,
            llm_models: None,
            oauth_available: Vec::new(),
            portable: false,
            jmap_only: false,
            print_fonts: None,
            open_urls: Vec::new(),
            vote_sending: None,
            pim: HashMap::new(),
            rooms: HashMap::new(),
            free_busy: HashMap::new(),
            module: pim::Module::Mail,
            cal_mode: pim::CalMode::Week,
            cal_day: None,
            index: None,
            read_due: None,
            read_on_leave: None,
            body_words: HashMap::new(),
            body_words_bytes: 0,
            registry: Arc::new(crate::cmd::registry()),
            // Compose ids also name Outbox entries, which outlive a session: start from the clock.
            next_id: u64::try_from(clock.now.max(0)).unwrap_or(0).saturating_mul(1000),
            seed: (clock.now as u64) ^ 0x9E37_79B9_7F4A_7C15,
            build_sha: None,
            update: UpdateCheck::NotAsked,
        };
        if let Some(store) = &s.store {
            s.rules = store.load_doc("rules.json").unwrap_or_default();
            s.snoozes = store.load_doc("snoozes.json").unwrap_or_default();
            s.contacts = Contacts::load(store);
            s.pending_marks = store
                .load_doc::<Vec<(u64, MessageKey, Flag)>>(PENDING_MARKS)
                .unwrap_or_default()
                .into_iter()
                .map(|(id, k, f)| (id, (k, f)))
                .collect();
            let accounts = store.load_accounts().unwrap_or_else(|e| {
                log::warn!("accounts: {e}");
                Vec::new()
            });
            for config in accounts {
                // Online only: held in memory from the start (anything left on disk goes).
                if config.keeps_online()
                    && let Err(e) = store.set_online_only(&config.id, true)
                {
                    log::warn!("{}: couldn't be kept online only: {e}", config.id);
                }
                let mut folders = store.load_folders(&config.id).unwrap_or_default();
                // A folder belongs to the account it's kept with (a data file brought in from
                // another account once kept that one's name).
                for f in &mut folders {
                    if f.account != config.id {
                        f.account.clone_from(&config.id);
                    }
                }
                sort_folders(&mut folders);
                s.accounts.push(AccountState { config, folders, status: SyncStatus::Offline, quota: None, vacation: VacationState::Unknown });
            }
        }
        let mut local = Vec::new();
        for a in &s.accounts {
            if a.is_local() {
                local.push(a.config.id.clone());
            } else {
                s.jobs.push(Job::Start { account: a.config.clone(), password: None });
            }
        }
        // Counted again on opening, unless big: a big data file's counts were kept as it came in
        // and with every change since (counting meant reading its every folder's list).
        for id in local {
            let kept: u64 = s.account(&id).map(|a| a.folders.iter().map(|f| u64::from(f.total)).sum()).unwrap_or(0);
            if kept <= RECOUNT_MAX {
                s.recount_local(&id, None);
            }
        }
        // What's already cached isn't new.
        let inboxes: Vec<(String, String)> =
            s.accounts.iter().filter_map(|a| a.folder(FolderRole::Inbox).map(|f| (a.config.id.clone(), f.path.clone()))).collect();
        for (a, f) in inboxes {
            s.note_new_mail(&a, &f);
        }
        for id in s.accounts.iter().map(|a| a.config.id.clone()).collect::<Vec<_>>() {
            s.mark_junk(&id);
        }
        s.refresh_outbox();
        s.fetch_signature_pictures();
        s.load_pim();
        s.load_smime();
        s.view = s.default_view();
        s.refresh_list();
        s
    }

    /// A session with no cache (tests, previews).
    pub fn blank() -> Session {
        Session::open(None, Clock::default())
    }

    pub fn registry(&self) -> Arc<Registry> {
        self.registry.clone()
    }

    /// Run a command by id. Disabled commands fail with [`CmdError::Disabled`]. A panic inside a
    /// command is caught and reported as an error rather than taking the app down.
    pub fn run(&mut self, id: &str, params: &Value) -> CmdResult {
        let reg = self.registry.clone();
        let spec = reg.get(id).ok_or_else(|| CmdError::Unknown(id.to_string()))?;
        if let Some(why) = (spec.enabled)(self) {
            return Err(CmdError::Disabled(why.to_string()));
        }
        let run = spec.run;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(self, params)));
        match result {
            Ok(r) => r,
            Err(_) => {
                log::error!("command {id} panicked");
                PANICS_CAUGHT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                self.refresh_list();
                Err(CmdError::Failed(format!("{id} failed unexpectedly")))
            }
        }
    }

    /// Jobs for the network layer, oldest first.
    pub fn take_jobs(&mut self) -> Vec<Job> {
        self.update_calendar_reading();
        std::mem::take(&mut self.jobs)
    }

    pub fn push_job(&mut self, job: Job) {
        self.jobs.push(job);
    }

    pub fn set_clock(&mut self, now: i64, offset: i32) {
        self.clock = Clock::new(now, offset).in_zone(self.clock.zone);
    }

    /// The local time zone, by the name the system gives (`Europe/Berlin`): times then show
    /// with the offset of their own date. One it doesn't know, or that doesn't fit the clock's
    /// offset now, leaves today's offset for all dates.
    pub fn set_zone(&mut self, name: Option<&str>) {
        let (now, offset) = (self.clock.now, i64::from(self.clock.offset));
        self.clock.zone = name.and_then(quatrelle_model::zone::Zone::named).filter(|z| z.offset_at(now) == offset);
    }

    fn fresh_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// A fresh `Content-ID` for a picture placed in a message.
    pub fn new_cid(&mut self) -> String {
        format!("{:016x}@quatrelle", self.random())
    }

    fn random(&mut self) -> u64 {
        // xorshift64*: unique Message-IDs, not cryptography.
        let mut x = self.seed.max(1);
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.seed = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

/// Most web pictures loaded for one message.
const MAX_PICTURES: usize = 60;

/// A picture from the web.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Picture {
    Loading,
    Ready(Arc<Vec<u8>>),
    Failed(String),
}

/// Where a move goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MoveTarget {
    Role(FolderRole),
    Folder(String),
}

/// `{"key": "a1:7:42:INBOX"}` or `{"account":…,"folder":…,"validity":…,"uid":…}`.
pub fn parse_key(v: &Value) -> Option<MessageKey> {
    if let Some(s) = v.get("key").and_then(Value::as_str) {
        return MessageKey::from_id(s);
    }
    serde_json::from_value::<MessageKey>(v.clone()).ok().filter(|k| !k.account.is_empty())
}

/// What a follow-up date is kept under: the Message-ID, or (without one) the message's place.
fn follow_up_id(m: &Summary) -> String {
    if m.message_id.trim().is_empty() { m.key.to_id() } else { m.message_id.trim().to_string() }
}

impl SearchScope {
    pub const ALL: [SearchScope; 4] = [SearchScope::Current, SearchScope::Subfolders, SearchScope::Mailbox, SearchScope::All];

    /// Its name in commands (`search.run`'s `scope`).
    pub fn code(self) -> &'static str {
        match self {
            SearchScope::All => "all",
            SearchScope::Mailbox => "mailbox",
            SearchScope::Subfolders => "subfolders",
            SearchScope::Current => "current",
        }
    }

    pub fn from_code(code: &str) -> Option<SearchScope> {
        SearchScope::ALL.into_iter().find(|s| s.code().eq_ignore_ascii_case(code))
    }
}
