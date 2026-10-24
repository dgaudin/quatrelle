//! Tests of the EWS calendar and contact items, both ways.

use super::events::{recurrence_json, recurrence_xml};
use super::*;
use crate::ews::parse;

#[test]
fn recurrences_both_ways() {
    let start = quatrelle_jmap::utc_date("2026-10-05T09:00:00Z").unwrap_or(0);
    for rule in [
        "FREQ=WEEKLY;BYDAY=MO,WE;INTERVAL=2;UNTIL=20261231T235959Z",
        "FREQ=MONTHLY;BYDAY=-1FR;COUNT=6",
        "FREQ=YEARLY;BYMONTH=3;BYMONTHDAY=14",
        "FREQ=DAILY",
    ] {
        let xml = recurrence_xml(rule, start).unwrap();
        let el = parse(&xml);
        let back = el.find("Recurrence").and_then(recurrence_json).as_ref().and_then(rrule_of).unwrap();
        assert_eq!(back, rule, "{xml}");
    }
    assert!(recurrence_xml("FREQ=HOURLY", start).is_none());
    let junk = parse("<t:Recurrence><t:WeeklyRecurrence><t:Interval>x</t:Interval></t:WeeklyRecurrence></t:Recurrence>");
    assert!(junk.find("Recurrence").and_then(recurrence_json).is_some(), "a bad number falls back");
}

#[test]
fn contacts_both_ways() {
    let card = Card {
        uid: "x".into(),
        name: "Ana Lima".into(),
        given: "Ana".into(),
        family: "Lima".into(),
        emails: vec![Typed { value: "ana@contoso.example".into(), kind: String::new() }],
        // Issue #1: each number keeps what it is, two of a kind too.
        phones: vec![
            Typed { value: "+1 555 0101".into(), kind: "work".into() },
            Typed { value: "+1 555 0102".into(), kind: "work".into() },
            Typed { value: "+1 555 0103".into(), kind: "home".into() },
            Typed { value: "+1 555 0100".into(), kind: "cell".into() },
            Typed { value: "+1 555 0104".into(), kind: "fax".into() },
        ],
        org: "Contoso & Co".into(),
        title: "Producer".into(),
        note: "Met at the fair".into(),
        addresses: vec![
            PostalAddress {
                kind: "work".into(),
                street: "An street 1".into(),
                city: "Brussel".into(),
                postcode: "1000".into(),
                country: "Belgium".into(),
                ..Default::default()
            },
            PostalAddress { kind: "home".into(), street: "Rue Haute 9".into(), city: "Gent".into(), ..Default::default() },
        ],
        birthday: "1990-07-12".into(),
        extra: Vec::new(),
        ..Card::default()
    };
    let xml = contact_xml(&card).replace("<t:Contact>", "<t:Contact><t:ItemId Id=\"C1\" ChangeKey=\"k\"/>");
    // In the schema's order: addresses between emails and phones, the birthday after phones.
    let at = |s: &str| xml.find(s).unwrap();
    assert!(at("<t:EmailAddresses>") < at("<t:PhysicalAddresses>") && at("<t:PhysicalAddresses>") < at("<t:PhoneNumbers>"));
    assert!(at("<t:PhoneNumbers>") < at("<t:Birthday>") && at("<t:Birthday>") < at("<t:JobTitle>"));
    assert!(xml.contains("<t:Entry Key=\"BusinessPhone2\">+1 555 0102</t:Entry>"));
    let back = card_of(parse(&xml).find("Contact").unwrap()).unwrap();
    assert_eq!((back.name.as_str(), back.org.as_str(), back.note.as_str(), back.uid.as_str()), ("Ana Lima", "Contoso & Co", "Met at the fair", "C1"));
    assert_eq!(back.phones, card.phones);
    assert_eq!(back.emails, card.emails);
    assert_eq!(back.addresses, card.addresses);
    assert_eq!(back.birthday, "1990-07-12");
    // Exchange's own: a birthday kept as the start of that day in Brussels, in UTC.
    let theirs = "<t:Contact><t:ItemId Id=\"C2\"/><t:Birthday>1990-07-11T22:00:00Z</t:Birthday></t:Contact>";
    assert_eq!(card_of(parse(theirs).find("Contact").unwrap()).unwrap().birthday, "1990-07-12");
}

