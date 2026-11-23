// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Per-account workers.
//!
//! [`Pool`] owns one thread per account. Each thread keeps an IMAP connection, runs the
//! engine's jobs in order, replays changes made offline, syncs every folder on connect and
//! periodically, and otherwise IDLEs on the Inbox so new mail shows up at once. Failures never
//! take the app down: a broken connection is retried with exponential back-off, a refused
//! password stops retrying until the user enters a new one.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use quatrelle_model::job::Secret;
use quatrelle_model::{
    AccountConfig, Event, FOLLOWED_PUBLIC_DOC, Flag, Folder, FolderRole, Job, MessageKey, Op, SyncStatus, is_public_folder, sort_folders,
};
use quatrelle_store::{NoteGuard, Noted, OutboxItem, Store};

use crate::imap::Client;
use crate::jmap;
use crate::stream::Stream;
use crate::sync::SyncReport;
use crate::sync::{SyncOptions, fetch_body, sync_folder};
use crate::{NetError, Result, smtp};
use quatrelle_model::addr::trust::Trust;

/// Where passwords live. The desktop app backs this with the system keychain.
pub trait Credentials: Send + Sync {
    fn get(&self, account: &AccountConfig) -> Option<String>;
    fn set(&self, account: &AccountConfig, password: &str) -> std::result::Result<(), String>;
    fn delete(&self, account: &AccountConfig);
    /// The outgoing server's own password (accounts with an `smtp_username`), kept apart.
    fn get_smtp(&self, account: &AccountConfig) -> Option<String>;
    fn set_smtp(&self, account: &AccountConfig, password: &str) -> std::result::Result<(), String>;
    fn delete_smtp(&self, account: &AccountConfig);
    /// Another secret of the account's, named by `what` (`smime`: its S/MIME certificate's
    /// private key). A store without them keeps none.
    fn get_secret(&self, _account: &AccountConfig, _what: &str) -> Option<String> {
        None
    }
    fn set_secret(&self, _account: &AccountConfig, _what: &str, _secret: &str) -> std::result::Result<(), String> {
        Err("there's no keychain to keep it in".into())
    }
    fn delete_secret(&self, _account: &AccountConfig, _what: &str) {}
    /// Whether what is kept here is still there the next time Quatrelle starts (the system's
    /// keychain). Credentials held in memory say no: a sign-in they take lasts this run only,
    /// and whoever signs in is told so.
    fn persists(&self) -> bool {
        true
    }
}

/// Passwords held in memory only (tests, the CLI, and when no keychain is available).
#[derive(Default)]
pub struct MemoryCredentials(Mutex<HashMap<String, String>>);

impl Credentials for MemoryCredentials {
    fn get(&self, account: &AccountConfig) -> Option<String> {
        self.0.lock().ok().and_then(|m| m.get(&account.id).cloned())
    }
    fn set(&self, account: &AccountConfig, password: &str) -> std::result::Result<(), String> {
        self.0.lock().map_err(|e| e.to_string())?.insert(account.id.clone(), password.to_string());
        Ok(())
    }
    fn delete(&self, account: &AccountConfig) {
        if let Ok(mut m) = self.0.lock() {
            m.remove(&account.id);
        }
    }
    fn get_smtp(&self, account: &AccountConfig) -> Option<String> {
        self.0.lock().ok().and_then(|m| m.get(&format!("{}|smtp", account.id)).cloned())
    }
    fn set_smtp(&self, account: &AccountConfig, password: &str) -> std::result::Result<(), String> {
        self.0.lock().map_err(|e| e.to_string())?.insert(format!("{}|smtp", account.id), password.to_string());
        Ok(())
    }
    fn delete_smtp(&self, account: &AccountConfig) {
        if let Ok(mut m) = self.0.lock() {
            m.remove(&format!("{}|smtp", account.id));
        }
    }
    fn get_secret(&self, account: &AccountConfig, what: &str) -> Option<String> {
        self.0.lock().ok().and_then(|m| m.get(&format!("{}|{what}", account.id)).cloned())
    }
    fn set_secret(&self, account: &AccountConfig, what: &str, secret: &str) -> std::result::Result<(), String> {
        self.0.lock().map_err(|e| e.to_string())?.insert(format!("{}|{what}", account.id), secret.to_string());
        Ok(())
    }
    fn delete_secret(&self, account: &AccountConfig, what: &str) {
        if let Ok(mut m) = self.0.lock() {
            m.remove(&format!("{}|{what}", account.id));
        }
    }
    fn persists(&self) -> bool {
        false
    }
}

