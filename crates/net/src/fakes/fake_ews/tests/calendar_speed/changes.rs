// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The calendar read by what changed in it, as Outlook's Cached Exchange Mode reads it: the
//! owner's 239 appointments, with series and occurrences changed on their own, were read whole at
//! each change, 3 to 6 s holding the main connection while new mail waited (06/11).

use std::time::Instant;

use quatrelle_model::pim::{DavSync, PIM_DOC, Pim};

use super::*;

/// The clock of these tests: Wednesday 14 October 2026, 17:46 UTC.
const NOW: i64 = 1_792_000_000;
/// The owner's calendar: plain appointments and weekly series, some of whose occurrences moved.
const SINGLES: usize = 219;
const SERIES: usize = 20;
const MOVED: usize = 5;

fn at(t: i64) -> String {
    crate::ews_pim::utc(t)
}

fn element(xml: &str) -> El {
    parse(xml).kids.remove(0)
}

/// The `n`th plain appointment: from five months back to two months on, half an hour each.
fn single(n: usize) -> El {
    let day = (NOW - 150 * 86_400 + i64::try_from(n).unwrap() * 210 * 86_400 / 219).div_euclid(86_400);
    let start = day * 86_400 + (8 + i64::try_from(n % 9).unwrap()) * 3600;
    element(&format!(
        "<t:CalendarItem><t:Subject>Meeting {n:03}</t:Subject><t:Start>{}</t:Start><t:End>{}</t:End>\
         <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>m-{n}</t:UID><t:CalendarItemType>Single</t:CalendarItemType></t:CalendarItem>",
        at(start),
        at(start + 1800)
    ))
}

/// When the `n`th series begins: a Monday fourteen weeks back.
fn series_start(n: usize) -> i64 {
    let day = NOW.div_euclid(86_400);
    let monday = day - (day + 3).rem_euclid(7) - 14 * 7;
    monday * 86_400 + (9 + i64::try_from(n % 8).unwrap()) * 3600
}

fn series(n: usize) -> El {
    let start = series_start(n);
    element(&format!(
        "<t:CalendarItem><t:Subject>Series {n:02}</t:Subject><t:Start>{}</t:Start><t:End>{}</t:End>\
         <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>s-{n}</t:UID><t:CalendarItemType>RecurringMaster</t:CalendarItemType>\
         <t:Recurrence><t:WeeklyRecurrence><t:Interval>1</t:Interval><t:DaysOfWeek>Monday</t:DaysOfWeek></t:WeeklyRecurrence>\
         <t:NoEndRecurrence><t:StartDate>{}</t:StartDate></t:NoEndRecurrence></t:Recurrence></t:CalendarItem>",
        at(start),
        at(start + 1800),
        at(start).get(..10).unwrap()
    ))
}

/// The third occurrence of the `n`th series, two hours later than its series says.
fn moved(n: usize, subject: &str) -> El {
    let was = series_start(n) + 2 * 7 * 86_400;
    element(&format!(
        "<t:CalendarItem><t:Subject>{subject}</t:Subject><t:Start>{}</t:Start><t:End>{}</t:End><t:OriginalStart>{}</t:OriginalStart>\
         <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>s-{n}</t:UID><t:CalendarItemType>Exception</t:CalendarItemType></t:CalendarItem>",
        at(was + 7200),
        at(was + 9000),
        at(was)
    ))
}

/// The owner's calendar on the fake server, each appointment costing the server 15 ms to give (the
/// owner's 239 took 3 to 6 s).
fn owner(streaming: bool) -> (Data, String, String) {
    let mut d = Data { streaming, pim_item_cost: Some(Duration::from_millis(15)), ..Default::default() };
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let cal = d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    for n in 0..SINGLES {
        d.pim.push((format!("E{n}"), cal.clone(), single(n)));
    }
    for n in 0..SERIES {
        d.pim.push((format!("S{n}"), cal.clone(), series(n)));
    }
    for n in 0..MOVED {
        d.pim.push((format!("S{n}:occ3"), cal.clone(), moved(n, &format!("Series {n:02}, later"))));
    }
    (d, inbox, cal)
}

