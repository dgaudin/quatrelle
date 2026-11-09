// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A change the server refuses for now (Gmail's "System Error", RFC 5530's `UNAVAILABLE`) is tried
//! again, three times at most; a final refusal drops it at once.

use super::*;

const TRASH: &str = "[Gmail]/Trash";

fn count(st: &Arc<Mutex<State>>, name: &str) -> usize {
    st.lock().unwrap().mailbox(name).map_or(0, |b| b.msgs.len())
}

fn moves(st: &Arc<Mutex<State>>) -> usize {
    st.lock().unwrap().log.iter().filter(|l| l.to_ascii_uppercase().starts_with("UID MOVE")).count()
}

/// A Gmail-like account online with `mail` messages in its Inbox.
fn online(st: &Arc<Mutex<State>>, name: &str, mail: usize) -> (crate::worker::Pool, Store) {
    {
        let mut s = st.lock().unwrap();
        s.gmail();
        for n in 0..mail {
            s.add("INBOX", &format!("mail {n}"), &[]);
        }
    }
    let port = serve_tcp(st);
    let store = store(name);
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    pool.opts.prefetch = 0;
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port, security: Security::None },
        smtp: ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None },
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    (pool, store)
}

fn inbox_keys(store: &Store) -> Vec<MessageKey> {
    let mut v: Vec<MessageKey> = store.load_folder("a1", "INBOX").unwrap().messages.into_iter().map(|m| m.key).collect();
    v.sort_by_key(|k| k.uid);
    v
}

/// The change handed to the pool, which journals it and makes it in the cache.
fn make(pool: &mut crate::worker::Pool, op: Op) {
    pool.submit(Job::Apply { account: "a1".into(), op });
}

/// Events seen until `done` holds (the journal empty and the server's state as wanted).
fn settle(pool: &crate::worker::Pool, what: &str, mut done: impl FnMut() -> bool) -> Vec<Event> {
    let end = Instant::now() + Duration::from_secs(20);
    let mut seen = Vec::new();
    while !done() {
        assert!(Instant::now() < end, "timed out waiting for {what}; saw {seen:?}");
        if let Some(e) = pool.wait(Duration::from_millis(50)) {
            seen.push(e);
        }
    }
    // A moment more, for an event still on its way.
    let quiet = Instant::now() + Duration::from_millis(300);
    while Instant::now() < quiet {
        if let Some(e) = pool.wait(Duration::from_millis(50)) {
            seen.push(e);
        }
    }
    seen
}

fn failed(seen: &[Event]) -> usize {
    seen.iter().filter(|e| matches!(e, Event::OpFailed { .. })).count()
}

/// Gmail answers a delete once `NO System Error (Failure)`: the message stays deleted, nothing is
/// said, and the server has it in the Trash at the next pass.
#[test]
fn a_system_error_once_leaves_the_delete_waiting_then_done() {
    for (name, answer) in [("system-error", "System Error (Failure)"), ("unavailable", "[UNAVAILABLE] Temporary failure, try again")] {
        let st = State::new();
        let (mut pool, store) = online(&st, &format!("refusal-{name}"), 2);
        st.lock().unwrap().refuse_command("UID MOVE", answer, 1);
        let first = inbox_keys(&store).remove(0);
        make(&mut pool, Op::Move { keys: vec![first], to: TRASH.into() });
        let seen = settle(&pool, "the message in the Trash", || count(&st, TRASH) == 1 && store.load_ops("a1").unwrap().is_empty());
        assert_eq!(failed(&seen), 0, "{name}: no OpFailed: {seen:?}");
        assert_eq!((count(&st, "INBOX"), moves(&st)), (1, 2), "{name}: refused once, then done");
        assert_eq!(inbox_keys(&store).len(), 1, "{name}: the message stays deleted here");
        pool.submit(Job::Remove { account: "a1".into() });
    }
}

/// Three passing refusals in a row: the change is dropped and said, as before, and the change
/// after it goes through.
#[test]
fn three_passing_refusals_drop_the_change_and_the_next_one_goes_through() {
    let st = State::new();
    let (mut pool, store) = online(&st, "refusal-three", 2);
    st.lock().unwrap().refuse_command("UID MOVE", "System Error (Failure)", 3);
    let keys = inbox_keys(&store);
    make(&mut pool, Op::Move { keys: vec![keys[0].clone()], to: TRASH.into() });
    make(&mut pool, Op::SetFlag { keys: vec![keys[1].clone()], flag: Flag::Seen, on: true });
    let seen = settle(&pool, "the flag set", || {
        store.load_ops("a1").unwrap().is_empty()
            && st.lock().unwrap().mailbox("INBOX").is_some_and(|m| m.msgs.iter().any(|m| m.flags.iter().any(|f| f == "\\Seen")))
    });
    assert_eq!(failed(&seen), 1, "one OpFailed: {seen:?}");
    assert_eq!(moves(&st), 3, "three tries, no more");
    assert_eq!((count(&st, "INBOX"), count(&st, TRASH)), (2, 0), "the server kept the message");
    pool.submit(Job::Remove { account: "a1".into() });
}

/// A final refusal is dropped at once, not tried again (a move is, once, after the folders are
/// listed anew: that is older and stays).
#[test]
fn a_refusal_with_no_perm_is_dropped_at_once() {
    let st = State::new();
    let (mut pool, store) = online(&st, "refusal-noperm", 1);
    st.lock().unwrap().refuse_command("UID STORE", "[NOPERM] you may not", 1);
    let first = inbox_keys(&store).remove(0);
    make(&mut pool, Op::SetFlag { keys: vec![first], flag: Flag::Seen, on: true });
    let stores = |st: &Arc<Mutex<State>>| st.lock().unwrap().log.iter().filter(|l| l.to_ascii_uppercase().starts_with("UID STORE")).count();
    let seen = settle(&pool, "the change dropped", || store.load_ops("a1").unwrap().is_empty() && stores(&st) >= 1);
    assert_eq!(failed(&seen), 1, "{seen:?}");
    assert_eq!(stores(&st), 1);
    pool.submit(Job::Remove { account: "a1".into() });
}
