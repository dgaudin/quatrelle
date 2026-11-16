// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The search of the local copy: words of a message's text and of its sender's name, chats by
//! their names, kept as the copy is written (an edit replaces a message's words, a deletion takes
//! them out), read back from the file in slices and held to the caps, the oldest chats' words
//! leaving first past the index's bound.

use std::sync::Arc;
use std::time::Instant;

use quatrelle_model::pim::teams::{self as model, Block, BlockKind, Chat, ChatKind, ChatMessage, ChatText, Job, Member, Run, Sender, TEAMS_ACCOUNT};
use quatrelle_store::Store;

use super::worker::{AtDesk, Kept, Seen, desk, emit_into, grant, teams_of};
use super::{MINUTE, T0};
use crate::teams::search::{Found, MAX_HITS};
use crate::teams::store::{ChatRecord, Copy, Identity, Opened, chat_hash};
use crate::teams::worker::Worker;

const A: &str = "19:a1b2@thread.v2";
const B: &str = "19:c3d4@thread.v2";

fn who() -> Identity {
    Identity { tenant_id: "t-1".into(), object_id: "u-me".into() }
}

fn chat(id: &str, topic: Option<&str>) -> Chat {
    Chat {
        id: id.into(),
        kind: ChatKind::Group,
        topic: topic.map(str::to_string),
        members: vec![
            Member { user_id: "u-me".into(), name: "Morgan Lee".into(), ..Member::default() },
            Member { user_id: "u-ana".into(), name: "Ana Lima".into(), ..Member::default() },
        ],
        last_read: None,
        hidden: false,
        left: false,
        unreadable: None,
        preview: None,
        last_activity: Some(T0),
        updated: None,
        web_url: None,
        tenant_id: None,
    }
}

fn text(s: &str) -> ChatText {
    ChatText { blocks: vec![Block { kind: BlockKind::Paragraph, runs: vec![Run { text: s.into(), ..Default::default() }] }] }
}

/// Message `i` of a chat, made `i` minutes after T0 (its id its time), from Ana, saying `says`.
fn msg(chat: &str, i: i64, says: &str) -> ChatMessage {
    let at = T0 + i * MINUTE;
    ChatMessage {
        id: at.to_string(),
        chat: chat.into(),
        sender: Sender::User { id: "u-ana".into(), name: "Ana Lima".into(), identity_type: Some("aadUser".into()), tenant_id: Some("t-1".into()) },
        created: at,
        modified: at,
        body: text(says),
        ..ChatMessage::default()
    }
}

fn id(i: i64) -> String {
    (T0 + i * MINUTE).to_string()
}

/// The copy in `store`, its index read back whole.
fn open(store: &Store) -> Copy {
    let (copy, _) = Copy::open(store.clone(), &who()).unwrap();
    let mut slices = 0;
    while copy.index_step() {
        slices += 1;
        assert!(slices < 1_000, "the index never ends loading");
    }
    copy
}

/// A copy in memory with chats A ("Budget review") and B (no topic), its index loaded, and its
/// store.
fn copy() -> (Copy, Store) {
    let store = Store::memory();
    let (copy, opened) = Copy::open(store.clone(), &who()).unwrap();
    assert_eq!(opened, Opened::New);
    copy.put_chat(&ChatRecord::new(chat(A, Some("Budget review")))).unwrap();
    copy.put_chat(&ChatRecord::new(chat(B, None))).unwrap();
    drop(copy);
    (open(&store), store)
}

fn hits(copy: &Copy, query: &str) -> Vec<(String, String)> {
    let found = copy.found(query);
    assert!(!found.indexing, "the index is loaded");
    found.hits
}

fn one(chat: &str, message: &str) -> Vec<(String, String)> {
    vec![(chat.to_string(), message.to_string())]
}

/// The index's segment of a chat as the file keeps it.
fn segment(store: &Store, chat: &str) -> Option<String> {
    store.account_file(TEAMS_ACCOUNT, &format!("index/{}.json", chat_hash(chat))).unwrap().map(|b| String::from_utf8(b).unwrap())
}

