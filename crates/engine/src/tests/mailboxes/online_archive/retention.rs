// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: Exchange's retention policy (the tags that delete mail) of a message and of a
//! folder, beside its archive policy.

use quatrelle_model::{ArchiveTag, PolicyKind};

use super::*;

const YEAR: &str = "22222222-aaaa-4bbb-8ccc-000000000002";
const DELETE: &str = "55555555-aaaa-4bbb-8ccc-000000000005";
const WEEK: &str = "66666666-aaaa-4bbb-8ccc-000000000006";

fn tags() -> Vec<ArchiveTag> {
    let retention = |id: &str, name: &str, days: u32| ArchiveTag {
        id: id.into(),
        name: name.into(),
        days: Some(days),
        never: false,
        kind: PolicyKind::Retention,
    };
    vec![
        ArchiveTag { id: YEAR.into(), name: "1 Year Move to Archive".into(), days: Some(365), never: false, kind: PolicyKind::Archive },
        retention(DELETE, "3 Year Delete", 1095),
        retention(WEEK, "1 Week Delete", 7),
    ]
}

/// An account whose server offers archive and retention tags, its Inbox message selected.
fn offered(name: &str) -> (Session, Store, String, MessageKey) {
    let (mut s, store, id) = with_archive(name);
    s.apply(Event::ArchiveTags { account: id.clone(), tags: tags() });
    let key = message(&id, "INBOX", 1, "").key;
    s.run("list.select", &json!({"key": key.to_id()})).unwrap();
    s.take_jobs();
    (s, store, id, key)
}

fn tag_ops(jobs: &[Job]) -> Vec<Op> {
    jobs.iter().filter_map(|j| if let Job::Apply { op: op @ Op::SetArchiveTag { .. }, .. } = j { Some(op.clone()) } else { None }).collect()
}

fn names(s: &Session, account: &str, kind: PolicyKind) -> Vec<String> {
    s.policy_tags(account, kind).map(|t| t.name.clone()).collect()
}

/// The tags told are told apart by kind; a message's retention tag and the day the server plans
/// to delete it are read with its archive tag, each shown with its own name and day.
#[test]
fn a_messages_retention_tag_is_read_beside_its_archive_tag() {
    let (mut s, store, id, key) = offered("retention-read");
    assert_eq!(names(&s, &id, PolicyKind::Archive), ["1 Year Move to Archive"]);
    assert_eq!(names(&s, &id, PolicyKind::Retention), ["3 Year Delete", "1 Week Delete"]);
    s.apply(Event::ArchiveTagRead {
        key: key.clone(),
        tag: Some(YEAR.into()),
        date: Some("2027-11-12".into()),
        retention_tag: Some(WEEK.into()),
        retention_date: Some("2026-11-19".into()),
    });
    let archive = s.message_policy(&key, PolicyKind::Archive).unwrap();
    let retention = s.message_policy(&key, PolicyKind::Retention).unwrap();
    assert_eq!((archive.name.as_str(), archive.day.as_deref()), ("1 Year Move to Archive", Some("2027-11-12")));
    assert_eq!((retention.name.as_str(), retention.day.as_deref()), ("1 Week Delete", Some("2026-11-19")));
    assert_eq!(s.message_tag(&key, PolicyKind::Retention), Some(WEEK));
    // A retention id the server didn't offer as one names nothing.
    s.apply(Event::ArchiveTagRead { key: key.clone(), tag: None, date: None, retention_tag: Some(YEAR.into()), retention_date: None });
    assert_eq!(s.message_policy(&key, PolicyKind::Retention), None);
    assert_eq!(s.message_policy(&key, PolicyKind::Archive), None);
    let _ = std::fs::remove_dir_all(store.root());
}

/// `mail.retentionPolicy` queues a retention change for the journal, shown at once, leaving the
/// archive tag as it is; null takes it off; the server's refusal puts the old one back.
#[test]
fn a_messages_retention_tag_is_set_taken_off_and_put_back() {
    let (mut s, store, id, key) = offered("retention-set");
    s.apply(Event::ArchiveTagRead { key: key.clone(), tag: Some(YEAR.into()), date: None, retention_tag: Some(DELETE.into()), retention_date: None });
    s.run("mail.retentionPolicy", &json!({"tag": WEEK})).unwrap();
    let ops = tag_ops(&s.take_jobs());
    assert_eq!(ops, [Op::SetArchiveTag { keys: vec![key.clone()], folder: None, tag: Some(WEEK.into()), kind: PolicyKind::Retention }]);
    assert_eq!(s.message_tag(&key, PolicyKind::Retention), Some(WEEK));
    assert_eq!(s.message_tag(&key, PolicyKind::Archive), Some(YEAR), "the archive tag stays");

    s.apply(Event::OpFailed { account: id.clone(), op: ops[0].clone(), error: "refused".into() });
    assert_eq!(s.message_tag(&key, PolicyKind::Retention), Some(DELETE), "put back");
    assert_eq!(s.message_tag(&key, PolicyKind::Archive), Some(YEAR));

    s.run("mail.retentionPolicy", &json!({"tag": null})).unwrap();
    let ops = tag_ops(&s.take_jobs());
    assert_eq!(ops, [Op::SetArchiveTag { keys: vec![key.clone()], folder: None, tag: None, kind: PolicyKind::Retention }]);
    assert_eq!(s.message_policy(&key, PolicyKind::Retention), None);
    assert_eq!(s.message_tag(&key, PolicyKind::Archive), Some(YEAR));
    let _ = std::fs::remove_dir_all(store.root());
}

