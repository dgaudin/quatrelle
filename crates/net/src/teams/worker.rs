// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The Teams worker: the one thread that talks to Teams. It owns the Graph client, the access
//! token, the limiter and the local copy, takes the engine's jobs in order and answers with
//! events.
//!
//! * Nothing runs unasked. The browser opens only for [`Job::SignIn`], through the window
//!   (`Event::OpenUrl`); a kept sign-in is gone on with only on [`Job::Start`].
//! * One worker for the pool's life: the copy has one writer and the token one keeper. Switched
//!   off ([`Job::Stop`]) or signed out it asks nothing, holds no file and no token, and waits on
//!   its channel for the jobs after, taken in their order; the thread ends with the channel.
//! * The refresh token lives in the system keychain only ([`keyring`], [`REFRESH_KEY`]), the
//!   access token in memory. Both are renewed here and nowhere else, so two renewals never race;
//!   every request to Teams goes from this thread, so two reads of a chat are never in flight.
//! * The account is what Microsoft says it is (`signing.rs`): who by `GET /me`, of which
//!   organisation by the sign-in's own `tid`, else by the user's own entry among a chat's
//!   members, never by a chat's `tenantId`. The copy is opened for that identity only.
//! * Waits are deadlines the loop looks at, never a sleep: a job is heard at once. Each turn
//!   runs behind `fenced`: a panic costs the step in hand, and the next one waits.
//! * The log says counts, HTTP statuses with Graph's error codes and keyed hashes
//!   ([`Worker::hashed`]): never a name, an address, a topic or an id.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use quatrelle_model::Event;
use quatrelle_model::job::Secret;
use quatrelle_model::pim::teams::{self, ChatMessage, Job, Status, TeamsSettings};
use quatrelle_store::Store;
use sha2::{Digest, Sha256};

use super::backfill::{self, Backfill};
use super::graph::TeamsClient;
use super::limits::{Cadence, Limiter, Route, retry_wait};
use super::store::Copy;
use super::sync::{self, SyncState};
use crate::oauth::{self, Tokens};
use crate::worker::Credentials;
use crate::worker::connect::{Panics, fenced};
use crate::{NetError, Result};

mod signing;

pub(crate) use signing::{Authority, Microsoft};
pub use signing::{REFRESH_KEY, keyring};
#[cfg(test)]
pub(crate) use signing::{is_guid, missing_scope};

/// How the worker reports: the pool's sink in the app, a list in the tests.
pub(crate) type Emit = Arc<dyn Fn(Event) + Send + Sync>;

/// The least between two turns of the loop: whatever a step leaves due, the loop never spins.
const TURN: Duration = Duration::from_millis(20);
/// How often the loop looks while a sign-in in the browser is awaited.
const AWAITING: Duration = Duration::from_millis(200);
/// After a failure (no network, a server's error): tried again then.
const RETRY: Duration = Duration::from_secs(10);

/// What a test changes of a worker: a step that panics, and a shorter wait after it.
#[cfg(test)]
#[derive(Clone, Default)]
pub(crate) struct Hooks {
    /// The next reads of the chat list panic, this many of them: a bug met in a step.
    pub(crate) trip_list: Arc<std::sync::atomic::AtomicU32>,
    /// The wait after a panic, in place of [`Panics::wait`]'s half minute.
    pub(crate) panic_wait: Option<Duration>,
    /// How many times the loop woke, for a job or for a deadline.
    pub(crate) turns: Arc<std::sync::atomic::AtomicU32>,
    /// The next openings of the copy panic, this many of them: a damaged file, a parser's bug.
    pub(crate) trip_open: Arc<std::sync::atomic::AtomicU32>,
    /// The next sign-ins panic once the browser has answered, this many of them.
    pub(crate) trip_sign_in: Arc<std::sync::atomic::AtomicU32>,
    /// The history reads of this chat panic once their page has come.
    pub(crate) trip_history: Option<String>,
}

#[cfg(test)]
impl Hooks {
    fn trip(&self) {
        Hooks::trip_on(&self.trip_list);
    }

    fn trip_on(count: &std::sync::atomic::AtomicU32) {
        use std::sync::atomic::Ordering::SeqCst;
        if count.fetch_update(SeqCst, SeqCst, |n| n.checked_sub(1)).is_ok() {
            panic!("a panic the test asked for");
        }
    }
}

