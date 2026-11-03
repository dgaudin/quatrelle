// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: `SEQUENCE` is the organizer's revision number (RFC 5545 §3.8.7.4): a change the
//! user makes to a copy of someone else's meeting leaves it alone, so the organizer's next update
//! isn't taken for an older one.

use super::*;

fn ana_meeting(sequence: u32, start: i64) -> CalEvent {
    CalEvent {
        uid: "seq@soundlab.example".into(),
        summary: "Mix review".into(),
        start,
        end: start + 3600,
        organizer: Some(Attendee { email: ANA.1.into(), name: ANA.0.into(), ..Default::default() }),
        attendees: vec![me("NEEDS-ACTION")],
        sequence,
        ..Default::default()
    }
}

#[test]
fn changing_my_copy_of_anas_meeting_keeps_her_sequence_and_her_next_update() {
    let mut s = sample_session("sequence-attendee");
    let uid = "seq@soundlab.example";
    deliver(&mut s, 1, ANA, "REQUEST", &[ana_meeting(1, NOW + 86_400)], NOW);
    s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    assert_eq!(s.find_event(uid).unwrap().1.event.sequence, 1);

    s.run("event.update", &json!({"uid": uid, "showAs": "free", "send": false})).unwrap();
    s.run("event.update", &json!({"uid": uid, "showAs": "busy", "send": false})).unwrap();
    s.run("event.update", &json!({"uid": uid, "categories": ["Work"], "send": false})).unwrap();
    assert_eq!(s.find_event(uid).unwrap().1.event.sequence, 1, "the organizer's number");

    let moved = NOW + 2 * 86_400;
    deliver(&mut s, 2, ANA, "REQUEST", &[ana_meeting(2, moved)], NOW + 60);
    assert_eq!(s.invite().unwrap().problem, None, "not out of date");
    s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    let stored = s.find_event(uid).unwrap().1.event.clone();
    assert_eq!((stored.start, stored.sequence), (moved, 2));
}

#[test]
fn changing_my_own_meeting_raises_its_sequence() {
    let mut s = sample_session("sequence-organizer");
    let made = s
        .run("event.create", &json!({"summary": "Standup", "start": "2026-12-01T10:00", "attendees": "ana@soundlab.example", "account": "sample"}))
        .unwrap();
    let uid = made["uid"].as_str().unwrap().to_string();
    let before = s.find_event(&uid).unwrap().1.event.sequence;
    s.run("event.update", &json!({"uid": uid, "location": "Room 2", "send": false})).unwrap();
    assert_eq!(s.find_event(&uid).unwrap().1.event.sequence, before + 1);
}