#[test]
fn finds_a_word_of_a_message_and_a_chat_name() {
    let (copy, store) = copy();
    copy.upsert(A, &[msg(A, 1, "The launch plan, for l'équipe")]).unwrap();
    copy.upsert(B, &[msg(B, 2, "Lunch at noon?")]).unwrap();

    let check = |copy: &Copy| {
        assert_eq!(hits(copy, "launch"), one(A, &id(1)));
        // Every word, as the beginning of one, whatever its case or accents.
        assert_eq!(hits(copy, "LAUNCH pla"), one(A, &id(1)));
        assert_eq!(hits(copy, "equipe"), one(A, &id(1)));
        assert_eq!(hits(copy, "launch noon"), Vec::<(String, String)>::new());
        // The sender's name: of both messages, the newest first.
        assert_eq!(hits(copy, "lima"), vec![(B.to_string(), id(2)), (A.to_string(), id(1))]);
        // A chat by its name: no message of it.
        assert_eq!(hits(copy, "budget rev"), one(A, ""));
        assert_eq!(hits(copy, ""), Vec::<(String, String)>::new());
        let found = copy.found("noon");
        assert_eq!(found, Found { hits: one(B, &id(2)), indexing: false, partial: false });
    };
    check(&copy);
    // Read back from the file as it was written.
    drop(copy);
    check(&open(&store));
}

#[test]
fn an_edit_replaces_the_indexed_words() {
    let (copy, store) = copy();
    copy.upsert(A, &[msg(A, 1, "first draft"), msg(A, 2, "a draft too")]).unwrap();
    let mut edited = msg(A, 1, "final version");
    edited.edited = Some(T0 + 5 * MINUTE);
    copy.upsert(A, &[edited]).unwrap();

    let check = |copy: &Copy| {
        assert_eq!(hits(copy, "first"), Vec::<(String, String)>::new());
        assert_eq!(hits(copy, "draft"), one(A, &id(2)));
        assert_eq!(hits(copy, "final"), one(A, &id(1)));
    };
    check(&copy);
    drop(copy);
    check(&open(&store));
}

#[test]
fn a_deleted_message_leaves_the_index() {
    let (copy, store) = copy();
    copy.upsert(A, &[msg(A, 1, "the secret plan"), msg(A, 2, "plan b")]).unwrap();
    assert!(segment(&store, A).unwrap().contains("secret"));
    let mut deleted = msg(A, 1, "");
    deleted.deleted = Some(T0 + 9 * MINUTE);
    copy.upsert(A, &[deleted]).unwrap();

    let check = |copy: &Copy| {
        assert_eq!(hits(copy, "secret"), Vec::<(String, String)>::new());
        assert_eq!(hits(copy, "plan"), one(A, &id(2)));
    };
    check(&copy);
    // Nothing of it in the file either: not its words, not its id.
    let kept = segment(&store, A).unwrap();
    assert!(!kept.contains("secret") && !kept.contains(&id(1)), "{kept}");
    drop(copy);
    check(&open(&store));
}

#[test]
fn words_per_message_and_length_are_capped() {
    let (copy, store) = copy();
    // The sender's two words ("ana", "lima") come first, then the text's: 2,000 in all.
    let many: Vec<String> = (0..2_500).map(|i| format!("w{i:04}")).collect();
    let long = "x".repeat(100);
    copy.upsert(A, &[msg(A, 1, &many.join(" ")), msg(A, 2, &format!("a {long} word"))]).unwrap();

    assert_eq!(hits(&copy, "ana w1997"), one(A, &id(1)));
    assert_eq!(hits(&copy, "w1998"), Vec::<(String, String)>::new());
    // A word is kept to its first 64 characters, and so is a word looked for.
    assert_eq!(hits(&copy, &"x".repeat(64)), one(A, &id(2)));
    assert_eq!(hits(&copy, &long), one(A, &id(2)));
    let kept: serde_json::Value = serde_json::from_str(&segment(&store, A).unwrap()).unwrap();
    let words = kept["words"].as_object().unwrap();
    assert_eq!(words.len(), 2_000 + 3, "2,000 of the first message, 'a', the long one and 'word' of the second ('ana' and 'lima' shared)");
    assert!(words.keys().all(|w| w.chars().count() <= 64));
}