/// Sends events and wakes the UI.
#[derive(Clone)]
struct Sink {
    tx: Sender<Event>,
    wake: Arc<Mutex<Option<Arc<dyn Fn() + Send + Sync>>>>,
    /// What each account's syncs made on the server, until the calendars as saved show it.
    stored: Arc<Mutex<HashMap<String, dav::Stored>>>,
}

impl Sink {
    fn emit(&self, e: Event) {
        log_event(&e);
        self.note_stored(&e);
        let _ = self.tx.send(e);
        if let Ok(w) = self.wake.lock()
            && let Some(f) = w.as_ref()
        {
            f();
        }
    }
}

struct Handle {
    tx: Sender<Job>,
    thread: Option<JoinHandle<()>>,
    /// The account's second connection, for work the user sees happen (sending, automatic
    /// replies): it never waits behind a sync. `None` where it can't be used (POP, JMAP sends).
    quick: Option<Sender<Job>>,
    quick_thread: Option<JoinHandle<()>>,
    /// Set as the account stops, when its worker may yet be in a slow call: what's being brought
    /// on the side ([`rest::Rest`]) writes nothing from then on.
    stop: Arc<AtomicBool>,
    /// Messages opened while the main connection is at work, brought on a copy of it.
    opened: Arc<opening::Opened>,
}

/// What an account signs in with, shared by its connections: the password or refresh token, and
/// the current access token (refreshed by one, used by both).
struct Auth {
    secret: String,
    access: Option<crate::oauth::Tokens>,
}

impl Auth {
    fn shared(secret: impl Into<String>) -> Arc<Mutex<Auth>> {
        Arc::new(Mutex::new(Auth { secret: secret.into(), access: None }))
    }
}

/// Queue a job, unless it's a folder to read that is waiting already (a few messages sent in a
/// row each ask for Sent Items: once is enough), or the calendars' and contacts' sync (each
/// calendar ticked and each task changed asks for it, and the one waiting reads them all).
fn push_job(queue: &mut VecDeque<Job>, j: Job) {
    if matches!(j, Job::SyncFolder { .. } | Job::SyncDav { .. } | Job::FetchTextFirst { .. }) && queue.contains(&j) {
        return;
    }
    // Calendars just ticked: the sync waiting reads them too, else those waiting take them in
    // (none named stands for every calendar ticked, as the calendar opens).
    if let Job::SyncCalendars { account, calendars } = &j {
        if queue.iter().any(|q| matches!(q, Job::SyncDav { account: a } if a == account)) {
            return;
        }
        if let Some(Job::SyncCalendars { calendars: waiting, .. }) =
            queue.iter_mut().find(|q| matches!(q, Job::SyncCalendars { account: a, .. } if a == account))
        {
            if calendars.is_empty() {
                waiting.clear();
            } else if !waiting.is_empty() {
                for c in calendars {
                    if !waiting.contains(c) {
                        waiting.push(c.clone());
                    }
                }
            }
            return;
        }
    }
    // A newer search makes a waiting one pointless.
    if let Job::Search { account, .. } = &j {
        queue.retain(|q| !matches!(q, Job::Search { account: a, .. } if a == account));
    }
    queue.push_back(j);
}

