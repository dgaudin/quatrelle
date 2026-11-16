// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What a write puts in the index, in bytes: one new message in a long chat and in a short one,
//! an edit and a deletion, and a history's first copy, page after page. Where a message goes among
//! a chat's tranches, and only those it changes written: a run of one millisecond, a creation time
//! moved, a chat copied again, a tranche emptied, a chat past its tranches or its bytes. What a
//! query reads of a long chat, and the tranches that leave first past the index's bound, then
//! written again.

use super::formats::{NONE, failing_copy};
use super::*;
use crate::teams::search::{Goes, Span, candidates, route};

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

/// A chat's tranches as the file keeps them, by number.
fn tranche_files(store: &Store, chat: &str) -> Vec<String> {
    let folder = format!("index/{}", chat_hash(chat));
    let mut names: Vec<(u32, String)> = store
        .account_files(TEAMS_ACCOUNT, &folder)
        .unwrap()
        .into_iter()
        .filter_map(|name| Some((name.strip_suffix(".json")?.parse().ok()?, name)))
        .collect();
    names.sort_unstable();
    names
        .into_iter()
        .map(|(_, name)| String::from_utf8(store.account_file(TEAMS_ACCOUNT, &format!("{folder}/{name}")).unwrap().unwrap()).unwrap())
        .collect()
}

/// The ids of the messages a tranche's file holds.
pub(super) fn ids_in(file: &str) -> Vec<String> {
    let kept: serde_json::Value = serde_json::from_str(file).unwrap();
    kept["messages"].as_array().unwrap().iter().map(|m| m[0].as_str().unwrap().to_string()).filter(|id| !id.is_empty()).collect()
}

/// How many files under `index/` hold this message's id.
fn files_holding(store: &Store, message: &str) -> usize {
    let quoted = format!("\"{message}\"");
    store.snapshot(|p| p.starts_with(&format!("{TEAMS_ACCOUNT}/index/"))).iter().filter(|(_, b)| String::from_utf8_lossy(b).contains(&quoted)).count()
}

/// A message of `chat` under this id, made at `at`, saying `says`.
pub(super) fn msg_at(chat: &str, id: &str, at: i64, says: &str) -> ChatMessage {
    ChatMessage { id: id.into(), created: at, modified: at, ..msg(chat, 0, says) }
}

/// A message's words changed (`edited`) or the message deleted, at minute `at`.
fn edited(mut m: ChatMessage, at: i64) -> ChatMessage {
    m.edited = Some(T0 + at * MINUTE);
    m
}

pub(super) fn deleted(mut m: ChatMessage, at: i64) -> ChatMessage {
    m.deleted = Some(T0 + at * MINUTE);
    m
}

/// The `index/` keys changed since the last call.
fn index_keys(store: &Store) -> Vec<String> {
    changes(store).into_iter().map(|(k, _)| k).filter(|k| k.starts_with("index/")).collect()
}

#[test]
fn route_and_candidates_follow_the_first_times() {
    let s = |first, newest, held| Span { first, newest, held };
    let spans = [s(10, 19, 8), s(20, 29, 8), s(30, 35, 3)];
    assert_eq!(route(&[], 5, 8), Goes::After);
    assert_eq!(route(&spans, 5, 8), Goes::Before);
    assert_eq!(route(&spans, 15, 8), Goes::Into(0));
    assert_eq!(route(&spans, 20, 8), Goes::Into(1));
    assert_eq!(route(&spans, 40, 8), Goes::Into(2), "the last isn't full");
    let full = [s(10, 19, 8), s(20, 29, 8), s(30, 35, 8)];
    assert_eq!(route(&full, 40, 8), Goes::After);
    assert_eq!(route(&full, 35, 8), Goes::Into(2), "its newest's millisecond");
    assert_eq!(route(&full, 10, 8), Goes::Into(0), "the first's first millisecond");
    assert_eq!(candidates(&spans, 5), 0..0);
    assert_eq!(candidates(&spans, 20), 0..2);
    assert_eq!(candidates(&spans, 25), 1..2);
    assert_eq!(candidates(&spans, 30), 1..3);
}

