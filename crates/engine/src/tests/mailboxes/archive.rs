// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: IMAP mail moved into the Archives account on this computer, and put back.

use quatrelle_model::{AccountConfig, AccountRole, Folder, Security, ServerConfig};

use super::*;

fn imap(id: &str, name: &str, email: &str) -> AccountConfig {
    let server = ServerConfig { host: "imap.studio.example".into(), port: 993, security: Security::Tls };
    AccountConfig { id: id.into(), name: name.into(), email: email.into(), imap: server.clone(), smtp: server, ..Default::default() }
}

fn message(account: &str, folder: &str, uid: u32, subject: &str) -> Summary {
    let key = MessageKey { account: account.into(), folder: folder.into(), validity: 1, uid };
    Summary { key, subject: subject.into(), message_id: format!("{subject}-{uid}@studio.example").replace(' ', "-"), ..Default::default() }
}

/// Folders and messages kept for an account, as a sync would leave them.
fn keep(store: &Store, account: &str, folder: &str, subjects: &[&str]) {
    let mut list = store.load_folders(account).unwrap();
    list.push(Folder { account: account.into(), path: folder.into(), delimiter: Some('/'), selectable: true, ..Default::default() });
    store.save_folders(account, &list).unwrap();
    let mut st = store.load_folder(account, folder).unwrap();
    st.validity = 1;
    for (i, subject) in subjects.iter().enumerate() {
        st.messages.push(message(account, folder, u32::try_from(i + 1).unwrap(), subject));
    }
    store.save_folder(account, folder, &st).unwrap();
}

/// Two IMAP accounts, each with an Inbox of three messages; `a1`'s Inbox on view.
fn two_accounts(name: &str) -> Session {
    let store = temp(name);
    store.save_accounts(&[imap("a1", "Studio", "me@studio.example"), imap("a2", "Home", "me@home.example")]).unwrap();
    keep(&store, "a1", "INBOX", &["Plans", "Budget", "Rushes"]);
    keep(&store, "a2", "INBOX", &["Recipes"]);
    let mut s = Session::open(Some(store), quatrelle_model::dates::Clock::new(NOW, 0));
    s.take_jobs();
    s.run("folder.open", &json!({"account": "a1", "folder": "INBOX"})).unwrap();
    s
}

fn archives_of(s: &Session) -> Option<String> {
    s.accounts.iter().find(|a| a.config.role == Some(AccountRole::Archives)).map(|a| a.config.id.clone())
}

fn archive_jobs(jobs: Vec<Job>) -> Vec<(String, Vec<MessageKey>, String)> {
    jobs.into_iter()
        .filter_map(|j| match j {
            Job::Archive { account, keys, to_account } => Some((account, keys, to_account)),
            _ => None,
        })
        .collect()
}

#[test]
fn the_archives_account_is_made_once() {
    let mut s = two_accounts("archive-made-once");
    assert!(archives_of(&s).is_none());
    s.run("list.select", &key_of(&s, "Plans")).unwrap();
    s.run("mail.move", &json!({"account": "archives", "folder": "-"})).unwrap();
    let id = archives_of(&s).expect("made on first use");
    let made = s.account(&id).unwrap();
    assert!(made.is_local(), "no server");
    assert_eq!(made.config.name, "Archives");
    let kept = s.store.as_ref().unwrap().load_accounts().unwrap();
    assert!(kept.iter().any(|a| a.id == id && a.role == Some(AccountRole::Archives)), "kept with the others");
    let jobs = archive_jobs(s.take_jobs());
    assert_eq!(jobs.len(), 1);
    assert_eq!((jobs[0].0.as_str(), jobs[0].2.as_str()), ("a1", id.as_str()));
    // The next time it is found, by its mark, whatever it is called.
    s.accounts.iter_mut().filter(|a| a.config.id == id).for_each(|a| a.config.name = "Old mail".into());
    s.run("list.select", &key_of(&s, "Budget")).unwrap();
    s.run("mail.move", &json!({"account": id, "folder": "-"})).unwrap();
    assert_eq!(s.accounts.iter().filter(|a| a.config.role == Some(AccountRole::Archives)).count(), 1);
    assert_eq!(s.accounts.len(), 3);
    assert_eq!(archive_jobs(s.take_jobs())[0].2, id);
    done(s);
}