/// Whether a job goes on the account's second connection: sending (not over JMAP, whose filing
/// needs the main connection's mailbox ids), automatic replies, and what a meeting being
/// written waits on (rooms, free/busy), which must not sit behind a long sync; and a colleague's
/// calendar just ticked (on-premises Exchange), which waited behind the sync of everything as
/// the app started and the calendar's own read of every calendar (15 s, 29/10).
fn quick_job(j: &Job, acc: &AccountConfig) -> bool {
    match j {
        Job::Send { .. } => acc.pop.is_none() && acc.jmap.is_none(),
        Job::GetVacation { .. } | Job::SetVacation { .. } => acc.pop.is_none(),
        Job::GetRooms { .. } | Job::GetAvailability { .. } | Job::GetMailTips { .. } => acc.ews.is_some() || acc.graph.is_some(),
        Job::ListRecoverable { .. } | Job::RestoreRecoverable { .. } => acc.ews.is_some() || acc.graph.is_some(),
        Job::ServerFolderSizes { .. } | Job::FindCertificates { .. } | Job::SearchDirectory { .. } | Job::ExpandList { .. } => acc.ews.is_some(),
        Job::SyncCalendars { calendars, .. } => acc.ews.is_some() && !calendars.is_empty(),
        _ => false,
    }
}

/// Jobs a full sync serves between two folders rather than at its end: what someone is waiting
/// on (an opened message) and the edits they just made.
fn between_folders(j: &Job) -> bool {
    matches!(j, Job::FetchBody { .. } | Job::FetchTextFirst { .. } | Job::ReadOnline { .. } | Job::Apply { .. })
}

/// Runs jobs for every account.
pub struct Pool {
    store: Store,
    creds: Arc<dyn Credentials>,
    sink: Sink,
    rx: Receiver<Event>,
    workers: HashMap<String, Handle>,
    configs: HashMap<String, AccountConfig>,
    pub opts: SyncOptions,
    /// Where ManageSieve listens on IMAP hosts (automatic replies); tests change it.
    pub sieve_port: u16,
    /// Picture downloads, on their own thread so they never wait behind a sync.
    fetcher: Option<Sender<String>>,
    /// Webex's jobs, on their own thread, one after the other.
    webex_jobs: Option<Sender<Job>>,
    /// Working offline: no account connects until it's turned off.
    offline: bool,
    /// The server search still wanted ([`Job::SearchServer`]'s `search`; 0: none), shared with
    /// every worker so a search running stops when it's no longer wanted.
    search: Arc<AtomicU64>,
    /// The data file imports that are running, by account: set, the flag stops the import at its
    /// next item ([`Job::StopImport`]).
    imports: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    /// Edits to the cache, written on a thread of their own (`pool::edit_writer`): marking a message
    /// read rewrote its folder's list on the window's thread, seconds for a folder of a hundred
    /// thousand messages, at every message opened. `None` once the pool shuts down.
    edits: Option<Sender<pool::Edit>>,
    edits_thread: Option<JoinHandle<()>>,
    /// The Teams worker, started with the first of its jobs and kept for the pool's life: one
    /// worker, whatever is switched off or signed out meanwhile (`pool/teams.rs`).
    teams: Option<crate::teams::worker::Handle>,
    /// Tests: where the Teams worker is sent, and how many were started.
    #[cfg(test)]
    pub(crate) teams_test: pool::teams::Test,
}

/// Write an edit to the cache and journal it (`Store::add_op`), report what changed, then hand it to
/// the account's thread, which takes it to the server. Journaled first: a sync saving the folder
/// meanwhile makes the edit again on what it saves (`Store::save_folder`) rather than putting the
/// old flags back over it. Nothing, for an account removed since: its files go at once on the
/// window's thread, and a change queued before wrote its folder and its queue back.
fn write_edit(store: &Store, sink: &Sink, account: String, op: Op, worker: Option<Sender<Job>>, seen: u64) {
    if store.while_kept(&account, seen, || make_edit(store, sink, &account, &op)).is_none() {
        log::info!("{account}: a change to mail dropped, the account removed");
        return;
    }
    // Wake the account's thread: it uploads the queue at its next opportunity.
    if let Some(w) = worker {
        let _ = w.send(Job::Apply { account, op });
    }
}

