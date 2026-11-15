// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: Exchange's archive policy of a message and of a folder.

use quatrelle_model::{ArchiveTag, PolicyKind};

use super::*;
use crate::ARCHIVE_READ_ONLY;

const YEAR: &str = "22222222-aaaa-4bbb-8ccc-000000000002";
const NEVER: &str = "44444444-aaaa-4bbb-8ccc-000000000004";

fn tags() -> Vec<ArchiveTag> {
    vec![
        ArchiveTag { id: YEAR.into(), name: "1 Year Move to Archive".into(), days: Some(365), never: false, kind: PolicyKind::Archive },
        ArchiveTag { id: NEVER.into(), name: "Never Move to Archive".into(), days: None, never: true, kind: PolicyKind::Archive },
    ]
}

/// An account whose server offers tags, its Inbox message selected.
fn offered(name: &str) -> (Session, Store, String, MessageKey) {
    let (mut s, store, id) = with_archive(name);
    s.apply(Event::ArchiveTags { account: id.clone(), tags: tags() });
    let key = message(&id, "INBOX", 1, "").key;
    s.run("list.select", &json!({"key": key.to_id()})).unwrap();
    s.take_jobs();
    (s, store, id, key)
}

fn tag_ops(jobs: &[Job]) -> Vec<&Op> {
    jobs.iter().filter_map(|j| if let Job::Apply { op: op @ Op::SetArchiveTag { .. }, .. } = j { Some(op) } else { None }).collect()
}

/// A message is opened: its tag is asked; what comes is shown with the tag's name and the day;
/// setting one is a change for the journal (so it is made offline and sent later), shown at
/// once; "as its folder" takes it off.
#[test]
fn a_tag_set_offline_is_sent_on_reconnect() {
    let (mut s, store, id, key) = offered("policy-set");
    s.open_message(&key);
    let jobs = s.take_jobs();
    assert!(jobs.iter().any(|j| matches!(j, Job::ReadArchiveTag { key: k } if *k == key)), "{jobs:?}");
    assert_eq!(s.message_policy(&key, PolicyKind::Archive), None);
    s.apply(Event::ArchiveTagRead { key: key.clone(), tag: Some(NEVER.into()), date: None, retention_tag: None, retention_date: None });
    assert_eq!(s.message_policy(&key, PolicyKind::Archive).map(|p| p.name), Some("Never Move to Archive".into()));

    s.run("mail.archivePolicy", &json!({"tag": YEAR})).unwrap();
    let jobs = s.take_jobs();
    let ops = tag_ops(&jobs);
    assert_eq!(ops.len(), 1, "{jobs:?}");
    assert_eq!(*ops[0], Op::SetArchiveTag { keys: vec![key.clone()], folder: None, tag: Some(YEAR.into()), kind: PolicyKind::Archive });
    assert_eq!(s.message_policy(&key, PolicyKind::Archive).map(|p| p.name), Some("1 Year Move to Archive".into()));

    s.run("mail.archivePolicy", &json!({"tag": null})).unwrap();
    let jobs = s.take_jobs();
    assert_eq!(*tag_ops(&jobs)[0], Op::SetArchiveTag { keys: vec![key.clone()], folder: None, tag: None, kind: PolicyKind::Archive });
    assert_eq!(s.message_policy(&key, PolicyKind::Archive), None);

    // The day the server plans comes with what it reads afterwards, once it has the change.
    s.run("mail.archivePolicy", &json!({"tag": YEAR})).unwrap();
    s.apply(Event::ArchiveTagRead {
        key: key.clone(),
        tag: Some(YEAR.into()),
        date: Some("2027-11-10".into()),
        retention_tag: None,
        retention_date: None,
    });
    assert_eq!(s.message_policy(&key, PolicyKind::Archive).and_then(|p| p.day), Some("2027-11-10".into()));
    let _ = std::fs::remove_dir_all(store.root());
    let _ = id;
}

