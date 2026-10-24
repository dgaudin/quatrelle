//! Calendars, contacts and notes over EWS: the account's own, other people's, and Outlook's calendar groups.

use super::*;

/// Three plain appointments carrying one UID: the sync hands the engine three distinct
/// identities, not one.
#[test]
fn meetings_reusing_one_uid_each_reach_the_calendar() {
    use quatrelle_model::pim::Pim;
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let cal = d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    for (id, day) in [("C1", "06"), ("C2", "13"), ("C3", "20")] {
        let it = parse(&format!(
            "<t:CalendarItem><t:Subject>Safety committee</t:Subject><t:Start>2026-10-{day}T08:00:00Z</t:Start><t:End>2026-10-{day}T10:00:00Z</t:End>\
             <t:IsAllDayEvent>false</t:IsAllDayEvent><t:CalendarItemType>Single</t:CalendarItemType><t:UID>committee-series</t:UID></t:CalendarItem>"
        ));
        d.pim.push((id.into(), cal.clone(), it.kids[0].clone()));
    }
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let sync = crate::ews_pim::sync(&c, &Pim::default(), 1_792_000_000).unwrap();
    // Distinct (UID, instance) pairs: what the engine stores separately.
    let ids: std::collections::HashSet<(String, Option<i64>)> =
        sync.events.iter().filter(|e| e.event.summary == "Safety committee").map(|e| (e.event.uid.clone(), e.event.recurrence_id)).collect();
    assert_eq!(ids.len(), 3, "{ids:?}");
}

#[test]
fn calendars_and_contacts_sync_and_autodiscover_finds_the_server() {
    use quatrelle_model::ical::CalEvent;
    use quatrelle_model::pim::{LOCAL, Pim, StoredCard, StoredEvent, Tombstone};
    use quatrelle_model::vcard::{Card, Typed};
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let cal = d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let standup = parse(
        "<t:CalendarItem><t:Subject>Stand-up</t:Subject><t:Body BodyType=\"Text\">Daily</t:Body><t:Start>2026-10-05T09:00:00Z</t:Start>\
         <t:End>2026-10-05T09:15:00Z</t:End><t:IsAllDayEvent>false</t:IsAllDayEvent><t:Location>Room 2</t:Location><t:UID>standup-1</t:UID>\
         <t:Recurrence><t:WeeklyRecurrence><t:Interval>1</t:Interval><t:DaysOfWeek>Monday Wednesday Friday</t:DaysOfWeek></t:WeeklyRecurrence>\
         <t:NoEndRecurrence><t:StartDate>2026-10-05Z</t:StartDate></t:NoEndRecurrence></t:Recurrence></t:CalendarItem>",
    );
    d.pim.push((
        "OLD".into(),
        cal.clone(),
        parse("<t:CalendarItem><t:Subject>Gone</t:Subject><t:Start>2026-10-01T09:00:00Z</t:Start></t:CalendarItem>").kids[0].clone(),
    ));
    d.pim.push(("E1".into(), cal.clone(), standup.kids[0].clone()));
    let server = serve(d);
    let user = format!("{DOMAIN}\\{USER}");
    assert_eq!(crate::ews::autodiscover("riley@contoso.example", &user, PASSWORD, Some(&server.url)).unwrap(), server.url);
    let c = crate::ews::Client::connect(&server.url, &user, PASSWORD, "riley@contoso.example").unwrap();
    let mut pim = Pim::default();
    pim.events.push(StoredEvent {
        calendar: LOCAL.into(),
        event: CalEvent {
            uid: "local-1".into(),
            summary: "Poster review".into(),
            start: 1_792_000_000,
            end: 1_792_003_600,
            rrule: Some("FREQ=DAILY;COUNT=3".into()),
            ..Default::default()
        },
        dirty: true,
        ..Default::default()
    });
    pim.cards.push(StoredCard {
        book: LOCAL.into(),
        card: Card {
            uid: "p1".into(),
            name: "Ben Ito".into(),
            emails: vec![Typed { value: "ben@contoso.example".into(), kind: String::new() }],
            ..Default::default()
        },
        dirty: true,
        ..Default::default()
    });
    pim.deleted.push(Tombstone { href: "OLD".into(), etag: String::new(), ..Default::default() });
    let sync = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    assert_eq!(sync.deleted, ["OLD"]);
    let standup = sync.events.iter().find(|e| e.event.summary == "Stand-up").unwrap();
    assert_eq!(standup.event.rrule.as_deref(), Some("FREQ=WEEKLY;BYDAY=MO,WE,FR"));
    assert_eq!((standup.event.uid.as_str(), standup.event.location.as_str(), standup.event.end - standup.event.start), ("standup-1", "Room 2", 900));
    let poster = sync.events.iter().find(|e| e.event.summary == "Poster review").unwrap();
    assert_eq!(poster.event.rrule.as_deref(), Some("FREQ=DAILY;COUNT=3"), "made on the server with its recurrence");
    assert!(sync.cards.iter().any(|k| k.card.name == "Ben Ito" && k.card.emails[0].value == "ben@contoso.example"));
    assert_eq!(sync.pushed_events.len(), 1);
    // A change goes field by field; the series keeps its id.
    let mut changed = pim.clone();
    changed.deleted.clear();
    changed.cards.clear();
    changed.events = vec![StoredEvent {
        href: standup.href.clone(),
        event: CalEvent { summary: "Stand-up (moved)".into(), rrule: None, ..standup.event.clone() },
        dirty: true,
        ..standup.clone()
    }];
    let sync = crate::ews_pim::sync(&c, &changed, 1_792_000_000).unwrap();
    let moved = sync.events.iter().find(|e| e.href == standup.href).unwrap();
    assert_eq!((moved.event.summary.as_str(), moved.event.rrule.as_deref()), ("Stand-up (moved)", None));
    // No Notes folder on this server: notes stay on this computer.
    assert!(sync.notes.is_none());
}

