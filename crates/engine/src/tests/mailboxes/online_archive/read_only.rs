// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: the online archive is read-only in this version, whatever the way in.

use super::*;
use crate::ARCHIVE_READ_ONLY;

fn changes(jobs: &[Job]) -> Vec<&Job> {
    jobs.iter().filter(|j| matches!(j, Job::Apply { .. })).collect()
}

/// The archive's Projects folder on view with two messages, one selected.
fn archive_open(name: &str) -> (Session, Store, String, MessageKey) {
    let (mut s, store, id) = with_archive(name);
    s.run("folder.open", &json!({"account": id, "folder": PROJECTS})).unwrap();
    s.take_jobs();
    listed(&mut s, &store, &id, PROJECTS, vec![message(&id, PROJECTS, 1, "Old 1"), message(&id, PROJECTS, 2, "Old 2")]);
    let key = message(&id, PROJECTS, 1, "").key;
    s.run("list.select", &json!({"key": key.to_id()})).unwrap();
    s.take_jobs();
    (s, store, id, key)
}

fn refused(s: &mut Session, id: &str, params: &Value) {
    let r = s.run(id, params);
    assert!(matches!(&r, Err(CmdError::Disabled(why)) if why == ARCHIVE_READ_ONLY), "{id} {params}: {r:?}");
    let jobs = s.take_jobs();
    assert!(changes(&jobs).is_empty(), "{id} {params}: the server is asked for {jobs:?}");
}

/// Every way of moving, deleting, archiving or making folders is refused with the one sentence
/// and asks the server for nothing; marking read works and Reply writes in the account.
#[test]
fn the_online_archive_refuses_every_change() {
    let (mut s, store, id, key) = archive_open("online-archive-refuses");
    let before = s.list.len();
    refused(&mut s, "mail.move", &json!({"folder": "Deleted Items"}));
    refused(&mut s, "mail.move", &json!({"folder": OLD_INBOX}));
    refused(&mut s, "mail.move", &json!({"folder": "role:trash"}));
    refused(&mut s, "mail.move", &json!({"folder": "INBOX", "account": id}));
    refused(&mut s, "mail.move", &json!({"folder": "Inbox", "account": "archives"}));
    refused(&mut s, "mail.delete", &json!({}));
    refused(&mut s, "mail.delete", &json!({"permanent": true}));
    refused(&mut s, "mail.deleteForGood", &json!({}));
    refused(&mut s, "mail.archive", &json!({}));
    refused(&mut s, "mail.junk", &json!({}));
    refused(&mut s, "mail.notJunk", &json!({}));
    refused(&mut s, "mail.ignore", &json!({}));
    refused(&mut s, "mail.snooze", &json!({"preset": "later"}));
    refused(&mut s, "mail.cleanUp", &json!({}));
    refused(&mut s, "junk.blockSender", &json!({}));
    assert!(s.prefs.blocked_senders.is_empty(), "nothing is half done");
    // A Quick Step that moves, however it is run; one that only marks goes through.
    s.run("quickStep.create", &json!({"name": "File", "actions": [{"do": "markRead"}, {"do": "moveTo", "value": "Sent Items"}]})).unwrap();
    s.run("quickStep.create", &json!({"name": "Bin", "actions": [{"do": "delete"}]})).unwrap();
    s.run("quickStep.create", &json!({"name": "Mark", "actions": [{"do": "markRead"}]})).unwrap();
    refused(&mut s, "quickStep.run", &json!({"name": "File"}));
    refused(&mut s, "quickStep.run", &json!({"name": "Bin"}));
    refused(&mut s, "quickStep.1", &json!({}));
    assert!(s.summary(&key).is_some_and(|m| !m.flags.seen), "the step that refused marked nothing");
    s.run("quickStep.run", &json!({"name": "Mark"})).unwrap();
    s.take_jobs();
    // Folders under the archive, and the archive's own.
    for (cmd, params) in [
        ("folder.create", json!({"name": "New", "parent": TOP})),
        ("folder.create", json!({"name": "New", "parent": PROJECTS})),
        ("folder.rename", json!({"folder": PROJECTS, "name": "Other"})),
        ("folder.rename", json!({"folder": TOP, "name": "Other"})),
        ("folder.move", json!({"folder": PROJECTS, "parent": ""})),
        ("folder.delete", json!({"folder": PROJECTS})),
        ("folder.delete", json!({"folder": PROJECTS, "permanent": true})),
        ("folder.empty", json!({"folder": format!("{TOP}/Deleted Items")})),
        ("folder.empty", json!({"folder": PROJECTS})),
    ] {
        let r = s.run(cmd, &params);
        assert!(matches!(&r, Err(CmdError::Disabled(why)) if why == ARCHIVE_READ_ONLY), "{cmd} {params}: {r:?}");
        assert!(changes(&s.take_jobs()).is_empty(), "{cmd} {params}");
    }
    // A folder of the account moved under the archive.
    s.run("folder.create", &json!({"name": "Mine"})).unwrap();
    s.take_jobs();
    let r = s.run("folder.move", &json!({"folder": "Mine", "parent": PROJECTS}));
    assert!(matches!(&r, Err(CmdError::Disabled(why)) if why == ARCHIVE_READ_ONLY), "{r:?}");
    assert_eq!(s.list.len(), before, "nothing left the list");
    assert!(s.account(&id).unwrap().folders.iter().any(|f| f.path == PROJECTS));
    // Marks reach the server.
    s.run("mail.markUnread", &json!({})).unwrap();
    s.run("mail.markRead", &json!({})).unwrap();
    let jobs = s.take_jobs();
    assert!(jobs.iter().any(|j| matches!(j, Job::Apply { op: Op::SetFlag { flag: Flag::Seen, on: true, .. }, .. })), "{jobs:?}");
    // Reply opens a message in the account.
    store.write_body(&key, b"From: Zoe Park <zoe@contoso.example>\r\nTo: riley@contoso.example\r\nSubject: Old 1\r\n\r\nHello.\r\n").unwrap();
    s.open_message(&key);
    s.run("mail.reply", &json!({})).unwrap();
    assert_eq!(s.compose.as_ref().map(|c| c.account.as_str()), Some(id.as_str()));
    let _ = std::fs::remove_dir_all(store.root());
}

