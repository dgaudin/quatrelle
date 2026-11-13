// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The mailbox's own other calendars (folders of appointments under Calendar, as Outlook makes
//! them): listed, read by their changes as the Calendar is, left unread away from the calendar,
//! new appointments made in their folder; and a calendar made, renamed or deleted there.

use quatrelle_model::ical::CalEvent;
use quatrelle_model::pim::{CalendarInfo, DavSync, Pim, StoredEvent};

use super::*;
use crate::fakes::fake_ews::server::Server;

const NOW: i64 = 1_792_000_000;

fn appointment(subject: &str, day: u32, uid: &str) -> El {
    parse(&format!(
        "<t:CalendarItem><t:Subject>{subject}</t:Subject><t:Start>2026-11-{day:02}T09:00:00Z</t:Start><t:End>2026-11-{day:02}T10:00:00Z</t:End>\
         <t:IsAllDayEvent>false</t:IsAllDayEvent><t:CalendarItemType>Single</t:CalendarItemType><t:UID>{uid}</t:UID></t:CalendarItem>"
    ))
    .kids[0]
        .clone()
}

/// A mailbox with its Calendar, Trips under it, Team trips under Trips, a folder of notes under
/// Calendar (not a calendar), Deleted Items; an appointment in each calendar. Its folder ids:
/// (Calendar, Trips, Team trips).
fn mailbox() -> (Data, String, String, String) {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Deleted Items", None, Some("deleteditems"), "IPF.Note");
    let cal = d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    let trips = d.folder("Trips\u{202E}\n", Some(&cal), None, "IPF.Appointment");
    let team = d.folder("Team trips", Some(&trips), None, "IPF.Appointment.Shared");
    d.folder("Ideas", Some(&cal), None, "IPF.Note");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    d.pim.push(("M1".into(), cal.clone(), appointment("Stand-up", 16, "standup")));
    d.pim.push(("T1".into(), trips.clone(), appointment("Train to Lyon", 20, "train")));
    d.pim.push(("R1".into(), team.clone(), appointment("Team offsite", 24, "offsite")));
    (d, cal, trips, team)
}

fn client(server: &Server) -> crate::ews::Client {
    crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap()
}

fn id(folder: &str) -> String {
    format!("ews:folder:{folder}")
}

fn listed<'a>(sync: &'a DavSync, folder: &str) -> Option<&'a CalendarInfo> {
    sync.calendars.as_ref().and_then(|l| l.iter().find(|c| c.id == id(folder)))
}

fn summaries(sync: &DavSync, calendar: &str) -> Vec<String> {
    sync.events.iter().filter(|e| e.calendar == calendar).map(|e| e.event.summary.clone()).collect()
}

/// Listed under the main calendar (a folder of notes isn't a calendar), named on one line, each
/// read whole the first time with a state to go on from, then by its changes: one appointment
/// more read, the other kept as it was.
#[test]
fn own_other_calendars_are_listed_then_read_by_their_changes() {
    let (d, cal, trips, team) = mailbox();
    let server = serve(d);
    let c = client(&server);
    let first = crate::ews_pim::sync(&c, &Pim::default(), NOW).unwrap();
    assert!(first.errors.is_empty(), "{:?}", first.errors);
    let main = first.calendars.as_ref().and_then(|l| l.iter().find(|c| c.id == crate::ews_pim::CALENDAR)).cloned().unwrap();
    assert!(main.main && !main.read_only);
    let t = listed(&first, &trips).cloned().unwrap_or_else(|| panic!("Trips listed: {:?}", first.calendars));
    assert_eq!((t.name.as_str(), t.main, t.read_only, t.group.as_str()), ("Trips", false, false, ""));
    assert!(t.ctag.starts_with("f1:"), "a state to go on from: {t:?}");
    assert!(listed(&first, &team).is_some(), "a calendar inside another, its class a kind of appointment folder");
    assert_eq!(first.calendars.as_ref().map(Vec::len), Some(3), "the folder of notes isn't one: {:?}", first.calendars);
    assert_eq!(summaries(&first, &id(&trips)), ["Train to Lyon"]);
    assert_eq!(summaries(&first, &id(&team)), ["Team offsite"]);
    assert!(first.synced_calendars.contains(&id(&trips)));
    assert!(!summaries(&first, crate::ews_pim::CALENDAR).contains(&"Train to Lyon".to_string()), "each in its own calendar");
    let _ = cal;

    server.data.lock().unwrap().pim.push(("T2".into(), trips.clone(), appointment("Hotel check-in", 20, "hotel")));
    let pim = Pim { calendars: first.calendars.clone().unwrap_or_default(), events: first.events.clone(), ..Default::default() };
    let second = crate::ews_pim::sync(&c, &pim, NOW).unwrap();
    assert!(second.errors.is_empty(), "{:?}", second.errors);
    assert_eq!(summaries(&second, &id(&trips)), ["Hotel check-in"], "only what changed is read");
    assert!(second.kept.contains(&"T1".to_string()), "the rest kept as it was: {:?}", second.kept);
    assert!(second.synced_calendars.contains(&id(&trips)));
}