/// A meeting's date can be changed on the server: a field URI that names its property on the
/// wrong type makes Exchange refuse the whole `UpdateItem` ("The request is invalid."), and
/// the change then stays on this computer while Outlook keeps the old day.
#[test]
fn a_meeting_moves_to_another_day_on_the_server() {
    use quatrelle_model::ical::CalEvent;
    use quatrelle_model::pim::{Pim, StoredEvent};
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let cal = d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    d.pim.push((
        "M1".into(),
        cal,
        parse(
            "<t:CalendarItem><t:Subject>test dga</t:Subject><t:Body BodyType=\"Text\">Point</t:Body>\
             <t:Start>2026-10-21T09:00:00Z</t:Start><t:End>2026-10-21T09:30:00Z</t:End><t:IsAllDayEvent>false</t:IsAllDayEvent>\
             <t:Location>Bureau</t:Location><t:UID>test-dga-1</t:UID></t:CalendarItem>",
        )
        .kids[0]
            .clone(),
    ));
    let server = serve(d);
    let user = format!("{DOMAIN}\\{USER}");
    let c = crate::ews::Client::connect(&server.url, &user, PASSWORD, "riley@contoso.example").unwrap();
    let mut pim = Pim::default();
    pim.events.push(StoredEvent {
        calendar: crate::ews_pim::CALENDAR.into(),
        href: "M1".into(),
        event: CalEvent {
            uid: "test-dga-1".into(),
            summary: "test dga".into(),
            location: "Bureau".into(),
            start: 1_792_000_000,
            end: 1_792_001_800,
            ..Default::default()
        },
        dirty: true,
        ..Default::default()
    });
    let sync = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    let day = {
        let data = server.data.lock().unwrap();
        let (_, _, el) = data.pim.iter().find(|(i, _, _)| i == "M1").unwrap();
        el.text_of("Start").unwrap_or_default().to_string()
    };
    assert_eq!(day, "2026-10-14T17:46:40Z", "the server has the new day");
}

#[test]
fn sticky_notes_sync_with_the_notes_folder() {
    use quatrelle_model::pim::{Note, Pim, StoredNote};
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let notes = d.folder("Notes", None, Some("notes"), "IPF.StickyNote");
    let old = parse(
        "<t:Message><t:ItemClass>IPM.StickyNote</t:ItemClass><t:Subject>Call Ana</t:Subject><t:Body BodyType=\"Text\">Call Ana\r\nabout the mix</t:Body>\
         <t:ExtendedProperty><t:ExtendedFieldURI PropertySetId=\"0006200E-0000-0000-C000-000000000046\" PropertyId=\"35584\" PropertyType=\"Integer\"/><t:Value>2</t:Value></t:ExtendedProperty></t:Message>",
    );
    d.pim.push(("N1".into(), notes, old.kids[0].clone()));
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let mut pim = Pim::default();
    pim.notes.push(StoredNote {
        note: Note { uid: "n-local".into(), text: "Buy paper\n170 gsm".into(), color: 1, modified: 5 },
        dirty: true,
        ..Default::default()
    });
    let sync = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    let list = sync.notes.unwrap();
    let ana = list.iter().find(|n| n.href == "note:N1").unwrap();
    assert_eq!((ana.note.text.as_str(), ana.note.color), ("Call Ana\nabout the mix", 2));
    assert!(list.iter().any(|n| n.note.text == "Buy paper\n170 gsm" && n.note.color == 1), "made in the Notes folder, its colour kept");
    assert_eq!(sync.pushed_notes.len(), 1);
    assert!(sync.pushed_notes[0].href.starts_with("note:"));
}

#[test]
fn someone_elses_calendar_opens_read_only() {
    use quatrelle_model::pim::{Pim, shared_calendar_id};
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let review = parse(
        "<t:CalendarItem><t:Subject>Budget review</t:Subject><t:Start>2026-10-06T13:00:00Z</t:Start><t:End>2026-10-06T14:00:00Z</t:End>\
         <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>budget-1</t:UID></t:CalendarItem>",
    );
    d.pim.push(("S1".into(), "mailbox:ana@contoso.example".into(), review.kids[0].clone()));
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let pim = Pim { shared: vec!["ana@contoso.example".into(), "nobody@contoso.example".into()], ..Default::default() };
    let sync = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    let id = shared_calendar_id("ana@contoso.example");
    let cals = sync.calendars.clone().unwrap();
    assert!(cals.iter().any(|x| x.id == id && x.read_only && x.name == "ana@contoso.example"));
    assert!(sync.events.iter().any(|e| e.calendar == id && e.event.summary == "Budget review"));
    assert!(sync.synced_calendars.contains(&id));
    // One that can't be read says so, and the rest still syncs.
    assert!(sync.errors.iter().any(|e| e.starts_with("nobody@contoso.example's calendar")), "{:?}", sync.errors);
    assert!(!sync.synced_calendars.contains(&shared_calendar_id("nobody@contoso.example")));
}

