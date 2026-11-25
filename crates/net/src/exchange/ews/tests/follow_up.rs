// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Tests of the EWS transport: a follow-up marked complete, as Outlook keeps it on Exchange
//! (`PidTagFlagStatus` 1, and `PidTagFlagCompleteTime` the time it was done), both ways.

use quatrelle_model::FOLLOW_UP_COMPLETE;

use super::*;
use crate::fakes::fake_ews::{self, Data};

/// Marked complete in Outlook, a message comes in complete, on the day Exchange says; marked
/// complete here, Exchange gets status 1 and the time; flagged again, status 2 without the
/// time; cleared, neither. Each change made elsewhere comes in at the next sync.
#[test]
fn a_follow_up_marked_complete_goes_both_ways_with_its_day() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let done = d.deliver(&inbox, b"From: a@x.example\r\nSubject: done\r\n\r\nx\r\n", false, "2024-05-01T09:00:00Z");
    let flagged = d.deliver(&inbox, b"From: a@x.example\r\nSubject: flagged\r\n\r\nx\r\n", false, "2024-05-02T09:00:00Z");
    for i in d.items.iter_mut() {
        if i.id == done {
            i.complete = true;
            i.complete_time = Some("2026-10-30T09:12:00Z".into());
        }
        i.flagged = i.id == flagged;
    }
    let server = fake_ews::serve(d);
    let dir = std::env::temp_dir().join(format!("quatrelle-ews-follow-up-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let user = format!("{}\\{}", fake_ews::DOMAIN, fake_ews::USER);
    let c = Client::connect(&server.url, &user, fake_ews::PASSWORD, "riley@contoso.example").unwrap();
    let mut st = State::default();
    list_folders(&c, &mut st, "a1").unwrap();
    let opts = SyncOptions { prefetch: 0, ..Default::default() };
    let msg = |subject: &str| store.load_folder("a1", "INBOX").unwrap().messages.iter().find(|m| m.subject == subject).cloned().unwrap();
    let on_server = |id: &str| {
        let d = server.data.lock().unwrap();
        d.items.iter().find(|i| i.id == id).map(|i| (i.flagged, i.complete, i.complete_time.clone())).unwrap()
    };
    let at = |text: &str| jmap::utc_date(text).unwrap();

    // Complete in Outlook: complete here, on its day.
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    let m = msg("done");
    assert!(m.flags.follow_up_complete() && m.flags.completed == at("2026-10-30T09:12:00Z"), "{:?}", m.flags);
    assert!(msg("flagged").flags.flagged && !msg("flagged").flags.has(FOLLOW_UP_COMPLETE));

    // Mark Complete here (the flag off, then the keyword on): status 1 and the time.
    let keys = vec![msg("flagged").key];
    let flag = |on| Op::SetFlag { keys: keys.clone(), flag: Flag::Flagged, on, all: false };
    let complete = |on| Op::SetKeyword { keys: keys.clone(), keyword: FOLLOW_UP_COMPLETE.into(), on, name: None };
    for op in [flag(false), complete(true)] {
        apply_remote(&c, &st, &op).unwrap();
    }
    let (f, cmp, time) = on_server(&flagged);
    assert!(!f && cmp && time.as_deref().and_then(jmap::utc_date).is_some_and(|t| t > at("2026-01-01T00:00:00Z")), "{time:?}");
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    assert!(msg("flagged").flags.follow_up_complete() && msg("flagged").flags.completed > 0, "{:?}", msg("flagged").flags);

    // Its Undo (the keyword off, then the flag on): flagged, the time gone.
    for op in [complete(false), flag(true)] {
        apply_remote(&c, &st, &op).unwrap();
    }
    assert_eq!(on_server(&flagged), (true, false, None));
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    let m = msg("flagged");
    assert!(m.flags.flagged && !m.flags.has(FOLLOW_UP_COMPLETE) && m.flags.completed == 0, "{:?}", m.flags);

    // Cleared: neither.
    apply_remote(&c, &st, &flag(false)).unwrap();
    assert_eq!(on_server(&flagged), (false, false, None));

    // In Outlook: the one done flagged again, the other marked complete; the next sync (only
    // what changed) says so.
    {
        let mut d = server.data.lock().unwrap();
        for i in d.items.iter_mut() {
            if i.id == done {
                (i.flagged, i.complete, i.complete_time) = (true, false, None);
            } else if i.id == flagged {
                (i.complete, i.complete_time) = (true, Some("2026-11-02T16:30:00Z".into()));
            }
        }
    }
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    let m = msg("done");
    assert!(m.flags.flagged && !m.flags.has(FOLLOW_UP_COMPLETE) && m.flags.completed == 0, "{:?}", m.flags);
    let m = msg("flagged");
    assert!(m.flags.follow_up_complete() && m.flags.completed == at("2026-11-02T16:30:00Z"), "{:?}", m.flags);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A follow-up's start and due dates and its reminder, as Outlook keeps them ([MS-OXOFLAG]
/// 2.2.1): read from Exchange, written to it, and cleared with the flag. Flagging sets the plain
/// red flag's icon and title ([MS-OXOFLAG] 3.1.4.1).
#[test]
fn follow_up_dates_and_its_reminder_go_both_ways() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let id = d.deliver(&inbox, b"From: a@x.example\r\nSubject: due\r\n\r\nx\r\n", true, "2024-05-01T09:00:00Z");
    for i in d.items.iter_mut() {
        i.flagged = true;
        i.follow_up = fake_ews::FollowUp {
            start: Some("2026-11-02T00:00:00Z".into()),
            due: Some("2026-11-06T00:00:00Z".into()),
            common_start: Some("2026-11-02T00:00:00Z".into()),
            common_end: Some("2026-11-06T00:00:00Z".into()),
            reminder_set: true,
            reminder_time: Some("2026-11-06T09:00:00Z".into()),
            reminder_signal: Some("2026-11-06T09:00:00Z".into()),
            request: Some("Follow up".into()),
            title: None,
        };
    }
    let server = fake_ews::serve(d);
    let dir = std::env::temp_dir().join(format!("quatrelle-ews-follow-up-dates-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let user = format!("{}\\{}", fake_ews::DOMAIN, fake_ews::USER);
    let c = Client::connect(&server.url, &user, fake_ews::PASSWORD, "riley@contoso.example").unwrap();
    let mut st = State::default();
    list_folders(&c, &mut st, "a1").unwrap();
    let opts = SyncOptions { prefetch: 0, ..Default::default() };
    let at = |text: &str| jmap::utc_date(text).unwrap();

    // As Outlook set it: start, due, reminder and title.
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    let m = store.load_folder("a1", "INBOX").unwrap().messages[0].clone();
    assert!(m.flags.flagged, "{:?}", m.flags);
    assert_eq!(m.flags.follow_up.start, at("2026-11-02T00:00:00Z"), "{:?}", m.flags.follow_up);
    assert_eq!(m.flags.follow_up.due, at("2026-11-06T00:00:00Z"), "{:?}", m.flags.follow_up);
    assert_eq!(m.flags.follow_up.reminder, at("2026-11-06T09:00:00Z"), "{:?}", m.flags.follow_up);
    assert_eq!(m.flags.follow_up.title, "Follow up");

    // Set here: Exchange gets the dates and the reminder (both its times).
    let keys = vec![m.key];
    let start = at("2026-12-01T00:00:00Z");
    let due = at("2026-12-04T00:00:00Z");
    let reminder = at("2026-12-04T08:30:00Z");
    apply_remote(
        &c,
        &st,
        &Op::SetFollowUp {
            keys: keys.clone(),
            start: Some(FieldChange::Set(start)),
            due: Some(FieldChange::Set(due)),
            reminder: Some(FieldChange::Set(reminder)),
        },
    )
    .unwrap();
    {
        let d = server.data.lock().unwrap();
        let f = &d.items.iter().find(|i| i.id == id).unwrap().follow_up;
        // The Task date as it was given (the day's floating midnight, written as it is), and the
        // Common one the same midnight in the local zone: both are set ([MS-OXOFLAG] 3.1.4.1.3).
        let zone = crate::system_zone();
        assert_eq!(f.start.as_deref().and_then(jmap::utc_date), Some(start), "{f:?}");
        assert_eq!(f.due.as_deref().and_then(jmap::utc_date), Some(due), "{f:?}");
        assert_eq!(f.common_start.as_deref().and_then(jmap::utc_date), Some(zone.to_utc(start)), "{f:?}");
        assert_eq!(f.common_end.as_deref().and_then(jmap::utc_date), Some(zone.to_utc(due)), "{f:?}");
        assert!(f.reminder_set, "{f:?}");
        assert_eq!(f.reminder_time.as_deref().and_then(jmap::utc_date), Some(reminder), "{f:?}");
        assert_eq!(f.reminder_signal.as_deref().and_then(jmap::utc_date), Some(reminder), "{f:?}");
    }
    // The next sync reads them back.
    server.data.lock().unwrap().items.iter_mut().for_each(|i| i.follow_up.request = Some("Send the report".into()));
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    let m = store.load_folder("a1", "INBOX").unwrap().messages[0].clone();
    assert_eq!((m.flags.follow_up.start, m.flags.follow_up.due, m.flags.follow_up.reminder), (start, due, reminder), "{:?}", m.flags.follow_up);
    assert_eq!(m.flags.follow_up.title, "Send the report");

    // Taken off here: Exchange has no reminder and no dates, Task or Common.
    apply_remote(
        &c,
        &st,
        &Op::SetFollowUp { keys: keys.clone(), start: Some(FieldChange::Clear), due: Some(FieldChange::Clear), reminder: Some(FieldChange::Clear) },
    )
    .unwrap();
    {
        let d = server.data.lock().unwrap();
        let f = &d.items.iter().find(|i| i.id == id).unwrap().follow_up;
        assert!(
            !f.reminder_set
                && f.start.is_none()
                && f.due.is_none()
                && f.common_start.is_none()
                && f.common_end.is_none()
                && f.reminder_time.is_none(),
            "{f:?}"
        );
    }

    // Flagged here: the plain red flag's icon and title ([MS-OXOFLAG] 3.1.4.1).
    apply_remote(&c, &st, &Op::SetFlag { keys: keys.clone(), flag: Flag::Flagged, on: false, all: false }).unwrap();
    apply_remote(&c, &st, &Op::SetFlag { keys, flag: Flag::Flagged, on: true, all: false }).unwrap();
    let d = server.data.lock().unwrap();
    let i = d.items.iter().find(|i| i.id == id).unwrap();
    assert!(i.flagged && i.follow_up.request.as_deref() == Some("Follow up"), "{i:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A follow-up date Exchange sends that isn't one, or is MAPI's "none", is ignored rather than
/// kept as a day in 1601.
#[test]
fn a_follow_up_date_that_isnt_one_is_ignored() {
    let item = |props: &str| {
        let doc = parse(&format!("<t:Message><t:IsRead>true</t:IsRead>{props}</t:Message>"));
        doc.find("Message").map(flags).unwrap_or_default()
    };
    let lid = |set: &str, id: u32, kind: &str, v: &str| {
        format!(
            "<t:ExtendedProperty><t:ExtendedFieldURI DistinguishedPropertySetId=\"{set}\" PropertyId=\"{id}\" PropertyType=\"{kind}\"/><t:Value>{v}</t:Value></t:ExtendedProperty>"
        )
    };
    let f = item(&format!(
        "{}{}{}{}",
        lid("Task", 33028, "SystemTime", "not a time"),
        lid("Task", 33029, "SystemTime", "1601-01-01T00:00:00Z"),
        lid("Common", 34051, "Boolean", "true"),
        lid("Common", 34144, "SystemTime", "4501-01-01T00:00:00Z")
    ));
    assert_eq!(f.follow_up, Default::default(), "{f:?}");
}

/// What the server says of a follow-up is read with care: a status other than 1 or 2, a time
/// that isn't one, or a time without the status.
#[test]
fn a_follow_up_read_from_exchange_is_read_with_care() {
    let item = |props: &str| {
        let doc = parse(&format!("<t:Message><t:IsRead>true</t:IsRead>{props}</t:Message>"));
        doc.find("Message").map(flags).unwrap_or_default()
    };
    let prop = |tag: &str, kind: &str, v: &str| {
        format!("<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"{tag}\" PropertyType=\"{kind}\"/><t:Value>{v}</t:Value></t:ExtendedProperty>")
    };
    let status = |v: &str| prop("0x1090", "Integer", v);
    let time = |v: &str| prop("0x1091", "SystemTime", v);
    let f = item(&format!("{}{}", status("1"), time("2026-10-30T09:12:00Z")));
    assert!(f.follow_up_complete() && f.completed > 0, "{f:?}");
    let f = item(&format!("{}{}", status("1"), time("not a time")));
    assert!(f.follow_up_complete() && f.completed == 0, "{f:?}");
    let f = item(&format!("{}{}", status("1"), time("1601-01-01T00:00:00Z")));
    assert!(f.follow_up_complete() && f.completed == 0, "a time before 1970 isn't kept: {f:?}");
    let f = item(&format!("{}{}", status("2"), time("2026-10-30T09:12:00Z")));
    assert!(f.flagged && !f.has(FOLLOW_UP_COMPLETE) && f.completed == 0, "{f:?}");
    let f = item(&time("2026-10-30T09:12:00Z"));
    assert!(!f.flagged && !f.has(FOLLOW_UP_COMPLETE) && f.completed == 0, "{f:?}");
    let f = item(&status("7"));
    assert!(!f.flagged && !f.has(FOLLOW_UP_COMPLETE), "{f:?}");
}

/// Marked complete in Outlook before this version read the mark: the cache has the message
/// without its tick, and a sync that asks only what changed never brings it. The first sync of
/// each folder asks once for its completed follow-ups (one request a folder, more for more than
/// a page), ticks those kept here, and never asks again; a pass cut short goes on where it
/// stopped.
#[test]
fn follow_ups_completed_before_this_version_are_ticked_once() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let projects = d.folder("Projects", None, None, "IPF.Note");
    let mail = |s: &str| format!("From: a@x.example\r\nSubject: {s}\r\n\r\nx\r\n").into_bytes();
    let done = d.deliver(&inbox, &mail("done"), true, "2024-05-01T09:00:00Z");
    d.deliver(&inbox, &mail("plain"), true, "2024-05-02T09:00:00Z");
    // Three in a folder: two pages of two (the tests' page).
    let finished: Vec<String> =
        (0..3).map(|n| d.deliver(&projects, &mail(&format!("task {n}")), true, &format!("2024-06-0{}T09:00:00Z", n + 1))).collect();
    for i in d.items.iter_mut() {
        if i.id == done || finished.contains(&i.id) {
            i.complete = true;
            i.complete_time = Some("2026-10-30T09:12:00Z".into());
        }
    }
    let server = fake_ews::serve(d);
    let dir = std::env::temp_dir().join(format!("quatrelle-ews-ticks-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let user = format!("{}\\{}", fake_ews::DOMAIN, fake_ews::USER);
    let c = Client::connect(&server.url, &user, fake_ews::PASSWORD, "riley@contoso.example").unwrap();
    let mut st = State::default();
    list_folders(&c, &mut st, "a1").unwrap();
    let opts = SyncOptions { prefetch: 0, ..Default::default() };
    let folders = ["INBOX", "Projects"];
    let asked = || server.data.lock().unwrap().calls.iter().filter(|c| c.as_str() == "FindItem completed").count();
    let ticked = |folder: &str| {
        store.load_folder("a1", folder).unwrap().messages.iter().filter(|m| m.flags.follow_up_complete() && m.flags.completed > 0).count()
    };
    for f in folders {
        sync_folder(&c, &mut st, &store, "a1", f, &opts).unwrap();
    }
    assert_eq!(asked(), 0, "nothing kept yet: nothing to tick, the folders are listed in full");
    // As the version before left it: the cache without the ticks, the state without the pass.
    st = as_before(&store, &st, &folders);
    assert_eq!((ticked("INBOX"), ticked("Projects")), (0, 0));

    // Cut short: the Inbox had it, then the app closed.
    let r = sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    assert_eq!(ticked("INBOX"), 1, "the first sync ticks it");
    assert!(r.changed >= 1, "and says the folder changed: {r:?}");
    assert_eq!(asked(), 1, "one request for the Inbox");
    let m = store.load_folder("a1", "INBOX").unwrap().messages.into_iter().find(|m| m.subject == "done").unwrap();
    assert_eq!(m.flags.completed, jmap::utc_date("2026-10-30T09:12:00Z").unwrap(), "on the day Outlook noted");
    st.save(&store, "a1").unwrap();
    let mut st = State::load(&store, "a1").unwrap();

    // Started again: the Inbox isn't asked again, the other folder is (two pages).
    for f in folders {
        sync_folder(&c, &mut st, &store, "a1", f, &opts).unwrap();
    }
    assert_eq!((ticked("INBOX"), ticked("Projects")), (1, 3));
    assert_eq!(asked(), 3, "the Inbox once, the other folder's two pages");
    assert!(st.finish_ticks(&folders.map(str::to_string)), "every folder had it: the pass is done");
    for f in folders {
        sync_folder(&c, &mut st, &store, "a1", f, &opts).unwrap();
    }
    assert_eq!(asked(), 3, "never asked again");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The cache and the state as the version before left them: the ticks not read, no pass.
fn as_before(store: &Store, st: &State, folders: &[&str]) -> State {
    for f in folders {
        let mut fs = store.load_folder("a1", f).unwrap();
        for m in fs.messages.iter_mut() {
            m.flags.set_keyword(FOLLOW_UP_COMPLETE, false);
            m.flags.completed = 0;
        }
        store.save_folder("a1", f, &fs).unwrap();
    }
    let mut v = serde_json::to_value(st).unwrap();
    // (Neither pass: the one for replies and forwards came after, R18.)
    v.as_object_mut().unwrap().remove("ticks");
    v.as_object_mut().unwrap().remove("replies");
    serde_json::from_value::<State>(v).unwrap()
}

/// A shared mailbox's folder and a public folder followed have the pass too, each asked where
/// it lives (the shared mailbox anchored to its address, the public folder routed to the
/// mailbox holding its content), once.
#[test]
fn shared_and_public_folders_have_their_ticks_read_where_they_live() {
    use crate::fakes::fake_ews::SharedBox;
    let mut d = Data::default();
    d.pf_mailbox = Some("PFContent@contoso.example".into());
    d.pf_hierarchy = Some("PFHierarchy@contoso.example".into());
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let root = d.folder("IPM_SUBTREE", None, Some("publicfoldersroot"), "IPF.Note");
    let news = d.folder("Announcements", Some(&root), None, "IPF.Note");
    let team =
        d.shared_mailbox(SharedBox { address: "team@contoso.example".into(), name: "Team".into(), automap: false, legacy: false, granted: true });
    let mail = |s: &str| format!("From: a@x.example\r\nSubject: {s}\r\n\r\nx\r\n").into_bytes();
    let a = d.deliver(&news, &mail("public"), true, "2026-10-14T09:00:00Z");
    let b = d.deliver(&team, &mail("shared"), true, "2026-10-14T10:00:00Z");
    for i in d.items.iter_mut().filter(|i| i.id == a || i.id == b) {
        i.complete = true;
        i.complete_time = Some("2026-10-30T09:12:00Z".into());
    }
    let server = fake_ews::serve(d);
    let user = format!("{}\\{}", fake_ews::DOMAIN, fake_ews::USER);
    let c = Client::connect(&server.url, &user, fake_ews::PASSWORD, "riley@contoso.example").unwrap();
    c.set_shared(&["team@contoso.example".to_string()]);
    let store = crate::fake::tests_store("ews-ticks-shared-public");
    let mut st = State::default();
    let listed = list_folders(&c, &mut st, "a1").unwrap().folders;
    let news_path = listed.iter().find(|f| f.path.ends_with("Announcements")).map(|f| f.path.clone()).unwrap();
    let folders = [news_path.as_str(), "Team/Inbox"];
    let opts = SyncOptions { prefetch: 0, ..Default::default() };
    for f in folders {
        sync_folder(&c, &mut st, &store, "a1", f, &opts).unwrap();
    }
    st = as_before(&store, &st, &folders);
    let asked = || server.data.lock().unwrap().calls.iter().filter(|c| c.as_str() == "FindItem completed").count();
    for f in folders {
        sync_folder(&c, &mut st, &store, "a1", f, &opts).unwrap();
        let fs = store.load_folder("a1", f).unwrap();
        assert!(fs.messages.iter().all(|m| m.flags.follow_up_complete() && m.flags.completed > 0), "{f}: {:?}", fs.messages);
    }
    assert_eq!(asked(), 2, "once each, where it lives");
    // The pass for replies and forwards too, there (R18).
    let replies = server.data.lock().unwrap().calls.iter().filter(|c| c.as_str() == "FindItem replied").count();
    assert_eq!(replies, 2, "the replies asked for where each lives");
    let _ = std::fs::remove_dir_all(store.root());
}

/// Marked complete here (the flag off, then the keyword on, as the engine sends it): Exchange
/// keeps the follow-up's dates — the list shows them struck through — and only its reminder goes
/// with the icon. A lone flag-off write (Undo of the keyword) leaves the dates too; Clear Flag's
/// own SetFollowUp takes them off.
#[test]
fn marking_complete_keeps_the_follow_up_dates_on_exchange() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let id = d.deliver(&inbox, b"From: a@x.example\r\nSubject: due\r\n\r\nx\r\n", true, "2024-05-01T09:00:00Z");
    for i in d.items.iter_mut() {
        i.flagged = true;
        i.follow_up = fake_ews::FollowUp {
            start: Some("2026-11-02T00:00:00Z".into()),
            due: Some("2026-11-06T00:00:00Z".into()),
            common_start: Some("2026-11-02T00:00:00Z".into()),
            common_end: Some("2026-11-06T00:00:00Z".into()),
            reminder_set: true,
            reminder_time: Some("2026-11-06T09:00:00Z".into()),
            reminder_signal: Some("2026-11-06T09:00:00Z".into()),
            request: Some("Follow up".into()),
            title: None,
        };
    }
    let server = fake_ews::serve(d);
    let dir = std::env::temp_dir().join(format!("quatrelle-ews-complete-dates-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let user = format!("{}\\{}", fake_ews::DOMAIN, fake_ews::USER);
    let c = Client::connect(&server.url, &user, fake_ews::PASSWORD, "riley@contoso.example").unwrap();
    let mut st = State::default();
    list_folders(&c, &mut st, "a1").unwrap();
    let opts = SyncOptions { prefetch: 0, ..Default::default() };
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    let m = store.load_folder("a1", "INBOX").unwrap().messages[0].clone();
    let keys = vec![m.key];
    let at = |text: &str| jmap::utc_date(text).unwrap();

    // Mark Complete: the flag off, then the keyword on.
    let flag = |on| Op::SetFlag { keys: keys.clone(), flag: Flag::Flagged, on, all: false };
    let complete = |on| Op::SetKeyword { keys: keys.clone(), keyword: FOLLOW_UP_COMPLETE.into(), on, name: None };
    for op in [flag(false), complete(true)] {
        apply_remote(&c, &st, &op).unwrap();
    }
    {
        let d = server.data.lock().unwrap();
        let f = &d.items.iter().find(|i| i.id == id).unwrap().follow_up;
        // The dates stay, Task and Common; the reminder goes with the icon.
        assert_eq!(f.start.as_deref().and_then(jmap::utc_date), Some(at("2026-11-02T00:00:00Z")), "{f:?}");
        assert_eq!(f.due.as_deref().and_then(jmap::utc_date), Some(at("2026-11-06T00:00:00Z")), "{f:?}");
        assert_eq!(f.common_start.as_deref().and_then(jmap::utc_date), Some(at("2026-11-02T00:00:00Z")), "{f:?}");
        assert_eq!(f.common_end.as_deref().and_then(jmap::utc_date), Some(at("2026-11-06T00:00:00Z")), "{f:?}");
        assert!(!f.reminder_set && f.reminder_time.is_none() && f.reminder_signal.is_none(), "{f:?}");
    }
    // And the next sync reads them back on the completed message.
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    let m = store.load_folder("a1", "INBOX").unwrap().messages[0].clone();
    assert!(m.flags.follow_up_complete());
    assert_eq!((m.flags.follow_up.start, m.flags.follow_up.due), (at("2026-11-02T00:00:00Z"), at("2026-11-06T00:00:00Z")), "{:?}", m.flags.follow_up);

    // Clear Flag's own SetFollowUp takes the dates off.
    apply_remote(
        &c,
        &st,
        &Op::SetFollowUp { keys, start: Some(FieldChange::Clear), due: Some(FieldChange::Clear), reminder: Some(FieldChange::Clear) },
    )
    .unwrap();
    let d = server.data.lock().unwrap();
    let f = &d.items.iter().find(|i| i.id == id).unwrap().follow_up;
    assert!(f.start.is_none() && f.due.is_none() && f.common_start.is_none() && f.common_end.is_none(), "{f:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A follow-up Outlook flagged before this version read its dates (the cache has the message
/// flagged with no dates, a sync that asks only what changed never brings them): the first sync
/// of each folder asks once for its flagged messages and reads their dates and reminder, and
/// never asks again. A change naming one field alone leaves the server's other fields.
#[test]
fn flagged_follow_ups_before_this_version_are_read_once() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let mail = |s: &str| format!("From: a@x.example\r\nSubject: {s}\r\n\r\nx\r\n").into_bytes();
    let flagged = d.deliver(&inbox, &mail("followed"), true, "2024-05-01T09:00:00Z");
    d.deliver(&inbox, &mail("plain"), true, "2024-05-02T09:00:00Z");
    for i in d.items.iter_mut() {
        if i.id == flagged {
            i.flagged = true;
            i.follow_up = fake_ews::FollowUp {
                start: Some("2026-11-02T00:00:00Z".into()),
                due: Some("2026-11-06T00:00:00Z".into()),
                common_start: Some("2026-11-02T00:00:00Z".into()),
                common_end: Some("2026-11-06T00:00:00Z".into()),
                reminder_set: true,
                reminder_time: Some("2026-11-06T09:00:00Z".into()),
                reminder_signal: Some("2026-11-06T09:00:00Z".into()),
                request: Some("Review".into()),
                title: None,
            };
        }
    }
    let server = fake_ews::serve(d);
    let dir = std::env::temp_dir().join(format!("quatrelle-ews-followups-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let user = format!("{}\\{}", fake_ews::DOMAIN, fake_ews::USER);
    let c = Client::connect(&server.url, &user, fake_ews::PASSWORD, "riley@contoso.example").unwrap();
    let mut st = State::default();
    list_folders(&c, &mut st, "a1").unwrap();
    let opts = SyncOptions { prefetch: 0, ..Default::default() };
    let at = |text: &str| jmap::utc_date(text).unwrap();
    let asked = || server.data.lock().unwrap().calls.iter().filter(|c| c.as_str() == "FindItem followUps").count();

    // Listed in full first: the flagged one comes in with its dates.
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    let m = store.load_folder("a1", "INBOX").unwrap().messages.iter().find(|m| m.subject == "followed").cloned().unwrap();
    assert_eq!(
        (m.flags.follow_up.start, m.flags.follow_up.due, m.flags.follow_up.reminder),
        (at("2026-11-02T00:00:00Z"), at("2026-11-06T00:00:00Z"), at("2026-11-06T09:00:00Z"))
    );
    assert_eq!(asked(), 0, "nothing kept before it was listed in full: no pass");

    // As the version before left it: the cache without the dates, the state without the pass.
    {
        let mut fs = store.load_folder("a1", "INBOX").unwrap();
        for m in fs.messages.iter_mut() {
            m.flags.follow_up = Default::default();
        }
        store.save_folder("a1", "INBOX", &fs).unwrap();
        let mut v = serde_json::to_value(&st).unwrap();
        v.as_object_mut().unwrap().remove("follow_ups");
        st = serde_json::from_value::<State>(v).unwrap();
    }
    // The first sync set the folder's sync state: only what changed is asked now.
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    assert_eq!(asked(), 1, "the flagged follow-ups asked for once");
    let m = store.load_folder("a1", "INBOX").unwrap().messages.iter().find(|m| m.subject == "followed").cloned().unwrap();
    assert_eq!(
        (m.flags.follow_up.start, m.flags.follow_up.due, m.flags.follow_up.reminder),
        (at("2026-11-02T00:00:00Z"), at("2026-11-06T00:00:00Z"), at("2026-11-06T09:00:00Z")),
        "the pass read them: {:?}",
        m.flags.follow_up
    );
    assert_eq!(m.flags.follow_up.title, "Review");
    // Never asked again.
    st.save(&store, "a1").unwrap();
    let mut st = State::load(&store, "a1").unwrap();
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    assert_eq!(asked(), 1, "never asked again");

    // A change naming one field alone (Add Reminder…): the server's dates stay.
    let keys = vec![m.key];
    let reminder = at("2026-11-07T08:00:00Z");
    apply_remote(&c, &st, &Op::SetFollowUp { keys, start: None, due: None, reminder: Some(FieldChange::Set(reminder)) }).unwrap();
    {
        let d = server.data.lock().unwrap();
        let f = &d.items.iter().find(|i| i.id == flagged).unwrap().follow_up;
        assert_eq!(f.start.as_deref().and_then(jmap::utc_date), Some(at("2026-11-02T00:00:00Z")), "start left as it was: {f:?}");
        assert_eq!(f.due.as_deref().and_then(jmap::utc_date), Some(at("2026-11-06T00:00:00Z")), "due left as it was: {f:?}");
        assert_eq!(f.common_start.as_deref().and_then(jmap::utc_date), Some(at("2026-11-02T00:00:00Z")), "{f:?}");
        assert_eq!(f.reminder_signal.as_deref().and_then(jmap::utc_date), Some(reminder), "{f:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The To-Do List's title is the task's (`PidLidToDoTitle`, 0x85A4), read with the flag's
/// fields everywhere; what the flag asks for (`PidLidFlagRequest`) is the fallback.
#[test]
fn the_follow_up_s_title_is_the_task_s_todo_title() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let _id = d.deliver(&inbox, b"From: a@x.example\r\nSubject: due\r\n\r\nx\r\n", true, "2024-05-01T09:00:00Z");
    for i in d.items.iter_mut() {
        i.flagged = true;
        i.follow_up = fake_ews::FollowUp {
            due: Some("2026-11-06T00:00:00Z".into()),
            request: Some("Follow up".into()),
            title: Some("Review the proofs".into()),
            ..Default::default()
        };
    }
    let server = fake_ews::serve(d);
    let dir = std::env::temp_dir().join(format!("quatrelle-ews-title-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let user = format!("{}\\{}", fake_ews::DOMAIN, fake_ews::USER);
    let c = Client::connect(&server.url, &user, fake_ews::PASSWORD, "riley@contoso.example").unwrap();
    let mut st = State::default();
    list_folders(&c, &mut st, "a1").unwrap();
    let opts = SyncOptions { prefetch: 0, ..Default::default() };

    sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    let m = store.load_folder("a1", "INBOX").unwrap().messages[0].clone();
    assert_eq!(m.flags.follow_up.title, "Review the proofs", "the task's title, not the flag's request: {:?}", m.flags.follow_up);

    // One with no task title: the flag's request.
    {
        let d = server.data.lock().unwrap();
        let inbox = d.folders.iter().find(|f| f.name == "Inbox").map(|f| f.id.clone()).unwrap_or_default();
        drop(d);
        let _id2 = server.data.lock().unwrap().deliver(&inbox, b"From: a@x.example\r\nSubject: plain\r\n\r\nx\r\n", true, "2024-05-02T09:00:00Z");
        server.data.lock().unwrap().items.iter_mut().filter(|i| i.id == _id2).for_each(|i| {
            i.flagged = true;
            i.follow_up.request = Some("Review".into());
        });
    }
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    let m = store.load_folder("a1", "INBOX").unwrap().messages.iter().find(|m| m.subject == "plain").cloned().unwrap();
    assert_eq!(m.flags.follow_up.title, "Review", "the flag's request when the task has no title: {:?}", m.flags.follow_up);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The EWS writes follow [MS-OXOTASK]/[MS-OXOFLAG]: a start date set without a due one takes
/// the same day for both (the due is required with the start), and Clear Flag takes the task's
/// title (`PidLidToDoTitle`) off with the rest of the flag's properties.
#[test]
fn a_start_date_goes_with_a_due_one_and_clear_flag_takes_the_title_off() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let id = d.deliver(&inbox, b"From: a@x.example\r\nSubject: ews writes\r\n\r\nx\r\n", true, "2024-05-01T09:00:00Z");
    for i in d.items.iter_mut() {
        i.flagged = true;
        i.follow_up = fake_ews::FollowUp { request: Some("Follow up".into()), title: Some("Review".into()), ..Default::default() };
    }
    let server = fake_ews::serve(d);
    let dir = std::env::temp_dir().join(format!("quatrelle-ews-writes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let user = format!("{}\\{}", fake_ews::DOMAIN, fake_ews::USER);
    let c = Client::connect(&server.url, &user, fake_ews::PASSWORD, "riley@contoso.example").unwrap();
    let mut st = State::default();
    list_folders(&c, &mut st, "a1").unwrap();
    let opts = SyncOptions { prefetch: 0, ..Default::default() };
    let at = |text: &str| jmap::utc_date(text).unwrap();
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    let m = store.load_folder("a1", "INBOX").unwrap().messages[0].clone();
    let keys = vec![m.key.clone()];

    // A start without a due: both land on the server, the same day.
    let start = at("2026-12-01T00:00:00Z");
    apply_remote(&c, &st, &Op::SetFollowUp { keys: keys.clone(), start: Some(FieldChange::Set(start)), due: None, reminder: None }).unwrap();
    {
        let d = server.data.lock().unwrap();
        let f = &d.items.iter().find(|i| i.id == id).unwrap().follow_up;
        assert_eq!(
            (
                f.start.as_deref().and_then(jmap::utc_date),
                f.due.as_deref().and_then(jmap::utc_date),
                f.common_start.is_some(),
                f.common_end.is_some()
            ),
            (Some(start), Some(start), true, true),
            "a start set without a due takes the same day for both: {f:?}"
        );
    }

    // Clear Flag: the title goes with the rest of the flag.
    apply_remote(&c, &st, &Op::SetFlag { keys: keys.clone(), flag: Flag::Flagged, on: false, all: false }).unwrap();
    {
        let d = server.data.lock().unwrap();
        let i = d.items.iter().find(|i| i.id == id).unwrap();
        assert!(!i.flagged && i.follow_up.request.is_none() && i.follow_up.title.is_none(), "the flag's properties all gone: {i:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