#[test]
fn a_hostile_index_is_held_to_the_caps() {
    let (copy, store) = copy();
    copy.upsert(A, &[msg(A, 1, "kept")]).unwrap();
    copy.upsert(B, &[msg(B, 1, "kept")]).unwrap();
    drop(copy);
    let long = "y".repeat(100);
    let many: serde_json::Map<String, serde_json::Value> = (0..2_500).map(|i| (format!("z{i:04}"), serde_json::json!([0]))).collect();
    let mut words = many;
    words.insert(long.clone(), serde_json::json!([0]));
    // A place no message has, a free place (an empty id).
    words.insert("ghost".into(), serde_json::json!([7, 1]));
    let planted = serde_json::json!({ "chat": A, "messages": [[id(1), T0 + MINUTE], ["", 0]], "words": words });
    // A's segment as planted; B's under A's key would be another chat's: passed over.
    let under_another = serde_json::json!({ "chat": A, "messages": [[id(1), T0]], "words": { "stolen": [0] } });
    store
        .write_account_files(
            TEAMS_ACCOUNT,
            &[
                (format!("index/{}.json", chat_hash(A)), serde_json::to_vec(&planted).unwrap()),
                (format!("index/{}.json", chat_hash(B)), serde_json::to_vec(&under_another).unwrap()),
            ],
        )
        .unwrap();
    let copy = open(&store);
    assert_eq!(hits(&copy, "z0000"), one(A, &id(1)));
    assert_eq!(hits(&copy, "z1999"), one(A, &id(1)));
    assert_eq!(hits(&copy, "z2000"), Vec::<(String, String)>::new(), "2,000 words a message at most");
    assert_eq!(hits(&copy, &long), Vec::<(String, String)>::new(), "a word past 64 characters");
    assert_eq!(hits(&copy, "ghost"), Vec::<(String, String)>::new(), "no message there");
    assert_eq!(hits(&copy, "stolen"), Vec::<(String, String)>::new());
    assert!(copy.found("kept").hits.iter().all(|(c, m)| !c.is_empty() && !m.is_empty()));
}

#[test]
fn nothing_is_found_under_an_empty_id() {
    let (copy, store) = copy();
    copy.upsert(A, &[msg(A, 1, "kept")]).unwrap();
    drop(copy);
    let planted = serde_json::json!({ "chat": A, "messages": [["", T0], [id(1), T0 + MINUTE]], "words": { "ghost": [0], "kept": [1] } });
    store.write_account_files(TEAMS_ACCOUNT, &[(format!("index/{}.json", chat_hash(A)), serde_json::to_vec(&planted).unwrap())]).unwrap();
    let copy = open(&store);
    assert_eq!(hits(&copy, "ghost"), Vec::<(String, String)>::new());
    assert_eq!(hits(&copy, "kept"), one(A, &id(1)));
}