/// What the window keeps once a sync's result is in, as the engine merges it: in a calendar read,
/// those the sync says are unchanged (`kept`) and those it gave; the calendars' list as it gave it.
fn kept_after(pim: &Pim, sync: &DavSync) -> Pim {
    let mut events: Vec<_> =
        pim.events.iter().filter(|e| !sync.synced_calendars.contains(&e.calendar) || sync.kept.contains(&e.href)).cloned().collect();
    events.extend(sync.events.iter().cloned());
    let mut calendars = pim.calendars.clone();
    for x in sync.calendars.iter().flatten() {
        match calendars.iter_mut().find(|k| k.id == x.id) {
            Some(k) => *k = x.clone(),
            None => calendars.push(x.clone()),
        }
    }
    let mut cards: Vec<_> = pim.cards.iter().filter(|k| !sync.synced_books.contains(&k.book) || sync.kept.contains(&k.href)).cloned().collect();
    cards.extend(sync.cards.iter().cloned());
    let mut books = pim.books.clone();
    for x in sync.books.iter().flatten() {
        match books.iter_mut().find(|k| k.id == x.id) {
            Some(k) => *k = x.clone(),
            None => books.push(x.clone()),
        }
    }
    Pim { calendars, events, books, cards, ..Default::default() }
}

/// The calendar as kept, to compare: each appointment's item, place in its series, subject and
/// times, in order.
fn calendar(pim: &Pim) -> Vec<(String, Option<i64>, String, i64, i64)> {
    let mut list: Vec<_> = pim
        .events
        .iter()
        .filter(|e| e.calendar == crate::ews_pim::CALENDAR)
        .map(|e| (e.href.clone(), e.event.recurrence_id, e.event.summary.clone(), e.event.start, e.event.end))
        .collect();
    list.sort();
    list
}

/// What a sync cost the server: appointments given, requests, and how long it took.
fn cost(server: &super::super::server::Server, f: impl FnOnce() -> DavSync) -> (DavSync, usize, usize, f32) {
    let (given, calls) = {
        let d = server.data.lock().unwrap();
        (d.pim_given, d.calls.len())
    };
    let began = Instant::now();
    let out = f();
    let took = began.elapsed().as_secs_f32();
    let d = server.data.lock().unwrap();
    (out, d.pim_given - given, d.calls.len() - calls, took)
}

/// The sync the worker runs when the server tells of a change in the calendar.
fn told(c: &crate::ews::Client, pim: &Pim, now: i64) -> DavSync {
    crate::ews_pim::sync_changes(c, pim, now).unwrap()
}

/// One appointment changed in Outlook, then one occurrence of a series moved: the next sync reads
/// what changed, and the calendar kept here is the one a whole read gives.
#[test]
fn one_appointment_changed_is_read_alone() {
    let (d, _, _) = owner(false);
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let (first, given, requests, took) = cost(&server, || told(&c, &Pim::default(), NOW));
    eprintln!("R21 probe: first sync: {given} appointments given, {requests} requests, {took:.2} s");
    let pim = kept_after(&Pim::default(), &first);
    assert_eq!(calendar(&pim).len(), SINGLES + SERIES + MOVED, "every appointment and occurrence moved");
    // A meeting moved in Outlook.
    {
        let mut d = server.data.lock().unwrap();
        let it = d.pim.iter_mut().find(|(id, _, _)| id == "E200").map(|(_, _, el)| el).unwrap();
        it.kids.retain(|k| k.name != "Subject");
        it.kids.push(element("<t:Subject>Meeting 200, moved</t:Subject>"));
    }
    let (next, given, requests, took) = cost(&server, || told(&c, &pim, NOW + 60));
    eprintln!("R21 probe: one appointment changed: {given} appointments given, {requests} requests, {took:.2} s");
    assert_eq!(given, 1, "the one changed, alone");
    let pim = kept_after(&pim, &next);
    let whole = kept_after(&Pim::default(), &told(&c, &Pim::default(), NOW + 60));
    assert_eq!(calendar(&pim), calendar(&whole), "as a whole read has it");
    assert!(calendar(&pim).iter().any(|(href, _, s, _, _)| href == "E200" && s == "Meeting 200, moved"));
    // One occurrence of a series moved again.
    {
        let mut d = server.data.lock().unwrap();
        *d.pim.iter_mut().find(|(id, _, _)| id == "S2:occ3").map(|(_, _, el)| el).unwrap() = moved(2, "Series 02, later again");
    }
    let (next, given, requests, took) = cost(&server, || told(&c, &pim, NOW + 120));
    eprintln!("R21 probe: one occurrence moved: {given} appointments given, {requests} requests, {took:.2} s");
    assert_eq!(given, 2, "the series and its occurrence moved");
    let pim = kept_after(&pim, &next);
    let whole = kept_after(&Pim::default(), &told(&c, &Pim::default(), NOW + 120));
    assert_eq!(calendar(&pim), calendar(&whole), "as a whole read has it");
    assert!(calendar(&pim).iter().any(|(href, r, s, _, _)| href == "S2:occ3" && r.is_some() && s == "Series 02, later again"));
}