/// A change the server refuses, or can't make because it moved the message into the archive
/// meanwhile, puts the old tag back and says so.
#[test]
fn a_tag_the_server_refuses_is_put_back() {
    let (mut s, store, id, key) = offered("policy-refused");
    s.apply(Event::ArchiveTagRead { key: key.clone(), tag: Some(NEVER.into()), date: None, retention_tag: None, retention_date: None });
    s.run("mail.archivePolicy", &json!({"tag": YEAR})).unwrap();
    let op = tag_ops(&s.take_jobs())[0].clone();
    assert_eq!(s.message_policy(&key, PolicyKind::Archive).map(|p| p.name), Some("1 Year Move to Archive".into()));
    let before = s.notices.len();
    s.apply(Event::OpFailed { account: id.clone(), op, error: "not on the server any more".into() });
    assert_eq!(s.message_policy(&key, PolicyKind::Archive).map(|p| p.name), Some("Never Move to Archive".into()), "put back");
    assert!(s.notices.len() > before && s.notices.last().is_some_and(|n| n.error), "{:?}", s.notices.last());
    let _ = std::fs::remove_dir_all(store.root());
}

/// Review focus 5: a tag set on a message the server has meanwhile moved to the archive is
/// dropped with a notice: the failure is final (the network layer drops the change), nothing
/// is sent again from here, and the message shows what it showed before.
#[test]
fn a_tag_on_mail_gone_to_the_archive_is_dropped() {
    let (mut s, store, id, key) = offered("policy-moved");
    s.run("mail.archivePolicy", &json!({"tag": NEVER})).unwrap();
    let op = tag_ops(&s.take_jobs())[0].clone();
    s.apply(Event::OpFailed { account: id.clone(), op, error: "a message is no longer in the mailbox under that name: the server moved it".into() });
    assert_eq!(s.message_policy(&key, PolicyKind::Archive), None);
    assert!(s.take_jobs().iter().all(|j| !matches!(j, Job::Apply { .. })), "nothing is sent again");
    assert!(s.notices.last().is_some_and(|n| n.error && n.text.contains("server moved it")));
    let _ = std::fs::remove_dir_all(store.root());
}

/// A tag that isn't one of the server's is refused, and so is a command with no `tag`; with no
/// tags offered there is nothing to set.
#[test]
fn a_tag_that_isnt_the_servers_is_refused() {
    let (mut s, store, id, _key) = offered("policy-unknown");
    for params in [json!({"tag": "99999999-aaaa-4bbb-8ccc-000000000009"}), json!({"tag": "x"}), json!({}), json!({"tag": 3})] {
        let r = s.run("mail.archivePolicy", &params);
        assert!(matches!(r, Err(CmdError::Params(_))), "{params}: {r:?}");
        let mut folder = params.clone();
        folder["folder"] = json!("INBOX");
        let r = s.run("folder.archivePolicy", &folder);
        assert!(matches!(r, Err(CmdError::Params(_))), "{params}: {r:?}");
    }
    assert!(tag_ops(&s.take_jobs()).is_empty());
    // The server offers none.
    s.apply(Event::ArchiveTags { account: id, tags: Vec::new() });
    let r = s.run("mail.archivePolicy", &json!({"tag": YEAR}));
    assert!(matches!(r, Err(CmdError::Disabled(_))), "{r:?}");
    let r = s.run("folder.archivePolicy", &json!({"folder": "INBOX", "tag": null}));
    assert!(matches!(r, Err(CmdError::Disabled(_))), "{r:?}");
    assert!(tag_ops(&s.take_jobs()).is_empty());
    let _ = std::fs::remove_dir_all(store.root());
}

/// A folder's tag is set (shown at once), sent, put back when refused; taken off with null.
#[test]
fn a_folders_tag_is_set_and_put_back() {
    let (mut s, store, id, _key) = offered("policy-folder");
    s.run("folder.archivePolicy", &json!({"folder": "INBOX", "tag": YEAR})).unwrap();
    let jobs = s.take_jobs();
    let op = tag_ops(&jobs)[0].clone();
    assert_eq!(op, Op::SetArchiveTag { keys: Vec::new(), folder: Some("INBOX".into()), tag: Some(YEAR.into()), kind: PolicyKind::Archive });
    assert_eq!(s.folder_tag(&id, "INBOX", PolicyKind::Archive), Some(YEAR));
    s.apply(Event::OpFailed { account: id.clone(), op, error: "refused".into() });
    assert_eq!(s.folder_tag(&id, "INBOX", PolicyKind::Archive), None);
    s.run("folder.archivePolicy", &json!({"folder": "INBOX", "tag": YEAR})).unwrap();
    s.run("folder.archivePolicy", &json!({"folder": "INBOX", "tag": null})).unwrap();
    assert_eq!(s.folder_tag(&id, "INBOX", PolicyKind::Archive), None);
    let _ = std::fs::remove_dir_all(store.root());
}