/// `folder.retentionPolicy` sets a folder's retention tag apart from its archive tag, put back
/// when refused, taken off with null.
#[test]
fn a_folders_retention_tag_is_set_and_put_back() {
    let (mut s, store, id, _key) = offered("retention-folder");
    s.run("folder.archivePolicy", &json!({"folder": "INBOX", "tag": YEAR})).unwrap();
    s.run("folder.retentionPolicy", &json!({"folder": "INBOX", "tag": WEEK, "account": id})).unwrap();
    let ops = tag_ops(&s.take_jobs());
    let retention = Op::SetArchiveTag { keys: Vec::new(), folder: Some("INBOX".into()), tag: Some(WEEK.into()), kind: PolicyKind::Retention };
    assert_eq!(ops.last(), Some(&retention));
    assert_eq!((s.folder_tag(&id, "INBOX", PolicyKind::Archive), s.folder_tag(&id, "INBOX", PolicyKind::Retention)), (Some(YEAR), Some(WEEK)));
    s.apply(Event::OpFailed { account: id.clone(), op: retention, error: "refused".into() });
    assert_eq!((s.folder_tag(&id, "INBOX", PolicyKind::Archive), s.folder_tag(&id, "INBOX", PolicyKind::Retention)), (Some(YEAR), None));
    s.run("folder.retentionPolicy", &json!({"folder": "INBOX", "tag": DELETE})).unwrap();
    s.run("folder.retentionPolicy", &json!({"folder": "INBOX", "tag": null})).unwrap();
    assert_eq!(s.folder_tag(&id, "INBOX", PolicyKind::Retention), None);
    assert_eq!(s.folder_tag(&id, "INBOX", PolicyKind::Archive), Some(YEAR));
    let _ = std::fs::remove_dir_all(store.root());
}

/// An archive tag isn't a retention tag nor the other way round; a command without `tag`, or
/// with junk, is refused; nothing is queued.
#[test]
fn a_tag_of_the_other_kind_is_refused() {
    let (mut s, store, _id, _key) = offered("retention-wrong-kind");
    let refused = [json!({"tag": YEAR}), json!({"tag": "x"}), json!({}), json!({"tag": [WEEK]}), json!(null)];
    for params in refused {
        let r = s.run("mail.retentionPolicy", &params);
        assert!(matches!(r, Err(CmdError::Params(_))), "{params}: {r:?}");
        let mut folder = if params.is_object() { params.clone() } else { json!({}) };
        folder["folder"] = json!("INBOX");
        let r = s.run("folder.retentionPolicy", &folder);
        assert!(matches!(r, Err(CmdError::Params(_))), "{folder}: {r:?}");
    }
    let r = s.run("mail.archivePolicy", &json!({"tag": WEEK}));
    assert!(matches!(r, Err(CmdError::Params(_))), "{r:?}");
    assert!(tag_ops(&s.take_jobs()).is_empty());
    let _ = std::fs::remove_dir_all(store.root());
}

/// The retention commands are off for a server that offers no retention tag, in the online
/// archive, and for an account that isn't Exchange; the archive ones stay on for a server that
/// offers only archive tags.
#[test]
fn the_retention_policy_is_offered_only_by_exchange_with_retention_tags() {
    let off = |s: &Session, id: &str| s.registry().get(id).and_then(|c| (c.enabled)(s));
    let (mut s, store, id, _key) = offered("retention-offered");
    assert_eq!((off(&s, "mail.retentionPolicy"), off(&s, "folder.retentionPolicy")), (None, None));
    // Only archive tags.
    s.apply(Event::ArchiveTags { account: id.clone(), tags: tags().into_iter().filter(|t| t.kind == PolicyKind::Archive).collect() });
    assert!(off(&s, "mail.retentionPolicy").is_some() && off(&s, "folder.retentionPolicy").is_some());
    assert_eq!(off(&s, "mail.archivePolicy"), None);
    let r = s.run("folder.retentionPolicy", &json!({"folder": "INBOX", "tag": null}));
    assert!(matches!(r, Err(CmdError::Disabled(_))), "{r:?}");
    // Only retention tags: the archive commands are off, the retention ones on.
    s.apply(Event::ArchiveTags { account: id.clone(), tags: tags().into_iter().filter(|t| t.kind == PolicyKind::Retention).collect() });
    assert!(off(&s, "mail.archivePolicy").is_some());
    assert_eq!(off(&s, "mail.retentionPolicy"), None);
    assert!(s.offers_policy(&id, "INBOX") && !s.offers_policy(&id, PROJECTS), "not in the archive");
    let _ = std::fs::remove_dir_all(store.root());

    // An account that isn't Exchange: tags told for it change nothing.
    let mut s = sample_session("retention-imap");
    let account = s.accounts[0].config.id.clone();
    s.apply(Event::ArchiveTags { account: account.clone(), tags: tags() });
    let first = s.list.first().map(|m| m.key.clone()).unwrap();
    s.run("list.select", &json!({"key": first.to_id()})).unwrap();
    assert!(!s.offers_policy(&account, "INBOX"));
    assert!(off(&s, "mail.retentionPolicy").is_some());
    let r = s.run("mail.retentionPolicy", &json!({"tag": WEEK}));
    assert!(matches!(r, Err(CmdError::Disabled(_))), "{r:?}");
    assert!(tag_ops(&s.take_jobs()).is_empty());
    done(s);
}
