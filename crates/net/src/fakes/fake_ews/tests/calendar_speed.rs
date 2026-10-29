//! How fast a calendar ticked shows: a big one read over several connections, each of those
//! ticked together shown as soon as it's read, and a shared mailbox without a calendar asked
//! about once (29/10: a colleague's calendar ticked showed a quarter of a minute later).

use super::*;

fn calls(server: &super::super::server::Server, op: &str) -> usize {
    server.data.lock().unwrap().calls.iter().filter(|c| c.as_str() == op).count()
}

fn appointment(subject: &str, day: u32, hour: u32) -> El {
    parse(&format!(
        "<t:CalendarItem><t:Subject>{subject}</t:Subject><t:Start>2026-10-{day:02}T{hour:02}:00:00Z</t:Start>\
         <t:End>2026-10-{day:02}T{hour:02}:30:00Z</t:End><t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>{subject}</t:UID></t:CalendarItem>"
    ))
    .kids
    .remove(0)
}

/// A calendar of 230 appointments is asked over four connections at once, in eight batches alike
/// (two each), and comes whole, in the server's order.
#[test]
fn a_big_calendar_is_read_over_several_connections() {
    use quatrelle_model::pim::Pim;
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let cal = d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let names: Vec<String> = (0..230).map(|i| format!("Meeting {i:03}")).collect();
    for (i, name) in names.iter().enumerate() {
        let i = u32::try_from(i).unwrap();
        d.pim.push((format!("E{i}"), cal.clone(), appointment(name, 1 + i % 28, 8 + i / 28)));
    }
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    // A second read, what a session asks once (Autodiscover, on a connection of its own) asked.
    crate::ews_pim::sync_calendars(&c, &Pim::default(), &[], 1_792_000_000).unwrap();
    let handshakes = server.data.lock().unwrap().handshakes;
    let items = calls(&server, "GetItem");
    let sync = crate::ews_pim::sync_calendars(&c, &Pim::default(), &[], 1_792_000_000).unwrap();
    let read: Vec<&str> = sync.events.iter().filter(|e| e.calendar == crate::ews_pim::CALENDAR).map(|e| e.event.summary.as_str()).collect();
    assert_eq!(read, names, "whole, in order");
    assert_eq!(calls(&server, "GetItem") - items, 8, "eight batches, a share of two for each connection");
    // Each connection takes the next batch: one started late may find none left.
    let more = server.data.lock().unwrap().handshakes - handshakes;
    assert!((1..=3).contains(&more), "more connections, each signed in: {more}");
}

