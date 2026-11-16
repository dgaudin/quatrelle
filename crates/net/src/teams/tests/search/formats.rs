// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The index's files: a chat's tranches written by number and read back, the stub, an old
//! segment read then cut at the chat's first write, a chat's tranches read to its cap, a copy
//! rolled back to the old format and forward again, an old segment past 1,024 tranches, and a
//! store that can't list a chat's tranches. Hostile tranches, tranches without a head, a head
//! that can't be read, and a run parted at a lower number read back whole. The fuzz targets'
//! seeds, each read back as it says.

use super::tranches::{changes, deleted, ids_in, msg_at};
use super::*;
use crate::teams::search::fuzz_index_input;
use crate::teams::store::fuzz_files;
use crate::teams::{fuzz_teams_copy, fuzz_teams_index};

pub(super) const NONE: Vec<(String, String)> = Vec::new();

/// A chat's stub, as this version writes it at the head of a chat's tranches.
fn stub_of(chat: &str) -> (String, Vec<u8>) {
    let stub = serde_json::json!({ "chat": chat, "messages": [], "words": {}, "moved": 2 });
    (format!("index/{}.json", chat_hash(chat)), serde_json::to_vec(&stub).unwrap())
}

/// A tranche of `chat` that says it's number `tranche`, holding these messages and words.
fn tranche_file(chat: &str, tranche: i64, messages: &[(String, i64)], words: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({ "chat": chat, "tranche": tranche, "messages": messages, "words": words })).unwrap()
}

/// [`copy`], on a store whose files can be made to fail.
pub(super) fn failing_copy() -> (Copy, Store) {
    let store = Store::failing();
    let (copy, _) = Copy::open(store.clone(), &who()).unwrap();
    copy.put_chat(&ChatRecord::new(chat(A, Some("Budget review")))).unwrap();
    copy.put_chat(&ChatRecord::new(chat(B, None))).unwrap();
    drop(copy);
    (open(&store), store)
}

/// A chat's words in the old format (one segment), holding message `i` with these words.
fn old_segment(chat: &str, i: i64, words: serde_json::Value) -> (String, Vec<u8>) {
    let old = serde_json::json!({ "chat": chat, "messages": [[id(i), T0 + i * MINUTE]], "words": words });
    (format!("index/{}.json", chat_hash(chat)), serde_json::to_vec(&old).unwrap())
}

#[test]
fn a_segment_of_the_old_format_is_read_then_cut_at_the_first_write() {
    let (copy, store) = copy();
    drop(copy);
    let h = chat_hash(A);
    let messages: Vec<(String, i64)> = (0..2_500).map(|i| (id(i), T0 + i * MINUTE)).collect();
    let even: Vec<u32> = (0u32..2_500).filter(|p| p % 2 == 0).collect();
    let old = serde_json::json!({ "chat": A, "messages": messages, "words": { "plan": (0u32..2_500).collect::<Vec<_>>(), "even": even } });
    store.write_account_files(TEAMS_ACCOUNT, &[(format!("index/{h}.json"), serde_json::to_vec(&old).unwrap())]).unwrap();
    let copy = open(&store);
    let before = (hits(&copy, "plan"), hits(&copy, "even plan"));
    assert_eq!(before.0, (2_300..2_500).rev().map(|i| (A.to_string(), id(i))).collect::<Vec<_>>());
    store.track_changes();
    copy.upsert(A, &[msg(A, 3_000, "lunch")]).unwrap();
    let index: Vec<String> = changes(&store).into_iter().map(|(k, _)| k).filter(|k| k.starts_with("index/")).collect();
    assert_eq!(index, [format!("index/{h}.json"), format!("index/{h}/0.json"), format!("index/{h}/1.json"), format!("index/{h}/2.json")]);
    for copy in [copy, open(&store)] {
        assert_eq!((hits(&copy, "plan"), hits(&copy, "even plan")), before);
        assert_eq!(hits(&copy, "lunch"), one(A, &id(3_000)));
    }
    // The stub, as the old version reads it: an empty segment.
    #[derive(serde::Deserialize)]
    struct OldShape {
        chat: String,
        messages: Vec<(String, i64)>,
        words: std::collections::BTreeMap<String, Vec<u32>>,
    }
    let stub = store.account_file(TEAMS_ACCOUNT, &format!("index/{h}.json")).unwrap().unwrap();
    let read: OldShape = serde_json::from_slice(&stub).unwrap();
    assert!(read.chat == A && read.messages.is_empty() && read.words.is_empty());
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&stub).unwrap()["moved"], 2);
}