/// [`write_edit`]'s change journaled and made in the cache.
fn make_edit(store: &Store, sink: &Sink, account: &str, op: &Op) {
    let account = account.to_string();
    if let Err(e) = store.add_op(&account, op.clone()) {
        sink.emit(Event::Notice { account: account.clone(), message: e.to_string() });
    }
    match store.apply_op(&account, op) {
        Ok(changed) => {
            // Told only as read (the disk failing): taken for none, the account showed no folders.
            if op.is_folder_op() {
                match store.load_folders(&account) {
                    Ok(folders) => sink.emit(Event::Folders { account: account.clone(), folders }),
                    Err(e) => log::warn!("{account}: the folder list wasn't read back: {e}"),
                }
            }
            for folder in changed {
                sink.emit(Event::FolderChanged { account: account.clone(), folder });
            }
        }
        Err(e) => sink.emit(Event::Notice { account, message: e.to_string() }),
    }
}

pub use quatrelle_model::MICROSOFT_NO_PASSWORDS;

/// What a mailbox Microsoft won't open over IMAP needs: IMAP switched on (Outlook.com's own
/// setting; an organisation's administrator for work accounts), the right account picked in the
/// browser, or Microsoft Graph, which doesn't use IMAP.
fn microsoft_imap_closed(email: &str, signed_in_as: Option<&str>) -> String {
    let who = signed_in_as.map(|o| format!(" (the browser signed in as {o})")).unwrap_or_default();
    let fix = if quatrelle_model::microsoft_consumer(email) {
        "Turn on IMAP on Outlook.com (Settings › Mail › Forwarding and IMAP), or connect the account through Microsoft Graph instead"
    } else {
        "Your organisation may have turned IMAP off: connect the account through Microsoft Graph instead, or ask its administrator"
    };
    format!("{}{who}. {fix}.", quatrelle_model::MICROSOFT_IMAP_CLOSED)
}

pub(crate) fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX)).unwrap_or(0)
}

pub(crate) mod ahead;
mod changes;
pub(crate) mod connect;
mod dav;
mod listening;
mod opening;
mod pgp;
mod pool;
mod rest;
mod scheduling;
mod send;
mod sizes;
mod smime;
mod sync;
mod webex;

use changes::{PENDING_DRAFTS, PendingDraft, keep_draft};
pub use changes::{apply_remote, apply_remote_with};
use connect::{SyncDone, connect_for_sync, fenced, reader_stops, sync_folders};

/// Largest picture downloaded for a message.
const MAX_PICTURE: usize = 8 << 20;

const FULL_SYNC_EVERY: Duration = Duration::from_secs(15 * 60);
const IDLE_FOR: Duration = Duration::from_secs(10 * 60);
const POLL_EVERY: Duration = Duration::from_secs(120);
/// An Exchange account's Inbox is looked at more often: a look is one GetFolder now (an
/// unchanged folder isn't read), so new mail shows within half a minute.
const EWS_POLL_EVERY: Duration = Duration::from_secs(30);
/// When Sent Items is read again after Graph or Exchange took a message (its copy is filed a
/// moment later), and once more in case that was too soon.
#[cfg(not(test))]
const SENT_SOON: Duration = Duration::from_secs(3);
#[cfg(not(test))]
const SENT_LATER: Duration = Duration::from_secs(15);
#[cfg(test)]
const SENT_SOON: Duration = Duration::from_secs(2);
#[cfg(test)]
const SENT_LATER: Duration = Duration::from_secs(6);
/// How often the wait for those looks whether its connection has ended.
const SENT_LATER_SLICE: Duration = Duration::from_millis(250);
/// An account's second connection lets go after this long with nothing to do.
const QUICK_IDLE: Duration = Duration::from_secs(5 * 60);

/// The connection to an account's server.
/// Split the rules a server lists into the ones Quatrelle holds the id of, and the names of the
/// others (Outlook's own, the server's web page, a script the user wrote).
fn split_rules(
    all: Vec<quatrelle_model::organize::ForeignRule>,
    ours: &[String],
) -> (Vec<(String, String)>, Vec<quatrelle_model::organize::ForeignRule>) {
    let mut mine = Vec::new();
    let mut others = Vec::new();
    for r in all {
        if ours.iter().any(|o| o == &r.id) {
            mine.push((r.name, r.id));
        } else if !r.id.is_empty() && !r.name.is_empty() {
            others.push(r);
        }
    }
    (mine, others)
}