/// A first sync (nothing known of other people's calendars yet) lists them and reads none:
/// Outlook leaves them unticked, and reading a whole organisation's weeks is minutes.
#[test]
fn a_first_sync_lists_other_peoples_calendars_without_reading_them() {
    use quatrelle_model::pim::{Pim, shared_calendar_id};
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let cal = d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    let contacts = d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let ev = parse(
        "<t:CalendarItem><t:Subject>Budget review</t:Subject><t:Start>2026-10-06T13:00:00Z</t:Start><t:End>2026-10-06T14:00:00Z</t:End>\
         <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>budget-1</t:UID></t:CalendarItem>",
    );
    d.pim.push(("M1".into(), cal, ev.kids[0].clone()));
    // Ana's calendar, in one of Outlook's groups.
    d.pim.push(("S1".into(), "mailbox:ana@contoso.example".into(), ev.kids[0].clone()));
    let dn = |who: &str| format!("/o=Contoso/ou=Exchange Administrative Group (FYDIBOHF23SPDLT)/cn=Recipients/cn={who}");
    d.directory = vec![(dn("ana"), "ana@contoso.example".into())];
    d.wlinks = vec![wlink("IPM.Microsoft.WunderBar.Link", 3, 2, "Équipe : Studio", "Ana Lima", &dn("ana"))];
    // One contact of this account's own.
    d.pim.push((
        "C1".into(),
        contacts,
        parse(
            "<t:Contact><t:FileAs>Ana Lima</t:FileAs><t:DisplayName>Ana Lima</t:DisplayName><t:GivenName>Ana</t:GivenName>\
             <t:EmailAddresses><t:Entry>SmtpAddress</t:Entry><t:EmailAddress>ana@contoso.example</t:EmailAddress></t:EmailAddresses></t:Contact>",
        )
        .kids[0]
            .clone(),
    ));
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let sync = crate::ews_pim::sync(&c, &Pim::default(), 1_792_000_000).unwrap();
    // Its own calendar read.
    assert!(sync.events.iter().any(|e| e.calendar == crate::ews_pim::CALENDAR && e.event.summary == "Budget review"));
    // Ana's listed, and nothing of it read.
    let ana = shared_calendar_id("ana@contoso.example");
    assert!(sync.calendars.as_ref().is_some_and(|c| c.iter().any(|x| x.id == ana)), "{:?}", sync.calendars);
    assert!(!sync.events.iter().any(|e| e.calendar == ana), "not read");
    assert!(!sync.synced_calendars.contains(&ana));
    // And the contacts came anyway.
    assert!(sync.cards.iter().any(|k| k.card.name == "Ana Lima"), "contacts: {}", sync.cards.len());
}

/// Away from the calendar only this account's own is read; in it, other people's calendars
/// left unticked are listed but not read.
#[test]
fn only_the_calendars_shown_are_read() {
    use quatrelle_model::pim::{Pim, shared_calendar_id};
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let ev = parse(
        "<t:CalendarItem><t:Subject>Budget review</t:Subject><t:Start>2026-10-06T13:00:00Z</t:Start><t:End>2026-10-06T14:00:00Z</t:End>\
         <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>budget-1</t:UID></t:CalendarItem>",
    );
    d.pim.push(("S1".into(), "mailbox:ana@contoso.example".into(), ev.kids[0].clone()));
    d.pim.push(("S2".into(), "mailbox:tom@contoso.example".into(), ev.kids[0].clone()));
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let (ana, tom) = (shared_calendar_id("ana@contoso.example"), shared_calendar_id("tom@contoso.example"));
    let shared = vec!["ana@contoso.example".to_string(), "tom@contoso.example".to_string()];
    // In Mail: this account's calendar alone, and the sync says it looked at no more.
    let mail = crate::ews_pim::sync(&c, &Pim { shared: shared.clone(), own_only: true, ..Default::default() }, 1_792_000_000).unwrap();
    assert!(mail.partial);
    assert_eq!(mail.calendars.as_ref().map(Vec::len), Some(1));
    assert!(!mail.events.iter().any(|e| e.calendar == ana || e.calendar == tom));
    // In the calendar, Tom's unticked: listed, not read.
    let cal = crate::ews_pim::sync(&c, &Pim { shared, not_read: vec![tom.clone()], ..Default::default() }, 1_792_000_000).unwrap();
    assert!(!cal.partial);
    let ids: Vec<&str> = cal.calendars.iter().flatten().map(|x| x.id.as_str()).collect();
    assert!(ids.contains(&ana.as_str()) && ids.contains(&tom.as_str()), "{ids:?}");
    assert!(cal.events.iter().any(|e| e.calendar == ana));
    assert!(!cal.events.iter().any(|e| e.calendar == tom));
    assert!(cal.synced_calendars.contains(&ana) && !cal.synced_calendars.contains(&tom));
}

