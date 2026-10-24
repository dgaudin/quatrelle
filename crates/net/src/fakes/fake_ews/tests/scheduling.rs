//! Tests over the fake EWS server: the rooms it offers and when people and rooms are busy, for
//! a meeting being written.

use quatrelle_model::scheduling::BusyKind;

use super::*;

fn client(server: &super::server::Server) -> crate::ews::Client {
    crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap()
}

fn appointment(subject: &str, start: &str, end: &str) -> El {
    parse(&format!(
        "<t:CalendarItem><t:Subject>{subject}</t:Subject><t:Start>{start}</t:Start><t:End>{end}</t:End>\
         <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>{subject}</t:UID></t:CalendarItem>"
    ))
    .kids
    .remove(0)
}

/// Room lists and their rooms come as Outlook's Room Finder lists them; a server with none set
/// up (most on-premises ones) answers with none, which isn't a failure; one without the
/// operation refuses.
#[test]
fn rooms_come_from_the_lists_the_server_has() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.room_lists = vec![("Lyon".into(), "rooms.lyon@contoso.example".into()), ("Paris".into(), "rooms.paris@contoso.example".into())];
    d.rooms = vec![
        ("Salle Rhône".into(), "Room.Rhone@contoso.example".into(), "rooms.lyon@contoso.example".into()),
        ("Salle Saône".into(), "room.saone@contoso.example".into(), "rooms.lyon@contoso.example".into()),
        ("Salle Seine".into(), "room.seine@contoso.example".into(), "rooms.paris@contoso.example".into()),
    ];
    let server = serve(d);
    let (lists, rooms) = crate::ews_pim::scheduling::rooms(&client(&server)).unwrap();
    assert_eq!(lists.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["Lyon", "Paris"]);
    assert_eq!(
        rooms.iter().map(|r| (r.name.as_str(), r.list.as_str())).collect::<Vec<_>>(),
        [("Salle Rhône", "Lyon"), ("Salle Saône", "Lyon"), ("Salle Seine", "Paris")]
    );
    assert_eq!(rooms[0].email, "room.rhone@contoso.example", "addresses in lower case");
    assert_eq!(server.data.lock().unwrap().calls.iter().filter(|c| c.as_str() == "GetRooms").count(), 2, "one GetRooms per list");
    // No lists set up.
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let server = serve(d);
    let (lists, rooms) = crate::ews_pim::scheduling::rooms(&client(&server)).unwrap();
    assert!(lists.is_empty() && rooms.is_empty());
    // The operation refused.
    let mut d = Data { rooms_refused: true, ..Default::default() };
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let server = serve(d);
    assert!(crate::ews_pim::scheduling::rooms(&client(&server)).is_err());
}

