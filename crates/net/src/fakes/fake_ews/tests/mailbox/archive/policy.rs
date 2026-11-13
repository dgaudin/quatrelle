// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Tests over the fake EWS server: the archive policy tags the server offers, and the tag a
//! message or a folder carries (read when opened or listed, set, taken off, refused, lost).

use std::sync::atomic::Ordering;

use quatrelle_model::{ArchiveTag, MessageKey, PolicyKind};

use super::*;

fn client(server: &crate::fakes::fake_ews::server::Server) -> crate::ews::Client {
    connect(server)
}

/// Riley's mailbox with one message in the Inbox and the server's default policy tags: the data,
/// the Inbox's id and the message's.
pub(super) fn tagged() -> (Data, String, String) {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Projects", None, None, "IPF.Note");
    let id = d.deliver(&inbox, &mail("Harbour plans", "p1"), true, "2026-10-19T08:00:00Z");
    d.offer_tags();
    (d, inbox, id)
}

/// A started account (online, synced) over this data: the server, the pool and the store.
pub(super) fn started(d: Data, name: &str) -> (crate::fakes::fake_ews::server::Server, crate::worker::Pool, Store) {
    let server = serve(d);
    let store = crate::fake::tests_store(name);
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    pool.submit(Job::Start { account: super::reading::account(&server), password: Some(Secret(PASSWORD.into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    eventually("the Inbox", || store.load_folder("a1", "INBOX").is_ok_and(|f| !f.messages.is_empty()));
    (server, pool, store)
}

pub(super) fn inbox_key(store: &Store) -> MessageKey {
    store.load_folder("a1", "INBOX").unwrap().messages[0].key.clone()
}

pub(super) fn set_kind(pool: &mut crate::worker::Pool, key: &MessageKey, kind: PolicyKind, tag: Option<&str>) {
    let op = Op::SetArchiveTag { keys: vec![key.clone()], folder: None, tag: tag.map(str::to_string), kind };
    pool.submit(Job::Apply { account: "a1".into(), op });
}

fn set(pool: &mut crate::worker::Pool, key: &MessageKey, tag: Option<&str>) {
    set_kind(pool, key, PolicyKind::Archive, tag);
}

/// What the worker tells of a message's tags: the archive tag and day, the retention tag and day.
pub(super) fn read_all(pool: &mut crate::worker::Pool, key: &MessageKey) -> [Option<String>; 4] {
    pool.submit(Job::ReadArchiveTag { key: key.clone() });
    let seen = until(pool, "the tag read", |e| matches!(e, Event::ArchiveTagRead { key: k, .. } if k == key));
    match seen.into_iter().last() {
        Some(Event::ArchiveTagRead { tag, date, retention_tag, retention_date, .. }) => [tag, date, retention_tag, retention_date],
        other => panic!("{other:?}"),
    }
}

fn read(pool: &mut crate::worker::Pool, key: &MessageKey) -> (Option<String>, Option<String>) {
    let [tag, date, ..] = read_all(pool, key);
    (tag, date)
}

/// Of the tags the server lists (a default one, two personal ones that move to the archive, the
/// personal "never", two personal delete tags, a hidden one and one that only marks mail), the
/// personal archive and delete ones come, in the server's order, with their names as the server
/// gives them, their days, their kind, and the "never" told apart.
#[test]
fn the_tags_offered_are_the_servers_personal_archive_and_delete_tags() {
    let (d, ..) = tagged();
    let server = serve(d);
    let c = client(&server);
    let tags = c.archive_tags().unwrap();
    let archive = |id: &str, name: &str, days: Option<u32>| ArchiveTag {
        id: id.into(),
        name: name.into(),
        days,
        never: days.is_none(),
        kind: PolicyKind::Archive,
    };
    let retention = |id: &str, name: &str, days: u32| ArchiveTag {
        id: id.into(),
        name: name.into(),
        days: Some(days),
        never: false,
        kind: PolicyKind::Retention,
    };
    assert_eq!(
        tags,
        [
            archive(TAG_YEAR, "1 Year Move to Archive", Some(365)),
            archive(TAG_FIVE, "5 Year Move to Archive", Some(1825)),
            archive(TAG_NEVER, "Never Move to Archive", None),
            retention(TAG_DELETE, "3 Year Delete", 1095),
            retention(TAG_WEEK, "1 Week Delete", 7),
        ],
        "not the default one, the hidden one, nor the one that only marks mail"
    );
    assert!(!tags.iter().any(|t| ["11111111-aaaa-4bbb-8ccc-000000000001", TAG_HIDDEN, TAG_MARK].contains(&t.id.as_str())));
    // Asked once per session.
    c.archive_tags().unwrap();
    c.archive_tags().unwrap();
    let asked = server.data.lock().unwrap().calls.iter().filter(|c| *c == "GetUserRetentionPolicyTags").count();
    assert_eq!(asked, 1, "the list is kept for the session");
}

/// A server that offers no tag, one that doesn't know the operation (Exchange 2010) and one that
/// refuses give none, and its folders are listed without asking for a tag the server may not know.
#[test]
fn a_server_without_tags_offers_none() {
    let (mut d, ..) = tagged();
    d.policy.offered = Some(Vec::new());
    assert!(client(&serve(d)).archive_tags().unwrap().is_empty());

    let (mut d, ..) = tagged();
    d.policy.offered = None;
    let server = serve(d);
    let c = client(&server);
    assert!(c.archive_tags().unwrap().is_empty());
    let folders = crate::ews::list_folders(&c, &mut crate::jmap::State::default(), "a1").unwrap();
    assert!(folders.iter().all(|f| f.archive_tag.is_none()));
    assert_eq!(server.data.lock().unwrap().policy.list_reads.load(Ordering::Relaxed), 0, "no tag asked of a server that has none");

    let (mut d, ..) = tagged();
    d.policy.raw = Some("<m:ResponseCode>ErrorAccessDenied</m:ResponseCode><m:MessageText>no</m:MessageText>".into());
    assert!(client(&serve(d)).archive_tags().unwrap().is_empty());
}

/// A hostile server: ids that aren't GUIDs, names of five thousand characters with control
/// characters, periods that are not numbers or far too long, the same tag over and over, a
/// thousand tags. What comes is bounded and checked.
#[test]
fn hostile_tags_are_dropped() {
    let (mut d, ..) = tagged();
    let tag = |id: &str, name: &str, period: &str| {
        format!(
            "<t:RetentionPolicyTag><t:DisplayName>{name}</t:DisplayName><t:RetentionId>{id}</t:RetentionId>{period}<t:Type>Personal</t:Type>\
             <t:RetentionAction>MoveToArchive</t:RetentionAction><t:IsVisible>true</t:IsVisible></t:RetentionPolicyTag>"
        )
    };
    let mut list = String::new();
    list += &tag("not-a-guid", "Bad id", "<t:RetentionPeriod>30</t:RetentionPeriod>");
    list += &tag(&format!("{TAG_YEAR}-extra"), "Long id", "");
    list += &tag(TAG_YEAR, &format!("Bell\u{7}\u{0} {}", "x".repeat(5_000)), "<t:RetentionPeriod>365</t:RetentionPeriod>");
    list += &tag(TAG_YEAR, "Same id twice", "<t:RetentionPeriod>365</t:RetentionPeriod>");
    list += &tag(TAG_FIVE, "Not a number", "<t:RetentionPeriod>soon</t:RetentionPeriod>");
    list += &tag(TAG_NEVER, "Far too long", "<t:RetentionPeriod>4294967295</t:RetentionPeriod>");
    list += &tag(TAG_DELETE, "", "<t:RetentionPeriod>30</t:RetentionPeriod>");
    for i in 0..1_000 {
        list += &tag(&format!("00000000-0000-4000-8000-{i:012x}"), &format!("Tag {i}"), "<t:RetentionPeriod>90</t:RetentionPeriod>");
    }
    d.policy.raw = Some(format!("<m:ResponseCode>NoError</m:ResponseCode><m:RetentionPolicyTags>{list}</m:RetentionPolicyTags>"));
    let tags = client(&serve(d)).archive_tags().unwrap();
    assert_eq!(tags.len(), 100, "the list is capped");
    assert_eq!(tags[0].id, TAG_YEAR);
    assert_eq!(tags[0].name.chars().count(), 120);
    assert!(!tags[0].name.chars().any(char::is_control), "{:?}", tags[0].name);
    for t in &tags {
        assert!(quatrelle_model::is_tag_id(&t.id) && !t.name.is_empty() && t.name.chars().count() <= 120, "{t:?}");
    }
    assert_eq!(tags.iter().filter(|t| t.id == TAG_YEAR).count(), 1, "each tag once");
    assert!(!tags.iter().any(|t| t.name == "Bad id" || t.name == "Not a number" || t.name == "Far too long"));
}

/// A message's own tag is read when it is opened, with the day the server plans to move it, set
/// by a change (Exchange takes it as an explicit tag), and taken off. The lists' sync never
/// asks for it.
#[test]
fn a_messages_tag_is_read_set_and_taken_off() {
    let (mut d, _, id) = tagged();
    // Outlook gave it a tag before: read as the server has it.
    d.tag_item(&id, TAG_FIVE, Some("2026-12-24T00:00:00Z"));
    let (server, mut pool, store) = started(d, "ews-policy-message");
    let key = inbox_key(&store);
    let data = server.data.clone();
    assert_eq!(data.lock().unwrap().policy.item_reads.load(Ordering::Relaxed), 0, "the sync asked for no tag");
    assert_eq!(read(&mut pool, &key), (Some(TAG_FIVE.into()), Some("2026-12-24".into())), "read as the server has it");

    set(&mut pool, &key, Some(TAG_YEAR));
    eventually("the tag on the server", || data.lock().unwrap().policy.items.values().any(|(t, _)| t == TAG_YEAR));
    assert_eq!(read(&mut pool, &key), (Some(TAG_YEAR.into()), Some(ARCHIVE_DAY[..10].into())));
    // The journal is empty once it is sent.
    eventually("the change sent", || store.load_ops("a1").unwrap().is_empty());

    set(&mut pool, &key, Some(TAG_NEVER));
    eventually("the other tag", || data.lock().unwrap().policy.items.values().any(|(t, _)| t == TAG_NEVER));
    assert_eq!(read(&mut pool, &key), (Some(TAG_NEVER.into()), None), "the one that keeps mail has no day");

    set(&mut pool, &key, None);
    eventually("the tag taken off", || data.lock().unwrap().policy.items.is_empty());
    assert_eq!(read(&mut pool, &key), (None, None));
    let sent = data.lock().unwrap().policy.writes;
    assert_eq!(sent, 3, "one write for each change");
}

/// A folder's tag comes with the folder list, is set by a change and taken off.
#[test]
fn a_folders_tag_is_read_set_and_taken_off() {
    let (mut d, ..) = tagged();
    let projects = d.by_name("Projects").map(|f| f.id.clone()).unwrap();
    d.policy.folders.insert(projects.clone(), TAG_FIVE.into());
    let (server, mut pool, store) = started(d, "ews-policy-folder");
    let data = server.data.clone();
    let tag_of = |store: &Store| store.load_folders("a1").unwrap().into_iter().find(|f| f.path == "Projects").and_then(|f| f.archive_tag);
    assert_eq!(tag_of(&store), Some(TAG_FIVE.into()), "listed with the folder");
    assert_eq!(tag_of_inbox(&store), None);

    let op = |tag: Option<&str>| Op::SetArchiveTag {
        keys: Vec::new(),
        folder: Some("Projects".into()),
        tag: tag.map(str::to_string),
        kind: PolicyKind::Archive,
    };
    pool.submit(Job::Apply { account: "a1".into(), op: op(Some(TAG_YEAR)) });
    eventually("the folder's tag set", || data.lock().unwrap().policy.folders.get(&projects).is_some_and(|t| t == TAG_YEAR));
    pool.submit(Job::Apply { account: "a1".into(), op: op(None) });
    eventually("the folder's tag off", || data.lock().unwrap().policy.folders.is_empty());
    // Listed again, it carries none.
    pool.submit(Job::SyncAll { account: "a1".into() });
    until(&pool, "the folders listed again", |e| matches!(e, Event::Folders { folders, .. } if folders.iter().all(|f| f.archive_tag.is_none())));
}

fn tag_of_inbox(store: &Store) -> Option<String> {
    store.load_folders("a1").unwrap().into_iter().find(|f| f.path == "INBOX").and_then(|f| f.archive_tag)
}

/// The tags are told to the engine with the first folder listing.
#[test]
fn the_tags_are_told_with_the_folder_list() {
    let (d, ..) = tagged();
    let server = serve(d);
    let store = crate::fake::tests_store("ews-policy-told");
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    pool.submit(Job::Start { account: super::reading::account(&server), password: Some(Secret(PASSWORD.into())) });
    let seen = until(&pool, "the tags", |e| matches!(e, Event::ArchiveTags { .. }));
    let Some(Event::ArchiveTags { tags, .. }) = seen.last() else { panic!("{seen:?}") };
    assert_eq!(tags.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), [TAG_YEAR, TAG_FIVE, TAG_NEVER, TAG_DELETE, TAG_WEEK]);
}

/// A tag set while the message was moved to the archive by the server (it has another id there,
/// the one the change names is gone): the change is dropped with a failure told, once, and it
/// is not in the journal to be tried again.
#[test]
fn a_tag_on_mail_gone_to_the_archive_is_dropped() {
    let (mut d, ..) = tagged();
    let root = d.archive_mailbox("Online Archive - Riley");
    let old = d.archive_folder("Inbox", &root);
    let (server, mut pool, store) = started(d, "ews-policy-moved");
    let key = inbox_key(&store);
    let data = server.data.clone();
    let id = data.lock().unwrap().items[0].id.clone();
    // The server moves it before the change reaches it.
    assert!(data.lock().unwrap().move_to_archive(&id, &old).is_some());
    set(&mut pool, &key, Some(TAG_YEAR));
    let seen = until(&pool, "the failure", |e| matches!(e, Event::OpFailed { .. }));
    assert!(matches!(seen.last(), Some(Event::OpFailed { op: Op::SetArchiveTag { .. }, .. })), "{seen:?}");
    assert!(store.load_ops("a1").unwrap().is_empty(), "not kept to be tried again");
    let writes = data.lock().unwrap().policy.writes;
    assert!(data.lock().unwrap().policy.items.is_empty(), "nothing was tagged");
    // Not tried again at the next pass.
    pool.submit(Job::SyncAll { account: "a1".into() });
    until(&pool, "a pass", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    assert_eq!(data.lock().unwrap().policy.writes, writes);
}

/// A tag the server refuses (`ErrorAccessDenied`) is a failure told; the change is not kept.
#[test]
fn a_tag_the_server_refuses_is_a_failure() {
    let (mut d, ..) = tagged();
    d.policy.refuse_writes = true;
    let (_server, mut pool, store) = started(d, "ews-policy-refused");
    let key = inbox_key(&store);
    set(&mut pool, &key, Some(TAG_YEAR));
    until(&pool, "the failure", |e| matches!(e, Event::OpFailed { op: Op::SetArchiveTag { .. }, .. }));
    assert!(store.load_ops("a1").unwrap().is_empty());
}

/// Exchange makes its "never" tags by disabling them: a personal tag whose action is `None` is
/// an archive tag when `IsArchive` says so (here with no period), a retention tag when it says
/// not (here a period of 0), both "never", after the others in the server's order; the disabled
/// archive tag is set on a message as an archive tag, and has no day.
#[test]
fn disabled_personal_tags_are_the_never_ones() {
    let (mut d, ..) = tagged();
    d.offer_disabled_tags();
    let (server, mut pool, store) = started(d, "ews-policy-disabled");
    let tags = client(&server).archive_tags().unwrap();
    let never = |id: &str, name: &str, kind| ArchiveTag { id: id.into(), name: name.into(), days: None, never: true, kind };
    assert_eq!(tags.len(), 7, "{tags:?}");
    assert_eq!(
        tags[5..],
        [never(TAG_OFF_ARCHIVE, "Never Archive", PolicyKind::Archive), never(TAG_OFF_DELETE, "Never Delete", PolicyKind::Retention)]
    );
    let key = inbox_key(&store);
    let data = server.data.clone();
    set(&mut pool, &key, Some(TAG_OFF_ARCHIVE));
    eventually("the tag on the server", || data.lock().unwrap().policy.items.values().any(|(t, _)| t == TAG_OFF_ARCHIVE));
    assert_eq!(read(&mut pool, &key), (Some(TAG_OFF_ARCHIVE.into()), None), "a tag that never acts plans no day");
}

/// One personal tag as a server may write it: its name, action, period and `IsArchive` (none:
/// the element left out).
fn written(n: u32, name: &str, action: &str, period: Option<&str>, archive: Option<&str>) -> String {
    format!(
        "<t:RetentionPolicyTag><t:DisplayName>{name}</t:DisplayName><t:RetentionId>00000000-0000-4000-8000-{n:012x}</t:RetentionId>{}\
         <t:Type>Personal</t:Type><t:RetentionAction>{action}</t:RetentionAction><t:IsVisible>true</t:IsVisible>{}</t:RetentionPolicyTag>",
        period.map(|p| format!("<t:RetentionPeriod>{p}</t:RetentionPeriod>")).unwrap_or_default(),
        archive.map(|a| format!("<t:IsArchive>{a}</t:IsArchive>")).unwrap_or_default()
    )
}

/// Microsoft's documents don't show how a disabled tag is answered: each plausible form is read.
/// `IsArchive` true makes an archive tag whether the action is `MoveToArchive` or `None`; a
/// disabled one (`None`) with a period of 0, none or one that isn't a number is "never", an
/// archive or a retention tag as `IsArchive` says. On a tag that acts, a period that isn't a
/// number, or too long, still leaves it out; so does a disabled one whose kind isn't said, and
/// one that only marks mail.
#[test]
fn every_plausible_form_of_a_never_tag_is_read() {
    let (mut d, ..) = tagged();
    let list = [
        written(1, "Archive, said twice", "MoveToArchive", Some("365"), Some("true")),
        written(2, "Never archive, 0", "None", Some("0"), Some("true")),
        written(3, "Never archive, none", "None", None, Some("true")),
        written(4, "Never archive, not a number", "None", Some("unlimited"), Some("true")),
        written(5, "Never delete, 0", "None", Some("0"), Some("false")),
        written(6, "Never delete, none", "None", None, Some("false")),
        written(7, "Never delete, not a number", "None", Some("unlimited"), Some("false")),
        written(8, "Acts, not a number", "MoveToArchive", Some("unlimited"), Some("true")),
        written(9, "Acts, far too long", "DeleteAndAllowRecovery", Some("4294967295"), Some("false")),
        written(10, "Disabled, kind unsaid", "None", Some("0"), None),
        written(11, "Marks only", "MarkAsPastRetentionLimit", Some("365"), Some("true")),
    ]
    .concat();
    d.policy.raw = Some(format!("<m:ResponseCode>NoError</m:ResponseCode><m:RetentionPolicyTags>{list}</m:RetentionPolicyTags>"));
    let tags = client(&serve(d)).archive_tags().unwrap();
    let seen: Vec<(&str, Option<u32>, bool, PolicyKind)> = tags.iter().map(|t| (t.name.as_str(), t.days, t.never, t.kind)).collect();
    use PolicyKind::{Archive, Retention};
    assert_eq!(
        seen,
        [
            ("Archive, said twice", Some(365), false, Archive),
            ("Never archive, 0", None, true, Archive),
            ("Never archive, none", None, true, Archive),
            ("Never archive, not a number", None, true, Archive),
            ("Never delete, 0", None, true, Retention),
            ("Never delete, none", None, true, Retention),
            ("Never delete, not a number", None, true, Retention),
        ]
    );
}

/// A personal tag left out is told in the log with why, as the server says it (its name, action,
/// period, `IsArchive`, `IsVisible`: cleaned and bounded, nothing else of it), so the log says
/// why a tag doesn't show; the mailbox's default tags aren't told; past twenty, they're counted.
#[test]
fn the_personal_tags_left_out_are_told_with_why() {
    let hidden = "<t:RetentionPolicyTag><t:DisplayName>1 Month\u{7}\n Delete</t:DisplayName><t:RetentionId>77777777-aaaa-4bbb-8ccc-000000000007</t:RetentionId>\
                  <t:RetentionPeriod>30</t:RetentionPeriod><t:Type>Personal</t:Type><t:RetentionAction>DeleteAndAllowRecovery</t:RetentionAction>\
                  <t:IsVisible>false</t:IsVisible><t:IsArchive>false</t:IsArchive></t:RetentionPolicyTag>";
    let default = "<t:RetentionPolicyTag><t:DisplayName>Default 2 years</t:DisplayName><t:RetentionId>11111111-aaaa-4bbb-8ccc-000000000001</t:RetentionId>\
                   <t:Type>DefaultFolder</t:Type><t:RetentionAction>Bogus</t:RetentionAction></t:RetentionPolicyTag>";
    let odd = written(1, "Odd", &format!("Never\nMind{}", "x".repeat(5_000)), Some("soon"), None);
    let marks: String = (2..32).map(|n| written(n, &format!("Mark {n}"), "MarkAsPastRetentionLimit", Some("365"), Some("false"))).collect();
    let xml = format!("<m:RetentionPolicyTags>{hidden}{default}{odd}{marks}</m:RetentionPolicyTags>");
    let (tags, left, more) = crate::ews::tags_read(&crate::ews::parse(&xml));
    assert!(tags.is_empty(), "{tags:?}");
    let first = &left[0];
    assert_eq!(
        (first.name.as_str(), first.reason, first.action.as_str(), first.period.as_str(), first.is_archive.as_str(), first.visible.as_str()),
        ("1 Month Delete", "hidden from the user", "DeleteAndAllowRecovery", "30", "false", "false")
    );
    let line = first.to_string();
    assert!(line.contains("\"1 Month Delete\"") && line.contains("hidden from the user") && !line.contains("77777777"), "{line}");
    assert!(!left.iter().any(|l| l.name.starts_with("Default")), "the mailbox's default tags aren't the user's");
    let odd = &left[1];
    assert_eq!((odd.name.as_str(), odd.reason, odd.is_archive.as_str()), ("Odd", "its action neither archives nor deletes", "(none)"));
    assert!(
        odd.action.starts_with("Never Mind") && odd.action.chars().count() <= 40 && !odd.action.chars().any(char::is_control),
        "{:?}",
        odd.action
    );
    assert_eq!(left[2].reason, "it only marks mail past its limit");
    assert_eq!((left.len(), more), (20, 12), "twenty told, the rest counted");
}