/// A calendar is read from six months back (and its recurring series whenever they began):
/// reading every meeting since a calendar began, for each colleague's, never ended.
#[test]
fn calendars_are_read_from_six_months_back() {
    use quatrelle_model::pim::Pim;
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let cal = d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let item = |subject: &str, start: &str, end: &str, recurring: bool| {
        let rec = if recurring {
            "<t:Recurrence><t:WeeklyRecurrence><t:Interval>1</t:Interval><t:DaysOfWeek>Monday</t:DaysOfWeek></t:WeeklyRecurrence>\
             <t:NoEndRecurrence><t:StartDate>2019-01-07</t:StartDate></t:NoEndRecurrence></t:Recurrence>"
        } else {
            ""
        };
        parse(&format!(
            "<t:CalendarItem><t:Subject>{subject}</t:Subject><t:Start>{start}</t:Start><t:End>{end}</t:End>\
             <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>{subject}</t:UID>{rec}</t:CalendarItem>"
        ))
        .kids
        .remove(0)
    };
    d.pim.push(("P1".into(), cal.clone(), item("Kick-off 2019", "2019-01-07T09:00:00Z", "2019-01-07T10:00:00Z", false)));
    d.pim.push(("P2".into(), cal.clone(), item("Weekly", "2019-01-07T09:00:00Z", "2019-01-07T09:30:00Z", true)));
    d.pim.push(("P3".into(), cal, item("Review", "2026-10-06T13:00:00Z", "2026-10-06T14:00:00Z", false)));
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let sync = crate::ews_pim::sync(&c, &Pim::default(), 1_792_000_000).unwrap();
    let names: Vec<&str> = sync.events.iter().map(|e| e.event.summary.as_str()).collect();
    assert!(names.contains(&"Weekly") && names.contains(&"Review"), "{names:?}");
    assert!(!names.contains(&"Kick-off 2019"), "{names:?}");
}

/// An Outlook navigation shortcut to someone's calendar, in a group of the Calendar pane.
fn wlink(class: &str, section: i64, kind: i64, group: &str, name: &str, dn: &str) -> String {
    let mut eid = vec![0u8; 4];
    eid.extend([0xDC, 0xA7, 0x40, 0xC8, 0xC0, 0x42, 0x10, 0x1A, 0xB4, 0xB9, 0x08, 0x00, 0x2B, 0x2F, 0xE1, 0x82]);
    eid.extend([1, 0, 0, 0, 0, 0, 0, 0]);
    eid.extend(dn.as_bytes());
    eid.push(0);
    let p = |tag: &str, kind: &str, v: &str| {
        format!("<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"{tag}\" PropertyType=\"{kind}\"/><t:Value>{v}</t:Value></t:ExtendedProperty>")
    };
    format!(
        "<t:Message><t:ItemId Id=\"W{name}\"/>{}{}{}{}{}{}</t:Message>",
        p("0x1a", "String", class),
        p("0xe1d", "String", name),
        p("0x6849", "Integer", &kind.to_string()),
        p("0x6851", "String", group),
        p("0x6852", "Integer", &section.to_string()),
        p("0x6854", "Binary", &crate::imap::base64(&eid)),
    )
}