/// The pool's hold on the worker, for the pool's life: jobs go in, in order; dropped, the worker
/// ends.
pub(crate) struct Handle {
    tx: Sender<Job>,
    #[cfg(test)]
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Handle {
    pub(crate) fn submit(&self, job: Job) {
        let _ = self.tx.send(job);
    }

    /// Whether the worker's thread ends within `wait` of its channel closing.
    #[cfg(test)]
    pub(crate) fn ends_within(self, wait: Duration) -> bool {
        let Handle { tx, thread } = self;
        drop(tx);
        let until = Instant::now() + wait;
        while Instant::now() < until && thread.as_ref().is_some_and(|t| !t.is_finished()) {
            std::thread::sleep(Duration::from_millis(5));
        }
        thread.as_ref().is_none_or(std::thread::JoinHandle::is_finished)
    }
}

/// Start the worker on a thread named `teams`. `graph` is Graph's address
/// (`https://graph.microsoft.com/v1.0`; a fake's in tests), `authority` who signs in and renews.
/// Nothing is asked of anyone until a job says so.
pub(crate) fn start(store: Store, creds: Arc<dyn Credentials>, emit: Emit, graph: String, authority: Box<dyn Authority>) -> std::io::Result<Handle> {
    spawn(Worker::new(store, creds, emit, graph, authority)?)
}

/// [`start`], with a test's hooks.
#[cfg(test)]
pub(crate) fn start_with(
    store: Store,
    creds: Arc<dyn Credentials>,
    emit: Emit,
    graph: String,
    authority: Box<dyn Authority>,
    hooks: Hooks,
) -> std::io::Result<Handle> {
    let mut w = Worker::new(store, creds, emit, graph, authority)?;
    w.hooks = hooks;
    spawn(w)
}

fn spawn(w: Worker) -> std::io::Result<Handle> {
    let (tx, rx) = mpsc::channel();
    let thread = std::thread::Builder::new().name("teams".into()).spawn(move || w.run(&rx))?;
    #[cfg(not(test))]
    drop(thread);
    Ok(Handle {
        tx,
        #[cfg(test)]
        thread: Some(thread),
    })
}

/// What the worker's thread owns. The synchronisation, the history, the pictures and the search
/// are functions on it (`sync.rs`, `backfill.rs`, `pictures.rs`, `search.rs`).
pub(crate) struct Worker {
    /// The Graph client, once signed in and Graph has said whose token it is; `None` while
    /// signed out, after "sign in again", or before the network is there at a start.
    pub(crate) client: Option<TeamsClient>,
    /// The local copy, opened for the account signed in (and left open to read after "sign in
    /// again").
    pub(crate) copy: Option<Copy>,
    pub(crate) limiter: Limiter,
    /// The access token and when it runs out: in memory only.
    pub(crate) tokens: Option<Tokens>,
    /// The Options: the application and directory signed in with, and who signed in. Its
    /// `object_id` and `tenant_id` are the identity the copy is bound to.
    pub(crate) settings: TeamsSettings,
    pub(crate) emit: Emit,
    /// The chat on screen.
    pub(crate) open: Option<String>,
    /// Set by a step whose route must wait ([`Worker::may`]): when it may go. The loop doesn't
    /// turn again before, however due the step's cadence is.
    pub(crate) held: Option<Instant>,
    /// The refresh token, for the renewals: the keychain's, or the session's only when there's
    /// no keychain (`no_keychain`).
    refresh: Option<Secret>,
    no_keychain: bool,
    /// When the access token was last renewed.
    renewed: Option<Instant>,
    /// Whether Teams is asked anything: signed in and going.
    polling: bool,
    /// Nothing is asked before then: after a failure, or a panic.
    not_before: Option<Instant>,
    /// The status last reported.
    said: Option<Status>,
    /// Where the synchronisation stands between two steps (`sync.rs`).
    pub(crate) sync: SyncState,
    /// The sign-in awaited from the browser (its number), and where its thread answers.
    signing: Option<u64>,
    sign_ins: u64,
    answers: (Sender<signing::Authorized>, Receiver<signing::Authorized>),
    /// What the log's hashes are keyed with: made at start, never written.
    key: Vec<u8>,
    /// The cache the copy is in (its drive is what the history looks at before it downloads).
    pub(crate) store: Store,
    creds: Arc<dyn Credentials>,
    graph: String,
    authority: Arc<dyn Authority>,
    /// Where the history stands (`backfill.rs`).
    pub(crate) backfill: Backfill,
    #[cfg(test)]
    pub(crate) hooks: Hooks,
}

impl Worker {
    /// A worker signed out, asking nothing. Fails only when the system gives no randomness for
    /// the log's key.
    pub(crate) fn new(
        store: Store,
        creds: Arc<dyn Credentials>,
        emit: Emit,
        graph: String,
        authority: Box<dyn Authority>,
    ) -> std::io::Result<Worker> {
        let key = oauth::secret_token().map_err(|_| std::io::Error::other("no randomness to key the log's hashes with"))?.into_bytes();
        Ok(Worker {
            client: None,
            copy: None,
            limiter: Limiter::new(Instant::now()),
            tokens: None,
            settings: TeamsSettings::default(),
            emit,
            open: None,
            held: None,
            refresh: None,
            no_keychain: false,
            renewed: None,
            polling: false,
            not_before: None,
            said: None,
            sync: SyncState::default(),
            backfill: Backfill::default(),
            signing: None,
            sign_ins: 0,
            answers: mpsc::channel(),
            key,
            store,
            creds,
            graph: graph.trim().trim_end_matches('/').to_string(),
            authority: Arc::from(authority),
            #[cfg(test)]
            hooks: Hooks::default(),
        })
    }