/// New mail coming while the calendar is read, after the server told of a change in it: how long
/// after it came it's in the Inbox here.
#[test]
fn new_mail_during_the_calendars_sync_is_seen_soon() {
    let (d, inbox, cal) = owner(true);
    let server = serve(d);
    let store = crate::fake::tests_store("ews-calendar-changes-mail");
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "riley@contoso.example".into(),
        username: format!("{DOMAIN}\\{USER}"),
        ews: Some(server.url.clone()),
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret(PASSWORD.into())) });
    let seen = until(&pool, "the full sync's read of the calendar", |e| matches!(e, Event::DavSynced { .. }));
    // Kept as the window keeps it.
    let Some(Event::DavSynced { sync, .. }) = seen.last() else { panic!("the sync's result: {seen:?}") };
    store.save_account_doc("a1", PIM_DOC, &kept_after(&Pim::default(), sync)).unwrap();
    eventually("listening", || server.data.lock().unwrap().calls.iter().any(|c| c == "GetStreamingEvents"));
    std::thread::sleep(Duration::from_millis(600));
    // A meeting arrives in the calendar; mail comes once its read began.
    let given = server.data.lock().unwrap().pim_given;
    let start = at(NOW + 86_400);
    server.data.lock().unwrap().pim.push((
        "N1".into(),
        cal,
        element(&format!(
            "<t:CalendarItem><t:Subject>Budget review</t:Subject><t:Start>{start}</t:Start><t:End>{start}</t:End>\
             <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>budget-1</t:UID></t:CalendarItem>"
        )),
    ));
    eventually("the calendar's read under way", || server.data.lock().unwrap().pim_given > given);
    let came = Instant::now();
    server.data.lock().unwrap().deliver(&inbox, &mail("During the calendar", "n1"), false, "2026-10-24T08:00:00Z");
    let has = || store.load_folder("a1", "INBOX").is_ok_and(|f| f.messages.iter().any(|m| m.subject == "During the calendar"));
    eventually("the new mail", has);
    let mail_after = came.elapsed().as_secs_f32();
    let met =
        until(&pool, "the meeting", |e| matches!(e, Event::DavSynced { sync, .. } if sync.events.iter().any(|x| x.event.summary == "Budget review")));
    let read_after = came.elapsed().as_secs_f32();
    let read = met.iter().rev().find_map(|e| if let Event::DavSynced { sync, .. } = e { Some(sync.events.len()) } else { None }).unwrap_or(0);
    eprintln!(
        "R21 probe: new mail seen {mail_after:.2} s after it came; the calendar's sync done {read_after:.2} s after ({read} appointments given)"
    );
    assert_eq!(read, 1, "the new meeting alone");
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

/// The owner's calendar on the fake server, read once whole: the server, a connection, and what
/// the window keeps.
fn read_once() -> (super::super::server::Server, crate::ews::Client, Pim) {
    let (mut d, _, _) = owner(false);
    d.pim_item_cost = None;
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let first = told(&c, &Pim::default(), NOW);
    let pim = kept_after(&Pim::default(), &first);
    (server, c, pim)
}

/// The calendar as a whole read has it now.
fn whole(c: &crate::ews::Client) -> Vec<(String, Option<i64>, String, i64, i64)> {
    calendar(&kept_after(&Pim::default(), &told(c, &Pim::default(), NOW + 600)))
}

/// A state the server no longer takes (lost, too old) has the calendar read whole once, then by its
/// changes again; one never kept (a profile from before) too.
#[test]
fn a_refused_state_reads_the_calendar_whole_once() {
    let (server, c, mut pim) = read_once();
    let state = pim.calendars.iter().find(|k| k.id == crate::ews_pim::CALENDAR).map(|k| k.ctag.clone()).unwrap();
    assert!(!state.is_empty(), "a state kept with the calendar");
    for junk in ["p1!!not-one", ""] {
        pim.calendars.iter_mut().filter(|k| k.id == crate::ews_pim::CALENDAR).for_each(|k| k.ctag = junk.into());
        let (next, given, _, _) = cost(&server, || told(&c, &pim, NOW + 60));
        assert_eq!(given, SINGLES + SERIES + MOVED, "read whole ({junk:?})");
        assert!(next.kept.is_empty());
        pim = kept_after(&pim, &next);
        assert_eq!(calendar(&pim), whole(&c));
        let (_, given, _, _) = cost(&server, || told(&c, &pim, NOW + 120));
        assert_eq!(given, 0, "then by its changes: none");
    }
}

