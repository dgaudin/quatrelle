// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! An account's worker: opening a message, from here or from the server, and reading older mail
//! on the server. A message opened while the main connection is at work (a folder's sync, the
//! calendars' and contacts') doesn't wait for that work to end: a copy of the connection, taken
//! as the work began, brings it (issue #24), its text first when that's quicker.

use std::collections::BTreeMap;
use std::sync::PoisonError;

use super::rest::{Rest, Whole, named};
use super::*;

impl Worker {
    pub(super) fn fetch(&mut self, key: MessageKey) {
        // A copy of the connection brings it already (opened during a sync).
        if !self.claim(&key) {
            return;
        }
        if let Some(raw) = self.store.read_body(&key) {
            // Brought by a copy of the connection meanwhile: its summary is filled now that this
            // connection is free to save the list.
            let filled = if self.rest.owes(&key) { crate::sync::keep_source(&self.store, &key, &raw).ok().flatten() } else { None };
            self.sink.emit(Event::Body { key, raw });
            if let Some(summary) = filled {
                self.sink.emit(Event::Filled { summary: Box::new(summary) });
            }
            return;
        }
        let store = self.store.clone();
        let numbered = self.jstate.len();
        let r = (|| -> Result<(Vec<u8>, Option<quatrelle_model::Summary>)> {
            if self.conn.is_none() {
                self.connect()?;
            }
            match self.conn.as_mut() {
                Some(Conn::Jmap(c)) => jmap::fetch_body(c, &self.jstate, &store, &key),
                Some(Conn::Graph(c)) => crate::graph::fetch_body(c, &self.jstate, &store, &key),
                Some(Conn::Ews(c)) => crate::ews::fetch_body(c, &mut self.jstate, &store, &key),
                // POP mail is downloaded whole: what isn't here isn't anywhere.
                Some(Conn::Pop) => {
                    store.read_body(&key).map(|raw| (raw, None)).ok_or_else(|| NetError::No("this message isn't on this computer".into()))
                }
                _ => fetch_body(self.client()?, &store, &key),
            }
        })();
        if self.jstate.len() != numbered {
            // A message found again by its Message-ID: the numbering is worth keeping.
            self.save_jstate();
        }
        match r {
            // Its summary filled from it, said as it is now: the window takes it into the folder
            // it holds (it read all of the folder again for it, 196 ms on 50,000 messages).
            Ok((raw, filled)) => {
                self.sink.emit(Event::Body { key, raw });
                if let Some(summary) = filled {
                    self.sink.emit(Event::Filled { summary: Box::new(summary) });
                }
            }
            Err(e) => {
                self.sink.emit(Event::BodyFailed { key, error: e.to_string() });
                if e.is_fatal() || matches!(e, NetError::Auth(_)) {
                    self.failed(e);
                }
            }
        }
    }

    /// A page of a folder's mail older than what's kept, read on the server and held in memory
    /// by the session (Exchange, as Cached Exchange Mode's "more on the server"): numbered, and
    /// kept numbered as a search's finds are, but not kept.
    pub(super) fn older(&mut self, folder: &str, offset: u32) {
        let account = self.id();
        let before = self.opts.since(now()).unwrap_or(i64::MAX);
        let numbered = self.jstate.len();
        let r = (|| -> Result<(Vec<quatrelle_model::Summary>, bool)> {
            if self.conn.is_none() {
                self.connect()?;
            }
            match self.conn.as_ref() {
                Some(Conn::Ews(c)) => crate::ews::older(c, &mut self.jstate, &account, folder, before, offset as usize, OLDER_PAGE),
                _ => Err(NetError::No("this account reads older mail by keeping more of it".into())),
            }
        })();
        if self.jstate.len() != numbered {
            self.save_jstate();
        }
        match r {
            Ok((items, more)) => {
                for n in items.iter().map(|m| m.key.uid) {
                    if !self.search_numbers.contains(&n) {
                        self.search_numbers.push(n);
                    }
                }
                if self.search_numbers.len() > 4_000 {
                    self.search_numbers.drain(..2_000);
                }
                self.sink.emit(Event::Older { account, folder: folder.to_string(), items, more, error: None });
            }
            Err(e) => {
                self.sink.emit(Event::Older { account, folder: folder.to_string(), items: Vec::new(), more: false, error: Some(e.to_string()) })
            }
        }
    }

