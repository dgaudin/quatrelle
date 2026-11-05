// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: a search paced by the window's clock, a share of each frame.

use std::time::{Duration, Instant};

use quatrelle_model::{AccountConfig, Folder};
use quatrelle_store::FolderState;

use crate::SearchClock;

use super::*;

/// The share of a frame the window gives a search.
const BUDGET: Duration = Duration::from_millis(6);

/// A local account `a1` with one folder of `per` messages, every `every`-th downloaded with a
/// long body (`body_kb` KB of words): those cost the most to index.
fn heavy_store(name: &str, per: u32, every: u32, body_kb: usize) -> Store {
    let store = temp(name);
    store.save_accounts(&[AccountConfig { id: "a1".into(), email: "me@studio.example".into(), ..Default::default() }]).unwrap();
    let folder = Folder { account: "a1".into(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, total: per, ..Default::default() };
    store.save_folders("a1", &[folder]).unwrap();
    let words: String = (0..body_kb * 1024 / 8).map(|i| format!("w{:05} ", i % 99_991)).collect();
    let mut messages = Vec::new();
    for uid in (1..=per).rev() {
        let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
        let subject = if uid.is_multiple_of(100) { format!("needle {uid}") } else { format!("Report {uid}") };
        if uid.is_multiple_of(every) {
            let raw = format!("From: ann@studio.example\r\nSubject: {subject}\r\nContent-Type: text/plain\r\n\r\n{words}\r\n");
            store.write_body(&key, raw.as_bytes()).unwrap();
        }
        messages.push(Summary {
            key,
            subject,
            from: EmailAddress::new("Ann Person", "ann@studio.example"),
            date: NOW - 86_400 + i64::from(uid) * 10,
            ..Default::default()
        });
    }
    store.save_folder("a1", "INBOX", &FolderState { validity: 1, messages, ..Default::default() }).unwrap();
    store
}

fn open(store: &Store) -> Session {
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    s.take_jobs();
    s
}

/// A search driven a call at a time, `index` messages at most a call: how long each call took,
/// and how many it found.
fn drive(s: &mut Session, index: u64) -> (Vec<Duration>, usize) {
    let mut times = Vec::new();
    for _ in 1..100_000 {
        let t = Instant::now();
        let v = s.run("search.run", &json!({"query": "needle", "scope": "mailbox", "index": index})).unwrap();
        times.push(t.elapsed());
        if v["pending"] == false {
            return (times, s.list.len());
        }
    }
    panic!("the search must finish");
}

/// Downloaded messages with long bodies (a few milliseconds each to index, here), searched as the
/// window searches: 500 messages a call took 480 ms a call here, a frame frozen for half a
/// second each; given a share of the frame by the window's clock, a call keeps to it (a message
/// is begun only while one as long as the longest so far still fits), and the search finds what
/// a whole one does (every hundredth message).
///
/// Not paced yet: the call that ends the folder keeps its index on disk (one unit, growing with
/// the folder); the next joins it to the mailbox's (the two were one call of 45 ms here).
#[test]
fn a_search_keeps_to_its_share_of_a_frame() {
    let store = heavy_store("paced", 1200, 2, 16);
    let mut s = open(&store);
    let start = Instant::now();
    s.set_search_clock(Some(SearchClock { budget: BUDGET, now: Box::new(move || start.elapsed()) }));
    let (times, found) = drive(&mut s, 4096);
    let (indexing, last) = times.split_at(times.len().saturating_sub(2));
    let slowest = indexing.iter().max().copied().unwrap_or_default();
    println!("{} calls, slowest indexing {slowest:?}, the last two (the folder kept, then joined) {last:?}; {found} found", times.len());
    assert_eq!(found, 12);
    assert!(times.len() > 20, "spread over many frames: {} calls", times.len());
    // Generous for a loaded machine: one message is a couple of milliseconds here.
    assert!(slowest <= 5 * BUDGET, "a call took {slowest:?}");
    done(s);
}

/// Without the window's clock (the command line, the tests), a call still takes the messages it
/// asks for, no more.
#[test]
fn without_a_clock_a_call_takes_its_count() {
    let store = heavy_store("paced-count", 1200, 600, 1);
    let mut s = open(&store);
    let (times, found) = drive(&mut s, 500);
    assert_eq!((times.len(), found), (3, 12), "1200 messages, 500 a call");
    done(s);
}