/// Deleted in Outlook: a meeting, and a series with its occurrence moved, go here too; a meeting
/// made in Outlook comes; nothing is there twice.
#[test]
fn what_is_deleted_or_made_on_the_server_follows() {
    let (server, c, pim) = read_once();
    {
        let mut d = server.data.lock().unwrap();
        d.pim.retain(|(id, _, _)| id != "E10" && id != "S1" && id != "S1:occ3");
        let cal = d.folders.iter().find(|f| f.name == "Calendar").map(|f| f.id.clone()).unwrap();
        d.pim.push(("N1".into(), cal, single(300)));
    }
    let next = told(&c, &pim, NOW + 60);
    let pim = kept_after(&pim, &next);
    let now = calendar(&pim);
    assert_eq!(now, whole(&c));
    assert!(!now.iter().any(|(href, ..)| href == "E10" || href == "S1" || href.starts_with("S1:")), "gone, the series' occurrence with it");
    assert_eq!(now.iter().filter(|(href, ..)| href == "N1").count(), 1, "the new one, once");
}

/// Changed here and deleted on the server meanwhile: the server wins (the change is let go, and
/// the engine keeps a copy in Sync Issues › Conflicts), as when the calendar was read whole.
#[test]
fn changed_here_and_deleted_there_the_server_wins() {
    let (server, c, mut pim) = read_once();
    let e = pim.events.iter_mut().find(|e| e.href == "E20").unwrap();
    e.event.summary = "Changed here".into();
    e.dirty = true;
    server.data.lock().unwrap().pim.retain(|(id, _, _)| id != "E20");
    let next = told(&c, &pim, NOW + 60);
    assert_eq!(next.gone_events, ["E20"], "said gone, for the engine to let it go");
    assert!(!next.kept.iter().any(|h| h == "E20") && !next.events.iter().any(|e| e.href == "E20"));
}

/// An appointment made here goes up, and the calendar read by its changes then has it under its
/// item there, once.
#[test]
fn an_appointment_made_here_is_kept_once() {
    use quatrelle_model::ical::CalEvent;
    use quatrelle_model::pim::StoredEvent;
    let (_server, c, mut pim) = read_once();
    pim.events.push(StoredEvent {
        calendar: crate::ews_pim::CALENDAR.into(),
        event: CalEvent {
            uid: "dentist-1@quatrelle".into(),
            summary: "Dentist".into(),
            start: NOW + 86_400,
            end: NOW + 88_200,
            ..Default::default()
        },
        dirty: true,
        ..Default::default()
    });
    let next = told(&c, &pim, NOW + 60);
    let [made] = next.pushed_events.as_slice() else { panic!("made once: {:?}", next.pushed_events) };
    let read: Vec<&str> = next.events.iter().map(|e| e.href.as_str()).collect();
    assert_eq!(read, [made.href.as_str()], "read back by its item, alone");
}

/// A deletion the server refused: the appointment stays there, so it comes back here.
#[test]
fn a_deletion_the_server_refused_comes_back() {
    use quatrelle_model::pim::Tombstone;
    let (server, c, mut pim) = read_once();
    pim.events.retain(|e| e.href != "E30");
    pim.deleted.push(Tombstone { href: "E30".into(), etag: String::new(), calendar: crate::ews_pim::CALENDAR.into() });
    server.data.lock().unwrap().undeletable.push("E30".into());
    let next = told(&c, &pim, NOW + 60);
    assert!(next.deleted.iter().any(|h| h == "E30"), "no longer waiting to be deleted");
    assert!(next.errors.iter().any(|e| e.starts_with("not deleted on the server")), "{:?}", next.errors);
    assert!(next.events.iter().any(|e| e.href == "E30"), "read again: back here");
}

/// The contacts follow the same way: one changed in Outlook is read alone, one deleted goes.
#[test]
fn the_contacts_are_read_by_their_changes() {
    let contact = |name: &str| element(&format!("<t:Contact><t:DisplayName>{name}</t:DisplayName><t:GivenName>{name}</t:GivenName></t:Contact>"));
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    let book = d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    for n in 0..30 {
        d.pim.push((format!("C{n}"), book.clone(), contact(&format!("Person {n:02}"))));
    }
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let first = told(&c, &Pim::default(), NOW);
    assert_eq!(first.cards.len(), 30);
    let pim = kept_after(&Pim::default(), &first);
    {
        let mut d = server.data.lock().unwrap();
        *d.pim.iter_mut().find(|(id, _, _)| id == "C3").map(|(_, _, el)| el).unwrap() = contact("Person 03, moved");
        d.pim.retain(|(id, _, _)| id != "C4");
    }
    let (next, given, _, _) = cost(&server, || told(&c, &pim, NOW + 60));
    assert_eq!(given, 1, "the one changed, alone");
    let pim = kept_after(&pim, &next);
    let mut names: Vec<String> = pim.cards.iter().map(|k| k.card.display()).collect();
    names.sort();
    assert_eq!(names.len(), 29);
    assert!(names.iter().any(|n| n == "Person 03, moved") && !names.iter().any(|n| n == "Person 04"));
}
