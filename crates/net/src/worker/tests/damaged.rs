// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What a worker does with a file of the account's that is there but damaged (its bytes aren't
//! what it keeps): a change waiting, never passed over; never read as none.

use super::read_errors::{imap_account, worker};
use super::*;

/// The fake server's message `uid` of `mailbox`, with its flags; `None` when it isn't there.
fn on_server(st: &Arc<Mutex<crate::fake::State>>, mailbox: &str, uid: u32) -> Option<Vec<String>> {
    let st = st.lock().unwrap();
    st.boxes.iter().find(|b| b.name == mailbox)?.msgs.iter().find(|m| m.uid == uid).map(|m| m.flags.clone())
}

/// The commands the fake server was sent that start with `verb` (after `UID`).
fn sent(st: &Arc<Mutex<crate::fake::State>>, verb: &str) -> usize {
    let st = st.lock().unwrap();
    st.log.iter().filter(|l| l.to_ascii_uppercase().trim_start_matches("UID ").starts_with(verb)).count()
}

/// Found in review: a change waiting whose file was damaged was passed over, and the ones made
/// after it were sent as if it had never been made (here a message moved, after a change to it
/// that's lost); nothing said so. It and every change made after it are kept aside, as they are,
/// and said; the one before it goes, once.
#[test]
fn a_damaged_change_waiting_is_never_skipped_over() {
    let st = crate::fake::State::new();
    let (a, b) = {
        let mut s = st.lock().unwrap();
        (s.add("INBOX", "Plans", &[]), s.add("INBOX", "Budget", &[]))
    };
    let port = crate::fake::serve_tcp(&st);
    let store = Store::failing();
    let key = |uid| MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 100, uid };
    store.add_op("a1", Op::SetFlag { keys: vec![key(a)], flag: Flag::Seen, on: true }).unwrap();
    store.add_op("a1", Op::SetFlag { keys: vec![key(b)], flag: Flag::Flagged, on: true }).unwrap();
    store.add_op("a1", Op::Move { keys: vec![key(b)], to: "Archive".into() }).unwrap();
    let mut names = store.account_files("a1", "ops").unwrap();
    names.sort();
    assert_eq!(names.len(), 3);
    let damaged = b"{\"op\":\"setFlag\",\"keys\":[{\"acc".to_vec();
    store.write_account_files("a1", &[(format!("ops/{}", names[1]), damaged.clone())]).unwrap();
    let third = store.account_file("a1", &format!("ops/{}", names[2])).unwrap().unwrap();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let mut w = worker(imap_account(port), "pw", &store, &pool);
    w.connect().unwrap();
    pool.poll();
    w.replay();
    assert_eq!(on_server(&st, "INBOX", b), Some(Vec::new()), "moved on the server past a change that couldn't be read");
    assert_eq!(sent(&st, "MOVE"), 0, "a change after it was sent");
    assert!(on_server(&st, "INBOX", a).is_some_and(|f| f.iter().any(|f| f == "\\Seen")), "the one before it wasn't sent");
    let told: Vec<usize> =
        pool.poll().into_iter().filter_map(|e| if let Event::OpsSetAside { count, .. } = e { Some(count) } else { None }).collect();
    assert_eq!(told, [2], "not said");
    let mut aside = store.account_files("a1", "ops-aside").unwrap();
    aside.sort();
    assert_eq!(aside, names[1..], "not kept aside");
    assert_eq!(store.account_file("a1", &format!("ops-aside/{}", names[1])).unwrap(), Some(damaged), "changed");
    assert_eq!(store.account_file("a1", &format!("ops-aside/{}", names[2])).unwrap(), Some(third), "changed");
    assert!(store.load_ops("a1").unwrap().is_empty());
    // Once: nothing more is sent nor set aside at the next replay.
    let stores = sent(&st, "STORE");
    w.replay();
    assert_eq!(sent(&st, "STORE"), stores, "sent twice");
    assert!(!pool.poll().iter().any(|e| matches!(e, Event::OpsSetAside { .. })), "said twice");
    assert_eq!(store.account_files("a1", "ops-aside/2").unwrap_or_default(), Vec::<String>::new());
}
