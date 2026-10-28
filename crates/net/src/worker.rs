//! Per-account workers.
//!
//! [`Pool`] owns one thread per account. Each thread keeps an IMAP connection, runs the
//! engine's jobs in order, replays changes made offline, syncs every folder on connect and
//! periodically, and otherwise IDLEs on the Inbox so new mail shows up at once. Failures never
//! take the app down: a broken connection is retried with exponential back-off, a refused
//! password stops retrying until the user enters a new one.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use quatrelle_model::job::Secret;
use quatrelle_model::{
    AccountConfig, Event, FOLLOWED_PUBLIC_DOC, Flag, Folder, FolderRole, Job, MessageKey, Op, SyncStatus, is_public_folder, sort_folders,
};
use quatrelle_store::{OutboxItem, Store};

use crate::imap::{Client, uid_set};
use crate::jmap;
use crate::stream::Stream;
use crate::sync::SyncReport;
use crate::sync::{SyncOptions, fetch_body, sync_folder};
use crate::{NetError, Result, smtp};

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
}

/// Sends events and wakes the UI.
#[derive(Clone)]
struct Sink {
    tx: Sender<Event>,
    wake: Arc<Mutex<Option<Arc<dyn Fn() + Send + Sync>>>>,
}

impl Sink {
    fn emit(&self, e: Event) {
        log_event(&e);
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
    if matches!(j, Job::SyncFolder { .. } | Job::SyncDav { .. }) && queue.contains(&j) {
        return;
    }
    // Calendars just ticked: the sync waiting reads them too, else those waiting take them in.
    if let Job::SyncCalendars { account, calendars } = &j {
        if queue.iter().any(|q| matches!(q, Job::SyncDav { account: a } if a == account)) {
            return;
        }
        if let Some(Job::SyncCalendars { calendars: waiting, .. }) =
            queue.iter_mut().find(|q| matches!(q, Job::SyncCalendars { account: a, .. } if a == account))
        {
            for c in calendars {
                if !waiting.contains(c) {
                    waiting.push(c.clone());
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
/// written waits on (rooms, free/busy), which must not sit behind a long sync.
fn quick_job(j: &Job, acc: &AccountConfig) -> bool {
    match j {
        Job::Send { .. } => acc.pop.is_none() && acc.jmap.is_none(),
        Job::GetVacation { .. } | Job::SetVacation { .. } => acc.pop.is_none(),
        Job::GetRooms { .. } | Job::GetAvailability { .. } => acc.ews.is_some() || acc.graph.is_some(),
        Job::ServerFolderSizes { .. } | Job::FindCertificates { .. } | Job::SearchDirectory { .. } => acc.ews.is_some(),
        _ => false,
    }
}

/// Jobs a full sync serves between two folders rather than at its end: what someone is waiting
/// on (an opened message) and the edits they just made.
fn between_folders(j: &Job) -> bool {
    matches!(j, Job::FetchBody { .. } | Job::Apply { .. })
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
    /// Working offline: no account connects until it's turned off.
    offline: bool,
    /// The server search still wanted ([`Job::SearchServer`]'s `search`; 0: none), shared with
    /// every worker so a search running stops when it's no longer wanted.
    search: Arc<AtomicU64>,
    /// Edits to the cache, written on a thread of their own ([`edit_writer`]): marking a message
    /// read rewrote its folder's list on the window's thread, seconds for a folder of a hundred
    /// thousand messages, at every message opened. `None` once the pool shuts down.
    edits: Option<Sender<Edit>>,
    edits_thread: Option<JoinHandle<()>>,
}

/// An edit for the writer: the account, the change, and the account's thread to tell after.
type Edit = (String, Op, Option<Sender<Job>>);

/// Write an edit to the cache and journal it (ops.json), report what changed, then hand it to
/// the account's thread, which takes it to the server. Journaled first: a sync saving the folder
/// meanwhile makes the edit again on what it saves (`Store::save_folder`) rather than putting the
/// old flags back over it.
fn write_edit(store: &Store, sink: &Sink, account: String, op: Op, worker: Option<Sender<Job>>) {
    if let Err(e) = store.add_op(&account, op.clone()) {
        sink.emit(Event::Notice { account: account.clone(), message: e.to_string() });
    }
    match store.apply_op(&account, &op) {
        Ok(changed) => {
            if op.is_folder_op() {
                let folders = store.load_folders(&account).unwrap_or_default();
                sink.emit(Event::Folders { account: account.clone(), folders });
            }
            for folder in changed {
                sink.emit(Event::FolderChanged { account: account.clone(), folder });
            }
        }
        Err(e) => sink.emit(Event::Notice { account: account.clone(), message: e.to_string() }),
    }
    // Wake the account's thread: it uploads the queue at its next opportunity.
    if let Some(w) = worker {
        let _ = w.send(Job::Apply { account, op });
    }
}

/// The thread writing edits, in the order they were made, until the pool shuts down.
fn edit_writer(store: Store, sink: Sink, rx: Receiver<Edit>) {
    for (account, op, worker) in rx {
        write_edit(&store, &sink, account, op, worker);
    }
}

/// Drafts that couldn't be saved to the server (offline, or it failed), kept with the account
/// until it connects again.
const PENDING_DRAFTS: &str = "drafts-pending.json";

#[derive(serde::Serialize, serde::Deserialize)]
struct PendingDraft {
    folder: String,
    #[serde(default)]
    replaces: Option<quatrelle_model::MessageKey>,
    raw: Vec<u8>,
}

/// Keep a draft on this computer for the next connection.
fn keep_draft(store: &Store, account: &str, folder: &str, replaces: Option<quatrelle_model::MessageKey>, raw: Vec<u8>) -> Result<()> {
    let mut list: Vec<PendingDraft> = store.load_account_doc(account, PENDING_DRAFTS).unwrap_or_default();
    // A newer version of the same draft replaces the one kept.
    if let Some(r) = &replaces {
        list.retain(|d| d.replaces.as_ref() != Some(r));
    }
    list.push(PendingDraft { folder: folder.to_string(), replaces, raw });
    store.save_account_doc(account, PENDING_DRAFTS, &list).map_err(NetError::from)
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

mod ahead;
mod changes;
mod connect;
mod listening;
mod pool;
mod scheduling;
mod send;
mod sizes;
mod smime;
mod sync;

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
const SENT_SOON: Duration = Duration::from_secs(3);
const SENT_LATER: Duration = Duration::from_secs(15);
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
    closed: bool,
    /// Where a rotated OAuth refresh token is saved.
    creds: Option<Arc<dyn Credentials>>,
    /// The ManageSieve port (4190; tests use their own).
    sieve_port: u16,
    /// The JMAP server can't push: poll instead.
    no_push: bool,
    /// The last push wait ended in an Inbox sync (so the next one needn't check first).
    pushed: bool,
    /// Folders to look at again soon: Graph and Exchange file a sent message's copy a moment
    /// after taking it, so Sent Items is read again then rather than at the next poll.
    soon: Vec<(Instant, String)>,
    /// Folders asked to keep more than the setting (Show more on the server): how many of each
    /// (`windows.json` with the account).
    windows: HashMap<String, usize>,
    /// The public folders kept in sync (the user's Favorites among them, written by the session
    /// as [`FOLLOWED_PUBLIC_DOC`]); the others are listed only when opened.
    followed: BTreeSet<String>,
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
        let folders = store.load_folders(&acc.id).unwrap_or_default();
        let jstate = if acc.is_api() && numbering { jmap::State::load(&store, &acc.id) } else { jmap::State::default() };
        // Online only (issue #4): nothing downloaded in full ahead of reading; a message comes
        // when it's opened, and is held in memory only.
        let opts = if acc.keeps_online() { SyncOptions { prefetch: 0, full: false, ..opts } } else { opts };
        let windows = store.load_account_doc(&acc.id, WINDOWS).unwrap_or_default();
        let followed = store.load_account_doc(&acc.id, FOLLOWED_PUBLIC_DOC).unwrap_or_default();
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
            followed,
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
            closed: false,
            creds: None,
            sieve_port: crate::sieve::PORT,
            no_push: false,
            pushed: false,
            soon: Vec::new(),
            search: Arc::new(AtomicU64::new(0)),
            next_poll: Instant::now() + POLL_EVERY,
            listen: listening::Listening::default(),
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

    fn run(mut self, rx: Receiver<Job>) {
        self.status(SyncStatus::Connecting);
        while !self.closed {
            self.drain(&rx);
            if self.closed {
                break;
            }
            self.fresh_token();
            if let Some(job) = self.queue.pop_front() {
                self.handle(job);
                continue;
            }
            if self.conn.is_none() {
                if self.auth_failed {
                    match rx.recv() {
                        Ok(j) => push_job(&mut self.queue, j),
                        Err(_) => break,
                    }
                    continue;
                }
                let wait = self.next_attempt.saturating_duration_since(Instant::now());
                if !wait.is_zero() {
                    match rx.recv_timeout(wait) {
                        Ok(j) => push_job(&mut self.queue, j),
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                    continue;
                }
                self.connect_and_sync();
                continue;
            }
            if self.last_full.is_none_or(|t| t.elapsed() >= FULL_SYNC_EVERY) {
                self.sync_all(&rx);
                continue;
            }
            // Nothing asked: a few more messages downloaded whole, with a look at the Inbox every
            // half minute meanwhile.
            if !self.ahead.is_empty() && self.room_to_download() {
                // Told of changes, what changed is read between two steps.
                if self.listening() {
                    self.hear(Duration::ZERO);
                }
                if Instant::now() < self.ahead_poll {
                    self.ahead_step();
                } else {
                    self.look_between_downloads();
                }
                continue;
            }
            self.wait_for_mail(&rx);
        }
        if let Some(Conn::Imap(c)) = self.conn.as_mut() {
            c.logout();
        }
    }
}

/// Group keys by folder, select each folder, and skip keys whose UIDVALIDITY is stale.
fn by_folder<S: std::io::Read + std::io::Write>(
    c: &mut Client<S>,
    keys: &[MessageKey],
    mut f: impl FnMut(&mut Client<S>, &str, &str) -> Result<()>,
) -> Result<()> {
    let mut folders: Vec<&str> = keys.iter().map(|k| k.folder.as_str()).collect();
    folders.sort_unstable();
    folders.dedup();
    for folder in folders {
        let info = c.select(folder, false)?;
        let uids: Vec<u32> = keys.iter().filter(|k| k.folder == folder && k.validity == info.validity).map(|k| k.uid).collect();
        if uids.is_empty() {
            continue;
        }
        for chunk in crate::imap::uid_chunks(&uids, 500) {
            f(c, folder, &uid_set(&chunk))?;
        }
    }
    Ok(())
}

/// Permanent delete. On Gmail (`X-GM-EXT-1`) an EXPUNGE in a label folder merely archives to
/// All Mail, so those messages are first moved into Trash (COPYUID gives their new UIDs) and
/// expunged in Trash.
fn expunge_keys<S: std::io::Read + std::io::Write>(c: &mut Client<S>, keys: &[MessageKey], trash: Option<&str>) -> Result<()> {
    let to_trash = trash.filter(|_| c.has("X-GM-EXT-1"));
    let mut in_trash: Vec<u32> = Vec::new();
    by_folder(c, keys, |c, folder, set| match to_trash {
        Some(t) if folder != t => {
            let uids = c.uid_move_to(set, t)?;
            if uids.is_empty() {
                log::info!("{folder}: moved to {t} for deleting, but the server didn't say where");
            }
            in_trash.extend(uids);
            Ok(())
        }
        _ => {
            c.uid_store(set, true, Flag::Deleted.imap())?;
            c.expunge(set)
        }
    })?;
    if let (Some(t), false) = (to_trash, in_trash.is_empty()) {
        c.select(t, false)?;
        for chunk in crate::imap::uid_chunks(&in_trash, 500) {
            let set = uid_set(&chunk);
            c.uid_store(&set, true, Flag::Deleted.imap())?;
            c.expunge(&set)?;
        }
    }
    Ok(())
}

/// Apply one change on the server.
pub fn apply_remote<S: std::io::Read + std::io::Write>(c: &mut Client<S>, op: &Op) -> Result<()> {
    apply_remote_with(c, op, None)
}

/// [`apply_remote`] with the account's Trash path, which permanent deletes on Gmail need.
pub fn apply_remote_with<S: std::io::Read + std::io::Write>(c: &mut Client<S>, op: &Op, trash: Option<&str>) -> Result<()> {
    match op {
        Op::SetFlag { keys, flag, on } => by_folder(c, keys, |c, _, set| c.uid_store(set, *on, flag.imap())),
        // A keyword that isn't an atom would make the command malformed: the engine never sends one.
        Op::SetKeyword { keys, keyword, on, .. } if quatrelle_model::is_keyword(keyword) => {
            by_folder(c, keys, |c, _, set| c.uid_store(set, *on, keyword))
        }
        Op::SetKeyword { .. } => Ok(()),
        Op::Move { keys, to } => by_folder(c, keys, |c, folder, set| if folder == to { Ok(()) } else { c.uid_move(set, to) }),
        Op::Expunge { keys } => expunge_keys(c, keys, trash),
        Op::CreateFolder { path } => c.create(path),
        Op::RenameFolder { from, to } => c.rename(from, to),
        Op::DeleteFolder { path } => c.delete(path),
        Op::EmptyFolder { path } => c.empty(path),
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
        Event::OpFailed { account, error, .. } => log::warn!("{account}: a change failed: {error}"),
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
/// `walked` (read on the server) are listed when opened.
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
    let mut v: Vec<(&Folder, u8)> = folders.iter().filter(|f| f.selectable && walked(f)).map(|f| (f, rank(f))).collect();
    v.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.path.cmp(&b.0.path)));
    v.into_iter().map(|(f, _)| f.path.clone()).collect()
}

/// What a connection of its own sends back while it reads folders.
enum SyncDone {
    /// A folder read, and what changed there (or why not, as it was: a connection that timed
    /// out must stop the sync, not pass for "no such folder"); with the numbers given meanwhile.
    Folder(String, std::result::Result<SyncReport, NetError>, Vec<(String, u32)>),
    /// The numbers it gave while reading, when it's done.
    State(Box<jmap::State>),
    /// It couldn't connect: this account syncs on one connection.
    Refused,
}

/// Open a connection like the account's main one, for a folder reader of its own.
fn connect_for_sync(acc: &AccountConfig, store: &Store, secret: &str) -> Result<Conn> {
    let user = if acc.username.is_empty() { acc.email.clone() } else { acc.username.clone() };
    if let Some(url) = &acc.graph {
        return Ok(Conn::Graph(crate::graph::Client::connect(url, secret)?));
    }
    if acc.pop.is_some() {
        return Err(NetError::No("POP accounts read mail on one connection".into()));
    }
    if let Some(url) = &acc.ews {
        let url = if url == crate::ews::AUTODISCOVER {
            let kept: String = store.load_account_doc(&acc.id, "ews.json").unwrap_or_default();
            if kept.is_empty() {
                let found = crate::ews::autodiscover(&acc.email, &user, secret, None)?;
                let _ = store.save_account_doc(&acc.id, "ews.json", &found);
                found
            } else {
                kept
            }
        } else {
            url.clone()
        };
        let c = crate::ews::Client::connect(&url, &user, secret, &acc.email)?;
        c.set_shared(&acc.shared_mailboxes);
        return Ok(Conn::Ews(Box::new(c)));
    }
    if let Some(url) = &acc.jmap {
        return Ok(Conn::Jmap(jmap::Client::connect(url, &user, secret, acc.oauth.is_some())?));
    }
    let mut c = Client::connect(&acc.imap)?;
    if acc.oauth.is_some() {
        c.login_xoauth2(&user, secret)?;
    } else {
        c.login(&user, secret)?;
    }
    Ok(Conn::Imap(c))
}

/// Read folders from a shared list on a connection of its own, reporting each as it's read.
fn sync_folders(
    acc: AccountConfig,
    store: Store,
    secret: String,
    mut state: jmap::State,
    queue: Arc<Mutex<VecDeque<(String, SyncOptions)>>>,
    tx: Sender<SyncDone>,
) {
    let mut conn = match connect_for_sync(&acc, &store, &secret) {
        Ok(c) => c,
        Err(e) => {
            log::info!("{}: a second connection wasn't taken ({e}): the rest syncs on the first", acc.id);
            let _ = tx.send(SyncDone::Refused);
            let _ = tx.send(SyncDone::State(Box::new(state)));
            return;
        }
    };
    while let Some((path, opts)) = queue.lock().ok().and_then(|mut q| q.pop_front()) {
        let r = match &mut conn {
            Conn::Imap(c) => sync_folder(c, &store, &acc.id, &path, &opts),
            Conn::Jmap(c) => jmap::sync_folder(c, &mut state, &store, &acc.id, &path, &opts),
            Conn::Graph(c) => crate::graph::sync_folder(c, &mut state, &store, &acc.id, &path, &opts),
            Conn::Ews(c) => crate::ews::sync_folder(c, &mut state, &store, &acc.id, &path, &opts),
            Conn::Pop => Err(NetError::No("POP accounts read mail on one connection".into())),
        };
        let stop = r.as_ref().is_err_and(reader_stops);
        if tx.send(SyncDone::Folder(path, r, state.take_given())).is_err() {
            break;
        }
        if stop {
            break;
        }
    }
    if let Conn::Imap(mut c) = conn {
        c.logout();
    }
    let _ = tx.send(SyncDone::State(Box::new(state)));
}

/// A reader connection's error that ends it (the main connection reads its folder instead): its
/// connection is gone or its sign-in refused.
fn reader_stops(e: &NetError) -> bool {
    e.is_fatal() || matches!(e, NetError::Auth(_))
}

fn jumps_queue(j: &Job) -> bool {
    matches!(
        j,
        Job::FetchBody { .. }
            | Job::FetchPhoto { .. }
            | Job::GetServerRules { .. }
            | Job::GetVacation { .. }
            | Job::SetVacation { .. }
            | Job::ServerFolderSizes { .. }
            | Job::FindCertificates { .. }
            | Job::SearchDirectory { .. }
            | Job::SyncDav { .. }
            | Job::SyncCalendars { .. }
            | Job::SyncFolder { .. }
    )
}

/// How many messages one server-side search answers with.
const SEARCH_LIMIT: usize = 200;

#[cfg(test)]
mod tests;
