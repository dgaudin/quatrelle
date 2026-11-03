// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: answering an invitation sets how the meeting shows in the calendar, and the
//! answer sent carries no alarm (RFC 5546).

use quatrelle_model::ical::ShowAs;

use super::*;

fn invitation(uid: &str, summary: &str) -> CalEvent {
    CalEvent {
        uid: uid.into(),
        summary: summary.into(),
        start: NOW + 86_400,
        end: NOW + 86_400 + 3600,
        organizer: Some(Attendee { email: ANA.1.into(), name: ANA.0.into(), ..Default::default() }),
        attendees: vec![me("NEEDS-ACTION")],
        show_as: ShowAs::Tentative,
        extra: vec!["X-MICROSOFT-CDO-INTENDEDSTATUS:BUSY".into()],
        ..Default::default()
    }
}

fn alarm() -> Vec<String> {
    ["BEGIN:VALARM", "ACTION:AUDIO", "TRIGGER;RELATED=END:PT5M", "END:VALARM"].map(String::from).to_vec()
}

#[test]
fn an_outlook_invitation_accepted_shows_as_busy() {
    let mut s = sample_session("showing-busy");
    deliver(&mut s, 1, ANA, "REQUEST", &[invitation("busy@soundlab.example", "Mix review")], NOW);
    s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    let (_, stored) = s.find_event("busy@soundlab.example").unwrap();
    assert_eq!(stored.event.show_as, ShowAs::Busy);
    let ics = quatrelle_model::ical::write(std::slice::from_ref(&stored.event), None, NOW);
    assert!(!ics.contains("X-MICROSOFT-CDO-BUSYSTATUS"), "busy is the default: {ics}");
    assert_eq!(ics.matches("X-MICROSOFT-CDO-INTENDEDSTATUS").count(), 1, "{ics}");

    deliver(&mut s, 2, ANA, "REQUEST", &[invitation("maybe@soundlab.example", "Lunch")], NOW);
    s.run("invite.respond", &json!({"answer": "tentative"})).unwrap();
    assert_eq!(s.find_event("maybe@soundlab.example").unwrap().1.event.show_as, ShowAs::Tentative);
}

#[test]
fn the_reply_carries_no_alarm() {
    let mut s = sample_session("showing-no-alarm");
    let e = CalEvent { reminder: Some(15), extra: alarm(), ..invitation("alarm@soundlab.example", "Mix review") };
    deliver(&mut s, 1, ANA, "REQUEST", &[e], NOW);
    s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    let (method, raw) = sent_calendar(&s, "Sent", "Accepted: Mix review");
    assert_eq!(method, "REPLY");
    assert!(!raw.contains("VALARM") && !raw.contains("TRIGGER"), "{raw}");
    assert_eq!(s.find_event("alarm@soundlab.example").unwrap().1.event.reminder, Some(15));
}

#[test]
fn an_alarm_alone_leaves_the_reply_whole() {
    let mut s = sample_session("showing-alarm-alone");
    let e = CalEvent { extra: alarm(), ..invitation("alone@soundlab.example", "Mix review") };
    deliver(&mut s, 1, ANA, "REQUEST", &[e], NOW);
    s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    let (_, raw) = sent_calendar(&s, "Sent", "Accepted: Mix review");
    assert!(!raw.contains("BEGIN:VALARM") && !raw.contains("END:VALARM"), "{raw}");
}