/// Mail of the mailbox filed into a folder of the archive: refused, as a move, a drag, a Quick
/// Step and a rule would do it.
#[test]
fn nothing_can_be_filed_into_the_archive() {
    let (mut s, store, id) = with_archive("online-archive-into");
    let key = message(&id, "INBOX", 1, "").key;
    s.run("list.select", &json!({"key": key.to_id()})).unwrap();
    refused(&mut s, "mail.move", &json!({"folder": PROJECTS}));
    refused(&mut s, "mail.move", &json!({"folder": TOP}));
    s.run("quickStep.create", &json!({"name": "Old", "actions": [{"do": "moveTo", "value": PROJECTS}]})).unwrap();
    refused(&mut s, "quickStep.run", &json!({"name": "Old"}));
    assert_eq!(s.list.len(), 1, "the message is still in the Inbox");
    // The mailbox's own mail still moves.
    s.run("mail.move", &json!({"folder": "Sent Items"})).unwrap();
    assert!(!changes(&s.take_jobs()).is_empty());
    let _ = std::fs::remove_dir_all(store.root());
}

/// Review focus 4: a selection of mailbox mail and archive mail (a search result) is refused
/// whole; the mailbox's mail isn't deleted either.
#[test]
fn a_selection_with_archive_mail_is_refused_whole() {
    let (mut s, store, id) = with_archive("online-archive-mixed");
    s.run("folder.open", &json!({"account": id, "folder": PROJECTS})).unwrap();
    s.take_jobs();
    s.run("search.run", &json!({"query": "harbour", "scope": "mailbox"})).unwrap();
    s.take_jobs();
    let (mine, old) = (message(&id, "INBOX", 1, "harbour plans"), message(&id, PROJECTS, 9, "The harbour in 2019"));
    s.apply(Event::Found { account: id.clone(), query: "harbour".into(), items: vec![mine.clone(), old.clone()], error: None });
    assert_eq!(s.list.len(), 2, "{:?}", s.list.iter().map(|m| &m.subject).collect::<Vec<_>>());
    s.run("list.select", &json!({"key": mine.key.to_id()})).unwrap();
    s.run("list.select", &json!({"key": old.key.to_id(), "add": true})).unwrap();
    assert_eq!(s.selection.len(), 2);
    for (cmd, params) in [
        ("mail.delete", json!({})),
        ("mail.deleteForGood", json!({})),
        ("mail.move", json!({"folder": "Sent Items"})),
        ("mail.archive", json!({})),
        ("mail.junk", json!({})),
        ("mail.snooze", json!({"preset": "later"})),
    ] {
        refused(&mut s, cmd, &params);
    }
    assert_eq!(s.list.len(), 2);
    assert_eq!(s.selection.len(), 2, "the selection is kept");
    assert!(s.folder_state(&id, "INBOX").messages.iter().any(|m| m.key == mine.key), "the Inbox still holds it");
    let _ = std::fs::remove_dir_all(store.root());
}

/// The commands say they are off with a selection in the archive, so the window greys them; the
/// ones that only read or mark stay on.
#[test]
fn the_commands_that_change_mail_are_off_in_the_archive() {
    let (mut s, store, _id, _key) = archive_open("online-archive-off");
    assert!(s.read_only_folder(&_id, PROJECTS) && !s.read_only_folder(&_id, "INBOX"));
    let off = |s: &Session, id: &str| s.registry().get(id).and_then(|c| (c.enabled)(s));
    for c in
        ["mail.move", "mail.delete", "mail.deleteForGood", "mail.archive", "mail.junk", "mail.notJunk", "mail.ignore", "mail.snooze", "mail.cleanUp"]
    {
        assert_eq!(off(&s, c), Some(ARCHIVE_READ_ONLY), "{c}");
    }
    s.run("quickStep.create", &json!({"name": "Bin", "actions": [{"do": "delete"}]})).unwrap();
    s.run("quickStep.create", &json!({"name": "Mark", "actions": [{"do": "markRead"}]})).unwrap();
    assert_eq!(off(&s, "quickStep.1"), Some(ARCHIVE_READ_ONLY));
    assert_eq!(off(&s, "quickStep.2"), None);
    for c in ["mail.markRead", "mail.markUnread", "mail.flag"] {
        assert_eq!(off(&s, c), None, "{c}");
    }
    let _ = std::fs::remove_dir_all(store.root());
}
