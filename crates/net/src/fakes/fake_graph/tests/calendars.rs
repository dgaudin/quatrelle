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
        // Counted before the change goes: the worker may read the list again before the
        // refusal is heard here.
        let before = server.data.lock().unwrap().calls.iter().filter(|c| c.starts_with("GET /me/calendars?")).count();
        pool.submit(Job::Apply { account: "a1".into(), op });
        let said = refused(&until(&pool, "the refusal", |e| matches!(e, Event::OpFailed { .. })));
        assert!(said.iter().any(|e| e.contains(why)), "{why}: {said:?}");
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
    d.calendars.push(("CAL3".into(), "Ana Exemple".into()));
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

/// Ids Graph never gives (someone else's by address, a public folder's, one kept on this
/// computer, one subscribed to by its link) are refused without a request.
#[test]
fn ids_graph_never_gives_are_refused_without_asking() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"));
    d.calendars.push(("CAL1".into(), "Calendar".into()));
    let server = serve(d);
    let store = crate::fake::tests_store("graph-calendar-foreign-ids");
    let mut pool = start(&server, &store);
    for calendar in ["shared:ana@contoso.example", "public:AAMkPub=", "local", "web:https://holidays.example/de.ics"] {
        pool.submit(Job::Apply { account: "a1".into(), op: Op::DeleteCalendar { calendar: calendar.into() } });
        until(&pool, "the refusal", |e| matches!(e, Event::OpFailed { .. }));
    }
    let asked: Vec<String> = server.data.lock().unwrap().calls.iter().filter(|c| c.starts_with("DELETE /me/calendars/")).cloned().collect();
    assert!(asked.is_empty(), "{asked:?}");
    let _ = std::fs::remove_dir_all(store.root());
}

/// Calendars removed from the list on this computer (`Pim::removed`), one of the account's own
/// and a colleague's kept in a group: listed, their events never asked for, nothing of them read;
/// the others read as before.
#[test]
fn a_removed_calendar_is_listed_and_its_events_never_asked_for() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"));
    d.calendars.push(("CAL1".into(), "Calendar".into()));
    d.calendars.push(("CAL2".into(), "Trips".into()));
    d.groups.push(("G1".into(), "My Calendars".into(), vec!["CAL1".into(), "CAL2".into()]));
    d.groups.push(("G2".into(), "Team".into(), vec!["SC1".into()]));
    d.group_calendars.push(("SC1".into(), "Calendar".into(), "Zoe".into(), false));
    for (id, cal, subject) in [("E1", "CAL1", "Stand-up"), ("E2", "CAL2", "Train to Lyon"), ("E3", "SC1", "Planning")] {
        d.events.push(json!({"id": id, "calendar": cal, "iCalUId": id, "subject": subject,
            "start": {"dateTime": "2026-11-20T09:00:00.0000000"}, "end": {"dateTime": "2026-11-20T10:00:00.0000000"}}));
    }
    let server = serve(d);
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let pim = quatrelle_model::pim::Pim { removed: vec!["CAL2".into(), "SC1".into()], ..Default::default() };
    let sync = crate::graph_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    assert_eq!(sync.calendar_groups.as_deref(), Some(&["Team".to_string()][..]), "My Calendars isn't a group of its own");
    let listed = sync.calendars.clone().unwrap_or_default();
    assert!(["CAL1", "CAL2", "SC1"].iter().all(|id| listed.iter().any(|x| x.id == *id)), "each listed: {listed:?}");
    for id in ["CAL2", "SC1"] {
        assert!(!sync.synced_calendars.iter().any(|c| c == id) && sync.events.iter().all(|e| e.calendar != id), "{id} not read");
    }
    assert!(sync.events.iter().any(|e| e.calendar == "CAL1" && e.event.summary == "Stand-up"), "the main one read");
    let asked: Vec<String> = server.data.lock().unwrap().calls.iter().filter(|c| c.contains("/events")).cloned().collect();
    assert!(asked.iter().all(|c| !c.contains("CAL2") && !c.contains("SC1")), "{asked:?}");
    assert!(asked.iter().any(|c| c.contains("CAL1")), "{asked:?}");
}

/// Microsoft 365's calendar groups are made, renamed and deleted as Graph has them; moving a
/// calendar between groups, and ordering the groups, are refused with the reason (Graph v1.0
/// can't do either), so the pane can grey them.
#[test]
fn calendar_groups_are_made_renamed_and_deleted_on_microsoft_365() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"));
    d.calendars.push(("CAL1".into(), "Calendar".into()));
    d.groups.push(("G1".into(), "My Calendars".into(), vec!["CAL1".into()]));
    d.groups.push(("G2".into(), "Team".into(), Vec::new()));
    let server = serve(d);
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let st = crate::jmap::State::default();
    let names = || server.data.lock().unwrap().groups.iter().map(|(_, n, _)| n.clone()).collect::<Vec<_>>();
    crate::graph::apply_remote(&c, &st, &Op::CreateCalendarGroup { name: "Rooms".into() }).unwrap();
    assert!(names().iter().any(|n| n == "Rooms"), "{:?}", names());
    crate::graph::apply_remote(&c, &st, &Op::RenameCalendarGroup { group: "Rooms".into(), name: "Salles".into() }).unwrap();
    assert!(names().iter().any(|n| n == "Salles"), "{:?}", names());
    crate::graph::apply_remote(&c, &st, &Op::DeleteCalendarGroup { group: "Salles".into() }).unwrap();
    assert!(!names().iter().any(|n| n == "Salles"), "{:?}", names());
    let refused = |op: Op| crate::graph::apply_remote(&c, &st, &op).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(refused(Op::MoveCalendarGroup { group: "Team".into(), up: true }).contains("keeps no order"));
    assert!(refused(Op::MoveCalendarToGroup { calendar: "CAL1".into(), name: "Calendar".into(), group: "Team".into() }).contains("can't move"));
}

/// Deleting a group that still holds calendars isn't done: what Graph makes of them on a real
/// account isn't known, so none is risked.
#[test]
fn a_group_that_still_holds_calendars_is_not_deleted() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"));
    d.calendars.push(("CAL1".into(), "Calendar".into()));
    d.calendars.push(("CAL2".into(), "Trips".into()));
    d.groups.push(("G1".into(), "My Calendars".into(), vec!["CAL1".into()]));
    d.groups.push(("G2".into(), "Team".into(), vec!["CAL2".into()]));
    let server = serve(d);
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let st = crate::jmap::State::default();
    let e = crate::graph::apply_remote(&c, &st, &Op::DeleteCalendarGroup { group: "Team".into() }).unwrap_err();
    assert!(e.to_string().contains("still holds calendars"), "{e}");
    assert!(server.data.lock().unwrap().groups.iter().any(|(_, n, _)| n == "Team"), "the group stays");
}