#[test]
fn tranches_are_written_by_number_and_read_back() {
    let (copy, store) = copy();
    copy.index.borrow_mut().tranche_cap = Some(8);
    let h = chat_hash(A);
    copy.upsert(A, &(0..20).map(|i| msg(A, i, &format!("plan {i}"))).collect::<Vec<_>>()).unwrap();
    for (n, held) in [(0u32, 8usize), (1, 8), (2, 4)] {
        let bytes = store.account_file(TEAMS_ACCOUNT, &format!("index/{h}/{n}.json")).unwrap().unwrap();
        let kept: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(kept["chat"], A);
        assert_eq!(kept["tranche"], n);
        let messages: Vec<(String, i64)> = serde_json::from_value(kept["messages"].clone()).unwrap();
        assert_eq!(messages.len(), held, "tranche {n}");
        assert!(messages.windows(2).all(|w| w[0].1 <= w[1].1), "tranche {n} by creation time: {messages:?}");
        assert!(messages.iter().all(|(id, _)| !id.is_empty()), "tranche {n} holds a free place");
    }
    assert!(store.account_file(TEAMS_ACCOUNT, &format!("index/{h}/3.json")).unwrap().is_none());
    // Its head, with its first tranches: what tells them from those an old version left.
    let head = store.account_file(TEAMS_ACCOUNT, &format!("index/{h}.json")).unwrap().expect("a chat new to the index gets its stub");
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&head).unwrap()["moved"], 2);
    drop(copy);
    let copy = open(&store);
    let plan = hits(&copy, "plan");
    assert_eq!(plan.len(), 20);
    assert_eq!(plan, (0..20).rev().map(|i| (A.to_string(), id(i))).collect::<Vec<_>>(), "newest first");
}

/// A segment without `moved` beside tranches: the old version wrote it after them. They may hold
/// the words of a message it deleted since, so they go, and the segment is taken.
#[test]
fn an_old_segment_beside_tranches_is_taken_and_they_go() {
    let (copy, store) = copy();
    copy.upsert(A, &[msg(A, 1, "tranche words")]).unwrap();
    drop(copy);
    let h = chat_hash(A);
    assert!(index_files(&store, A).unwrap().contains("tranche"));
    store.write_account_files(TEAMS_ACCOUNT, &[old_segment(A, 7, serde_json::json!({ "old": [0], "words": [0] }))]).unwrap();
    let copy = open(&store);
    assert_eq!(hits(&copy, "old"), one(A, &id(7)));
    assert_eq!(hits(&copy, "tranche"), NONE);
    assert!(copy.found("old").partial);
    assert!(store.snapshot(|p| p.starts_with(&format!("{TEAMS_ACCOUNT}/index/{h}/"))).is_empty());
}

/// Nine tranches of 1.9 MiB: eight are read (15.2 MiB), the ninth would pass the chat's 16 MiB.
#[test]
fn past_16_mb_a_chats_tranches_are_skipped_and_said() {
    let (copy, store) = copy();
    drop(copy);
    let h = chat_hash(A);
    let pad = "p".repeat((19 << 20) / 10);
    // Their head: tranches without one read as a rollback.
    let files: Vec<(String, Vec<u8>)> = (0..9i64)
        .map(|n| {
            let kept =
                serde_json::json!({ "chat": A, "tranche": n, "messages": [[id(n), T0 + n * MINUTE]], "words": { format!("w{n}"): [0] }, "pad": pad });
            (format!("index/{h}/{n}.json"), serde_json::to_vec(&kept).unwrap())
        })
        .chain([stub_of(A)])
        .collect();
    store.write_account_files(TEAMS_ACCOUNT, &files).unwrap();
    let copy = open(&store);
    assert_eq!(hits(&copy, "w7"), one(A, &id(7)));
    assert_eq!(hits(&copy, "w8"), NONE);
    assert!(copy.found("w7").partial);
    copy.upsert(A, &[msg(A, 20, "lunch")]).unwrap();
    assert!(store.account_file(TEAMS_ACCOUNT, &format!("index/{h}/8.json")).unwrap().is_none());
}