/// Away from the calendar only the main one is read, as other people's wait; one unticked is
/// listed as it was, not read.
#[test]
fn away_from_the_calendar_or_unticked_they_wait() {
    let (d, _, trips, team) = mailbox();
    let server = serve(d);
    let c = client(&server);
    let first = crate::ews_pim::sync(&c, &Pim::default(), NOW).unwrap();
    let pim = Pim { calendars: first.calendars.clone().unwrap_or_default(), events: first.events.clone(), own_only: true, ..Default::default() };
    let away = crate::ews_pim::sync(&c, &pim, NOW).unwrap();
    assert!(away.partial, "the others stay as they are");
    assert_eq!(away.calendars.as_ref().map(|l| l.iter().map(|c| c.id.clone()).collect::<Vec<_>>()), Some(vec![crate::ews_pim::CALENDAR.to_string()]));
    assert!(!away.synced_calendars.contains(&id(&trips)));

    let kept_state = listed(&first, &team).map(|c| c.ctag.clone()).unwrap();
    let pim = Pim { own_only: false, not_read: vec![id(&team)], ..pim };
    let unticked = crate::ews_pim::sync(&c, &pim, NOW).unwrap();
    let t = listed(&unticked, &team).cloned().unwrap();
    assert_eq!(t.ctag, kept_state, "listed as it was");
    assert!(!unticked.synced_calendars.contains(&id(&team)) && summaries(&unticked, &id(&team)).is_empty(), "not read");
    assert!(unticked.synced_calendars.contains(&id(&trips)), "the ticked one read");
}

/// A calendar whose folder the server refuses is passed over (listed as it was, not read): the
/// others are read, and the sync goes on.
#[test]
fn a_calendar_the_server_refuses_is_passed_over() {
    let (mut d, _, trips, team) = mailbox();
    d.refused_folders.push(team.clone());
    let server = serve(d);
    let c = client(&server);
    let sync = crate::ews_pim::sync(&c, &Pim::default(), NOW).unwrap();
    assert!(listed(&sync, &team).is_some(), "still listed: {:?}", sync.calendars);
    assert!(!sync.synced_calendars.contains(&id(&team)) && summaries(&sync, &id(&team)).is_empty());
    assert_eq!(summaries(&sync, &id(&trips)), ["Train to Lyon"]);
}

/// An appointment made here in Trips is made in Trips' folder, and stays in that calendar; one
/// changed there goes by its item.
#[test]
fn an_appointment_made_in_another_calendar_goes_to_its_folder() {
    let (d, cal, trips, _) = mailbox();
    let server = serve(d);
    let c = client(&server);
    let first = crate::ews_pim::sync(&c, &Pim::default(), NOW).unwrap();
    let mut pim = Pim { calendars: first.calendars.clone().unwrap_or_default(), events: first.events.clone(), ..Default::default() };
    let event = CalEvent { uid: "museum-1".into(), summary: "Museum".into(), start: NOW + 86_400, end: NOW + 90_000, ..Default::default() };
    pim.events.push(StoredEvent { calendar: id(&trips), event, dirty: true, ..Default::default() });
    let sync = crate::ews_pim::sync(&c, &pim, NOW).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    let pushed = sync.pushed_events.iter().find(|e| e.event.summary == "Museum").cloned().unwrap();
    assert_eq!(pushed.calendar, id(&trips), "it stays in Trips");
    let folder = server.data.lock().unwrap().pim.iter().find(|(i, _, _)| *i == pushed.href).map(|(_, f, _)| f.clone());
    assert_eq!(folder.as_deref(), Some(trips.as_str()), "made in Trips' folder, not in {cal}");
}

