// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: an Outlook data file import whose last save failed.

use super::*;

/// The folder list an import's end brings, as the pool sends it before saying it's done.
fn inbox_of(s: &mut Session, id: &str) {
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
    s.apply(Event::Imported { account: late.clone(), messages: 30, skipped: 0, done: true, error: Some("disk full".into()) });
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
    s.apply(Event::Imported { account: late.clone(), messages: 30, skipped: 0, done: true, error: Some("disk full".into()) });
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
    s.apply(Event::Imported { account: cards.clone(), messages: 0, skipped: 0, done: true, error: Some("disk full".into()) });
    assert!(s.account(&cards).is_some(), "contacts came in: the account stays");
    assert!(s.pim[&cards].cards.iter().any(|c| c.card.name == "Ana Lima"), "and they show");
    // The folder list can't be saved again either: said, and the account has no folders after.
    store.fail(None, Some("folders.json"));
    let lost = s.run("account.importPst", &json!({"path": "C:/Mail/lost.pst"})).unwrap()["account"].as_str().unwrap().to_string();
    inbox_of(&mut s, &lost);
    s.apply(Event::Imported { account: lost.clone(), messages: 5, skipped: 0, done: true, error: Some("disk full".into()) });
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
    s.apply(Event::Imported { account: bad.clone(), messages: 0, skipped: 0, done: true, error: Some("not an Outlook data file".into()) });
    assert!(s.account(&bad).is_none());
    let _ = std::fs::remove_dir_all(store.root());
}
