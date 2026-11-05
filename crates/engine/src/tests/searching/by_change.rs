// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: a folder's kept index follows its changes, a message at a time. One message
//! come in, gone, moved, or its source come later, and the next search (in the same run, or the
//! next one) reads that message alone, not the folder's every message again.

use super::*;

const MESSAGES: u32 = 300;

fn key(folder: &str, uid: u32) -> MessageKey {
    MessageKey { account: "a1".into(), folder: folder.into(), validity: 1, uid }
}

/// A message with its source kept, `report` in the subject (the body says what it's given:
/// words of its own are `u<uid>z`, none a prefix of another's).
fn message(store: &Store, folder: &str, uid: u32, body: &str) -> Summary {
    let k = key(folder, uid);
    store.write_body(&k, format!("From: a@x.example\r\nSubject: Report {uid}\r\n\r\n{body}\r\n").as_bytes()).unwrap();
    Summary { key: k, subject: format!("Report {uid}"), ..Default::default() }
}

/// An account with an Inbox of [`MESSAGES`] downloaded messages and an Archive of ten.
fn mailbox(name: &str) -> Store {
    let store = temp(name);
    store.save_accounts(&[quatrelle_model::AccountConfig { id: "a1".into(), email: "me@studio.example".into(), ..Default::default() }]).unwrap();
    let folder = |path: &str, role| quatrelle_model::Folder { account: "a1".into(), path: path.into(), role, selectable: true, ..Default::default() };
    store.save_folders("a1", &[folder("INBOX", FolderRole::Inbox), folder("Archive", FolderRole::Archive)]).unwrap();
    for (path, uids) in [("INBOX", 1..=MESSAGES), ("Archive", 1001..=1010)] {
        let mut st = store.load_folder("a1", path).unwrap();
        st.validity = 1;
        for uid in uids.rev() {
            st.messages.push(message(&store, path, uid, &format!("u{uid}z common")));
        }
        store.save_folder("a1", path, &st).unwrap();
    }
    store
}

fn open(store: &Store) -> Session {
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    s.take_jobs();
    s
}

/// A search over every folder: what it found, and the sources it read for it.
fn search(s: &mut Session, store: &Store, q: &str) -> (u64, u64) {
    let before = store.reads();
    let found = s.run("search.run", &json!({"query": q, "scope": "all"})).unwrap()["results"].as_u64().unwrap_or(0);
    (found, store.reads() - before)
}

fn change(store: &Store, folder: &str, f: impl FnOnce(&mut Vec<Summary>)) {
    let mut st = store.load_folder("a1", folder).unwrap();
    f(&mut st.messages);
    store.save_folder("a1", folder, &st).unwrap();
}

