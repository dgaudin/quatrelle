// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: what's dropped because the server no longer has it is kept, as Outlook keeps
//! it in Sync Issues › Conflicts (the owner's decision, 03/11): a mailbox on this computer,
//! made at the first one, with one message for each, the version that was here attached.

use quatrelle_model::ical::CalEvent;
use quatrelle_model::pim::{CalendarInfo, DavSync, StoredEvent, StoredTask, Task};

use super::*;
use crate::session::pim::{CONFLICTS, SYNC_ISSUES};

const MINE: &str = "ews:calendar";

fn exchange(name: &str) -> (Session, String) {
    let mut s = Session::open(Some(temp(name)), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s
        .run("account.add", &json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    (s, id)
}

/// The messages in Sync Issues › Conflicts, with their sources.
fn conflicts(s: &Session) -> Vec<(String, Vec<u8>)> {
    let store = s.store.as_ref().unwrap();
    let st = store.load_folder(SYNC_ISSUES, CONFLICTS).unwrap_or_default();
    st.messages.iter().map(|m| (m.subject.clone(), store.read_body(&m.key).unwrap_or_default())).collect()
}

/// The one attachment of a message: its name and its text.
fn attached(raw: &[u8]) -> (String, String) {
    let p = crate::mime::parse(raw).unwrap();
    assert_eq!(p.attachments.len(), 1, "{:?}", p.attachments);
    let a = &p.attachments[0];
    let data = crate::mime::attachment_data(raw, a.index).unwrap();
    (a.name.clone(), String::from_utf8(data).unwrap())
}

#[test]
fn an_appointment_dropped_is_kept_in_sync_issues_conflicts() {
    let (mut s, id) = exchange("conflicts-event");
    assert!(s.account(SYNC_ISSUES).is_none(), "nothing shown before the first");
    let start = 1_792_400_400;
    let test = StoredEvent {
        calendar: MINE.into(),
        href: "GONE1".into(),
        etag: "ck".into(),
        event: CalEvent { uid: "test-1".into(), summary: "test".into(), location: "Bureau".into(), start, end: start + 1800, ..Default::default() },
        dirty: true,
        ..Default::default()
    };
    {
        let p = s.pim_mut(&id);
        p.calendars = vec![CalendarInfo { id: MINE.into(), name: "Calendar".into(), ..Default::default() }];
        p.events = vec![test.clone()];
    }
    let server = |gone: &[&str]| DavSync {
        calendars: Some(vec![CalendarInfo { id: MINE.into(), name: "Calendar".into(), ..Default::default() }]),
        synced_calendars: vec![MINE.into()],
        gone_events: gone.iter().map(|g| (*g).to_string()).collect(),
        ..Default::default()
    };
    s.apply_dav(&id, &server(&["GONE1"]));
    assert!(s.pim[&id].events.is_empty());
    let a = s.account(SYNC_ISSUES).expect("made at the first");
    assert!(a.is_local() && a.folders.iter().any(|f| f.path == CONFLICTS));
    let kept = conflicts(&s);
    assert_eq!(kept.len(), 1);
    assert!(kept[0].0.starts_with("test — deleted on the server ("), "{}", kept[0].0);
    let (name, ics) = attached(&kept[0].1);
    assert!(name.ends_with(".ics"), "{name}");
    assert!(ics.contains("METHOD:PUBLISH"), "{ics}");
    let back = quatrelle_model::ical::parse(&ics, 0).events;
    assert_eq!(back.len(), 1);
    assert_eq!((back[0].uid.as_str(), back[0].summary.as_str(), back[0].location.as_str(), back[0].start), ("test-1", "test", "Bureau", start));
    assert!(s.notices.iter().any(|n| n.text.starts_with("test: ") && n.text.contains("Sync Issues › Conflicts")), "the banner says where");
    // The next sync adds nothing; the mailbox here is never synced.
    s.apply_dav(&id, &server(&[]));
    assert_eq!(conflicts(&s).len(), 1);
    assert!(!s.take_jobs().iter().any(|j| j.account() == SYNC_ISSUES));
    let store = s.store.clone().unwrap();
    let mut again = Session::open(Some(store), quatrelle_model::dates::Clock::new(NOW, 0));
    assert!(again.account(SYNC_ISSUES).is_some(), "kept");
    assert!(!again.take_jobs().iter().any(|j| j.account() == SYNC_ISSUES), "no worker for it");
    done(s);
}

#[test]
fn a_task_dropped_is_kept_in_sync_issues_conflicts() {
    let (mut s, id) = exchange("conflicts-task");
    let task = StoredTask {
        href: "task:T1".into(),
        task: Task { uid: "task:T1".into(), subject: "Devis Martin".into(), notes: "Relancer".into(), ..Default::default() },
        dirty: true,
        ..Default::default()
    };
    s.pim_mut(&id).tasks = vec![task];
    let read = |gone: &[&str]| DavSync { tasks: Some(Vec::new()), gone_tasks: gone.iter().map(|g| (*g).to_string()).collect(), ..Default::default() };
    s.apply_dav(&id, &read(&["task:T1"]));
    let kept = conflicts(&s);
    assert_eq!(kept.len(), 1);
    assert!(kept[0].0.starts_with("Devis Martin — deleted on the server ("), "{}", kept[0].0);
    let (name, ics) = attached(&kept[0].1);
    assert!(name.ends_with(".ics") && ics.contains("BEGIN:VTODO") && ics.contains("SUMMARY:Devis Martin"), "{name}: {ics}");
    s.apply_dav(&id, &read(&[]));
    assert_eq!(conflicts(&s).len(), 1, "nothing added");
    done(s);
}
