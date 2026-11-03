// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: an appointment changed here and deleted on the server meanwhile. Its change was
//! refused at every sync ("not on the server any more"), the appointment kept here for good
//! (the owner's "test", 03/11). The server wins: it goes here too, said once.

use quatrelle_model::ical::CalEvent;
use quatrelle_model::pim::{CalendarInfo, DavSync, StoredEvent};

use super::*;

const MINE: &str = "ews:calendar";

#[test]
fn an_appointment_deleted_on_the_server_goes_here_too_said_once() {
    let mut s = Session::open(Some(temp("gone-on-server")), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s
        .run("account.add", &json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    let start = 1_792_400_400;
    let event = |href: &str, uid: &str, summary: &str| StoredEvent {
        calendar: MINE.into(),
        href: href.into(),
        etag: "ck".into(),
        event: CalEvent { uid: uid.into(), summary: summary.into(), start, end: start + 1800, ..Default::default() },
        ..Default::default()
    };
    let kept = event("KEPT1", "kept-1", "Lunch");
    {
        let p = s.pim_mut(&id);
        p.calendars = vec![CalendarInfo { id: MINE.into(), name: "Calendar".into(), ..Default::default() }];
        // "test", moved here; a series and one of its occurrences changed here; one left alone.
        let mut test = event("GONE1", "test-1", "test");
        test.dirty = true;
        let series = StoredEvent {
            event: CalEvent { rrule: Some("FREQ=WEEKLY".into()), ..event("SERIES1", "standup-1", "Stand-up").event },
            ..event("SERIES1", "standup-1", "Stand-up")
        };
        let occurrence = StoredEvent {
            href: String::new(),
            etag: String::new(),
            event: CalEvent {
                recurrence_id: Some(start + 7 * 86_400),
                start: start + 7 * 86_400 + 3600,
                end: start + 7 * 86_400 + 5400,
                ..series.event.clone()
            },
            dirty: true,
            ..series.clone()
        };
        p.events = vec![test, series, occurrence, kept.clone()];
    }
    let server = |gone: &[&str]| DavSync {
        calendars: Some(vec![CalendarInfo { id: MINE.into(), name: "Calendar".into(), ..Default::default() }]),
        synced_calendars: vec![MINE.into()],
        events: vec![kept.clone()],
        gone_events: gone.iter().map(|g| (*g).to_string()).collect(),
        ..Default::default()
    };
    s.apply_dav(&id, &server(&["GONE1", "SERIES1"]));
    let uids: Vec<&str> = s.pim[&id].events.iter().map(|e| e.event.uid.as_str()).collect();
    assert_eq!(uids, ["kept-1"], "gone here too, the series with its occurrence");
    let said =
        |s: &Session| s.notices.iter().filter(|n| n.text.contains("deleted on the server meanwhile")).map(|n| n.text.clone()).collect::<Vec<_>>();
    assert_eq!(said(&s).len(), 2, "one message per appointment: {:?}", said(&s));
    assert!(said(&s).iter().any(|t| t.starts_with("test: ")), "{:?}", said(&s));
    assert!(!s.notices.iter().any(|n| n.text.contains("its series is no longer on the server")), "the occurrence went with its series, unsaid");
    // The next sync: nothing comes back, nothing is said again.
    s.apply_dav(&id, &server(&[]));
    assert_eq!(s.pim[&id].events.len(), 1);
    assert_eq!(said(&s).len(), 2, "said once");
    done(s);
}
