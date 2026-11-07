// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The appointments the export makes of events, looked at before they are written: a series'
//! pattern read back by the import for any rule Outlook's pattern says.

use super::*;
use crate::data_files::pst_export::calendar::{self, Owner};
use crate::data_files::pst_export::pim::{Item, Key, Value};

/// A named property of an item, by its number.
fn lid_of(item: &Item, id: u32) -> Option<&Value> {
    item.props.iter().find_map(|(k, v)| matches!(k, Key::Lid(_, i) if *i == id).then_some(v))
}

/// A series no pattern says goes in one occurrence at a time: its files on the first only, not
/// copied into each (a 100 kB file on 2,000 hours was 200 MB held at once, and in the file).
#[test]
fn a_series_one_by_one_carries_its_files_once() {
    let data = vec![7u8; 100_000];
    let e = CalEvent {
        uid: "render@studio.example".into(),
        summary: "Render check".into(),
        start: 1_794_000_000,
        end: 1_794_000_600,
        rrule: Some("FREQ=HOURLY;COUNT=2000".into()),
        extra: vec![crate::pst_pim::event_file("cut.bin", "application/octet-stream", &data), "X-KEPT:1".into()],
        ..Default::default()
    };
    let items = calendar::items(&[&e], 0, &Owner::default());
    assert_eq!(items.len(), 2000);
    assert!(items.iter().all(|(_, i)| i.occurrence));
    let held: usize = items.iter().flat_map(|(_, i)| i.files.iter().map(|f| f.data.len())).sum();
    assert_eq!(held, data.len());
    assert_eq!(items[0].1.files.len(), 1);
}

/// Changed occurrences of a series written one occurrence at a time that no start written takes
/// (one the rule doesn't make, or of a rule that makes none) go in on their own, as the pattern's
/// leftovers do; one called off adds nothing.
#[test]
fn changed_occurrences_no_start_takes_go_in_on_their_own() {
    let e = CalEvent {
        uid: "check@studio.example".into(),
        summary: "Check".into(),
        start: 1_794_000_000,
        end: 1_794_000_600,
        rrule: Some("FREQ=HOURLY;COUNT=3".into()),
        ..Default::default()
    };
    let changed = |rid: i64, summary: &str, status: &str| CalEvent {
        recurrence_id: Some(rid),
        start: rid + 600,
        end: rid + 1200,
        summary: summary.into(),
        status: status.into(),
        rrule: None,
        ..e.clone()
    };
    let moved = changed(e.start + 1800, "Moved", "");
    let taken = changed(e.start + 3600, "Later", "");
    let off = changed(e.start + 5400, "Off", "CANCELLED");
    let items = calendar::items(&[&e, &moved, &taken, &off], 0, &Owner::default());
    let titles: Vec<(&str, &str)> = items.iter().map(|(u, i)| (u.as_str(), i.subject.as_str())).collect();
    let uid = |t: i64| format!("check@studio.example/{t}");
    assert_eq!(titles.len(), 4, "{titles:?}");
    assert!(titles.contains(&(uid(e.start + 3600).as_str(), "Later")), "{titles:?}");
    assert!(titles.contains(&(uid(e.start + 1800).as_str(), "Moved")), "{titles:?}");
    assert!(!titles.iter().any(|(_, t)| *t == "Off"));
    // A rule that makes no day: the event as it starts, and its changed occurrence.
    let never = CalEvent { rrule: Some("FREQ=YEARLY;BYMONTH=2;BYMONTHDAY=30".into()), ..e.clone() };
    let items = calendar::items(&[&never, &moved], 0, &Owner::default());
    let titles: Vec<&str> = items.iter().map(|(_, i)| i.subject.as_str()).collect();
    assert_eq!(titles, ["Check", "Moved"]);
}

/// An all-day series as Outlook keeps one, from its local midnights (New York's here), read back
/// by the import keeps its days and its deleted days.
#[test]
fn an_all_day_series_from_local_midnights_keeps_its_deleted_days() {
    let ny = Zone::named("America/New_York").unwrap();
    let day = days_from_civil(2026, 11, 20) * 86_400;
    let e = CalEvent {
        uid: "fair@studio.example".into(),
        summary: "Fair".into(),
        start: day,
        end: day + 86_400,
        all_day: true,
        rrule: Some("FREQ=DAILY;COUNT=4".into()),
        exdates: vec![day + 86_400],
        ..Default::default()
    };
    let blob = written(&e).unwrap();
    let x = crate::pst_pim::Extra {
        start: Some(ny.to_utc(day)),
        end: Some(ny.to_utc(day + 86_400)),
        all_day: true,
        recurrence: blob,
        ..Default::default()
    };
    let back = crate::pst_pim::event_of(&x, "Fair", "", &EmailAddress::default(), &[], "fair@studio.example".into()).unwrap();
    assert_eq!((back.start, back.end, back.all_day), (day, day + 86_400, true));
    assert_eq!(back.exdates, [day + 86_400]);
    assert_eq!(ical::starts(&back, i64::MAX), ical::starts(&e, i64::MAX));
}

fn int_of(item: &Item, id: u32) -> Option<i32> {
    match lid_of(item, id) {
        Some(Value::Int(n)) => Some(*n),
        _ => None,
    }
}