    /// A message's source read on the server for showing, not kept (mail older than what's
    /// kept, read online).
    pub(super) fn read_online_body(&mut self, key: MessageKey) {
        let store = self.store.clone();
        let r = (|| -> Result<Vec<u8>> {
            if self.conn.is_none() {
                self.connect()?;
            }
            match self.conn.as_ref() {
                Some(Conn::Ews(c)) => crate::ews::read_online(c, &mut self.jstate, &store, &key),
                _ => Err(NetError::No("this account reads its mail by keeping it".into())),
            }
        })();
        match r {
            Ok(raw) => self.sink.emit(Event::Body { key, raw }),
            Err(e) => self.sink.emit(Event::BodyFailed { key, error: e.to_string() }),
        }
    }
}

/// Why a message didn't come: the account stopped first.
const STOPPED: &str = "the account stopped";

/// How long a message opened waits for the main connection's work to end before a copy of the
/// connection brings it: a short work (an unchanged folder looked at, a small mailbox's
/// calendars) is done sooner than a copy signs in and finds the message.
const GRACE: Duration = Duration::from_millis(400);

/// A copy of an IMAP connection: the account and what it signs in with, as the main connection
/// had them when the copy was taken. It connects when it's first used.
#[derive(Clone)]
pub(super) struct ImapCopy {
    acc: AccountConfig,
    secret: String,
    store: Store,
}

impl ImapCopy {
    fn connect(&self) -> Result<Client<Stream>> {
        match connect_for_sync(&self.acc, &self.store, &self.secret)? {
            Conn::Imap(c) => Ok(c),
            _ => Err(NetError::No("not an IMAP account".into())),
        }
    }

    /// `held`, connected first when it isn't; let go of when the connection is lost.
    fn with<T>(&self, held: &mut Option<Client<Stream>>, f: impl FnOnce(&mut Client<Stream>) -> Result<T>) -> Result<T> {
        let c = match held {
            Some(c) => c,
            None => held.insert(self.connect()?),
        };
        let r = f(c);
        if r.as_ref().is_err_and(NetError::is_fatal) {
            *held = None;
        }
        r
    }

    /// A message's whole source, nothing kept: the caller keeps it.
    pub(super) fn source(&self, held: &mut Option<Client<Stream>>, key: &MessageKey) -> Result<Vec<u8>> {
        self.with(held, |c| crate::sync::fetch_source(c, key))
    }
}

/// A copy of the main connection, taken as it goes to work.
enum Copy {
    /// With each folder's id on the server: the message's id is found by its Message-ID, the
    /// numbering staying with the main connection (550,000 ids, never read again for this).
    Ews(Box<crate::ews::Client>, Arc<BTreeMap<String, String>>),
    Imap(Box<ImapCopy>),
}

impl Copy {
    /// Another copy, for a thread of its own.
    fn again(&self) -> Copy {
        match self {
            Copy::Ews(c, folders) => Copy::Ews(Box::new(c.sibling()), Arc::clone(folders)),
            Copy::Imap(i) => Copy::Imap(i.clone()),
        }
    }
}

/// What the account's threads share about messages opened while its main connection works.
#[derive(Default)]
struct Slot {
    /// How many works the main connection is in (one inside another counts twice)…
    working: usize,
    /// …and the copy of it taken as it went to the first.
    copy: Option<Copy>,
    /// Messages a copy may bring, the last opened first, each with whether its text comes first
    /// and when it was opened. Each is in the main connection's queue too: whichever starts it
    /// first has it ([`Opened::claim`]).
    waiting: VecDeque<(MessageKey, bool, Instant)>,
    /// The one a copy is bringing…
    running: Option<MessageKey>,
    /// …and those the main connection passed over meanwhile: back to it, should the copy fail.
    passed: Vec<MessageKey>,
    /// The thread bringing them is up.
    up: bool,
}