enum Conn {
    Imap(Client<Stream>),
    Jmap(jmap::Client),
    Graph(crate::graph::Client),
    Ews(Box<crate::ews::Client>),
    /// POP3: signed in once to check the password; every download is a session of its own.
    Pop,
}

struct Worker {
    acc: AccountConfig,
    auth: Arc<Mutex<Auth>>,
    /// Set on an account's second connection: the main one's queue, for what this one leaves to
    /// it (a message that can't go yet, Sent Items to look at).
    main: Option<Sender<Job>>,
    store: Store,
    sink: Sink,
    opts: SyncOptions,
    conn: Option<Conn>,
    /// JMAP and Graph accounts: message numbers and mailbox ids.
    jstate: jmap::State,
    /// The numbers a server-side search gave messages that are in no folder's cache (they live
    /// on the server only): never pruned, or a result couldn't be opened.
    search_numbers: Vec<u32>,
    folders: Vec<Folder>,
    queue: VecDeque<Job>,
    backoff: Duration,
    next_attempt: Instant,
    last_full: Option<Instant>,
    /// Not connecting again until asked (Send / Receive, a new start): the sign-in was refused,
    /// or the server said no to the account itself.
    auth_failed: bool,
    /// An OAuth refusal was answered with a fresh token once; another means signing in again.
    token_retried: bool,
    /// The calendars' and contacts' sync led to a site the account doesn't trust (an item's
    /// address, a collection's): it waits for the account's next start (allowing the site
    /// starts it again). A look for a missing home that stops at one doesn't stop the rest.
    dav_waits: bool,
    /// The sites asked about ([`Event::TrustAsked`]), once each.
    dav_asked: BTreeSet<String>,
    closed: bool,
    /// Where a rotated OAuth refresh token is saved.
    creds: Option<Arc<dyn Credentials>>,
    /// The ManageSieve port (4190; tests use their own).
    sieve_port: u16,
    /// The JMAP server can't push: poll instead.
    no_push: bool,
    /// The last push wait ended in an Inbox sync (so the next one needn't check first).
    pushed: bool,
    /// What the Inbox's last sync was told by its `SELECT` (UIDVALIDITY, UIDNEXT, count), on
    /// whichever connection read it; none after one that failed. See `wait_for_mail`.
    inbox_synced: Option<(u32, u32, u32)>,
    /// Folders to look at again soon: Graph and Exchange file a sent message's copy a moment
    /// after taking it, so Sent Items is read again then rather than at the next poll.
    soon: Vec<(Instant, String)>,
    /// Folders asked to keep more than the setting (Show more on the server): how many of each
    /// (`windows.json` with the account).
    windows: HashMap<String, usize>,
    /// The public folders kept in sync (the user's Favorites among them, written by the session
    /// as [`FOLLOWED_PUBLIC_DOC`]); the others are listed only when opened.
    followed: BTreeSet<String>,
    /// That list couldn't be read, or is damaged: `followed` is the one before (none at the
    /// start), and no public folder is let go of till it's read.
    followed_unread: bool,
    /// Folders whose kept messages are still to be downloaded whole (Cached Exchange Mode's
    /// offline file), a few at a time while nothing else is asked; and the last one's count
    /// still missing, to let go of a folder that no longer gets anywhere.
    ahead: VecDeque<String>,
    ahead_left: Option<(String, usize)>,
    /// What of the first one is still to download (an Exchange account's).
    ahead_todo: crate::ews::AheadTodo,
    /// Downloading ahead paused for room on the disk: when it's looked at again.
    room_again: Option<Instant>,
    /// When the Inbox is next looked at while downloading ahead.
    ahead_poll: Instant,
    /// The server search still wanted (the pool's).
    search: Arc<AtomicU64>,
    /// Exchange telling of changes as they happen.
    listen: listening::Listening,
    /// When the Inbox is next looked at, on a server that can't say when mail comes (no IDLE,
    /// no push). A deadline, not a wait restarted by every job: while someone works (messages
    /// opened, pictures, read marks), the look kept being put off, and new mail with it.
    next_poll: Instant,
    /// The IMAP change the server refused for now, tried again at its time (see `changes::Waiting`).
    waiting: Option<changes::Waiting>,
    /// Big messages whose text came first, on their way whole. Shared with [`Worker::opened`].
    rest: Arc<rest::Rest>,
    /// The main connection's: messages opened while it's at work go to a copy of it.
    opened: Option<Arc<opening::Opened>>,
    /// Folders gone from the server whose cache was kept as changes to the folders waited to be
    /// sent ([`Worker::relist`]): it goes once they're sent.
    gone_meanwhile: Vec<String>,
    /// The folders of a zone the last listing couldn't list (a shared mailbox, public folders,
    /// the online archive), kept as they were ([`Worker::kept_unlisted`]).
    unlisted_kept: HashSet<String>,
    /// The folder list before couldn't be read, or is damaged: a zone a listing misses may have
    /// folders here the worker doesn't know of; no number is pruned, nor the list kept, till one
    /// misses none (whose save sets a damaged one aside first).
    folders_unread: bool,
    /// Why the account's numbering couldn't be read, while it can't: its mail is set aside.
    numbering_unread: Option<String>,
    /// How many times the account was removed when this worker was made: what it writes waits for
    /// a removal and isn't written after one ([`Store::while_kept`]).
    removals: u64,
    /// Held while this worker lives: a thread it starts to ask something of the main connection
    /// later (Sent Items read again) ends with it, rather than keep a stopped account's main
    /// thread waiting for its jobs.
    alive: Arc<()>,
}