/// Made under Calendar as a calendar folder, renamed, deleted into Deleted Items (with its
/// appointments); a name taken refused as Exchange says it, the main calendar never changed, one
/// deleted already counted as done.
#[test]
fn a_calendar_is_made_renamed_and_deleted_on_exchange() {
    let (d, cal, trips, _) = mailbox();
    let server = serve(d);
    let c = client(&server);
    let mut st = crate::jmap::State::default();
    crate::ews::list_folders(&c, &mut st, "a1").unwrap();
    crate::ews::apply_remote(&c, &st, &Op::CreateCalendar { name: "Garden club".into() }).unwrap();
    {
        let d = server.data.lock().unwrap();
        let made = d.folders.iter().find(|f| f.name == "Garden club").cloned().unwrap();
        assert_eq!((made.parent.as_str(), made.class.as_str()), (cal.as_str(), "IPF.Appointment"));
    }
    let taken = crate::ews::apply_remote(&c, &st, &Op::CreateCalendar { name: "Trips\u{202E}\n".into() });
    assert!(matches!(&taken, Err(crate::NetError::No(e)) if e.contains("ErrorFolderExists")), "{taken:?}");

    crate::ews::apply_remote(&c, &st, &Op::RenameCalendar { calendar: id(&trips), name: "Travel".into() }).unwrap();
    assert_eq!(server.data.lock().unwrap().folders.iter().find(|f| f.id == trips).map(|f| f.name.clone()).as_deref(), Some("Travel"));
    let calls = server.data.lock().unwrap().calls.len();
    for op in [
        Op::RenameCalendar { calendar: crate::ews_pim::CALENDAR.into(), name: "Agenda".into() },
        Op::DeleteCalendar { calendar: crate::ews_pim::CALENDAR.into() },
        Op::RenameCalendar { calendar: "shared:ana@contoso.example".into(), name: "x".into() },
    ] {
        assert!(matches!(crate::ews::apply_remote(&c, &st, &op), Err(crate::NetError::No(_))), "{op:?}");
    }
    assert_eq!(server.data.lock().unwrap().calls.len(), calls, "none of those asked of the server");
    let gone = crate::ews::apply_remote(&c, &st, &Op::RenameCalendar { calendar: id("NOPE"), name: "x".into() });
    assert!(matches!(&gone, Err(crate::NetError::No(e)) if e.contains("not on the server")), "{gone:?}");

    crate::ews::apply_remote(&c, &st, &Op::DeleteCalendar { calendar: id(&trips) }).unwrap();
    {
        let d = server.data.lock().unwrap();
        let bin = d.folders.iter().find(|f| f.known.as_deref() == Some("deleteditems")).map(|f| f.id.clone()).unwrap();
        assert_eq!(d.folders.iter().find(|f| f.id == trips).map(|f| f.parent.clone()), Some(bin), "into Deleted Items");
        assert!(d.pim.iter().any(|(i, f, _)| i == "T1" && *f == trips), "its appointments with it");
    }
    crate::ews::apply_remote(&c, &st, &Op::DeleteCalendar { calendar: id("NOPE") }).unwrap();
    let after = crate::ews_pim::sync(&c, &Pim::default(), NOW).unwrap();
    assert!(listed(&after, &trips).is_none(), "no longer under Calendar: {:?}", after.calendars);
    assert!(after.calendars.as_ref().is_some_and(|l| l.iter().any(|c| c.name == "Garden club")));
}

