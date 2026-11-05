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
fn search(s: &mut Session, q: &str) -> (usize, Duration, Duration, usize) {
    let mut times = Vec::new();
    let t = Instant::now();
    s.run_search_budgeted(q, SearchScope::All, SLICE);
    times.push(t.elapsed());
    while s.search_pending && times.len() < 1_000_000 {
        let t = Instant::now();
        s.search_more(SLICE);
        times.push(t.elapsed());
    }
    let total = times.iter().sum();
    let worst = times.iter().max().copied().unwrap_or_default();
    (times.len(), total, worst, s.list.len())
}

fn report(what: &str, (calls, total, worst, found): (usize, Duration, Duration, usize)) {
    println!(
        "{what}: {calls} calls (about {:.1} s of frames at 60 Hz), {:.2} s in them, worst {:.1} ms; found {found}",
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
        (accented, Some(truth.accented)),
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
    drop(s);

    // A message comes into the Inbox: a restart reads the Inbox again, whole.
    let mut inbox = store.load_folder(big::ACCOUNT, "INBOX").unwrap();
    let uid = inbox.messages.iter().map(|m| m.key.uid).max().unwrap_or(0) + 1;
    let key = MessageKey { account: big::ACCOUNT.into(), folder: "INBOX".into(), validity: 1, uid };
    store.write_body(&key, format!("Subject: New one\r\n\r\nA {} arrived.\r\n", big::RARE).as_bytes()).unwrap();
    inbox.messages.insert(0, Summary { key, subject: "New one".into(), date: NOW, ..Default::default() });
    store.save_folder(big::ACCOUNT, "INBOX", &inbox).unwrap();
    let mut s = window(&store);
    report("after a restart, a message come into the Inbox", search(&mut s, big::RARE));
    drop(s);
    drop(store);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The made-up mailbox's folders.
fn s_folders(store: &Store) -> Vec<String> {
    store.load_folders(big::ACCOUNT).unwrap_or_default().into_iter().map(|f| f.path).collect()
}