/// The calendars Outlook keeps in groups of its Calendar pane come in under those groups,
/// read as other people's calendars; shortcuts to mail folders, to this account's own
/// calendar, or to names the directory doesn't know are left alone.
#[test]
fn outlook_calendar_groups_come_in_with_their_calendars() {
    use quatrelle_model::pim::{Pim, shared_calendar_id};
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let standup = parse(
        "<t:CalendarItem><t:Subject>Stand-up</t:Subject><t:Start>2026-10-06T08:00:00Z</t:Start><t:End>2026-10-06T08:15:00Z</t:End>\
         <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>standup-1</t:UID></t:CalendarItem>",
    );
    d.pim.push(("S1".into(), "mailbox:ana@contoso.example".into(), standup.kids[0].clone()));
    d.pim.push(("S2".into(), "mailbox:room.lyon@contoso.example".into(), standup.kids[0].clone()));
    let dn = |who: &str| format!("/o=Contoso/ou=Exchange Administrative Group (FYDIBOHF23SPDLT)/cn=Recipients/cn={who}");
    d.directory = vec![
        (dn("ana"), "Ana@Contoso.example".into()),
        (dn("lyon"), "room.lyon@contoso.example".into()),
        (dn("riley"), "riley@contoso.example".into()),
    ];
    let link = "IPM.Microsoft.WunderBar.Link";
    d.wlinks = vec![
        wlink(link, 3, 2, "Équipe : Studio", "Ana Lima", &dn("ana")),
        wlink(link, 3, 2, "Salles", "Salle Lyon", &dn("lyon")),
        // Twice, in another group: one calendar.
        wlink(link, 3, 2, "Calendriers partagés", "Ana", &dn("ana")),
        // This account's own calendar, a mail folder shortcut, a group header, someone unknown.
        wlink(link, 3, 2, "Mes calendriers", "Riley", &dn("riley")),
        wlink(link, 1, 2, "Favorites", "Ana's Inbox", &dn("ana")),
        wlink(link, 3, 4, "Salles", "", ""),
        wlink(link, 3, 2, "Salles", "Ghost", &dn("ghost")),
    ];
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let links = crate::ews_pim::calendar_links(&c).unwrap();
    assert_eq!(
        links.iter().map(|l| (l.email.as_str(), l.name.as_str(), l.group.as_str())).collect::<Vec<_>>(),
        [("ana@contoso.example", "Ana Lima", "Équipe : Studio"), ("room.lyon@contoso.example", "Salle Lyon", "Salles")]
    );
    // Opened here by address as well: once, under its group. The list is known (a second
    // sync: the first only lists them, see `a_first_sync_lists_other_peoples_calendars_without_reading_them`).
    let known = |id: &str| quatrelle_model::pim::CalendarInfo { id: id.into(), ..Default::default() };
    let pim =
        Pim { shared: vec!["ana@contoso.example".into()], calendars: vec![known(&shared_calendar_id("ana@contoso.example"))], ..Default::default() };
    let sync = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    let cals = sync.calendars.clone().unwrap();
    let ana = shared_calendar_id("ana@contoso.example");
    let room = shared_calendar_id("room.lyon@contoso.example");
    assert_eq!(cals.iter().filter(|x| x.id == ana).count(), 1);
    assert!(cals.iter().any(|x| x.id == room && x.name == "Salle Lyon" && x.group == "Salles" && x.read_only), "{cals:?}");
    assert!(sync.events.iter().any(|e| e.calendar == room && e.event.summary == "Stand-up"));
    assert!(sync.synced_calendars.contains(&room));
}

/// Someone who made this account an editor of their calendar: events go in, change and go
/// there; someone who didn't keeps theirs read-only.
#[test]
fn someone_elses_calendar_can_be_changed_by_an_editor() {
    use quatrelle_model::ical::CalEvent;
    use quatrelle_model::pim::{Pim, StoredEvent, Tombstone, shared_calendar_id};
    let mut d = Data { editors: vec!["ana@contoso.example".into()], ..Default::default() };
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let item = |subject: &str, uid: &str| {
        parse(&format!(
            "<t:CalendarItem><t:Subject>{subject}</t:Subject><t:Start>2026-10-06T13:00:00Z</t:Start><t:End>2026-10-06T14:00:00Z</t:End>\
             <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>{uid}</t:UID></t:CalendarItem>"
        ))
        .kids[0]
            .clone()
    };
    d.pim.push(("S1".into(), "mailbox:ana@contoso.example".into(), item("Budget review", "budget-1")));
    d.pim.push(("S2".into(), "mailbox:ana@contoso.example".into(), item("Team lunch", "lunch-1")));
    d.pim.push(("B1".into(), "mailbox:ben@contoso.example".into(), item("Ben's call", "call-1")));
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let (ana, ben) = (shared_calendar_id("ana@contoso.example"), shared_calendar_id("ben@contoso.example"));
    let mut pim = Pim { shared: vec!["ana@contoso.example".into(), "ben@contoso.example".into()], ..Default::default() };
    let first = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    let cals = first.calendars.clone().unwrap();
    let ro = |id: &str| cals.iter().find(|x| x.id == id).map(|x| x.read_only);
    assert_eq!((ro(&ana), ro(&ben)), (Some(false), Some(true)));
    let mut review = first.events.iter().find(|e| e.event.summary == "Budget review").cloned().unwrap();
    let lunch = first.events.iter().find(|e| e.event.summary == "Team lunch").cloned().unwrap();
    review.event.summary = "Budget review (moved)".into();
    review.dirty = true;
    pim.events = vec![
        review,
        StoredEvent {
            calendar: ana.clone(),
            event: CalEvent { uid: "new-1".into(), summary: "Poster check".into(), start: 1_792_000_000, end: 1_792_003_600, ..Default::default() },
            dirty: true,
            ..Default::default()
        },
    ];
    pim.deleted = vec![Tombstone { href: lunch.href.clone(), etag: String::new(), calendar: ana.clone() }];
    let second = crate::ews_pim::sync(&c, &pim, 1_792_000_100).unwrap();
    assert!(second.errors.is_empty(), "{:?}", second.errors);
    assert!(second.pushed_events.iter().all(|e| e.calendar == ana), "they stay in Ana's calendar");
    let d = server.data.lock().unwrap();
    let in_ana: Vec<String> = d
        .pim
        .iter()
        .filter(|(_, f, _)| f == "mailbox:ana@contoso.example")
        .filter_map(|(_, _, el)| el.text_of("Subject").map(str::to_string))
        .collect();
    assert!(in_ana.contains(&"Budget review (moved)".to_string()) && in_ana.contains(&"Poster check".to_string()), "{in_ana:?}");
    assert!(!in_ana.contains(&"Team lunch".to_string()), "deleted");
    assert!(!d.pim.iter().any(|(_, f, el)| !f.starts_with("mailbox:") && el.text_of("Subject") == Some("Poster check")), "not in this mailbox");
}

