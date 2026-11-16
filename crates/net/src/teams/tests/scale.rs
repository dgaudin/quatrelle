// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The copy at scale, on disk: a copy of 2,000 chats and 300,000 messages (one chat of 30,000)
//! beside one ten times smaller, the worker signed in to the fake Graph over each. Opening the
//! long chat and going back through it 50 pages take no more than twice as long as in the smaller
//! copy, a search no more than twice and 2 ms. So do a new message put in the long chat and one
//! sent there through the fake: a write puts in only the index tranches it changes
//! (`index/<hash>/<n>.json`), not the chat's words whole, so writes are held to twice the smaller
//! copy's like the rest. What a write puts in the index is measured in bytes by the search's own
//! tests (`search/tranches.rs`); here the disk's commit takes most of a write's time.
//!
//! `#[ignore]`: making 300,000 messages in a debug build takes minutes. Run it with
//! `cargo test -p quatrelle-net --lib teams::tests::scale -- --ignored`.

use std::time::{Duration, Instant};

use quatrelle_mime::teams_compose;
use quatrelle_model::pim::teams::{Block, BlockKind, Chat, ChatKind, ChatMessage, ChatText, Event, Job, Member, Op, Run as Words, Sender};
use quatrelle_store::Store;

use super::sync::{NOW, Run, STEP, run_with};
use super::{ME, MINUTE, TENANT, me, teams_with};
use crate::fake_graph::{FakeChat, FakeUser};
use crate::teams::store::{ChatRecord, Copy, Identity};

const DAY: i64 = 24 * 60 * MINUTE;
/// The long chat, the one opened.
const LONG: &str = "19:long@thread.v2";
/// What the messages speak of, one after the other: a seventh of them of the budget.
const TOPICS: [&str; 7] = ["budget", "launch", "review", "travel", "hiring", "design", "support"];

/// A copy's shape: `chats` chats, `total` messages in all, the long chat `long` of them.
#[derive(Clone, Copy, Debug)]
struct Shape {
    chats: usize,
    total: usize,
    long: usize,
}

impl Shape {
    fn id(i: usize) -> String {
        if i == 0 { LONG.to_string() } else { format!("19:c{i:04}@thread.v2") }
    }

    /// How many messages chat `i` has: the rest shared as evenly as can be.
    fn count(&self, i: usize) -> usize {
        let (others, rest) = (self.chats - 1, self.total - self.long);
        match i {
            0 => self.long,
            _ => rest / others + usize::from(i <= rest % others),
        }
    }

    /// When chat `i`'s last message was made: the long chat's the day before the fake's clock,
    /// the others' over a month before (no cadence reads them).
    fn last(i: usize) -> i64 {
        if i == 0 { NOW - DAY } else { NOW - 31 * DAY - i as i64 * 10 * MINUTE }
    }

    /// Message `k` of chat `i`, a minute after the one before (0 its first; past its count, one
    /// new since), from Ana or from the user in turn.
    fn message(&self, i: usize, k: usize) -> ChatMessage {
        let created = Shape::last(i) + (k as i64 - (self.count(i) as i64 - 1)) * MINUTE;
        let sender = if k.is_multiple_of(2) {
            Sender::User { id: "u-ana".into(), name: "Ana Lima".into(), identity_type: Some("aadUser".into()), tenant_id: Some(TENANT.into()) }
        } else {
            Sender::User { id: ME.into(), name: "Morgan Lee".into(), identity_type: Some("aadUser".into()), tenant_id: Some(TENANT.into()) }
        };
        let text = format!("Message {k} about the {}", TOPICS[k % TOPICS.len()]);
        ChatMessage {
            id: created.to_string(),
            chat: Shape::id(i),
            sender,
            created,
            modified: created,
            body: ChatText { blocks: vec![Block { kind: BlockKind::Paragraph, runs: vec![Words { text, ..Default::default() }] }] },
            ..ChatMessage::default()
        }
    }

    /// Chat `i`'s record: its whole history copied, read up to its last message.
    fn record(&self, i: usize) -> ChatRecord {
        let member = |id: &str, name: &str| Member { user_id: id.into(), name: name.into(), tenant_id: Some(TENANT.into()), ..Member::default() };
        let last = Shape::last(i);
        let first = last - (self.count(i) as i64 - 1) * MINUTE;
        let chat = Chat {
            id: Shape::id(i),
            kind: ChatKind::Group,
            topic: Some(format!("Project {i}")),
            members: vec![member(ME, "Morgan Lee"), member("u-ana", "Ana Lima"), member("u-bruno", "Bruno Rossi")],
            last_read: Some(last),
            hidden: false,
            left: false,
            unreadable: None,
            preview: None,
            last_activity: Some(last),
            updated: Some(first),
            web_url: None,
            tenant_id: Some(TENANT.into()),
        };
        ChatRecord { watermark: Some(last), history_from: Some(first), history_oldest: Some(first), done: true, ..ChatRecord::new(chat) }
    }
}