/// A meeting someone else organizes is one the user received (asfReceived), with the user's own
/// answer; one the account's address organizes is the user's own (respOrganized); one whose
/// server gave the user's answer is received whoever's addresses are known.
#[test]
fn a_meeting_someone_else_organizes_is_received() {
    let meeting = CalEvent {
        uid: "budget@studio.example".into(),
        summary: "Budget".into(),
        start: 1_793_000_000,
        end: 1_793_003_600,
        organizer: Some(person("riley@studio.example", "Riley", "", false)),
        attendees: vec![person("ana@studio.example", "Ana", "ACCEPTED", false), person("ben@studio.example", "Ben", "NEEDS-ACTION", true)],
        ..Default::default()
    };
    let state = |owner: &Owner, e: &CalEvent| {
        let items = calendar::items(&[e], 0, owner);
        (int_of(&items[0].1, 0x8217), int_of(&items[0].1, 0x8218))
    };
    let me = |a: &str| Owner { me: vec![a.into()], ..Default::default() };
    assert_eq!(state(&me("ANA@studio.example"), &meeting), (Some(3), Some(3)));
    assert_eq!(state(&me("ben@studio.example"), &meeting), (Some(3), Some(5)));
    // Not among the guests either: not answered yet.
    assert_eq!(state(&me("zoe@studio.example"), &meeting), (Some(3), Some(5)));
    assert_eq!(state(&me("riley@studio.example"), &meeting), (Some(1), Some(1)));
    // Nothing known of the account: as the user's own, unless the server gave the user's answer.
    assert_eq!(state(&Owner::default(), &meeting), (Some(1), Some(1)));
    let answered = CalEvent { my_answer: "TENTATIVE".into(), ..meeting.clone() };
    assert_eq!(state(&Owner::default(), &answered), (Some(3), Some(2)));
    // Not a meeting: neither.
    let alone = CalEvent { attendees: Vec::new(), ..meeting.clone() };
    assert_eq!(state(&me("ana@studio.example"), &alone), (Some(0), None));
}

/// An all-day event starts and ends at this computer's midnights, in its zone, as Outlook keeps
/// one; read back by the import, it and a series of them keep their days and deleted days.
#[test]
fn all_day_at_this_computers_midnights() {
    let ny = Zone::named("America/New_York").unwrap();
    let owner = Owner { zone: ny, ..Default::default() };
    let day = days_from_civil(2026, 11, 20) * 86_400;
    let e = CalEvent {
        uid: "fair@studio.example".into(),
        summary: "Fair".into(),
        start: day,
        end: day + 86_400,
        all_day: true,
        rrule: Some("FREQ=DAILY;COUNT=4".into()),
        exdates: vec![day + 86_400],
        ..Default::default()
    };
    let items = calendar::items(&[&e], 0, &owner);
    let item = &items[0].1;
    let time = |id| match lid_of(item, id) {
        Some(Value::Time(t)) => *t,
        _ => panic!("{id:x}"),
    };
    assert_eq!(time(0x820D), item::filetime(ny.to_utc(day)));
    assert_eq!(time(0x820E), item::filetime(ny.to_utc(day + 86_400)));
    assert_eq!(time(0x8235), item::filetime(ny.to_utc(day)), "the series' first day, from its local midnight");
    assert!(matches!(lid_of(item, 0x8234), Some(Value::Text(t)) if t == "America/New_York"));
    let Some(Value::Bin(blob)) = lid_of(item, 0x8216) else { panic!("a pattern") };
    // Read back as Outlook's own would be.
    let x = crate::pst_pim::Extra {
        start: Some(ny.to_utc(day)),
        end: Some(ny.to_utc(day + 86_400)),
        all_day: true,
        recurrence: blob.clone(),
        ..Default::default()
    };
    let back = crate::pst_pim::event_of(&x, "Fair", "", &EmailAddress::default(), &[], "fair@studio.example".into()).unwrap();
    assert_eq!((back.start, back.end, back.all_day), (day, day + 86_400, true));
    assert_eq!(back.exdates, [day + 86_400]);
    assert_eq!(ical::starts(&back, i64::MAX), ical::starts(&e, i64::MAX));
    // With no zone known, UTC as before.
    let utc = calendar::items(&[&e], 0, &Owner::default());
    assert!(matches!(lid_of(&utc[0].1, 0x820D), Some(Value::Time(t)) if *t == item::filetime(day)));
}

/// A rule at set hours and one with added dates go in one occurrence at a time, at the hours and
/// dates they make (three a day for three hours, not one every hour).
#[test]
fn hours_and_added_dates_go_in_as_their_occurrences() {
    let day = days_from_civil(2026, 11, 2) * 86_400;
    let hours = CalEvent {
        uid: "h@studio.example".into(),
        start: day + 9 * 3600,
        end: day + 9 * 3600 + 600,
        rrule: Some("FREQ=HOURLY;BYHOUR=9,10,11;COUNT=6".into()),
        ..Default::default()
    };
    let starts = |e: &CalEvent| {
        calendar::items(&[e], 0, &Owner::default())
            .iter()
            .map(|(_, i)| match lid_of(i, 0x820D) {
                Some(Value::Time(t)) => (*t - item::filetime(day)) / 36_000_000_000,
                _ => -1,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(starts(&hours), [9, 10, 11, 33, 34, 35]);
    let added = CalEvent { uid: "r@studio.example".into(), rrule: None, rdates: vec![day + 30 * 3600, day + 50 * 3600], ..hours.clone() };
    assert_eq!(starts(&added), [9, 30, 50]);
}

/// The pattern the export writes for `e` (none when it goes in one occurrence at a time).
fn written(e: &CalEvent) -> Option<Vec<u8>> {
    calendar::series(e, &[], &Owner::default(), false)?.into_iter().find_map(|(k, v)| match (k, v) {
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
