//! Per-account workers.
//!
//! [`Pool`] owns one thread per account. Each thread keeps an IMAP connection, runs the
//! engine's jobs in order, replays changes made offline, syncs every folder on connect and
//! periodically, and otherwise IDLEs on the Inbox so new mail shows up at once. Failures never
//! take the app down: a broken connection is retried with exponential back-off, a refused
//! password stops retrying until the user enters a new one.

use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use quatrelle_model::job::Secret;
use quatrelle_model::{AccountConfig, Event, Flag, Folder, FolderRole, Job, MessageKey, Op, SyncStatus, sort_folders};
use quatrelle_store::{OutboxItem, Store};

use crate::imap::{Client, uid_set};
use crate::jmap;
use crate::stream::Stream;
use crate::sync::{SyncOptions, SyncReport, fetch_body, sync_folder};
use crate::{NetError, Result, smtp};

/// How many messages one server-side search answers with.
const SEARCH_LIMIT: usize = 200;

/// Where passwords live. The desktop app backs this with the system keychain.
pub trait Credentials: Send + Sync {
    fn get(&self, account: &AccountConfig) -> Option<String>;
    fn set(&self, account: &AccountConfig, password: &str) -> std::result::Result<(), String>;
    fn delete(&self, account: &AccountConfig);
    /// The outgoing server's own password (accounts with an `smtp_username`), kept apart.
    fn get_smtp(&self, account: &AccountConfig) -> Option<String>;
    fn set_smtp(&self, account: &AccountConfig, password: &str) -> std::result::Result<(), String>;
    fn delete_smtp(&self, account: &AccountConfig);
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
/// row each ask for Sent Items: once is enough).
fn push_job(queue: &mut VecDeque<Job>, j: Job) {
    if matches!(j, Job::SyncFolder { .. }) && queue.contains(&j) {
        return;
    }
    // A newer search makes a waiting one pointless.
    if let Job::Search { account, .. } = &j {
        queue.retain(|q| !matches!(q, Job::Search { account: a, .. } if a == account));
    }
    queue.push_back(j);
}

/// Whether a job goes on the account's second connection: sending (not over JMAP, whose filing
/// needs the main connection's mailbox ids) and automatic replies.
fn quick_job(j: &Job, acc: &AccountConfig) -> bool {
    match j {
        Job::Send { .. } => acc.pop.is_none() && acc.jmap.is_none(),
        Job::GetVacation { .. } | Job::SetVacation { .. } => acc.pop.is_none(),
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

impl Pool {
    pub fn new(store: Store, creds: Arc<dyn Credentials>) -> Pool {
        let (tx, rx) = mpsc::channel();
        Pool {
            store,
            creds,
            sink: Sink { tx, wake: Arc::new(Mutex::new(None)) },
            rx,
            workers: HashMap::new(),
            configs: HashMap::new(),
            opts: SyncOptions::default(),
            sieve_port: crate::sieve::PORT,
            fetcher: None,
            offline: false,
        }
    }

    /// Browser sign-in for an OAuth account, on its own thread: the page opens through the UI
    /// (`Event::OpenUrl`), the refresh token goes to the keychain, then the account starts.
    fn sign_in(&mut self, account: AccountConfig) {
        let Some(p) = crate::oauth::provider_for_account(&account) else {
            let error = format!("signing in with {} isn't set up in this copy of Quatrelle", account.oauth.as_deref().unwrap_or("that provider"));
            return self.sink.emit(Event::Status { account: account.id, status: SyncStatus::NeedsPassword { error } });
        };
        let (sink, creds) = (self.sink.clone(), self.creds.clone());
        let spawned = std::thread::Builder::new().name("mail-sign-in".into()).spawn(move || {
            let mut open = |url: &str| -> std::result::Result<(), String> {
                sink.emit(Event::OpenUrl { url: url.to_string() });
                Ok(())
            };
            match crate::oauth::authorize(&p, &account.email, &mut open, Duration::from_secs(5 * 60)) {
                Ok(t) => match t.refresh {
                    Some(r) => match creds.set(&account, &r) {
                        Ok(()) => sink.emit(Event::SignedIn { account: account.id, user: t.user }),
                        Err(e) => sink.emit(Event::Status {
                            account: account.id,
                            status: SyncStatus::NeedsPassword { error: format!("the sign-in couldn't be kept in the keychain: {e}") },
                        }),
                    },
                    None => sink.emit(Event::Status {
                        account: account.id,
                        status: SyncStatus::NeedsPassword { error: format!("{} gave no lasting sign-in; try again", p.name) },
                    }),
                },
                Err(e) => sink.emit(Event::Status { account: account.id, status: SyncStatus::NeedsPassword { error: e.to_string() } }),
            }
        });
        if let Err(e) = spawned {
            log::warn!("sign-in: {e}");
        }
    }

    fn fetch_url(&mut self, url: String) {
        if self.fetcher.is_none() {
            let (tx, rx) = mpsc::channel::<String>();
            let sink = self.sink.clone();
            let spawned = std::thread::Builder::new().name("mail-pictures".into()).spawn(move || {
                for url in rx {
                    let r = crate::http::get(&url, MAX_PICTURE);
                    let e = match r {
                        Ok(resp) if resp.ok() => Event::Fetched { url, data: Some(resp.body), error: None },
                        Ok(resp) => Event::Fetched { url, data: None, error: Some(format!("HTTP {}", resp.status)) },
                        Err(e) => Event::Fetched { url, data: None, error: Some(e.to_string()) },
                    };
                    sink.emit(e);
                }
            });
            match spawned {
                Ok(_) => self.fetcher = Some(tx),
                Err(e) => return self.sink.emit(Event::Fetched { url, data: None, error: Some(e.to_string()) }),
            }
        }
        if let Some(tx) = &self.fetcher {
            let _ = tx.send(url);
        }
    }

    /// Export to a data file on a thread of its own.
    fn export_pst(&mut self, account: String, path: String, name: String) {
        let sink = self.sink.clone();
        let store = self.store.clone();
        let spawned = std::thread::Builder::new().name("mail-export".into()).spawn(move || {
            let mut progress = |messages| sink.emit(Event::Exported { account: account.clone(), messages, skipped: 0, done: false, error: None });
            let event = match crate::pst_export::export(&store, &account, &name, std::path::Path::new(&path), &mut progress) {
                Ok(done) => Event::Exported { account, messages: done.messages, skipped: done.skipped, done: true, error: None },
                Err(e) => Event::Exported { account, messages: 0, skipped: 0, done: true, error: Some(e) },
            };
            sink.emit(event);
        });
        if let Err(e) = spawned {
            self.sink.emit(Event::Notice { account: String::new(), message: format!("export: {e}") });
        }
    }

    /// Import a data file on a thread of its own: an Outlook one (`.pst`), or Quatrelle's own
    /// (`.quatrelle`, copied in as it is).
    fn import_pst(&mut self, account: String, path: String) {
        let sink = self.sink.clone();
        let store = self.store.clone();
        let spawned = std::thread::Builder::new().name("mail-import".into()).spawn(move || {
            if path.to_ascii_lowercase().ends_with(".quatrelle") {
                match store.import_data_file(&account, std::path::Path::new(&path)) {
                    Ok(messages) => {
                        let folders = store.load_folders(&account).unwrap_or_default();
                        let paths: Vec<String> = folders.iter().map(|f| f.path.clone()).collect();
                        sink.emit(Event::Folders { account: account.clone(), folders });
                        for folder in paths {
                            sink.emit(Event::FolderChanged { account: account.clone(), folder });
                        }
                        sink.emit(Event::Imported { account, messages, skipped: 0, done: true, error: None });
                    }
                    Err(e) => sink.emit(Event::Imported { account, messages: 0, skipped: 0, done: true, error: Some(e.to_string()) }),
                }
                return;
            }
            let mut progress = |messages| sink.emit(Event::Imported { account: account.clone(), messages, skipped: 0, done: false, error: None });
            let result = crate::pst::import(std::path::Path::new(&path), &store, &account, &mut progress);
            match result {
                Ok(done) => {
                    let paths: Vec<String> = done.folders.iter().map(|f| f.path.clone()).collect();
                    sink.emit(Event::Folders { account: account.clone(), folders: done.folders });
                    for folder in paths {
                        sink.emit(Event::FolderChanged { account: account.clone(), folder });
                    }
                    sink.emit(Event::Imported { account, messages: done.messages, skipped: done.skipped, done: true, error: None });
                }
                Err(e) => sink.emit(Event::Imported { account, messages: 0, skipped: 0, done: true, error: Some(e) }),
            }
        });
        if let Err(e) = spawned {
            self.sink.emit(Event::Notice { account: String::new(), message: format!("import: {e}") });
        }
    }

    /// Called (from worker threads) after every event, e.g. to request a UI repaint.
    pub fn set_waker(&self, f: Arc<dyn Fn() + Send + Sync>) {
        if let Ok(mut w) = self.sink.wake.lock() {
            *w = Some(f);
        }
    }

    pub fn submit(&mut self, job: Job) {
        match job {
            Job::FetchUrl { url } => self.fetch_url(url),
            Job::CheckUpdates => {
                let sink = self.sink.clone();
                let spawned = std::thread::Builder::new().name("mail-updates".into()).spawn(move || {
                    let (releases, error) = match crate::updates::releases() {
                        Ok(r) => (r, None),
                        Err(e) => (Vec::new(), Some(e.to_string())),
                    };
                    sink.emit(Event::Releases { releases, error });
                });
                if let Err(e) = spawned {
                    self.sink.emit(Event::Releases { releases: Vec::new(), error: Some(e.to_string()) });
                }
            }
            Job::SignIn { account } => self.sign_in(account),
            Job::Discover { email } => {
                let sink = self.sink.clone();
                let spawned = std::thread::Builder::new().name("mail-discover".into()).spawn(move || {
                    let found = crate::autoconfig::discover(&email);
                    let (config, source) = match found {
                        Some(f) => (Some(Box::new(f.config)), f.source),
                        None => (None, String::new()),
                    };
                    sink.emit(Event::Discovered { email, config, source });
                });
                if let Err(e) = spawned {
                    log::warn!("autoconfiguration: {e}");
                }
            }
            Job::ImportPst { account, path } => self.import_pst(account, path),
            Job::ExportPst { account, path, name } => self.export_pst(account, path, name),
            Job::SetSync { messages, offline } => {
                let opts =
                    SyncOptions { window: if messages == 0 { usize::MAX } else { messages as usize }, prefetch: offline as usize, ..self.opts };
                if opts.window != self.opts.window || opts.prefetch != self.opts.prefetch {
                    self.opts = opts;
                    // Connected accounts start again with it (a rare change, from Options).
                    let running: Vec<AccountConfig> = self.workers.keys().filter_map(|id| self.configs.get(id).cloned()).collect();
                    for c in running {
                        self.start(c, None);
                    }
                }
            }
            Job::WorkOffline { on } => {
                self.offline = on;
                if on {
                    let ids: Vec<String> = self.workers.keys().cloned().collect();
                    for id in ids {
                        self.stop(&id);
                        self.sink.emit(Event::Status { account: id, status: SyncStatus::Offline });
                    }
                } else {
                    let configs: Vec<AccountConfig> = self.configs.values().cloned().collect();
                    for c in configs {
                        self.start(c, None);
                    }
                }
            }
            Job::Start { account, password } if self.offline => {
                // Working offline: remembered (and its password kept) for when that's turned off.
                if let Some(p) = &password
                    && let Err(e) = self.creds.set(&account, &p.0)
                {
                    self.sink.emit(Event::Notice {
                        account: account.id.clone(),
                        message: format!("the password could not be saved in the keychain: {e}"),
                    });
                }
                self.sink.emit(Event::Status { account: account.id.clone(), status: SyncStatus::Offline });
                self.configs.insert(account.id.clone(), account);
            }
            Job::Start { account, password } => self.start(account, password),
            Job::SmtpPassword { account, password } => {
                if let Err(e) = self.creds.set_smtp(&account, &password.0) {
                    self.sink.emit(Event::Notice {
                        account: account.id.clone(),
                        message: format!("the outgoing server's password could not be saved in the keychain: {e}"),
                    });
                }
            }
            Job::Remove { account } => {
                self.stop(&account);
                if let Some(c) = self.configs.remove(&account) {
                    self.creds.delete(&c);
                    self.creds.delete_smtp(&c);
                }
            }
            Job::Search { account, query, .. } if !self.workers.contains_key(&account) => {
                // No connection: the local index's answer stands.
                self.sink.emit(Event::Found { account, query, items: Vec::new(), error: Some("the account is offline".into()) });
            }
            Job::FetchBody { key } if !self.workers.contains_key(&key.account) => {
                // Offline or not started: the cache may still have it.
                match self.store.read_body(&key) {
                    Some(raw) => self.sink.emit(Event::Body { key, raw }),
                    None => self.sink.emit(Event::BodyFailed { key, error: "not downloaded yet, and the account is offline".into() }),
                }
            }
            Job::Send { account, id, raw, from, recipients, sent_folder } if !self.workers.contains_key(&account) => {
                // Not connected: straight into the Outbox.
                let item = OutboxItem { id, from, recipients, sent_folder, queued: now(), attempts: 0, error: Some("offline".into()), not_before: 0 };
                match self.store.outbox_add(&account, &item, &raw) {
                    Ok(()) => self.sink.emit(Event::Queued { account, id, error: "offline".into() }),
                    Err(e) => self.sink.emit(Event::SendFailed { account, id, error: e.to_string() }),
                }
            }
            // Offline: changes wait in the cache for the next sync.
            Job::SyncDav { account } if !self.workers.contains_key(&account) => {}
            // Working offline: nothing connects, by choice.
            Job::SyncAll { account } | Job::SyncFolder { account, .. } | Job::FlushOutbox { account }
                if self.offline && !self.workers.contains_key(&account) => {}
            Job::SaveDraft { account, raw, folder, replaces } if !self.workers.contains_key(&account) => {
                match keep_draft(&self.store, &account, &folder, replaces, raw) {
                    Ok(()) => self.sink.emit(Event::Notice {
                        account,
                        message: "the draft is kept on this computer; it goes to Drafts when you're back online".into(),
                    }),
                    Err(e) => self.sink.emit(Event::Notice { account, message: format!("the draft couldn't be kept: {e}") }),
                }
            }
            Job::Vote { key, choice } if !self.workers.contains_key(&key.account) => {
                self.sink.emit(Event::Voted { key, choice, error: Some("the account is offline".into()) });
            }
            Job::GetVacation { account } | Job::SetVacation { account, .. } if !self.workers.contains_key(&account) => {
                self.sink.emit(Event::Vacation { account, vacation: None, error: Some("the account is offline".into()) });
            }
            // The change goes into the cache and ops.json here, on the caller's thread, before
            // the account thread is free: it may be minutes into a full sync. Waiting meant a
            // folder refresh could paint the server's old state over the edit, and closing the
            // app dropped it. A POP account's worker keeps its own bookkeeping, so it still goes
            // that way.
            Job::Apply { account, op } if self.records_edits(&account) => {
                match self.store.apply_op(&account, &op) {
                    Ok(changed) => {
                        if op.is_folder_op() {
                            let folders = self.store.load_folders(&account).unwrap_or_default();
                            self.sink.emit(Event::Folders { account: account.clone(), folders });
                        }
                        if let Err(e) = self.store.add_op(&account, op.clone()) {
                            self.sink.emit(Event::Notice { account: account.clone(), message: e.to_string() });
                        }
                        for folder in changed {
                            self.sink.emit(Event::FolderChanged { account: account.clone(), folder });
                        }
                    }
                    Err(e) => self.sink.emit(Event::Notice { account: account.clone(), message: e.to_string() }),
                }
                // Wake the account thread: it uploads the queue at its next opportunity.
                if let Some(h) = self.workers.get(&account) {
                    let _ = h.tx.send(Job::Apply { account, op });
                }
            }
            other => {
                let id = other.account().to_string();
                let quick = self.configs.get(&id).is_some_and(|c| quick_job(&other, c));
                match self.workers.get(&id) {
                    Some(h) => {
                        // Sending and automatic replies go on the second connection; if it has
                        // stopped, the main one does it (after its sync).
                        let other = match h.quick.as_ref().filter(|_| quick) {
                            Some(q) => match q.send(other) {
                                Ok(()) => return,
                                Err(mpsc::SendError(j)) => j,
                            },
                            None => other,
                        };
                        if h.tx.send(other).is_err() {
                            self.sink.emit(Event::Notice {
                                account: id,
                                message: "the account's connection stopped; restart it from the account menu".into(),
                            });
                        }
                    }
                    None => self.sink.emit(Event::Notice { account: id, message: "the account is not connected".into() }),
                }
            }
        }
    }

    /// Whether this side writes an account's edit to the cache and ops.json: always when no
    /// thread runs for it, and for a running one unless it is POP (whose thread does it, with
    /// bookkeeping of its own).
    fn records_edits(&self, account: &str) -> bool {
        match self.workers.get(account) {
            None => true,
            Some(_) => matches!(self.configs.get(account), Some(c) if c.pop.is_none()),
        }
    }

    /// Events reported since the last call.
    pub fn poll(&self) -> Vec<Event> {
        self.rx.try_iter().collect()
    }

    /// Block until an event arrives or `timeout` passes (for the CLI).
    pub fn wait(&self, timeout: Duration) -> Option<Event> {
        self.rx.recv_timeout(timeout).ok()
    }

    fn start(&mut self, account: AccountConfig, password: Option<Secret>) {
        self.stop(&account.id);
        if let Some(p) = &password
            && let Err(e) = self.creds.set(&account, &p.0)
        {
            self.sink.emit(Event::Notice { account: account.id.clone(), message: format!("the password could not be saved in the keychain: {e}") });
        }
        let pw = password.map(|p| p.0).or_else(|| self.creds.get(&account));
        let Some(pw) = pw else {
            self.sink.emit(Event::Status { account: account.id.clone(), status: SyncStatus::NeedsPassword { error: String::new() } });
            return;
        };
        let (tx, rx) = mpsc::channel();
        let auth = Auth::shared(pw);
        let mut w = Worker::new(account.clone(), auth.clone(), self.store.clone(), self.sink.clone(), self.opts);
        w.creds = Some(self.creds.clone());
        w.sieve_port = self.sieve_port;
        // The second connection: quiet (the account's status is the main one's), connecting
        // only when there's something to do.
        let (qtx, qrx) = mpsc::channel();
        let mut q = Worker::new(account.clone(), auth, self.store.clone(), self.sink.clone(), self.opts);
        q.creds = Some(self.creds.clone());
        q.sieve_port = self.sieve_port;
        q.main = Some(tx.clone());
        let quick_thread = std::thread::Builder::new().name(format!("mail-{}-quick", account.id)).spawn(move || q.run_quick(qrx)).ok();
        let quick = quick_thread.is_some().then_some(qtx);
        let name = format!("mail-{}", account.id);
        match std::thread::Builder::new().name(name).spawn(move || w.run(rx)) {
            Ok(thread) => {
                self.workers.insert(account.id.clone(), Handle { tx, thread: Some(thread), quick, quick_thread });
            }
            Err(e) => self.sink.emit(Event::Status { account: account.id.clone(), status: SyncStatus::Error { error: e.to_string() } }),
        }
        self.configs.insert(account.id.clone(), account);
    }

    fn stop(&mut self, account: &str) {
        if let Some(mut h) = self.workers.remove(account) {
            drop(h.tx);
            drop(h.quick.take());
            // The thread notices the closed channel within half a second (IDLE slices); don't
            // block the UI waiting for a slow network call to finish.
            for t in [h.thread.take(), h.quick_thread.take()].into_iter().flatten() {
                std::thread::spawn(move || {
                    let _ = t.join();
                });
            }
        }
    }

    pub fn shutdown(&mut self) {
        let ids: Vec<String> = self.workers.keys().cloned().collect();
        for id in ids {
            self.stop(&id);
        }
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Largest picture downloaded for a message.
const MAX_PICTURE: usize = 8 << 20;

const FULL_SYNC_EVERY: Duration = Duration::from_secs(15 * 60);
const IDLE_FOR: Duration = Duration::from_secs(10 * 60);
const POLL_EVERY: Duration = Duration::from_secs(120);
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
}

/// Where an account keeps its folders' own windows.
const WINDOWS: &str = "windows.json";

impl Worker {
    fn new(acc: AccountConfig, auth: Arc<Mutex<Auth>>, store: Store, sink: Sink, opts: SyncOptions) -> Worker {
        let folders = store.load_folders(&acc.id).unwrap_or_default();
        let jstate = if acc.is_api() { jmap::State::load(&store, &acc.id) } else { jmap::State::default() };
        // Online only (issue #4): nothing downloaded in full ahead of reading; a message comes
        // when it's opened, and is held in memory only.
        let opts = if acc.keeps_online() { SyncOptions { prefetch: 0, ..opts } } else { opts };
        let windows = store.load_account_doc(&acc.id, WINDOWS).unwrap_or_default();
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
            queue: VecDeque::new(),
            backoff: Duration::from_secs(5),
            next_attempt: Instant::now(),
            last_full: None,
            auth_failed: false,
            closed: false,
            creds: None,
            sieve_port: crate::sieve::PORT,
            no_push: false,
            pushed: false,
            soon: Vec::new(),
        }
    }

    /// What to sign in with: the password, or for OAuth accounts a current access token
    /// (refreshed from the stored refresh token when needed).
    fn secret(&mut self) -> Result<String> {
        // Held while refreshing: the account's other connection waits for the new token rather
        // than refreshing with the same old one.
        let mut auth = self.auth.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(id) = self.acc.oauth.clone() else { return Ok(auth.secret.clone()) };
        if let Some(t) = &auth.access
            && t.expires_at > Instant::now()
        {
            return Ok(t.access.clone());
        }
        let p = crate::oauth::provider_for_account(&self.acc)
            .ok_or_else(|| NetError::Auth(format!("signing in with {id} isn't set up in this copy of Quatrelle")))?;
        let t = crate::oauth::refresh(&p, &auth.secret)?;
        // Some providers hand out a new refresh token each time: keep the newest.
        if let Some(r) = t.refresh.clone().filter(|r| *r != auth.secret) {
            auth.secret = r.clone();
            if let Some(c) = &self.creds
                && let Err(e) = c.set(&self.acc, &r)
            {
                self.sink
                    .emit(Event::Notice { account: self.acc.id.clone(), message: format!("the new sign-in couldn't be saved in the keychain: {e}") });
            }
        }
        let access = t.access.clone();
        auth.access = Some(t);
        Ok(access)
    }

    /// What Microsoft's sign-in granted (its scopes), when known.
    fn granted(&self) -> String {
        self.auth.lock().unwrap_or_else(std::sync::PoisonError::into_inner).access.as_ref().map(|t| t.granted.clone()).unwrap_or_default()
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
            self.wait_for_mail(&rx);
        }
        if let Some(Conn::Imap(c)) = self.conn.as_mut() {
            c.logout();
        }
    }

    /// The account's second connection: sends mail and reads or changes the automatic replies,
    /// whatever the main connection is doing. It connects when there's something to do, and lets
    /// go of an idle connection after a while.
    fn run_quick(mut self, rx: Receiver<Job>) {
        loop {
            let job = match rx.recv_timeout(QUICK_IDLE) {
                Ok(j) => j,
                Err(RecvTimeoutError::Timeout) => {
                    self.let_go();
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => break,
            };
            match job {
                Job::Send { account, id, raw, from, recipients, sent_folder } => {
                    self.send_quick(Job::Send { account, id, raw, from, recipients, sent_folder });
                }
                j @ (Job::GetVacation { .. } | Job::SetVacation { .. }) => self.handle(j),
                // Anything else belongs to the main connection.
                other => {
                    if let Some(m) = &self.main {
                        let _ = m.send(other);
                    }
                }
            }
        }
        self.let_go();
    }

    /// Close an idle connection (servers drop them anyway).
    fn let_go(&mut self) {
        if let Some(Conn::Imap(c)) = self.conn.as_mut() {
            c.logout();
        }
        self.conn = None;
    }

    /// Send on the second connection. Sent: a copy filed in Sent Items (IMAP), and the main
    /// connection told to look there. Not sent for now (offline, asked to wait): handed to the
    /// main connection, which keeps it in the Outbox; refused: said.
    fn send_quick(&mut self, job: Job) {
        let Job::Send { id, raw, from, recipients, sent_folder, .. } = &job else { return };
        match self.deliver(raw, from, recipients, sent_folder.as_deref()) {
            Ok(()) => {
                log::info!("{}: message {id} sent to {} recipient(s)", self.acc.id, recipients.len());
                self.sink.emit(Event::Sent { account: self.id(), id: *id });
                self.file_sent_quick(raw, sent_folder.clone());
            }
            Err(e) if e.is_retryable() => {
                log::info!("{}: sending on the second connection: {e}; left to the main one", self.acc.id);
                self.conn = None;
                if let Some(m) = &self.main
                    && m.send(job.clone()).is_err()
                {
                    self.sink.emit(Event::SendFailed { account: self.id(), id: *id, error: e.to_string() });
                }
            }
            Err(e) => {
                if matches!(e, NetError::Closed(_) | NetError::Io(_)) {
                    self.conn = None;
                }
                self.sink.emit(Event::SendFailed { account: self.id(), id: *id, error: e.to_string() });
            }
        }
    }

    /// A sent message's copy: Exchange, Graph and JMAP file it themselves; IMAP servers get one
    /// (not Gmail, which files its own). Then the main connection reads Sent Items, now and, where
    /// the server files it a moment later, again then.
    fn file_sent_quick(&mut self, raw: &[u8], sent_folder: Option<String>) {
        let folder = sent_folder.or_else(|| self.folders.iter().find(|f| f.role == FolderRole::Sent).map(|f| f.path.clone()));
        let Some(folder) = folder else { return };
        if !self.acc.is_api() {
            let files_itself = self.acc.smtp.host.ends_with("gmail.com") || self.acc.smtp.host.ends_with("googlemail.com");
            if !files_itself {
                let r = (|| -> Result<()> {
                    if self.conn.is_none() {
                        self.connect()?;
                    }
                    self.client()?.append(&folder, "\\Seen", raw).map(|_| ())
                })();
                if let Err(e) = r {
                    self.conn = None;
                    self.notice(format!("sent, but no copy was saved in Sent Items: {e}"));
                    return;
                }
            }
        }
        let Some(main) = self.main.clone() else { return };
        let job = Job::SyncFolder { account: self.id(), folder: folder.clone() };
        let _ = main.send(job.clone());
        if self.acc.is_api() {
            // Graph and Exchange file the copy a moment after taking the message.
            let _ = std::thread::Builder::new().name("sent-later".into()).spawn(move || {
                for wait in [SENT_SOON, SENT_LATER.saturating_sub(SENT_SOON)] {
                    std::thread::sleep(wait);
                    if main.send(job.clone()).is_err() {
                        return;
                    }
                }
            });
        }
    }

    /// Wait for new mail: IDLE on the Inbox when the server supports it, else poll.
    fn wait_for_mail(&mut self, rx: &Receiver<Job>) {
        let inbox = self.inbox();
        if matches!(self.conn, Some(Conn::Jmap(_))) && !self.no_push && inbox.is_some() && self.wait_for_push(rx) {
            return;
        }
        let idle = matches!(&self.conn, Some(Conn::Imap(c)) if c.has("IDLE"));
        if !idle || inbox.is_none() {
            let next_soon = self.soon.iter().map(|(at, _)| *at).min();
            let wait = next_soon.map_or(POLL_EVERY, |at| at.saturating_duration_since(Instant::now()).min(POLL_EVERY));
            match rx.recv_timeout(wait) {
                Ok(j) => push_job(&mut self.queue, j),
                Err(RecvTimeoutError::Disconnected) => self.closed = true,
                Err(RecvTimeoutError::Timeout) if next_soon.is_some_and(|at| at <= Instant::now()) => {
                    let now = Instant::now();
                    let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.soon).into_iter().partition(|(at, _)| *at <= now);
                    self.soon = later;
                    let mut folders: Vec<String> = due.into_iter().map(|(_, f)| f).collect();
                    folders.dedup();
                    for f in folders {
                        self.sync_one(&f);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    if let Some(f) = inbox {
                        self.sync_one(&f);
                    }
                }
            }
            return;
        }
        let Some(inbox) = inbox else { return };
        let result = (|| -> Result<bool> {
            let Some(Conn::Imap(c)) = self.conn.as_mut() else { return Err(NetError::Closed("not connected".into())) };
            if c.selected() != Some(inbox.as_str()) {
                c.select(&inbox, false)?;
            }
            let queue = &mut self.queue;
            let closed = &mut self.closed;
            c.idle(IDLE_FOR, || {
                loop {
                    match rx.try_recv() {
                        Ok(j) => push_job(queue, j),
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            *closed = true;
                            break;
                        }
                    }
                }
                !queue.is_empty() || *closed
            })
        })();
        match result {
            Ok(true) => self.sync_one(&inbox),
            Ok(false) => {}
            Err(e) => self.failed(e),
        }
    }

    /// JMAP push: hold the server's event stream open (as IMAP holds IDLE), checking for jobs
    /// between short reads; a `state` event means something changed. `false` when the server
    /// can't push (the caller polls instead).
    fn wait_for_push(&mut self, rx: &Receiver<Job>) -> bool {
        let Some(Conn::Jmap(c)) = &self.conn else { return false };
        let mut es = match c.events() {
            Ok(Some(es)) => es,
            Ok(None) => {
                self.no_push = true;
                return false;
            }
            Err(e @ NetError::Auth(_)) => {
                self.failed(e);
                return true;
            }
            Err(e) => {
                // Push not working (a proxy, an old server): poll from now on.
                log::info!("{}: no JMAP push: {e}", self.acc.id);
                self.no_push = true;
                return false;
            }
        };
        // Mail that came between the last sync and this stream opening sends no event (unless
        // the server replays it, RFC 8620 §7.3): check once now that nothing will be missed.
        if !std::mem::take(&mut self.pushed)
            && let Some(f) = self.inbox()
        {
            self.sync_one(&f);
        }
        let end = Instant::now() + IDLE_FOR;
        let mut changed = false;
        while Instant::now() < end && !changed {
            self.drain(rx);
            if self.closed || !self.queue.is_empty() {
                return true;
            }
            match es.next(Duration::from_millis(500)) {
                Ok(Some(e)) => changed = e.event == "state",
                Ok(None) => {}
                Err(e) => {
                    // Dropped: reopened on the next wait.
                    log::info!("{}: JMAP push ended: {e}", self.acc.id);
                    break;
                }
            }
        }
        if changed && let Some(f) = self.inbox() {
            self.sync_one(&f);
            self.pushed = true;
        }
        true
    }

    fn inbox(&self) -> Option<String> {
        self.folders.iter().find(|f| f.role == FolderRole::Inbox && self.acc.folder_owner(&f.path).is_none()).map(|f| f.path.clone())
    }

    /// Note a failure; drop the connection when it is unusable.
    fn failed(&mut self, e: NetError) {
        log::warn!("{}: {e}", self.acc.id);
        match e {
            // Microsoft took the token but won't open the mailbox: nothing to retry until changed.
            NetError::Auth(m) | NetError::Bad(m) | NetError::No(m)
                if self.acc.oauth.as_deref() == Some("microsoft") && m.to_ascii_lowercase().contains("authenticated but not connected") =>
            {
                self.conn = None;
                self.auth_failed = true;
                self.status(SyncStatus::Error { error: microsoft_imap_closed(&self.acc.email, self.acc.oauth_user.as_deref()) });
            }
            // Microsoft's servers refuse passwords from mail apps: asking for it again only loops.
            NetError::Auth(m) if quatrelle_model::microsoft_password_account(&self.acc) => {
                self.conn = None;
                self.auth_failed = true;
                self.status(SyncStatus::Error { error: format!("{MICROSOFT_NO_PASSWORDS} ({m})") });
            }
            NetError::Auth(m) => {
                self.conn = None;
                self.auth_failed = true;
                self.status(SyncStatus::NeedsPassword { error: m });
            }
            e if e.is_fatal() => {
                self.conn = None;
                self.next_attempt = Instant::now() + self.backoff;
                self.backoff = (self.backoff * 2).min(Duration::from_secs(300));
                self.status(SyncStatus::Error { error: e.to_string() });
            }
            e => self.notice(e.to_string()),
        }
    }

    fn connect(&mut self) -> Result<()> {
        if self.conn.is_some() {
            return Ok(());
        }
        self.status(SyncStatus::Connecting);
        let secret = self.secret()?;
        let user = if self.acc.username.is_empty() { self.acc.email.clone() } else { self.acc.username.clone() };
        if let Some(url) = &self.acc.graph {
            self.conn = Some(Conn::Graph(crate::graph::Client::connect(url, &secret)?));
            self.backoff = Duration::from_secs(5);
            return Ok(());
        }
        if self.acc.pop.is_some() {
            crate::pop::open(&self.acc, &secret)?.quit()?;
            self.conn = Some(Conn::Pop);
            self.backoff = Duration::from_secs(5);
            return Ok(());
        }
        if let Some(url) = &self.acc.ews {
            // `autodiscover`: found the first time, then kept.
            let url = if url == crate::ews::AUTODISCOVER {
                let kept: String = self.store.load_account_doc(&self.acc.id, "ews.json").unwrap_or_default();
                if kept.is_empty() {
                    let found = crate::ews::autodiscover(&self.acc.email, &user, &secret, None)?;
                    let _ = self.store.save_account_doc(&self.acc.id, "ews.json", &found);
                    found
                } else {
                    kept
                }
            } else {
                url.clone()
            };
            let c = crate::ews::Client::connect(&url, &user, &secret, &self.acc.email)?;
            c.set_shared(&self.acc.shared_mailboxes);
            self.conn = Some(Conn::Ews(Box::new(c)));
            self.backoff = Duration::from_secs(5);
            return Ok(());
        }
        if let Some(url) = &self.acc.jmap {
            self.conn = Some(Conn::Jmap(jmap::Client::connect(url, &user, &secret, self.acc.oauth.is_some())?));
            self.backoff = Duration::from_secs(5);
            return Ok(());
        }
        let c = if self.acc.oauth.is_some() {
            self.imap_with_token(&user, &secret)?
        } else {
            let mut c = Client::connect(&self.acc.imap)?;
            c.login(&user, &secret)?;
            c
        };
        self.conn = Some(Conn::Imap(c));
        self.backoff = Duration::from_secs(5);
        Ok(())
    }

    /// An IMAP connection signed in with an OAuth token. Microsoft can take the token and then
    /// refuse the mailbox ("User is authenticated but not connected"): IMAP is switched off for
    /// it (Outlook.com's own setting, or an organisation's), or the browser signed in as another
    /// address. The address it signed in as is tried once; otherwise it's a refusal that says
    /// what to do, not a sign-in to ask for again (that only loops).
    fn imap_with_token(&self, user: &str, token: &str) -> Result<Client<Stream>> {
        let mut c = Client::connect(&self.acc.imap)?;
        let m = match c.login_xoauth2(user, token) {
            Ok(()) => return Ok(c),
            Err(NetError::Auth(m) | NetError::Bad(m) | NetError::No(m))
                if self.acc.oauth.as_deref() == Some("microsoft") && m.to_ascii_lowercase().contains("not connected") =>
            {
                m
            }
            Err(e) => return Err(e),
        };
        log::warn!("{}: {m}", self.acc.id);
        let other = self.acc.oauth_user.as_deref().filter(|o| !o.eq_ignore_ascii_case(user));
        if let Some(other) = other {
            let mut c = Client::connect(&self.acc.imap)?;
            if c.login_xoauth2(other, token).is_ok() {
                return Ok(c);
            }
        }
        Err(NetError::Refused(microsoft_imap_closed(&self.acc.email, other)))
    }

    /// A connection that couldn't be made is never tried again at once (that looped, showing
    /// "Connecting…" for good): a refusal (Graph's "no mailbox", a setting the server won't take)
    /// waits until asked, anything else for the backoff.
    fn connect_failed(&mut self, e: NetError) {
        match e {
            e @ NetError::Auth(_) => self.failed(e),
            e if e.is_fatal() => self.failed(e),
            e => {
                log::warn!("{}: {e}", self.acc.id);
                self.conn = None;
                self.auth_failed = true;
                self.status(SyncStatus::Error { error: e.to_string() });
            }
        }
    }

    fn connect_and_sync(&mut self) {
        if let Err(e) = self.connect() {
            self.connect_failed(e);
            return;
        }
        self.replay();
        self.flush_outbox();
        // Drafts kept while offline go to the server first.
        let kept: Vec<PendingDraft> = self.store.load_account_doc(&self.acc.id, PENDING_DRAFTS).unwrap_or_default();
        if !kept.is_empty() {
            let _ = self.store.save_account_doc(&self.acc.id, PENDING_DRAFTS, &Vec::<PendingDraft>::new());
            for d in kept.into_iter().rev() {
                self.queue.push_front(Job::SaveDraft { account: self.acc.id.clone(), raw: d.raw, folder: d.folder, replaces: d.replaces });
            }
        }
        self.last_full = None;
    }

    fn client(&mut self) -> Result<&mut Client<Stream>> {
        match self.conn.as_mut() {
            Some(Conn::Imap(c)) => Ok(c),
            _ => Err(NetError::Closed("not connected".into())),
        }
    }

    /// Save the JMAP / Graph numbering after it changed.
    fn save_jstate(&self) {
        if self.acc.is_api()
            && let Err(e) = self.jstate.save(&self.store, &self.acc.id)
        {
            log::warn!("{}: {e}", self.acc.id);
        }
    }

    /// The folder list, then every folder (Inbox first), staying responsive to opened messages.
    /// Refresh the folder list from the server; `false` when that failed.
    fn relist(&mut self) -> bool {
        let id = self.id();
        let listed = match self.conn.as_mut() {
            Some(Conn::Imap(c)) => c.list(&id),
            Some(Conn::Jmap(c)) => jmap::list_folders(c, &mut self.jstate, &id),
            Some(Conn::Graph(c)) => crate::graph::list_folders(c, &mut self.jstate, &id, &self.acc.shared_mailboxes),
            Some(Conn::Ews(c)) => crate::ews::list_folders(c, &mut self.jstate, &id),
            Some(Conn::Pop) => crate::pop::folders(&self.store, &id),
            None => Err(NetError::Closed("not connected".into())),
        };
        let jmap = matches!(self.conn, Some(Conn::Jmap(_) | Conn::Graph(_) | Conn::Ews(_)));
        if jmap {
            self.save_jstate();
        }
        match listed {
            Ok(mut list) => {
                // JMAP lists the counts; IMAP's come from each folder's sync.
                for f in list.iter_mut().filter(|_| !jmap) {
                    if let Some(old) = self.folders.iter().find(|o| o.path == f.path) {
                        f.unread = old.unread;
                        f.total = old.total;
                    }
                }
                sort_folders(&mut list);
                // Folders gone from the server lose their cache.
                for old in &self.folders {
                    if !list.iter().any(|f| f.path == old.path) {
                        let _ = self.store.clear_folder(&id, &old.path);
                    }
                }
                self.folders = list;
                self.publish_folders();
                true
            }
            Err(e) => {
                self.failed(e);
                false
            }
        }
    }

    fn sync_all(&mut self, rx: &Receiver<Job>) {
        let began = Instant::now();
        if !self.relist() {
            return;
        }
        let mut paths = sync_order(&self.folders);
        if self.opts.parallel > 0 && paths.len() > 2 {
            paths = self.sync_parallel(paths, rx);
            if self.closed {
                return;
            }
        }
        for path in paths {
            self.drain(rx);
            // What someone is waiting on jumps the queue: an opened message, an edit just made.
            let (urgent, rest): (VecDeque<Job>, VecDeque<Job>) = std::mem::take(&mut self.queue).into_iter().partition(between_folders);
            self.queue = rest;
            for j in urgent {
                self.handle(j);
            }
            if self.closed || self.conn.is_none() {
                return;
            }
            self.sync_one(&path);
        }
        // "Online" is said once the mail is current: the calendars and contacts can take far
        // longer, and shouldn't hold the status bar on the last folder's name.
        if self.conn.is_some() {
            self.report_quota();
            self.prune_jmap_numbers();
            self.last_full = Some(Instant::now());
            log::info!("{}: full sync done in {:.0} s", self.acc.id, began.elapsed().as_secs_f32());
            // Anything left from a failed attempt gets another try every full sync.
            self.flush_outbox();
            log::info!("{}: synced {} folders in {:.1} s", self.acc.id, self.folders.len(), began.elapsed().as_secs_f32());
            self.status(SyncStatus::Online { at: now() });
        }
        if self.conn.is_some() {
            let began = Instant::now();
            self.sync_dav();
            log::info!("{}: PIM pass took {:.1} s", self.acc.id, began.elapsed().as_secs_f32());
        }
    }

    /// Forget the numbers of JMAP messages no longer cached (nor waiting in a change, nor found
    /// by a search).
    fn prune_jmap_numbers(&mut self) {
        if !matches!(self.conn, Some(Conn::Jmap(_) | Conn::Graph(_) | Conn::Ews(_))) {
            return;
        }
        let ops = self.store.load_ops(&self.acc.id).unwrap_or_default();
        let mut keep: std::collections::HashSet<u32> = ops.iter().flat_map(|o| o.keys().iter().map(|k| k.uid).collect::<Vec<_>>()).collect();
        for f in &self.folders {
            if let Ok(st) = self.store.load_folder(&self.acc.id, &f.path) {
                keep.extend(st.messages.iter().map(|m| m.key.uid));
            }
        }
        keep.extend(self.search_numbers.iter().copied());
        self.jstate.prune(&keep);
        self.save_jstate();
    }

    fn publish_folders(&self) {
        let _ = self.store.save_folders(&self.acc.id, &self.folders);
        self.sink.emit(Event::Folders { account: self.id(), folders: self.folders.clone() });
    }

    fn sync_one(&mut self, path: &str) {
        self.status(SyncStatus::Syncing { folder: path.to_string() });
        // A folder asked to keep more keeps them (the API servers keep only the newest window).
        let window = self.windows.get(path).copied().unwrap_or(0).max(self.opts.window);
        let (store, opts, id) = (self.store.clone(), SyncOptions { window, ..self.opts }, self.id());
        let result = match self.conn.as_mut() {
            Some(Conn::Imap(c)) => sync_folder(c, &store, &id, path, &opts),
            Some(Conn::Jmap(c)) => {
                let r = jmap::sync_folder(c, &mut self.jstate, &store, &id, path, &opts);
                self.save_jstate();
                r
            }
            Some(Conn::Graph(c)) => {
                let r = crate::graph::sync_folder(c, &mut self.jstate, &store, &id, path, &opts);
                self.save_jstate();
                r
            }
            Some(Conn::Ews(c)) => {
                let r = crate::ews::sync_folder(c, &mut self.jstate, &store, &id, path, &opts);
                self.save_jstate();
                r
            }
            // POP: the Inbox is downloaded; every other folder is here only.
            Some(Conn::Pop) if self.folders.iter().any(|f| f.path == path && f.role == FolderRole::Inbox) => {
                let secret = self.secret();
                (|| {
                    let mut c = crate::pop::open(&self.acc, &secret?)?;
                    let r = crate::pop::fetch_new(&mut c, &store, &id, path, &crate::pop::Keep::of(&self.acc), now())?;
                    let _ = c.quit();
                    Ok(r)
                })()
            }
            Some(Conn::Pop) => Ok(crate::pop::local_counts(&store, &id, path)),
            None => Err(NetError::Closed("not connected".into())),
        };
        self.apply_report(path, result);
    }

    /// What a folder's sync changed: its counts, its list if any, and the account's state.
    fn apply_report(&mut self, path: &str, result: std::result::Result<SyncReport, NetError>) {
        match result {
            Ok(mut r) => {
                // The listing predates an edit still waiting: re-apply it to what was just saved,
                // or the folder shows the server's old state until the edit goes out.
                if self.reapply_waiting(path) {
                    r.changed += 1;
                }
                let first = self.folders.iter().find(|f| f.path == path).is_some_and(|f| f.total == 0 && r.total > 0);
                if let Some(f) = self.folders.iter_mut().find(|f| f.path == path) {
                    let counts_changed = f.unread != r.unread || f.total != r.total;
                    f.unread = r.unread;
                    f.total = r.total;
                    if counts_changed {
                        self.publish_folders();
                    }
                }
                if r.touched() || first {
                    self.sink.emit(Event::FolderChanged { account: self.acc.id.clone(), folder: path.to_string() });
                }
                if self.last_full.is_some() {
                    self.status(SyncStatus::Online { at: now() });
                }
            }
            Err(e) => self.failed(e),
        }
    }

    /// Read folders on connections of their own (up to `opts.parallel`), each taking folders
    /// from a shared list, while this connection stays free for what someone is waiting on.
    /// Answers the folders no connection read (none unless one was refused): the caller reads
    /// them here.
    fn sync_parallel(&mut self, paths: Vec<String>, rx: &Receiver<Job>) -> Vec<String> {
        let n = self.opts.parallel.min(3).min(paths.len().saturating_sub(1));
        if n == 0 || matches!(self.conn, Some(Conn::Pop) | None) {
            return paths;
        }
        let Ok(secret) = self.secret() else {
            return paths;
        };
        let queue = Arc::new(Mutex::new(paths.into_iter().collect::<VecDeque<String>>()));
        let (tx, done) = mpsc::channel::<SyncDone>();
        let mut running = 0usize;
        for _ in 0..n {
            // The numbers it may give, kept apart from this connection's (see `State::reserve`).
            let state = self.jstate.reserve(1_000_000);
            let (acc, store) = (self.acc.clone(), self.store.clone());
            let (secret, opts, q, tx) = (secret.clone(), self.opts, queue.clone(), tx.clone());
            if std::thread::Builder::new()
                .name(format!("{} sync", acc.id))
                .spawn(move || sync_folders(acc, store, secret, opts, state, q, tx))
                .is_err()
            {
                break;
            }
            running += 1;
        }
        drop(tx);
        // This connection answers what someone waits on meanwhile, then takes a folder of its
        // own if the others are still at it.
        let mut refused = false;
        while running > 0 {
            self.drain(rx);
            let urgent: Vec<Job> = self.queue.iter().filter(|j| jumps_queue(j)).cloned().collect();
            self.queue.retain(|j| !jumps_queue(j));
            for j in urgent {
                self.handle(j);
            }
            if self.closed {
                break;
            }
            match done.recv_timeout(std::time::Duration::from_millis(25)) {
                Ok(SyncDone::Folder(path, result)) => {
                    self.status(SyncStatus::Syncing { folder: path.clone() });
                    // A reader that timed out or lost its connection says the server doesn't want
                    // several of us right now (Exchange throttles a first full sync): the rest
                    // goes on this connection, and a fatal error stops the pass (`failed`).
                    if result.as_ref().is_err_and(NetError::is_fatal) {
                        self.opts.parallel = 0;
                    }
                    self.apply_report(&path, result);
                }
                Ok(SyncDone::State(state)) => {
                    self.jstate.absorb(state);
                    running -= 1;
                }
                Ok(SyncDone::Refused) => {
                    // A server that takes only one connection (or says it's busy): the rest waits
                    // for this one, and this account syncs on one connection from now on.
                    refused = true;
                    self.opts.parallel = 0;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        self.jstate.compact();
        self.save_jstate();
        // Whatever no connection took (a refusal, or a thread that couldn't start).
        let _ = refused;
        queue.lock().map(|q| q.iter().cloned().collect()).unwrap_or_default()
    }

    fn handle(&mut self, job: Job) {
        match job {
            Job::SyncAll { .. } => {
                if self.conn.is_none() {
                    self.auth_failed = false;
                    self.next_attempt = Instant::now();
                    self.connect_and_sync();
                }
                self.last_full = None;
            }
            Job::SyncFolder { folder, .. } => {
                if self.conn.is_none() {
                    self.next_attempt = Instant::now();
                    self.connect_and_sync();
                }
                if self.conn.is_some() {
                    self.sync_one(&folder);
                }
            }
            Job::MoreMail { folder, more, .. } => {
                let kept = self.store.load_folder(&self.acc.id, &folder).map(|f| f.messages.len()).unwrap_or(0);
                let now = self.windows.get(&folder).copied().unwrap_or(0).max(self.opts.window).max(kept);
                self.windows.insert(folder.clone(), now.saturating_add(more as usize).min(1_000_000));
                if let Err(e) = self.store.save_account_doc(&self.acc.id, WINDOWS, &self.windows) {
                    log::warn!("{}: {WINDOWS}: {e}", self.acc.id);
                }
                if self.conn.is_none() {
                    self.next_attempt = Instant::now();
                    self.connect_and_sync();
                }
                if self.conn.is_some() {
                    self.sync_one(&folder);
                }
            }
            Job::FetchBody { key } => self.fetch(key),
            Job::Search { query, folder, .. } => self.search(query, folder),
            // Already in the cache and ops.json (the pool wrote them): push the queue.
            Job::Apply { op, .. } if self.acc.pop.is_none() => {
                if op.is_folder_op() {
                    self.folders = self.store.load_folders(&self.acc.id).unwrap_or_default();
                }
                if self.conn.is_some() {
                    self.replay();
                }
            }
            Job::Vote { key, choice } => self.vote(key, choice),
            Job::Apply { op, .. } => {
                // POP: a move is carried out here (there's no server copy to come back); what's
                // deleted for good is noted, for removing the server's copy when the account says.
                if self.acc.pop.is_some() {
                    let gone: Vec<quatrelle_model::MessageKey> = match &op {
                        Op::Expunge { keys } => keys.clone(),
                        Op::EmptyFolder { path } => {
                            self.store.load_folder(&self.acc.id, path).map(|f| f.messages.into_iter().map(|m| m.key).collect()).unwrap_or_default()
                        }
                        _ => Vec::new(),
                    };
                    if let Err(e) = crate::pop::deleted_for_good(&self.store, &self.acc.id, &gone) {
                        self.notice(e.to_string());
                    }
                }
                let applied = match &op {
                    Op::Move { keys, to } if self.acc.pop.is_some() => crate::pop::move_locally(&self.store, &self.acc.id, keys, to),
                    _ => self.store.apply_op(&self.acc.id, &op).map_err(NetError::from),
                };
                match applied {
                    Ok(changed) => {
                        if op.is_folder_op() {
                            self.folders = self.store.load_folders(&self.acc.id).unwrap_or_default();
                            self.publish_folders();
                        }
                        for folder in changed {
                            self.sink.emit(Event::FolderChanged { account: self.id(), folder });
                        }
                    }
                    Err(e) => self.notice(e.to_string()),
                }
                if let Err(e) = self.store.add_op(&self.acc.id, op) {
                    self.notice(e.to_string());
                }
                if self.conn.is_some() {
                    self.replay();
                }
            }
            Job::Send { id, raw, from, recipients, sent_folder, .. } => self.send(id, &raw, &from, &recipients, sent_folder),
            Job::SaveDraft { raw, folder, replaces, .. } => {
                let (raw_kept, replaces_kept) = (raw.clone(), replaces.clone());
                let r = (|| -> Result<()> {
                    if self.conn.is_none() {
                        self.connect()?;
                    }
                    if let Some(Conn::Pop) = self.conn.as_ref() {
                        let flags = quatrelle_model::Flags { draft: true, seen: true, ..Default::default() };
                        crate::pop::file_locally(&self.store, &self.acc.id, &folder, &raw, flags, now())?;
                        if let Some(old) = replaces {
                            self.store.apply_op(&self.acc.id, &Op::Expunge { keys: vec![old] })?;
                        }
                        return Ok(());
                    }
                    if let Some(Conn::Jmap(c)) = self.conn.as_mut() {
                        jmap::append(c, &self.jstate, &folder, &raw, &["$draft", "$seen"])?;
                        if let Some(old) = replaces {
                            jmap::apply_remote(c, &self.jstate, &Op::Expunge { keys: vec![old] })?;
                        }
                        return Ok(());
                    }
                    if let Some(Conn::Graph(c)) = self.conn.as_mut() {
                        crate::graph::append(c, &self.jstate, &folder, &raw)?;
                        if let Some(old) = replaces {
                            crate::graph::apply_remote(c, &self.jstate, &Op::Expunge { keys: vec![old] })?;
                        }
                        return Ok(());
                    }
                    if let Some(Conn::Ews(c)) = self.conn.as_mut() {
                        crate::ews::append(c, &self.jstate, &folder, &raw)?;
                        if let Some(old) = replaces {
                            crate::ews::apply_remote(c, &self.jstate, &Op::Expunge { keys: vec![old] })?;
                        }
                        return Ok(());
                    }
                    let trash = self.folders.iter().find(|f| f.role == FolderRole::Trash).map(|f| f.path.clone());
                    let c = self.client()?;
                    c.append(&folder, "\\Draft \\Seen", &raw)?;
                    if let Some(old) = replaces {
                        expunge_keys(c, &[old], trash.as_deref())?;
                    }
                    Ok(())
                })();
                match r {
                    Ok(()) => self.sync_one(&folder),
                    Err(e) => {
                        // Not lost: kept on this computer and saved when the account connects.
                        match keep_draft(&self.store, &self.acc.id, &folder, replaces_kept, raw_kept) {
                            Ok(()) => self.notice(
                                "the draft couldn't be saved to the server; it's kept on this computer and saved there when the account connects",
                            ),
                            Err(k) => self.notice(format!("the draft couldn't be saved: {k}")),
                        }
                        self.failed(e);
                    }
                }
            }
            Job::FlushOutbox { .. } => self.flush_outbox(),
            Job::SyncDav { .. } => self.sync_dav(),
            Job::GetVacation { .. } => {
                let r = self.vacation(None);
                self.report_vacation(r, None);
            }
            Job::SetVacation { vacation, .. } => {
                let r = self.vacation(Some(&vacation));
                match r {
                    Ok(v) => self.report_vacation(Ok(v), None),
                    // Show what the server still has, with why the change failed.
                    Err(e) => {
                        let back = self.vacation(None);
                        self.report_vacation(back, Some(e.to_string()));
                    }
                }
            }
            Job::SetServerRules { rules, delete, categories, .. } => self.set_server_rules(rules, delete, categories),
            Job::GetServerRules { ours, .. } => match self.read_server_rules(ours) {
                Ok((ids, others, cleared)) => {
                    log::info!("{}: the server keeps {} rules ({} of ours, {} others)", self.id(), ids.len() + others.len(), ids.len(), others.len());
                    self.sink.emit(Event::ServerRules { account: self.id(), ids, others, cleared, error: None });
                }
                Err(e) => {
                    log::info!("{}: the server's rules couldn't be read: {e}", self.id());
                    self.sink.emit(Event::ServerRules {
                        account: self.id(),
                        ids: Vec::new(),
                        others: Vec::new(),
                        cleared: Vec::new(),
                        error: Some(e.to_string()),
                    })
                }
            },
            Job::SetServerRuleEnabled { id, enabled, ours, .. } => self.change_server_rule(&id, Some(enabled), false, ours),
            Job::SetServerRule { id, rule, ours, .. } => self.edit_server_rule(&id, &rule, ours),
            Job::DeleteServerRule { id, ours, .. } => self.change_server_rule(&id, None, true, ours),
            Job::FetchPhoto { email, contact, .. } => self.fetch_photo(&email, contact.as_deref()),
            Job::FetchUrl { .. } | Job::Discover { .. } | Job::SignIn { .. } | Job::ImportPst { .. } | Job::ExportPst { .. } | Job::CheckUpdates => {}
            Job::Start { .. } | Job::SmtpPassword { .. } | Job::Remove { .. } | Job::WorkOffline { .. } | Job::SetSync { .. } => {}
        }
    }

    /// Put the account's server-marked rules on its server (Exchange's inbox rules), so mail is
    /// filed while Quatrelle is closed, and remove the ones turned off or deleted here.
    fn set_server_rules(&mut self, rules: Vec<quatrelle_model::organize::Rule>, delete: Vec<String>, categories: Vec<(String, String)>) {
        use quatrelle_model::organize::RuleAction;
        let r = (|| -> Result<(Vec<(String, String)>, Vec<quatrelle_model::organize::ForeignRule>)> {
            if rules.is_empty() && delete.is_empty() {
                return Ok((Vec::new(), Vec::new()));
            }
            if self.conn.is_none() {
                self.connect()?;
            }
            // An IMAP account keeps them in its Sieve script, through ManageSieve: the script
            // holds the whole set, so it is written again (nothing to remove by id).
            if matches!(self.conn, Some(Conn::Imap(_))) {
                let (wanted, ids) = self.sieve_rules(&rules);
                let secret = self.secret()?;
                let user = if self.acc.username.is_empty() { self.acc.email.clone() } else { self.acc.username.clone() };
                let mut s = crate::sieve::Client::connect(&self.acc.imap, self.sieve_port).map_err(|e| match e {
                    NetError::Io(_) => NetError::No("this server doesn't take inbox rules from Quatrelle (no ManageSieve)".into()),
                    e => e,
                })?;
                if self.acc.oauth.is_some() {
                    s.login_xoauth2(&user, &secret)?;
                } else {
                    s.login(&user, &secret)?;
                }
                let written = s.set_rules(wanted, categories);
                let listed = s.list();
                s.logout();
                written?;
                let (names, _) = listed?;
                let others = names
                    .iter()
                    .filter(|n| n.as_str() != crate::sieve::SCRIPT)
                    .map(|n| quatrelle_model::organize::ForeignRule {
                        account: self.acc.id.clone(),
                        id: format!("sieve:{n}"),
                        name: n.clone(),
                        enabled: true,
                        runnable: false,
                        rule: None,
                        summary: "a Sieve script".into(),
                    })
                    .collect();
                return Ok((ids, others));
            }
            // The folder each rule files mail into, by the id the server knows it by.
            let mut wanted: Vec<(quatrelle_model::organize::Rule, Option<String>)> = Vec::new();
            for rule in rules.iter().filter(|r| r.server && r.server_ready()) {
                let folder: Option<&str> = rule.actions.iter().find_map(|a| match a {
                    RuleAction::MoveTo(path) => Some(path.as_str()),
                    RuleAction::MoveToRole(role) => self.folders.iter().find(|f| f.role == *role && f.mailbox.is_none()).map(|f| f.path.as_str()),
                    _ => None,
                });
                let id = match folder {
                    Some(path) => match self.jstate.mailbox(path) {
                        Ok(id) => Some(id.to_string()),
                        Err(_) => {
                            // Its folder isn't on the server (any more): keeping the rule here is
                            // better than filing mail nowhere.
                            self.notice(format!("the rule \"{}\" waits: its folder isn't on the server", rule.name));
                            continue;
                        }
                    },
                    None => None,
                };
                wanted.push((rule.clone(), id));
            }
            let ids = match self.conn.as_mut() {
                Some(Conn::Ews(c)) => c.set_inbox_rules(&wanted, &delete),
                Some(Conn::Graph(c)) => c.set_message_rules(&wanted, &delete),
                _ => Err(NetError::No("this server's rules can't be changed from Quatrelle".into())),
            }?;
            // What else the server runs: made in Outlook, or on its own web page.
            let all = match self.conn.as_ref() {
                Some(Conn::Ews(c)) => c.inbox_rules()?,
                Some(Conn::Graph(c)) => c.message_rules()?,
                _ => Vec::new(),
            };
            let mine: Vec<String> = ids.iter().map(|(_, id)| id.clone()).collect();
            let (_, mut others) = split_rules(all, &mine);
            for r in &mut others {
                r.account.clone_from(&self.acc.id);
            }
            self.resolve_rule_folders(&mut others);
            Ok((ids, others))
        })();
        match r {
            Ok((ids, others)) => self.sink.emit(Event::ServerRules { account: self.acc.id.clone(), ids, others, cleared: delete, error: None }),
            Err(e) => self.sink.emit(Event::ServerRules {
                account: self.acc.id.clone(),
                ids: Vec::new(),
                others: Vec::new(),
                cleared: Vec::new(),
                error: Some(e.to_string()),
            }),
        }
    }

    /// What the account's server keeps: the ids Quatrelle's own rules have there (`ours` is what
    /// it holds), the names of the ones it didn't make, and its own the server no longer has.
    fn read_server_rules(&mut self, ours: Vec<String>) -> Result<(Vec<(String, String)>, Vec<quatrelle_model::organize::ForeignRule>, Vec<String>)> {
        if self.conn.is_none() {
            self.connect()?;
        }
        if matches!(self.conn, Some(Conn::Pop)) {
            return Err(NetError::No("POP accounts keep no rules on their server".into()));
        }
        if matches!(self.conn, Some(Conn::Imap(_))) {
            let secret = self.secret()?;
            let user = if self.acc.username.is_empty() { self.acc.email.clone() } else { self.acc.username.clone() };
            let mut s = crate::sieve::Client::connect(&self.acc.imap, self.sieve_port).map_err(|e| match e {
                NetError::Io(_) => NetError::No("this server keeps no rules for Quatrelle (no ManageSieve)".into()),
                e => e,
            })?;
            if self.acc.oauth.is_some() {
                s.login_xoauth2(&user, &secret)?;
            } else {
                s.login(&user, &secret)?;
            }
            let listed = s.list();
            s.logout();
            let (names, _) = listed?;
            // Our script holds our rules; without it, they're gone from the server.
            let own = names.iter().any(|n| n == crate::sieve::SCRIPT);
            let ids =
                if own { ours.iter().map(|id| (id.strip_prefix("sieve:").unwrap_or(id).to_string(), id.clone())).collect() } else { Vec::new() };
            let cleared = if own { Vec::new() } else { ours };
            let others = names
                .iter()
                .filter(|n| n.as_str() != crate::sieve::SCRIPT)
                .map(|n| quatrelle_model::organize::ForeignRule {
                    account: self.acc.id.clone(),
                    id: format!("sieve:{n}"),
                    name: n.clone(),
                    enabled: true,
                    runnable: false,
                    rule: None,
                    summary: "a Sieve script".into(),
                })
                .collect();
            return Ok((ids, others, cleared));
        }
        let all = match self.conn.as_ref() {
            Some(Conn::Ews(c)) => c.inbox_rules()?,
            Some(Conn::Graph(c)) => c.message_rules()?,
            _ => return Err(NetError::No("this server keeps no inbox rules".into())),
        };
        let on_server: Vec<&str> = all.iter().map(|r| r.id.as_str()).collect();
        let cleared = ours.iter().filter(|id| !on_server.contains(&id.as_str())).cloned().collect();
        let (ids, mut others) = split_rules(all, &ours);
        for r in &mut others {
            r.account.clone_from(&self.acc.id);
        }
        self.resolve_rule_folders(&mut others);
        Ok((ids, others, cleared))
    }

    /// The folder path behind the id a server's rule files mail into (Exchange names folders by
    /// id, Quatrelle by path).
    fn path_of_folder_id(&self, id: &str) -> Option<String> {
        self.folders
            .iter()
            .filter(|f| f.mailbox.is_none())
            .find(|f| self.jstate.mailboxes.get(&f.path).is_some_and(|x| x == id))
            .map(|f| f.path.clone())
    }

    /// A server rule's `MoveTo` names a folder by the id the server knows; in Quatrelle's terms
    /// it's the folder's path, so the rule can be shown and changed here.
    fn resolve_rule_folders(&self, rules: &mut [quatrelle_model::organize::ForeignRule]) {
        for f in rules.iter_mut() {
            let Some(rule) = &mut f.rule else { continue };
            for a in &mut rule.actions {
                if let quatrelle_model::organize::RuleAction::MoveTo(path) = a
                    && let Some(id) = path.strip_prefix("id:")
                    && let Some(found) = self.path_of_folder_id(id)
                {
                    *path = found;
                }
            }
        }
    }

    /// Change one of the server's own rules (one Quatrelle didn't make): only that rule is sent,
    /// with the place it already has there, so the other rules and their order stay as they are.
    fn edit_server_rule(&mut self, id: &str, rule: &quatrelle_model::organize::Rule, ours: Vec<String>) {
        use quatrelle_model::organize::RuleAction;
        let r = (|| -> Result<()> {
            if self.conn.is_none() {
                self.connect()?;
            }
            let mut rule = rule.clone();
            rule.remote_id = Some(id.to_string());
            rule.server = true;
            let folder: Option<String> = match rule.actions.iter().find_map(|a| match a {
                RuleAction::MoveTo(path) => Some(path.clone()),
                RuleAction::MoveToRole(role) => self.folders.iter().find(|f| f.role == *role && f.mailbox.is_none()).map(|f| f.path.clone()),
                _ => None,
            }) {
                Some(path) => Some(match path.strip_prefix("id:") {
                    Some(id) => id.to_string(),
                    None => self.jstate.mailbox(&path)?.to_string(),
                }),
                None => None,
            };
            let wanted = [(rule, folder)];
            match self.conn.as_mut() {
                Some(Conn::Ews(c)) => c.set_inbox_rules(&wanted, &[]).map(|_| ()),
                Some(Conn::Graph(c)) => c.set_message_rules(&wanted, &[]).map(|_| ()),
                _ => Err(NetError::No("this server's rules can't be changed from Quatrelle".into())),
            }
        })();
        match r {
            Ok(()) => match self.read_server_rules(ours) {
                Ok((ids, others, cleared)) => self.sink.emit(Event::ServerRules { account: self.id(), ids, others, cleared, error: None }),
                Err(e) => self.sink.emit(Event::ServerRules {
                    account: self.id(),
                    ids: Vec::new(),
                    others: Vec::new(),
                    cleared: Vec::new(),
                    error: Some(e.to_string()),
                }),
            },
            Err(e) => self.sink.emit(Event::ServerRules {
                account: self.id(),
                ids: Vec::new(),
                others: Vec::new(),
                cleared: Vec::new(),
                error: Some(e.to_string()),
            }),
        }
    }

    /// Turn one of the server's own rules on or off, or remove it; then read them all back, so
    /// the dialog shows what the server now keeps.
    fn change_server_rule(&mut self, id: &str, enabled: Option<bool>, delete: bool, ours: Vec<String>) {
        let r = (|| -> Result<()> {
            if self.conn.is_none() {
                self.connect()?;
            }
            match self.conn.as_ref() {
                Some(Conn::Ews(c)) => {
                    if delete {
                        c.delete_rule(id)
                    } else {
                        c.set_rule_enabled(id, enabled.unwrap_or(true))
                    }
                }
                Some(Conn::Graph(c)) => {
                    if delete {
                        c.delete_rule(id)
                    } else {
                        c.set_rule_enabled(id, enabled.unwrap_or(true))
                    }
                }
                _ => Err(NetError::No("this server's rules can't be changed from Quatrelle".into())),
            }
        })();
        match r {
            Ok(()) => match self.read_server_rules(ours) {
                Ok((ids, others, cleared)) => self.sink.emit(Event::ServerRules { account: self.id(), ids, others, cleared, error: None }),
                Err(e) => self.sink.emit(Event::ServerRules {
                    account: self.id(),
                    ids: Vec::new(),
                    others: Vec::new(),
                    cleared: Vec::new(),
                    error: Some(e.to_string()),
                }),
            },
            Err(e) => self.sink.emit(Event::ServerRules {
                account: self.id(),
                ids: Vec::new(),
                others: Vec::new(),
                cleared: Vec::new(),
                error: Some(e.to_string()),
            }),
        }
    }

    /// The picture kept for a person: the one their contact item carries (`contact`), or else
    /// the one the directory keeps for their address. None is an answer too (the person simply
    /// has no picture, or the server doesn't keep them).
    fn fetch_photo(&mut self, email: &str, contact: Option<&str>) {
        let r = (|| -> Result<(String, Vec<u8>)> {
            if self.conn.is_none() {
                self.connect()?;
            }
            if let Some(id) = contact {
                let own = match self.conn.as_mut() {
                    Some(Conn::Ews(c)) => c.contact_photo(id),
                    Some(Conn::Graph(c)) => c.contact_photo(id),
                    _ => Err(NetError::No("this server keeps no contact pictures".into())),
                };
                // A contact without a picture of its own may still have one in the directory.
                match own {
                    Ok(p) => return Ok(p),
                    Err(NetError::No(_)) => {}
                    Err(e) => return Err(e),
                }
            }
            match self.conn.as_mut() {
                Some(Conn::Ews(c)) => c.user_photo(email),
                Some(Conn::Graph(c)) => c.user_photo(email),
                _ => Err(NetError::No("this server keeps no pictures".into())),
            }
        })();
        let (media, data, error) = match r {
            Ok((media, data)) => (media, data, None),
            Err(e) => (String::new(), Vec::new(), Some(e.to_string())),
        };
        // Said in the log either way: "why is there no picture?" is answered without a debugger.
        match &error {
            Some(e) => log::info!("picture of {email}: none ({e})"),
            None => log::info!("picture of {email}: {} bytes, {media}", data.len()),
        }
        self.sink.emit(Event::Photo { email: email.to_string(), media, data, error });
    }

    /// The rules an IMAP account's Sieve script gets, their destinations resolved to the
    /// server's folder names, and the id kept for each (the script holds the rules by name).
    fn sieve_rules(&self, rules: &[quatrelle_model::organize::Rule]) -> (Vec<quatrelle_model::organize::Rule>, Vec<(String, String)>) {
        use quatrelle_model::organize::RuleAction;
        let mut wanted = Vec::new();
        let mut ids = Vec::new();
        for rule in rules.iter().filter(|r| r.server && r.sieve_ready()) {
            if !rule.enabled {
                // Off: it isn't in the script. Its id still comes back, so it isn't sent again
                // until it changes.
                ids.push((rule.name.clone(), format!("sieve:{}", rule.name)));
                continue;
            }
            let mut kept = rule.clone();
            let mut actions = Vec::new();
            let mut missing = false;
            for a in &rule.actions {
                match a {
                    RuleAction::MoveTo(path) => {
                        if self.folders.iter().any(|f| &f.path == path) {
                            actions.push(a.clone());
                        } else {
                            missing = true;
                        }
                    }
                    RuleAction::MoveToRole(role) => match self.folders.iter().find(|f| f.role == *role && f.mailbox.is_none()) {
                        Some(f) => actions.push(RuleAction::MoveTo(f.path.clone())),
                        None => missing = true,
                    },
                    // Deleting is filing in Deleted Items, as Quatrelle does here.
                    RuleAction::Delete => match self.folders.iter().find(|f| f.role == quatrelle_model::FolderRole::Trash && f.mailbox.is_none()) {
                        Some(f) => actions.push(RuleAction::MoveTo(f.path.clone())),
                        None => missing = true,
                    },
                    other => actions.push(other.clone()),
                }
            }
            // Its folder isn't on the server (any more): keeping the rule here is better than
            // filing mail nowhere.
            if missing {
                self.notice(format!("the rule \"{}\" waits: its folder isn't on the server", rule.name));
                continue;
            }
            kept.actions = actions;
            ids.push((rule.name.clone(), format!("sieve:{}", rule.name)));
            wanted.push(kept);
        }
        (wanted, ids)
    }

    /// Read (and first change, when `set`) the automatic replies: JMAP's own, or a Sieve script
    /// through ManageSieve on the IMAP host.
    fn vacation(&mut self, set: Option<&quatrelle_model::job::Vacation>) -> Result<quatrelle_model::job::Vacation> {
        // Not connected: one try, and a failure shows as the account's status (it said
        // "Connecting…" for good when the vacation page was opened on an account that can't).
        // (The second connection asks ManageSieve straight away: it needs no IMAP session.)
        if self.conn.is_none() && (self.main.is_none() || self.acc.is_api()) {
            if self.auth_failed {
                return Err(NetError::Refused("it shows once the account is connected".into()));
            }
            if let Err(e) = self.connect() {
                if self.main.is_some() {
                    return Err(e);
                }
                self.connect_failed(e);
                return Err(NetError::Refused("it shows once the account is connected".into()));
            }
        }
        if let Some(Conn::Pop) = &self.conn {
            return Err(NetError::No("POP accounts have no automatic replies on the server".into()));
        }
        if let Some(Conn::Jmap(c)) = &self.conn {
            if let Some(v) = set {
                c.set_vacation(v)?;
            }
            return c.vacation();
        }
        if let Some(Conn::Graph(c)) = &self.conn {
            if let Some(v) = set {
                c.set_vacation(v)?;
            }
            return c.vacation();
        }
        if let Some(Conn::Ews(c)) = &self.conn {
            if let Some(v) = set {
                c.set_vacation(v)?;
            }
            return c.vacation();
        }
        let secret = self.secret()?;
        let user = if self.acc.username.is_empty() { self.acc.email.clone() } else { self.acc.username.clone() };
        let mut s = crate::sieve::Client::connect(&self.acc.imap, self.sieve_port).map_err(|e| match e {
            NetError::Io(_) => NetError::No("this server doesn't offer automatic replies (no ManageSieve)".into()),
            e => e,
        })?;
        if self.acc.oauth.is_some() {
            s.login_xoauth2(&user, &secret)?;
        } else {
            s.login(&user, &secret)?;
        }
        if let Some(v) = set {
            s.set_vacation(v)?;
        }
        let v = s.vacation();
        s.logout();
        v
    }

    fn report_vacation(&self, r: Result<quatrelle_model::job::Vacation>, failed: Option<String>) {
        let (vacation, error) = match r {
            Ok(v) => (Some(v), failed),
            Err(e) => (None, Some(failed.unwrap_or_else(|| e.to_string()))),
        };
        self.sink.emit(Event::Vacation { account: self.id(), vacation, error });
    }

    /// Sync calendars and address books with the account's CalDAV / CardDAV server (password
    /// accounts: OAuth tokens are for mail only).
    fn sync_dav(&mut self) {
        // Exchange Online: its calendars and contacts through Graph.
        if let Some(Conn::Graph(c)) = &self.conn {
            let pim: quatrelle_model::pim::Pim = self.store.load_account_doc(&self.acc.id, quatrelle_model::pim::PIM_DOC).unwrap_or_default();
            match crate::graph_pim::sync(c, &pim, now()) {
                Ok(sync) => self.sink.emit(Event::DavSynced { account: self.id(), sync: Box::new(sync) }),
                Err(e) => log::info!("{}: calendars and contacts: {e}", self.acc.id),
            }
            return;
        }
        // On-premises Exchange: its calendar and contacts through EWS.
        if let Some(Conn::Ews(c)) = &self.conn {
            let pim: quatrelle_model::pim::Pim = self.store.load_account_doc(&self.acc.id, quatrelle_model::pim::PIM_DOC).unwrap_or_default();
            match crate::ews_pim::sync(c, &pim, now()) {
                Ok(sync) => self.sink.emit(Event::DavSynced { account: self.id(), sync: Box::new(sync) }),
                Err(e) => log::info!("{}: calendars and contacts: {e}", self.acc.id),
            }
            return;
        }
        // OAuth tokens are for mail only.
        if self.acc.oauth.is_some() || self.acc.ews.is_some() {
            return;
        }
        let pim: quatrelle_model::pim::Pim = self.store.load_account_doc(&self.acc.id, quatrelle_model::pim::PIM_DOC).unwrap_or_default();
        let user = if self.acc.username.is_empty() { self.acc.email.clone() } else { self.acc.username.clone() };
        let password = self.auth.lock().unwrap_or_else(std::sync::PoisonError::into_inner).secret.clone();
        let dav = crate::dav::Dav::new(&user, &password);
        let host = self.acc.pop.as_ref().map_or(self.acc.imap.host.as_str(), |p| p.host.as_str());
        match crate::dav::sync(&dav, &self.acc.email, host, &pim, now()) {
            Ok(sync) => self.sink.emit(Event::DavSynced { account: self.id(), sync: Box::new(sync) }),
            Err(e) => {
                log::info!("{}: calendars and contacts: {e}", self.acc.id);
                if matches!(e, NetError::Auth(_)) {
                    self.notice(format!("calendars and contacts: {e}"));
                }
            }
        }
    }

    /// Report the space used on the server (when it publishes a quota).
    fn report_quota(&mut self) {
        let q = match self.conn.as_mut() {
            Some(Conn::Imap(c)) => c.quota(),
            Some(Conn::Jmap(c)) => c.quota(),
            // Graph doesn't tell a mailbox's size to the mailbox's owner.
            Some(Conn::Graph(_) | Conn::Ews(_) | Conn::Pop) | None => return,
        };
        match q {
            Ok(Some((used, limit))) => self.sink.emit(Event::Quota { account: self.id(), used, limit }),
            Ok(None) => {}
            Err(e) if e.is_fatal() => self.failed(e),
            Err(e) => log::info!("{}: quota: {e}", self.acc.id),
        }
    }

    /// Search with the server's own index (Exchange's): the local one can only find a message
    /// whose text is here, and a search over years of mail needs the rest.
    fn search(&mut self, query: String, folder: Option<String>) {
        let account = self.acc.id.clone();
        if self.conn.is_none()
            && let Err(e) = self.connect()
        {
            self.sink.emit(Event::Found { account, query, items: Vec::new(), error: Some(e.to_string()) });
            return;
        }
        // Which folders to ask: the one on view, or every folder of the account, the biggest
        // first (Gmail's All Mail holds everything; the walk stops when its time is up).
        let paths: Vec<String> = match &folder {
            Some(f) => vec![f.clone()],
            None => {
                let mut fs: Vec<&quatrelle_model::Folder> = self.folders.iter().collect();
                fs.sort_by_key(|f| std::cmp::Reverse(f.total));
                fs.into_iter().map(|f| f.path.clone()).collect()
            }
        };
        // The numbering an id gets is kept, so a message found by a search can be read afterwards
        // (and after a restart). The state is taken out of `self` — it can't be borrowed twice for
        // the call — and goes back whatever the call answered: one left empty would key every
        // cached message to nothing.
        let mut state = std::mem::take(&mut self.jstate);
        let store = self.store.clone();
        let r = match self.conn.as_mut() {
            Some(Conn::Ews(c)) => crate::ews::search(c, &mut state, &account, &paths, &query, SEARCH_LIMIT),
            // IMAP's own SEARCH: the words may be in a message older than the window kept here,
            // or in a body never downloaded.
            Some(Conn::Imap(c)) => crate::sync::search_folders(
                c,
                &store,
                &account,
                &paths,
                &query,
                SEARCH_LIMIT,
                std::time::Duration::from_secs(crate::sync::SEARCH_SECONDS),
                crate::sync::SEARCH_FOLDERS,
            ),
            // The other transports answer from this computer alone.
            _ => Ok(Vec::new()),
        };
        let numbers: Vec<u32> = r.as_ref().map(|items| items.iter().map(|m| m.key.uid).collect()).unwrap_or_default();
        let _ = state.save(&self.store, &account);
        self.jstate = state;
        // What only a search found isn't in any folder's cache, so the numbers it may not be
        // kept by must survive the next cleanup: the message is still one to open.
        // Only the API transports number a message themselves; an IMAP key is the server's UID,
        // kept in its folder's cache already.
        if self.acc.is_api() {
            for n in numbers {
                if !self.search_numbers.contains(&n) {
                    self.search_numbers.push(n);
                }
            }
            if self.search_numbers.len() > 4_000 {
                self.search_numbers.drain(..2_000);
            }
        }
        match r {
            Ok(items) => self.sink.emit(Event::Found { account, query, items, error: None }),
            Err(e) => {
                self.sink.emit(Event::Found { account, query, items: Vec::new(), error: Some(e.to_string()) });
                if e.is_fatal() {
                    self.failed(e);
                }
            }
        }
    }

    fn fetch(&mut self, key: MessageKey) {
        if let Some(raw) = self.store.read_body(&key) {
            self.sink.emit(Event::Body { key, raw });
            return;
        }
        let store = self.store.clone();
        let numbered = self.jstate.len();
        let r = (|| -> Result<Vec<u8>> {
            if self.conn.is_none() {
                self.connect()?;
            }
            match self.conn.as_mut() {
                Some(Conn::Jmap(c)) => jmap::fetch_body(c, &self.jstate, &store, &key),
                Some(Conn::Graph(c)) => crate::graph::fetch_body(c, &self.jstate, &store, &key),
                Some(Conn::Ews(c)) => crate::ews::fetch_body(c, &mut self.jstate, &store, &key),
                // POP mail is downloaded whole: what isn't here isn't anywhere.
                Some(Conn::Pop) => store.read_body(&key).ok_or_else(|| NetError::No("this message isn't on this computer".into())),
                _ => fetch_body(self.client()?, &store, &key),
            }
        })();
        if self.jstate.len() != numbered {
            // A message found again by its Message-ID: the numbering is worth keeping.
            self.save_jstate();
        }
        match r {
            Ok(raw) => {
                self.sink.emit(Event::Body { key: key.clone(), raw });
                self.sink.emit(Event::FolderChanged { account: key.account.clone(), folder: key.folder.clone() });
            }
            Err(e) => {
                self.sink.emit(Event::BodyFailed { key, error: e.to_string() });
                if e.is_fatal() || matches!(e, NetError::Auth(_)) {
                    self.failed(e);
                }
            }
        }
    }

    /// Answer a message's voting buttons; the choice is kept with the message once it's gone.
    fn vote(&mut self, key: MessageKey, choice: String) {
        let r = (|| -> Result<()> {
            if self.conn.is_none() {
                self.connect()?;
            }
            let subject =
                self.store.load_folder(&key.account, &key.folder).ok().and_then(|f| f.find(key.uid).map(|m| m.subject.clone())).unwrap_or_default();
            match self.conn.as_ref() {
                Some(Conn::Graph(c)) => crate::graph::vote(c, &self.jstate, &key, &choice, &subject),
                Some(Conn::Ews(c)) => crate::ews::vote(c, &self.jstate, &key, &choice, &subject),
                _ => Err(NetError::No("voting buttons can only be answered on Exchange and Microsoft 365 accounts".into())),
            }
        })();
        match r {
            Ok(()) => {
                if let Ok(mut fs) = self.store.load_folder(&key.account, &key.folder)
                    && let Some(m) = fs.find_mut(key.uid)
                {
                    m.voted = choice.clone();
                    if let Err(e) = self.store.save_folder(&key.account, &key.folder, &fs) {
                        self.notice(e.to_string());
                    }
                }
                self.sink.emit(Event::Voted { key: key.clone(), choice, error: None });
                self.sink.emit(Event::FolderChanged { account: key.account, folder: key.folder });
            }
            Err(e) => {
                self.sink.emit(Event::Voted { key, choice, error: Some(e.to_string()) });
                if e.is_fatal() || matches!(e, NetError::Auth(_)) {
                    self.failed(e);
                }
            }
        }
    }

    /// Hand a message to the server: SMTP, or for JMAP accounts a submission (which also files
    /// the copy in Sent).
    fn deliver(&mut self, raw: &[u8], from: &str, recipients: &[String], sent_folder: Option<&str>) -> Result<()> {
        if self.acc.graph.is_some() {
            if self.conn.is_none() {
                self.connect()?;
            }
            let Some(Conn::Graph(c)) = &self.conn else { return Err(NetError::Closed("not connected".into())) };
            return crate::graph::send(c, raw, recipients);
        }
        if self.acc.ews.is_some() {
            if self.conn.is_none() {
                self.connect()?;
            }
            let Some(Conn::Ews(c)) = &self.conn else { return Err(NetError::Closed("not connected".into())) };
            return crate::ews::send(c, raw, recipients);
        }
        if self.acc.jmap.is_some() {
            if self.conn.is_none() {
                self.connect()?;
            }
            let sent = sent_folder.map(str::to_string).or_else(|| self.folders.iter().find(|f| f.role == FolderRole::Sent).map(|f| f.path.clone()));
            let Some(Conn::Jmap(c)) = &self.conn else { return Err(NetError::Closed("not connected".into())) };
            return jmap::send(c, &self.jstate, sent.as_deref(), raw, from, recipients);
        }
        let (user, secret) = match self.acc.smtp_username.as_deref().map(str::trim).filter(|u| !u.is_empty() && self.acc.oauth.is_none()) {
            // The outgoing server's own sign-in (issue #3: its user name differs from the
            // incoming server's), with its own password.
            Some(u) => {
                let pw = self.creds.as_ref().and_then(|c| c.get_smtp(&self.acc)).ok_or_else(|| {
                    NetError::Auth(format!(
                        "the outgoing server's password for {u} isn't on this computer: enter it in the account's Server settings"
                    ))
                })?;
                (u.to_string(), pw)
            }
            None => (if self.acc.username.is_empty() { self.acc.email.clone() } else { self.acc.username.clone() }, self.secret()?),
        };
        smtp::send_with(&self.acc.smtp, &user, &secret, self.acc.oauth.is_some(), from, recipients, raw).map_err(|e| match e {
            // Say whether the sign-in covered sending: Microsoft lists what it granted.
            NetError::Auth(m) if self.acc.oauth.as_deref() == Some("microsoft") => {
                let granted = self.granted();
                let why = if granted.is_empty() {
                    String::new()
                } else if granted.to_ascii_lowercase().contains("smtp.send") {
                    " (the sign-in includes sending: Microsoft's server refused it)".to_string()
                } else {
                    format!(" (Microsoft's sign-in didn't include sending; it granted: {granted})")
                };
                NetError::Auth(format!("{m}{why}"))
            }
            e => e,
        })
    }

    fn send(&mut self, id: u64, raw: &[u8], from: &str, recipients: &[String], sent_folder: Option<String>) {
        let sent = self.deliver(raw, from, recipients, sent_folder.as_deref());
        if let Err(e) = sent {
            if e.is_retryable() {
                // Offline or asked to wait: keep it in the Outbox and send it on reconnect.
                let item = OutboxItem {
                    id,
                    from: from.to_string(),
                    recipients: recipients.to_vec(),
                    sent_folder,
                    queued: now(),
                    attempts: 1,
                    error: Some(e.to_string()),
                    not_before: 0,
                };
                match self.store.outbox_add(&self.acc.id, &item, raw) {
                    Ok(()) => self.sink.emit(Event::Queued { account: self.id(), id, error: e.to_string() }),
                    Err(se) => {
                        self.sink.emit(Event::SendFailed { account: self.id(), id, error: format!("{e} (and the Outbox couldn't keep it: {se})") })
                    }
                }
            } else {
                self.sink.emit(Event::SendFailed { account: self.id(), id, error: e.to_string() });
            }
            return;
        }
        self.after_sent(id, raw, sent_folder);
    }

    /// Send what waits in the Outbox, oldest first. Stops at the first failure that may clear
    /// up later (still offline); a message refused for good stays, marked with the error.
    fn flush_outbox(&mut self) {
        for mut item in self.store.outbox(&self.acc.id) {
            // Held back (Delay Delivery) until its time; then dated when it goes.
            if item.not_before > now() {
                continue;
            }
            let Some(raw) = self.store.outbox_raw(&self.acc.id, item.id) else { continue };
            let raw = if item.not_before > 0 { quatrelle_mime::with_date(&raw, now()) } else { raw };
            if item.error.as_deref().is_some_and(|e| e.starts_with("refused:")) {
                // Refused for good: waits for the user to edit or delete it.
                continue;
            }
            let sent = self.deliver(&raw, &item.from, &item.recipients, item.sent_folder.as_deref());
            match sent {
                Ok(()) => {
                    self.store.outbox_remove(&self.acc.id, item.id);
                    let folder = item.sent_folder.take();
                    self.after_sent(item.id, &raw, folder);
                }
                Err(e) => {
                    let retry = e.is_retryable();
                    item.attempts = item.attempts.saturating_add(1);
                    item.error = Some(if retry { e.to_string() } else { format!("refused: {e}") });
                    let _ = self.store.outbox_update(&self.acc.id, &item);
                    if retry {
                        self.sink.emit(Event::Queued { account: self.id(), id: item.id, error: e.to_string() });
                        return;
                    }
                    self.sink.emit(Event::SendFailed { account: self.id(), id: item.id, error: e.to_string() });
                }
            }
        }
    }

    /// A message went out: tell the engine and file a copy in Sent.
    fn after_sent(&mut self, id: u64, raw: &[u8], sent_folder: Option<String>) {
        self.sink.emit(Event::Sent { account: self.id(), id });
        if self.acc.is_api() {
            // Filed when it was submitted.
            let sent = sent_folder.or_else(|| self.folders.iter().find(|f| f.role == FolderRole::Sent).map(|f| f.path.clone()));
            if let Some(f) = sent.filter(|_| self.conn.is_some()) {
                self.sync_one(&f);
                // Graph and Exchange file the copy a moment later: look again then.
                if matches!(self.conn, Some(Conn::Graph(_) | Conn::Ews(_))) {
                    let now = Instant::now();
                    self.soon.retain(|(_, g)| *g != f);
                    self.soon.push((now + SENT_SOON, f.clone()));
                    self.soon.push((now + SENT_LATER, f));
                }
            }
            return;
        }
        // Gmail files sent mail itself; elsewhere keep a copy in Sent.
        let files_itself = self.acc.smtp.host.ends_with("gmail.com") || self.acc.smtp.host.ends_with("googlemail.com");
        if let Some(folder) = sent_folder.filter(|_| !files_itself) {
            let r = (|| -> Result<()> {
                if self.conn.is_none() {
                    self.connect()?;
                }
                // POP: Sent Items is on this computer.
                if let Some(Conn::Pop) = self.conn.as_ref() {
                    let flags = quatrelle_model::Flags { seen: true, ..Default::default() };
                    return crate::pop::file_locally(&self.store, &self.acc.id, &folder, raw, flags, now()).map(drop);
                }
                self.client()?.append(&folder, "\\Seen", raw).map(|_| ())
            })();
            match r {
                Ok(()) => self.sync_one(&folder),
                Err(e) => self.notice(format!("sent, but no copy was saved in Sent Items: {e}")),
            }
        }
    }

    /// Re-apply the edits still waiting that touch `path` to its freshly saved cache; `true` when
    /// the cache changed as a result.
    fn reapply_waiting(&mut self, path: &str) -> bool {
        let mut any = false;
        for op in self.store.load_ops(&self.acc.id).unwrap_or_default() {
            let touches = match &op {
                Op::EmptyFolder { path: p } => p == path,
                o if o.is_folder_op() => false,
                o => o.keys().iter().any(|k| k.folder == path),
            };
            if touches && self.store.apply_op(&self.acc.id, &op).is_ok_and(|c| !c.is_empty()) {
                any = true;
            }
        }
        any
    }

    /// Replay offline changes on the server, oldest first.
    fn replay(&mut self) {
        let mut destinations: Vec<String> = Vec::new();
        let mut relist = false;
        let trash = self.folders.iter().find(|f| f.role == FolderRole::Trash).map(|f| f.path.clone());
        // The queue is ops.json's: bounded, in case taking an entry out keeps failing.
        for _ in 0..100_000 {
            let Some(op) = self.store.load_ops(&self.acc.id).unwrap_or_default().into_iter().next() else { break };
            let r = match self.conn.as_mut() {
                Some(Conn::Imap(c)) => apply_remote_with(c, &op, trash.as_deref()),
                Some(Conn::Jmap(c)) => jmap::apply_remote(c, &self.jstate, &op),
                Some(Conn::Graph(c)) => crate::graph::apply_remote(c, &self.jstate, &op),
                Some(Conn::Ews(c)) => crate::ews::apply_remote(c, &self.jstate, &op),
                // POP: changes are made here only (the cache has them already).
                Some(Conn::Pop) => Ok(()),
                None => Err(NetError::Closed("not connected".into())),
            };
            match r {
                Ok(()) => {
                    match &op {
                        Op::Move { to, .. } => destinations.push(to.clone()),
                        Op::RenameFolder { to, .. } => {
                            relist = true;
                            destinations.push(to.clone());
                        }
                        Op::CreateFolder { .. } | Op::DeleteFolder { .. } => relist = true,
                        Op::EmptyFolder { path } => destinations.push(path.clone()),
                        Op::SetFlag { .. } | Op::SetKeyword { .. } | Op::Expunge { .. } => {}
                    }
                    if let Err(e) = self.store.remove_op(&self.acc.id, &op) {
                        self.notice(e.to_string());
                        break;
                    }
                    // The next edit may name this folder (or its new name): refresh the ids now
                    // rather than after the pass.
                    if op.is_folder_op() && relist {
                        relist = !self.relist();
                    }
                }
                Err(e) if e.is_fatal() => {
                    self.failed(e);
                    return;
                }
                Err(e) => {
                    // The server refused: drop the change and resync so the cache matches the
                    // server again (forcing a full flag check).
                    if let Err(e) = self.store.remove_op(&self.acc.id, &op) {
                        self.notice(e.to_string());
                        break;
                    }
                    let mut folders: Vec<String> = op.keys().iter().map(|k| k.folder.clone()).collect();
                    folders.dedup();
                    for f in &folders {
                        if let Ok(mut st) = self.store.load_folder(&self.acc.id, f) {
                            st.highest_modseq = 0;
                            st.uid_next = 0;
                            let _ = self.store.save_folder(&self.acc.id, f, &st);
                        }
                    }
                    relist |= op.is_folder_op();
                    self.sink.emit(Event::OpFailed { account: self.id(), op, error: e.to_string() });
                    destinations.extend(folders);
                }
            }
        }
        if relist {
            self.relist();
        }
        destinations.sort();
        destinations.dedup();
        for d in destinations {
            if self.conn.is_some() {
                self.sync_one(&d);
            }
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
        Op::SetKeyword { keys, keyword, on } if quatrelle_model::is_keyword(keyword) => {
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
        Event::Notice { account, message } => log::info!("{account}: {message}"),
        Event::DavSynced { account, sync } => {
            let names: Vec<&str> = sync.calendars.iter().flatten().map(|c| c.name.as_str()).collect();
            log::info!("{account}: calendars {names:?}, {} events", sync.events.len());
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
/// it is in), then what mail is about, the rest, and the archive last — an account with public
/// folders and years of archives takes minutes, and the Inbox must not wait for them.
fn sync_order(folders: &[Folder]) -> Vec<String> {
    let rank = |f: &Folder| match f.role {
        quatrelle_model::FolderRole::Inbox => 0,
        quatrelle_model::FolderRole::Drafts => 1,
        quatrelle_model::FolderRole::Sent => 2,
        quatrelle_model::FolderRole::Junk => 3,
        quatrelle_model::FolderRole::Trash => 4,
        quatrelle_model::FolderRole::Other => 5,
        quatrelle_model::FolderRole::Archive => 6,
    };
    let mut v: Vec<(&Folder, u8)> = folders.iter().filter(|f| f.selectable).map(|f| (f, rank(f))).collect();
    v.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.path.cmp(&b.0.path)));
    v.into_iter().map(|(f, _)| f.path.clone()).collect()
}

/// What a connection of its own sends back while it reads folders.
enum SyncDone {
    /// A folder read, and what changed there (or why not, as it was: a connection that timed
    /// out must stop the sync, not pass for "no such folder").
    Folder(String, std::result::Result<SyncReport, NetError>),
    /// The numbers it gave while reading, when it's done.
    State(jmap::State),
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
    opts: SyncOptions,
    mut state: jmap::State,
    queue: Arc<Mutex<VecDeque<String>>>,
    tx: Sender<SyncDone>,
) {
    let mut conn = match connect_for_sync(&acc, &store, &secret) {
        Ok(c) => c,
        Err(e) => {
            log::info!("{}: a second connection wasn't taken ({e}): the rest syncs on the first", acc.id);
            let _ = tx.send(SyncDone::Refused);
            let _ = tx.send(SyncDone::State(state));
            return;
        }
    };
    while let Some(path) = queue.lock().ok().and_then(|mut q| q.pop_front()) {
        let r = match &mut conn {
            Conn::Imap(c) => sync_folder(c, &store, &acc.id, &path, &opts),
            Conn::Jmap(c) => jmap::sync_folder(c, &mut state, &store, &acc.id, &path, &opts),
            Conn::Graph(c) => crate::graph::sync_folder(c, &mut state, &store, &acc.id, &path, &opts),
            Conn::Ews(c) => crate::ews::sync_folder(c, &mut state, &store, &acc.id, &path, &opts),
            Conn::Pop => Err(NetError::No("POP accounts read mail on one connection".into())),
        };
        let closed = matches!(&r, Err(NetError::Closed(_)));
        if tx.send(SyncDone::Folder(path, r)).is_err() {
            break;
        }
        if closed {
            break;
        }
    }
    if let Conn::Imap(mut c) = conn {
        c.logout();
    }
    let _ = tx.send(SyncDone::State(state));
}

fn jumps_queue(j: &Job) -> bool {
    matches!(
        j,
        Job::FetchBody { .. }
            | Job::FetchPhoto { .. }
            | Job::GetServerRules { .. }
            | Job::GetVacation { .. }
            | Job::SetVacation { .. }
            | Job::SyncDav { .. }
            | Job::SyncFolder { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A full sync walks the Inbox first, then what mail is about, then the rest, the archive
    /// last: a mailbox of public folders and years of archives must not hold the Inbox back.
    #[test]
    fn a_full_sync_starts_with_the_inbox() {
        use quatrelle_model::FolderRole;
        let f = |path: &str, role: FolderRole| Folder {
            account: "a1".into(),
            path: path.into(),
            delimiter: Some('/'),
            role,
            selectable: true,
            unread: 0,
            total: 0,
            name: None,
            mailbox: None,
            mailbox_role: quatrelle_model::FolderRole::Other,
        };
        let folders = vec![
            f("ASP", FolderRole::Other),
            f("Archives/2025", FolderRole::Archive),
            f("Public Folders", FolderRole::Other),
            f("Éléments envoyés", FolderRole::Sent),
            f("Brouillons", FolderRole::Drafts),
            f("Corbeille", FolderRole::Trash),
            f("INBOX", FolderRole::Inbox),
            Folder { selectable: false, ..f("Conteneur", FolderRole::Other) },
        ];
        assert_eq!(
            sync_order(&folders),
            ["INBOX", "Brouillons", "Éléments envoyés", "Corbeille", "ASP", "Public Folders", "Archives/2025"],
            "Inbox, drafts, sent, trash, the rest (by name), the archive last; no containers"
        );
    }

    /// A server's rules split by whose they are: the ones Quatrelle holds the id of, and the
    /// ones it didn't make (Outlook's own, the server's web page).
    #[test]
    fn server_rules_split_by_whose_they_are() {
        let rule = |name: &str, id: &str| quatrelle_model::organize::ForeignRule {
            account: String::new(),
            id: id.into(),
            name: name.into(),
            enabled: true,
            runnable: false,
            rule: None,
            summary: String::new(),
        };
        let (mine, others) = split_rules(vec![rule("Outlook rule", "R9"), rule("Invoices", "R1")], &["R1".to_string()]);
        assert_eq!(mine, vec![("Invoices".to_string(), "R1".to_string())]);
        assert_eq!(others.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["Outlook rule"]);
        let (mine, others) = split_rules(vec![rule("Theirs", "R2")], &[]);
        assert!(mine.is_empty());
        assert_eq!(others.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["Theirs"], "nothing of ours there: all of it is theirs");
    }

    /// Automatic replies are answered between two folders of a long first sync, not after it
    /// (File › Info said "Checking with the server…" all the while).
    #[test]
    fn automatic_replies_jump_the_full_syncs_queue() {
        assert!(jumps_queue(&Job::GetVacation { account: "a".into() }));
        assert!(jumps_queue(&Job::SetVacation { account: "a".into(), vacation: Default::default() }));
        assert!(jumps_queue(&Job::SyncDav { account: "a".into() }));
        assert!(jumps_queue(&Job::SyncFolder { account: "a".into(), folder: "INBOX".into() }));
        assert!(!jumps_queue(&Job::SyncAll { account: "a".into() }));
    }

    /// Sent Items asked for after each of several messages is read once.
    #[test]
    fn a_folder_waiting_to_be_read_isnt_queued_twice() {
        let mut q = VecDeque::new();
        let sent = Job::SyncFolder { account: "a".into(), folder: "Sent".into() };
        push_job(&mut q, sent.clone());
        push_job(&mut q, Job::SyncFolder { account: "a".into(), folder: "INBOX".into() });
        push_job(&mut q, sent.clone());
        push_job(&mut q, Job::GetVacation { account: "a".into() });
        push_job(&mut q, Job::GetVacation { account: "a".into() });
        assert_eq!(q.len(), 4, "only the folder read is merged");
    }

    #[test]
    fn memory_credentials() {
        let m = MemoryCredentials::default();
        let a = AccountConfig { id: "a1".into(), ..Default::default() };
        assert_eq!(m.get(&a), None);
        m.set(&a, "pw").unwrap();
        assert_eq!(m.get(&a).as_deref(), Some("pw"));
        m.delete(&a);
        assert_eq!(m.get(&a), None);
    }

    /// Microsoft can take a token and then refuse the mailbox ("User is authenticated but not
    /// connected"). The address the browser signed in as is tried; otherwise it's a refusal that
    /// says what to do, not a sign-in to ask for again.
    #[test]
    fn a_mailbox_microsoft_wont_open_says_why() {
        let st = crate::fake::State::new();
        st.lock().unwrap().not_connected.push("zoe@hotmail.example".into());
        let port = crate::fake::serve_tcp(&st);
        let store = crate::fake::tests_store("closed-mailbox");
        let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
        let mut acc = AccountConfig {
            id: "a1".into(),
            email: "zoe@hotmail.example".into(),
            imap: quatrelle_model::ServerConfig { host: "127.0.0.1".into(), port, security: quatrelle_model::Security::None },
            oauth: Some("microsoft".into()),
            ..Default::default()
        };
        let w = Worker::new(acc.clone(), Auth::shared("refresh"), store.clone(), pool.sink.clone(), SyncOptions::default());
        let e = w.imap_with_token("zoe@hotmail.example", crate::fake::GOOD_TOKEN).err().unwrap();
        assert!(e.to_string().starts_with(quatrelle_model::MICROSOFT_IMAP_CLOSED) && e.to_string().contains("IMAP"), "{e:?}");
        // The browser signed in as the mailbox's own address: that one opens it.
        acc.oauth_user = Some("me@studio.example".into());
        let w = Worker::new(acc.clone(), Auth::shared("refresh"), store.clone(), pool.sink.clone(), SyncOptions::default());
        assert!(w.imap_with_token("zoe@hotmail.example", crate::fake::GOOD_TOKEN).is_ok());
        // Signed in as another account that can't open it either: named in the refusal.
        acc.oauth_user = Some("someone@else.example".into());
        let w = Worker::new(acc, Auth::shared("refresh"), store, pool.sink.clone(), SyncOptions::default());
        let e = w.imap_with_token("zoe@hotmail.example", crate::fake::GOOD_TOKEN).err().unwrap();
        assert!(matches!(&e, NetError::Refused(m) if m.contains("signed in as someone@else.example")), "{e:?}");
        // A token that's simply refused is still a sign-in to ask for again.
        let w = Worker::new(
            AccountConfig { oauth_user: None, ..w.acc.clone() },
            Auth::shared("refresh"),
            crate::fake::tests_store("closed-mailbox-2"),
            pool.sink.clone(),
            SyncOptions::default(),
        );
        assert!(matches!(w.imap_with_token("zoe@hotmail.example", "expired"), Err(NetError::Auth(_))));
    }

    /// A Quatrelle data file (`.quatrelle`) comes in through the same job as a `.pst`.
    #[test]
    fn a_quatrelle_data_file_is_imported() {
        let t = std::env::temp_dir().join(format!("quatrelle-worker-datafile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        let old = Store::open_packed(t.join("old"), t.join("Old Files")).unwrap();
        old.save_accounts(&[AccountConfig { id: "a1".into(), email: "zoe@studio.example".into(), ..Default::default() }]).unwrap();
        old.save_folders("a1", &[Folder { path: "INBOX".into(), role: FolderRole::Inbox, ..Default::default() }]).unwrap();
        let k = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 3 };
        let mut st = old.load_folder("a1", "INBOX").unwrap();
        st.messages.push(quatrelle_model::Summary { key: k, subject: "Kept".into(), ..Default::default() });
        old.save_folder("a1", "INBOX", &st).unwrap();
        let file = old.data_file("a1").unwrap();
        drop(old);
        let store = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
        let mut p = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
        p.submit(Job::ImportPst { account: "p1".into(), path: file.to_string_lossy().into_owned() });
        let end = Instant::now() + Duration::from_secs(20);
        let mut seen = Vec::new();
        while Instant::now() < end && !seen.iter().any(|e| matches!(e, Event::Imported { done: true, .. })) {
            seen.extend(p.wait(Duration::from_millis(100)));
        }
        assert!(seen.iter().any(|e| matches!(e, Event::Imported { messages: 1, done: true, error: None, .. })), "{seen:?}");
        assert!(seen.iter().any(|e| matches!(e, Event::Folders { folders, .. } if folders.len() == 1)));
        assert_eq!(store.load_folder("p1", "INBOX").unwrap().messages[0].key.account, "p1");
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn pool_without_password_asks_for_one() {
        let dir = std::env::temp_dir().join(format!("quatrelle-pool-{}", std::process::id()));
        let store = Store::open(&dir).unwrap();
        let mut p = Pool::new(store, Arc::new(MemoryCredentials::default()));
        let a = AccountConfig { id: "a1".into(), ..Default::default() };
        p.submit(Job::Start { account: a, password: None });
        assert!(matches!(p.poll().as_slice(), [Event::Status { status: SyncStatus::NeedsPassword { .. }, .. }]));
        p.submit(Job::SyncAll { account: "a1".into() });
        assert!(matches!(p.poll().as_slice(), [Event::Notice { .. }]));
        let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
        p.submit(Job::FetchBody { key: key.clone() });
        assert!(matches!(p.poll().as_slice(), [Event::BodyFailed { .. }]));
        p.store.write_body(&key, b"Subject: cached\r\n\r\nx").unwrap();
        p.submit(Job::FetchBody { key });
        assert!(matches!(p.poll().as_slice(), [Event::Body { .. }]));
        let _ = std::fs::remove_dir_all(dir);
    }
}