    /// The thread: a job as soon as one comes, else a turn at the next deadline, or nothing until
    /// a job when there's nothing to ask; until the channel's end (the pool's).
    fn run(mut self, rx: &Receiver<Job>) {
        let mut panics = Panics::default();
        loop {
            let now = Instant::now();
            let job = match self.wake(now) {
                Some(at) => match rx.recv_timeout(at.saturating_duration_since(now)) {
                    Ok(job) => Some(job),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => break,
                },
                None => match rx.recv() {
                    Ok(job) => Some(job),
                    Err(_) => break,
                },
            };
            #[cfg(test)]
            self.hooks.turns.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let done = fenced(|| {
                if let Some(job) = job {
                    self.job(job);
                }
                self.turn(Instant::now());
            });
            if done.is_none() {
                self.after_panic(&mut panics);
            }
        }
        log::info!("teams: the worker ended");
    }

    /// When the loop looks again, a job apart: the next cadence due, the time a held step may
    /// go, the end of a wait after a failure; never at once ([`TURN`] at least). `None` when
    /// there is nothing to look at until a job comes: switched off, signed out, or waiting to be
    /// signed in again, and no sign-in on its way from the browser.
    pub(crate) fn wake(&self, now: Instant) -> Option<Instant> {
        let after = |wait: Duration| now.checked_add(wait).unwrap_or(now);
        let mut wake = None;
        if self.polling {
            let mut at = self.limiter.next_deadline(now);
            for later in [self.held, self.not_before].into_iter().flatten() {
                at = at.max(later);
            }
            wake = Some(at.max(after(TURN)));
        }
        if self.signing.is_some() {
            wake = Some(wake.map_or(after(AWAITING), |at| at.min(after(AWAITING))));
        }
        wake
    }

    /// A step panicked: what it was doing is given up, and nothing is asked for a wait that
    /// grows while panics keep coming (jobs are still heard).
    fn after_panic(&mut self, panics: &mut Panics) {
        let now = Instant::now();
        let wait = panics.wait(now);
        #[cfg(test)]
        let wait = self.hooks.panic_wait.unwrap_or(wait);
        log::error!("teams: internal error: the step in hand was given up; going on in {} s", wait.as_secs());
        self.sync.give_up();
        self.held = None;
        self.not_before = now.checked_add(wait);
    }

