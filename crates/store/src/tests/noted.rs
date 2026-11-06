// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Store tests: what saves of a folder's list change in it, noted while a sync asks
//! (`Store::note_changes`), must take the list before to the list after.

use proptest::prelude::*;
use quatrelle_model::{Flag, ListChange};

use super::*;
use crate::folders::MARKS;

/// The list on the disk, by number.
fn kept(s: &Store) -> BTreeMap<u32, Summary> {
    s.load_folder("a1", "INBOX").unwrap().messages.into_iter().map(|m| (m.key.uid, m)).collect()
}

/// `before` with `change` taken, as the window takes it.
fn taken(mut before: BTreeMap<u32, Summary>, change: &ListChange) -> BTreeMap<u32, Summary> {
    for uid in &change.removed {
        before.remove(uid);
    }
    for m in &change.upserts {
        before.insert(m.key.uid, m.clone());
    }
    before
}

fn list(uids: impl Iterator<Item = u32>) -> FolderState {
    let mut messages: Vec<Summary> = uids.map(|u| msg(u, u % 3 == 0)).collect();
    sort_newest_first(&mut messages);
    FolderState { validity: 3, messages, ..Default::default() }
}

fn changed(noted: Noted) -> ListChange {
    match noted {
        Noted::Changed(c) => *c,
        other => panic!("not told: {other:?}"),
    }
}

/// Three come, one goes, one marked read, in a small list and in one kept in pages: told exactly
/// that, with what the list holds after.
#[test]
fn a_sync_is_told_as_what_it_changed() {
    for n in [100u32, 10_000] {
        let s = Store::memory();
        let before = list(1..=n);
        s.save_folder("a1", "INBOX", &before).unwrap();
        let guard = s.note_changes("a1", "INBOX");
        let mut st = s.load_folder("a1", "INBOX").unwrap();
        st.messages.retain(|m| m.key.uid != n / 2);
        if let Some(m) = st.messages.iter_mut().find(|m| m.key.uid == 7) {
            m.flags.seen = !m.flags.seen;
        }
        for uid in n + 1..=n + 3 {
            st.messages.insert(0, msg(uid, false));
        }
        s.save_folder("a1", "INBOX", &st).unwrap();
        let c = changed(guard.take());
        let mut upserts: Vec<u32> = c.upserts.iter().map(|m| m.key.uid).collect();
        upserts.sort_unstable();
        assert_eq!(upserts, vec![7, n + 1, n + 2, n + 3], "{n}");
        assert_eq!(c.removed, vec![n / 2], "{n}");
        assert_eq!(c.count, st.messages.len(), "{n}");
        assert_eq!(c.print, ListChange::print(st.messages.iter().map(|m| m.key.uid)), "{n}");
        assert_eq!(taken(before.messages.iter().map(|m| (m.key.uid, m.clone())).collect(), &c), kept(&s), "{n}");
        assert!(!s.noting("a1", "INBOX"), "{n}: noting stopped");
    }
}

/// A big folder's marks are what the disk has: a sync that leaves a marked flag as marked tells
/// nothing of it; one that puts it back tells it.
#[test]
fn marks_beside_the_pages_are_the_list() {
    let s = Store::memory();
    s.save_folder("a1", "INBOX", &list(1..=9_000)).unwrap();
    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 3, uid: 5_000 };
    s.apply_op("a1", &Op::SetFlag { keys: vec![key.clone()], flag: Flag::Flagged, on: true }).unwrap();
    assert!(s.has_marks(&s.folder_dir("a1", "INBOX").join(MARKS)), "kept beside the pages");
    let before = kept(&s);
    // The sync read the list (the mark in it) and brought a message.
    let guard = s.note_changes("a1", "INBOX");
    let mut st = s.load_folder("a1", "INBOX").unwrap();
    st.messages.insert(0, msg(9_001, false));
    s.save_folder("a1", "INBOX", &st).unwrap();
    let c = changed(guard.so_far());
    assert_eq!(c.upserts.iter().map(|m| m.key.uid).collect::<Vec<_>>(), vec![9_001]);
    assert!(c.removed.is_empty());
    let before = taken(before, &c);
    assert_eq!(before, kept(&s));
    // The server says it's not flagged.
    let mut st = s.load_folder("a1", "INBOX").unwrap();
    if let Some(m) = st.find_mut(5_000) {
        m.flags.flagged = false;
    }
    s.save_folder("a1", "INBOX", &st).unwrap();
    let c = changed(guard.take());
    assert_eq!(c.upserts.iter().map(|m| m.key.uid).collect::<Vec<_>>(), vec![5_000]);
    assert_eq!(taken(before, &c), kept(&s));
}