/// Free/busy for the organizer, a colleague and a room: each one's calendar between the dates,
/// in UTC; a mailbox the account may not read says so on its own; a window shorter than a day
/// is asked for as a day (Exchange answers for nothing less).
#[test]
fn availability_is_read_for_people_and_rooms() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let cal = d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.pim.push(("P1".into(), cal.clone(), appointment("Budget", "2026-10-22T09:00:00Z", "2026-10-22T10:00:00Z")));
    d.pim.push(("P2".into(), cal, appointment("Last week", "2026-10-15T09:00:00Z", "2026-10-15T10:00:00Z")));
    d.pim.push(("R1".into(), "mailbox:room.rhone@contoso.example".into(), appointment("Dailies", "2026-10-22T13:00:00Z", "2026-10-22T14:00:00Z")));
    d.taken.push(("ana@contoso.example".into(), "2026-10-22T11:00:00Z".into(), "2026-10-22T12:00:00Z".into(), "Tentative".into(), "Lunch".into()));
    d.no_freebusy = vec!["secret@contoso.example".into()];
    let server = serve(d);
    let c = client(&server);
    let from = quatrelle_jmap::utc_date("2026-10-22T00:00:00Z").unwrap();
    let emails =
        ["riley@contoso.example".to_string(), "ana@contoso.example".into(), "room.rhone@contoso.example".into(), "secret@contoso.example".into()];
    let out = crate::ews_pim::scheduling::availability(&c, &emails, from, from + 3600).unwrap();
    assert_eq!(out.len(), 4);
    let slots = |i: usize| out[i].slots.iter().map(|s| (s.start - from, s.end - from, s.kind, s.subject.clone())).collect::<Vec<_>>();
    assert_eq!(slots(0), [(9 * 3600, 10 * 3600, BusyKind::Busy, "Budget".to_string())], "this day's only");
    assert_eq!(slots(1), [(11 * 3600, 12 * 3600, BusyKind::Tentative, "Lunch".to_string())]);
    assert_eq!(slots(2), [(13 * 3600, 14 * 3600, BusyKind::Busy, "Dailies".to_string())]);
    assert!(out[3].error.as_deref().is_some_and(|e| e.contains("free/busy")), "{:?}", out[3]);
    let asked = server.data.lock().unwrap().availability_asked.clone();
    assert_eq!(asked.len(), 1);
    assert_eq!((asked[0].1.as_str(), asked[0].2.as_str()), ("2026-10-22T00:00:00", "2026-10-23T00:00:00"), "a day at least, UTC without a zone");
    assert_eq!(asked[0].0.len(), 4);
}

/// Through the pool: rooms and free/busy are answered on the account's second connection
/// while the first is still held by a slow mailbox sync, so the form never waits on it;
/// nothing to do with mail is touched.
#[test]
fn rooms_and_availability_are_answered_while_the_mailbox_syncs() {
    let mut d = Data { slow_sync: Some(Duration::from_secs(6)), ..Default::default() };
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.deliver(&inbox, &mail("Hello", "m1"), false, "2026-10-20T09:00:00Z");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.room_lists = vec![("Lyon".into(), "rooms.lyon@contoso.example".into())];
    d.rooms = vec![("Salle Rhône".into(), "room.rhone@contoso.example".into(), "rooms.lyon@contoso.example".into())];
    d.taken.push((
        "room.rhone@contoso.example".into(),
        "2026-10-22T13:00:00Z".into(),
        "2026-10-22T14:00:00Z".into(),
        "Busy".into(),
        "Dailies".into(),
    ));
    let server = serve(d);
    let store = crate::fake::tests_store("ews-scheduling");
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "riley@contoso.example".into(),
        username: format!("{DOMAIN}\\{USER}"),
        ews: Some(server.url.clone()),
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret(PASSWORD.into())) });
    until(&pool, "connecting", |e| matches!(e, Event::Status { status: SyncStatus::Syncing { .. } | SyncStatus::Connecting, .. }));
    let began = Instant::now();
    pool.submit(Job::GetRooms { account: "a1".into() });
    let from = quatrelle_jmap::utc_date("2026-10-22T00:00:00Z").unwrap();
    pool.submit(Job::GetAvailability {
        account: "a1".into(),
        emails: vec!["riley@contoso.example".into(), "room.rhone@contoso.example".into()],
        from,
        to: from + 86_400,
    });
    let seen = until(&pool, "rooms", |e| matches!(e, Event::Rooms { .. }));
    match seen.last() {
        Some(Event::Rooms { lists, rooms, error, .. }) => {
            assert_eq!((lists.len(), rooms.len(), error.as_deref()), (1, 1, None));
            assert_eq!(rooms[0].name, "Salle Rhône");
        }
        other => panic!("{other:?}"),
    }
    let seen = until(&pool, "availability", |e| matches!(e, Event::Availability { .. }));
    match seen.last() {
        Some(Event::Availability { people, error, from: f, to, .. }) => {
            assert_eq!((people.len(), error.as_deref(), *f, *to), (2, None, from, from + 86_400));
            assert_eq!(people[1].slots.len(), 1);
            assert_eq!(people[1].slots[0].subject, "Dailies");
        }
        other => panic!("{other:?}"),
    }
    assert!(began.elapsed() < Duration::from_secs(6), "answered on the second connection, not after the sync");
    drop(pool);
}