/// Messages opened while the account's main connection is at work (issue #24). Each goes to the
/// main connection's queue as ever, and here too: unless the main connection starts it first, a
/// thread of its own brings it a moment later ([`GRACE`]), the last opened first, on a copy of
/// the main connection: the text of a big one first, then its whole on the thread that brings
/// whole messages ([`Rest`]); a small one whole.
pub(super) struct Opened {
    slot: Mutex<Slot>,
    rest: Arc<Rest>,
    /// The main connection's queue, for a message passed over that a copy couldn't bring; emptied
    /// as the account stops, or neither thread would end.
    main: Mutex<Option<Sender<Job>>>,
    store: Store,
    sink: Sink,
}

/// The main connection at work: while this is held, a message opened goes to a copy of it.
pub(super) struct AtWork(Option<Arc<Opened>>);

impl Drop for AtWork {
    fn drop(&mut self) {
        if let Some(o) = self.0.take() {
            let mut s = o.slot.lock().unwrap_or_else(PoisonError::into_inner);
            s.working = s.working.saturating_sub(1);
            if s.working == 0 {
                s.copy = None;
            }
        }
    }
}

/// Lets go of the queue when the thread unwinds.
struct Unwinding<'a>(&'a Opened);

impl Drop for Unwinding<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.0.let_go();
        }
    }
}

impl Opened {
    pub(super) fn new(rest: Arc<Rest>, main: Sender<Job>, store: Store, sink: Sink) -> Opened {
        Opened { slot: Mutex::default(), rest, main: Mutex::new(Some(main)), store, sink }
    }

    /// The thread bringing whole messages, shared with the main connection.
    pub(super) fn rest(&self) -> Arc<Rest> {
        Arc::clone(&self.rest)
    }

    /// The account stops: nothing goes back to the main connection, no copy is taken.
    pub(super) fn close(&self) {
        *self.main.lock().unwrap_or_else(PoisonError::into_inner) = None;
        self.slot.lock().unwrap_or_else(PoisonError::into_inner).copy = None;
    }

    /// A message opened, offered to a copy of the main connection when that one is at work and a
    /// copy of it may sign in (none was refused this session). The job goes to the main
    /// connection all the same.
    pub(super) fn offer(self: &Arc<Self>, job: &Job) {
        let (key, text) = match job {
            Job::FetchBody { key } => (key, false),
            Job::FetchTextFirst { key } => (key, true),
            _ => return,
        };
        if self.rest.refused() || self.rest.stopped() {
            return;
        }
        let Ok(mut s) = self.slot.lock() else { return };
        if s.working == 0 || s.copy.is_none() || s.running.as_ref() == Some(key) {
            return;
        }
        s.waiting.retain(|(k, ..)| k != key);
        s.waiting.push_front((key.clone(), text, Instant::now()));
        if s.up {
            return;
        }
        // The lock is held until `up` is set: the thread's first look at the queue comes after.
        let me = Arc::clone(self);
        match std::thread::Builder::new().name(format!("opened-{}", key.account)).spawn(move || me.work()) {
            Ok(_) => s.up = true,
            Err(e) => {
                log::info!("{}: no thread for a message opened during a sync: {e}", key.account);
                s.waiting.retain(|(k, ..)| k != key);
            }
        }
    }

    /// The main connection is about to bring this message: `false` when a copy has it already.
    pub(super) fn claim(&self, key: &MessageKey) -> bool {
        let mut s = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
        if s.running.as_ref() == Some(key) {
            s.passed.push(key.clone());
            return false;
        }
        s.waiting.retain(|(k, ..)| k != key);
        true
    }

    /// The next message, once it has waited [`GRACE`] and the main connection hasn't taken it,
    /// with a copy of the connection if the main one is still at work. With none, the same lock
    /// says the thread is gone.
    fn next(&self) -> Option<((MessageKey, bool), Option<Copy>)> {
        loop {
            let wait = {
                let mut s = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
                let Some(&(_, _, opened)) = s.waiting.front() else {
                    s.up = false;
                    return None;
                };
                let left = (opened + GRACE).saturating_duration_since(Instant::now());
                if left.is_zero() || self.rest.stopped() {
                    let (key, text, _) = s.waiting.pop_front()?;
                    s.running = Some(key.clone());
                    let copy = s.copy.as_ref().map(Copy::again);
                    return Some(((key, text), copy));
                }
                left
            };
            std::thread::sleep(wait.min(Duration::from_millis(20)));
        }
    }