/// Calendars ticked one after the other: each is shown as soon as it's read, not when all are,
/// and the second doesn't wait for the first to be read (one ticked as the one before was being
/// read showed 5.7 s later, where alone it took 3, 29/10); two at a time, the third waits.
#[test]
fn each_calendar_ticked_shows_as_soon_as_it_is_read() {
    use quatrelle_model::pim::shared_calendar_id;
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    d.pim.push(("S1".into(), "mailbox:ana@contoso.example".into(), appointment("Budget review", 6, 13)));
    d.pim.push(("S2".into(), "mailbox:tom@contoso.example".into(), appointment("Site visit", 7, 9)));
    d.pim.push(("S3".into(), "mailbox:lee@contoso.example".into(), appointment("Board", 8, 10)));
    d.slow_items = Some(std::time::Duration::from_millis(600));
    let server = serve(d);
    let store = crate::fake::tests_store("ews-calendars-each");
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
    let left = ["ana", "tom", "lee"].map(|n| shared_calendar_id(&format!("{n}@contoso.example")));
    for id in &left {
        pool.submit(Job::SyncCalendars { account: "a1".into(), calendars: vec![id.clone()] });
    }
    let alone = |e: &Event, id: &str| matches!(e, Event::DavSynced { sync, .. } if sync.partial && sync.synced_calendars == [id] && sync.events.iter().all(|x| x.calendar == id));
    let mut left = left.to_vec();
    while !left.is_empty() {
        until(&pool, "each calendar alone", |e| {
            let was = left.len();
            left.retain(|id| !alone(e, id));
            left.len() < was
        });
    }
    assert_eq!(server.data.lock().unwrap().most_items_in_flight, 2, "read at the same time, two at most");
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

/// A shared mailbox the account opens whose calendar isn't there (three of the owner's, each
/// asked three ways at every read of the calendars) is asked about once a session.
#[test]
fn a_shared_mailbox_without_a_calendar_is_asked_about_once() {
    use quatrelle_model::pim::{CalendarInfo, Pim, shared_calendar_id};
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    d.pim.push(("S1".into(), "mailbox:ana@contoso.example".into(), appointment("Budget review", 6, 13)));
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    c.set_shared(&["frontdesk@contoso.example".into()]);
    let ana = shared_calendar_id("ana@contoso.example");
    let info = |id: &str| CalendarInfo { id: id.into(), name: id.into(), read_only: true, ..Default::default() };
    let pim = Pim { shared: vec!["ana@contoso.example".into()], calendars: vec![info(crate::ews_pim::CALENDAR), info(&ana)], ..Default::default() };
    let read = || {
        let before = calls(&server, "FindItem");
        let sync = crate::ews_pim::sync_calendars(&c, &pim, &[], 1_792_000_000).unwrap();
        assert!(sync.events.iter().any(|e| e.calendar == ana), "Ana's still read");
        assert!(!sync.calendars.iter().flatten().any(|x| x.id == shared_calendar_id("frontdesk@contoso.example")));
        calls(&server, "FindItem") - before
    };
    let first = read();
    assert_eq!(first - read(), 3, "its calendar asked for three ways the first time, not again");
}

/// A series' occurrences changed on their own are read from six months back, as the rest of the
/// calendar: a colleague's meetings moved now and then for years were hundreds, read one batch
/// after another once the calendar was (2.3 s of 6, 29/10).
#[test]
fn only_the_occurrences_changed_lately_are_read() {
    use quatrelle_model::pim::Pim;
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let cal = d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let weekly = parse(
        "<t:CalendarItem><t:Subject>Weekly</t:Subject><t:Start>2019-01-07T09:00:00Z</t:Start><t:End>2019-01-07T09:30:00Z</t:End>\
         <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>weekly</t:UID><t:CalendarItemType>RecurringMaster</t:CalendarItemType>\
         <t:Recurrence><t:WeeklyRecurrence><t:Interval>1</t:Interval><t:DaysOfWeek>Monday</t:DaysOfWeek></t:WeeklyRecurrence>\
         <t:NoEndRecurrence><t:StartDate>2019-01-07</t:StartDate></t:NoEndRecurrence></t:Recurrence></t:CalendarItem>",
    );
    d.pim.push(("M1".into(), cal.clone(), weekly.kids[0].clone()));
    let moved = |day: &str| {
        parse(&format!(
            "<t:CalendarItem><t:Subject>Weekly</t:Subject><t:Start>{day}T10:00:00Z</t:Start><t:End>{day}T10:30:00Z</t:End>\
             <t:OriginalStart>{day}T09:00:00Z</t:OriginalStart><t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>weekly</t:UID>\
             <t:CalendarItemType>Exception</t:CalendarItemType></t:CalendarItem>"
        ))
        .kids
        .remove(0)
    };
    d.pim.push(("M1:occ2".into(), cal.clone(), moved("2019-01-14")));
    d.pim.push(("M1:occ406".into(), cal, moved("2026-10-12")));
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let sync = crate::ews_pim::sync_calendars(&c, &Pim::default(), &[], 1_792_000_000).unwrap();
    let moved: Vec<&str> = sync.events.iter().filter(|e| e.event.recurrence_id.is_some()).map(|e| e.href.as_str()).collect();
    assert_eq!(moved, ["M1:occ406"], "the one of 2019 isn't read");
    assert!(sync.events.iter().any(|e| e.href == "M1" && e.event.recurrence_id.is_none()), "the series is");
}

/// An occurrence changed on its own the server refuses (it refused a colleague's 41, though
/// their series came, 29/10) shows from what its series says of it: on its new day, not its old.
#[test]
fn an_occurrence_the_server_refuses_shows_from_its_series() {
    use quatrelle_model::pim::Pim;
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let cal = d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let weekly = parse(
        "<t:CalendarItem><t:Subject>Weekly</t:Subject><t:Start>2026-09-07T09:00:00Z</t:Start><t:End>2026-09-07T09:30:00Z</t:End>\
         <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>weekly</t:UID><t:CalendarItemType>RecurringMaster</t:CalendarItemType>\
         <t:Recurrence><t:WeeklyRecurrence><t:Interval>1</t:Interval><t:DaysOfWeek>Monday</t:DaysOfWeek></t:WeeklyRecurrence>\
         <t:NoEndRecurrence><t:StartDate>2026-09-07</t:StartDate></t:NoEndRecurrence></t:Recurrence></t:CalendarItem>",
    );
    d.pim.push(("M1".into(), cal.clone(), weekly.kids[0].clone()));
    let moved = |day: &str, subject: &str| {
        parse(&format!(
            "<t:CalendarItem><t:Subject>{subject}</t:Subject><t:Start>{day}T14:00:00Z</t:Start><t:End>{day}T15:00:00Z</t:End>\
             <t:OriginalStart>{day}T09:00:00Z</t:OriginalStart><t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>weekly</t:UID>\
             <t:CalendarItemType>Exception</t:CalendarItemType></t:CalendarItem>"
        ))
        .kids
        .remove(0)
    };
    d.pim.push(("M1:occ5".into(), cal.clone(), moved("2026-10-05", "Weekly, later")));
    d.pim.push(("M1:occ6".into(), cal, moved("2026-10-12", "Weekly, later")));
    d.refused.push("M1:occ6".into());
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let sync = crate::ews_pim::sync_calendars(&c, &Pim::default(), &[], 1_792_000_000).unwrap();
    let at = |t: &str| quatrelle_jmap::utc_date(t).unwrap();
    let instance = |href: &str| sync.events.iter().find(|e| e.href == href).map(|e| &e.event).unwrap();
    let given = instance("M1:occ5");
    assert_eq!((given.summary.as_str(), given.start), ("Weekly, later", at("2026-10-05T14:00:00Z")), "given: as the server has it");
    let refused = instance("M1:occ6");
    assert_eq!(refused.recurrence_id, Some(at("2026-10-12T09:00:00Z")), "in its place in the series");
    assert_eq!((refused.start, refused.end), (at("2026-10-12T14:00:00Z"), at("2026-10-12T15:00:00Z")), "at its own times");
    assert_eq!((refused.summary.as_str(), refused.rrule.as_deref()), ("Weekly", None), "under the series' subject, once");
    assert_eq!(sync.events.iter().filter(|e| e.event.recurrence_id.is_some()).count(), 2, "each once");
}

/// A calendar's appointments, and a series' occurrences changed on their own, are asked for by
/// the fields read of them, not all their properties (a colleague's 870 appointments cost the
/// server 10 s, 29/10), and come as whole as before.
#[test]
fn an_appointment_is_asked_for_by_the_fields_read_of_it() {
    use quatrelle_model::pim::Pim;
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let cal = d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    for i in 0..120u32 {
        d.pim.push((format!("E{i}"), cal.clone(), appointment(&format!("Meeting {i:03}"), 1 + i % 28, 8 + i / 28)));
    }
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let sync = crate::ews_pim::sync_calendars(&c, &Pim::default(), &[], 1_792_000_000).unwrap();
    assert_eq!(sync.events.iter().filter(|e| e.calendar == crate::ews_pim::CALENDAR).count(), 120, "every one");
    let shapes = server.data.lock().unwrap().item_shapes.clone();
    assert!(!shapes.is_empty() && shapes.iter().all(|s| s == "fields"), "by their fields: {shapes:?}");
}

/// A server whose schema doesn't know a field named (it refuses the whole request, or each item
/// of it) is asked for all the properties instead, and so for the rest of the session: the
/// calendar comes whole.
#[test]
fn a_server_refusing_the_fields_named_is_asked_for_everything() {
    refusing_the_fields(false);
    refusing_the_fields(true);
}

fn refusing_the_fields(each: bool) {
    use quatrelle_model::pim::Pim;
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let cal = d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    for i in 0..230u32 {
        d.pim.push((format!("E{i}"), cal.clone(), appointment(&format!("Meeting {i:03}"), 1 + i % 28, 8 + i / 28)));
    }
    (d.refuse_fields, d.refuse_fields_each) = (!each, each);
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let read = |c: &crate::ews::Client| {
        let sync = crate::ews_pim::sync_calendars(c, &Pim::default(), &[], 1_792_000_000).unwrap();
        sync.events.iter().filter(|e| e.calendar == crate::ews_pim::CALENDAR).count()
    };
    assert_eq!(read(&c), 230, "whole, all their properties asked");
    let first = std::mem::take(&mut server.data.lock().unwrap().item_shapes);
    assert!(first.iter().any(|s| s == "fields") && first.iter().any(|s| s == "AllProperties"), "{first:?}");
    // Each connection asks once at most before it knows.
    assert!(first.iter().filter(|s| *s == "fields").count() <= 4, "{first:?}");
    assert_eq!(read(&c), 230, "whole again");
    let again = server.data.lock().unwrap().item_shapes.clone();
    assert!(again.iter().all(|s| s == "AllProperties"), "not asked by their fields again: {again:?}");
}