#[test]
fn moving_across_accounts_asks_the_worker() {
    let mut s = two_accounts("archive-asks");
    let (plans, budget) = (key_of(&s, "Plans"), key_of(&s, "Budget"));
    s.run("list.select", &plans).unwrap();
    s.run("list.extend", &budget).ok();
    let selected = s.selected().len();
    s.run("mail.move", &json!({"account": "archives", "folder": "-"})).unwrap();
    let jobs = archive_jobs(s.take_jobs());
    assert_eq!(jobs.len(), 1, "one job for the account");
    assert_eq!(jobs[0].1.len(), selected);
    // They leave the list at once, before the worker has answered.
    assert!(!s.list.iter().any(|m| m.subject == "Plans"));
    let id = archives_of(&s).unwrap();
    let from = jobs[0].1[0].clone();
    let to = MessageKey { account: id.clone(), folder: "Studio/INBOX".into(), validity: 1, uid: 1 };
    // The worker takes it out of the cache and says so before it reports (a deletion in the journal).
    s.store.as_ref().unwrap().apply_op("a1", &Op::Expunge { keys: vec![from.clone()], tidy: false }).unwrap();
    s.apply(Event::FolderChanged { account: "a1".into(), folder: "INBOX".into() });
    assert!(!s.list.iter().any(|m| m.subject == "Plans"));
    s.apply(Event::Archived { account: "a1".into(), moved: vec![(from, to)], kept: Vec::new() });
    assert!(s.notices.iter().any(|n| n.text == "1 messages archived on this computer"), "{:?}", s.notices);
    assert!(!s.list.iter().any(|m| m.subject == "Plans"), "gone for good");
    done(s);
}

#[test]
fn kept_messages_come_back_to_the_list() {
    let mut s = two_accounts("archive-kept");
    s.run("list.select", &key_of(&s, "Plans")).unwrap();
    s.run("mail.move", &json!({"account": "archives", "folder": "-"})).unwrap();
    let from = archive_jobs(s.take_jobs()).remove(0).1.remove(0);
    assert!(!s.list.iter().any(|m| m.subject == "Plans"));
    s.apply(Event::Archived { account: "a1".into(), moved: Vec::new(), kept: vec![(from, "not enough room on the disk (500 MB needed)".into())] });
    assert!(s.list.iter().any(|m| m.subject == "Plans"), "back in the list");
    assert!(s.notices.iter().any(|n| n.error && n.text.contains("not enough room")), "{:?}", s.notices);
    assert!(s.undo.is_empty(), "nothing moved, nothing to undo");
    done(s);
}

#[test]
fn other_pairs_of_accounts_are_refused() {
    let mut s = two_accounts("archive-refused");
    s.run("list.select", &key_of(&s, "Plans")).unwrap();
    let r = s.run("mail.move", &json!({"account": "a2", "folder": "INBOX"}));
    assert!(matches!(&r, Err(CmdError::Disabled(m)) if m.contains("not between these accounts yet")), "{r:?}");
    assert!(s.take_jobs().is_empty());
    assert!(s.list.iter().any(|m| m.subject == "Plans"), "it stays");
    // An account that isn't there.
    assert!(s.run("mail.move", &json!({"account": "a9", "folder": "INBOX"})).is_err());
    // Mail of the sample mailbox (no server) isn't archived either.
    assert!(archives_of(&s).is_none(), "nothing made for a refusal");
    done(s);
}

#[test]
fn undo_puts_archived_mail_back() {
    let mut s = two_accounts("archive-undo");
    s.run("list.select", &key_of(&s, "Plans")).unwrap();
    s.run("mail.move", &json!({"account": "archives", "folder": "-"})).unwrap();
    let from = archive_jobs(s.take_jobs()).remove(0).1.remove(0);
    let id = archives_of(&s).unwrap();
    let to = MessageKey { account: id, folder: "Studio/INBOX".into(), validity: 1, uid: 4 };
    s.apply(Event::Archived { account: "a1".into(), moved: vec![(from, to.clone())], kept: Vec::new() });
    s.run("edit.undo", &json!({})).unwrap();
    let jobs = s.take_jobs();
    assert!(
        matches!(jobs.as_slice(), [Job::PutBack { account, keys, to_folder }] if account == "a1" && keys.as_slice() == std::slice::from_ref(&to) && to_folder == "INBOX"),
        "{jobs:?}"
    );
    // It is told of, and puts nothing more to undo.
    s.apply(Event::PutBack { account: "a1".into(), back: vec![(to, Some(9))], kept: Vec::new() });
    assert!(s.notices.iter().any(|n| n.text == "1 messages put back on the server"), "{:?}", s.notices);
    assert!(s.undo.is_empty());
    done(s);
}

