// SPDX-License-Identifier: MIT OR Apache-2.0
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
    std::fs::OpenOptions::new().append(true).open(&pst).unwrap().set_len(std::fs::metadata(&pst).unwrap().len() + 512).unwrap();
    p.submit(Job::ResumeImport { account: "p1".into(), path: path.clone(), allow_changed: false });
    let end = until_done(&p, &mut Vec::new(), |_| {});
    assert!(matches!(&end, Event::Imported { messages: N, error: Some(e), .. } if e == quatrelle_model::IMPORT_FILE_CHANGED), "{end:?}");
    p.submit(Job::ResumeImport { account: "p2".into(), path, allow_changed: true });
    let end = until_done(&p, &mut Vec::new(), |_| {});
    assert!(matches!(&end, Event::Imported { messages: 0, error: Some(e), .. } if e == quatrelle_model::IMPORT_NO_JOURNAL), "{end:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