#[test]
fn until_loaded_a_search_says_indexing() {
    let store = Store::memory();
    {
        let (copy, _) = Copy::open(store.clone(), &who()).unwrap();
        for (i, c) in ["19:a@thread.v2", "19:b@thread.v2", "19:c@thread.v2"].into_iter().enumerate() {
            copy.put_chat(&ChatRecord::new(chat(c, None))).unwrap();
            copy.upsert(c, &[msg(c, i64::try_from(i).unwrap(), "the launch plan")]).unwrap();
        }
    }
    let seen = Seen::default();
    let mut w =
        Worker::new(store.clone(), Arc::new(Kept::default()), emit_into(&seen), "https://graph.invalid".into(), Box::new(AtDesk(desk(grant(None)))))
            .unwrap();
    w.copy = Some(Copy::open(store, &who()).unwrap().0);
    let now = Instant::now();
    let last_found = |seen: &Seen| {
        teams_of(seen)
            .into_iter()
            .rev()
            .find_map(|e| if let model::Event::Found { query, hits, indexing, partial } = e { Some((query, hits, indexing, partial)) } else { None })
            .unwrap()
    };

    w.job(Job::Search { query: "launch".into() });
    assert_eq!(last_found(&seen), ("launch".to_string(), Vec::new(), true, false));
    // While it loads, the loop comes back at once for the next slice, signed in or not.
    assert!(w.wake(now).is_some_and(|at| at <= now + std::time::Duration::from_millis(20)));
    // A slice a turn: the chats listed, then one chat's words a turn.
    w.turn(now);
    w.turn(now);
    w.job(Job::Search { query: "launch".into() });
    assert!(last_found(&seen).2, "two chats of three still to read");
    for _ in 0..3 {
        w.turn(now);
    }
    w.job(Job::Search { query: "launch".into() });
    let (_, hits, indexing, partial) = last_found(&seen);
    assert!(!indexing && !partial);
    assert_eq!(hits.len(), 3);
    assert_eq!(w.wake(now), None, "loaded and signed out: nothing to wake for");
}

#[test]
fn at_most_200_hits_newest_first() {
    let (copy, _store) = copy();
    let msgs: Vec<ChatMessage> = (0..250).map(|i| msg(A, i, &format!("plan number {i}"))).collect();
    copy.upsert(A, &msgs).unwrap();
    let found = hits(&copy, "plan");
    assert_eq!(found.len(), MAX_HITS);
    let want: Vec<(String, String)> = (50..250).rev().map(|i| (A.to_string(), id(i))).collect();
    assert_eq!(found, want);
}

#[test]
fn the_oldest_chats_leave_first_past_the_cap() {
    let (copy, store) = copy();
    copy.upsert(A, &[msg(A, 1, "plan for the old chat")]).unwrap();
    let a_bytes = segment(&store, A).unwrap().len() as u64;
    // Room for about one chat's words.
    copy.index.borrow_mut().cap = Some(a_bytes + a_bytes / 2);
    copy.upsert(B, &[msg(B, 100, "plan for the new chat")]).unwrap();

    let found = copy.found("plan");
    assert_eq!(found, Found { hits: one(B, &id(100)), indexing: false, partial: true });
    assert!(segment(&store, A).is_none(), "the old chat's words left the file");
    // Its name is still found; and a search says it covers recent chats only, after a restart too.
    assert_eq!(hits(&copy, "budget"), one(A, ""));
    drop(copy);
    let copy = open(&store);
    assert!(copy.found("plan").partial);
    assert_eq!(copy.found("plan").hits, one(B, &id(100)));
}

#[test]
fn the_resident_index_stays_within_its_bound() {
    let (copy, store) = copy();
    let words: Vec<String> = (0..300).map(|i| format!("old{i}")).collect();
    copy.upsert(A, &[msg(A, 1, &words.join(" "))]).unwrap();
    let one_chat = copy.index.borrow().resident();
    // Room in memory for about one chat's words; the file's bound far off.
    let bound = one_chat.saturating_add(one_chat / 2);
    copy.index.borrow_mut().resident_cap = Some(bound);
    let words: Vec<String> = (0..300).map(|i| format!("new{i}")).collect();
    copy.upsert(B, &[msg(B, 100, &words.join(" "))]).unwrap();

    assert!(copy.index.borrow().resident() <= bound, "{} past its bound", copy.index.borrow().resident());
    assert_eq!(copy.found("new1"), Found { hits: one(B, &id(100)), indexing: false, partial: true });
    assert_eq!(hits(&copy, "old1"), Vec::<(String, String)>::new());
    assert!(segment(&store, A).is_none(), "the old chat's words left the file too");
}