    /// Done with the message running: whether the main connection passed it over meanwhile.
    fn done(&self, key: &MessageKey) -> bool {
        let mut s = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
        s.running = None;
        let passed = s.passed.iter().any(|k| k == key);
        s.passed.retain(|k| k != key);
        passed
    }

    /// Back to the main connection, which brings it after its work (`text`: as its text first).
    fn back(&self, key: MessageKey, text: bool) {
        let job = if text { Job::FetchTextFirst { key: key.clone() } } else { Job::FetchBody { key: key.clone() } };
        let main = self.main.lock().unwrap_or_else(PoisonError::into_inner).clone();
        if main.is_none_or(|m| m.send(job).is_err()) {
            self.sink.emit(Event::BodyFailed { key, error: STOPPED.into() });
        }
    }

    /// The thread ends with messages left: those waiting are in the main connection's queue, and
    /// the one running goes back there if it was passed over.
    fn let_go(&self) {
        let running = {
            let mut s = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
            s.up = false;
            s.waiting.clear();
            s.running.clone()
        };
        if let Some(key) = running
            && self.done(&key)
        {
            self.back(key, false);
        }
    }

    /// The thread: each message waiting, the last opened first, on one copy of the connection.
    fn work(&self) {
        let _unwinding = Unwinding(self);
        let (mut copy, mut imap) = (None::<Copy>, None);
        while let Some(((key, text), fresh)) = self.next() {
            if copy.is_none() {
                copy = fresh;
            }
            let brought = match &copy {
                _ if self.rest.stopped() => Ok(Brought::Stopped),
                Some(c) => self.bring(&key, text, c, &mut imap),
                None => Ok(Brought::Back(text)),
            };
            let passed = self.done(&key);
            match brought {
                Ok(Brought::Told) => {}
                Ok(Brought::Stopped) => self.sink.emit(Event::BodyFailed { key, error: STOPPED.into() }),
                // Not brought: the main connection has it in its queue, unless it passed it over.
                Ok(Brought::Back(text)) => {
                    if passed {
                        self.back(key, text);
                    }
                }
                // A copy refused its sign-in, as a password changed in Active Directory does: no
                // other is tried this session (each would cost a refused sign-in, and a few lock
                // the account). The main connection, signed in before, brings them all.
                Err((text, e @ NetError::Auth(_))) => {
                    self.rest.refuse(&key, &e.to_string());
                    if passed {
                        self.back(key, text);
                    }
                    self.let_go();
                    return;
                }
                Err((text, e)) => {
                    log::info!("{}: the copy of the connection couldn't bring it ({e}): the main connection will", named(&key));
                    if passed {
                        self.back(key, text);
                    }
                }
            }
        }
    }

