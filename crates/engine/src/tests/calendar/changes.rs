// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: the account's own Exchange calendar read by what changed in it (R21). What the
//! sync says is unchanged stays as it is, what it read takes its place, what it doesn't name goes,
//! and the state it ends at is saved with the appointments it read.

use quatrelle_model::ical::CalEvent;
use quatrelle_model::pim::{CalendarInfo, DavSync, Pim, StoredEvent};

use super::*;

const MINE: &str = "ews:calendar";

#[test]
fn a_calendar_read_by_its_changes_keeps_the_rest() {
    let store = temp("calendar-changes");
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s
        .run("account.add", &json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    let start = NOW + 86_400;
    let event = |href: &str, uid: &str, summary: &str| StoredEvent {
        calendar: MINE.into(),
        href: href.into(),
        etag: "ck".into(),
        event: CalEvent { uid: uid.into(), summary: summary.into(), start, end: start + 1800, ..Default::default() },
        ..Default::default()
    };
    let series = StoredEvent {
        event: CalEvent { rrule: Some("FREQ=WEEKLY".into()), ..event("S1", "standup-1", "Stand-up").event },
        ..event("S1", "standup-1", "Stand-up")
    };
    let moved = StoredEvent {
        href: "S1:occ3".into(),
        event: CalEvent { recurrence_id: Some(start + 14 * 86_400), rrule: None, start: start + 14 * 86_400 + 3600, ..series.event.clone() },
        ..series.clone()
    };
    let mut mine = event("D1", "dentist-1", "Dentist, moved here");
    mine.dirty = true;
    {
        let p = s.pim_mut(&id);
        p.calendars = vec![CalendarInfo { id: MINE.into(), name: "Calendar".into(), ctag: "state-1".into(), ..Default::default() }];
        p.events = vec![
            event("A1", "lunch-1", "Lunch"),
            event("B1", "review-1", "Review"),
            event("C1", "gone-1", "Gone"),
            series.clone(),
            moved.clone(),
            mine,
        ];
    }
    let changes = DavSync {
        calendars: Some(vec![CalendarInfo { id: MINE.into(), name: "Calendar".into(), ctag: "state-2".into(), ..Default::default() }]),
        synced_calendars: vec![MINE.into()],
        events: vec![event("B1", "review-1", "Review, later")],
        kept: ["A1", "S1", "S1:occ3", "D1"].map(String::from).to_vec(),
        partial: true,
        ..Default::default()
    };
    let shown = |s: &Session| {
        let mut list: Vec<(String, String, bool)> = s.pim[&id].events.iter().map(|e| (e.href.clone(), e.event.summary.clone(), e.dirty)).collect();
        list.sort();
        list
    };
    let expected = [
        ("A1", "Lunch", false),
        ("B1", "Review, later", false),
        ("D1", "Dentist, moved here", true),
        ("S1", "Stand-up", false),
        ("S1:occ3", "Stand-up", false),
    ]
    .map(|(h, t, d)| (h.to_string(), t.to_string(), d))
    .to_vec();
    s.apply_dav(&id, &changes);
    assert_eq!(shown(&s), expected, "the rest as it was, the change made here kept, the one deleted gone");
    let saved: Pim = store.load_account_doc(&id, crate::session::pim::PIM_DOC).unwrap();
    assert_eq!(saved.calendars.iter().find(|k| k.id == MINE).map(|k| k.ctag.as_str()), Some("state-2"), "the state saved with them");
    assert_eq!(saved.events.len(), 5);
    // The same changes again (a sync that started from the state before): nothing moves.
    s.apply_dav(&id, &changes);
    assert_eq!(shown(&s), expected);
    done(s);
}