/// In the online archive the policy isn't offered: not on its mail, not on its folders, and a
/// selection mixing them is refused whole; opening its mail asks for no tag.
#[test]
fn the_archive_policy_isnt_offered_in_the_archive() {
    let (mut s, store, id, _key) = offered("policy-archive");
    s.run("folder.open", &json!({"account": id, "folder": PROJECTS})).unwrap();
    s.take_jobs();
    listed(&mut s, &store, &id, PROJECTS, vec![message(&id, PROJECTS, 1, "Old 1")]);
    let old = message(&id, PROJECTS, 1, "").key;
    s.run("list.select", &json!({"key": old.to_id()})).unwrap();
    s.open_message(&old);
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::ReadArchiveTag { .. })), "no tag asked for archive mail");
    let r = s.run("mail.archivePolicy", &json!({"tag": YEAR}));
    assert!(matches!(&r, Err(CmdError::Disabled(why)) if why == ARCHIVE_READ_ONLY), "{r:?}");
    for folder in [PROJECTS, TOP] {
        let r = s.run("folder.archivePolicy", &json!({"folder": folder, "tag": YEAR}));
        assert!(matches!(&r, Err(CmdError::Disabled(why)) if why == ARCHIVE_READ_ONLY), "{folder}: {r:?}");
    }
    assert!(tag_ops(&s.take_jobs()).is_empty());
    assert!(!s.offers_policy(&id, PROJECTS) && s.offers_policy(&id, "INBOX"));
    let _ = std::fs::remove_dir_all(store.root());
}

/// No tags of the message's own, nor a day: what a read of a message without them answers.
fn read(key: &MessageKey, archive: Option<&str>, day: Option<&str>) -> Event {
    Event::ArchiveTagRead {
        key: key.clone(),
        tag: archive.map(str::to_string),
        date: day.map(str::to_string),
        retention_tag: None,
        retention_date: None,
    }
}

/// A tag set showed « As its folder » once the read asked when the message was opened came
/// back, answered before the change reached the server (the read goes ahead of the journal). A
/// change waiting stands over a read that doesn't carry it; a read that carries it (its id in
/// capitals too) ends the wait, with its day; then what the server says is shown again.
#[test]
fn a_tag_set_stands_over_a_read_from_before_it() {
    let (mut s, store, _id, key) = offered("policy-stale");
    s.open_message(&key);
    s.take_jobs();
    s.run("mail.archivePolicy", &json!({"tag": NEVER})).unwrap();
    assert_eq!(s.message_tag(&key, PolicyKind::Archive), Some(NEVER), "at once");
    s.apply(read(&key, None, None));
    assert_eq!(s.message_tag(&key, PolicyKind::Archive), Some(NEVER), "the change still waits");
    s.apply(read(&key, Some(&NEVER.to_ascii_uppercase()), Some("2027-11-12")));
    let p = s.message_policy(&key, PolicyKind::Archive).expect("the tag");
    assert_eq!((p.id.as_str(), p.day.as_deref()), (NEVER, Some("2027-11-12")));
    s.apply(read(&key, None, None));
    assert_eq!(s.message_tag(&key, PolicyKind::Archive), None, "the server's word again");
    let _ = std::fs::remove_dir_all(store.root());
}

/// A tag set stands however many other messages' tags are read meanwhile (the memory of them was
/// emptied whole once full, the change with it).
#[test]
fn a_tag_set_stands_while_many_others_are_read() {
    let (mut s, store, _id, key) = offered("policy-many");
    s.run("mail.archivePolicy", &json!({"tag": NEVER})).unwrap();
    for uid in 100..700 {
        s.apply(read(&MessageKey { uid, ..key.clone() }, Some(YEAR), None));
    }
    assert_eq!(s.message_tag(&key, PolicyKind::Archive), Some(NEVER));
    let _ = std::fs::remove_dir_all(store.root());
}

