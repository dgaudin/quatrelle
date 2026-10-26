//! An in-memory IMAP server for tests: enough of RFC 3501 + UIDPLUS, MOVE and CONDSTORE to run
//! the real client and sync code against, with state the test can change between syncs.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};

use quatrelle_store::Store;

use crate::imap::Client;
use crate::sync::{SyncOptions, sync_folder};

pub struct Msg {
    pub uid: u32,
    pub flags: Vec<String>,
    pub raw: Vec<u8>,
    pub modseq: u64,
    /// When the server received it (INTERNALDATE, Unix seconds), what `SEARCH SINCE` compares.
    pub received: i64,
}

/// What the fake says it received every message at unless told: 14 October 2026, 09:30 UTC.
pub const RECEIVED: i64 = 1_791_970_200;

/// A time as an INTERNALDATE (`14-Oct-2026 09:30:00 +0000`, the day padded to two).
pub fn internal_date(t: i64) -> String {
    let secs = t.rem_euclid(86_400);
    format!("{:>11} {:02}:{:02}:{:02} +0000", crate::sync::imap_date(t), secs / 3600, secs % 3600 / 60, secs % 60)
}

pub struct Mailbox {
    pub name: String,
    pub attr: &'static str,
    pub validity: u32,
    pub uid_next: u32,
    pub msgs: Vec<Msg>,
    /// Expunged UIDs and the modseq of their removal (for QRESYNC's VANISHED).
    pub expunged: Vec<(u32, u64)>,
}

pub struct State {
    pub boxes: Vec<Mailbox>,
    pub modseq: u64,
    pub caps: &'static str,
    /// Every command received (tags stripped).
    pub log: Vec<String>,
    /// Addresses whose good token is taken but whose mailbox then won't open, as Microsoft
    /// answers when IMAP is off or the token is another account's.
    pub not_connected: Vec<String>,
    /// What every password sign-in is refused with, when set (Outlook.com: "Basic authentication
    /// is disabled").
    pub refuse_passwords: Option<String>,
    /// How long a text `SEARCH` takes (a server reading every message rather than an index).
    pub search_delay: std::time::Duration,
    /// Connections open now, and the most there were at once (what a sync on several
    /// connections shows).
    pub open: usize,
    pub peak: usize,
    /// A hostile server: after the next `NOOP`, this is sent over and over, forever.
    pub flood: Option<Vec<u8>>,
    /// How long each read of the connection waits (a server trickling its answer).
    pub trickle: std::time::Duration,
    /// Mail delivered to the Inbox (this subject) when a command containing this text first
    /// comes: mail arriving at a set point of a sync.
    pub deliver_on: Option<(String, String)>,
}

impl State {
    pub fn new() -> Arc<Mutex<State>> {
        Arc::new(Mutex::new(State {
            boxes: vec![
                Mailbox { name: "INBOX".into(), attr: "", validity: 100, uid_next: 1, msgs: Vec::new(), expunged: Vec::new() },
                Mailbox { name: "Sent".into(), attr: "\\Sent", validity: 200, uid_next: 1, msgs: Vec::new(), expunged: Vec::new() },
                Mailbox { name: "Archive".into(), attr: "\\Archive", validity: 300, uid_next: 1, msgs: Vec::new(), expunged: Vec::new() },
                Mailbox { name: "Projects/Caf&AOk-".into(), attr: "", validity: 400, uid_next: 1, msgs: Vec::new(), expunged: Vec::new() },
            ],
            modseq: 10,
            caps: "IMAP4rev1 UIDPLUS MOVE CONDSTORE ENABLE QUOTA",
            log: Vec::new(),
            not_connected: Vec::new(),
            refuse_passwords: None,
            search_delay: std::time::Duration::ZERO,
            open: 0,
            peak: 0,
            flood: None,
            trickle: std::time::Duration::ZERO,
            deliver_on: None,
        }))
    }

    pub fn mailbox(&mut self, name: &str) -> Option<&mut Mailbox> {
        self.boxes.iter_mut().find(|b| b.name == name)
    }

    /// Remove a message the way another client would (recorded for VANISHED).
    pub fn expunge(&mut self, mailbox: &str, uid: u32) {
        self.modseq += 1;
        let ms = self.modseq;
        if let Some(b) = self.mailbox(mailbox) {
            b.msgs.retain(|m| m.uid != uid);
            b.expunged.push((uid, ms));
        }
    }