#[test]
fn a_new_message_opens_a_tranche_past_a_full_one() {
    let (copy, store) = copy();
    copy.index.borrow_mut().tranche_cap = Some(8);
    let h = chat_hash(A);
    copy.upsert(A, &(10..18).map(|i| msg(A, i, &format!("plan {i}"))).collect::<Vec<_>>()).unwrap();
    assert_eq!(store.account_files(TEAMS_ACCOUNT, &format!("index/{h}")).unwrap(), ["0.json"]);
    store.track_changes();
    copy.upsert(A, &[msg(A, 20, "plan 20")]).unwrap();
    assert_eq!(index_keys(&store), [format!("index/{h}/1.json")], "newer than all, past a full tranche");
    copy.upsert(A, &[msg(A, 1, "plan 1")]).unwrap();
    assert_eq!(index_keys(&store), [format!("index/{h}/2.json")], "older than all, before a full tranche");
    copy.upsert(A, &[edited(msg(A, 15, "plan 15 again"), 30)]).unwrap();
    assert_eq!(index_keys(&store), [format!("index/{h}/0.json")], "an edit writes its tranche alone");
    // The first of its tranche too: it goes back where it was, not to the tranche before.
    copy.upsert(A, &[edited(msg(A, 10, "plan 10 again"), 31)]).unwrap();
    assert_eq!(index_keys(&store), [format!("index/{h}/0.json")], "the first of its tranche, edited");
    let want: Vec<(String, String)> = std::iter::once(20).chain((10..18).rev()).chain([1]).map(|i| (A.to_string(), id(i))).collect();
    for copy in [copy, open(&store)] {
        assert_eq!(hits(&copy, "plan"), want);
        assert_eq!(hits(&copy, "again"), vec![(A.to_string(), id(15)), (A.to_string(), id(10))]);
    }
}

/// Graph sends a chat's messages newest first: past a full tranche, a batch of new ones opens one
/// tranche, and the full one isn't written again.
#[test]
fn a_batch_newest_first_opens_one_tranche_past_a_full_one() {
    let (copy, store) = copy();
    copy.index.borrow_mut().tranche_cap = Some(8);
    let h = chat_hash(A);
    copy.upsert(A, &(0..8).map(|i| msg(A, i, &format!("plan {i}"))).collect::<Vec<_>>()).unwrap();
    store.track_changes();
    copy.upsert(A, &[10, 9, 8].map(|i| msg(A, i, &format!("plan {i}")))).unwrap();
    assert_eq!(index_keys(&store), [format!("index/{h}/1.json")]);
    let file = store.account_file(TEAMS_ACCOUNT, &format!("index/{h}/1.json")).unwrap().unwrap();
    assert_eq!(ids_in(&String::from_utf8(file).unwrap()), [id(8), id(9), id(10)]);
    let want: Vec<(String, String)> = [10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0].map(|i| (A.to_string(), id(i))).to_vec();
    assert_eq!(hits(&open(&store), "plan"), want);
}