/// A's words in tranches, message 1 saying the secret, then the rollback: B's words planted in
/// the old format (message 5) and never written since; the old version deleted message 1 and wrote
/// A's words in its own shape, holding only message 3.
fn rolled_back(copy: Copy, store: &Store) {
    copy.upsert(A, &[msg(A, 1, "the secret plan"), msg(A, 2, "plan b")]).unwrap();
    drop(copy);
    assert!(index_files(store, A).unwrap().contains("secret"));
    let planted = [old_segment(B, 5, serde_json::json!({ "budget": [0], "talk": [0] })), old_segment(A, 3, serde_json::json!({ "lunch": [0] }))];
    store.write_account_files(TEAMS_ACCOUNT, &planted).unwrap();
}

#[test]
fn a_copy_rolled_back_and_forward_keeps_no_deleted_words() {
    let ha = chat_hash(A);
    let (copy, store) = copy();
    rolled_back(copy, &store);
    let copy = open(&store);
    assert_eq!(hits(&copy, "secret"), NONE);
    assert_eq!(hits(&copy, "lunch"), one(A, &id(3)));
    assert!(hits(&copy, "budget").contains(&(B.to_string(), id(5))));
    assert!(copy.found("lunch").partial);
    assert!(store.snapshot(|p| p.starts_with(&format!("{TEAMS_ACCOUNT}/index/{ha}/"))).is_empty());

    // The same where A's tranches can't be removed: never read, emptied by A's next write.
    let (copy, store) = failing_copy();
    rolled_back(copy, &store);
    store.fail(None, Some(&format!("index/{ha}")));
    let copy = open(&store);
    assert_eq!(hits(&copy, "secret"), NONE);
    assert!(index_files(&store, A).unwrap().contains("secret"), "the folder stayed");
    copy.upsert(A, &[msg(A, 4, "dinner")]).unwrap();
    assert!(!index_files(&store, A).unwrap().contains("secret"));
    drop(copy);
    store.fail(None, None);
    assert_eq!(hits(&open(&store), "secret"), NONE);

    // A chat of the old format leaving the index whole while its tranches can't be removed keeps
    // its head: read again, its tranches go once they can.
    let (copy, store) = failing_copy();
    rolled_back(copy, &store);
    store.fail(None, Some(&format!("index/{ha}")));
    let head = |c: &str| store.account_file(TEAMS_ACCOUNT, &format!("index/{}.json", chat_hash(c))).unwrap().map_or(0, |b| b.len() as u64);
    let (copy, _) = Copy::open(store.clone(), &who()).unwrap();
    // Room for B's words and half of A's: A, the oldest, leaves.
    copy.index.borrow_mut().cap = Some(head(B) + head(A) / 2);
    while copy.index_step() {}
    assert!(copy.found("budget").hits.contains(&(B.to_string(), id(5))));
    assert_eq!(copy.found("lunch").hits, NONE, "A left the index");
    assert!(head(A) > 0, "A's head stays while its tranches do");
    drop(copy);
    store.fail(None, None);
    assert_eq!(hits(&open(&store), "secret"), NONE);
}

