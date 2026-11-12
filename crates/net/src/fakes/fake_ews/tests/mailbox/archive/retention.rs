// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Tests over the fake EWS server: the retention policy (the personal tags that delete mail)
//! beside the archive policy: the tag a message or a folder carries (`item:PolicyTag`,
//! `folder:PolicyTag`), read, set and taken off, and the day the server plans to delete it.

use std::sync::atomic::Ordering;

use quatrelle_model::PolicyKind;

use super::policy::{inbox_key, read_all, set_kind, started, tagged};
use super::*;

/// A message's own retention tag is read when it is opened, with the day the server plans to
/// delete it (`item:RetentionDate`), apart from its archive tag and the day the server plans to
/// move it (`PidTagArchiveDate`); set by a change of its own kind, and taken off without
/// touching the archive tag.
#[test]
fn a_messages_retention_tag_is_read_set_and_taken_off() {
    let (mut d, _, id) = tagged();
    d.tag_item(&id, TAG_FIVE, Some("2031-10-19T00:00:00Z"));
    d.retain_item(&id, TAG_DELETE, Some("2029-10-19T00:00:00Z"));
    let (server, mut pool, store) = started(d, "ews-retention-message");
    let key = inbox_key(&store);
    let data = server.data.clone();
    assert_eq!(data.lock().unwrap().policy.item_reads.load(Ordering::Relaxed), 0, "the sync asked for no tag");
    let some = |s: &str| Some(s.to_string());
    assert_eq!(read_all(&mut pool, &key), [some(TAG_FIVE), some("2031-10-19"), some(TAG_DELETE), some("2029-10-19")], "each with its own day");

    set_kind(&mut pool, &key, PolicyKind::Retention, Some(TAG_WEEK));
    eventually("the retention tag on the server", || data.lock().unwrap().policy.retained.values().any(|(t, _)| t == TAG_WEEK));
    assert_eq!(read_all(&mut pool, &key), [some(TAG_FIVE), some("2031-10-19"), some(TAG_WEEK), some(&DELETE_DAY[..10])]);
    eventually("the change sent", || store.load_ops("a1").unwrap().is_empty());

    set_kind(&mut pool, &key, PolicyKind::Retention, None);
    eventually("the retention tag taken off", || data.lock().unwrap().policy.retained.is_empty());
    assert_eq!(read_all(&mut pool, &key), [some(TAG_FIVE), some("2031-10-19"), None, None], "the archive tag stays");
    assert_eq!(data.lock().unwrap().policy.writes, 2, "one write for each change");
}

/// A folder's retention tag comes with the folder list beside its archive tag, is set by a
/// change and taken off.
#[test]
fn a_folders_retention_tag_is_read_set_and_taken_off() {
    let (mut d, ..) = tagged();
    let projects = d.by_name("Projects").map(|f| f.id.clone()).unwrap();
    d.policy.folders.insert(projects.clone(), TAG_FIVE.into());
    d.policy.retained_folders.insert(projects.clone(), TAG_DELETE.into());
    let (server, mut pool, store) = started(d, "ews-retention-folder");
    let data = server.data.clone();
    let projects_of = |store: &Store| store.load_folders("a1").unwrap().into_iter().find(|f| f.path == "Projects").unwrap();
    let listed = projects_of(&store);
    assert_eq!((listed.archive_tag.as_deref(), listed.retention_tag.as_deref()), (Some(TAG_FIVE), Some(TAG_DELETE)), "listed with the folder");

    let op = |tag: Option<&str>| Op::SetArchiveTag {
        keys: Vec::new(),
        folder: Some("Projects".into()),
        tag: tag.map(str::to_string),
        kind: PolicyKind::Retention,
    };
    pool.submit(Job::Apply { account: "a1".into(), op: op(Some(TAG_WEEK)) });
    eventually("the folder's retention tag set", || data.lock().unwrap().policy.retained_folders.get(&projects).is_some_and(|t| t == TAG_WEEK));
    pool.submit(Job::Apply { account: "a1".into(), op: op(None) });
    eventually("the folder's retention tag off", || data.lock().unwrap().policy.retained_folders.is_empty());
    assert_eq!(data.lock().unwrap().policy.folders.get(&projects).map(String::as_str), Some(TAG_FIVE), "the archive tag stays");
    pool.submit(Job::SyncAll { account: "a1".into() });
    until(
        &pool,
        "the folders listed again",
        |e| matches!(e, Event::Folders { folders, .. } if folders.iter().all(|f| f.retention_tag.is_none()) && folders.iter().any(|f| f.archive_tag.is_some())),
    );
}

/// A retention tag a hostile server names over several lines, with bidi controls, is shown on
/// one line without them; one it lists twice, or once as an archive tag and once as a delete
/// tag, comes once.
#[test]
fn hostile_retention_tags_are_cleaned() {
    let (mut d, ..) = tagged();
    let tag = |id: &str, name: &str, action: &str| {
        format!(
            "<t:RetentionPolicyTag><t:DisplayName>{name}</t:DisplayName><t:RetentionId>{id}</t:RetentionId>\
             <t:RetentionPeriod>7</t:RetentionPeriod><t:Type>Personal</t:Type><t:RetentionAction>{action}</t:RetentionAction>\
             <t:IsVisible>true</t:IsVisible></t:RetentionPolicyTag>"
        )
    };
    let mut list = String::new();
    list += &tag(TAG_WEEK, "1 Week\n\n  Delete\u{202E}gnp.exe", "PermanentlyDelete");
    list += &tag(TAG_WEEK, "Again", "DeleteAndAllowRecovery");
    list += &tag(TAG_YEAR, "1 Year Move to Archive", "MoveToArchive");
    list += &tag(TAG_YEAR, "Same id, deletes", "DeleteAndAllowRecovery");
    list += &tag(TAG_MARK, "Marks", "MarkAsPastRetentionLimit");
    d.policy.raw = Some(format!("<m:ResponseCode>NoError</m:ResponseCode><m:RetentionPolicyTags>{list}</m:RetentionPolicyTags>"));
    let server = serve(d);
    let tags = connect(&server).archive_tags().unwrap();
    let seen: Vec<(&str, &str, PolicyKind)> = tags.iter().map(|t| (t.id.as_str(), t.name.as_str(), t.kind)).collect();
    assert_eq!(
        seen,
        [(TAG_WEEK, "1 Week Deletegnp.exe", PolicyKind::Retention), (TAG_YEAR, "1 Year Move to Archive", PolicyKind::Archive)],
        "{tags:?}"
    );
}

/// A retention tag change that names an archive tag is refused by the server; the change is a
/// failure told, and not kept.
#[test]
fn an_archive_tag_set_as_a_retention_tag_is_a_failure() {
    let (d, ..) = tagged();
    let (server, mut pool, store) = started(d, "ews-retention-wrong-kind");
    let key = inbox_key(&store);
    set_kind(&mut pool, &key, PolicyKind::Retention, Some(TAG_YEAR));
    until(&pool, "the failure", |e| matches!(e, Event::OpFailed { op: Op::SetArchiveTag { kind: PolicyKind::Retention, .. }, .. }));
    assert!(store.load_ops("a1").unwrap().is_empty());
    assert!(server.data.lock().unwrap().policy.retained.is_empty());
}