/// Where an account keeps its folders' own windows.
const WINDOWS: &str = "windows.json";
/// Older mail read on the server a page at a time (Cached Exchange Mode's "more on the server").
const OLDER_PAGE: usize = 100;
/// Messages downloaded whole in one step ahead of reading: a step is short, so whatever is
/// asked meanwhile waits for one at most.
const AHEAD_STEP: usize = 20;
/// How often the Inbox is looked at while messages are downloaded ahead (IDLE and push wait).
#[cfg(not(test))]
const AHEAD_POLL: Duration = Duration::from_secs(30);
#[cfg(test)]
const AHEAD_POLL: Duration = Duration::from_millis(300);
/// The newest messages of a folder read on the server (a public folder not followed, a shared
/// mailbox not kept) listed when it's opened (Show more asks for more, as for any folder).
#[cfg(not(test))]
const ONLINE_WINDOW: usize = 500;
#[cfg(test)]
pub(crate) const ONLINE_WINDOW: usize = 25;
/// The most messages of a folder a search of the server brings in.
const FOUND_MAX: usize = 500;

impl Worker {
    fn new(acc: AccountConfig, auth: Arc<Mutex<Auth>>, store: Store, sink: Sink, opts: SyncOptions) -> Worker {
        Worker::with_numbering(acc, auth, store, sink, opts, true)
    }