/// A store that can't list a chat's tranches: read back, the chat isn't searched (`partial`);
/// written, the write fails, so a deletion's words never stay in a tranche the index didn't see.
#[test]
fn a_chat_whose_tranches_cant_be_listed_isnt_written() {
    let (copy, store) = failing_copy();
    copy.index.borrow_mut().tranche_cap = Some(8);
    copy.upsert(A, &(0..16).map(|i| msg(A, i, &format!("plan secret{i}"))).collect::<Vec<_>>()).unwrap();
    drop(copy);
    let h = chat_hash(A);
    assert!(index_files(&store, A).unwrap().contains("secret12"));
    // Only the listing fails: the head's path ends with `.json`.
    store.fail(Some(&format!("index/{h}")), None);
    let copy = open(&store);
    assert_eq!(hits(&copy, "plan"), NONE);
    assert!(copy.found("plan").partial);
    drop(copy);
    // Reopened without loading: A isn't in memory, so the write reads it from the file.
    let (copy, _) = Copy::open(store.clone(), &who()).unwrap();
    let mut gone = msg(A, 12, "");
    gone.deleted = Some(T0 + 99 * MINUTE);
    assert!(copy.upsert(A, std::slice::from_ref(&gone)).is_err());
    store.fail(None, None);
    assert_eq!(copy.upsert(A, std::slice::from_ref(&gone)).unwrap().len(), 1);
    drop(copy);
    assert!(!index_files(&store, A).unwrap().contains("secret12"));
    let copy = open(&store);
    assert_eq!(hits(&copy, "secret12"), NONE);
    assert_eq!(hits(&copy, "plan").len(), 15);
}

/// An old segment cut in memory into more than 1,024 tranches: the newest 1,024 kept, and said.
#[test]
fn an_old_segment_past_1024_tranches_keeps_the_newest() {
    let (copy, store) = copy();
    drop(copy);
    let messages: Vec<(String, i64)> = (0..1_025).map(|i| (id(i), T0 + i * MINUTE)).collect();
    let old =
        serde_json::json!({ "chat": A, "messages": messages, "words": { "plan": (0u32..1_025).collect::<Vec<_>>(), "oldest": [0], "second": [1] } });
    store.write_account_files(TEAMS_ACCOUNT, &[(format!("index/{}.json", chat_hash(A)), serde_json::to_vec(&old).unwrap())]).unwrap();
    let (copy, _) = Copy::open(store.clone(), &who()).unwrap();
    copy.index.borrow_mut().tranche_cap = Some(1);
    while copy.index_step() {}
    assert_eq!(hits(&copy, "oldest"), NONE);
    assert_eq!(hits(&copy, "second"), one(A, &id(1)));
    assert_eq!(hits(&copy, "plan").len(), MAX_HITS);
    assert!(copy.found("plan").partial);
}