/// Review Focus 1: a bot's burst at a tranche's edge stays whole, unless it's longer than a
/// tranche; each of its messages found, edited and deleted wherever it went, after a restart too.
#[test]
fn a_run_of_one_millisecond_stays_in_one_tranche() {
    let (copy, store) = copy();
    copy.index.borrow_mut().tranche_cap = Some(8);
    let t6 = T0 + 6 * MINUTE;
    let run = |k: usize| format!("run-{k}");
    let mut msgs: Vec<ChatMessage> = (0..7).map(|i| msg(A, i, &format!("plan {i}"))).collect();
    msgs.extend((0..6).map(|k| msg_at(A, &run(k), t6, &format!("burst {k}"))));
    copy.upsert(A, &msgs).unwrap();
    let files = tranche_files(&store, A);
    assert_eq!(files.len(), 2, "{files:?}");
    let with_six = |store: &Store| {
        let mut held = tranche_files(store, A).iter().map(|f| ids_in(f)).find(|ids| ids.contains(&id(6))).unwrap();
        held.sort_unstable();
        held
    };
    let mut want: Vec<String> = std::iter::once(id(6)).chain((0..6).map(run)).collect();
    want.sort_unstable();
    assert_eq!(with_six(&store), want, "message 6 and the six runs in one tranche");
    let runs = |ks: &[usize]| ks.iter().map(|k| (A.to_string(), run(*k))).collect::<Vec<_>>();
    assert_eq!(hits(&copy, "burst"), runs(&[0, 1, 2, 3, 4, 5]), "one time: by id");

    copy.upsert(A, &[deleted(msg_at(A, &run(3), t6, ""), 50), edited(msg_at(A, &run(0), t6, "burst final"), 50)]).unwrap();
    assert_eq!(hits(&copy, "burst"), runs(&[0, 1, 2, 4, 5]));
    assert_eq!(hits(&copy, "final"), runs(&[0]));

    // A run longer than a tranche, in one later millisecond: parted, and only it.
    let tr = T0 + 100 * MINUTE;
    let ring = |k: usize| format!("ring-{k:02}");
    copy.upsert(A, &(0..20).map(|k| msg_at(A, &ring(k), tr, &format!("ring {k}"))).collect::<Vec<_>>()).unwrap();
    let mut rings: Vec<usize> =
        tranche_files(&store, A).iter().map(|f| ids_in(f).iter().filter(|id| id.starts_with("ring-")).count()).filter(|n| *n > 0).collect();
    rings.sort_unstable();
    assert_eq!(rings, [4, 8, 8]);
    let mut want: Vec<String> = std::iter::once(id(6)).chain([0, 1, 2, 4, 5].map(run)).collect();
    want.sort_unstable();
    assert_eq!(with_six(&store), want);
    copy.upsert(A, &[deleted(msg_at(A, &ring(10), tr, ""), 150), edited(msg_at(A, &ring(17), tr, "ring 17 changed"), 150)]).unwrap();

    let check = |copy: &Copy| {
        assert_eq!(hits(copy, "burst"), runs(&[0, 1, 2, 4, 5]));
        assert_eq!(hits(copy, "final"), runs(&[0]));
        assert_eq!(hits(copy, "burst 3"), NONE);
        let all: Vec<(String, String)> = (0..20).filter(|k| *k != 10).map(|k| (A.to_string(), ring(k))).collect();
        assert_eq!(hits(copy, "ring"), all);
        assert_eq!(hits(copy, "ring 10"), NONE);
        assert_eq!(hits(copy, "changed"), one(A, &ring(17)));
    };
    check(&copy);
    drop(copy);
    check(&open(&store));
    assert_eq!(with_six(&store), want);
}

/// Review Focus 3: a creation time Graph moved on an edit, near (its pages find it) and far
/// (they take it for a new one): held once by the index, by its new words only.
#[test]
fn a_message_whose_creation_time_moved_is_held_once() {
    let (copy, store) = copy();
    copy.index.borrow_mut().tranche_cap = Some(8);
    // Four pages: 0..100, 100..200, 200..300, 300..350.
    filled(&copy, A, 350);
    let mut near = edited(msg(A, 5, "near moved"), 400);
    near.created += 1;
    let mut far = edited(msg(A, 10, "far moved"), 400);
    far.created = T0 + 300 * MINUTE + 1;
    let (_, new) = copy.upsert_new(A, &[near, far]).unwrap();
    assert_eq!(new, std::collections::HashSet::from([id(10)]), "past its pages, taken for a new one");

    let check = |copy: &Copy| {
        assert_eq!(hits(copy, "moved"), vec![(A.to_string(), id(10)), (A.to_string(), id(5))]);
        assert!(!hits(copy, "message 10").contains(&(A.to_string(), id(10))), "its old words went");
        assert!(!hits(copy, "message 5").contains(&(A.to_string(), id(5))));
        assert_eq!(files_holding(&store, &id(10)), 1);
        assert_eq!(files_holding(&store, &id(5)), 1);
    };
    check(&copy);
    drop(copy);
    check(&open(&store));
}

