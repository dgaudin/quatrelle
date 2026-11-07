// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Store tests: the changes waiting for the server, kept a file each, and made again on a list a
//! sync saves in one pass over it.

use std::time::{Duration, Instant};

use proptest::prelude::*;
use quatrelle_model::{Flag, Flags};

use super::*;

/// A folder of `n` messages numbered from 1, a few read.
fn folder(n: u32) -> FolderState {
    let mut st = FolderState {
        validity: 3,
        uid_next: n + 1,
        messages: (1..=n)
            .map(|uid| Summary {
                key: MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 3, uid },
                subject: format!("m{uid}"),
                flags: Flags { seen: uid % 7 == 0, ..Default::default() },
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    sort_newest_first(&mut st.messages);
    st
}

fn key(uid: u32) -> MessageKey {
    MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 3, uid }
}

/// The `i`th change of a burst made offline on a folder of `n`: mostly marks, some categories,
/// some moved away or deleted.
fn burst_op(i: u32, n: u32) -> Op {
    let uid = (i.wrapping_mul(7919) % n) + 1;
    match i % 10 {
        0 => Op::Move { keys: vec![key(uid)], to: "Archive".into() },
        1 => Op::Expunge { keys: vec![key(uid)], tidy: false },
        2 | 3 => Op::SetKeyword { keys: vec![key(uid)], keyword: "$label1".into(), on: i % 4 == 2, name: Some("Red".into()) },
        4 => Op::SetFlag { keys: vec![key(uid)], flag: Flag::Flagged, on: true },
        _ => Op::SetFlag { keys: vec![key(uid)], flag: Flag::Seen, on: !i.is_multiple_of(3) },
    }
}

/// What a burst of 1,000 changes made offline on a folder of 50,000 costs: adding the last of
/// them, and the saves of the sync that follows (the folder as the server had it, saved with
/// the changes made again on it; then each change waiting made again on the folder, as the
/// account's thread does after a sync). Run it in a release build:
///
/// ```text
/// cargo test -p quatrelle-store --release --lib journal::a_burst_of_changes -- --ignored --nocapture
/// ```
#[test]
#[ignore = "a bench: run it with --release, --ignored and --nocapture"]
fn a_burst_of_changes_made_offline() {
    let dir = std::env::temp_dir().join(format!("quatrelle-store-burst-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let s = Store::open_packed(dir.join("root"), dir.join("files")).unwrap();
    let n = 50_000;
    let st = folder(n);
    s.save_folder("a1", "INBOX", &st).unwrap();
    let (mut adds, mut applied) = (Vec::new(), Duration::ZERO);
    for i in 0..1000 {
        let op = burst_op(i, n);
        let t = Instant::now();
        s.add_op("a1", op.clone()).unwrap();
        adds.push(t.elapsed());
        let t = Instant::now();
        s.apply_op("a1", &op).unwrap();
        applied += t.elapsed();
    }
    let ms = |d: Duration| d.as_secs_f64() * 1e3;
    println!("the burst made in the cache (the edit writer's): {:.1} s", applied.as_secs_f64());
    let last = adds.last().copied().unwrap_or_default();
    let first10: Duration = adds.iter().take(10).sum();
    let last10: Duration = adds.iter().rev().take(10).sum();
    println!("adding: the first 10 {:.2} ms each, the last 10 {:.2} ms each, the 1,000th {:.2} ms", ms(first10) / 10.0, ms(last10) / 10.0, ms(last));
    // The sync: the folder as the server had it, saved; the changes made again on it.
    let t = Instant::now();
    s.save_folder("a1", "INBOX", &st).unwrap();
    println!("the sync's save of the folder: {:.1} ms", ms(t.elapsed()));
    let t = Instant::now();
    let changed = s.save_folder_changed("a1", "INBOX", &st, &std::collections::HashSet::from([1u32])).map(drop);
    changed.unwrap();
    println!("a save of a few messages filled: {:.1} ms", ms(t.elapsed()));
    // The account's thread then looks at the changes waiting on the folder saved
    // (`reapply_waiting`): it made each again there (309 s here before R19); now only a folder
    // still to empty, which a save doesn't make again.
    let t = Instant::now();
    let waiting = s.load_ops("a1").unwrap();
    let mut n_again = 0;
    for op in &waiting {
        if matches!(op, Op::EmptyFolder { .. }) {
            s.apply_op("a1", op).unwrap();
        }
        n_again += usize::from(op.keys().iter().any(|k| k.folder == "INBOX"));
    }
    println!("the account's thread after the sync, {n_again} changes waiting there: {:.1} ms", ms(t.elapsed()));
    println!("the changes waiting, in one file as before: {} KB", serde_json::to_vec(&waiting).unwrap().len() / 1000);
    // The list ends as the changes left it.
    let back = s.load_folder("a1", "INBOX").unwrap();
    let mut want = st.clone();
    for i in 0..1000 {
        reapply_each(&mut want, "a1", "INBOX", &[burst_op(i, n)]);
    }
    assert_eq!(back.messages.len(), want.messages.len());
    assert!(back.messages.iter().zip(&want.messages).all(|(a, b)| a.key == b.key && a.flags == b.flags));
    drop(s);
    let _ = fs::remove_dir_all(&dir);
}

/// The changes waiting made again on a list one at a time, each over the whole list (as before
/// the single pass): the reference the single pass is held to. The numbers changed.
fn reapply_each(st: &mut FolderState, account: &str, folder: &str, ops: &[Op]) -> HashSet<u32> {
    let mut touched = HashSet::new();
    let validity = st.validity;
    let uids = |keys: &[MessageKey]| -> HashSet<u32> {
        keys.iter().filter(|k| k.account == account && k.folder == folder && k.validity == validity).map(|k| k.uid).collect()
    };
    for op in ops {
        match op {
            Op::SetFlag { keys, flag, on } => {
                let wanted = uids(keys);
                for m in st.messages.iter_mut().filter(|m| wanted.contains(&m.key.uid)) {
                    let before = m.flags.clone();
                    flag.set(&mut m.flags, *on);
                    if m.flags != before {
                        touched.insert(m.key.uid);
                    }
                }
            }
            Op::SetKeyword { keys, keyword, on, name } => {
                let wanted = uids(keys);
                for m in st.messages.iter_mut().filter(|m| wanted.contains(&m.key.uid)) {
                    let before = m.flags.clone();
                    m.flags.set_category(keyword, name.as_deref(), *on);
                    if m.flags != before {
                        touched.insert(m.key.uid);
                    }
                }
            }
            Op::Move { keys, .. } | Op::Expunge { keys, .. } => {
                let gone = uids(keys);
                st.messages.retain(|m| !gone.contains(&m.key.uid));
            }
            _ => {}
        }
    }
    touched
}

/// A message of a small folder for the proptest: its number and flags.
fn small(uid: u32, seen: bool, flagged: bool) -> Summary {
    Summary { key: key(uid), subject: format!("m{uid}"), flags: Flags { seen, flagged, ..Default::default() }, ..Default::default() }
}

/// An edit for the proptest, on a few numbers (some not in the list, some of another folder or
/// numbering, some named twice).
fn any_op() -> impl Strategy<Value = Op> {
    let keys = proptest::collection::vec((0u32..40, 0u8..6), 1..5).prop_map(|ks| {
        ks.into_iter()
            .map(|(uid, w)| match w {
                0 => MessageKey { folder: "Other".into(), ..key(uid) },
                1 => MessageKey { validity: 9, ..key(uid) },
                _ => key(uid),
            })
            .collect::<Vec<_>>()
    });
    (keys, 0u8..6, any::<bool>()).prop_map(|(keys, kind, on)| match kind {
        0 => Op::SetFlag { keys, flag: Flag::Seen, on },
        1 => Op::SetFlag { keys, flag: Flag::Flagged, on },
        2 => Op::SetKeyword { keys, keyword: "$label1".into(), on, name: Some("Red".into()) },
        3 => Op::SetKeyword { keys, keyword: "$label2".into(), on, name: None },
        4 => Op::Move { keys, to: "Archive".into() },
        _ => Op::Expunge { keys, tidy: false },
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 200, ..ProptestConfig::default() })]
    /// Edits waiting made again in one pass give the list, and the numbers changed, that making
    /// them one at a time over the whole list gave.
    #[test]
    fn one_pass_makes_the_edits_as_one_at_a_time(
        list in proptest::collection::vec((any::<bool>(), any::<bool>()), 0..30),
        ops in proptest::collection::vec(any_op(), 0..25),
    ) {
        let mut st = FolderState { validity: 3, messages: list.iter().enumerate().map(|(i, (s, f))| small(i as u32, *s, *f)).collect(), ..Default::default() };
        let (once, touched) = crate::folders::reapply(st.messages.iter(), "a1", "INBOX", 3, &ops);
        let once: Vec<Summary> = once.into_iter().map(std::borrow::Cow::into_owned).collect();
        let each_touched = reapply_each(&mut st, "a1", "INBOX", &ops);
        prop_assert_eq!(once, st.messages);
        prop_assert_eq!(touched, each_touched);
    }
}

/// The journal keeps the changes in the order made, a file each: adding one writes it alone;
/// the oldest is read alone; one taken out is the first equal to it; a damaged one is set aside
/// (that change lost, not the queue).
#[test]
fn the_journal_keeps_a_file_a_change_in_order() {
    let s = Store::memory();
    let ops: Vec<Op> = (0..1000).map(|i| burst_op(i, 50)).collect();
    for op in &ops[..999] {
        s.add_op("a1", op.clone()).unwrap();
    }
    s.track_changes();
    s.add_op("a1", ops[999].clone()).unwrap();
    let written = s.take_changes();
    assert_eq!(written.len(), 1, "the 1,000th alone");
    assert!(written[0].1.as_ref().is_some_and(|b| b.len() < 1024));
    assert!(written[0].0.ends_with("ops/00000000000000001000.json"), "{:?}", written[0].0);
    assert_eq!(s.load_ops("a1").unwrap(), ops);
    assert_eq!(s.first_op("a1").unwrap().as_ref(), Some(&ops[0]));
    // The first equal taken out: a change made twice leaves its later copy.
    let twice = Op::SetFlag { keys: vec![key(7)], flag: Flag::Seen, on: true };
    let s = Store::memory();
    for op in [twice.clone(), ops[1].clone(), twice.clone()] {
        s.add_op("a1", op).unwrap();
    }
    s.remove_op("a1", &twice).unwrap();
    assert_eq!(s.load_ops("a1").unwrap(), [ops[1].clone(), twice.clone()]);
    s.remove_op("a1", &ops[2]).unwrap();
    assert_eq!(s.load_ops("a1").unwrap().len(), 2, "not there: nothing taken out");
    // A damaged change: set aside, the others kept, the next numbered after it all the same.
    let dir = s.account_dir("a1").join("ops");
    s.files.write(&dir.join("00000000000000000002.json"), b"{not json").unwrap();
    assert_eq!(s.load_ops("a1").unwrap(), std::slice::from_ref(&twice));
    s.add_op("a1", ops[3].clone()).unwrap();
    assert_eq!(s.load_ops("a1").unwrap(), [twice.clone(), ops[3].clone()]);
    s.save_ops("a1", &[]).unwrap();
    assert!(s.load_ops("a1").unwrap().is_empty() && s.first_op("a1").unwrap().is_none());
}

/// An `ops.json` from before the journal is the head of the queue: read first, the changes
/// added after it, taken out of it first; it goes once empty, nothing copied (so nothing is
/// ever there twice). One that can't be read stops a change being taken out.
#[test]
fn an_ops_json_from_before_is_read_first_and_empties() {
    let s = Store::memory();
    let path = s.account_dir("a1").join("ops.json");
    let old = vec![burst_op(0, 50), burst_op(1, 50)];
    s.files.write(&path, &serde_json::to_vec(&old).unwrap()).unwrap();
    let new = burst_op(4, 50);
    s.add_op("a1", new.clone()).unwrap();
    assert_eq!(s.load_ops("a1").unwrap(), [old[0].clone(), old[1].clone(), new.clone()]);
    assert_eq!(s.first_op("a1").unwrap().as_ref(), Some(&old[0]));
    s.remove_op("a1", &old[0]).unwrap();
    assert_eq!(s.first_op("a1").unwrap().as_ref(), Some(&old[1]));
    s.remove_op("a1", &old[1]).unwrap();
    assert!(!s.files.exists(&path), "gone once empty");
    assert_eq!(s.load_ops("a1").unwrap(), std::slice::from_ref(&new));
    // A list saved by a sync makes it again, from either.
    let f = Store::failing();
    let st = folder(10);
    f.save_folder("a1", "INBOX", &st).unwrap();
    let fpath = f.account_dir("a1").join("ops.json");
    let seen = Op::SetFlag { keys: vec![key(3)], flag: Flag::Seen, on: true };
    f.files.write(&fpath, &serde_json::to_vec(std::slice::from_ref(&seen)).unwrap()).unwrap();
    f.add_op("a1", Op::Expunge { keys: vec![key(5)], tidy: false }).unwrap();
    f.save_folder("a1", "INBOX", &st).unwrap();
    let back = f.load_folder("a1", "INBOX").unwrap();
    assert!(back.find(3).unwrap().flags.seen && back.find(5).is_none());
    f.fail(Some("ops.json"), None);
    assert!(f.remove_op("a1", &seen).is_err(), "can't be read: not written over");
    f.fail(None, None);
    assert_eq!(f.load_ops("a1").unwrap().len(), 2);
}

/// Deleting for good, as a journal written before Quatrelle told its own tidying (a draft's
/// earlier copies) from the user's deleting kept it: read back whole, as the user's (Exchange
/// keeps it recoverable, the safe side), from either form of the journal, and taken out once
/// done. The tidying is kept as such; the user's is written as before, without the field.
#[test]
fn a_deletion_written_before_tidying_was_told_apart_reads_as_the_users() {
    let s = Store::memory();
    let old = br#"{"op":"expunge","keys":[{"account":"a1","folder":"INBOX","validity":3,"uid":5}]}"#;
    s.files.write(&s.account_dir("a1").join("ops.json"), &[b"[".as_slice(), old, b"]"].concat()).unwrap();
    s.files.write(&s.account_dir("a1").join("ops").join("00000000000000000001.json"), old).unwrap();
    let users = Op::Expunge { keys: vec![key(5)], tidy: false };
    assert_eq!(s.load_ops("a1").unwrap(), [users.clone(), users.clone()], "both read, as the user's");
    let tidy = Op::Expunge { keys: vec![key(6)], tidy: true };
    s.add_op("a1", tidy.clone()).unwrap();
    assert_eq!(s.load_ops("a1").unwrap().last(), Some(&tidy), "tidying read back as tidying");
    s.remove_op("a1", &users).unwrap();
    s.remove_op("a1", &users).unwrap();
    assert_eq!(s.load_ops("a1").unwrap(), std::slice::from_ref(&tidy), "the old ones taken out once done");
    assert_eq!(serde_json::to_vec(&users).unwrap(), old.to_vec(), "the user's written as before");
}
