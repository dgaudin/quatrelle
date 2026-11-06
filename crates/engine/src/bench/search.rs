// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What search costs on a mailbox the size of a big real one, to weigh an index kept on disk
//! (upstream's, the survey's P3) against this fork's: built in memory a slice a frame, each
//! folder's kept in its `search.idx` once read and taken back by a later run while the folder
//! hasn't changed. The mailbox is made up ([`crate::sample::big`]: 50,000 messages in 40
//! folders, every one downloaded; `MC_MESSAGES` for another size, `MC_WORDS` and `MC_VOCABULARY`
//! for longer texts in more words than its notes of 40 in 2048), in a data file as the app keeps
//! it. No copy of anyone's mail.
//!
//! The window is played as it searches: its clock, 6 ms of each call, at most 4096 messages a
//! call, a call a frame. Printed: the calls (frames) and the time they took, the worst call, what
//! the index holds and what the process holds (`VmRSS`), for the first search, the searches
//! after it, a search after a restart, and one after a restart with a message come into the
//! Inbox. Run it in a release build:
//!
//! ```text
//! cargo test -p quatrelle-engine --release --lib bench::search::fifty_thousand -- --ignored --nocapture
//! ```

use std::time::{Duration, Instant};

use crate::model::{MessageKey, Summary};
use crate::sample::big;
use crate::{SearchClock, SearchScope, Session};

use super::*;

const NOW: i64 = 1_800_000_000;
/// What the window gives a search: a share of each frame, and messages at most.
const SHARE: Duration = Duration::from_millis(6);
const SLICE: usize = 4096;
/// A frame at 60 Hz: what the calls would take on screen, drawing between them.
const FRAME_MS: f64 = 1000.0 / 60.0;

/// The process's resident memory now, in MB.
fn rss_mb() -> f64 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    s.lines().find(|l| l.starts_with("VmRSS:")).and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) / 1024.0
}

/// A session on the store with the window's clock.
fn window(store: &Store) -> Session {
    let mut s = Session::open(Some(store.clone()), Clock::new(NOW, 0));
    let started = Instant::now();
    s.set_search_clock(Some(SearchClock { budget: SHARE, now: Box::new(move || started.elapsed()) }));
    s
}

/// A search over every folder, as the window carries it: a call, then a call a frame while more
/// is left. The calls, their time in all and the worst; what it found.
fn search(s: &mut Session, q: &str) -> (usize, Duration, Duration, usize, u64) {
    let reads = s.store.as_ref().map_or(0, |st| st.reads());
    let mut times = Vec::new();
    let t = Instant::now();
    s.run_search_budgeted(q, SearchScope::All, SLICE);
    times.push(t.elapsed());
    lane(s, t.elapsed());
    while s.search_pending && times.len() < 1_000_000 {
        let t = Instant::now();
        s.search_more(SLICE);
        times.push(t.elapsed());
        lane(s, t.elapsed());
    }
    let total = times.iter().sum();
    let worst = times.iter().max().copied().unwrap_or_default();
    let read = s.store.as_ref().map_or(0, |st| st.reads()).saturating_sub(reads);
    (times.len(), total, worst, s.list.len(), read)
}

thread_local! {
    /// The search lane: one thread, as the window's.
    static LANE: std::cell::OnceCell<std::sync::mpsc::Sender<crate::IndexWork>> = const { std::cell::OnceCell::new() };
}

/// What the window does between two calls, when it takes the kept indexes back on its search
/// lane: the work sent to a thread, and the rest of the frame drawn (the lane works meanwhile).
fn lane(s: &mut Session, call: Duration) {
    if let Some(work) = s.take_index_work() {
        LANE.with(|l| {
            let tx = l.get_or_init(|| {
                let (tx, rx) = std::sync::mpsc::channel::<crate::IndexWork>();
                std::thread::spawn(move || {
                    while let Ok(w) = rx.recv() {
                        w.run();
                    }
                });
                tx
            });
            let _ = tx.send(work);
        });
    }
    if s.index_reading_deferred() {
        std::thread::sleep(Duration::from_secs_f64(FRAME_MS / 1000.0).saturating_sub(call));
    }
}

fn report(what: &str, (calls, total, worst, found, read): (usize, Duration, Duration, usize, u64)) {
    println!(
        "{what}: {calls} calls (about {:.1} s of frames at 60 Hz), {:.2} s in them, worst {:.1} ms; found {found}; {read} sources read",
        calls as f64 * FRAME_MS / 1000.0,
        total.as_secs_f64(),
        worst.as_secs_f64() * 1000.0
    );
}