/// Review Focus 3: a chat whose pages start again while its index stays, copied again with new
/// words: each message held once, by its new words only. A run of one millisecond cut over three
/// tranches sends its messages, copied again, to the last of them: each is taken out of the one
/// that held it.
#[test]
fn a_chat_copied_again_from_its_start_is_indexed_once() {
    let (copy, store) = copy();
    copy.index.borrow_mut().tranche_cap = Some(8);
    let t = T0 + 10 * MINUTE;
    let ring = |k: usize| format!("ring-{k:02}");
    let ids: Vec<String> = (0..4).map(id).chain((0..20).map(ring)).collect();
    let made = |says: &str| -> Vec<ChatMessage> {
        (0..4).map(|i| msg(A, i, &format!("{says} {i}"))).chain((0..20).map(|k| msg_at(A, &ring(k), t, &format!("{says} {k}")))).collect()
    };
    copy.upsert(A, &made("ring")).unwrap();
    let held: Vec<usize> = tranche_files(&store, A).iter().map(|f| ids_in(f).len()).collect();
    assert_eq!(held, [4, 8, 8, 4]);
    // Its pages don't hold together: its copy starts again.
    let mut r = copy.chat(A).unwrap();
    r.pages = vec![T0];
    r.page_keys = vec![];
    store.write_account_files(TEAMS_ACCOUNT, &[(format!("chats/{}.json", chat_hash(A)), serde_json::to_vec(&r).unwrap())]).unwrap();
    let (_, new) = copy.upsert_new(A, &made("copied")).unwrap();
    assert_eq!(new.len(), 24, "every message new to the pages");

    let check = |copy: &Copy| {
        for m in &ids {
            assert_eq!(files_holding(&store, m), 1, "{m}");
        }
        assert_eq!(hits(copy, "ring"), NONE, "the old words went");
        assert_eq!(hits(copy, "copied").len(), 24);
    };
    check(&copy);
    drop(copy);
    check(&open(&store));
}

#[test]
fn a_tranche_emptied_is_removed() {
    let (copy, store) = copy();
    copy.index.borrow_mut().tranche_cap = Some(8);
    let h = chat_hash(A);
    copy.upsert(A, &(0..16).map(|i| msg(A, i, &format!("plan {i}"))).collect::<Vec<_>>()).unwrap();
    assert!(store.account_file(TEAMS_ACCOUNT, &format!("index/{h}/1.json")).unwrap().is_some());
    store.track_changes();
    copy.upsert(A, &(8..16).map(|i| deleted(msg(A, i, ""), 99)).collect::<Vec<_>>()).unwrap();
    let index: Vec<(String, Option<usize>)> = changes(&store).into_iter().filter(|(k, _)| k.starts_with("index/")).collect();
    assert_eq!(index, [(format!("index/{h}/1.json"), None)], "removed; 0.json not written");
    assert!((8..16).all(|i| files_holding(&store, &id(i)) == 0));
    drop(copy);
    assert_eq!(hits(&open(&store), "plan"), (0..8).rev().map(|i| (A.to_string(), id(i))).collect::<Vec<_>>());
}

#[test]
fn past_1024_tranches_the_oldest_go() {
    let (copy, store) = copy();
    copy.index.borrow_mut().tranche_cap = Some(1);
    let h = chat_hash(A);
    copy.upsert(A, &(0..1_025).map(|i| msg(A, i, &format!("plan {i}"))).collect::<Vec<_>>()).unwrap();
    let count = || store.account_files(TEAMS_ACCOUNT, &format!("index/{h}")).unwrap().len();
    assert_eq!(count(), 1_024);
    assert_eq!(hits(&copy, "plan 0"), NONE);
    assert!(copy.found("plan").partial);
    copy.upsert(A, &[msg(A, 1_025, "plan 1025")]).unwrap();
    assert_eq!(count(), 1_024);
    let plan_1 = hits(&copy, "plan 1");
    assert!(!plan_1.contains(&(A.to_string(), id(1))), "{plan_1:?}");
    assert!(plan_1.contains(&(A.to_string(), id(1_025))));
}