/// Issue #1, again: a server whose answer about someone's calendar folder doesn't say an
/// editor may change it, while each of its events does (as Outlook finds by trying).
#[test]
fn an_editor_is_known_by_the_events_rights_too() {
    use quatrelle_model::pim::{Pim, shared_calendar_id};
    let mut d = Data { editors: vec!["ana@contoso.example".into()], item_rights_only: true, ..Default::default() };
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let item = parse(
        "<t:CalendarItem><t:Subject>Budget review</t:Subject><t:Start>2026-10-06T13:00:00Z</t:Start><t:End>2026-10-06T14:00:00Z</t:End>\
         <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>budget-1</t:UID></t:CalendarItem>",
    );
    d.pim.push(("S1".into(), "mailbox:ana@contoso.example".into(), item.kids[0].clone()));
    d.pim.push(("B1".into(), "mailbox:ben@contoso.example".into(), item.kids[0].clone()));
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let pim = Pim { shared: vec!["ana@contoso.example".into(), "ben@contoso.example".into()], ..Default::default() };
    let sync = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    let cals = sync.calendars.clone().unwrap();
    let ro = |id: &str| cals.iter().find(|x| x.id == id).map(|x| x.read_only);
    assert_eq!(
        (ro(&shared_calendar_id("ana@contoso.example")), ro(&shared_calendar_id("ben@contoso.example"))),
        (Some(false), Some(true)),
        "Ana made this account an editor; Ben didn't"
    );
}

/// One occurrence of a series dragged to another hour: asked for by its place in the series
/// (`OccurrenceItemId`), changed as an item of its own, and read back with the series at the
/// next sync on its new hour (the series' deleted occurrences as the days taken out of it). A
/// meeting the user organizes is sent to its guests by the server when the change asks for it,
/// against the item as last seen; otherwise no one hears of it.
#[test]
fn an_occurrence_moves_on_its_own_and_the_server_tells_the_guests() {
    use quatrelle_model::ical::CalEvent;
    use quatrelle_model::pim::{Pim, StoredEvent};
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let cal = d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let standup = parse(
        "<t:CalendarItem><t:Subject>Stand-up</t:Subject><t:Start>2026-10-05T09:00:00Z</t:Start><t:End>2026-10-05T09:15:00Z</t:End>\
         <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>standup-1</t:UID><t:CalendarItemType>RecurringMaster</t:CalendarItemType>\
         <t:Recurrence><t:WeeklyRecurrence><t:Interval>1</t:Interval><t:DaysOfWeek>Monday</t:DaysOfWeek></t:WeeklyRecurrence>\
         <t:NoEndRecurrence><t:StartDate>2026-10-05Z</t:StartDate></t:NoEndRecurrence></t:Recurrence>\
         <t:DeletedOccurrences><t:DeletedOccurrence><t:Start>2026-10-19T09:00:00Z</t:Start></t:DeletedOccurrence></t:DeletedOccurrences></t:CalendarItem>",
    );
    d.pim.push(("E1".into(), cal.clone(), standup.kids[0].clone()));
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let at = |s: &str| quatrelle_jmap::utc_date(s).unwrap();
    let first = crate::ews_pim::sync(&c, &Pim::default(), 1_792_000_000).unwrap();
    let master = first.events.iter().find(|e| e.event.uid == "standup-1").cloned().unwrap();
    assert_eq!(master.event.exdates, [at("2026-10-19T09:00:00Z")], "the deleted occurrence is a day taken out");
    // The second Monday's, an hour later: an instance of its own, with no item yet.
    let second = at("2026-10-12T09:00:00Z");
    let mut pim = Pim { events: first.events.clone(), ..Default::default() };
    pim.events.push(StoredEvent {
        calendar: crate::ews_pim::CALENDAR.into(),
        href: String::new(),
        etag: String::new(),
        event: CalEvent {
            recurrence_id: Some(second),
            rrule: None,
            exdates: Vec::new(),
            start: second + 3600,
            end: second + 4500,
            ..master.event.clone()
        },
        dirty: true,
        notify: false,
        respond: None,
    });
    let second_sync = crate::ews_pim::sync(&c, &pim, 1_792_000_100).unwrap();
    assert!(second_sync.errors.is_empty(), "{:?}", second_sync.errors);
    let pushed = second_sync.pushed_events.iter().find(|e| e.event.recurrence_id == Some(second)).unwrap();
    assert_eq!(pushed.href, "E1:occ2", "the occurrence's own item, the second in the series");
    // Read back with the series: the instance on its new hour, the series itself as it was.
    let back: Vec<&StoredEvent> = second_sync.events.iter().filter(|e| e.event.uid == "standup-1").collect();
    assert_eq!(back.len(), 2, "{back:?}");
    let instance = back.iter().find(|e| e.event.recurrence_id.is_some()).unwrap();
    assert_eq!((instance.event.recurrence_id, instance.event.start, instance.href.as_str()), (Some(second), second + 3600, "E1:occ2"));
    assert_eq!((instance.event.summary.as_str(), instance.event.rrule.as_deref()), ("Stand-up", None));
    assert!(back.iter().any(|e| e.event.recurrence_id.is_none() && e.event.start == master.event.start));
    assert_eq!(server.data.lock().unwrap().meeting_sends, ["SendToNone"]);
    // The series moved by its organizer with "Send update": the server tells the guests,
    // resolving against the item as last seen; "Save without sending": no one hears of it.
    for (notify, delta) in [(true, 1800), (false, 3600)] {
        let mut pim = Pim { events: second_sync.events.clone(), ..Default::default() };
        for e in pim.events.iter_mut().filter(|e| e.event.uid == "standup-1" && e.event.recurrence_id.is_none()) {
            e.event.start = master.event.start + delta;
            e.event.end = master.event.end + delta;
            e.dirty = true;
            e.notify = notify;
        }
        let again = crate::ews_pim::sync(&c, &pim, 1_792_000_200).unwrap();
        assert!(again.errors.is_empty(), "{:?}", again.errors);
    }
    // A new meeting with guests, sent by the server when asked: the guests go with it (the
    // organizer isn't one of them), so the server has someone to invite.
    let who = |name: &str, email: &str| quatrelle_model::ical::Attendee {
        name: name.into(),
        email: email.into(),
        partstat: "NEEDS-ACTION".into(),
        rsvp: true,
        cutype: String::new(),
    };
    let mut pim = Pim::default();
    pim.events.push(StoredEvent {
        calendar: crate::ews_pim::CALENDAR.into(),
        href: String::new(),
        etag: String::new(),
        event: CalEvent {
            uid: "new-1".into(),
            summary: "Budget".into(),
            start: 1_792_000_000,
            end: 1_792_003_600,
            organizer: Some(who("Riley", "riley@contoso.example")),
            attendees: vec![who("Riley", "riley@contoso.example"), who("Ana Lima", "ana@contoso.example")],
            ..Default::default()
        },
        dirty: true,
        notify: true,
        respond: None,
    });
    let made = crate::ews_pim::sync(&c, &pim, 1_792_000_300).unwrap();
    assert!(made.errors.is_empty(), "{:?}", made.errors);
    assert_eq!(server.data.lock().unwrap().meeting_sends, ["SendToNone", "SendToAllAndSaveCopy", "SendToNone", "SendToAllAndSaveCopy"]);
    let budget = made.events.iter().find(|e| e.event.summary == "Budget").unwrap();
    let guests: Vec<&str> = budget.event.attendees.iter().map(|a| a.email.as_str()).collect();
    assert_eq!(guests, ["ana@contoso.example"]);
    // Someone else's meeting changed here (its reminder, say) keeps their guest list.
    let mut theirs = budget.clone();
    theirs.event.organizer = Some(who("Ana Lima", "ana@contoso.example"));
    theirs.event.attendees = vec![who("Riley", "riley@contoso.example")];
    assert!(crate::ews_pim::events::event_fields(&theirs.event, "riley@contoso.example").iter().all(|(uri, _)| *uri != "calendar:RequiredAttendees"));
}

