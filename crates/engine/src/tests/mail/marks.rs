// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: a read mark in a big folder writes the mark, not the page it's in.

use super::*;

/// A message marked read in a local account's folder of 10,941 writes a few bytes, the folder's
/// count with it: it saved the whole page of 4,096 summaries it's in (about 2 MB, 24 to 90 ms of
/// the window's frame on a real file). Read again, the folder has the mark.
#[test]
fn a_mark_read_in_a_big_folder_writes_under_64_kib() {
    let store = Store::memory();
    sample::seed_if_empty(&store, NOW).unwrap();
    let n = 10_941u32;
    let mut st = store.load_folder("sample", "Projects").unwrap();
    st.validity = 7;
    st.messages = (1..=n)
        .map(|uid| quatrelle_model::Summary {
            key: quatrelle_model::MessageKey { account: "sample".into(), folder: "Projects".into(), validity: 7, uid },
            subject: format!("Report {uid}"),
            date: NOW - i64::from(uid) * 60,
            ..Default::default()
        })
        .collect();
    store.save_folder("sample", "Projects", &st).unwrap();
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    s.run("folder.open", &json!({"account": "sample", "folder": "Projects"})).unwrap();
    store.track_changes();
    let k = s.list.get(5000).map(|m| m.key.clone()).unwrap();
    s.apply_op("sample", Op::SetFlag { keys: vec![k.clone()], flag: Flag::Seen, on: true });
    let written = store.take_changes();
    let bytes: usize = written.iter().map(|(_, b)| b.as_ref().map_or(0, Vec::len)).sum();
    assert!(bytes < 64 << 10, "{bytes} bytes in {} files", written.len());
    assert!(store.load_folder("sample", "Projects").unwrap().find(k.uid).unwrap().flags.seen);
    let unread = store.load_folders("sample").unwrap().into_iter().find(|f| f.path == "Projects").map(|f| f.unread);
    assert_eq!(unread, Some(n - 1), "the count kept with it");
    let mut again = Session::open(Some(store), quatrelle_model::dates::Clock::new(NOW, 0));
    again.run("folder.open", &json!({"account": "sample", "folder": "Projects"})).unwrap();
    assert!(again.summary(&k).is_some_and(|m| m.flags.seen), "opened again: read");
}