/// Tranches planted under A's folder: too big, damaged, another chat's, under another number, past
/// 1,024, under names this version never writes, one holding a message a lower-numbered tranche
/// holds (and one holding nothing else), one holding a message past the next tranche's first.
/// Read back, each is passed over or freed of what it can't hold; A's next write empties what was
/// passed over and writes again what was freed, and leaves alone what it never reads.
#[test]
fn a_hostile_tranche_is_held_to_the_caps() {
    let (copy, store) = copy();
    let h = chat_hash(A);
    copy.upsert(A, &(1..4).map(|i| msg(A, i, &format!("kept {i}"))).collect::<Vec<_>>()).unwrap();
    drop(copy);
    let at = |i: i64| (id(i), T0 + i * MINUTE);
    let huge = serde_json::json!({ "chat": A, "tranche": 1, "messages": [at(10)], "words": { "huge": [0] }, "pad": "h".repeat(2 << 20) });
    let unnamed = tranche_file(A, 5, &[at(60)], serde_json::json!({ "unnamed": [0] }));
    let planted: Vec<(String, Vec<u8>)> = [
        ("1", serde_json::to_vec(&huge).unwrap()),
        ("2", b"{not json".to_vec()),
        ("3", tranche_file(B, 3, &[at(30)], serde_json::json!({ "stolen": [0] }))),
        ("4", tranche_file(A, 9, &[at(40)], serde_json::json!({ "misfiled": [0] }))),
        ("1024", tranche_file(A, 1_024, &[at(45)], serde_json::json!({ "beyond": [0] }))),
        ("05", unnamed.clone()),
        ("x", unnamed.clone()),
        // Message 2 again, at its own time: tranche 0 holds it.
        ("6", tranche_file(A, 6, &[at(2), at(50)], serde_json::json!({ "twice": [0], "fine": [1] }))),
        // Message 71 made at minute 200: past tranche 8's first.
        ("7", tranche_file(A, 7, &[at(70), (id(71), T0 + 200 * MINUTE)], serde_json::json!({ "seventy": [0], "outside": [1] }))),
        ("8", tranche_file(A, 8, &[at(100)], serde_json::json!({ "hundred": [0] }))),
        // Message 3 alone, again: left holding nothing, an orphan.
        ("9", tranche_file(A, 9, &[at(3)], serde_json::json!({ "thrice": [0] }))),
    ]
    .into_iter()
    .map(|(n, bytes)| (format!("index/{h}/{n}.json"), bytes))
    .collect();
    store.write_account_files(TEAMS_ACCOUNT, &planted).unwrap();
    let check = |copy: &Copy| {
        for word in ["huge", "stolen", "misfiled", "beyond", "unnamed", "twice", "outside", "thrice"] {
            assert_eq!(hits(copy, word), NONE, "{word}");
        }
        assert_eq!(hits(copy, "kept").len(), 3);
        assert_eq!(hits(copy, "fine"), one(A, &id(50)));
        assert_eq!(hits(copy, "seventy"), one(A, &id(70)));
        assert_eq!(hits(copy, "hundred"), one(A, &id(100)));
    };
    let copy = open(&store);
    check(&copy);

    copy.upsert(A, &[msg(A, 300, "lunch")]).unwrap();
    let file = |n: &str| store.account_file(TEAMS_ACCOUNT, &format!("index/{h}/{n}.json")).unwrap();
    for n in ["1", "2", "3", "4", "9", "1024"] {
        assert_eq!(file(n), None, "{n}.json");
    }
    let quoted = |i: i64| format!("\"{}\"", id(i));
    let six = String::from_utf8(file("6").unwrap()).unwrap();
    assert!(six.contains("fine") && !six.contains("twice") && !six.contains(&quoted(2)), "{six}");
    let seven = String::from_utf8(file("7").unwrap()).unwrap();
    assert!(seven.contains("seventy") && !seven.contains("outside") && !seven.contains(&quoted(71)), "{seven}");
    assert_eq!(file("05"), Some(unnamed.clone()));
    assert_eq!(file("x"), Some(unnamed));
    for copy in [copy, open(&store)] {
        check(&copy);
        assert_eq!(hits(&copy, "lunch"), one(A, &id(300)));
    }
}