    /// A worker; `numbering`: with the account's numbering of message ids (JMAP, Graph and
    /// Exchange), which an Exchange account's second connection (sending, automatic replies)
    /// never uses — read for it too, a big mailbox's was held in memory twice.
    fn with_numbering(acc: AccountConfig, auth: Arc<Mutex<Auth>>, store: Store, sink: Sink, opts: SyncOptions, numbering: bool) -> Worker {
        // Counted before anything is read: a removal after it is seen.
        let removals = store.removals(&acc.id);
        // Not read: a zone a listing misses may have folders here this worker doesn't know of.
        let (folders, folders_unread) = match store.load_folders(&acc.id) {
            Ok(folders) => (folders, false),
            Err(e) => {
                log::warn!("{}: the folder list couldn't be read, nothing is let go of till a listing tells: {e}", acc.id);
                (Vec::new(), true)
            }
        };
        // Not read: the account's mail is set aside (nothing numbered from nothing, nothing saved),
        // and read again at each connection ([`Worker::reread_numbering`]).
        let (jstate, numbering_unread) = if acc.is_api() && numbering {
            match jmap::State::load(&store, &acc.id) {
                Ok(state) => (state, None),
                Err(e) => {
                    log::warn!("{}: the message numbering couldn't be read, the account's mail is set aside: {e}", acc.id);
                    (jmap::State::default(), Some(e.to_string()))
                }
            }
        } else {
            (jmap::State::default(), None)
        };
        // Online only (issue #4): nothing downloaded in full ahead of reading; a message comes
        // when it's opened, and is held in memory only.
        let opts = if acc.keeps_online() { SyncOptions { prefetch: 0, full: false, ..opts } } else { opts };
        let windows = store.load_account_doc(&acc.id, WINDOWS).unwrap_or_default();
        let followed = Worker::followed_doc(&store, &acc.id);
        let rest = Arc::new(rest::Rest::new(store.clone(), sink.clone()));
        Worker {
            acc,
            auth,
            main: None,
            store,
            sink,
            opts,
            conn: None,
            jstate,
            search_numbers: Vec::new(),
            folders,
            windows,
            followed_unread: followed.is_none(),
            followed: followed.unwrap_or_default(),
            ahead: VecDeque::new(),
            ahead_left: None,
            ahead_todo: crate::ews::AheadTodo::default(),
            room_again: None,
            ahead_poll: Instant::now(),
            queue: VecDeque::new(),
            backoff: Duration::from_secs(5),
            next_attempt: Instant::now(),
            last_full: None,
            auth_failed: false,
            token_retried: false,
            dav_waits: false,
            dav_asked: BTreeSet::new(),
            closed: false,
            creds: None,
            sieve_port: crate::sieve::PORT,
            no_push: false,
            pushed: false,
            inbox_synced: None,
            soon: Vec::new(),
            search: Arc::new(AtomicU64::new(0)),
            next_poll: Instant::now() + POLL_EVERY,
            waiting: None,
            listen: listening::Listening::default(),
            rest,
            opened: None,
            gone_meanwhile: Vec::new(),
            unlisted_kept: HashSet::new(),
            folders_unread,
            numbering_unread,
            removals,
            alive: Arc::default(),
        }
    }

    fn id(&self) -> String {
        self.acc.id.clone()
    }

    fn status(&self, status: SyncStatus) {
        // The second connection's comings and goings aren't the account's.
        if self.main.is_none() {
            match &status {
                SyncStatus::Error { error } => log::warn!("{}: {error}", self.acc.id),
                SyncStatus::NeedsPassword { error } => log::warn!("{}: needs signing in again: {error}", self.acc.id),
                SyncStatus::Online { .. } | SyncStatus::Syncing { .. } => {}
                s => log::info!("{}: {s:?}", self.acc.id),
            }
            self.sink.emit(Event::Status { account: self.id(), status });
        }
    }

    fn notice(&self, message: impl Into<String>) {
        self.sink.emit(Event::Notice { account: self.id(), message: message.into() });
    }

    fn drain(&mut self, rx: &Receiver<Job>) {
        loop {
            match rx.try_recv() {
                Ok(j) => push_job(&mut self.queue, j),
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.closed = true;
                    return;
                }
            }
        }
    }
}

