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
    let flag = |on| Op::SetFlag { keys: keys.clone(), flag: Flag::Flagged, on };
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
    let mut st = State::load(&store, "a1");

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
    v.as_object_mut().unwrap().remove("ticks");
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
    let listed = list_folders(&c, &mut st, "a1").unwrap();
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
    let _ = std::fs::remove_dir_all(store.root());
}
