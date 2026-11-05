// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: dismissing the reminder of an appointment takes it off the appointment, as Outlook
//! does (Exchange's `ReminderIsSet`), so moving it doesn't bring it back; a series keeps its
//! reminder and is answered per occurrence; a reminder is personal, so no guest is told (03/11).

use quatrelle_model::ical::{self, Attendee, CalEvent};
use quatrelle_model::pim::{CalendarInfo, DavSync, ReminderAnswer, StoredEvent};

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

/// A single appointment beside the series of [`exchange`], with its own item on the server.
fn add_single(s: &mut Session, id: &str) {
    s.pim_mut(id).events.push(StoredEvent {
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
}

fn key_for(s: &Session, uid: &str) -> String {
    s.due_reminders(s.clock.now).into_iter().find(|r| r.uid == uid).map(|r| r.key).expect("due")
}

/// Exchange (issue #40): one occurrence of a series dismissed, and a single appointment snoozed,
/// are for the server to learn too, so Outlook doesn't show them again: a sync is asked at once,
/// and each answer waits with the calendar (a restart keeps it) until a sync takes it. The
/// series itself doesn't change; answered again, the later answer stands.
#[test]
fn a_dismissed_occurrence_and_a_snooze_go_to_exchange() {
    let (mut s, id) = exchange("rem-tell-ews", Vec::new(), Some("FREQ=WEEKLY"));
    add_single(&mut s, &id);
    s.clock.now = NOW + 3600 - 600;
    let (series, single) = (key_for(&s, "m-1"), key_for(&s, "single"));
    s.run("reminders.dismiss", &json!({"key": series})).unwrap();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::SyncDav { account } if *account == id)), "the occurrence: the server is told");
    s.run("reminders.snooze", &json!({"key": single, "minutes": 5})).unwrap();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::SyncDav { account } if *account == id)), "the snooze: the server is told");
    let occurrence = NOW + 3600;
    let waiting = [
        ReminderAnswer { calendar: "ews:calendar".into(), href: "I1".into(), uid: "m-1".into(), occurrence, series: true, snooze_until: None },
        ReminderAnswer {
            calendar: "ews:calendar".into(),
            href: "I2".into(),
            uid: "single".into(),
            occurrence,
            series: false,
            snooze_until: Some(s.clock.now + 300),
        },
    ];
    assert_eq!(s.pim[&id].reminder_answers, waiting);
    assert!(!stored(&s, &id).dirty && stored(&s, &id).event.reminder == Some(15), "the series doesn't change");

    // Offline: nothing taken; a restart keeps them.
    let root = s.store.as_ref().unwrap().root().to_path_buf();
    let mut s = Session::open(Some(quatrelle_store::Store::open(&root).unwrap()), quatrelle_model::dates::Clock::new(s.clock.now, 0));
    assert_eq!(s.pim[&id].reminder_answers, waiting, "kept for the next sync");

    // The single one dismissed after its snooze: its reminder is turned off on the item, as
    // before, and its snooze no longer waits.
    s.clock.now += 300;
    let single = key_for(&s, "single");
    s.run("reminders.dismiss", &json!({"key": single})).unwrap();
    assert!(s.pim[&id].events.iter().any(|e| e.event.uid == "single" && e.dirty && e.event.reminder.is_none()));
    assert_eq!(s.pim[&id].reminder_answers, waiting[..1]);

    // The occurrence answered again (snoozed): the later answer stands, and a sync that took
    // the earlier one leaves it waiting; the sync that takes it ends its wait.
    let series = key_for(&s, "m-1");
    s.run("reminders.snooze", &json!({"key": series, "minutes": 10})).unwrap();
    let later = ReminderAnswer { snooze_until: Some(s.clock.now + 600), ..waiting[0].clone() };
    assert_eq!(s.pim[&id].reminder_answers, [later.clone()]);
    let took = |s: &mut Session, answered: Vec<ReminderAnswer>| {
        let sync = DavSync { answered, checked: s.clock.now, ..Default::default() };
        s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(sync) });
    };
    took(&mut s, waiting.to_vec());
    assert_eq!(s.pim[&id].reminder_answers, [later.clone()]);
    took(&mut s, vec![later]);
    assert!(s.pim[&id].reminder_answers.is_empty());
    assert!(due(&s, s.clock.now).is_empty(), "still answered here");
    done(s);
}

/// A shared calendar, or one this account may only read, has nothing told to its server.
#[test]
fn a_calendar_that_cant_be_changed_is_told_nothing() {
    let (mut s, id) = exchange("rem-read-only", Vec::new(), Some("FREQ=WEEKLY"));
    if let Some(c) = s.pim_mut(&id).calendars.first_mut() {
        c.read_only = true;
    }
    s.clock.now = NOW + 3600 - 600;
    let key = key_for(&s, "m-1");
    s.run("reminders.dismiss", &json!({"key": key})).unwrap();
    assert!(s.pim[&id].reminder_answers.is_empty());
    assert!(due(&s, s.clock.now).is_empty(), "answered here all the same");
    done(s);
}

/// Exchange Online (Graph) is told the same way: the occurrence's answer waits for the sync.
#[test]
fn an_answer_on_graph_waits_too() {
    let (mut s, id) = exchange("rem-tell-graph", Vec::new(), Some("FREQ=WEEKLY"));
    if let Some(a) = s.accounts.iter_mut().find(|a| a.config.id == id) {
        a.config.ews = None;
        a.config.graph = Some("https://graph.microsoft.com/v1.0".into());
    }
    s.clock.now = NOW + 3600 - 600;
    let key = key_for(&s, "m-1");
    s.run("reminders.snooze", &json!({"key": key, "minutes": 5})).unwrap();
    let a = &s.pim[&id].reminder_answers;
    assert_eq!((a.len(), a.first().map(|a| (a.series, a.snooze_until))), (1, Some((true, Some(s.clock.now + 300)))));
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::SyncDav { account } if *account == id)));
    done(s);
}

