// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Recurrence rules that match nothing end, and one with a frequency RFC 5545 does not have
//! is no rule: the event still occurs at its start.

use super::*;

/// A rule nothing ever matches ends too, window or none (it went on for ever before: the
/// calendar froze on an invitation that had one).
#[test]
fn a_rule_nothing_matches_ends() {
    let mk = |rrule: &str, start: i64| CalEvent { uid: "r".into(), start, end: start + 3600, rrule: Some(rrule.into()), ..Default::default() };
    let mon = at(2026, 10, 5, 9, 0); // a Monday
    for never in
        ["FREQ=DAILY;BYSETPOS=2", "FREQ=WEEKLY;BYDAY=MO;BYSETPOS=2", "FREQ=DAILY;BYMONTH=2;BYMONTHDAY=30", "FREQ=DAILY;INTERVAL=1000000;BYSETPOS=5"]
    {
        assert_eq!(starts(&mk(never, mon), i64::MAX), Vec::<i64>::new(), "{never}");
        assert_eq!(starts(&mk(never, mon), mon + 400 * 86_400), Vec::<i64>::new(), "{never}");
    }
}

/// A rule with a frequency RFC 5545 doesn't have (`FREQ=FORTNIGHTLY`, typed as a rule of its own)
/// is no rule: the event still occurs at its start, as one with no `FREQ` at all does, rather
/// than not at all.
#[test]
fn an_unknown_frequency_is_no_rule() {
    let start = at(2026, 10, 5, 9, 0);
    for rule in ["FREQ=FORTNIGHTLY", "FREQ=;COUNT=3", "COUNT=3", "FREQ=DAILYX;INTERVAL=2"] {
        let e = CalEvent { uid: "u".into(), start, end: start + 1800, rrule: Some(rule.into()), ..Default::default() };
        assert_eq!(starts(&e, i64::MAX), vec![start], "{rule}");
        assert_eq!(occurrences(std::slice::from_ref(&e), start - 86_400, start + 86_400).len(), 1, "{rule}");
    }
    let daily = CalEvent { uid: "d".into(), start, end: start + 1800, rrule: Some("freq=daily;count=3".into()), ..Default::default() };
    assert_eq!(starts(&daily, i64::MAX).len(), 3, "a known one in any case");
}
