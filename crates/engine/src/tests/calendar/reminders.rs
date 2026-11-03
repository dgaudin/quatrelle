// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: dismissing the reminder of an appointment takes it off the appointment, as Outlook
//! does (Exchange's `ReminderIsSet`), so moving it doesn't bring it back; a series keeps its
//! reminder and is answered per occurrence; a reminder is personal, so no guest is told (03/11).

use quatrelle_model::ical::{Attendee, CalEvent};
use quatrelle_model::pim::{CalendarInfo, StoredEvent};

use super::*;

const ME: &str = "zoe@contoso.example";

fn person(email: &str) -> Attendee {
    Attendee { email: email.into(), name: "P".into(), ..Default::default() }
}

/// An Exchange account whose calendar holds, as last read, a meeting the user organizes.
fn exchange(name: &str, attendees: Vec<Attendee>, rrule: Option<&str>) -> (Session, String) {
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
            uid: "m-1".into(),
            summary: "Sync".into(),
            start: NOW + 3600,
            end: NOW + 7200,
            reminder: Some(15),
            organizer: Some(person(ME)),
            rrule: rrule.map(str::to_string),
            attendees,
            ..Default::default()
        },
        ..Default::default()
    });
    (s, id)
}

fn stored(s: &Session, id: &str) -> StoredEvent {
    s.pim[id].events.iter().find(|e| e.event.uid == "m-1").cloned().unwrap()
}

fn due(s: &Session, at: i64) -> Vec<String> {
    s.due_reminders(at).into_iter().filter(|r| r.uid == "m-1").map(|r| r.key).collect()
}

/// A single appointment: its reminder is turned off on the appointment (and goes to the server);
/// moved to another time, it doesn't ring again.
#[test]
fn dismissing_a_single_appointments_reminder_turns_it_off_for_good() {
    let (mut s, id) = exchange("rem-single", Vec::new(), None);
    let at = NOW + 3600 - 10 * 60;
    s.clock.now = at;
    let key = due(&s, at).pop().expect("due");
    s.run("reminders.dismiss", &json!({"key": key})).unwrap();
    let e = stored(&s, &id);
    assert_eq!(e.event.reminder, None);
    assert!(e.dirty, "the server learns it");
    assert!(!e.notify);
    assert!(!s.prefs.reminders.contains_key(&key), "nothing kept on this computer");
    s.run("event.move", &json!({"uid": "m-1", "start": NOW + 1800, "end": NOW + 3600})).unwrap();
    assert!(due(&s, NOW + 1800 - 60).is_empty(), "moved: still no reminder");
    assert!(due(&s, NOW + 3600).is_empty());
    done(s);
}

/// A meeting with a real guest: nobody is told that a reminder was dismissed.
#[test]
fn dismissing_a_meetings_reminder_tells_no_guest() {
    let (mut s, id) = exchange("rem-guest", vec![person(ME), person("ana@contoso.example")], None);
    s.clock.now = NOW + 3600 - 600;
    let key = due(&s, s.clock.now).pop().expect("due");
    s.run("reminders.dismiss", &json!({"key": key})).unwrap();
    let e = stored(&s, &id);
    assert!(e.dirty && !e.notify && e.event.reminder.is_none(), "{e:?}");
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::Send { .. })));
    done(s);
}

/// On a calendar kept by mail (IMAP and CalDAV), no invitation goes out either: neither for a
/// meeting the user organizes nor, as a reply, for one someone else does.
#[test]
fn dismissing_a_reminder_mails_nothing() {
    let mut s = sample_session("rem-mail");
    let start = NOW + 3600;
    let day = quatrelle_model::dates::day_string(start.div_euclid(86_400));
    let make = |s: &mut Session, summary: &str| -> String {
        s.run(
            "event.create",
            &json!({"summary": summary, "start": format!("{day}T23:00"), "end": format!("{day}T23:30"), "reminder": 30, "attendees": "Zoe Park <zoe@studio.example>"}),
        )
        .unwrap()["uid"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let mine = make(&mut s, "Mine");
    let theirs = make(&mut s, "Theirs");
    s.update_event(&theirs, |e| e.organizer = Some(person("zoe@studio.example"))).unwrap();
    let sent = |s: &Session| s.store.as_ref().unwrap().load_folder("sample", "Sent").unwrap().messages.len();
    let before = sent(&s);
    s.take_jobs();
    let pick = |s: &mut Session, uid: &str| {
        let at = s.find_event(uid).map(|(_, e)| e.event.start).unwrap();
        s.clock.now = at - 60;
        s.due_reminders(at - 60).into_iter().find(|r| r.uid == uid).map(|r| r.key).expect("due")
    };
    for uid in [&mine, &theirs] {
        let key = pick(&mut s, uid);
        s.run("reminders.dismiss", &json!({"key": key})).unwrap();
        assert_eq!(s.find_event(uid).unwrap().1.event.reminder, None);
    }
    assert_eq!(sent(&s), before, "no invitation, no reply");
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::Send { .. })));
    done(s);
}

/// A weekly series keeps its reminder: dismissing one occurrence's moves on to the next.
#[test]
fn dismissing_one_occurrence_of_a_series_keeps_the_series_reminder() {
    let (mut s, id) = exchange("rem-series", Vec::new(), Some("FREQ=WEEKLY"));
    let first = NOW + 3600 - 600;
    s.clock.now = first;
    let key = due(&s, first).pop().expect("due");
    s.run("reminders.dismiss", &json!({"key": key})).unwrap();
    assert_eq!(stored(&s, &id).event.reminder, Some(15), "the series is untouched");
    assert!(!stored(&s, &id).dirty);
    assert_eq!(s.prefs.reminders.get(&key), Some(&0));
    assert!(due(&s, first).is_empty());
    let next = NOW + 3600 + 7 * 86_400 - 600;
    assert_eq!(due(&s, next).len(), 1, "next week's still comes");
    done(s);
}

/// Dismissing them all treats each kind its way.
#[test]
fn dismissing_every_reminder_mixes_both_kinds() {
    let (mut s, id) = exchange("rem-all", Vec::new(), Some("FREQ=WEEKLY"));
    let p = s.pim_mut(&id);
    p.events.push(StoredEvent {
        calendar: "ews:calendar".into(),
        href: "I2".into(),
        etag: "ck".into(),
        event: CalEvent {
            uid: "single".into(),
            summary: "Once".into(),
            start: NOW + 3600,
            end: NOW + 7200,
            reminder: Some(15),
            ..Default::default()
        },
        ..Default::default()
    });
    let at = NOW + 3600 - 600;
    s.clock.now = at;
    assert_eq!(s.due_reminders(at).len(), 2);
    assert_eq!(s.run("reminders.dismiss", &json!({})).unwrap()["dismissed"], 2);
    assert!(s.due_reminders(at).is_empty());
    assert_eq!(stored(&s, &id).event.reminder, Some(15), "the series keeps it");
    let single = s.pim[&id].events.iter().find(|e| e.event.uid == "single").unwrap();
    assert_eq!(single.event.reminder, None);
    assert!(single.dirty);
    assert_eq!(s.prefs.reminders.len(), 1, "only the series' occurrence is kept here");
    done(s);
}