const DAV: &str = "https://dav.example.org/cal/work/";

/// An event with a 15-minute reminder an hour after `NOW`, as a CalDAV server gave it; `daily`
/// makes it a series.
fn on_caldav(uid: &str, daily: bool) -> StoredEvent {
    StoredEvent {
        calendar: DAV.into(),
        href: format!("{DAV}{uid}.ics"),
        etag: "\"1\"".into(),
        event: CalEvent {
            uid: uid.into(),
            summary: format!("Event {uid}"),
            start: NOW + 3600,
            end: NOW + 7200,
            rrule: daily.then(|| "FREQ=DAILY;COUNT=5".to_string()),
            reminder: Some(15),
            sequence: 2,
            dtstamp: NOW,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// A CalDAV account whose calendar holds `events`, as its server's sync gave them.
fn caldav(name: &str, events: Vec<StoredEvent>) -> (Session, String) {
    let mut s = Session::open(Some(temp(name)), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let sync = DavSync {
        caldav_home: Some("https://dav.example.org/cal/".into()),
        calendars: Some(vec![CalendarInfo { id: DAV.into(), name: "Work".into(), ctag: "a".into(), ..Default::default() }]),
        events,
        synced_calendars: vec![DAV.into()],
        checked: NOW,
        ..Default::default()
    };
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(sync) });
    s.take_jobs();
    (s, id)
}

/// CalDAV (issue #40): an occurrence of a series dismissed here is written into the event as
/// RFC 9074 says, its alarm `ACKNOWLEDGED`, and the event goes to the server, its `SEQUENCE` as
/// it was (nothing for a guest to learn); the next occurrence still rings. A snooze stays here.
#[test]
fn a_dismissal_on_caldav_acknowledges_the_alarm() {
    let (mut s, id) = caldav("rem-caldav", vec![on_caldav("daily", true), on_caldav("two", true)]);
    s.clock.now = NOW + 3600 - 600;
    let (daily, two) = (key_for(&s, "daily"), key_for(&s, "two"));
    s.run("reminders.snooze", &json!({"key": two, "minutes": 5})).unwrap();
    assert!(!s.find_event("two").unwrap().1.dirty, "a snooze isn't the server's");
    s.run("reminders.dismiss", &json!({"key": daily})).unwrap();
    let e = s.find_event("daily").unwrap().1.clone();
    assert!(e.dirty, "the server learns it");
    assert_eq!(ical::acknowledged(&e.event), Some(s.clock.now));
    assert_eq!((e.event.sequence, e.event.reminder), (2, Some(15)), "not a change to the meeting");
    let written = ical::write(std::slice::from_ref(&e.event), None, NOW);
    let alarm = written.split("BEGIN:VALARM").nth(1).unwrap_or_default();
    assert!(alarm.contains("TRIGGER:-PT15M") && alarm.contains("ACKNOWLEDGED:"), "{written}");
    assert_eq!(ical::parse(&written, 0).events.first().and_then(|e| e.acknowledged), Some(s.clock.now), "read back");
    assert!(s.pim[&id].reminder_answers.is_empty());
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::SyncDav { account } if *account == id)));
    assert!(due(&s, s.clock.now).is_empty() && !s.due_reminders(s.clock.now).iter().any(|r| r.uid == "daily"));
    let tomorrow = NOW + 3600 + 86_400 - 600;
    assert!(s.due_reminders(tomorrow).iter().any(|r| r.uid == "daily"), "tomorrow's still rings");
    done(s);
}

/// An event another program dismissed that way (acknowledged after its alarm) isn't due here
/// either; one acknowledged before this alarm still is.
#[test]
fn an_alarm_acknowledged_elsewhere_isnt_due() {
    let mut elsewhere = on_caldav("three", false);
    ical::acknowledge(&mut elsewhere.event, NOW + 3600 - 14 * 60);
    let mut early = on_caldav("four", false);
    ical::acknowledge(&mut early.event, NOW - 86_400);
    let (s, _) = caldav("rem-caldav-elsewhere", vec![elsewhere, early]);
    let now_due: Vec<String> = s.due_reminders(NOW + 3600 - 10 * 60).into_iter().map(|r| r.uid).collect();
    assert!(!now_due.contains(&"three".to_string()), "{now_due:?}");
    assert!(now_due.contains(&"four".to_string()), "acknowledged before this alarm: still due ({now_due:?})");
    done(s);
}

/// A single appointment's reminder dismissed on CalDAV is turned off, as before, without its
/// `SEQUENCE` raised: a reminder is personal, no guest has anything to learn.
#[test]
fn a_single_dismissal_on_caldav_raises_no_sequence() {
    let (mut s, _) = caldav("rem-caldav-single", vec![on_caldav("one", false)]);
    s.clock.now = NOW + 3600 - 600;
    let key = key_for(&s, "one");
    s.run("reminders.dismiss", &json!({"key": key})).unwrap();
    let e = s.find_event("one").unwrap().1.clone();
    assert!(e.dirty && e.event.reminder.is_none());
    assert_eq!(e.event.sequence, 2, "not a change to the meeting");
    done(s);
}
