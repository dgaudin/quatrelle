// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: the folders' kept indexes taken back on the window's search lane after a
//! restart, not on its thread, and the search answering as it does without the lane.

use std::collections::BTreeSet;

use quatrelle_model::{AccountConfig, Folder};
use quatrelle_store::FolderState;

use super::*;

/// Six folders of 800 messages on an IMAP account; every fiftieth says "needle".
fn store_of(name: &str) -> Store {
    let store = temp(name);
    let mut config = AccountConfig { id: "a1".into(), email: "me@studio.example".into(), ..Default::default() };
    config.imap.host = "imap.studio.example".into();
    store.save_accounts(&[config]).unwrap();
    let mut list = Vec::new();
    for i in 0..6u32 {
        let path = if i == 0 { "INBOX".to_string() } else { format!("Folder{i}") };
        let role = if i == 0 { FolderRole::Inbox } else { FolderRole::Other };
        list.push(Folder { account: "a1".into(), path: path.clone(), role, selectable: true, total: 800, ..Default::default() });
        let messages = (1..=800u32)
            .rev()
            .map(|uid| Summary {
                key: MessageKey { account: "a1".into(), folder: path.clone(), validity: 1, uid },
                subject: if uid % 50 == 0 { format!("needle {path} {uid}") } else { format!("Report {path} {uid}") },
                date: NOW - 86_400 * 30 + i64::from(i * 800 + uid) * 60,
                ..Default::default()
            })
            .collect();
        store.save_folder("a1", &path, &FolderState { validity: 1, messages, ..Default::default() }).unwrap();
    }
    store.save_folders("a1", &list).unwrap();
    store
}

fn open(store: &Store, lane: bool) -> Session {
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    s.take_jobs();
    s.defer_index_reading(lane);
    s
}

/// A search as the window carries it, the lane's work run between calls (`lane`), or never;
/// the calls it took.
fn search(s: &mut Session, q: &str, lane: bool) -> usize {
    for call in 1..=2_000 {
        let v = s.run("search.run", &json!({"query": q, "scope": "mailbox", "index": 500})).unwrap();
        if v["pending"] == false {
            return call;
        }
        if let Some(work) = s.take_index_work()
            && lane
        {
            work.run();
        }
    }
    panic!("{q}: not finished");
}

fn found(s: &Session) -> BTreeSet<MessageKey> {
    s.list.iter().map(|m| m.key.clone()).collect()
}

/// After a restart, the kept indexes come back from the lane: the same finds, no folder's list
/// read on the window's thread for it.
#[test]
fn kept_indexes_are_taken_back_on_the_lane() {
    let store = store_of("search-ahead");
    let mut s = open(&store, false);
    search(&mut s, "needle", false);
    let first = found(&s);
    assert_eq!(first.len(), 6 * 16);
    drop(s);
    // On the window's thread, as before.
    let mut s = open(&store, false);
    let lists = s.lists_read();
    search(&mut s, "needle", false);
    assert_eq!(found(&s), first);
    let here = s.lists_read() - lists;
    drop(s);
    // On the lane.
    let mut s = open(&store, true);
    let lists = s.lists_read();
    search(&mut s, "needle", true);
    assert_eq!(found(&s), first);
    assert!(s.lists_read() - lists < here, "{} lists read here, {here} without the lane", s.lists_read() - lists);
}

/// A folder changed after the lane took its index back is taken back here, against its list now:
/// the message come since is found.
#[test]
fn a_folder_changed_since_the_lane_read_it_is_taken_back_here() {
    let store = store_of("search-ahead-changed");
    let mut s = open(&store, false);
    search(&mut s, "needle", false);
    drop(s);
    let mut s = open(&store, true);
    // The lane takes back every folder asked before the search joins them.
    let _ = s.run("search.run", &json!({"query": "needle", "scope": "mailbox", "index": 500})).unwrap();
    while let Some(work) = s.take_index_work() {
        work.run();
        let _ = s.run("search.run", &json!({"query": "needle", "scope": "mailbox", "index": 500})).unwrap();
    }
    let mut st = store.load_folder("a1", "Folder5").unwrap();
    let key = MessageKey { account: "a1".into(), folder: "Folder5".into(), validity: 1, uid: 801 };
    st.messages.insert(0, Summary { key: key.clone(), subject: "needle came since".into(), date: NOW, ..Default::default() });
    store.save_folder("a1", "Folder5", &st).unwrap();
    search(&mut s, "needle", true);
    assert!(found(&s).contains(&key), "the message come since is found");
    assert_eq!(found(&s).len(), 6 * 16 + 1);
}

/// A lane that never takes the work: once the wait is long, the search takes the folder back
/// itself, and ends.
#[test]
fn a_lane_that_never_works_is_waited_for_a_while() {
    let store = store_of("search-ahead-stuck");
    let mut s = open(&store, false);
    search(&mut s, "needle", false);
    let first = found(&s);
    drop(s);
    let mut s = open(&store, true);
    for _ in 0..5 {
        let _ = s.run("search.run", &json!({"query": "needle", "scope": "mailbox", "index": 500})).unwrap();
    }
    assert!(s.search_pending, "it waits for the lane");
    s.clock.now += 60;
    search(&mut s, "needle", false);
    assert_eq!(found(&s), first);
}
