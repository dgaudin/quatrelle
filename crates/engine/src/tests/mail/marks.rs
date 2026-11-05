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

/// The sample's unread count of its Inbox, as kept on disk.
fn unread_kept(store: &Store) -> u32 {
    store.load_folders("sample").unwrap().into_iter().find(|f| f.path == "INBOX").map_or(0, |f| f.unread)
}

/// A local account's marks, deferred (the window has a thread for them): nothing is written on
/// the session's thread; the work handed out writes them in order, each with its folder's count
/// (one transaction); another change waits for them and comes after; a session that ends with
/// marks waiting writes them. They were written on the window's thread, 25 ms at each Down.
#[test]
fn local_marks_are_written_on_their_thread_in_order() {
    let store = Store::memory();
    sample::seed_if_empty(&store, NOW).unwrap();
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    s.defer_local_marks(true);
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    let unread: Vec<MessageKey> = s.list.iter().filter(|m| !m.flags.seen).map(|m| m.key.clone()).collect();
    assert!(unread.len() >= 3, "the sample has unread mail");
    let before = unread_kept(&store);
    for k in &unread[..2] {
        s.run("list.open", &json!({"key": k.to_id()})).unwrap();
    }
    assert!(unread[..2].iter().all(|k| s.summary(k).is_some_and(|m| m.flags.seen)), "shown read at once");
    assert!(!store.load_folder("sample", "INBOX").unwrap().find(unread[0].uid).unwrap().flags.seen, "not written here");
    assert_eq!(s.local_marks_written_here(), 0);
    let work = s.take_mark_work().expect("work for the lane");
    assert!(s.take_mark_work().is_none(), "one at a time");
    std::thread::spawn(move || work.run()).join().unwrap();
    let kept = store.load_folder("sample", "INBOX").unwrap();
    assert!(unread[..2].iter().all(|k| kept.find(k.uid).unwrap().flags.seen), "written there");
    assert_eq!(unread_kept(&store), before - 2, "the count with them");

    // Another mark, its work handed out but not run yet; then a message deleted: the mark is
    // written first, here, and the deletion after it.
    s.run("list.open", &json!({"key": unread[2].to_id()})).unwrap();
    let late = s.take_mark_work().expect("work for the lane");
    s.run("mail.delete", &json!({})).unwrap();
    assert_eq!(s.local_marks_written_here(), 1, "finished before the deletion");
    late.run();
    assert!(store.load_folder("sample", "INBOX").unwrap().find(unread[2].uid).is_none(), "deleted after");

    // A mark left waiting as the session ends is written.
    let k = s.list.iter().find(|m| !m.flags.seen).map(|m| m.key.clone());
    if let Some(k) = k {
        s.apply_op("sample", Op::SetFlag { keys: vec![k.clone()], flag: Flag::Seen, on: true });
        assert!(!store.load_folder("sample", "INBOX").unwrap().find(k.uid).unwrap().flags.seen);
        drop(s);
        assert!(store.load_folder("sample", "INBOX").unwrap().find(k.uid).unwrap().flags.seen, "written as it ended");
    }
    let again = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    let inbox = again.account("sample").and_then(|a| a.folders.iter().find(|f| f.path == "INBOX")).map(|f| f.unread);
    assert_eq!(inbox, Some(unread_kept(&store)));
}

/// A local mark writes its folder's list (or marks) and the account's counts, and nothing else;
/// written at once where it isn't deferred (the CLI, `--cmd`, the tests).
#[test]
fn a_local_mark_writes_the_list_and_the_counts_alone() {
    let store = Store::memory();
    sample::seed_if_empty(&store, NOW).unwrap();
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    let k = s.list.iter().find(|m| !m.flags.seen).map(|m| m.key.clone()).unwrap();
    store.track_changes();
    s.apply_op("sample", Op::SetFlag { keys: vec![k.clone()], flag: Flag::Seen, on: true });
    let mut names: Vec<String> = store.take_changes().into_iter().map(|(p, _)| p.rsplit('/').next().unwrap_or_default().to_string()).collect();
    names.sort_unstable();
    assert_eq!(names, ["folders.json", "state.json"]);
    assert_eq!(s.local_marks_written_here(), 1);
    assert!(store.load_folder("sample", "INBOX").unwrap().find(k.uid).unwrap().flags.seen);
}
