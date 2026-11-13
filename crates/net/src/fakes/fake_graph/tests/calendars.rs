// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A calendar made, renamed or deleted on Microsoft 365 through the account's journal, the
//! calendars read again after each; what the server refuses said, and its list read again too.

use quatrelle_model::pim::{CalendarInfo, DavSync};

use super::*;
use crate::worker::{MemoryCredentials, Pool};

fn start(server: &Server, store: &Store) -> Pool {
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    pool.opts = crate::sync::SyncOptions { prefetch: 0, ..Default::default() };
    let account = AccountConfig { id: "a1".into(), email: "me@contoso.example".into(), graph: Some(server.base.clone()), ..Default::default() };
    pool.submit(Job::Start { account, password: Some(Secret(TOKEN.into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    pool
}

/// What a calendar sync among `seen` found, the last one.
fn last_sync(seen: &[Event]) -> &DavSync {
    seen.iter()
        .rev()
        .find_map(|e| match e {
            Event::DavSynced { sync, .. } => Some(sync.as_ref()),
            _ => None,
        })
        .expect("a calendar sync")
}

/// A calendar sync whose list has a calendar answering `f`.
fn lists(e: &Event, f: impl Fn(&CalendarInfo) -> bool) -> bool {
    matches!(e, Event::DavSynced { sync, .. } if sync.calendars.as_ref().is_some_and(|l| l.iter().any(&f)))
}

fn refused(seen: &[Event]) -> Vec<String> {
    seen.iter()
        .filter_map(|e| match e {
            Event::OpFailed { error, .. } => Some(error.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_calendar_is_made_renamed_and_deleted_then_read_again() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"));
    d.calendars.push(("CAL1".into(), "Calendar".into()));
    d.calendars.push(("CAL2".into(), "Trips".into()));
    d.events.push(json!({"id": "E1", "calendar": "CAL2", "iCalUId": "trip", "subject": "Train to Lyon",
        "start": {"dateTime": "2026-11-20T09:00:00.0000000"}, "end": {"dateTime": "2026-11-20T11:00:00.0000000"}}));
    let server = serve(d);
    let store = crate::fake::tests_store("graph-calendar-changes");
    let mut pool = start(&server, &store);
    pool.submit(Job::SyncDav { account: "a1".into() });
    let seen = until(&pool, "the calendars", |e| lists(e, |c| c.id == "CAL2"));
    let first = last_sync(&seen).calendars.clone().unwrap_or_default();
    assert!(first.iter().any(|c| c.id == "CAL1" && c.main), "the default calendar is the main one: {first:?}");
    assert!(first.iter().any(|c| c.id == "CAL2" && !c.main));

    pool.submit(Job::Apply { account: "a1".into(), op: Op::CreateCalendar { name: "Garden club".into() } });
    let seen = until(&pool, "the new calendar listed", |e| lists(e, |c| c.name == "Garden club"));
    assert!(refused(&seen).is_empty(), "{seen:?}");
    let made = server.data.lock().unwrap().calendars.iter().find(|(_, n)| n == "Garden club").map(|(c, _)| c.clone()).unwrap();

    pool.submit(Job::Apply { account: "a1".into(), op: Op::RenameCalendar { calendar: made.clone(), name: "Garden".into() } });
    until(&pool, "renamed", |e| lists(e, |c| c.id == made && c.name == "Garden"));

    pool.submit(Job::Apply { account: "a1".into(), op: Op::DeleteCalendar { calendar: "CAL2".into() } });
    let seen = until(
        &pool,
        "deleted",
        |e| matches!(e, Event::DavSynced { sync, .. } if sync.calendars.as_ref().is_some_and(|l| !l.iter().any(|c| c.id == "CAL2"))),
    );
    assert!(last_sync(&seen).events.iter().all(|e| e.calendar != "CAL2"), "its events with it");
    {
        let d = server.data.lock().unwrap();
        assert!(!d.events.iter().any(|e| e["id"] == "E1"));
        for call in ["POST /me/calendars", &format!("PATCH /me/calendars/{made}"), "DELETE /me/calendars/CAL2"] {
            assert!(d.calls.iter().any(|c| c.starts_with(call)), "{call}: {:?}", d.calls);
        }
    }
    // Deleted on the server already (in Outlook, meanwhile): done, not refused.
    pool.submit(Job::Apply { account: "a1".into(), op: Op::DeleteCalendar { calendar: "CAL2".into() } });
    pool.submit(Job::SyncDav { account: "a1".into() });
    let seen = until(&pool, "read again", |e| matches!(e, Event::DavSynced { .. }));
    assert!(refused(&seen).is_empty(), "{seen:?}");
    let _ = std::fs::remove_dir_all(store.root());
}

/// The server's refusals are said (a name taken, the default calendar), the change is dropped
/// from the journal, and the calendars are read again so what's shown is the server's.
#[test]
fn what_the_server_refuses_is_said_and_its_list_read_again() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"));
    d.calendars.push(("CAL1".into(), "Calendar".into()));
    d.calendars.push(("CAL2".into(), "Trips".into()));
    let server = serve(d);
    let store = crate::fake::tests_store("graph-calendar-refused");
    let mut pool = start(&server, &store);
    pool.submit(Job::SyncDav { account: "a1".into() });
    until(&pool, "the calendars", |e| lists(e, |c| c.id == "CAL1"));
    for (op, why) in [
        (Op::CreateCalendar { name: "trips".into() }, "HTTP 409"),
        (Op::DeleteCalendar { calendar: "CAL1".into() }, "HTTP 400"),
        (Op::RenameCalendar { calendar: "NOPE".into(), name: "x".into() }, "HTTP 404"),
    ] {
        pool.submit(Job::Apply { account: "a1".into(), op });
        let said = refused(&until(&pool, "the refusal", |e| matches!(e, Event::OpFailed { .. })));
        assert!(said.iter().any(|e| e.contains(why)), "{why}: {said:?}");
        let before = server.data.lock().unwrap().calls.iter().filter(|c| c.starts_with("GET /me/calendars?")).count();
        until(&pool, "the calendars read again", |e| lists(e, |c| c.id == "CAL1"));
        let after = server.data.lock().unwrap().calls.iter().filter(|c| c.starts_with("GET /me/calendars?")).count();
        assert!(after > before, "read again after the refusal");
    }
    assert!(store.load_ops("a1").unwrap().is_empty(), "no change left waiting");
    let d = server.data.lock().unwrap();
    assert_eq!(d.calendars.len(), 2, "{:?}", d.calendars);
    drop(d);
    let _ = std::fs::remove_dir_all(store.root());
}

/// A calendar someone else owns, shared with the account and listed among its own (canEdit as
/// theirs gave it), is never the account's own; nor is any but the main one when the calendar
/// groups couldn't be read (whose each is can't be told then).
#[test]
fn a_calendar_someone_else_owns_is_never_the_accounts_own() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"));
    d.calendars.push(("CAL1".into(), "Calendar".into()));
    d.calendars.push(("CAL2".into(), "Trips".into()));
    d.calendars.push(("CAL3".into(), "Ana Lima".into()));
    d.calendar_owners.push(("CAL3".into(), "ana@contoso.example".into()));
    let server = serve(d);
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let not_own =
        |sync: &quatrelle_model::pim::DavSync, id: &str| sync.calendars.as_ref().and_then(|l| l.iter().find(|c| c.id == id)).map(|c| c.not_own);
    let sync = crate::graph_pim::sync(&c, &quatrelle_model::pim::Pim::default(), 1_792_000_000).unwrap();
    assert_eq!((not_own(&sync, "CAL1"), not_own(&sync, "CAL2"), not_own(&sync, "CAL3")), (Some(false), Some(false), Some(true)));
    server.data.lock().unwrap().groups_refused = true;
    let sync = crate::graph_pim::sync(&c, &quatrelle_model::pim::Pim::default(), 1_792_000_000).unwrap();
    assert_eq!(
        (not_own(&sync, "CAL1"), not_own(&sync, "CAL2")),
        (Some(false), Some(true)),
        "the groups unread: none counted as own but the main one"
    );
}