/// A message only selected (the right-click menu on one not open), or opened before the server
/// told its tags, had none read: `mail.readPolicy` asks for them, once a message however often
/// it runs (the submenu runs it each frame it is drawn until they are known); the answer's id in
/// capitals names the offered tag; a message with no tag of its own is known to have none.
#[test]
fn a_message_not_opened_has_its_tags_read_once() {
    let reads = |s: &mut Session| s.take_jobs().iter().filter(|j| matches!(j, Job::ReadArchiveTag { .. })).count();
    let (mut s, store, id, key) = offered("policy-ask");
    assert!(!s.message_tags_known(&key));
    for _ in 0..3 {
        s.run("mail.readPolicy", &json!({"key": key.to_id()})).unwrap();
    }
    assert_eq!(reads(&mut s), 1, "one read");
    assert!(s.message_tags_known(&key), "asked");
    s.apply(read(&key, Some(&NEVER.to_ascii_uppercase()), None));
    assert_eq!(s.message_tag(&key, PolicyKind::Archive), Some(NEVER));
    assert_eq!(s.message_policy(&key, PolicyKind::Archive).map(|p| p.name), Some("Never Move to Archive".into()));
    s.apply(read(&key, None, None));
    assert_eq!(s.message_tag(&key, PolicyKind::Archive), None, "none of its own: as its folder");
    s.run("mail.readPolicy", &json!({"key": key.to_id()})).unwrap();
    assert_eq!(reads(&mut s), 0, "known");
    assert!(s.run("mail.readPolicy", &json!({"key": "junk"})).is_err());
    let _ = std::fs::remove_dir_all(store.root());
    let _ = id;

    // Opened before the server told its tags: nothing was asked then.
    let (mut s, store, id) = with_archive("policy-early");
    let key = message(&id, "INBOX", 1, "").key;
    s.open_message(&key);
    assert_eq!(reads(&mut s), 0);
    s.apply(Event::ArchiveTags { account: id.clone(), tags: tags() });
    assert!(!s.message_tags_known(&key));
    s.run("mail.readPolicy", &json!({"key": key.to_id()})).unwrap();
    assert_eq!(reads(&mut s), 1);
    let _ = std::fs::remove_dir_all(store.root());
}

/// Older mail, read on the server and never kept, has its tags read when it is opened, as the
/// submenu asks for them when it is only selected; a tag set on it is sent and ticked at once.
#[test]
fn older_mail_has_its_tags_read_and_set() {
    let reads = |s: &mut Session| s.take_jobs().iter().filter(|j| matches!(j, Job::ReadArchiveTag { .. })).count();
    let (mut s, store, id, _key) = offered("policy-older");
    let old = |uid: u32| Summary { subject: format!("Two years ago {uid}"), ..message(&id, "INBOX", uid, "") };
    s.apply(Event::Older { account: id.clone(), folder: "INBOX".into(), items: vec![old(7), old(8)], more: false, error: None });
    let key = old(7).key;
    s.open_message(&key);
    let jobs = s.take_jobs();
    assert!(jobs.iter().any(|j| matches!(j, Job::ReadOnline { key: k } if *k == key)), "read on the server: {jobs:?}");
    assert!(jobs.iter().any(|j| matches!(j, Job::ReadArchiveTag { key: k } if *k == key)), "{jobs:?}");
    let selected = old(8).key;
    s.run("list.select", &json!({"key": selected.to_id()})).unwrap();
    s.run("mail.readPolicy", &json!({"key": selected.to_id()})).unwrap();
    assert_eq!(reads(&mut s), 1, "only selected: asked for the submenu");
    s.apply(read(&selected, Some(NEVER), None));
    assert_eq!(s.message_tag(&selected, PolicyKind::Archive), Some(NEVER));
    s.open_message(&key);
    s.take_jobs();
    s.apply(read(&key, Some(YEAR), None));
    assert_eq!(s.message_tag(&key, PolicyKind::Archive), Some(YEAR));
    s.run("mail.archivePolicy", &json!({"tag": NEVER})).unwrap();
    let jobs = s.take_jobs();
    assert_eq!(*tag_ops(&jobs)[0], Op::SetArchiveTag { keys: vec![key.clone()], folder: None, tag: Some(NEVER.into()), kind: PolicyKind::Archive });
    assert_eq!(s.message_tag(&key, PolicyKind::Archive), Some(NEVER));
    let _ = std::fs::remove_dir_all(store.root());
}