    /// Set a message's flags the way another client would.
    pub fn set_flags(&mut self, mailbox: &str, uid: u32, flags: &[&str]) {
        self.modseq += 1;
        let ms = self.modseq;
        if let Some(m) = self.mailbox(mailbox).and_then(|b| b.msgs.iter_mut().find(|m| m.uid == uid)) {
            m.flags = flags.iter().map(|s| s.to_string()).collect();
            m.modseq = ms;
        }
    }

    /// Put a whole message (as sent) in a mailbox.
    pub fn add_raw(&mut self, mailbox: &str, raw: Vec<u8>) -> u32 {
        self.modseq += 1;
        let modseq = self.modseq;
        let Some(b) = self.mailbox(mailbox) else { return 0 };
        let uid = b.uid_next;
        b.uid_next += 1;
        b.msgs.push(Msg { uid, flags: Vec::new(), raw, modseq, received: RECEIVED });
        uid
    }

    /// Put a message dated `date` (its `Date` header, Unix seconds) that the server received at
    /// `received`.
    pub fn add_dated(&mut self, mailbox: &str, subject: &str, date: i64, received: i64) -> u32 {
        let raw = format!(
            "From: Zoe <zoe@studio.example>\r\nTo: me@studio.example\r\nSubject: {subject}\r\nDate: {}\r\n\r\nBody of {subject}.\r\n",
            internal_date(date).trim().replace('-', " ")
        );
        let uid = self.add_raw(mailbox, raw.into_bytes());
        if let Some(msg) = self.mailbox(mailbox).and_then(|b| b.msgs.iter_mut().find(|x| x.uid == uid)) {
            msg.received = received;
        }
        uid
    }

    pub fn add(&mut self, mailbox: &str, subject: &str, flags: &[&str]) -> u32 {
        self.modseq += 1;
        let modseq = self.modseq;
        let Some(b) = self.mailbox(mailbox) else { return 0 };
        let uid = b.uid_next;
        b.uid_next += 1;
        let raw = format!(
            "From: Zoe <zoe@studio.example>\r\nTo: me@studio.example\r\nSubject: {subject}\r\nMessage-ID: <{subject}-{uid}@studio.example>\r\nDate: Wed, 14 Oct 2026 09:{:02}:00 +0000\r\n\r\nBody of {subject}.\r\n",
            uid % 60
        );
        b.msgs.push(Msg { uid, flags: flags.iter().map(|s| s.to_string()).collect(), raw: raw.into_bytes(), modseq, received: RECEIVED });
        uid
    }
}

pub struct Fake {
    st: Arc<Mutex<State>>,
    selected: Option<String>,
    inbuf: Vec<u8>,
    out: VecDeque<u8>,
    literal: Option<(String, usize)>,
    /// SASL in progress: the tag, and whether the next line is the token (`true`) or the empty
    /// answer to an error challenge (`false`).
    sasl: Option<(String, bool)>,
    /// Signed in, but every command is refused (see `State::not_connected`).
    refused: bool,
    /// Sent forever once the output runs dry (see `State::flood`).
    flood: Option<Vec<u8>>,
    /// In IDLE (this tag) until `DONE`.
    idling: Option<String>,
}

/// The only access token the fake accepts.
pub const GOOD_TOKEN: &str = "good-token";

impl Fake {
    pub fn connect(st: &Arc<Mutex<State>>) -> Fake {
        let mut f = Fake {
            st: st.clone(),
            selected: None,
            inbuf: Vec::new(),
            out: VecDeque::new(),
            literal: None,
            sasl: None,
            refused: false,
            flood: None,
            idling: None,
        };
        f.say("* OK fake ready");
        f
    }

    fn say(&mut self, s: &str) {
        self.out.extend(s.as_bytes());
        self.out.extend(b"\r\n");
    }

    fn pump(&mut self) {
        loop {
            if let Some((line, n)) = self.literal.clone() {
                // literal bytes, then the rest of the command line
                if self.inbuf.len() < n {
                    return;
                }
                let rest = &self.inbuf[n..];
                let Some(end) = rest.windows(2).position(|w| w == b"\r\n") else { return };
                let data = self.inbuf[..n].to_vec();
                self.inbuf.drain(..n + end + 2);
                self.literal = None;
                self.command(&line, Some(data));
                continue;
            }
            let Some(end) = self.inbuf.windows(2).position(|w| w == b"\r\n") else { return };
            let line = String::from_utf8_lossy(&self.inbuf[..end]).to_string();
            self.inbuf.drain(..end + 2);
            if let Some(open) = line.rfind('{')
                && line.ends_with('}')
                && let Ok(n) = line[open + 1..line.len() - 1].trim_end_matches('+').parse::<usize>()
            {
                self.literal = Some((line[..open].to_string(), n));
                self.say("+ go ahead");
                continue;
            }
            if let Some(tag) = self.idling.take() {
                if line.trim().eq_ignore_ascii_case("DONE") {
                    self.say(&format!("{tag} OK idle done"));
                } else {
                    self.idling = Some(tag);
                }
                continue;
            }
            if let Some((tag, token_next)) = self.sasl.take() {
                self.sasl_step(&tag, token_next.then_some(line.as_str()));
                continue;
            }
            self.command(&line, None);
        }
    }