/// Appointments Exchange keeps as separate items under one series' UID (seen on an Exchange
/// 2016 server, a series expanded into single items): each one is its own event, so a whole
/// set of meetings isn't lost to a shared UID.
#[test]
fn single_items_sharing_a_uid_are_each_their_own() {
    let item = |id: &str, start: &str| {
        event_of(
            parse(&format!(
                "<t:CalendarItem><t:ItemId Id=\"{id}\"/><t:Subject>Comité de sécurité</t:Subject>\
                 <t:Start>{start}</t:Start><t:End>2026-10-23T10:00:00Z</t:End>\
                 <t:CalendarItemType>Single</t:CalendarItemType><t:UID>040000008200E00074C5</t:UID></t:CalendarItem>"
            ))
            .find("CalendarItem")
            .unwrap(),
        )
        .unwrap()
    };
    let a = item("I1", "2026-10-23T08:00:00Z");
    let b = item("I2", "2026-10-30T08:00:00Z");
    assert_ne!(a.uid, b.uid, "two appointments, two identities");
    assert_eq!(a.uid, item("I1", "2026-10-23T08:00:00Z").uid, "and the same one twice is the same");
    // A recurring master keeps the series' UID (its occurrences are worked out from it).
    let master = "<t:CalendarItem><t:ItemId Id=\"M1\"/><t:Subject>Weekly</t:Subject><t:Start>2026-10-02T08:00:00Z</t:Start>\
                  <t:End>2026-10-02T09:00:00Z</t:End><t:CalendarItemType>RecurringMaster</t:CalendarItemType>\
                  <t:UID>040000008200E00074C6</t:UID><t:Recurrence><t:WeeklyRecurrence><t:Interval>1</t:Interval>\
                  <t:DaysOfWeek>Friday</t:DaysOfWeek></t:WeeklyRecurrence><t:NoEndRecurrence><t:StartDate>2026-10-02</t:StartDate>\
                  </t:NoEndRecurrence></t:Recurrence></t:CalendarItem>";
    assert_eq!(event_of(parse(master).find("CalendarItem").unwrap()).unwrap().uid, "040000008200E00074C6");
}

/// An appointment's categories come in, and go out between its body and its start (the
/// schema's order); none at all aren't written.
#[test]
fn categories_both_ways() {
    let xml = "<t:CalendarItem><t:ItemId Id=\"E1\"/><t:Subject>Pitch</t:Subject><t:Categories><t:String>Clients</t:String><t:String> </t:String><t:String>Red category</t:String></t:Categories>\
               <t:Start>2026-10-16T10:00:00Z</t:Start><t:End>2026-10-16T11:00:00Z</t:End></t:CalendarItem>";
    let e = event_of(parse(xml).find("CalendarItem").unwrap()).unwrap();
    assert_eq!(e.categories, ["Clients", "Red category"]);
    // Each field carries the URI its property is declared on (`item:` for the item's own,
    // `calendar:` for the appointment's): Exchange refuses the request when one is wrong.
    let names: Vec<&str> = event_fields(&e, "riley@contoso.example").iter().map(|(uri, _)| *uri).collect();
    assert_eq!(names.get(..5), Some(&["item:Subject", "item:Body", "item:Categories", "item:ReminderIsSet", "calendar:Start"][..]));
    assert!(event_fields(&e, "riley@contoso.example").iter().any(|(_, x)| x.contains("<t:String>Red category</t:String>")));
    let plain = CalEvent { categories: Vec::new(), ..e };
    assert!(event_fields(&plain, "riley@contoso.example").iter().all(|(uri, _)| *uri != "item:Categories"));
}

/// A series set in Paris, read from Exchange, keeps its zone: its 10:00 stays 10:00 once the
/// clocks go back (08:00 UTC in summer, 09:00 in winter; it moved to 09:00 local when series
/// repeated in UTC), and it goes back out with its zone, by the Windows name Exchange knows.
#[test]
fn a_series_keeps_its_zone_across_the_winter_change() {
    let item = |zone: &str| {
        format!(
            "<t:CalendarItem><t:ItemId Id=\"S1\"/><t:Subject>Weekly point</t:Subject>\
             <t:Start>2026-10-19T08:00:00Z</t:Start><t:End>2026-10-19T09:00:00Z</t:End><t:IsAllDayEvent>false</t:IsAllDayEvent>\
             <t:CalendarItemType>RecurringMaster</t:CalendarItemType><t:UID>weekly</t:UID>\
             <t:Recurrence><t:WeeklyRecurrence><t:Interval>1</t:Interval><t:DaysOfWeek>Monday</t:DaysOfWeek></t:WeeklyRecurrence>\
             <t:NumberedRecurrence><t:StartDate>2026-10-19</t:StartDate><t:NumberOfOccurrences>3</t:NumberOfOccurrences></t:NumberedRecurrence></t:Recurrence>\
             {zone}</t:CalendarItem>"
        )
    };
    let read = |zone: &str| event_of(parse(&item(zone)).find("CalendarItem").unwrap()).unwrap();
    let e = read("<t:StartTimeZone Id=\"Romance Standard Time\" Name=\"(UTC+01:00) Brussels, Copenhagen, Madrid, Paris\"/>");
    assert_eq!(e.tzid, "Romance Standard Time");
    let at = |s: &str| quatrelle_jmap::utc_date(s).unwrap();
    assert_eq!(
        quatrelle_model::ical::starts(&e, at("2026-11-03T00:00:00Z")),
        [at("2026-10-19T08:00:00Z"), at("2026-10-26T09:00:00Z"), at("2026-11-02T09:00:00Z")],
        "10:00 in Paris each Monday"
    );
    let fields = event_fields(&e, "riley@contoso.example");
    let place = |uri: &str| fields.iter().position(|(u, _)| *u == uri);
    assert!(place("calendar:Recurrence") < place("calendar:StartTimeZone"), "the zone after the pattern, as the schema has it");
    assert!(fields.iter().any(|(_, x)| x == "<t:StartTimeZone Id=\"Romance Standard Time\"/>"));
    // One known by its IANA name (made here) goes out by its Windows one.
    let made_here = CalEvent { tzid: "Europe/Paris".into(), ..e.clone() };
    assert!(event_fields(&made_here, "riley@contoso.example").iter().any(|(_, x)| x == "<t:EndTimeZone Id=\"Romance Standard Time\"/>"));
    // No zone, a zone nobody knows, or an all-day event: none goes out, and none is read.
    assert_eq!(read("").tzid, "");
    assert_eq!(read("<t:StartTimeZone Id=\"Nowhere Standard Time\"/>").tzid, "");
    for e in [CalEvent { tzid: String::new(), ..e.clone() }, CalEvent { all_day: true, ..e.clone() }] {
        assert!(event_fields(&e, "riley@contoso.example").iter().all(|(u, _)| !u.ends_with("TimeZone")), "{}", e.tzid);
    }
}

