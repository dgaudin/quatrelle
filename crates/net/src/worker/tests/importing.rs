// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Tests: the pool's import of a data file is told with its total and its folder, stops when
//! asked and is gone on with.

use super::*;

/// Wait for an import's end, with everything told before it.
fn until_done(p: &Pool, seen: &mut Vec<Event>, mut each: impl FnMut(&Event)) -> Event {
    let end = Instant::now() + Duration::from_secs(120);
    while Instant::now() < end {
        if let Some(e) = p.wait(Duration::from_millis(100)) {
            each(&e);
            if matches!(e, Event::Imported { done: true, .. }) {
                return e;
            }
            seen.push(e);
        }
    }
    panic!("the import did not end: {seen:?}");
}

#[test]
fn an_import_is_stopped_and_gone_on_with() {
    const N: usize = 3000;
    let (dir, pst) = crate::pst::test_file("pool-stop", N);
    let path = pst.to_string_lossy().into_owned();
    let store = Store::memory();
    let mut p = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    // Nothing running: nothing to stop.
    p.submit(Job::StopImport { account: "p1".into() });
    p.submit(Job::ImportPst { account: "p1".into(), path: path.clone() });
    // Stopped as soon as it says how many there are.
    let mut seen = Vec::new();
    let mut told = Vec::new();
    loop {
        let Some(e) = p.wait(Duration::from_secs(60)) else { panic!("nothing told") };
        if let Event::Imported { total, done: false, .. } = &e {
            told.push(*total);
            break;
        }
        assert!(!matches!(e, Event::Imported { done: true, .. }), "{e:?}");
    }
    assert_eq!(told, [Some(N)]);
    p.submit(Job::StopImport { account: "p1".into() });
    let end = until_done(&p, &mut seen, |_| {});
    let Event::Imported { messages, total, stopped, error, .. } = end else { panic!() };
    assert!(stopped && error.is_none() && total == Some(N) && messages < N, "{messages} of {total:?}, stopped: {stopped}, {error:?}");
    assert!(seen.iter().any(|e| matches!(e, Event::Folders { folders, .. } if folders.iter().any(|f| f.path == "INBOX"))), "its folders are told");
    assert_eq!(store.load_folder("p1", "INBOX").unwrap().messages.len(), messages);
    let journal: quatrelle_model::ImportJournal = store.load_account_doc("p1", quatrelle_model::IMPORT_DOC).unwrap();
    assert!(journal.unfinished() && journal.path == path);
    // Gone on with: the rest comes in, told with the folder it is in.
    p.submit(Job::ResumeImport { account: "p1".into(), path: path.clone(), allow_changed: false });
    let mut seen = Vec::new();
    let end = until_done(&p, &mut seen, |_| {});
    assert!(matches!(end, Event::Imported { messages: N, stopped: false, error: None, total: Some(N), .. }), "{end:?}");
    assert!(seen.iter().any(|e| matches!(e, Event::Imported { folder, done: false, .. } if folder == "INBOX")), "{seen:?}");
    assert_eq!(store.load_folder("p1", "INBOX").unwrap().messages.len(), N);
    // A file that changed is said with what the account holds, so nothing there is taken for
    // lost; an account with no journal can't be gone on with.
    std::fs::OpenOptions::new().write(true).open(&pst).unwrap().set_len(std::fs::metadata(&pst).unwrap().len() + 512).unwrap();
    p.submit(Job::ResumeImport { account: "p1".into(), path: path.clone(), allow_changed: false });
    let end = until_done(&p, &mut Vec::new(), |_| {});
    assert!(matches!(&end, Event::Imported { messages: N, error: Some(e), .. } if e == quatrelle_model::IMPORT_FILE_CHANGED), "{end:?}");
    p.submit(Job::ResumeImport { account: "p2".into(), path, allow_changed: true });
    let end = until_done(&p, &mut Vec::new(), |_| {});
    assert!(matches!(&end, Event::Imported { messages: 0, error: Some(e), .. } if e == quatrelle_model::IMPORT_NO_JOURNAL), "{end:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Found in review: a data file copied in whole whose folder list couldn't be read back (the disk
/// failing) was said to have no folders, and the import said all was well. No list is said, and
/// the import says why.
#[test]
fn a_copy_whose_folder_list_cant_be_read_back_says_so() {
    let store = Store::failing();
    let inbox = Folder { account: "m1".into(), path: "INBOX".into(), role: quatrelle_model::FolderRole::Inbox, ..Folder::default() };
    store.save_folders("m1", std::slice::from_ref(&inbox)).unwrap();
    let p = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    store.fail(Some("folders.json"), None);
    let end = pool::importing::said_copied(&store, &p.sink, "m1", 4);
    store.fail(None, None);
    let seen = p.poll();
    assert!(!seen.iter().any(|e| matches!(e, Event::Folders { .. })), "no list said: {seen:?}");
    let Event::Imported { messages: 4, error: Some(error), done: true, .. } = &end else { panic!("{end:?}") };
    assert_eq!(error, quatrelle_model::job::IMPORT_FOLDERS_UNREAD, "said as the engine knows it");
    // Read back, it's said with the import's end.
    let end = pool::importing::said_copied(&store, &p.sink, "m1", 4);
    let seen = p.poll();
    assert!(seen.iter().any(|e| matches!(e, Event::Folders { folders, .. } if *folders == [inbox.clone()])), "{seen:?}");
    assert!(matches!(end, Event::Imported { messages: 4, error: None, done: true, .. }), "{end:?}");
}

/// An import to go on with that fails says what the account holds; a folder list that can't be
/// read then (the disk failing) was taken for none, and what an earlier run brought in for
/// nothing. What the import's journal noted is said.
#[test]
fn a_failed_import_whose_folder_list_cant_be_read_says_what_came_in() {
    let store = Store::failing();
    let inbox = Folder { account: "m1".into(), path: "INBOX".into(), total: 7, ..Folder::default() };
    store.save_folders("m1", &[inbox]).unwrap();
    let journal = quatrelle_model::ImportJournal { messages: 7, ..Default::default() };
    store.save_account_doc("m1", quatrelle_model::IMPORT_DOC, &journal).unwrap();
    assert_eq!(pool::importing::kept(&store, "m1", Some(false)), 7);
    store.fail(Some("folders.json"), None);
    let kept = pool::importing::kept(&store, "m1", Some(false));
    store.fail(None, None);
    assert_eq!(kept, 7, "taken for none");
    // A first import has nothing from before.
    assert_eq!(pool::importing::kept(&store, "m1", None), 0);
}