#[test]
fn the_cap_holds_while_the_index_loads() {
    let store = Store::memory();
    let chats: Vec<String> = (0..4).map(|i| format!("19:load{i}@thread.v2")).collect();
    {
        let (copy, _) = Copy::open(store.clone(), &who()).unwrap();
        for (i, c) in chats.iter().enumerate() {
            copy.put_chat(&ChatRecord::new(chat(c, None))).unwrap();
            copy.upsert(c, &[msg(c, 10 * i64::try_from(i).unwrap(), "the same plan for each chat")]).unwrap();
        }
    }
    let each = segment(&store, &chats[0]).unwrap().len() as u64;
    let (copy, _) = Copy::open(store.clone(), &who()).unwrap();
    // Room for two chats' words: never more held while the index is read back.
    copy.index.borrow_mut().cap = Some(2 * each + each / 2);
    let mut slices = 0;
    while copy.index_step() {
        assert!(copy.index.borrow().bytes() <= 2 * each + each / 2, "{} bytes held after {slices} slices", copy.index.borrow().bytes());
        slices += 1;
        assert!(slices < 100);
    }
    let found = copy.found("plan");
    assert!(found.partial);
    assert_eq!(found.hits, vec![(chats[3].clone(), id(30)), (chats[2].clone(), id(20))], "the newest chats kept");
}

#[test]
fn a_query_reads_only_the_newest_chats_it_needs() {
    let (copy, _store) = copy();
    // 30 chats of 10 messages each, chat i's after chat i - 1's: 300 messages with a word in "p".
    let chats: Vec<String> = (0..30).map(|i| format!("19:many{i:02}@thread.v2")).collect();
    for (c, n) in chats.iter().zip(0i64..) {
        copy.put_chat(&ChatRecord::new(chat(c, None))).unwrap();
        let msgs: Vec<ChatMessage> = (0..10).map(|k| msg(c, 1_000 + n * 10 + k, &format!("plan {k}"))).collect();
        copy.upsert(c, &msgs).unwrap();
    }
    let found = hits(&copy, "p");
    assert_eq!(found.len(), MAX_HITS);
    let want: Vec<(String, String)> = (100..300).rev().map(|j: i64| (chats[usize::try_from(j / 10).unwrap()].clone(), id(1_000 + j))).collect();
    assert_eq!(found, want);
    // The 20 newest chats give the 200 hits; the next one is older than the last of them.
    assert_eq!(copy.index.borrow().visited.get(), 20);
}

/// A word in 3,000 messages of one chat: the search looks at the newest 200 it gives, not at every
/// message that has it (a chat's words never make a search grow with the chat).
#[test]
fn a_search_looks_at_no_more_of_a_chat_than_it_gives() {
    let (copy, _store) = copy();
    for part in (0..3_000).collect::<Vec<i64>>().chunks(1_000) {
        let msgs: Vec<ChatMessage> = part.iter().map(|&i| msg(A, i, &format!("plan number {i}"))).collect();
        copy.upsert(A, &msgs).unwrap();
    }
    let found = hits(&copy, "plan");
    let want: Vec<(String, String)> = (2_800..3_000).rev().map(|i| (A.to_string(), id(i))).collect();
    assert_eq!(found, want);
    assert!(copy.index.borrow().looked.get() <= MAX_HITS, "{} messages looked at", copy.index.borrow().looked.get());
}