fn sizes(s: &Session, messages: usize) {
    let Some((docs, words, postings, bytes, kept)) = s.search_index_sizes() else { return };
    let n = messages.max(1) as f64;
    println!(
        "  index: {docs} messages, {words} words ({bytes} bytes), {postings} postings ({:.0} a message); \
         message text kept for the next search {:.1} MB; process {:.0} MB",
        postings as f64 / n,
        kept as f64 / 1e6,
        rss_mb()
    );
}

#[test]
#[ignore]
fn fifty_thousand() {
    let env = |k: &str, or: u64| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(or);
    let messages = u32::try_from(env("MC_MESSAGES", 50_000)).unwrap_or(50_000);
    let dir = std::env::temp_dir().join(format!("mc-search-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open_packed(dir.join("root"), dir.join("files")).unwrap();
    let t = Instant::now();
    let shape = big::Shape {
        messages,
        inbox: messages / 50 * 12,
        words: env("MC_WORDS", 40),
        vocabulary: env("MC_VOCABULARY", 2048),
        ..big::Shape::default()
    };
    let truth = big::generate(&store, NOW, shape).unwrap();
    let text: u64 = s_folders(&store)
        .iter()
        .flat_map(|f| store.load_folder(big::ACCOUNT, f).map(|st| st.messages).unwrap_or_default())
        .map(|m| u64::from(m.size))
        .sum();
    println!(
        "made {} messages ({} words a text, about, in {}; {:.0} MB of sources) in {:.1} s",
        truth.messages,
        shape.words,
        shape.vocabulary,
        text as f64 / 1e6,
        t.elapsed().as_secs_f64()
    );
    drop(store);
    let store = Store::open_packed(dir.join("root"), dir.join("files")).unwrap();
    let n = truth.messages;

    // The first search: the whole index built.
    let mut s = window(&store);
    let before = rss_mb();
    println!("opened: process {before:.0} MB");
    let first = search(&mut s, big::RARE);
    report("first search, the index built", first);
    assert_eq!(first.3, truth.rare);
    sizes(&s, n);
    println!("  process grew {:.0} MB ({:.0} bytes a message)", rss_mb() - before, (rss_mb() - before) * 1e6 / n as f64);

    // Searches after it, the index built.
    let accented = big::ACCENTED.to_lowercase().replace('ü', "u");
    for (q, want) in [
        (big::RARE.to_string(), Some(truth.rare)),
        (accented.clone(), Some(truth.accented)),
        (big::FAR.to_string(), Some(truth.far)),
        (format!("\"{}\"", big::PHRASE), Some(truth.phrase)),
        ("is:read".to_string(), Some(truth.read)),
    ] {
        let r = search(&mut s, &q);
        report(&format!("then {q}"), r);
        if let Some(want) = want {
            assert_eq!(r.3, want, "{q}");
        }
    }
    drop(s);

    // A restart: each folder's index taken back from its file.
    let kept: usize = s_folders(&store).iter().filter_map(|f| store.load_search_index(big::ACCOUNT, f)).map(|b| b.len()).sum();
    println!("kept on disk: {:.1} MB ({:.0} bytes a message)", kept as f64 / 1e6, kept as f64 / n as f64);
    let mut s = window(&store);
    let before = rss_mb();
    report("after a restart", search(&mut s, big::RARE));
    sizes(&s, n);
    println!("  process grew {:.0} MB", rss_mb() - before);
    // In the same run: a message comes into the Inbox, and the next search.
    let came = arrive(&store, 1, "Came while open");
    s.apply(crate::model::Event::FolderChanged { account: big::ACCOUNT.into(), folder: "INBOX".into() });
    let r = search(&mut s, big::RARE);
    report("in the run, a message come into the Inbox", r);
    assert_eq!(r.3, truth.rare + came);
    // Every message after it a place down in its list: searches that look at many of them.
    for q in [accented.as_str(), "is:read"] {
        report(&format!("  then {q}"), search(&mut s, q));
    }
    drop(s);

    // A message comes into the Inbox: what a restart reads again.
    let came = came + arrive(&store, 2, "New one");
    let mut s = window(&store);
    let r = search(&mut s, big::RARE);
    report("after a restart, a message come into the Inbox", r);
    assert_eq!(r.3, truth.rare + came);
    drop(s);
    // Then one deleted from the Inbox and one moved from it to another folder.
    let mut inbox = store.load_folder(big::ACCOUNT, "INBOX").unwrap();
    let gone = inbox.messages.remove(inbox.messages.len() / 2);
    let mut moved = inbox.messages.remove(inbox.messages.len() / 3);
    store.save_folder(big::ACCOUNT, "INBOX", &inbox).unwrap();
    store.remove_body(&gone.key);
    let other = s_folders(&store).into_iter().find(|f| f != "INBOX").unwrap_or_default();
    let raw = store.read_body(&moved.key).unwrap_or_default();
    store.remove_body(&moved.key);
    let mut there = store.load_folder(big::ACCOUNT, &other).unwrap();
    moved.key = MessageKey { folder: other.clone(), uid: there.messages.iter().map(|m| m.key.uid).max().unwrap_or(0) + 1, ..moved.key };
    store.write_body(&moved.key, &raw).unwrap();
    there.messages.insert(0, moved);
    store.save_folder(big::ACCOUNT, &other, &there).unwrap();
    let mut s = window(&store);
    report("after a restart, one deleted from the Inbox and one moved out of it", search(&mut s, big::RARE));
    drop(s);
    drop(store);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A message with the rare word comes into the Inbox, numbered after the others: how many came
/// (one).
fn arrive(store: &Store, n: u32, subject: &str) -> usize {
    let mut inbox = store.load_folder(big::ACCOUNT, "INBOX").unwrap();
    let uid = inbox.messages.iter().map(|m| m.key.uid).max().unwrap_or(0) + n;
    let key = MessageKey { account: big::ACCOUNT.into(), folder: "INBOX".into(), validity: 1, uid };
    store.write_body(&key, format!("Subject: {subject}\r\n\r\nA {} arrived.\r\n", big::RARE).as_bytes()).unwrap();
    inbox.messages.insert(0, Summary { key, subject: subject.into(), date: NOW, ..Default::default() });
    store.save_folder(big::ACCOUNT, "INBOX", &inbox).unwrap();
    1
}

/// The made-up mailbox's account made a server's, as the owner's (a local account's lists are
/// counted again at each start, and read whole by a search).
fn server_account(store: &Store) {
    let mut accounts = store.load_accounts().unwrap();
    for a in &mut accounts {
        a.imap.host = "imap.big.example".into();
        a.smtp.host = "smtp.big.example".into();
    }
    store.save_accounts(&accounts).unwrap();
}

/// The made-up mailbox's folders.
fn s_folders(store: &Store) -> Vec<String> {
    store.load_folders(big::ACCOUNT).unwrap_or_default().into_iter().map(|f| f.path).collect()
}

/// A search over every folder, counting the folders' lists read from the cache on disk (not
/// found in memory) at each call: the calls, the time in them, the worst call, the lists read
/// in all and the most in a call, and the messages they held in all.
fn search_lists(s: &mut Session, q: &str) -> (usize, Duration, Duration, u64, u64, u64) {
    let call = |s: &mut Session, first: bool| {
        let (read, messages) = (s.lists_read() + s.pages_read(), s.messages_read());
        let t = Instant::now();
        if first {
            s.run_search_budgeted(q, SearchScope::All, SLICE);
        } else {
            s.search_more(SLICE);
        }
        (t.elapsed(), s.lists_read() + s.pages_read() - read, s.messages_read() - messages)
    };
    let mut calls = vec![call(s, true)];
    while s.search_pending && calls.len() < 1_000_000 {
        calls.push(call(s, false));
    }
    let total = calls.iter().map(|c| c.0).sum();
    let worst = calls.iter().map(|c| c.0).max().unwrap_or_default();
    (calls.len(), total, worst, calls.iter().map(|c| c.1).sum(), calls.iter().map(|c| c.1).max().unwrap_or(0), calls.iter().map(|c| c.2).sum())
}

/// A search of everything in a mailbox bigger than the session's cache of folders' lists
/// (`MC_MESSAGES`, 60,000 by default, in `MC_FOLDERS`, 40 by default), on a server account: how
/// many lists (or pages of them) each call reads again from the disk, and what it costs, for the
/// first search (the index built) and the ones after.
///
/// ```text
/// cargo test -p quatrelle-engine --release --lib bench::search::lists_read_by_a_search_of_everything -- --ignored --nocapture
/// ```
#[test]
#[ignore]
fn lists_read_by_a_search_of_everything() {
    let messages = std::env::var("MC_MESSAGES").ok().and_then(|v| v.parse().ok()).unwrap_or(60_000u32);
    let dir = std::env::temp_dir().join(format!("mc-search-lists-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open_packed(dir.join("root"), dir.join("files")).unwrap();
    let folders = std::env::var("MC_FOLDERS").ok().and_then(|v| v.parse().ok()).unwrap_or(40usize);
    let shape = big::Shape { messages, inbox: messages / 50 * 12, folders, ..big::Shape::default() };
    let truth = big::generate(&store, NOW, shape).unwrap();
    // A server account, as the owner's (a local one's marks may be on their way to its lists,
    // which are read whole).
    server_account(&store);
    drop(store);
    let store = Store::open_packed(dir.join("root"), dir.join("files")).unwrap();
    let mut s = window(&store);
    s.take_jobs();
    let before = rss_mb();
    println!("{} messages in {} folders; opened: process {before:.0} MB", truth.messages, shape.folders);
    let accented = big::ACCENTED.to_lowercase().replace('ü', "u");
    for (i, q) in [big::RARE, accented.as_str(), big::FAR, "is:read", big::RARE].into_iter().enumerate() {
        let (calls, total, worst, lists, most, messages) = search_lists(&mut s, q);
        println!(
            "{}{q}: {calls} calls, {:.2} s in them, worst {:.1} ms; {lists} lists or pages read from the disk, at most {most} in a call, \
             {messages} messages in them; found {}; process {:.0} MB",
            if i == 0 { "first search (the index built), " } else { "then " },
            total.as_secs_f64(),
            worst.as_secs_f64() * 1000.0,
            s.list.len(),
            rss_mb()
        );
    }
    let cached: usize = s.cached_folders().len();
    println!("lists in memory: {cached} folders; process {:.0} MB (grew {:.0} MB)", rss_mb(), rss_mb() - before);
    drop(s);
    drop(store);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A restart over a mailbox whose index is kept (`MC_MESSAGES`, 50,000 by default, in 40
/// folders, `MC_WORDS` and `MC_VOCABULARY` for longer texts): what opening the session costs, and
/// the first search after it (the folders' kept indexes taken back), as the window carries it.
///
/// ```text
/// cargo test -p quatrelle-engine --release --lib bench::search::a_restart_with_the_index_kept -- --ignored --nocapture
/// ```
#[test]
#[ignore]
fn a_restart_with_the_index_kept() {
    let env = |k: &str, or: u64| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(or);
    let messages = u32::try_from(env("MC_MESSAGES", 50_000)).unwrap_or(50_000);
    let dir = std::env::temp_dir().join(format!("mc-search-restart-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open_packed(dir.join("root"), dir.join("files")).unwrap();
    let shape = big::Shape {
        messages,
        inbox: messages / 50 * 12,
        words: env("MC_WORDS", 40),
        vocabulary: env("MC_VOCABULARY", 2048),
        ..big::Shape::default()
    };
    let truth = big::generate(&store, NOW, shape).unwrap();
    server_account(&store);
    let mut s = window(&store);
    s.take_jobs();
    report("the index built and kept", search(&mut s, big::RARE));
    drop(s);
    drop(store);
    // On the window's thread, then on its search lane.
    for (run, deferred) in [(1, false), (2, false), (3, true), (4, true)] {
        let store = Store::open_packed(dir.join("root"), dir.join("files")).unwrap();
        let before = rss_mb();
        let t = Instant::now();
        let mut s = window(&store);
        let opened = t.elapsed();
        s.take_jobs();
        println!(
            "restart {run}: the session opened in {:.1} ms, {} lists read; process {:.0} MB (grew {:.0} MB)",
            opened.as_secs_f64() * 1000.0,
            s.lists_read(),
            rss_mb(),
            rss_mb() - before
        );
        s.defer_index_reading(deferred);
        let lists = s.lists_read();
        let t = Instant::now();
        let r = search(&mut s, big::RARE);
        report(if deferred { "  the first search, the kept indexes taken back on the lane" } else { "  the first search" }, r);
        println!("  done in {:.2} s on the clock", t.elapsed().as_secs_f64());
        assert_eq!(r.3, truth.rare);
        println!("  {} lists read from the disk; process {:.0} MB", s.lists_read() - lists, rss_mb());
        drop(s);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
