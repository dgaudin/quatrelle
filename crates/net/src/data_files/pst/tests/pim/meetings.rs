// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! A meeting as its appointment keeps it: the zone its times were set in, its organizer, each
//! guest's answer and its uid.

use outlook_pst::{UnicodePstRecipient, UnicodePstRecipientType, UnicodePstValue as V};
use quatrelle_model::dates::days_from_civil;
use quatrelle_model::ical;

use super::*;

/// A time zone definition (MS-OXOCAL 2.2.1.41) up to its key name, `name`.
fn zone_definition(name: &str) -> Vec<u8> {
    let units: Vec<u16> = name.encode_utf16().collect();
    let mut b = vec![0x02, 0x01, 0x06, 0x00, 0x02, 0x00];
    b.extend(u16::try_from(units.len()).unwrap().to_le_bytes());
    b.extend(units.iter().flat_map(|u| u.to_le_bytes()));
    b
}

/// A weekly meeting on Mondays at 09:00 in Paris, three times from 19 October 2026: summer time
/// ends on the 25th. Its organizer is flagged among its recipients; Ana accepted, Ben declined,
/// Chloé hasn't answered; it was made by a calendar other than Outlook, whose uid its global
/// object id holds. It comes in at 09:00 Paris time each week, from its organizer, with each
/// guest's answer and its own uid.
#[test]
fn a_meeting_keeps_its_zone_organizer_answers_and_uid() {
    let minutes = crate::pst_pim::tests::minutes;
    let pattern = crate::pst_pim::tests::blob(0x200B, 0x0001, 1, &[0b10], (0x2022, 3), &[], &[], minutes(2026, 10, 19));
    let start = days_from_civil(2026, 10, 19) * 86_400 + 7 * 3600;
    let zone = zone_definition("W. Europe Standard Time");
    let global = [
        &[0x04, 0, 0, 0, 0x82, 0, 0xE0, 0, 0x74, 0xC5, 0xB7, 0x10, 0x1A, 0x82, 0xE0, 0x08][..],
        &[0; 20],
        &34u32.to_le_bytes(),
        b"vCal-Uid\x01\0\0\0review@studio.example\0",
    ]
    .concat();
    let props = [
        named(APPOINTMENT, 0x820D, V::Time(filetime(start))),
        named(APPOINTMENT, 0x820E, V::Time(filetime(start + 3600))),
        named(APPOINTMENT, 0x8216, V::Binary(&pattern)),
        named(APPOINTMENT, 0x8234, V::Unicode("(UTC+01:00) Amsterdam, Berlin, Bern, Rome, Stockholm, Vienna")),
        named(APPOINTMENT, 0x825E, V::Binary(&zone)),
        named(MEETING, 0x0003, V::Binary(&global)),
        UnicodePstProperty::tagged(0x0063, V::Boolean(true)),
    ];
    let person = |name, email, flags, track_status| UnicodePstRecipient {
        name,
        email,
        recipient_type: UnicodePstRecipientType::To,
        flags: Some(flags),
        track_status,
    };
    let recipients = [
        person("Riley", "riley@studio.example", 0x3, None),
        person("Ana", "ana@posteo.example", 0x1, Some(3)),
        person("Ben", "ben@studio.example", 0x1, Some(4)),
        person("chloe@studio.example", "chloe@studio.example", 0x1, Some(0)),
    ];
    let meeting = UnicodePstMessage { recipients: &recipients, ..item("IPM.Appointment", "Review", "Weekly.", &props) };
    let b = Brought::of("meeting", &[Put::new(&["Calendar"], "IPF.Appointment", meeting)]);
    let pim = b.pim();
    let e = &pim.events.first().expect("the meeting").event;
    assert_eq!(e.uid, "review@studio.example");
    assert_eq!(e.tzid, "W. Europe Standard Time");
    assert_eq!(e.organizer.as_ref().map(|o| o.email.as_str()), Some("riley@studio.example"));
    let guests: Vec<(&str, &str, &str, bool)> =
        e.attendees.iter().map(|a| (a.email.as_str(), a.name.as_str(), a.partstat.as_str(), a.rsvp)).collect();
    assert_eq!(
        guests,
        [("ana@posteo.example", "Ana", "ACCEPTED", false), ("ben@studio.example", "Ben", "DECLINED", false), ("chloe@studio.example", "", "", true)]
    );
    // 09:00 in Paris each week: 07:00Z in summer time, 08:00Z after it.
    let day = |m: u32, d: u32, h: i64| days_from_civil(2026, m, d) * 86_400 + h * 3600;
    let shown: Vec<i64> = ical::occurrences(std::slice::from_ref(e), day(10, 1, 0), day(11, 30, 0)).iter().map(|o| o.start).collect();
    assert_eq!(shown, [day(10, 19, 7), day(10, 26, 8), day(11, 2, 8)]);
    // And so once written as a calendar file and read back.
    let back = ical::parse(&ical::write(std::slice::from_ref(e), None, 0), 0).events;
    let shown: Vec<i64> = ical::occurrences(&back, day(10, 1, 0), day(11, 30, 0)).iter().map(|o| o.start).collect();
    assert_eq!(shown, [day(10, 19, 7), day(10, 26, 8), day(11, 2, 8)]);
}
