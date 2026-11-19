// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The id of a Teams meeting rides on the event in its own file and never in an invitation; its
//! conference is written with its label.

use super::*;
use crate::pim::teams::{TEAMS_CONFERENCE_LABEL, valid_teams_meeting_id};

const ID: &str = "MSpkYzE3Njc0Yy04MWQ5";

fn teams_event(id: &str) -> CalEvent {
    CalEvent {
        uid: "u1".into(),
        summary: "Budget".into(),
        start: 1_795_165_200,
        end: 1_795_168_800,
        teams_meeting_id: Some(id.into()),
        ..Default::default()
    }
}

/// A calendar text with this event line, with or without a method.
fn text_with(method: &str, line: &str) -> String {
    format!("BEGIN:VCALENDAR\r\n{method}BEGIN:VEVENT\r\nUID:j\r\nDTSTART:20261016T140000Z\r\n{line}END:VEVENT\r\nEND:VCALENDAR\r\n")
}

#[test]
fn the_teams_id_round_trips_in_the_events_own_file() {
    let text = write(&[teams_event(ID)], None, 0);
    assert!(text.contains(&format!("X-QUATRELLE-TEAMS-ID:{ID}\r\n")), "{text}");
    let back = parse(&text, 0);
    let e = back.events.first().expect("one event");
    assert_eq!(e.teams_meeting_id.as_deref(), Some(ID));
    assert!(e.extra.is_empty(), "{:?}", e.extra);
}

#[test]
fn an_invitation_never_carries_or_brings_the_teams_id() {
    for m in ["REQUEST", "CANCEL"] {
        let out = write(&[teams_event(ID)], Some(m), 0);
        assert!(!out.contains("X-QUATRELLE-TEAMS-ID"), "{m}: {out}");
    }
    let ics = text_with("METHOD:REQUEST\r\n", &format!("X-QUATRELLE-TEAMS-ID:{ID}\r\n"));
    let cal = parse(&ics, 0);
    let e = cal.events.first().expect("one event");
    assert_eq!(e.teams_meeting_id, None);
    assert!(e.extra.iter().all(|x| !x.contains("TEAMS-ID")), "{:?}", e.extra);
    let own = text_with("", &format!("X-QUATRELLE-TEAMS-ID:{ID}\r\n"));
    assert_eq!(parse(&own, 0).events.first().and_then(|e| e.teams_meeting_id.as_deref()), Some(ID), "a file without a method is the user's own");
}

#[test]
fn an_implausible_teams_id_is_dropped_when_read() {
    for junk in ["a b".to_string(), "x".repeat(513), "a?b".to_string()] {
        let cal = parse(&text_with("", &format!("X-QUATRELLE-TEAMS-ID:{junk}\r\n")), 0);
        let e = cal.events.first().expect("one event");
        assert_eq!(e.teams_meeting_id, None, "{junk}");
        assert!(e.extra.iter().all(|x| !x.contains("TEAMS-ID")), "{junk}: {:?}", e.extra);
    }
    assert!(valid_teams_meeting_id("a/b+c="));
    let cal = parse(&text_with("", "X-QUATRELLE-TEAMS-ID:a/b+c=\r\n"), 0);
    assert_eq!(cal.events.first().and_then(|e| e.teams_meeting_id.as_deref()), Some("a/b+c="));
}

#[test]
fn the_conference_of_a_teams_meeting_is_written_with_its_label() {
    let link = "https://teams.microsoft.com/meet/1";
    let e = CalEvent { conference: Conference::new(link, TEAMS_CONFERENCE_LABEL), ..teams_event(ID) };
    let text = write(&[e], None, 0);
    let unfolded = text.replace("\r\n ", "");
    assert!(unfolded.contains(&format!("CONFERENCE;VALUE=URI;FEATURE=AUDIO,VIDEO;LABEL=Microsoft Teams:{link}\r\n")), "{unfolded}");
}