/// A folder of the test's own, empty.
fn temp(name: &str) -> std::path::PathBuf {
    let t = std::env::temp_dir().join(format!("quatrelle-teams-scale-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&t);
    t
}

/// The copy of `shape` made in `store`, a thousand messages a write.
fn build(store: &Store, shape: Shape) {
    let (copy, _) = Copy::open(store.clone(), &Identity { tenant_id: TENANT.into(), object_id: ME.into() }).unwrap();
    let records: Vec<ChatRecord> = (0..shape.chats).map(|i| shape.record(i)).collect();
    for part in records.chunks(50) {
        copy.put_chats(part).unwrap();
    }
    for i in (0..shape.chats).rev() {
        let messages: Vec<ChatMessage> = (0..shape.count(i)).map(|k| shape.message(i, k)).collect();
        for part in messages.chunks(1_000) {
            copy.upsert(&Shape::id(i), part).unwrap();
        }
    }
}

/// The middle of what a step's runs took.
fn median(mut took: Vec<Duration>) -> Duration {
    took.sort();
    took.get(took.len() / 2).copied().unwrap_or_default()
}

/// What each step took in a copy: opening the long chat, a page before, a new message put in,
/// a search, a message sent.
#[derive(Debug)]
struct Took {
    open: Duration,
    page: Duration,
    upsert: Duration,
    search: Duration,
    send: Duration,
}

/// The oldest message the events from `from` on said: the page they made begins there.
fn front(r: &Run, from: usize) -> Option<i64> {
    let seen = r.seen.lock().unwrap();
    seen.iter()
        .skip(from)
        .filter_map(|e| match e {
            quatrelle_model::Event::Teams { event: Event::Messages { messages, .. } } => messages.iter().map(|m| m.created).min(),
            _ => None,
        })
        .min()
}

/// The steps timed in a copy of `shape`, the worker signed in over it.
fn measure(name: &str, shape: Shape) -> Took {
    let dir = temp(name);
    let store = Store::open(&dir).unwrap();
    let started = Instant::now();
    build(&store, shape);
    eprintln!("teams scale: the copy {shape:?} made in {:?}", started.elapsed());
    // The fake has the long chat only: what it is sent to.
    let teams = teams_with(vec![FakeChat::new(LONG, "group", Some("Project 0"), vec![me(), FakeUser::new("u-ana", "Ana Lima")])]);
    let kept = store.clone();
    let mut r = run_with(teams, move |w| w.store = kept);
    {
        let copy = r.w.copy.as_ref().unwrap();
        while copy.index_step() {}
    }
    let job = |r: &mut Run, job: Job| {
        let started = Instant::now();
        r.w.job(job);
        started.elapsed()
    };
    // Opening the long chat, five times.
    let open = median((0..5).map(|_| job(&mut r, Job::Open { chat: Some(LONG.into()) })).collect());
    // Its pages before, up to 50 (the smaller copy's long chat has fewer).
    let mut pages = Vec::new();
    let mut before = r.w.copy.as_ref().unwrap().last_pages(LONG, 2).first().map(|m| m.created);
    while let Some(at) = before.filter(|_| pages.len() < 50) {
        let from = r.seen.lock().unwrap().len();
        pages.push(job(&mut r, Job::Page { chat: LONG.into(), before: at }));
        before = front(&r, from);
    }
    let page = median(pages);
    // A new message put in it, five times.
    let n = shape.count(0);
    let upsert = median(
        (0..5)
            .map(|j| {
                let m = shape.message(0, n + j);
                let started = Instant::now();
                r.w.copy.as_ref().unwrap().upsert(LONG, &[m]).unwrap();
                started.elapsed()
            })
            .collect(),
    );
    // A search, five times.
    let search = median((0..5).map(|_| job(&mut r, Job::Search { query: "budget".into() })).collect());
    let found =
        r.events().into_iter().rev().find_map(|e| if let Event::Found { hits, partial, .. } = e { Some((hits.len(), partial)) } else { None });
    assert_eq!(found, Some((200, false)), "{name}: the newest 200 found, the whole copy searched");
    // A message sent through the fake, five times, a second and a half apart: the job, and the
    // turn that posted it.
    let mut sends = Vec::new();
    for j in 0..5 {
        r.turns(Duration::from_millis(1_500));
        let posted = r.fake(|t| t.posts.len());
        let mut took =
            job(&mut r, Job::Op { op: Op::Send { chat: LONG.into(), local: format!("scale-{j}"), body: teams_compose("Lunch at noon?", &[], &[]) } });
        let until = r.at + Duration::from_secs(5);
        while r.fake(|t| t.posts.len()) == posted {
            assert!(r.at < until, "{name}: the message never went");
            r.at += STEP;
            let started = Instant::now();
            r.w.turn(r.t0 + r.at);
            if r.fake(|t| t.posts.len()) > posted {
                took += started.elapsed();
            }
        }
        sends.push(took);
    }
    let send = median(sends);
    drop(r);
    let _ = std::fs::remove_dir_all(dir);
    Took { open, page, upsert, search, send }
}

/// What the worker does for the screen, and what it writes in the long chat, takes no more than
/// twice as long in a copy ten times bigger (a search, twice and 2 ms: it takes some 50 µs)
/// (spec, Testing › Scale).
#[test]
#[ignore = "makes 330,000 messages: minutes in a debug build"]
fn a_copy_of_2000_chats_and_300000_messages_answers_in_bounded_time() {
    let small = measure("small", Shape { chats: 200, total: 30_000, long: 3_000 });
    let big = measure("big", Shape { chats: 2_000, total: 300_000, long: 30_000 });
    let said = format!("small {small:?}, big {big:?}");
    eprintln!("teams scale: {said}");
    let none = Duration::ZERO;
    let flat = [
        ("opening the long chat", small.open, big.open, none),
        ("a page before", small.page, big.page, none),
        ("a search", small.search, big.search, Duration::from_millis(2)),
        ("a new message put in", small.upsert, big.upsert, none),
        ("a message sent", small.send, big.send, none),
    ];
    let past: Vec<&str> = flat.iter().filter(|(_, a, b, margin)| *b > 2 * *a + *margin).map(|(what, ..)| *what).collect();
    assert!(past.is_empty(), "past their bounds: {past:?}; {said}");
}
