// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
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

/// The card asks for the invitation on the first frame of every message opened: one with no
/// calendar is looked at as it was parsed to be shown, and isn't read again (it was read and
/// parsed a second time on the window's thread, 104 to 149 ms for a big one upstream). One with
/// an invitation is read once more for its calendar's bytes.
#[test]
fn a_message_without_an_invitation_is_not_read_again_for_one() {
    let mut s = sample_session("invite-no-read");
    let store = s.store.clone().unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    let plain = s.list.first().map(|m| m.key.clone()).unwrap();
    s.run("list.open", &json!({"key": plain.to_id()})).unwrap();
    assert!(s.reading.as_ref().is_some_and(|r| r.parsed.is_some()));
    let reads = store.reads();
    assert!(s.invite().is_none());
    assert_eq!(store.reads(), reads, "nothing read again");
    let event = CalEvent {
        uid: "card@soundlab.example".into(),
        summary: "Card review".into(),
        start: NOW + 86_400,
        end: NOW + 86_400 + 3600,
        organizer: Some(Attendee { email: ANA.1.into(), name: ANA.0.into(), ..Default::default() }),
        attendees: vec![me("NEEDS-ACTION")],
        ..Default::default()
    };
    deliver(&mut s, 1, ANA, "REQUEST", &[event], NOW);
    let reads = store.reads();
    assert_eq!(s.invite().unwrap().event.summary, "Card review");
    assert_eq!(store.reads(), reads + 1, "read once for the calendar's bytes");
    done(s);
}