/// Through the account's journal: each change made, then the calendars read again.
#[test]
fn the_journal_makes_renames_and_deletes_then_reads_the_calendars_again() {
    let (d, _, trips, _) = mailbox();
    let server = serve(d);
    let store = crate::fake::tests_store("ews-calendar-changes");
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "riley@contoso.example".into(),
        username: format!("{DOMAIN}\\{USER}"),
        ews: Some(server.url.clone()),
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret(PASSWORD.into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let lists = |e: &Event, f: &dyn Fn(&CalendarInfo) -> bool| matches!(e, Event::DavSynced { sync, .. } if sync.calendars.as_ref().is_some_and(|l| l.iter().any(f)));
    pool.submit(Job::Apply { account: "a1".into(), op: Op::CreateCalendar { name: "Garden club".into() } });
    until(&pool, "the new calendar listed", |e| lists(e, &|c| c.name == "Garden club"));
    pool.submit(Job::Apply { account: "a1".into(), op: Op::RenameCalendar { calendar: id(&trips), name: "Travel".into() } });
    until(&pool, "renamed", |e| lists(e, &|c| c.id == id(&trips) && c.name == "Travel"));
    pool.submit(Job::Apply { account: "a1".into(), op: Op::DeleteCalendar { calendar: id(&trips) } });
    let seen = until(
        &pool,
        "deleted",
        |e| matches!(e, Event::DavSynced { sync, .. } if sync.calendars.as_ref().is_some_and(|l| !l.iter().any(|c| c.id == id(&trips)) && l.iter().any(|c| c.name == "Garden club"))),
    );
    assert!(!seen.iter().any(|e| matches!(e, Event::OpFailed { .. })), "{seen:?}");
    let _ = std::fs::remove_dir_all(store.root());
}

/// After a calendar made, renamed or deleted, the calendars alone are read again: not the
/// contacts, notes, tasks and categories a whole calendar-and-contacts sync reads.
#[test]
fn after_a_change_only_the_calendars_are_read_again() {
    let (d, _, trips, _) = mailbox();
    let server = serve(d);
    let store = crate::fake::tests_store("ews-calendar-reread");
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "riley@contoso.example".into(),
        username: format!("{DOMAIN}\\{USER}"),
        ews: Some(server.url.clone()),
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret(PASSWORD.into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let categories = || server.data.lock().unwrap().calls.iter().filter(|c| c.as_str() == "GetUserConfiguration").count();
    // The first sync's whole calendar-and-contacts sync asks for the categories after it tells
    // what it read: once it has, and nothing more comes.
    eventually("the first sync's categories", || categories() >= 1);
    let mut before = categories();
    loop {
        std::thread::sleep(Duration::from_millis(700));
        if categories() == before {
            break;
        }
        before = categories();
    }
    pool.submit(Job::Apply { account: "a1".into(), op: Op::RenameCalendar { calendar: id(&trips), name: "Travel".into() } });
    until(
        &pool,
        "renamed",
        |e| matches!(e, Event::DavSynced { sync, .. } if sync.calendars.as_ref().is_some_and(|l| l.iter().any(|c| c.name == "Travel"))),
    );
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(categories(), before, "the categories (a whole sync's) weren't read again");
    let _ = std::fs::remove_dir_all(store.root());
}

/// Exchange's own kinds of calendar folder (Birthdays, `IPF.Appointment.Birthday`): listed and
/// read, but kept read-only, never renamed, deleted or filed into from here.
#[test]
fn a_birthdays_calendar_is_read_but_read_only() {
    let (mut d, cal, trips, team) = mailbox();
    let birthdays = d.folder("Birthdays", Some(&cal), None, "IPF.Appointment.Birthday");
    d.pim.push(("B1".into(), birthdays.clone(), appointment("Ana's birthday", 21, "ana-birthday")));
    let server = serve(d);
    let sync = crate::ews_pim::sync(&client(&server), &Pim::default(), NOW).unwrap();
    let b = listed(&sync, &birthdays).cloned().unwrap();
    assert!(b.read_only, "{b:?}");
    assert_eq!(summaries(&sync, &id(&birthdays)), ["Ana's birthday"]);
    assert!(listed(&sync, &team).is_some_and(|c| c.read_only), "any kind but the plain one");
    assert!(listed(&sync, &trips).is_some_and(|c| !c.read_only));
}

/// What a listing can't hold is said once in the log: a page that isn't the last (folders past
/// it unlisted); and a folder id past what's kept of one is passed over.
#[test]
fn a_listing_past_its_page_is_said_and_a_huge_id_passed_over() {
    let (mut d, cal, trips, _) = mailbox();
    let huge = "A".repeat(600);
    d.folders.push(Fo {
        id: huge.clone(),
        name: "Huge".into(),
        parent: cal.clone(),
        class: "IPF.Appointment".into(),
        known: None,
        owner: String::new(),
    });
    for n in 0..260 {
        d.folder(&format!("Old {n}"), Some(&cal), None, "IPF.Note");
    }
    let server = serve(d);
    let tap = crate::fakes::logs::tap(|l| l.contains("folders under the calendar"));
    let sync = crate::ews_pim::sync(&client(&server), &Pim::default(), NOW).unwrap();
    assert!(listed(&sync, &trips).is_some());
    assert!(listed(&sync, &huge).is_none(), "an id that long isn't kept");
    let said = tap.lines();
    assert_eq!(said.iter().filter(|l| l.contains("one page")).count(), 1, "{said:?}");
}
