// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: Outlook data files opened as accounts and exported to, and an import whose
//! last save failed.

use super::*;

/// The folder list an import's end brings, as the pool sends it before saying it's done.
pub(super) fn inbox_of(s: &mut Session, id: &str) {
    let inbox = quatrelle_model::Folder { account: id.into(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
    s.apply(Event::Folders { account: id.into(), folders: vec![inbox] });
}

/// Mail came in, then the last save failed: the account and its mail stay, and it's said.
#[test]
fn a_failed_last_save_keeps_the_account() {
    let store = temp("pst-last-save");
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    let late = s.run("account.importPst", &json!({"path": "C:/Mail/late.pst"})).unwrap()["account"].as_str().unwrap().to_string();
    inbox_of(&mut s, &late);
    s.apply(Event::Imported {
        account: late.clone(),
        messages: 30,
        skipped: 0,
        total: None,
        folder: String::new(),
        stopped: false,
        done: true,
        error: Some("disk full".into()),
    });
    assert!(s.account(&late).is_some(), "what came in is kept");
    assert!(s.notices.last().is_some_and(|n| n.error && n.text.contains("30 messages imported") && n.text.contains("disk full")));
    assert_eq!(s.account(&late).unwrap().status, SyncStatus::Offline);
    let _ = std::fs::remove_dir_all(store.root());
}

/// An import whose last save failed keeps what came in past the next start: the folder list the
/// end brought is saved again. When that fails too, the notice says to import again; and a file
/// of contacts only is kept like one of mail.
#[test]
fn a_failed_last_save_is_kept_across_a_restart() {
    let store = Store::failing();
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    let late = s.run("account.importPst", &json!({"path": "C:/Mail/late.pst"})).unwrap()["account"].as_str().unwrap().to_string();
    inbox_of(&mut s, &late);
    s.apply(Event::Imported {
        account: late.clone(),
        messages: 30,
        skipped: 0,
        total: None,
        folder: String::new(),
        stopped: false,
        done: true,
        error: Some("disk full".into()),
    });
    assert!(s.notices.last().is_some_and(|n| n.error && n.text.contains("disk full") && !n.text.contains("again")));
    // A file of contacts only: they were written, the folder list after them wasn't.
    let cards = s.run("account.importPst", &json!({"path": "C:/Mail/cards.pst"})).unwrap()["account"].as_str().unwrap().to_string();
    inbox_of(&mut s, &cards);
    let mut pim = quatrelle_model::pim::Pim::default();
    pim.cards.push(quatrelle_model::pim::StoredCard {
        book: quatrelle_model::pim::LOCAL.into(),
        card: quatrelle_model::vcard::Card { uid: "c1".into(), name: "Ana Lima".into(), ..Default::default() },
        ..Default::default()
    });
    store.save_account_doc(&cards, quatrelle_model::pim::PIM_DOC, &pim).unwrap();
    s.apply(Event::Imported {
        account: cards.clone(),
        messages: 0,
        skipped: 0,
        total: None,
        folder: String::new(),
        stopped: false,
        done: true,
        error: Some("disk full".into()),
    });
    assert!(s.account(&cards).is_some(), "contacts came in: the account stays");
    assert!(s.pim[&cards].cards.iter().any(|c| c.card.name == "Ana Lima"), "and they show");
    // The folder list can't be saved again either: said, and the account has no folders after.
    store.fail(None, Some("folders.json"));
    let lost = s.run("account.importPst", &json!({"path": "C:/Mail/lost.pst"})).unwrap()["account"].as_str().unwrap().to_string();
    inbox_of(&mut s, &lost);
    s.apply(Event::Imported {
        account: lost.clone(),
        messages: 5,
        skipped: 0,
        total: None,
        folder: String::new(),
        stopped: false,
        done: true,
        error: Some("disk full".into()),
    });
    assert!(s.notices.last().is_some_and(|n| n.error && n.text.contains("Import the file again")));
    store.fail(None, None);

    let again = Session::open(Some(store), quatrelle_model::dates::Clock::new(NOW, 0));
    for id in [&late, &cards] {
        assert!(again.account(id).is_some_and(|a| a.folder(FolderRole::Inbox).is_some()), "{id} lists its folders after a restart");
    }
    assert!(again.account(&lost).is_some_and(|a| a.folders.is_empty()));
}

/// An import that brought nothing in still takes its account away again.
#[test]
fn a_failed_import_that_brought_nothing_goes() {
    let store = temp("pst-nothing");
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    let bad = s.run("account.importPst", &json!({"path": "/tmp/notes.txt"})).unwrap()["account"].as_str().unwrap().to_string();
    s.apply(Event::Imported {
        account: bad.clone(),
        messages: 0,
        skipped: 0,
        total: None,
        folder: String::new(),
        stopped: false,
        done: true,
        error: Some("not an Outlook data file".into()),
    });
    assert!(s.account(&bad).is_none());
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn outlook_data_files_open_as_accounts_on_this_computer() {
    let mut s = sample_session("pst");
    assert!(s.run("account.importPst", &json!({"path": "  "})).is_err());
    let id =
        s.run("account.importPst", &json!({"path": r"C:\Users\me\Documents\Archive 2024.pst"})).unwrap()["account"].as_str().unwrap().to_string();
    let a = s.account(&id).unwrap();
    assert_eq!((a.config.name.as_str(), a.config.email.as_str()), ("Archive 2024", "Archive 2024.pst"));
    assert!(a.is_local());
    assert!(matches!(s.take_jobs().as_slice(), [Job::ImportPst { account, path }] if *account == id && path.ends_with("Archive 2024.pst")));

    // The import reports progress, then its folders, then that it's done: the Inbox opens.
    s.apply(Event::Imported {
        account: id.clone(),
        messages: 200,
        skipped: 0,
        total: None,
        folder: String::new(),
        stopped: false,
        done: false,
        error: None,
    });
    // Followed in the status bar (`import_of`), not by a notice for each step.
    assert_eq!(s.import_of(&id).map(|i| i.done), Some(200));
    assert!(!s.notices.last().is_some_and(|n| n.text.contains("200")));
    let inbox = quatrelle_model::Folder {
        account: id.clone(),
        path: "INBOX".into(),
        delimiter: Some('/'),
        role: FolderRole::Inbox,
        selectable: true,
        ..Default::default()
    };
    s.apply(Event::Folders { account: id.clone(), folders: vec![inbox] });
    // The file's contacts were written next to its mail: they show once it's done.
    let mut pim = quatrelle_model::pim::Pim::default();
    pim.cards.push(quatrelle_model::pim::StoredCard {
        book: quatrelle_model::pim::LOCAL.into(),
        card: quatrelle_model::vcard::Card { uid: "c1".into(), name: "Ana Lima".into(), ..Default::default() },
        ..Default::default()
    });
    s.store.clone().unwrap().save_account_doc(&id, quatrelle_model::pim::PIM_DOC, &pim).unwrap();
    s.apply(Event::Imported {
        account: id.clone(),
        messages: 250,
        skipped: 2,
        total: None,
        folder: String::new(),
        stopped: false,
        done: true,
        error: None,
    });
    assert!(s.pim[&id].cards.iter().any(|c| c.card.name == "Ana Lima"));
    assert!(!s.pim[&id].calendars.is_empty(), "with a calendar to add to");
    assert!(s.notices.last().is_some_and(|n| n.text.contains("250 messages imported (2 could not be read)")));
    assert_eq!(s.view, View::Folder { account: id.clone(), folder: "INBOX".into() });
    assert_eq!(s.account(&id).unwrap().status, SyncStatus::Offline);

    // A file that isn't one: the account goes again.
    let bad = s.run("account.importPst", &json!({"path": "/tmp/notes.txt"})).unwrap()["account"].as_str().unwrap().to_string();
    s.apply(Event::Imported {
        account: bad.clone(),
        messages: 0,
        skipped: 0,
        total: None,
        folder: String::new(),
        stopped: false,
        done: true,
        error: Some("not an Outlook data file".into()),
    });
    assert!(s.account(&bad).is_none());
    assert!(s.notices.last().is_some_and(|n| n.error && n.text.contains("notes.txt could not be opened")));

    // Exporting: the current account, or the one asked for, to the file given.
    s.take_jobs();
    assert!(s.run("account.exportPst", &json!({"path": ""})).is_err());
    assert!(s.run("account.exportPst", &json!({"path": "out.pst", "account": "nope"})).is_err());
    // When flagged messages are due and the categories' names, kept only here, go along.
    s.prefs.follow_up.insert("due@example.com".into(), 20_000);
    s.prefs.follow_up_done.insert("done@example.com".into(), 19_000);
    s.run("account.exportPst", &json!({"path": "C:/out/Archive.pst", "account": id})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::ExportPst { account, path, name, marks }]
        if *account == id && path == "C:/out/Archive.pst" && name == "Archive 2024" && marks.due.get("due@example.com") == Some(&20_000)
            && marks.done.get("done@example.com") == Some(&19_000)
            && marks.categories.iter().any(|c| c.keyword == "$label4" && c.name == "To do")));
    s.apply(Event::Exported { account: id.clone(), messages: 100, skipped: 0, already: 0, occurrences: 0, replaced: 0, done: false, error: None });
    assert!(s.notices.last().is_some_and(|n| n.text.contains("100 messages exported so far")));
    s.apply(Event::Exported { account: id.clone(), messages: 120, skipped: 3, already: 0, occurrences: 0, replaced: 0, done: true, error: None });
    assert!(s.notices.last().is_some_and(|n| !n.error && n.text.contains("120 messages exported (3 not on this computer")));
    // Added to a data file that had some already: said so.
    s.apply(Event::Exported { account: id.clone(), messages: 5, skipped: 0, already: 9, occurrences: 0, replaced: 0, done: true, error: None });
    assert!(s.notices.last().is_some_and(|n| !n.error && n.text.contains("5 messages exported (9 the data file already had)")));
    s.apply(Event::Exported { account: id.clone(), messages: 5, skipped: 2, already: 9, occurrences: 0, replaced: 0, done: true, error: None });
    assert!(s.notices.last().is_some_and(|n| n.text.contains("(2 not on this computer were left out; 9 the data file already had)")));
    // A series Outlook can't keep the rule of, written one occurrence at a time: the user is told.
    s.apply(Event::Exported { account: id.clone(), messages: 5, skipped: 0, already: 0, occurrences: 40, replaced: 0, done: true, error: None });
    let last = &s.notices[s.notices.len() - 2..];
    assert!(last[0].text.contains("5 messages exported") && last[1].text.contains("40 occurrences of series whose rule Outlook can't keep"));
    // Items changed since the last export into the file, written in place of their earlier copies.
    s.apply(Event::Exported { account: id.clone(), messages: 0, skipped: 0, already: 9, occurrences: 0, replaced: 4, done: true, error: None });
    let last = &s.notices[s.notices.len() - 2..];
    assert!(last[0].text.contains("0 messages exported (9 the data file already had)"));
    assert!(last[1].text.contains("4 calendar items, contacts, notes or tasks changed since the last export replaced their earlier copies"));
    s.apply(Event::Exported {
        account: id.clone(),
        messages: 0,
        skipped: 0,
        already: 0,
        occurrences: 0,
        replaced: 0,
        done: true,
        error: Some("disk full".into()),
    });
    assert!(s.notices.last().is_some_and(|n| n.error && n.text.contains("disk full")));
    done(s);
}
