// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! What an event is written back as: without overflowing at the end of time, and with its
//! components begun plainly.

use super::*;

/// An event from a damaged cache, starting at the end of time, is written without overflowing.
#[test]
fn an_event_at_the_end_of_time_is_written() {
    for (all_day, tzid) in [(true, ""), (false, ""), (false, "Europe/Berlin"), (false, "Nowhere/Made-up")] {
        for start in [i64::MAX - 10, i64::MIN + 10] {
            let e = CalEvent { uid: "far".into(), start, end: 0, all_day, tzid: tzid.into(), ..Default::default() };
            let text = write(std::slice::from_ref(&e), None, 0);
            assert!(text.contains("UID:far") && text.contains("DTEND"), "{text}");
        }
    }
}

/// A component begun with spaces round its name (`BEGIN: valarm`, which reading takes) is written
/// back as servers take it.
#[test]
fn a_component_s_beginning_is_written_plainly() {
    let cal = parse(
        "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:sp\r\nDTSTART:20261103T090000Z\r\nBEGIN: x-big \r\nX-LINE:junk\r\nEND:x-big\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        0,
    );
    let text = write(&cal.events, None, 0);
    assert!(text.contains("\r\nBEGIN:x-big\r\nX-LINE:junk\r\nEND:x-big\r\n"), "{text}");
    // An alarm Quatrelle can't read is kept whole, its beginning written plainly too.
    let cal = parse(
        "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:sp2\r\nDTSTART:20261103T090000Z\r\nBEGIN: valarm \r\nX-WEIRD:1\r\nEND:valarm\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        0,
    );
    let text = write(&cal.events, None, 0);
    assert!(text.contains("\r\nBEGIN:valarm\r\nX-WEIRD:1\r\nEND:valarm\r\n"), "{text}");
}
