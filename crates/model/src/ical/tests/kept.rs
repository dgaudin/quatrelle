// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The lines an event keeps as written (`extra`) are capped inside its components too, and its
//! reminder is kept within them, where every reader of them looks.

use super::*;

fn one(lines: &str) -> CalEvent {
    let text = format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:x\r\nDTSTART:20261022T090000Z\r\nDTEND:20261022T100000Z\r\n{lines}END:VEVENT\r\nEND:VCALENDAR\r\n"
    );
    parse(&text, 0).events.into_iter().next().unwrap_or_default()
}

const ALARM: &str = "BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nACKNOWLEDGED:20261022T084600Z\r\nEND:VALARM\r\n";

/// A component of 100,000 lines, and one nested 10,000 deep: each went into the event's kept
/// lines whole, saved with the calendar and read through each time it was drawn.
#[test]
fn a_components_lines_are_capped() {
    let big = format!("BEGIN:X-BIG\r\n{}END:X-BIG\r\n", "X-LINE:junk\r\n".repeat(100_000));
    let e = one(&big);
    assert!(e.extra.len() <= 400, "{}", e.extra.len());
    assert_eq!(e.extra.first().map(String::as_str), Some("BEGIN:X-BIG"));
    assert_eq!(e.extra.last().map(String::as_str), Some("END:X-BIG"));
    let deep = format!("{}{}", "BEGIN:X-IN\r\n".repeat(10_000), "END:X-IN\r\n".repeat(10_000));
    let e = one(&deep);
    assert!(e.extra.len() <= 800, "{}", e.extra.len());
    let begun = e.extra.iter().filter(|l| l.starts_with("BEGIN:")).count();
    assert_eq!(begun, e.extra.iter().filter(|l| l.starts_with("END:")).count());
    // What comes after them is still read.
    let e = one(&format!("{big}{deep}SUMMARY:Board\r\n"));
    assert_eq!(e.summary, "Board");
}

/// An alarm after a long component, or after hundreds of properties, is still the event's
/// reminder: the reminder was read from the first 400 lines only, while dismissing it rewrote
/// every alarm.
#[test]
fn an_alarm_after_many_lines_is_the_reminder() {
    let big = format!("BEGIN:X-BIG\r\n{}END:X-BIG\r\n", "X-LINE:junk\r\n".repeat(500));
    // After the long component, a fixed-time alarm (`TRIGGER;VALUE=DATE-TIME:…`), kept whole in
    // `extra`: its 15 minutes before the start and its `ACKNOWLEDGED` are read from there.
    let fixed = "BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER;VALUE=DATE-TIME:20261022T084500Z\r\nACKNOWLEDGED:20261022T084600Z\r\nEND:VALARM\r\n";
    let e = one(&format!("{big}{fixed}"));
    assert_eq!(reminder_minutes(&e), Some(15), "{}", e.extra.len());
    assert_eq!(acknowledged(&e), Some(1_792_658_760));
    for before in [big, "X-PROP:1\r\n".repeat(300)] {
        let e = one(&format!("{before}{ALARM}"));
        assert_eq!(reminder_minutes(&e), Some(15), "{}", e.extra.len());
        assert_eq!(acknowledged(&e), Some(1_792_658_760));
        let mut set = e.clone();
        set_reminder(&mut set, Some(30));
        assert_eq!(reminder_minutes(&set), Some(30));
    }
}

/// An alarm Quatrelle can't read is kept whole or left whole out: one of 450 lines leaves
/// neither its `BEGIN:VALARM` nor any of its lines in `extra`, and the event is still read. A
/// small one still goes in, whole.
#[test]
fn an_unreadable_alarm_is_kept_whole_or_left_whole_out() {
    let mut big = String::from("BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:JUNK\r\n");
    for i in 0..446 {
        big.push_str(&format!("X-A{i}:junk\r\n"));
    }
    big.push_str("END:VALARM\r\n");
    assert!(big.lines().count() >= 450, "{}", big.lines().count());
    let e = one(&format!("{big}SUMMARY:Board\r\n"));
    assert_eq!(e.summary, "Board", "the event is still read");
    assert_eq!(e.reminder, None);
    assert!(!e.extra.iter().any(|l| l.contains("VALARM") || l.starts_with("X-A")), "left whole out: {:?}", e.extra);
    let small = "BEGIN:VALARM\r\nTRIGGER:JUNK\r\nX-A:1\r\nEND:VALARM\r\n";
    let e = one(small);
    assert_eq!(e.reminder, None);
    assert_eq!(e.extra.iter().filter(|l| l == &"BEGIN:VALARM").count(), 1, "kept whole: {:?}", e.extra);
    assert_eq!(e.extra.iter().filter(|l| l.starts_with("X-A")).count(), 1);
}