/// Flags changed alone (`save_folder_changed`) are those messages; what isn't told so is said
/// to be unknown: a list numbered anew, emptied, more changed than is carried.
#[test]
fn what_cant_be_told_is_unknown() {
    let s = Store::memory();
    s.save_folder("a1", "INBOX", &list(1..=6_000)).unwrap();
    let guard = s.note_changes("a1", "INBOX");
    let mut st = s.load_folder("a1", "INBOX").unwrap();
    for m in st.messages.iter_mut().filter(|m| m.key.uid < 4) {
        m.preview = "filled".into();
    }
    s.save_folder_changed("a1", "INBOX", &st, &[1, 2, 3].into_iter().collect()).unwrap();
    let c = changed(guard.so_far());
    assert_eq!(c.upserts.len(), 3);
    assert!(c.upserts.iter().all(|m| m.preview == "filled"));
    assert_eq!(guard.so_far(), Noted::Nothing, "told once");
    // Numbered anew.
    let mut again = list(1..=10);
    again.validity = 4;
    s.save_folder("a1", "INBOX", &again).unwrap();
    assert_eq!(guard.so_far(), Noted::Unknown);
    // Emptied.
    s.clear_folder("a1", "INBOX").unwrap();
    assert_eq!(guard.so_far(), Noted::Unknown);
    // A first list of many.
    s.save_folder("a1", "INBOX", &list(1..=(NOTED_MAX as u32 + 10))).unwrap();
    assert_eq!(guard.so_far(), Noted::Unknown);
    drop(guard);
    assert!(!s.noting("a1", "INBOX"), "dropped, noting stops");
    // Not noted, nothing is kept.
    s.save_folder("a1", "INBOX", &list(1..=5)).unwrap();
    assert!(s.noting.lock().unwrap().is_empty());
}

/// Two noting the same folder at once: each told what changed since the other was, noting stops
/// with the last.
#[test]
fn two_noting_the_same_folder() {
    let s = Store::memory();
    s.save_folder("a1", "INBOX", &list(1..=10)).unwrap();
    let a = s.note_changes("a1", "INBOX");
    let b = s.note_changes("a1", "INBOX");
    s.save_folder("a1", "INBOX", &list(1..=11)).unwrap();
    assert_eq!(changed(a.take()).upserts.len(), 1);
    assert!(s.noting("a1", "INBOX"));
    s.save_folder("a1", "INBOX", &list(2..=11)).unwrap();
    assert_eq!(changed(b.take()).removed, vec![1]);
    assert!(!s.noting("a1", "INBOX"));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// Whatever saves change (messages gone, come above and below the others, flags and text
    /// changed, marks beside the pages), the list before with what was noted is the list after.
    #[test]
    fn the_list_before_with_the_change_is_the_list_after(
        n in prop_oneof![1u32..300, 4_000u32..13_000],
        saves in prop::collection::vec((prop::collection::vec(any::<u32>(), 0..12), 0u32..4, 0u32..3, any::<bool>()), 1..4),
    ) {
        let s = Store::memory();
        // Numbered from 1,000, so some can come below.
        s.save_folder("a1", "INBOX", &list(1_000..1_000 + n)).unwrap();
        let mut before = kept(&s);
        let guard = s.note_changes("a1", "INBOX");
        for (touch, come, below, mark) in saves {
            let mut st = s.load_folder("a1", "INBOX").unwrap();
            let len = st.messages.len().max(1);
            if mark && let Some(m) = st.messages.get(touch.first().copied().unwrap_or(0) as usize % len) {
                // Made by the window: the list it holds has it already.
                let key = m.key.clone();
                if let Some(b) = before.get_mut(&key.uid) {
                    b.flags.answered = true;
                }
                s.apply_op("a1", &Op::SetFlag { keys: vec![key], flag: Flag::Answered, on: true }).unwrap();
                st = s.load_folder("a1", "INBOX").unwrap();
            }
            for (i, t) in touch.iter().enumerate() {
                let at = *t as usize % len;
                match i % 3 {
                    0 if at < st.messages.len() => {
                        st.messages.remove(at);
                    }
                    1 => {
                        if let Some(m) = st.messages.get_mut(at) {
                            m.flags.flagged = !m.flags.flagged;
                        }
                    }
                    _ => {
                        if let Some(m) = st.messages.get_mut(at) {
                            m.preview = format!("changed {t}");
                        }
                    }
                }
            }
            let top = st.messages.iter().map(|m| m.key.uid).max().unwrap_or(1_000);
            let low = st.messages.iter().map(|m| m.key.uid).min().unwrap_or(1_000);
            for k in 1..=come {
                st.messages.push(msg(top + k, false));
            }
            for k in 1..=below {
                st.messages.push(msg(low.saturating_sub(k), true));
            }
            sort_newest_first(&mut st.messages);
            s.save_folder("a1", "INBOX", &st).unwrap();
            let after = kept(&s);
            match guard.so_far() {
                Noted::Changed(c) => {
                    prop_assert_eq!(c.count, after.len());
                    prop_assert_eq!(c.print, ListChange::print(after.keys().copied()));
                    before = taken(before, &c);
                }
                Noted::Nothing => {}
                Noted::Unknown => prop_assert!(false, "told as unknown"),
            }
            prop_assert_eq!(&before, &after);
        }
    }
}
