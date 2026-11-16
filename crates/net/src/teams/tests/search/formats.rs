// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The index's files: a chat's tranches written by number and read back, the stub, an old
//! segment read then cut at the chat's first write, a chat's tranches read to its cap, a copy
//! rolled back to the old format and forward again, an old segment past 1,024 tranches, and a
//! store that can't list a chat's tranches.

use super::tranches::changes;
use super::*;

const NONE: Vec<(String, String)> = Vec::new();

/// [`copy`], on a store whose files can be made to fail.
fn failing_copy() -> (Copy, Store) {
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
    assert!(store.account_file(TEAMS_ACCOUNT, &format!("index/{h}.json")).unwrap().is_none(), "a chat new to the index gets no stub");
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
    let files: Vec<(String, Vec<u8>)> = (0..9i64)
        .map(|n| {
            let kept =
                serde_json::json!({ "chat": A, "tranche": n, "messages": [[id(n), T0 + n * MINUTE]], "words": { format!("w{n}"): [0] }, "pad": pad });
            (format!("index/{h}/{n}.json"), serde_json::to_vec(&kept).unwrap())
        })
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