#[test]
fn a_change_reads_the_message_changed_not_its_folder() {
    let store = mailbox("index-by-change");
    {
        let mut s = open(&store);
        assert_eq!(search(&mut s, &store, "common").0, u64::from(MESSAGES) + 10, "the first search reads all");
        // In the same run: a message comes in, read alone.
        change(&store, "INBOX", |l| l.insert(0, message(&store, "INBOX", 500, "u500z arrived")));
        s.apply(Event::FolderChanged { account: "a1".into(), folder: "INBOX".into() });
        let (found, read) = search(&mut s, &store, "arrived");
        assert!(found == 1 && read <= 1, "in the run: the new one alone ({found} found, {read} read)");
    }
    // The next run: the folder's kept index is taken, and only what changed is read.
    let mut s = open(&store);
    let (found, read) = search(&mut s, &store, "arrived");
    assert_eq!(found, 1);
    assert!(read <= 1, "after a restart, one message come in: {read} sources read, not the folder's {MESSAGES}");
    drop(s);

    // One gone, one moved to the Archive, one whose source came after its index (its preview
    // filled from it, as a download does): the next run reads the moved one and the late one.
    change(&store, "INBOX", |l| {
        l.retain(|m| m.key.uid != 10 && m.key.uid != 20);
        if let Some(m) = l.iter_mut().find(|m| m.key.uid == 30) {
            m.preview = "late words".into();
        }
    });
    store.remove_body(&key("INBOX", 10));
    store.remove_body(&key("INBOX", 20));
    change(&store, "Archive", |l| l.insert(0, message(&store, "Archive", 1020, "u20z moved")));
    store.write_body(&key("INBOX", 30), b"From: a@x.example\r\nSubject: Report 30\r\n\r\nu30z late words\r\n").unwrap();
    let mut s = open(&store);
    let (found, read) = search(&mut s, &store, "common");
    // The two gone don't answer, nor the late one (its body has other words now; the one come
    // in never had it).
    assert_eq!(found, u64::from(MESSAGES) - 3 + 10);
    assert!(read <= 3, "after a restart: {read} sources read, not the folders' {}", MESSAGES + 10);
    assert_eq!(search(&mut s, &store, "u10z").0, 0, "gone");
    assert_eq!(search(&mut s, &store, "u20z").0, 1, "moved: found in the Archive");
    assert_eq!(search(&mut s, &store, "late").0, 1, "its late words");
    drop(s);
    // And the run after that reads nothing at all.
    let mut s = open(&store);
    assert_eq!(search(&mut s, &store, "common"), (u64::from(MESSAGES) - 3 + 10, 0));
    done(s);
}

/// What can happen to a folder between two runs.
#[derive(Clone, Debug)]
enum Change {
    /// A message comes in, its subject and body each a word.
    Add { folder: usize, subject: usize, body: Option<usize> },
    /// The `nth` goes.
    Remove { folder: usize, nth: usize },
    /// The `nth` goes to the other folder.
    Move { folder: usize, nth: usize },
    /// The `nth`'s summary says something else (its preview filled).
    Preview { folder: usize, nth: usize, word: usize },
    /// The `nth`'s source comes (if it had none).
    Body { folder: usize, nth: usize, word: usize },
}

const WORDS: [&str; 8] = ["alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel"];
const FOLDERS: [&str; 2] = ["INBOX", "Archive"];

fn change_strategy() -> impl proptest::strategy::Strategy<Value = Change> {
    use proptest::prelude::*;
    let w = 0..WORDS.len();
    prop_oneof![
        (0..2usize, w.clone(), proptest::option::of(w.clone())).prop_map(|(folder, subject, body)| Change::Add { folder, subject, body }),
        (0..2usize, 0..20usize).prop_map(|(folder, nth)| Change::Remove { folder, nth }),
        (0..2usize, 0..20usize).prop_map(|(folder, nth)| Change::Move { folder, nth }),
        (0..2usize, 0..20usize, w.clone()).prop_map(|(folder, nth, word)| Change::Preview { folder, nth, word }),
        (0..2usize, 0..20usize, w).prop_map(|(folder, nth, word)| Change::Body { folder, nth, word }),
    ]
}

/// A message numbered `uid` in `folder`: its subject a word, its source (when it has one)
/// another.
fn put(store: &Store, folder: &str, uid: u32, subject: usize, body: Option<usize>) -> Summary {
    let k = key(folder, uid);
    if let Some(b) = body {
        store.write_body(&k, format!("From: a@x.example\r\nSubject: {}\r\n\r\nzulu {}\r\n", WORDS[subject], WORDS[b]).as_bytes()).unwrap();
    }
    Summary { key: k, subject: WORDS[subject].into(), ..Default::default() }
}