    /// One job of the engine's.
    pub(crate) fn job(&mut self, job: Job) {
        match job {
            Job::Start { settings } => self.start(settings),
            Job::SignIn { client_id, tenant } => self.sign_in(&client_id, &tenant),
            Job::SignOut { delete_copy } => self.sign_out(delete_copy),
            // Switched off: nothing asked, no file, no token; the thread stays for what comes
            // next. No status stands (what a start says after is said again).
            Job::Stop => {
                self.halt();
                self.copy = None;
                self.open = None;
                self.said = None;
                log::info!("teams: switched off");
            }
            Job::Open { chat } => sync::open(self, chat),
            Job::Page { chat, before } => sync::page(self, &chat, before),
            // Presence, the search and pictures: with the modules that read them.
            Job::Visible { .. } | Job::Search { .. } | Job::Picture { .. } => {}
        }
    }

    /// One turn of the loop at `now`: a sign-in's answer taken, then, when Teams may be asked,
    /// one step of what is due.
    pub(crate) fn turn(&mut self, now: Instant) {
        self.authorized(now);
        if !self.polling || self.not_before.is_some_and(|at| now < at) {
            return;
        }
        self.not_before = None;
        self.held = None;
        if self.ready(now) {
            self.tick(now);
        }
    }

    /// One step of whatever cadence is due (`sync.rs`), else of the history (`backfill.rs`): a
    /// request at most.
    pub(crate) fn tick(&mut self, now: Instant) {
        if !sync::tick(self, now) {
            backfill::step(self, now);
        }
    }

    /// The list is read: the first copy's progress while it runs, then up to date; or connected
    /// without a keychain, which the card goes on saying, the copy running or not (Review Focus
    /// 3). None hides "Offline" or "sign in again" when they come.
    pub(crate) fn up_to_date(&mut self) {
        let copying = backfill::progress(self);
        self.status(match copying {
            _ if self.no_keychain => Status::NoKeychain,
            Some(copying) => copying,
            None => Status::UpToDate,
        });
    }

    /// A test's panic in the list pass, when it asked for one.
    #[cfg(test)]
    pub(crate) fn trip_list(&self) {
        self.hooks.trip();
    }

    /// Whether a request of `route` may go at `now`, by the limiter. When it may not, the loop is
    /// told when ([`Worker::held`]) and the step is left for a later turn: nothing here sleeps.
    pub(crate) fn may(&mut self, route: Route, now: Instant) -> bool {
        let wait = self.limiter.wait(route, now);
        if wait.is_zero() {
            return true;
        }
        let at = now.checked_add(wait).unwrap_or(now);
        self.held = Some(self.held.map_or(at, |held| held.min(at)));
        false
    }

    /// A request to Graph on `route`, counted by the limiter. Refused for its token (401), the
    /// token is renewed and the request made once more; refused again, or the renewal refused,
    /// it is "sign in again" and nothing more is asked.
    pub(crate) fn request<T>(&mut self, route: Route, now: Instant, call: impl Fn(&TeamsClient) -> Result<T>) -> Result<T> {
        let first = self.ask(route, now, &call);
        let Err(NetError::Auth(_)) = first else { return first };
        log::info!("teams: the access token was refused; renewing it");
        self.renew(now)?;
        let again = self.ask(route, now, &call);
        if let Err(NetError::Auth(_)) = again {
            log::warn!("teams: the renewed token was refused too");
            self.sign_in_again();
        }
        again
    }

    fn ask<T>(&mut self, route: Route, now: Instant, call: &impl Fn(&TeamsClient) -> Result<T>) -> Result<T> {
        self.limiter.took(route, now);
        match &self.client {
            Some(client) => call(client),
            None => Err(NetError::Refused("Teams isn't signed in".into())),
        }
    }

    /// A step of `cadence` failed. Busy (429, 503): nothing at all is asked of Teams while its
    /// `Retry-After` runs, then that cadence goes at a slower pace. Anything else: said by its
    /// kind, "Offline" when it's the network, and tried again in a while. A refused token was
    /// dealt with where it was met.
    pub(crate) fn failed(&mut self, cadence: Cadence, what: &str, e: &NetError, now: Instant) {
        if !self.polling {
            return;
        }
        log::warn!("teams: {what} failed: {}", kind(e));
        if busy(e) {
            let wait = self.client.as_ref().and_then(TeamsClient::retry_after);
            self.limiter.throttled(cadence, wait, now);
            if let Some(until) = now.checked_add(retry_wait(wait)) {
                self.limiter.pause_all(until);
            }
            return;
        }
        self.pause(e, now);
    }

