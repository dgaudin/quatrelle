// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: a search of everything in a server mailbox bigger than the lists kept in memory
//! looks at what the index finds in the page of its folder's list it's in, not the folder's list
//! read whole (each pushing another out, every search read most of them again).

use std::collections::BTreeSet;

use quatrelle_model::{AccountConfig, Folder};
use quatrelle_store::FolderState;

use super::*;

/// Three folders of 9,000 messages (three pages of their lists each) on an IMAP account: every
/// thousandth message says "needle" (1, 1001…), a few pages of each folder.
fn big_server_store(name: &str) -> Store {
    let store = temp(name);
    let mut config = AccountConfig { id: "a1".into(), email: "me@studio.example".into(), ..Default::default() };
    config.imap.host = "imap.studio.example".into();
    store.save_accounts(&[config]).unwrap();
    let mut list = Vec::new();
    for i in 0..3u32 {
        let path = if i == 0 { "INBOX".to_string() } else { format!("Folder{i}") };
        let role = if i == 0 { FolderRole::Inbox } else { FolderRole::Other };
        list.push(Folder { account: "a1".into(), path: path.clone(), role, selectable: true, total: 9_000, ..Default::default() });
        let messages = (1..=9_000u32)
            .rev()
            .map(|uid| Summary {
                key: MessageKey { account: "a1".into(), folder: path.clone(), validity: 1, uid },
                subject: if uid % 1000 == 1 { format!("needle {path} {uid}") } else { format!("Report {path} {uid}") },
                date: NOW - 86_400 * 400 + i64::from(i * 9_000 + uid) * 60,
                ..Default::default()
            })
            .collect();
        store.save_folder("a1", &path, &FolderState { validity: 1, messages, ..Default::default() }).unwrap();
    }
    store.save_folders("a1", &list).unwrap();
    store
}

fn search(s: &mut Session, q: &str) {
    for _ in 0..10_000 {
        let v = s.run("search.run", &json!({"query": q, "scope": "mailbox", "index": 500})).unwrap();
        if v["pending"] == false {
            return;
        }
    }
    panic!("{q}: not finished");
}

/// The second search reads no list whole: the pages its finds are in, a few, and the lists in
/// memory stay as they were. It finds what the first did.
#[test]
fn a_search_of_everything_reads_the_pages_of_its_finds() {
    let store = big_server_store("search-pages");
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    s.take_jobs();
    // Room for a folder of the three.
    s.set_cache_limit(10_000);
    search(&mut s, "needle");
    let first: BTreeSet<MessageKey> = s.list.iter().map(|m| m.key.clone()).collect();
    assert_eq!(first.len(), 27, "nine in each folder");
    let (lists, pages, cached) = (s.lists_read(), s.pages_read(), s.cached_folders());
    for q in ["report 4", "needle"] {
        search(&mut s, q);
        assert_eq!(s.lists_read(), lists, "{q}: no list read whole");
        assert_eq!(s.cached_folders(), cached, "{q}: the lists in memory as they were");
    }
    let found: BTreeSet<MessageKey> = s.list.iter().map(|m| m.key.clone()).collect();
    assert_eq!(found, first);
    let read = s.pages_read() - pages;
    assert!(read > 0 && read <= 3 * 3 * 2, "{read} pages read for two searches");
}