/// Rolled back, the old version deleted the secret, then let A's words go past its own cap: it
/// takes the head away (the stub is A's segment to it), and leaves A's tranches without one.
/// Forward again, tranches without a head read as a rollback: they go (orphans when they can't),
/// and the deleted message's words with them.
#[test]
fn tranches_without_a_head_are_taken_for_a_rollback() {
    let h = chat_hash(A);
    let rolled_back = |copy: Copy, store: &Store| {
        // 0.json holds message 1, 1.json the secret.
        copy.index.borrow_mut().tranche_cap = Some(1);
        copy.upsert(A, &[msg(A, 1, "plan a"), msg(A, 2, "the secret plan")]).unwrap();
        drop(copy);
        assert!(index_files(store, A).unwrap().contains("secret"));
        store.remove_account_files(TEAMS_ACCOUNT, &format!("index/{h}.json")).unwrap();
    };
    let head = |store: &Store| store.account_file(TEAMS_ACCOUNT, &format!("index/{h}.json")).unwrap();
    let (copy, store) = copy();
    rolled_back(copy, &store);
    let copy = open(&store);
    assert_eq!(hits(&copy, "secret"), NONE);
    assert_eq!(hits(&copy, "plan"), NONE, "read again from what's left: nothing");
    assert!(copy.found("plan").partial);
    assert!(index_files(&store, A).is_none(), "its tranches went");
    // Its next write puts its head back with its first tranche.
    copy.upsert(A, &[msg(A, 4, "dinner")]).unwrap();
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&head(&store).unwrap()).unwrap()["moved"], 2);
    drop(copy);
    let copy = open(&store);
    assert_eq!(hits(&copy, "dinner"), one(A, &id(4)));
    assert_eq!(hits(&copy, "secret"), NONE);

    // The same where A's tranches can't be removed: orphans, the secret's emptied by A's next write.
    let (copy, store) = failing_copy();
    rolled_back(copy, &store);
    store.fail(None, Some(&format!("index/{h}")));
    let copy = open(&store);
    assert_eq!(hits(&copy, "secret"), NONE);
    assert!(index_files(&store, A).unwrap().contains("secret"), "the folder stayed");
    copy.upsert(A, &[msg(A, 4, "dinner")]).unwrap();
    assert!(!index_files(&store, A).unwrap().contains("secret"));
    assert!(head(&store).is_some());
    drop(copy);
    store.fail(None, None);
    let copy = open(&store);
    assert_eq!(hits(&copy, "secret"), NONE);
    assert_eq!(hits(&copy, "dinner"), one(A, &id(4)));
}

/// I1: a run longer than a tranche, parted after newer messages went in with it: the piece with
/// the newer ones takes a lower number than the run's first piece, and begins in the same
/// millisecond. Read back, tranches are ordered by first time, then newest: the newer ones stay
/// where `route` finds them, never freed as past the next tranche's first.
#[test]
fn a_run_longer_than_a_tranche_keeps_its_later_messages() {
    let (copy, store) = copy();
    copy.index.borrow_mut().tranche_cap = Some(8);
    let h = chat_hash(A);
    let t = T0 + 100 * MINUTE;
    let run = |k: usize| msg_at(A, &format!("run-{k:02}"), t, "run");
    copy.upsert(A, &(0..8).map(|i| msg(A, i, "old")).collect::<Vec<_>>()).unwrap();
    copy.upsert(A, &(0..3).map(run).collect::<Vec<_>>()).unwrap();
    // The old ones deleted: number 0 free, the run's three in tranche 1.
    copy.upsert(A, &(0..8).map(|i| deleted(msg(A, i, ""), 200)).collect::<Vec<_>>()).unwrap();
    copy.upsert(A, &[msg_at(A, "later-1", t + 1, "later"), msg_at(A, "later-2", t + 2, "later")]).unwrap();
    copy.upsert(A, &(3..12).map(run).collect::<Vec<_>>()).unwrap();
    let ids = |n: u32| ids_in(&String::from_utf8(store.account_file(TEAMS_ACCOUNT, &format!("index/{h}/{n}.json")).unwrap().unwrap()).unwrap());
    assert_eq!(ids(1), (0..8).map(|k| format!("run-{k:02}")).collect::<Vec<_>>(), "the run's first piece keeps its number");
    let mut rest = ids(0);
    rest.sort_unstable();
    assert_eq!(rest, ["later-1", "later-2", "run-08", "run-09", "run-10", "run-11"], "the rest takes the lowest free");
    let later = vec![(A.to_string(), "later-2".to_string()), (A.to_string(), "later-1".to_string())];
    for copy in [copy, open(&store)] {
        assert_eq!(hits(&copy, "later"), later);
        assert_eq!(hits(&copy, "run").len(), 12);
    }
}

