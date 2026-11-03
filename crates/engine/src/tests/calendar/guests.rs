// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: who counts as a guest to tell. An Exchange appointment the user made, with no
//! one invited, can come back from the server with the user as its one attendee (Exchange lists
//! the organizer): marked as a meeting to announce, each edit asked the server to send an update
//! to no one, which it refused, and the change stayed unsent, tried again at every sync (03/11).

use quatrelle_model::ical::{Attendee, CalEvent};
use quatrelle_model::pim::{CalendarInfo, StoredEvent};

use super::*;

const ME: &str = "zoe@contoso.example";

fn person(email: &str, partstat: &str) -> Attendee {
    Attendee { email: email.into(), name: "Zoe".into(), partstat: partstat.into(), ..Default::default() }
}

/// An Exchange account and, in its calendar as last read, an appointment the user organizes
/// with these attendees.
fn exchange_with(name: &str, attendees: Vec<Attendee>) -> (Session, String) {
    let mut s = Session::open(Some(temp(name)), quatrelle_model::dates::Clock::new(NOW, 0));
    let id =
        s.run("account.add", &json!({"email": ME, "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"})).unwrap()["account"]
            .as_str()
            .unwrap()
            .to_string();
    s.take_jobs();
    let p = s.pim_mut(&id);
    p.calendars = vec![CalendarInfo { id: "ews:calendar".into(), name: "Calendar".into(), ..Default::default() }];
    p.events.push(StoredEvent {
        calendar: "ews:calendar".into(),
        href: "I1".into(),
        etag: "ck".into(),
        event: CalEvent {
            uid: "test-1".into(),
            summary: "test".into(),
            start: NOW + 3600,
            end: NOW + 7200,
            organizer: Some(person(ME, "")),
            attendees,
            ..Default::default()
        },
        ..Default::default()
    });
    (s, id)
}

fn stored(s: &Session, id: &str) -> StoredEvent {
    s.pim[id].events.iter().find(|e| e.event.uid == "test-1").cloned().unwrap()
}

/// The user alone (as Exchange's `Organizer` answer reads), or someone with no address to
/// write to: nobody to tell. An edit is saved and sent without asking the server to send
/// anything; the calendar doesn't offer to send an update either.
#[test]
fn an_appointment_with_only_the_user_has_no_guests_to_tell() {
    for (name, attendees) in [
        ("ews-only-me", vec![person(ME, "ACCEPTED")]),
        ("ews-only-me-caps", vec![person("Zoe@Contoso.example", "ACCEPTED")]),
        ("ews-no-address", vec![person(ME, "ACCEPTED"), person("Ana Lima", "NEEDS-ACTION")]),
    ] {
        let (mut s, id) = exchange_with(name, attendees);
        s.run("event.update", &json!({"uid": "test-1", "summary": "x"})).unwrap();
        let e = stored(&s, &id);
        assert!(e.dirty && !e.notify, "{name}: saved, but no one to tell: {e:?}");
        assert_eq!(s.tell_guests(&id, "test-1", None, true).unwrap(), "none", "{name}");
        assert!(!stored(&s, &id).notify, "{name}");
        let shown = s.agenda(NOW, NOW + 86_400).into_iter().find(|i| i.uid == "test-1").unwrap();
        assert!(!shown.organized && shown.movable, "{name}: no update to offer");
        assert!(!s.organizes(&e.event), "{name}");
        done(s);
    }
}

/// A real guest: the server is asked to tell them, as before.
#[test]
fn a_meeting_with_a_guest_is_still_announced() {
    let (mut s, id) = exchange_with("ews-real-guest", vec![person(ME, "ACCEPTED"), person("ana@contoso.example", "NEEDS-ACTION")]);
    s.run("event.update", &json!({"uid": "test-1", "summary": "x"})).unwrap();
    let e = stored(&s, &id);
    assert!(e.dirty && e.notify, "{e:?}");
    let shown = s.agenda(NOW, NOW + 86_400).into_iter().find(|i| i.uid == "test-1").unwrap();
    assert!(shown.organized);
    done(s);
}