    /// XOAUTH2: accept the good token; anything else gets an error challenge, then NO.
    fn sasl_step(&mut self, tag: &str, token: Option<&str>) {
        match token {
            Some(t) if t.trim() == crate::oauth::xoauth2("me@studio.example", GOOD_TOKEN) => {
                let caps = self.st.lock().map(|s| s.caps).unwrap_or("");
                self.say(&format!("{tag} OK [CAPABILITY {caps}] signed in"));
            }
            Some(t) if self.st.lock().map(|s| s.not_connected.iter().any(|u| t.trim() == crate::oauth::xoauth2(u, GOOD_TOKEN))).unwrap_or(false) => {
                self.refused = true;
                self.say(&format!("{tag} OK AUTHENTICATE completed."));
            }
            Some(_) => {
                self.say("+ eyJzdGF0dXMiOiI0MDEiLCJzY2hlbWVzIjoiYmVhcmVyIn0=");
                self.sasl = Some((tag.to_string(), false));
            }
            None => self.say(&format!("{tag} NO [AUTHENTICATIONFAILED] invalid credentials")),
        }
    }
}

impl Read for Fake {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let trickle = self.st.lock().map(|s| s.trickle).unwrap_or_default();
        if !trickle.is_zero() {
            std::thread::sleep(trickle);
        }
        if self.out.is_empty()
            && let Some(f) = &self.flood
        {
            self.out.extend(f.iter());
        }
        let n = buf.len().min(self.out.len()).min(11);
        for b in buf.iter_mut().take(n) {
            *b = self.out.pop_front().unwrap_or(0);
        }
        Ok(n)
    }
}

impl Write for Fake {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inbuf.extend_from_slice(buf);
        self.pump();
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Serve the fake over TCP on localhost (one thread per connection), for end-to-end tests of
/// the worker pool. Returns the port.
pub fn serve_tcp(st: &Arc<Mutex<State>>) -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let st = st.clone();
    std::thread::spawn(move || {
        for sock in listener.incoming().flatten() {
            let st = st.clone();
            std::thread::spawn(move || {
                let mut fake = Fake::connect(&st);
                let mut sock = sock;
                let mut buf = [0u8; 8192];
                // Counted so a test can see that a sync really used several connections.
                if let Ok(mut s) = st.lock() {
                    s.open += 1;
                    s.peak = s.peak.max(s.open);
                }
                struct Open(Arc<Mutex<State>>);
                impl Drop for Open {
                    fn drop(&mut self) {
                        if let Ok(mut s) = self.0.lock() {
                            s.open = s.open.saturating_sub(1);
                        }
                    }
                }
                let _open = Open(st.clone());
                loop {
                    let out: Vec<u8> = fake.out.drain(..).collect();
                    if !out.is_empty() && sock.write_all(&out).is_err() {
                        return;
                    }
                    match sock.read(&mut buf) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => {
                            let _ = fake.write(&buf[..n]);
                        }
                    }
                }
            });
        }
    });
    port
}

/// A store in a temporary folder of its own, for other fakes' tests.
pub(crate) fn tests_store(name: &str) -> Store {
    store(name)
}