/// Review Focus 5: an old copy's head past SEGMENT_MAX, or damaged: passed over and said, the stub
/// in its place at the chat's first write, and its tranches kept at every later start.
#[test]
fn an_old_segment_too_big_to_read_is_replaced_by_the_stub() {
    let (copy, store) = copy();
    drop(copy);
    let (ha, hb) = (chat_hash(A), chat_hash(B));
    let old = serde_json::json!({ "chat": A, "messages": [[id(1), T0 + MINUTE]], "words": { "lost": [0] }, "pad": "p".repeat(16 << 20) });
    store.write_account_files(TEAMS_ACCOUNT, &[(format!("index/{ha}.json"), serde_json::to_vec(&old).unwrap())]).unwrap();
    let copy = open(&store);
    assert_eq!(copy.found("lost"), Found { hits: vec![], indexing: false, partial: true });
    store.track_changes();
    copy.upsert(A, &[msg(A, 2, "lunch")]).unwrap();
    let mut index: Vec<(String, Vec<u8>)> = store
        .take_changes()
        .into_iter()
        .filter_map(|(k, b)| Some((k.strip_prefix(&format!("{TEAMS_ACCOUNT}/"))?.to_string(), b?)))
        .filter(|(k, _)| k.starts_with("index/"))
        .collect();
    index.sort_unstable();
    let keys: Vec<&str> = index.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys, [format!("index/{ha}.json"), format!("index/{ha}/0.json")]);
    let stub = &index[0].1;
    assert!(stub.len() < 200, "{} bytes", stub.len());
    assert_eq!(serde_json::from_slice::<serde_json::Value>(stub).unwrap()["moved"], 2);
    drop(copy);
    for _ in 0..2 {
        let copy = open(&store);
        assert_eq!(hits(&copy, "lunch"), one(A, &id(2)));
        assert_eq!(store.account_files(TEAMS_ACCOUNT, &format!("index/{ha}")).unwrap(), ["0.json"]);
    }

    // B's head damaged beside its tranches: they go, and it's said (the copy's "partial" set aside
    // to see B's own).
    let copy = open(&store);
    copy.upsert(B, &[msg(B, 5, "budget talk")]).unwrap();
    drop(copy);
    store.write_account_files(TEAMS_ACCOUNT, &[(format!("index/{hb}.json"), b"{not json".to_vec())]).unwrap();
    store.remove_account_files(TEAMS_ACCOUNT, "index/partial").unwrap();
    let copy = open(&store);
    assert_eq!(hits(&copy, "talk"), NONE);
    assert!(copy.found("talk").partial);
    assert!(index_files(&store, B).is_none(), "B's tranches went");
    // What was written since A's write, the load's "partial" too, set aside.
    changes(&store);
    copy.upsert(B, &[msg(B, 6, "dinner talk")]).unwrap();
    let keys: Vec<String> = changes(&store).into_iter().map(|(k, _)| k).filter(|k| k.starts_with("index/")).collect();
    assert_eq!(keys, [format!("index/{hb}.json"), format!("index/{hb}/0.json")]);
    drop(copy);
    let copy = open(&store);
    assert_eq!(hits(&copy, "talk"), one(B, &id(6)));
    assert_eq!(hits(&copy, "lunch"), one(A, &id(2)));
}

/// The `teams_store` seeds of the index, and `teams_index`'s.
const LEGACY_SEGMENT: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fuzz/seeds/teams_store/legacy-segment"));
const TRANCHES: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fuzz/seeds/teams_store/tranches"));
const ROLLED_BACK: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fuzz/seeds/teams_store/rolled-back"));
const TRANCHE: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fuzz/seeds/teams_index/tranche"));