/// A meeting made with a room carries it as a resource the server books (`Resources`, after the
/// guests, as the schema orders them), and the room reads back as a resource, not a guest; the
/// room taken off a meeting is taken off the item.
#[test]
fn a_meeting_made_with_a_room_books_it_as_a_resource() {
    use quatrelle_model::ical::{Attendee, CalEvent};
    use quatrelle_model::pim::{Pim, StoredEvent};
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let server = serve(d);
    let c = client(&server);
    let who = |name: &str, email: &str, cutype: &str| Attendee {
        name: name.into(),
        email: email.into(),
        partstat: "NEEDS-ACTION".into(),
        rsvp: true,
        cutype: cutype.into(),
        role: String::new(),
    };
    let mut pim = Pim::default();
    pim.events.push(StoredEvent {
        calendar: crate::ews_pim::CALENDAR.into(),
        event: CalEvent {
            uid: "new-1".into(),
            summary: "Budget".into(),
            location: "Salle Rhône".into(),
            start: 1_792_000_000,
            end: 1_792_003_600,
            organizer: Some(who("Riley", "riley@contoso.example", "")),
            attendees: vec![who("Ana Lima", "ana@contoso.example", ""), who("Salle Rhône", "room.rhone@contoso.example", "ROOM")],
            ..Default::default()
        },
        dirty: true,
        notify: true,
        ..Default::default()
    });
    let made = crate::ews_pim::sync(&c, &pim, 1_792_000_300).unwrap();
    assert!(made.errors.is_empty(), "{:?}", made.errors);
    {
        let d = server.data.lock().unwrap();
        let (_, _, item) = d.pim.iter().find(|(_, _, el)| el.text_of("Subject") == Some("Budget")).expect("made");
        let names: Vec<&str> = item.kids.iter().map(|k| k.name.as_str()).collect();
        let at = |n: &str| names.iter().position(|x| *x == n).unwrap_or_else(|| panic!("no {n} in {names:?}"));
        assert!(at("Location") < at("RequiredAttendees") && at("RequiredAttendees") < at("Resources"), "{names:?}");
        let guests: Vec<&str> =
            item.child("RequiredAttendees").unwrap().children("Attendee").filter_map(|a| a.find("EmailAddress")).map(|e| e.text.as_str()).collect();
        let rooms: Vec<&str> =
            item.child("Resources").unwrap().children("Attendee").filter_map(|a| a.find("EmailAddress")).map(|e| e.text.as_str()).collect();
        assert_eq!((guests, rooms), (vec!["ana@contoso.example"], vec!["room.rhone@contoso.example"]));
    }
    let budget = made.events.iter().find(|e| e.event.summary == "Budget").unwrap();
    let kinds: Vec<(&str, &str)> = budget.event.attendees.iter().map(|a| (a.email.as_str(), a.cutype.as_str())).collect();
    assert_eq!(kinds, [("ana@contoso.example", ""), ("room.rhone@contoso.example", "RESOURCE")]);
    // The room taken off: the item loses its resources.
    let mut again = Pim { events: made.events.clone(), ..Default::default() };
    for e in again.events.iter_mut().filter(|e| e.event.summary == "Budget") {
        e.event.attendees.retain(|a| !a.is_resource());
        e.dirty = true;
    }
    let second = crate::ews_pim::sync(&c, &again, 1_792_000_400).unwrap();
    assert!(second.errors.is_empty(), "{:?}", second.errors);
    let d = server.data.lock().unwrap();
    let (_, _, item) = d.pim.iter().find(|(_, _, el)| el.text_of("Subject") == Some("Budget")).unwrap();
    assert!(item.child("Resources").is_none() && item.child("RequiredAttendees").is_some());
}