#[test]
fn past_its_bytes_a_chat_loses_its_oldest_tranches() {
    let (copy, store) = copy();
    copy.index.borrow_mut().tranche_cap = Some(8);
    let h = chat_hash(A);
    copy.upsert(A, &(0..8).map(|i| msg(A, i, &format!("plan {i}"))).collect::<Vec<_>>()).unwrap();
    let first = store.account_file(TEAMS_ACCOUNT, &format!("index/{h}/0.json")).unwrap().unwrap().len();
    copy.index.borrow_mut().chat_cap = Some(3 * first);
    for from in (8..40).step_by(8) {
        copy.upsert(A, &(from..from + 8).map(|i| msg(A, i, &format!("plan {i}"))).collect::<Vec<_>>()).unwrap();
    }
    let files = tranche_files(&store, A);
    assert!(files.len() <= 3, "{} files", files.len());
    let bytes: usize = files.iter().map(String::len).sum();
    assert!(bytes <= 3 * first, "{bytes} bytes for {}", 3 * first);
    let plan = hits(&copy, "plan");
    assert!(!plan.contains(&(A.to_string(), id(0))), "the oldest left");
    assert!(plan.contains(&(A.to_string(), id(39))), "the newest stay");
    assert!(copy.found("plan").partial);
}

#[test]
fn the_tranches_just_written_are_spared() {
    let (copy, store) = copy();
    copy.index.borrow_mut().tranche_cap = Some(8);
    let (ha, hb) = (chat_hash(A), chat_hash(B));
    copy.upsert(A, &(0..24).map(|i| msg(A, i, &format!("plan {i}"))).collect::<Vec<_>>()).unwrap();
    copy.upsert(B, &(100..108).map(|i| msg(B, i, &format!("plan {i}"))).collect::<Vec<_>>()).unwrap();
    let file = |h: &str, n: u32| store.account_file(TEAMS_ACCOUNT, &format!("index/{h}/{n}.json")).unwrap();
    let len = |h: &str, n: u32| file(h, n).unwrap().len() as u64;
    let all = len(&ha, 0) + len(&ha, 1) + len(&ha, 2) + len(&hb, 0);
    copy.index.borrow_mut().cap = Some(all - len(&ha, 1) / 2);
    // A's oldest tranche written again, a little longer.
    copy.upsert(A, &[edited(msg(A, 1, "edit 1"), 200)]).unwrap();
    assert!(String::from_utf8(file(&ha, 0).unwrap()).unwrap().contains("edit"), "the tranche just written stays");
    assert_eq!(file(&ha, 1), None, "the oldest of the others left");
    assert!(file(&ha, 2).is_some() && file(&hb, 0).is_some());
    assert!(copy.found("plan").partial);
}