/// What the account's connection did, for the log: statuses, sending, failures and what a
/// calendar sync brought (never a message's content).
fn log_event(e: &Event) {
    match e {
        Event::Status { account, status } => log::info!("{account}: {status:?}"),
        Event::Folders { account, folders } => log::info!("{account}: {} folders", folders.len()),
        Event::Sent { account, id } => log::info!("{account}: message {id} sent"),
        Event::SendFailed { account, id, error } => log::warn!("{account}: message {id} not sent: {error}"),
        Event::Queued { account, id, error } => log::warn!("{account}: message {id} waits in the Outbox: {error}"),
        Event::OpFailed { account, op, error } => log::warn!("{account}: a change failed ({}): {error}", op.summary()),
        Event::OpsSetAside { account, count } => log::warn!("{account}: {count} changes waiting kept aside, never sent"),
        // Logged where it's asked for, with the time it took: one line per message.
        Event::TextFirst { .. } => {}
        Event::BodyFailed { key, error } => log::warn!("{}: a message didn't download: {error}", key.account),
        Event::Vacation { account, error: Some(error), .. } => log::warn!("{account}: automatic replies: {error}"),
        Event::Voted { key, error: Some(error), .. } => log::warn!("{}: vote not sent: {error}", key.account),
        Event::Rooms { account, lists, rooms, error } => match error {
            Some(e) => log::warn!("{account}: rooms: {e}"),
            None => log::info!("{account}: {} room lists, {} rooms", lists.len(), rooms.len()),
        },
        Event::Availability { account, people, error, .. } => match error {
            Some(e) => log::warn!("{account}: availability: {e}"),
            None => log::info!("{account}: availability of {} read ({} refused)", people.len(), people.iter().filter(|p| p.error.is_some()).count()),
        },
        Event::FolderSizes { account, server, error: Some(e), .. } => {
            log::warn!("{account}: folder sizes ({}): {e}", if *server { "server" } else { "here" })
        }
        Event::Notice { account, message } => log::info!("{account}: {message}"),
        Event::DavSynced { account, sync } => {
            let names: Vec<&str> = sync.calendars.iter().flatten().map(|c| c.name.as_str()).collect();
            match (&sync.calendars, &sync.tasks) {
                (None, Some(tasks)) => log::info!("{account}: {} tasks", tasks.len()),
                _ => log::info!("{account}: calendars {names:?}, {} events", sync.events.len()),
            }
            for err in &sync.errors {
                log::warn!("{account}: {err}");
            }
        }
        _ => {}
    }
}

/// Jobs someone is looking at a spinner for, done between two folders of a full sync rather
/// than after it (the first sync of a large Exchange mailbox can take many minutes): an opened
/// message, automatic replies, a calendar just opened, a folder asked for (Send / Receive's
/// Inbox).
/// The order a full sync walks the folders in: the Inbox first (the mailbox is usable as soon as
/// it is in), then what mail is about, the rest, the archive, and the public folders followed
/// last — an account with public folders and years of archives takes minutes, and the Inbox
/// must not wait for them, nor the user's own folders named after "Public Folders". Folders not
/// `walked` (read on the server) are listed when opened; a folder of the online archive is never
/// walked, whatever `walked` says.
fn sync_order(folders: &[Folder], walked: impl Fn(&Folder) -> bool) -> Vec<String> {
    let rank = |f: &Folder| match f.role {
        _ if is_public_folder(&f.path) => 7,
        quatrelle_model::FolderRole::Inbox => 0,
        quatrelle_model::FolderRole::Drafts => 1,
        quatrelle_model::FolderRole::Sent => 2,
        quatrelle_model::FolderRole::Junk => 3,
        quatrelle_model::FolderRole::Trash => 4,
        quatrelle_model::FolderRole::Other => 5,
        quatrelle_model::FolderRole::Archive => 6,
    };
    let mut v: Vec<(&Folder, u8)> = folders.iter().filter(|f| f.selectable && !f.archive && walked(f)).map(|f| (f, rank(f))).collect();
    v.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.path.cmp(&b.0.path)));
    v.into_iter().map(|(f, _)| f.path.clone()).collect()
}

fn jumps_queue(j: &Job) -> bool {
    matches!(
        j,
        Job::FetchBody { .. }
            | Job::FetchTextFirst { .. }
            | Job::ReadOnline { .. }
            | Job::ReadArchiveTag { .. }
            | Job::FetchPhoto { .. }
            | Job::FetchEventFile { .. }
            | Job::GetServerRules { .. }
            | Job::GetVacation { .. }
            | Job::SetVacation { .. }
            | Job::ServerFolderSizes { .. }
            | Job::ListRecoverable { .. }
            | Job::RestoreRecoverable { .. }
            | Job::FindCertificates { .. }
            | Job::SearchDirectory { .. }
            | Job::ExpandList { .. }
            | Job::SyncDav { .. }
            | Job::SyncCalendars { .. }
            | Job::SyncFolder { .. }
    )
}

/// How many messages one server-side search answers with.
const SEARCH_LIMIT: usize = 200;

#[cfg(test)]
mod tests;