    /// Bring one message on `copy`: told, or left to the main connection. An error says whether
    /// the main connection is to bring its text first still.
    fn bring(&self, key: &MessageKey, text: bool, copy: &Copy, imap: &mut Option<Client<Stream>>) -> std::result::Result<Brought, (bool, NetError)> {
        if let Some(raw) = self.store.read_body(key) {
            self.sink.emit(Event::Body { key: key.clone(), raw });
            return Ok(Brought::Told);
        }
        let began = Instant::now();
        let shown = |raw: Vec<u8>| {
            log::info!(
                "{}: the message's text came first, on a copy of the connection while the main one works: {} bytes in {} ms",
                named(key),
                raw.len(),
                began.elapsed().as_millis()
            );
            self.sink.emit(Event::TextFirst { key: key.clone(), raw });
        };
        let raw = match copy {
            Copy::Ews(c, folders) => {
                let Some(id) = crate::ews::find_by_message_id(c, folders, &self.store, key).map_err(|e| (text, e))? else {
                    log::info!("{}: not found by its Message-ID: it waits for the main connection", named(key));
                    return Ok(Brought::Back(text));
                };
                if text && !self.rest.text_refused() {
                    match c.download_text(&id) {
                        Ok(Some(raw)) => {
                            shown(raw);
                            if self.rest.add(key.clone(), id.clone(), Whole::Ews(Box::new(c.sibling()))) {
                                return Ok(Brought::Told);
                            }
                        }
                        Ok(None) => {}
                        Err(NetError::No(e)) if crate::ews_pim::FIELD_REFUSED.iter().any(|r| e.contains(r)) => {
                            log::warn!("{}: the server refuses a message's text alone ({e}): messages come whole from now on", key.account);
                            self.rest.refuse_text();
                        }
                        Err(e @ NetError::Auth(_)) => return Err((text, e)),
                        Err(e) => log::info!("{}: the message's text didn't come ({e}): it comes whole", named(key)),
                    }
                }
                c.download(&id).map_err(|e| (false, e))?
            }
            Copy::Imap(i) => {
                if text {
                    match i.with(imap, |c| crate::sync::fetch_text(c, key)) {
                        Ok(Some(raw)) => {
                            shown(raw);
                            if self.rest.add(key.clone(), String::new(), Whole::Imap(i.clone())) {
                                return Ok(Brought::Told);
                            }
                        }
                        Ok(None) => {}
                        Err(e) if e.is_fatal() || matches!(e, NetError::Auth(_)) => return Err((text, e)),
                        Err(e) => log::info!("{}: the message's text didn't come ({e}): it comes whole", named(key)),
                    }
                }
                i.source(imap, key).map_err(|e| (false, e))?
            }
        };
        // Stopped meanwhile (quitting, working offline, the account removed): nothing written.
        if self.rest.stopped() {
            return Ok(Brought::Stopped);
        }
        self.store.write_body(key, &raw).map_err(|e| (false, e.into()))?;
        // The summary isn't filled here (the main connection may be saving the folder's list):
        // [`Worker::fetch`] does it when the message comes to it, the work done.
        self.rest.owe(key);
        log::info!(
            "{}: downloaded on a copy of the connection while the main one works: {} bytes in {} ms",
            named(key),
            raw.len(),
            began.elapsed().as_millis()
        );
        self.sink.emit(Event::Body { key: key.clone(), raw });
        Ok(Brought::Told)
    }
}

/// What became of a message a copy was given.
enum Brought {
    Told,
    /// The account stopped first.
    Stopped,
    /// The main connection brings it (`true`: as its text first).
    Back(bool),
}

impl Worker {
    /// This connection is about to bring a message: `false` when a copy of it has it already.
    pub(super) fn claim(&self, key: &MessageKey) -> bool {
        self.opened.as_ref().is_none_or(|o| o.claim(key))
    }

    /// The main connection goes to work (a folder's sync, the calendars'): until the guard is
    /// dropped, a message opened meanwhile goes to a copy of it, taken now.
    pub(super) fn at_work(&self) -> AtWork {
        let Some(o) = self.opened.clone() else { return AtWork(None) };
        {
            let mut s = o.slot.lock().unwrap_or_else(PoisonError::into_inner);
            s.working = s.working.saturating_add(1);
            if s.copy.is_none() {
                s.copy = self.copy();
            }
        }
        AtWork(Some(o))
    }

    /// A copy of this connection, where one can open a message without its numbering: Exchange
    /// (the message found by its Message-ID) and IMAP (by its UID). An OAuth account's copy signs
    /// in with the current token: none, none is taken.
    fn copy(&self) -> Option<Copy> {
        match &self.conn {
            Some(Conn::Ews(c)) => Some(Copy::Ews(Box::new(c.sibling()), Arc::new(self.jstate.mailboxes.clone()))),
            Some(Conn::Imap(_)) => {
                let auth = self.auth.lock().unwrap_or_else(PoisonError::into_inner);
                let secret = match self.acc.oauth {
                    None => auth.secret.clone(),
                    Some(_) => auth.access.as_ref().filter(|t| !t.expired()).map(|t| t.access.clone())?,
                };
                Some(Copy::Imap(Box::new(ImapCopy { acc: self.acc.clone(), secret, store: self.store.clone() })))
            }
            _ => None,
        }
    }

    /// A copy of this IMAP connection, for the thread bringing whole messages.
    pub(super) fn imap_copy(&self) -> Option<ImapCopy> {
        match self.copy() {
            Some(Copy::Imap(i)) => Some(*i),
            _ => None,
        }
    }
}
