// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: on Exchange, a Show As or a Private chosen around an answer to an invitation,
//! before the next sync. The server gives the answer (AcceptItem…), and only the answer went: the
//! choice was let go with it, and the next read brought the meeting back as the server keeps it.

use quatrelle_model::ical::{Attendee, CalEvent, ShowAs};
use quatrelle_model::pim::{CalendarInfo, DavSync, StoredEvent};

use super::*;

const MINE: &str = "ews:calendar";

/// An Exchange account whose calendar holds Ana's meeting, unanswered, as the server keeps it.
fn invited() -> (Session, String, StoredEvent) {
    let mut s = Session::open(Some(temp("answering-then-choosing")), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s
        .run("account.add", &json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    let person = |email: &str, partstat: &str| Attendee {
        email: email.into(),
        name: email.into(),
        partstat: partstat.into(),
        rsvp: true,
        ..Default::default()
    };
    let server = StoredEvent {
        calendar: MINE.into(),
        href: "E1".into(),
        etag: "ck".into(),
        event: CalEvent {
            uid: "mtg-1".into(),
            summary: "Planning".into(),
            start: NOW + 86_400,
            end: NOW + 90_000,
            organizer: Some(person("ana@contoso.example", "ACCEPTED")),
            attendees: vec![person("zoe@contoso.example", "NEEDS-ACTION")],
            my_answer: "NEEDS-ACTION".into(),
            show_as: ShowAs::Tentative,
            ..Default::default()
        },
        ..Default::default()
    };
    s.apply_dav(
        &id,
        &DavSync {
            calendars: Some(vec![CalendarInfo { id: MINE.into(), name: "Calendar".into(), ..Default::default() }]),
            synced_calendars: vec![MINE.into()],
            events: vec![server.clone()],
            ..Default::default()
        },
    );
    s.take_jobs();
    (s, id, server)
}

/// Answered, then made private and free (or the other way round): once the server has given the
/// answer, the choice is still to go, and goes; the server's copy read meanwhile doesn't take its
/// place.
#[test]
fn a_choice_made_around_an_answer_goes_after_it() {
    for answer_first in [true, false] {
        let (mut s, id, server) = invited();
        let choose = |s: &mut Session| s.run("event.update", &json!({"uid": "mtg-1", "private": true, "showAs": "free", "send": false})).unwrap();
        if answer_first {
            s.run("event.respond", &json!({"uid": "mtg-1", "answer": "accepted"})).unwrap();
            choose(&mut s);
        } else {
            choose(&mut s);
            s.run("event.respond", &json!({"uid": "mtg-1", "answer": "accepted"})).unwrap();
        }
        s.take_jobs();
        // The sync gives the answer; the server's copy is read: accepted, Busy, public.
        let accepted = StoredEvent {
            event: CalEvent { my_answer: "ACCEPTED".into(), show_as: ShowAs::Busy, ..server.event.clone() },
            etag: "ck2".into(),
            ..server.clone()
        };
        let sync = DavSync {
            calendars: Some(vec![CalendarInfo { id: MINE.into(), name: "Calendar".into(), ..Default::default() }]),
            synced_calendars: vec![MINE.into()],
            events: vec![accepted.clone()],
            responded: vec![("mtg-1".into(), "ACCEPTED".into())],
            ..Default::default()
        };
        s.apply_dav(&id, &sync);
        let e = s.pim[&id].events.iter().find(|e| e.event.uid == "mtg-1").cloned().unwrap();
        assert!(e.respond.is_none(), "the answer given");
        assert!(e.event.private && e.event.show_as == ShowAs::Free, "the choice kept (answer first: {answer_first}): {e:?}");
        assert!(e.dirty, "and still to go (answer first: {answer_first})");
        assert!(s.take_jobs().iter().any(|j| matches!(j, Job::SyncDav { .. })), "sent at once");
        // The next sync reads the server's copy again before this one goes: it stays as chosen.
        s.apply_dav(&id, &DavSync { responded: Vec::new(), ..sync });
        let e = s.pim[&id].events.iter().find(|e| e.event.uid == "mtg-1").cloned().unwrap();
        assert!(e.event.private && e.event.show_as == ShowAs::Free && e.dirty, "{e:?}");
        done(s);
    }
}