    /// Nothing is asked for a while after `e`; "Offline" when it's the network.
    pub(crate) fn pause(&mut self, e: &NetError, now: Instant) {
        if of_the_network(e) {
            self.status(Status::Offline);
        }
        self.not_before = now.checked_add(RETRY);
    }

    /// Report where the connection stands, when that changed.
    pub(crate) fn status(&mut self, status: Status) {
        if self.said.as_ref() != Some(&status) {
            self.said = Some(status.clone());
            self.say(teams::Event::Status { status });
        }
    }

    pub(crate) fn say(&self, event: teams::Event) {
        (self.emit)(Event::Teams { event });
    }

    /// Nothing more is asked of Teams: the tokens leave memory, the copy stays as it is.
    fn halt(&mut self) {
        self.polling = false;
        self.client = None;
        self.tokens = None;
        self.refresh = None;
        self.renewed = None;
        self.sync = SyncState::default();
        self.backfill.reset();
        self.signing = None;
        self.held = None;
        self.not_before = None;
    }

    /// An id as the log may say it: 8 hex digits of the SHA-256 of this run's key and the id.
    /// The key is random, made at start and never written, so a hash says nothing to who
    /// doesn't have it, nor from one run to the next.
    pub(crate) fn hashed(&self, id: &str) -> String {
        self.digest(id).iter().take(4).map(|b| format!("{b:02x}")).collect()
    }

    /// The same hash as a number: what the limiter knows a chat by ([`Route::Messages`]).
    pub(crate) fn key_of(&self, id: &str) -> u64 {
        self.digest(id).iter().take(8).fold(0, |n, b| (n << 8) | u64::from(*b))
    }

    fn digest(&self, id: &str) -> [u8; 32] {
        Sha256::new().chain_update(&self.key).chain_update(id.as_bytes()).finalize().into()
    }
}

/// The messages of a read that may be kept, shown and indexed: those with an id. One whose id
/// couldn't be read comes from the reader with an empty one (and can't be shown): nothing is
/// ever kept, listed or indexed under an empty id. Counted in the log, with those that have
/// their id but can't be shown.
pub(crate) fn with_ids(messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
    let read = messages.len();
    let kept: Vec<ChatMessage> = messages.into_iter().filter(|m| !m.id.is_empty()).collect();
    let (nameless, unshowable) = (read.saturating_sub(kept.len()), kept.iter().filter(|m| m.unshowable).count());
    if nameless > 0 || unshowable > 0 {
        log::warn!("teams: of {read} messages read, {nameless} had no id and were left out, {unshowable} can't be shown");
    }
    kept
}

/// A failure as the log may say it: the HTTP status and Graph's error code when that is all it
/// is, else its kind alone. Never an error's own words: they can carry an address, or what a
/// server wrote.
pub(super) fn kind(e: &NetError) -> String {
    match e {
        NetError::Protocol(m) | NetError::Closed(m) if http_status(m) => m.clone(),
        NetError::Io(e) => format!("the network ({:?})", e.kind()),
        NetError::Tls(_) => "the secure connection".into(),
        NetError::Auth(_) => "the sign-in refused".into(),
        NetError::Closed(_) => "the connection closed".into(),
        NetError::Protocol(_) => "an answer that couldn't be read".into(),
        NetError::Store(_) => "the local copy".into(),
        _ => "refused".into(),
    }
}

/// Whether a failure is Graph asking to wait: a 429 or a 503.
pub(super) fn busy(e: &NetError) -> bool {
    matches!(e, NetError::Closed(m) if m.starts_with("HTTP 429") || m.starts_with("HTTP 503"))
}

/// Whether a failure is the network's (no connection, the secure connection, a connection that
/// closed): nothing of Teams can be asked, whatever is asked.
pub(super) fn of_the_network(e: &NetError) -> bool {
    matches!(e, NetError::Io(_) | NetError::Tls(_)) || matches!(e, NetError::Closed(m) if !http_status(m))
}

/// Whether an error's text is only `HTTP <status>` and an error code, as the client writes them.
fn http_status(m: &str) -> bool {
    m.len() <= 80
        && m.strip_prefix("HTTP ").is_some_and(|rest| rest.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b' ' | b'_' | b'-' | b'.')))
}