/// Review Focus 4: an evicted tranche whose removal failed, or answered `Ok` but left the file (a
/// data file that couldn't look), becomes an orphan, emptied at its chat's next write: no word of
/// a message deleted since comes back.
#[test]
fn a_tranche_whose_removal_failed_keeps_no_deleted_words() {
    for quietly in [false, true] {
        let (copy, store) = failing_copy();
        let fault = |rule: Option<&str>| if quietly { store.fail_quietly(rule) } else { store.fail(None, rule) };
        copy.index.borrow_mut().tranche_cap = Some(8);
        let h = chat_hash(A);
        copy.upsert(A, &(0..16).map(|i| msg(A, i, &format!("plan secret{i}"))).collect::<Vec<_>>()).unwrap();
        drop(copy);
        let file = |n: u32| store.account_file(TEAMS_ACCOUNT, &format!("index/{h}/{n}.json")).unwrap();
        // Reopened without loading: room for tranche 1 and a half.
        let (copy, _) = Copy::open(store.clone(), &who()).unwrap();
        copy.index.borrow_mut().cap = Some(file(1).unwrap().len() as u64 * 3 / 2);
        fault(Some(&format!("index/{h}/0.json")));
        while copy.index_step() {}
        assert!(file(0).is_some(), "its removal failed (quietly: {quietly})");
        fault(None);
        let kept = index_files(&store, A).unwrap();
        assert!(kept.contains("secret3") && kept.contains(&format!("\"{}\"", id(3))));
        copy.upsert(A, &[deleted(msg(A, 3, ""), 99)]).unwrap();
        let kept = index_files(&store, A).unwrap_or_default();
        assert!(!kept.contains("secret3") && !kept.contains(&format!("\"{}\"", id(3))), "quietly: {quietly}\n{kept}");
        assert!(file(0).is_none_or(|b| ids_in(&String::from_utf8(b).unwrap()).is_empty()));
        assert_eq!(files_holding(&store, &id(3)), 0);
        drop(copy);
        let copy = open(&store);
        assert_eq!(hits(&copy, "secret3"), NONE, "quietly: {quietly}");
        assert!(copy.found("secret3").partial);
    }
}

/// Review Focus 2: a chat's oldest tranche evicted, then its messages edited and deleted: the
/// deleted one in no file, the edited one found by its new words, its newer tranches not written.
#[test]
fn edits_and_deletions_in_an_evicted_tranche() {
    let (copy, store) = copy();
    copy.index.borrow_mut().tranche_cap = Some(8);
    let (ha, hb) = (chat_hash(A), chat_hash(B));
    copy.upsert(A, &(0..24).map(|i| msg(A, i, &format!("plan {i}"))).collect::<Vec<_>>()).unwrap();
    copy.upsert(B, &(100..108).map(|i| msg(B, i, &format!("plan {i}"))).collect::<Vec<_>>()).unwrap();
    let file = |h: &str, n: u32| store.account_file(TEAMS_ACCOUNT, &format!("index/{h}/{n}.json")).unwrap();
    let len = |h: &str, n: u32| file(h, n).unwrap().len() as u64;
    let all = len(&ha, 0) + len(&ha, 1) + len(&ha, 2) + len(&hb, 0);
    copy.index.borrow_mut().cap = Some(all - len(&ha, 0) / 2);
    // B's message 100 edited to a text of the same length: A's oldest tranche leaves.
    copy.upsert(B, &[edited(msg(B, 100, "plan 900"), 200)]).unwrap();
    assert_eq!(file(&ha, 0), None);
    assert_eq!(files_holding(&store, &id(5)), 0);
    copy.index.borrow_mut().cap = None;
    store.track_changes();
    copy.upsert(A, &[edited(msg(A, 2, "plan 2 revised"), 300), deleted(msg(A, 5, ""), 300), msg(A, 30, "plan 30")]).unwrap();
    let a_keys: Vec<String> = index_keys(&store).into_iter().filter(|k| k.starts_with(&format!("index/{ha}"))).collect();
    assert_eq!(a_keys, [format!("index/{ha}/0.json"), format!("index/{ha}/3.json")], "1.json and 2.json not written");
    assert_eq!(ids_in(&String::from_utf8(file(&ha, 0).unwrap()).unwrap()), [id(2)]);
    assert_eq!(ids_in(&String::from_utf8(file(&ha, 3).unwrap()).unwrap()), [id(30)]);

    let check = |copy: &Copy| {
        assert_eq!(hits(copy, "revised"), one(A, &id(2)));
        assert!(hits(copy, "plan").contains(&(A.to_string(), id(30))));
        assert_eq!(files_holding(&store, &id(5)), 0);
        assert!(copy.found("plan").partial);
    };
    check(&copy);
    drop(copy);
    check(&open(&store));
}