pub(crate) fn store(name: &str) -> Store {
    let dir = std::env::temp_dir().join(format!("quatrelle-fake-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    Store::open(dir).unwrap()
}

/// Gap #7 through the worker, the same for JMAP, Graph and EWS accounts: a server whose Inbox
/// has `Message 1`…`Message 25` (bodies `tag1`…, the first five unread) and a window of ten. A
/// search of the server brings in what it finds past the window and keeps it across syncs while
/// the server has it, its flags followed; words, fields and the read state are asked; Show more
/// keeps more. `server(n, true)` marks `Message n` read on the server, `server(n, false)` deletes it.
pub(crate) fn search_the_server_past_the_window(name: &str, account: quatrelle_model::AccountConfig, secret: &str, server: &dyn Fn(u32, bool)) {
    use quatrelle_model::job::Secret;
    use quatrelle_model::{Event, Job, SyncStatus};
    let store = store(name);
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let until = |pool: &crate::worker::Pool, what: &str, f: &dyn Fn(&Event) -> bool| {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut seen = Vec::new();
        while std::time::Instant::now() < end {
            if let Some(e) = pool.wait(std::time::Duration::from_millis(100)) {
                let hit = f(&e);
                seen.push(e);
                if hit {
                    return seen;
                }
            }
        }
        panic!("timed out waiting for {what}; saw {seen:?}");
    };
    let online = |e: &Event| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. });
    let subjects = || {
        let mut v: Vec<String> = store.load_folder("a1", "INBOX").unwrap().messages.iter().map(|m| m.subject.clone()).collect();
        v.sort();
        v
    };
    let has = |s: &str| subjects().iter().any(|x| x == s);
    let search = |pool: &mut crate::worker::Pool, q: &str| {
        pool.submit(Job::SearchServer { account: "a1".into(), folders: vec!["INBOX".into()], query: q.into(), search: 1 });
        // What's found comes as it's found, then the end.
        let seen = until(pool, "searched", &|e| matches!(e, Event::ServerSearched { done: true, .. }));
        match seen.last() {
            Some(Event::ServerSearched { error: None, .. }) => {
                seen.iter().map(|e| if let Event::ServerSearched { keys, .. } = e { keys.len() } else { 0 }).sum::<usize>()
            }
            other => panic!("{q}: {other:?}"),
        }
    };
    pool.submit(Job::SetSync { days: 0, messages: 10, full: false });
    pool.submit(Job::Start { account, password: Some(Secret(secret.into())) });
    until(&pool, "online", &online);
    let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while subjects().len() < 10 {
        assert!(std::time::Instant::now() < end, "never synced: {:?}", subjects());
        let _ = pool.wait(std::time::Duration::from_millis(100));
    }
    assert!(has("Message 16") && !has("Message 15") && !has("Message 3"), "{:?}", subjects());

    // Words: found past the window, and kept by the next sync.
    assert_eq!(search(&mut pool, "tag3"), 1);
    assert!(has("Message 3"), "{:?}", subjects());
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() });
    until(&pool, "synced", &online);
    assert!(has("Message 3"), "kept past the window: {:?}", subjects());
    // A field and a word.
    assert_eq!(search(&mut pool, "subject:message tag7"), 1);
    assert!(has("Message 7"));

    // Show more: five more than kept (ten and the one found).
    pool.submit(Job::MoreMail { account: "a1".into(), folder: "INBOX".into(), more: 5 });
    let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !has("Message 10") {
        assert!(std::time::Instant::now() < end, "never showed more: {:?}", subjects());
        let _ = pool.wait(std::time::Duration::from_millis(100));
    }

    // The read state alone; finds add up, as IMAP keeps what it fetches (another search doesn't
    // take Message 3 and 7 away).
    assert_eq!(search(&mut pool, "is:unread"), 5);
    assert_eq!(search(&mut pool, "nothing-like-this"), 0);
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() });
    until(&pool, "synced", &online);
    assert!(has("Message 1") && has("Message 5") && has("Message 3") && has("Message 7"), "{:?}", subjects());

    // On the server, Message 3 is deleted and Message 2 read: the next sync follows both.
    server(3, false);
    server(2, true);
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() });
    // (A push stream may say "online" before this sync ends: wait for what it does.)
    let seen = |s: &str| store.load_folder("a1", "INBOX").unwrap().messages.iter().find(|m| m.subject == s).map(|m| m.flags.seen);
    let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while has("Message 3") || seen("Message 2") != Some(true) {
        assert!(std::time::Instant::now() < end, "never followed the server: {:?} {:?}", subjects(), seen("Message 2"));
        let _ = pool.wait(std::time::Duration::from_millis(100));
    }
    assert!(has("Message 7"), "{:?}", subjects());
    assert_eq!(seen("Message 1"), Some(false));
    pool.submit(Job::Remove { account: "a1".into() });
}

fn login(st: &Arc<Mutex<State>>) -> Client<Fake> {
    let mut c = Client::from_stream(Fake::connect(st)).unwrap();
    c.login("me@studio.example", "pw").unwrap();
    c
}

mod commands;
mod search;

use search::{in_set, search_matches, search_tokens};

#[cfg(test)]
mod tests;