/// What a search over every folder finds for each word.
fn answers(store: &Store) -> Vec<std::collections::BTreeSet<(String, u32)>> {
    let mut s = open(store);
    let out = WORDS
        .iter()
        .map(|w| {
            s.run("search.run", &json!({"query": w, "scope": "all"})).unwrap();
            s.list.iter().map(|m| (m.key.folder.clone(), m.key.uid)).collect()
        })
        .collect();
    s.run("search.clear", &json!({})).unwrap();
    out
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(40))]

    /// Messages come in, go, move, and change their summary or get their source between two
    /// runs: the kept index taken back with its changes finds what an index built anew does,
    /// and so does the index it keeps in turn.
    #[test]
    fn a_kept_index_taken_with_its_changes_finds_what_a_new_one_does(
        first in proptest::collection::vec((0..2usize, 0..WORDS.len(), proptest::option::of(0..WORDS.len())), 0..24),
        changes in proptest::collection::vec(change_strategy(), 0..16),
    ) {
        use std::sync::atomic::{AtomicU32, Ordering};
        static CASE: AtomicU32 = AtomicU32::new(0);
        let store = temp(&format!("index-by-change-prop-{}", CASE.fetch_add(1, Ordering::Relaxed)));
        store.save_accounts(&[quatrelle_model::AccountConfig { id: "a1".into(), email: "me@studio.example".into(), ..Default::default() }]).unwrap();
        let folder = |path: &str| quatrelle_model::Folder { account: "a1".into(), path: path.into(), selectable: true, ..Default::default() };
        store.save_folders("a1", &[folder(FOLDERS[0]), folder(FOLDERS[1])]).unwrap();
        let mut lists: [Vec<Summary>; 2] = [Vec::new(), Vec::new()];
        let mut next = 1u32;
        for (f, subject, body) in first {
            lists[f].insert(0, put(&store, FOLDERS[f], next, subject, body));
            next += 1;
        }
        let save = |lists: &[Vec<Summary>; 2]| {
            for (f, list) in lists.iter().enumerate() {
                let mut st = store.load_folder("a1", FOLDERS[f]).unwrap();
                st.validity = 1;
                st.messages = list.clone();
                store.save_folder("a1", FOLDERS[f], &st).unwrap();
            }
        };
        save(&lists);
        // The first run builds and keeps each folder's index.
        answers(&store);
        for c in changes {
            match c {
                Change::Add { folder, subject, body } => {
                    lists[folder].insert(0, put(&store, FOLDERS[folder], next, subject, body));
                    next += 1;
                }
                Change::Remove { folder, nth } if !lists[folder].is_empty() => {
                    let m = lists[folder].remove(nth % lists[folder].len());
                    store.remove_body(&m.key);
                }
                Change::Move { folder, nth } if !lists[folder].is_empty() => {
                    let mut m = lists[folder].remove(nth % lists[folder].len());
                    let raw = store.read_body(&m.key);
                    store.remove_body(&m.key);
                    m.key = key(FOLDERS[1 - folder], next);
                    next += 1;
                    if let Some(raw) = raw {
                        store.write_body(&m.key, &raw).unwrap();
                    }
                    lists[1 - folder].insert(0, m);
                }
                Change::Preview { folder, nth, word } if !lists[folder].is_empty() => {
                    let at = nth % lists[folder].len();
                    lists[folder][at].preview = format!("{} preview", WORDS[word]);
                }
                Change::Body { folder, nth, word } if !lists[folder].is_empty() => {
                    let at = nth % lists[folder].len();
                    let k = lists[folder][at].key.clone();
                    if !store.has_body(&k) {
                        store.write_body(&k, format!("From: a@x.example\r\n\r\nzulu {}\r\n", WORDS[word]).as_bytes()).unwrap();
                    }
                }
                _ => {}
            }
        }
        save(&lists);
        let taken = answers(&store);
        let kept_again = answers(&store);
        for f in FOLDERS {
            store.remove_search_index("a1", f);
        }
        let anew = answers(&store);
        let _ = std::fs::remove_dir_all(store.root());
        proptest::prop_assert_eq!(&taken, &anew, "the kept index taken with its changes");
        proptest::prop_assert_eq!(&kept_again, &anew, "the index it kept in turn");
    }
}
