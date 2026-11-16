// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What a write puts in the index, in bytes: one new message in a long chat and in a short one,
//! an edit and a deletion, and a history's first copy, page after page.

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