#[test]
fn archived_mail_moves_back_to_a_folder_of_the_server_and_undo_archives_it_again() {
    let mut s = two_accounts("archive-put-back");
    // The Archives account, with a message in it.
    s.run("list.select", &key_of(&s, "Plans")).unwrap();
    s.run("mail.move", &json!({"account": "archives", "folder": "-"})).unwrap();
    let from = archive_jobs(s.take_jobs()).remove(0).1.remove(0);
    let id = archives_of(&s).unwrap();
    let store = s.store.clone().unwrap();
    keep(&store, &id, "Studio/INBOX", &["Plans"]);
    let to = store.load_folder(&id, "Studio/INBOX").unwrap().messages[0].key.clone();
    s.apply(Event::Archived { account: "a1".into(), moved: vec![(from, to.clone())], kept: Vec::new() });
    s.undo.clear();
    s.apply(Event::Folders { account: id.clone(), folders: store.load_folders(&id).unwrap() });
    s.run("folder.open", &json!({"account": id, "folder": "Studio/INBOX"})).unwrap();
    s.run("list.select", &key_of(&s, "Plans")).unwrap();
    // Moving it to a folder of an account on a server puts it back.
    let r = s.run("mail.move", &json!({"account": "a1", "folder": "Nowhere"}));
    assert!(r.is_err(), "no such folder");
    s.run("mail.move", &json!({"account": "a1", "folder": "INBOX"})).unwrap();
    let jobs = s.take_jobs();
    assert!(
        matches!(jobs.as_slice(), [Job::PutBack { account, keys, to_folder }] if account == "a1" && keys.as_slice() == std::slice::from_ref(&to) && to_folder == "INBOX"),
        "{jobs:?}"
    );
    assert!(!s.list.iter().any(|m| m.subject == "Plans"), "it leaves Archives' list at once");
    // The server has it: its new key is found by its Message-ID to archive it again.
    keep(&store, "a1", "INBOX", &["Plans"]);
    s.apply(Event::PutBack { account: "a1".into(), back: vec![(to, Some(1))], kept: Vec::new() });
    s.apply(Event::FolderChanged { account: "a1".into(), folder: "INBOX".into() });
    s.run("edit.undo", &json!({})).unwrap();
    let again = archive_jobs(s.take_jobs());
    assert_eq!(again.len(), 1, "archived again");
    assert_eq!((again[0].0.as_str(), again[0].2.as_str()), ("a1", id.as_str()));
    done(s);
}

#[test]
fn the_archive_button_follows_the_accounts_setting() {
    let mut s = two_accounts("archive-button");
    // Not ticked: the server's Archive folder, as before (here none, so the move is refused).
    s.run("list.select", &key_of(&s, "Plans")).unwrap();
    let _ = s.run("mail.archive", &json!({}));
    assert!(archive_jobs(s.take_jobs()).is_empty(), "nothing goes to Archives unless the account says so");
    assert!(archives_of(&s).is_none());
    // Ticked: Archive files the selection into the Archives account.
    s.run("archive.settings", &json!({"account": "a1", "local": true})).unwrap();
    assert!(s.account("a1").unwrap().config.archive_local);
    let kept = s.store.as_ref().unwrap().load_accounts().unwrap();
    assert!(kept.iter().any(|a| a.id == "a1" && a.archive_local), "kept");
    s.run("list.select", &key_of(&s, "Plans")).unwrap();
    s.run("mail.archive", &json!({})).unwrap();
    let id = archives_of(&s).expect("made on first use");
    let jobs = archive_jobs(s.take_jobs());
    assert_eq!((jobs.len(), jobs[0].0.as_str(), jobs[0].2.as_str()), (1, "a1", id.as_str()));
    assert!(!s.list.iter().any(|m| m.subject == "Plans"), "gone from the list at once");
    // The other account is not ticked: its mail is not archived here.
    s.run("folder.open", &json!({"account": "a2", "folder": "INBOX"})).unwrap();
    s.run("list.select", &key_of(&s, "Recipes")).unwrap();
    let _ = s.run("mail.archive", &json!({}));
    assert!(archive_jobs(s.take_jobs()).is_empty());
    // Unticked again.
    s.run("archive.settings", &json!({"account": "a1", "local": false})).unwrap();
    assert!(!s.account("a1").unwrap().config.archive_local);
    done(s);
}

#[test]
fn archive_settings_are_for_imap_accounts_and_checked() {
    let mut s = two_accounts("archive-settings");
    s.run("archive.settings", &json!({"account": "a1", "afterMonths": 12})).unwrap();
    assert_eq!(s.account("a1").unwrap().config.archive_after_months, Some(12));
    s.run("archive.settings", &json!({"account": "a1", "afterMonths": null})).unwrap();
    assert_eq!(s.account("a1").unwrap().config.archive_after_months, None);
    for bad in [json!(0), json!(121), json!(-3), json!("x")] {
        assert!(s.run("archive.settings", &json!({"account": "a1", "afterMonths": bad})).is_err(), "{bad}");
    }
    assert!(s.run("archive.settings", &json!({"account": "a9", "local": true})).is_err());
    assert!(s.run("archive.settings", &json!({})).is_err());
    // Not for the Archives account itself, a local account.
    s.run("list.select", &key_of(&s, "Plans")).unwrap();
    s.run("mail.move", &json!({"account": "archives", "folder": "-"})).unwrap();
    let id = archives_of(&s).unwrap();
    assert!(s.run("archive.settings", &json!({"account": id, "local": true})).is_err());
    done(s);
}