/// Invitations are answered as Outlook answers them, by the server, on the meeting it placed in
/// the calendar (never a second copy, which made each accepted meeting show twice with the user
/// as its organizer): accepted on its item, answered once it's read when it wasn't in the
/// calendar here yet, and declined out of the calendar.
#[test]
fn invitations_are_answered_by_the_server_on_its_meeting() {
    use quatrelle_model::pim::{Pim, same_meeting};
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let cal = d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let meeting = |uid: &str, day: u32| {
        parse(&format!(
            "<t:CalendarItem><t:Subject>Review {uid}</t:Subject><t:Start>2026-10-{day}T09:00:00Z</t:Start><t:End>2026-10-{day}T10:00:00Z</t:End>\
             <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>{uid}</t:UID><t:CalendarItemType>Single</t:CalendarItemType>\
             <t:Organizer><t:Mailbox><t:Name>Ana</t:Name><t:EmailAddress>ana@contoso.example</t:EmailAddress></t:Mailbox></t:Organizer>\
             <t:RequiredAttendees><t:Attendee><t:Mailbox><t:EmailAddress>riley@contoso.example</t:EmailAddress></t:Mailbox><t:ResponseType>Unknown</t:ResponseType></t:Attendee></t:RequiredAttendees>\
             <t:MyResponseType>NoResponseReceived</t:MyResponseType></t:CalendarItem>"
        ))
        .kids[0]
            .clone()
    };
    d.pim.push(("E1".into(), cal.clone(), meeting("mtg-1", 27)));
    let server = serve(d);
    let data = server.data.clone();
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let first = crate::ews_pim::sync(&c, &Pim::default(), 1_792_000_000).unwrap();
    let item_count = || data.lock().unwrap().pim.len();
    // Accepted, with the meeting in the calendar here: answered on its item.
    let mut pim = Pim { events: first.events.clone(), ..Default::default() };
    let e = pim.events.iter_mut().find(|e| same_meeting(&e.event.uid, "mtg-1")).expect("the server's meeting");
    assert_ne!(e.event.uid, "mtg-1", "a single one is kept as UID#start");
    e.respond = Some("ACCEPTED".into());
    e.dirty = true;
    let before = item_count();
    let out = crate::ews_pim::sync(&c, &pim, 1_792_000_100).unwrap();
    assert_eq!(data.lock().unwrap().answers, [("E1".to_string(), "AcceptItem".to_string(), true)], "the organizer told by the server");
    assert_eq!(out.responded.len(), 1);
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    assert_eq!(item_count(), before, "no appointment created");
    // A new invitation, answered before the calendar here had it: answered once it's read.
    data.lock().unwrap().pim.push(("E2".into(), cal.clone(), meeting("mtg-2", 28)));
    let placeholder = quatrelle_model::pim::StoredEvent {
        calendar: crate::ews_pim::CALENDAR.into(),
        event: quatrelle_model::ical::CalEvent { uid: "mtg-2".into(), summary: "Review mtg-2".into(), ..Default::default() },
        dirty: true,
        respond: Some("TENTATIVE".into()),
        ..Default::default()
    };
    let pim = Pim { events: vec![placeholder], ..Default::default() };
    let before = item_count();
    let out = crate::ews_pim::sync(&c, &pim, 1_792_000_200).unwrap();
    assert_eq!(data.lock().unwrap().answers.last().map(|a| (a.0.as_str(), a.1.as_str())), Some(("E2", "TentativelyAcceptItem")));
    assert_eq!(out.responded, [("mtg-2".to_string(), "TENTATIVE".to_string())]);
    assert_eq!(item_count(), before, "no appointment created for it either");
    // Declined: the server takes it out of the calendar.
    let mut pim = Pim { events: out.events.clone(), ..Default::default() };
    let e = pim.events.iter_mut().find(|e| same_meeting(&e.event.uid, "mtg-2")).unwrap();
    e.respond = Some("DECLINED".into());
    e.dirty = true;
    let out = crate::ews_pim::sync(&c, &pim, 1_792_000_300).unwrap();
    assert!(!out.events.iter().any(|e| same_meeting(&e.event.uid, "mtg-2")), "declined: out of the calendar");
}