/// The X.500 name comes out of an address book entry id; anything else (too short, another
/// provider, not a name, no end) gives nothing or what's there, never a crash.
#[test]
fn address_book_entry_ids_give_their_x500_name() {
    let mut eid = vec![0u8; 4];
    eid.extend([0xDC, 0xA7, 0x40, 0xC8, 0xC0, 0x42, 0x10, 0x1A, 0xB4, 0xB9, 0x08, 0x00, 0x2B, 0x2F, 0xE1, 0x82]);
    eid.extend([1, 0, 0, 0, 0, 0, 0, 0]);
    let dn = "/o=Contoso/ou=Exchange Administrative Group (FYDIBOHF23SPDLT)/cn=Recipients/cn=ana";
    let mut full = eid.clone();
    full.extend(dn.as_bytes());
    full.push(0);
    full.extend(b"trailing");
    assert_eq!(address_book_dn(&full).as_deref(), Some(dn));
    // No final zero: the name still reads.
    let mut open = eid.clone();
    open.extend(dn.as_bytes());
    assert_eq!(address_book_dn(&open).as_deref(), Some(dn));
    let mut other = full.clone();
    if let Some(b) = other.get_mut(5) {
        *b = 0;
    }
    assert_eq!(address_book_dn(&other), None);
    let mut junk = eid.clone();
    junk.extend([0xFF, 0xFE, 0]);
    assert_eq!(address_book_dn(&junk), None);
    for n in 0..full.len() {
        let _ = address_book_dn(full.get(..n).unwrap_or_default());
    }
    assert_eq!(address_book_dn(&[]), None);
}

/// An on-premises Exchange names the organizer of one's own meeting by an internal (`EX`)
/// address, not read as an address: the meeting is the calendar owner's when their answer is
/// "Organizer" (so dragging it offers the update to its guests), and someone else's otherwise.
#[test]
fn ones_own_meeting_is_known_by_the_answer_organizer() {
    let item = |answer: &str| {
        format!(
            "<t:CalendarItem><t:ItemId Id=\"m1\" ChangeKey=\"c\"/><t:Subject>Weekly</t:Subject><t:UID>u-1</t:UID>\
             <t:Start>2026-10-21T08:00:00Z</t:Start><t:End>2026-10-21T09:00:00Z</t:End><t:CalendarItemType>Single</t:CalendarItemType>\
             <t:Organizer><t:Mailbox><t:Name>Riley Moreno</t:Name><t:EmailAddress>/O=CONTOSO/OU=EXCHANGE ADMINISTRATIVE GROUP/CN=RECIPIENTS/CN=RMORENO</t:EmailAddress>\
             <t:RoutingType>EX</t:RoutingType></t:Mailbox></t:Organizer>\
             <t:RequiredAttendees><t:Attendee><t:Mailbox><t:Name>Ana</t:Name><t:EmailAddress>ana@x.example</t:EmailAddress></t:Mailbox>\
             <t:ResponseType>Unknown</t:ResponseType></t:Attendee></t:RequiredAttendees><t:MyResponseType>{answer}</t:MyResponseType></t:CalendarItem>"
        )
    };
    let doc = parse(&item("Organizer"));
    let it = doc.find("CalendarItem").unwrap();
    let mut e = event_of(it).unwrap();
    assert!(e.organizer.is_none(), "an EX address isn't read as one");
    own_organizer(&mut e, it, "Riley@Contoso.example");
    let o = e.organizer.unwrap();
    assert_eq!((o.email.as_str(), o.name.as_str()), ("riley@contoso.example", "Riley Moreno"));
    let doc = parse(&item("Accept"));
    let it = doc.find("CalendarItem").unwrap();
    let mut e = event_of(it).unwrap();
    own_organizer(&mut e, it, "riley@contoso.example");
    assert!(e.organizer.is_none(), "someone else's: not made the owner's");
}
