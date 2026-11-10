// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: a data file's import followed, stopped and gone on with.

use super::data_files::inbox_of;
use super::*;
use quatrelle_model::{IMPORT_DOC, IMPORT_FILE_CHANGED, IMPORT_NO_JOURNAL, ImportJournal};

fn progress(s: &mut Session, id: &str, messages: usize, total: Option<usize>, folder: &str) {
    s.apply(Event::Imported { account: id.into(), messages, skipped: 0, total, folder: folder.into(), stopped: false, done: false, error: None });
}

fn ended(s: &mut Session, id: &str, messages: usize, total: Option<usize>, stopped: bool, error: Option<&str>) {
    s.apply(Event::Imported {
        account: id.into(),
        messages,
        skipped: 0,
        total,
        folder: String::new(),
        stopped,
        done: true,
        error: error.map(str::to_string),
    });
}

/// What an import stopped after `done` messages leaves in the cache.
fn journal(store: &Store, id: &str, done: usize, total: Option<usize>) {
    let j = ImportJournal { path: "C:/Mail/big.pst".into(), total, messages: done, ..Default::default() };
    store.save_account_doc(id, IMPORT_DOC, &j).unwrap();
}

fn open_import(s: &mut Session, name: &str) -> String {
    let id = s.run("account.importPst", &json!({"path": format!("C:/Mail/{name}.pst")})).unwrap()["account"].as_str().unwrap().to_string();
    inbox_of(s, &id);
    s.take_jobs();
    id
}

#[test]
fn stop_and_resume_are_enabled_only_when_they_apply() {
    let store = temp("import-enabled");
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    let enabled = |s: &Session, id: &str| s.registry().get(id).is_some_and(|c| (c.enabled)(s).is_none());
    assert!(!enabled(&s, "account.stopImport") && !enabled(&s, "account.resumeImport"), "no import: nothing to stop or go on with");
    let id = s.run("account.importPst", &json!({"path": "C:/Mail/big.pst"})).unwrap()["account"].as_str().unwrap().to_string();
    assert!(enabled(&s, "account.stopImport") && !enabled(&s, "account.resumeImport"), "running: stop only");
    assert!(s.run("account.resumeImport", &json!({"account": id})).is_err());
    progress(&mut s, &id, 400, Some(3000), "INBOX/Caf&AOk-");
    let state = s.import_of(&id).unwrap();
    assert_eq!((state.done, state.total, state.folder.as_str()), (400, Some(3000), "Café"));
    s.take_jobs();
    s.run("account.stopImport", &json!({})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::StopImport { account }] if *account == id));
    assert!(!enabled(&s, "account.stopImport"), "already stopping");
    assert!(s.import_of(&id).unwrap().stopping);
    journal(&store, &id, 450, Some(3000));
    ended(&mut s, &id, 450, Some(3000), true, None);
    assert!(s.import_of(&id).is_none() && s.account(&id).is_some());
    assert!(!enabled(&s, "account.stopImport") && enabled(&s, "account.resumeImport"), "stopped: go on only");
    assert!(
        s.notices.last().is_some_and(
            |n| !n.error && n.text == "big.pst: import stopped, 450 of 3000 messages are here. Right-click the account to go on with it."
        )
    );
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn an_import_without_a_total_says_so_in_its_notice() {
    let store = temp("import-no-total");
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = open_import(&mut s, "plain");
    journal(&store, &id, 12, None);
    ended(&mut s, &id, 12, None, true, None);
    assert!(s.notices.last().is_some_and(|n| n.text == "plain.pst: import stopped, 12 messages are here. Right-click the account to go on with it."));
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn an_unfinished_import_is_marked_after_a_restart_and_never_started_by_itself() {
    let store = temp("import-restart");
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = open_import(&mut s, "big");
    journal(&store, &id, 200, Some(900));
    drop(s);
    let mut again = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    let left = again.unfinished_import(&id).unwrap().clone();
    assert_eq!((left.path.as_str(), left.done, left.total), ("C:/Mail/big.pst", 200, Some(900)));
    assert!(again.import_of(&id).is_none());
    assert!(!again.take_jobs().iter().any(|j| matches!(j, Job::ImportPst { .. } | Job::ResumeImport { .. })), "only a click goes on with it");
    again.run("account.resumeImport", &json!({"account": id})).unwrap();
    assert!(matches!(again.take_jobs().as_slice(), [Job::ResumeImport { path, allow_changed: false, .. }] if path == "C:/Mail/big.pst"));
    let state = again.import_of(&id).unwrap();
    assert_eq!((state.done, state.total), (200, Some(900)), "the bar goes on from where it was");
    assert!(again.unfinished_import(&id).is_none(), "running, so not offered again");
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn a_stopped_account_stays_even_with_nothing_in_when_its_resume_fails() {
    let store = temp("import-stays");
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = open_import(&mut s, "empty");
    journal(&store, &id, 0, Some(50));
    ended(&mut s, &id, 0, Some(50), true, None);
    assert!(s.account(&id).is_some(), "stopped at 0 messages: the account stays");
    // The file changed: the question is raised, nothing is said as an error, the account stays.
    s.run("account.resumeImport", &json!({"account": id, "path": "D:/Mail/moved.pst"})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::ResumeImport { path, .. }] if path == "D:/Mail/moved.pst"));
    ended(&mut s, &id, 0, None, false, Some(IMPORT_FILE_CHANGED));
    assert!(s.account(&id).is_some());
    assert_eq!(s.ask_import, vec![id.clone()]);
    assert!(s.unfinished_import(&id).is_some(), "still to go on with");
    // Yes: it goes on, bringing in what is new.
    s.ask_import.clear();
    s.run("account.resumeImport", &json!({"account": id, "allowChanged": true})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::ResumeImport { allow_changed: true, .. }]));
    // Any other failure of a resume keeps the account too.
    ended(&mut s, &id, 0, None, false, Some("No such file"));
    assert!(s.account(&id).is_some());
    assert!(s.notices.last().is_some_and(|n| n.error && n.text.contains("No such file")));
    assert!(s.ask_import.is_empty());
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn an_import_with_no_journal_to_go_on_with_is_said() {
    let store = temp("import-no-journal");
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = open_import(&mut s, "lost");
    journal(&store, &id, 5, None);
    ended(&mut s, &id, 5, None, true, None);
    s.run("account.resumeImport", &json!({"account": id})).unwrap();
    // The journal went (damaged, set aside) before the resume read it.
    store.remove_account_doc(&id, IMPORT_DOC).unwrap();
    ended(&mut s, &id, 5, None, false, Some(IMPORT_NO_JOURNAL));
    assert!(s.account(&id).is_some());
    assert!(s.notices.last().is_some_and(|n| n.error && n.text.contains("Open the file again")));
    assert!(s.unfinished_import(&id).is_none(), "it can only be started over");
    assert!(s.run("account.resumeImport", &json!({"account": id})).is_err());
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn a_first_import_that_fails_with_no_journal_still_goes() {
    let store = temp("import-first-fails");
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = open_import(&mut s, "broken");
    ended(&mut s, &id, 0, None, false, Some("not a data file"));
    assert!(s.account(&id).is_none());
    let _ = std::fs::remove_dir_all(store.root());
}