/// The fuzz entries' seeds, each read back as it says before the entry runs on it: `teams_store`'s
/// an old segment, tranches behind the stub, an old segment beside a tranche (a rollback);
/// `teams_index`'s A's tranche 0 of three messages, two of one millisecond. Each passes its entry,
/// and so does junk, a tranche holding an id twice, and one whose message is held already.
#[test]
fn the_fuzz_entries_reach_the_tranches() {
    let h = chat_hash(A);
    // A seed's files in a store, the copy opened as the entry opens it and its index read back.
    let opened = |seed: &[u8]| {
        let (chat, files, message) = fuzz_files(seed);
        assert_eq!(chat, A);
        assert!(message.is_some_and(|m| serde_json::from_slice::<ChatMessage>(m).is_ok()), "a message to write");
        let store = Store::memory_from(files);
        let (copy, opened) = Copy::open(store.clone(), &Identity { tenant_id: "t-1".into(), object_id: "u-1".into() }).unwrap();
        assert_eq!(opened, Opened::Same, "the seed's identity");
        while copy.index_step() {}
        (copy, store)
    };
    let head = |store: &Store| -> serde_json::Value {
        serde_json::from_slice(&store.account_file(TEAMS_ACCOUNT, &format!("index/{h}.json")).unwrap().unwrap()).unwrap()
    };
    let both = vec![(A.to_string(), "2".to_string()), (A.to_string(), "1".to_string())];

    {
        let (seeded, store) = opened(LEGACY_SEGMENT);
        assert_eq!(hits(&seeded, "plan"), both, "the old segment's two messages");
        assert!(!seeded.found("plan").partial);
        assert!(head(&store).get("moved").is_none() && index_files(&store, A).is_none(), "the old format");
    }
    {
        let (seeded, store) = opened(TRANCHES);
        assert_eq!(hits(&seeded, "budget"), one(A, "1"), "tranche 0's words");
        assert_eq!(hits(&seeded, "lunch"), one(A, "2"), "tranche 1's words");
        assert_eq!(hits(&seeded, "plan"), both);
        assert!(!seeded.found("plan").partial);
        assert_eq!(head(&store)["moved"], 2);
        let mut names = store.account_files(TEAMS_ACCOUNT, &format!("index/{h}")).unwrap();
        names.sort_unstable();
        assert_eq!(names, ["0.json", "1.json"]);
    }
    {
        let (seeded, store) = opened(ROLLED_BACK);
        assert!(seeded.found("plan").partial, "a rollback said");
        assert!(index_files(&store, A).is_none(), "the tranche from before the rollback went");
        assert_eq!(hits(&seeded, "secret"), NONE);
        assert_eq!(hits(&seeded, "plan"), one(A, "2"), "the old segment's message");
    }

    for seed in [LEGACY_SEGMENT, TRANCHES, ROLLED_BACK] {
        assert_eq!(fuzz_teams_copy(seed), Ok(()));
    }

    // A's tranche 0 of three messages, the last two of one millisecond, and a message made in it
    // too: once the first is taken out, a run of three, cut two a tranche.
    let (chat, tranche, message, control) = fuzz_index_input(TRANCHE);
    assert_eq!(chat, A);
    let kept: serde_json::Value = serde_json::from_slice(tranche).unwrap();
    assert_eq!((&kept["chat"], &kept["tranche"]), (&serde_json::json!(A), &serde_json::json!(0)));
    let times: Vec<i64> = kept["messages"].as_array().unwrap().iter().map(|m| m[1].as_i64().unwrap()).collect();
    assert!(times.len() == 3 && times[1] == times[2] && times[0] < times[1], "{times:?}");
    assert_eq!(serde_json::from_slice::<ChatMessage>(message).unwrap().created, times[2]);
    assert_eq!((control % 3, 1 + control / 3 % 16), (0, 2), "cut from the front, two a tranche");
    // Read back by the copy as A's tranche 0, behind the stub: its three messages found.
    let (copy, store) = copy();
    drop(copy);
    store.write_account_files(TEAMS_ACCOUNT, &[stub_of(A), (format!("index/{h}/0.json"), tranche.to_vec())]).unwrap();
    assert_eq!(hits(&open(&store), "plan").len(), 3);

    for data in [
        TRANCHE,
        b"",
        b"\0\0\0",
        b"19:x\0{\"chat\":\"19:x\",\"tranche\":0,\"messages\":[[\"1\",0],[\"1\",0]],\"words\":{\"a\":[0,1,7]}}",
        // The message under id 1 (none is given) is held already: its id taken out first.
        b"19:x\0{\"chat\":\"19:x\",\"tranche\":0,\"messages\":[[\"2\",0],[\"1\",0]],\"words\":{}}",
    ] {
        assert_eq!(fuzz_teams_index(data), Ok(()), "{}", String::from_utf8_lossy(data));
    }
}
