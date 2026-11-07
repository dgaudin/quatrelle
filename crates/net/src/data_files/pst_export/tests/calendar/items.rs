// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The appointments the export makes of events, looked at before they are written: a series'
//! pattern read back by the import for any rule Outlook's pattern says.

use super::*;
use crate::data_files::pst_export::calendar;
use crate::data_files::pst_export::pim::{Key, Value};

/// The pattern the export writes for `e` (none when it goes in one occurrence at a time).
fn written(e: &CalEvent) -> Option<Vec<u8>> {
    calendar::series(e, &[])?.into_iter().find_map(|(k, v)| match (k, v) {
        (Key::Lid(_, 0x8216), Value::Bin(b)) => Some(b),
        _ => None,
    })
}

/// A rule of a kind Outlook's pattern says, as RFC 5545 writes it.
fn rule_strategy() -> impl proptest::strategy::Strategy<Value = String> {
    use proptest::prelude::*;
    let days = proptest::sample::subsequence(vec!["SU", "MO", "TU", "WE", "TH", "FR", "SA"], 1..=7).prop_map(|d| d.join(","));
    let nth = prop_oneof![Just(-1i32), 1..=4i32];
    let weekday = proptest::sample::select(vec!["SU", "MO", "TU", "WE", "TH", "FR", "SA"]);
    let end = prop_oneof![Just(String::new()), (1u32..40).prop_map(|n| format!(";COUNT={n}")), (1i64..700).prop_map(|d| format!(";UNTIL=+{d}"))];
    let kind = prop_oneof![
        (1u32..5).prop_map(|i| format!("FREQ=DAILY;INTERVAL={i}")),
        (1u32..4, days, proptest::sample::select(vec!["", ";WKST=MO", ";WKST=SU"]))
            .prop_map(|(i, d, w)| format!("FREQ=WEEKLY;INTERVAL={i};BYDAY={d}{w}")),
        (1u32..4, prop_oneof![Just(-1i32), 1..=28i32]).prop_map(|(i, d)| format!("FREQ=MONTHLY;INTERVAL={i};BYMONTHDAY={d}")),
        (1u32..4, nth.clone(), weekday.clone()).prop_map(|(i, n, w)| format!("FREQ=MONTHLY;INTERVAL={i};BYDAY={n}{w}")),
        (1u32..3).prop_map(|i| format!("FREQ=YEARLY;INTERVAL={i}")),
        (1u32..3, nth, weekday).prop_map(|(i, n, w)| format!("FREQ=YEARLY;INTERVAL={i};BYMONTH=M;BYDAY={n}{w}")),
    ];
    (kind, end).prop_map(|(k, e)| format!("{k}{e}"))
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(400))]
    /// Any rule Outlook's pattern says, in a zone with summer time or none, all day or not, with
    /// occurrences deleted: written as the export writes it and read back by the import
    /// (`pst_pim::recur`), the series has the same occurrences over three years.
    #[test]
    fn a_rule_written_and_read_back_keeps_its_occurrences(
        rule in rule_strategy(),
        day in 0i64..1000,
        minute in 0i64..(24 * 4),
        zone in proptest::sample::select(vec!["", "America/New_York", "Europe/Berlin", "Australia/Sydney", "Asia/Kolkata"]),
        all_day in proptest::prelude::any::<bool>(),
        deleted in proptest::collection::vec(0usize..40, 0..4),
    ) {
        let first = days_from_civil(2025, 1, 1) + day;
        let (_, month, _) = quatrelle_model::dates::civil_from_days(first);
        let rule = rule.replace("BYMONTH=M", &format!("BYMONTH={month}"));
        // An end on a day after the first.
        let rule = match rule.split_once(";UNTIL=+") {
            Some((head, days)) => {
                let (y, m, d) = quatrelle_model::dates::civil_from_days(first + days.parse::<i64>().unwrap());
                format!("{head};UNTIL={y:04}{m:02}{d:02}T235959Z")
            }
            None => rule,
        };
        let (start, tzid) = if all_day || zone.is_empty() {
            (first * 86_400 + if all_day { 0 } else { minute * 900 }, String::new())
        } else {
            (Zone::named(zone).unwrap().to_utc(first * 86_400 + minute * 900), zone.to_string())
        };
        let mut e = CalEvent { uid: "p".into(), start, end: start + if all_day { 86_400 } else { 1800 }, all_day, tzid, rrule: Some(rule.clone()), ..Default::default() };
        let all = ical::starts(&e, i64::MAX);
        e.exdates = deleted.iter().filter_map(|i| all.get(*i).copied()).collect();
        // A day Outlook keeps every month (the 1st to the 28th, or the last) and the rules above
        // are all said by the pattern.
        let blob = written(&e);
        proptest::prop_assert!(blob.is_some(), "{} at {}", rule, e.start);
        let blob = blob.unwrap();
        let tz = if e.all_day { String::new() } else { e.tzid.clone() };
        let (back_rule, exdates) = crate::pst_pim::recurrence_in(&blob, e.start, &tz).unwrap();
        let back = CalEvent { rrule: Some(back_rule.clone()), exdates, ..e.clone() };
        let window = |x: &CalEvent| ical::occurrences(std::slice::from_ref(x), e.start, e.start + 3 * 366 * 86_400).iter().map(|o| o.start).collect::<Vec<_>>();
        proptest::prop_assert_eq!(window(&back), window(&e), "{} → {}", rule, back_rule);
    }
}
