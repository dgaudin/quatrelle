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

/// A calendar of 230 appointments (five batches) is asked over four connections at once, and
/// comes whole, in the server's order.
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
    assert_eq!(calls(&server, "GetItem") - items, 5, "fifty at a time, as before");
    assert_eq!(server.data.lock().unwrap().handshakes - handshakes, 3, "three more connections, each signed in");
}

/// Two calendars ticked one after the other, read together: each is shown as soon as it's read,
/// not when both are.
#[test]
fn each_calendar_ticked_shows_as_soon_as_it_is_read() {
    use quatrelle_model::pim::shared_calendar_id;
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    d.pim.push(("S1".into(), "mailbox:ana@contoso.example".into(), appointment("Budget review", 6, 13)));
    d.pim.push(("S2".into(), "mailbox:tom@contoso.example".into(), appointment("Site visit", 7, 9)));
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
    let (ana, tom) = (shared_calendar_id("ana@contoso.example"), shared_calendar_id("tom@contoso.example"));
    pool.submit(Job::SyncCalendars { account: "a1".into(), calendars: vec![ana.clone(), tom.clone()] });
    let alone = |e: &Event, id: &str| matches!(e, Event::DavSynced { sync, .. } if sync.partial && sync.synced_calendars == [id] && sync.events.iter().all(|x| x.calendar == id));
    until(&pool, "Ana's calendar alone", |e| alone(e, &ana));
    until(&pool, "then Tom's", |e| alone(e, &tom));
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
