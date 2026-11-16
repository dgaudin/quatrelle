// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What a write puts in the index, in bytes: one new message in a long chat and in a short one,
//! an edit and a deletion, and a history's first copy, page after page. What a query reads of a
//! long chat, and the tranches that leave first past the index's bound.

use super::*;

/// Messages `0..n` of `chat`, each about the plan, upserted 1,000 at a time in creation order.
pub(super) fn filled(copy: &Copy, chat: &str, n: i64) {
    for from in (0..n).step_by(1_000) {
        let msgs: Vec<ChatMessage> = (from..n.min(from + 1_000)).map(|i| msg(chat, i, &format!("message {i} about the plan"))).collect();
        copy.upsert(chat, &msgs).unwrap();
    }
}

/// The files changed since the last call, by key (under the copy's folder): each with its bytes'
/// length, `None` for one removed.
pub(super) fn changes(store: &Store) -> Vec<(String, Option<usize>)> {
    let folder = format!("{TEAMS_ACCOUNT}/");
    let mut changed: Vec<(String, Option<usize>)> =
        store.take_changes().into_iter().map(|(k, b)| (k.strip_prefix(&folder).unwrap_or(&k).to_string(), b.map(|b| b.len()))).collect();
    changed.sort_by(|a, b| a.0.cmp(&b.0));
    changed
}

/// The bytes written under `index/`.
pub(super) fn index_bytes(changes: &[(String, Option<usize>)]) -> usize {
    changes.iter().filter(|(k, _)| k.starts_with("index/")).filter_map(|(_, n)| *n).sum()
}

/// The bytes written in the index while messages `0..n` of A are copied as a history is: in pages
/// of 50, the newest page first, then each page before it.
fn history(n: i64) -> usize {
    let (copy, store) = copy();
    store.track_changes();
    let mut written = 0;
    for page in (0..n / 50).rev() {
        let msgs: Vec<ChatMessage> = (page * 50..page * 50 + 50).map(|i| msg(A, i, &format!("message {i} about the plan"))).collect();
        copy.upsert(A, &msgs).unwrap();
        written += index_bytes(&changes(&store));
    }
    written
}

#[test]
#[ignore = "red until a write puts in only its tranches (plan Task 3)"]
fn an_insert_writes_a_bounded_part_of_the_index() {
    let (copy, store) = copy();
    filled(&copy, A, 30_500);
    filled(&copy, B, 3_500);
    store.track_changes();
    copy.upsert(A, &[msg(A, 30_500, "message 30500 about the plan")]).unwrap();
    let long = index_bytes(&changes(&store));
    copy.upsert(B, &[msg(B, 3_500, "message 3500 about the plan")]).unwrap();
    let short = index_bytes(&changes(&store));
    assert!(long <= 256 << 10, "{long} bytes of the index written for one message");
    assert!(long <= 2 * short, "{long} bytes for the long chat, {short} for the short one");
}

#[test]
#[ignore = "red until a write puts in only its tranches (plan Task 3)"]
fn an_edit_and_a_deletion_write_their_tranche_only() {
    let (copy, store) = copy();
    filled(&copy, A, 30_500);
    store.track_changes();
    let mut edited = msg(A, 10, "a secret recipe");
    edited.edited = Some(T0 + 40_000 * MINUTE);
    let mut gone = msg(A, 10, "");
    gone.deleted = Some(T0 + 40_001 * MINUTE);
    for m in [edited, gone] {
        copy.upsert(A, &[m]).unwrap();
        let ch = changes(&store);
        assert_eq!(ch.iter().filter(|(k, _)| k.starts_with("index/")).count(), 1, "{ch:?}");
        assert_eq!(ch.iter().filter(|(k, _)| k.starts_with("msgs/")).count(), 1, "{ch:?}");
        assert!(index_bytes(&ch) <= 256 << 10, "{ch:?}");
    }
    for (key, bytes) in store.snapshot(|p| p.starts_with(&format!("{TEAMS_ACCOUNT}/index/"))) {
        let kept = String::from_utf8_lossy(&bytes);
        assert!(!kept.contains("secret") && !kept.contains(&id(10)), "{key} keeps the deleted message");
    }
}