/// A weekly meeting made here in Paris, with a reminder, reaches Exchange (its fields in the
/// schema's order: the reminder, an item's field, before the start) with its zone, and comes
/// back from it in that zone, still at 10:00 after the clocks go back.
#[test]
fn a_series_made_here_goes_to_exchange_in_its_zone() {
    use quatrelle_model::ical::CalEvent;
    use quatrelle_model::pim::{Pim, StoredEvent};
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let at = |s: &str| quatrelle_jmap::utc_date(s).unwrap();
    let weekly = CalEvent {
        uid: "weekly-here".into(),
        summary: "Weekly point".into(),
        start: at("2026-10-19T08:00:00Z"),
        end: at("2026-10-19T09:00:00Z"),
        rrule: Some("FREQ=WEEKLY;BYDAY=MO;COUNT=3".into()),
        tzid: "Europe/Paris".into(),
        reminder: Some(15),
        ..Default::default()
    };
    let pim = Pim {
        events: vec![StoredEvent { calendar: crate::ews_pim::CALENDAR.into(), event: weekly, dirty: true, ..Default::default() }],
        ..Default::default()
    };
    let first = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    assert!(first.errors.is_empty(), "{:?}", first.errors);
    {
        let data = server.data.lock().unwrap();
        let (_, _, el) =
            data.pim.iter().find(|(_, _, el)| el.text_of("UID") == Some("weekly-here") || el.text_of("Subject") == Some("Weekly point")).unwrap();
        assert_eq!(el.child("StartTimeZone").and_then(|z| z.attr("Id")), Some("Romance Standard Time"));
        assert_eq!(el.child("EndTimeZone").and_then(|z| z.attr("Id")), Some("Romance Standard Time"));
        assert_eq!(el.find("StartDate").map(|d| d.text.as_str()), Some("2026-10-19"));
    }
    let again = crate::ews_pim::sync(&c, &Pim { events: first.pushed_events.clone(), ..Default::default() }, 1_792_000_100).unwrap();
    let back = again.events.iter().find(|e| e.event.summary == "Weekly point").unwrap();
    assert_eq!(back.event.tzid, "Romance Standard Time");
    assert_eq!(quatrelle_model::ical::starts(&back.event, at("2026-10-27T00:00:00Z")).last(), Some(&at("2026-10-26T09:00:00Z")));
}

/// The test server refuses a new appointment whose fields are out of the schema's order, as
/// Exchange does (a reminder written after the start failed so).
#[test]
fn an_appointment_out_of_the_schemas_order_is_refused() {
    let good = parse(
        "<t:CalendarItem><t:Subject>A</t:Subject><t:ReminderIsSet>true</t:ReminderIsSet><t:Start>2026-10-19T08:00:00Z</t:Start></t:CalendarItem>",
    );
    let bad = parse(
        "<t:CalendarItem><t:Subject>A</t:Subject><t:Start>2026-10-19T08:00:00Z</t:Start><t:ReminderIsSet>true</t:ReminderIsSet></t:CalendarItem>",
    );
    assert!(super::super::route::in_schema_order(&good.kids[0]));
    assert!(!super::super::route::in_schema_order(&bad.kids[0]));
}