/// The history comes after the newest messages, and the place of a message deleted or edited is
/// taken again: whatever order the messages came in, the hits are the newest, and a query of two
/// words gives the messages that have both; read back from the file, the same.
#[test]
fn the_newest_hits_whatever_order_the_messages_came_in() {
    let (copy, store) = copy();
    // The newest first, then the history before them, page after page.
    for start in [900, 700, 500, 300] {
        let len = if start == 900 { 100 } else { 200 };
        copy.upsert(A, &(start..start + len).map(|i| msg(A, i, &format!("plan {i} draft"))).collect::<Vec<_>>()).unwrap();
    }
    // An old one deleted, its place taken by a new one; a newer one edited.
    let mut gone = msg(A, 450, "plan");
    gone.deleted = Some(T0);
    copy.upsert(A, &[gone, msg(A, 1_001, "plan 1001 final"), msg(A, 990, "plan 990 final")]).unwrap();
    let want = |i: &[i64]| i.iter().map(|i| (A.to_string(), id(*i))).collect::<Vec<_>>();
    let plan: Vec<i64> = std::iter::once(1_001).chain((801..1_000).rev()).collect();
    let draft: Vec<i64> = (300..1_000).rev().filter(|i| *i != 990 && *i != 450).take(MAX_HITS).collect();
    for copy in [copy, open(&store)] {
        assert_eq!(hits(&copy, "plan"), want(&plan));
        assert_eq!(hits(&copy, "plan final"), want(&[1_001, 990]), "both words");
        assert_eq!(hits(&copy, "draft plan"), want(&draft));
    }
}

/// A-m3: a segment as the index wrote it before the lists were kept by date (delivery 1), each
/// word's places in the order the places were taken, the newest page first and the history after
/// it: read back, a search gives the newest hits first, 200 at most, two words alike, and looks at
/// no more of the chat than it gives.
#[test]
fn a_segment_kept_by_place_is_read_back_newest_first() {
    let (copy, store) = copy();
    copy.upsert(A, &[msg(A, 1, "plan")]).unwrap();
    drop(copy);
    let taken: Vec<i64> = [(900, 100), (700, 200), (500, 200), (300, 200)].into_iter().flat_map(|(from, n)| from..from + n).collect();
    let messages: Vec<(String, i64)> = taken.iter().map(|&i| (id(i), T0 + i * MINUTE)).collect();
    let places = |has: fn(i64) -> bool| -> Vec<u32> { (0u32..).zip(&taken).filter(|(_, i)| has(**i)).map(|(p, _)| p).collect() };
    let words = serde_json::json!({ "plan": places(|_| true), "draft": places(|i| i % 3 == 0) });
    let old = serde_json::json!({ "chat": A, "messages": messages, "words": words });
    store.write_account_files(TEAMS_ACCOUNT, &[(format!("index/{}.json", chat_hash(A)), serde_json::to_vec(&old).unwrap())]).unwrap();
    let copy = open(&store);
    let want = |i: Vec<i64>| i.into_iter().map(|i| (A.to_string(), id(i))).collect::<Vec<_>>();
    assert_eq!(hits(&copy, "plan"), want((800..1_000).rev().collect()));
    assert!(copy.index.borrow().looked.get() <= MAX_HITS, "{} messages looked at", copy.index.borrow().looked.get());
    assert_eq!(hits(&copy, "draft plan"), want((300..1_000).rev().filter(|i| i % 3 == 0).take(MAX_HITS).collect()));
}

/// 300 chats of one message each: a search that finds what it may give goes through the newest
/// chats it needs, by their words, and through the chats whose names have its words, not through
/// every chat. A chat renamed is found by its new name only.
#[test]
fn a_search_goes_through_only_the_chats_it_needs() {
    let (copy, _store) = copy();
    for i in 0..300 {
        let c = format!("19:one{i:03}@thread.v2");
        copy.put_chat(&ChatRecord::new(chat(&c, None))).unwrap();
        copy.upsert(&c, &[msg(&c, 1_000 + i, &format!("plan {i}"))]).unwrap();
    }
    assert_eq!(hits(&copy, "plan").len(), MAX_HITS);
    let through = copy.index.borrow().went_through.get();
    assert!(through <= MAX_HITS + 1, "{through} chats gone through");
    // By its name: "Budget review" renamed "Travel plans".
    assert_eq!(hits(&copy, "budget rev"), one(A, ""));
    copy.put_chat(&ChatRecord::new(chat(A, Some("Travel plans")))).unwrap();
    assert_eq!(hits(&copy, "budget"), Vec::<(String, String)>::new());
    assert_eq!(hits(&copy, "trav pla"), one(A, ""));
}