#[test]
#[ignore = "red until a write puts in only its tranches (plan Task 3)"]
fn the_first_copy_of_a_history_is_linear() {
    let (w3, w6) = (history(3_000), history(6_000));
    assert!(w6 * 10 <= w3 * 22, "{w3} bytes for 3,000 messages, {w6} for 6,000");
}

/// A word in every message of a chat of 30,500: the newest tranche gives the hits, the next one
/// is older than the last of them.
#[test]
fn a_query_reads_only_the_newest_tranches() {
    let (copy, _store) = copy();
    filled(&copy, A, 30_500);
    assert_eq!(hits(&copy, "message"), (30_300..30_500).rev().map(|i| (A.to_string(), id(i))).collect::<Vec<_>>());
    let ix = copy.index.borrow();
    assert!(ix.visited.get() <= 2, "{} tranches visited", ix.visited.get());
    assert!(ix.looked.get() <= MAX_HITS, "{} messages looked at", ix.looked.get());
}

#[test]
fn the_oldest_tranches_leave_first_across_chats() {
    let (copy, store) = copy();
    copy.index.borrow_mut().tranche_cap = Some(8);
    let (ha, hb) = (chat_hash(A), chat_hash(B));
    copy.upsert(A, &(0..24).map(|i| msg(A, i, &format!("plan {i:03}"))).collect::<Vec<_>>()).unwrap();
    copy.upsert(B, &(100..108).map(|i| msg(B, i, &format!("plan {i:03}"))).collect::<Vec<_>>()).unwrap();
    let file = |h: &str, n: u32| store.account_file(TEAMS_ACCOUNT, &format!("index/{h}/{n}.json")).unwrap().map(|b| b.len() as u64);
    let all: u64 = [file(&ha, 0), file(&ha, 1), file(&ha, 2), file(&hb, 0)].into_iter().map(Option::unwrap).sum();
    copy.index.borrow_mut().cap = Some(all - file(&ha, 0).unwrap() / 2);
    // B's message 100 edited to a text of the same length: B doesn't grow.
    let mut edited = msg(B, 100, "plan 900");
    edited.edited = Some(T0 + 200 * MINUTE);
    copy.upsert(B, &[edited]).unwrap();

    assert_eq!(file(&ha, 0), None, "A's oldest tranche left");
    assert!(file(&ha, 1).is_some() && file(&ha, 2).is_some());
    let found = copy.found("plan");
    let of_a: Vec<String> = found.hits.iter().filter(|(c, _)| c == A).map(|(_, m)| m.clone()).collect();
    assert_eq!(of_a, (8..24).rev().map(id).collect::<Vec<_>>());
    assert!(found.partial);
}

/// A chat of the old format is cut in memory only: it leaves the index whole, its head with it.
#[test]
fn a_chat_of_the_old_format_leaves_whole() {
    let (copy, store) = copy();
    copy.upsert(B, &(100..108).map(|i| msg(B, i, &format!("new plan {i}"))).collect::<Vec<_>>()).unwrap();
    drop(copy);
    let (ha, hb) = (chat_hash(A), chat_hash(B));
    let messages: Vec<(String, i64)> = (0..24).map(|i| (id(i), T0 + i * MINUTE)).collect();
    let old = serde_json::json!({ "chat": A, "messages": messages, "words": { "old": (0u32..24).collect::<Vec<_>>() } });
    store.write_account_files(TEAMS_ACCOUNT, &[(format!("index/{ha}.json"), serde_json::to_vec(&old).unwrap())]).unwrap();
    let len = |key: String| store.account_file(TEAMS_ACCOUNT, &key).unwrap().unwrap().len() as u64;
    let (a, b) = (len(format!("index/{ha}.json")), len(format!("index/{hb}/0.json")));
    let (copy, _) = Copy::open(store.clone(), &who()).unwrap();
    copy.index.borrow_mut().tranche_cap = Some(8);
    copy.index.borrow_mut().cap = Some(a + b - b / 2);
    let mut slices = 0;
    while copy.index_step() {
        slices += 1;
        assert!(slices < 100);
    }

    assert!(store.account_file(TEAMS_ACCOUNT, &format!("index/{ha}.json")).unwrap().is_none(), "A's head left with it");
    assert_eq!(hits(&copy, "old"), Vec::<(String, String)>::new());
    assert_eq!(hits(&copy, "new").len(), 8);
    assert!(copy.found("new").partial);
}
