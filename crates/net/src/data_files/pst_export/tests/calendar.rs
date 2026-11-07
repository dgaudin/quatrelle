// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The calendar through a data file: exported as Outlook appointments in its Calendar folder, and
//! read back by Quatrelle's import, field by field and occurrence by occurrence.

use super::state::{named_id, props_of};
use super::*;
use outlook_pst::UnicodePstSpecialFolder as Special;
use quatrelle_model::dates::days_from_civil;
use quatrelle_model::ical::{self, Attendee, CalEvent, PrivateKind, ShowAs};
use quatrelle_model::pim::files::event_file_lines;
use quatrelle_model::pim::{Pim, StoredEvent};
use quatrelle_model::zone::Zone;

pub(super) fn scratch(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("quatrelle-pst-export-cal-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Local wall-clock `y-m-d h:mi` in `zone`, as an instant.
pub(super) fn at(zone: &str, y: i64, m: u32, d: u32, h: i64, mi: i64) -> i64 {
    let local = days_from_civil(y, m, d) * 86_400 + h * 3600 + mi * 60;
    Zone::named(zone).unwrap().to_utc(local)
}

pub(super) fn person(email: &str, name: &str, partstat: &str, rsvp: bool) -> Attendee {
    Attendee { email: email.into(), name: name.into(), partstat: partstat.into(), rsvp, ..Default::default() }
}

/// The account's calendar: a single event with all Outlook keeps of one, an all-day event, a
/// weekly series in New York (across the change of summer time) with a moved, a retitled and a
/// deleted occurrence and one called off, an event in Tokyo and a daily series in Berlin, a
/// meeting with its organizer and guests, and an hourly series no pattern says.
fn calendar() -> Vec<CalEvent> {
    let ny = "America/New_York";
    let mut single = CalEvent {
        uid: "single@studio.example".into(),
        summary: "Screening — café".into(),
        description: "Bring the reels.\nAnd the notes.".into(),
        location: "Room 4".into(),
        start: 1_792_000_800,
        end: 1_792_006_200,
        categories: vec!["Film".into(), "Red".into()],
        sequence: 2,
        show_as: ShowAs::Tentative,
        private: true,
        private_kind: PrivateKind::Private,
        extra: vec![crate::pst_pim::event_file("agenda.txt", "text/plain", b"1. reels")],
        ..Default::default()
    };
    ical::set_reminder(&mut single, Some(30));
    let all_day = CalEvent {
        uid: "fair@studio.example".into(),
        summary: "Book fair".into(),
        start: days_from_civil(2026, 11, 20) * 86_400,
        end: days_from_civil(2026, 11, 22) * 86_400,
        all_day: true,
        show_as: ShowAs::Free,
        ..Default::default()
    };
    let mut weekly = CalEvent {
        uid: "weekly@studio.example".into(),
        summary: "Edit review".into(),
        location: "Suite B".into(),
        description: "Weekly.".into(),
        start: at(ny, 2026, 10, 6, 9, 30),
        end: at(ny, 2026, 10, 6, 10, 30),
        tzid: ny.into(),
        rrule: Some("FREQ=WEEKLY;BYDAY=TU,TH;COUNT=16".into()),
        exdates: vec![at(ny, 2026, 11, 10, 9, 30)],
        ..Default::default()
    };
    ical::set_reminder(&mut weekly, Some(10));
    let occurrence = |y, m, d| CalEvent { recurrence_id: Some(at(ny, y, m, d, 9, 30)), rrule: None, exdates: Vec::new(), ..weekly.clone() };
    // Moved by a day and an hour, after the clocks went back.
    let moved = CalEvent { start: at(ny, 2026, 11, 6, 14, 0), end: at(ny, 2026, 11, 6, 15, 30), ..occurrence(2026, 11, 5) };
    // Retitled, somewhere else, its own notes, no reminder, out of office.
    let mut retitled = CalEvent {
        summary: "Edit review — final cut".into(),
        location: "Theatre".into(),
        description: "Bring popcorn.".into(),
        show_as: ShowAs::OutOfOffice,
        ..occurrence(2026, 10, 15)
    };
    ical::set_reminder(&mut retitled, None);
    let called_off = CalEvent { status: "CANCELLED".into(), ..occurrence(2026, 11, 19) };
    let tokyo = CalEvent {
        uid: "tokyo@studio.example".into(),
        summary: "Call with Tokyo".into(),
        start: at("Asia/Tokyo", 2026, 12, 1, 18, 0),
        end: at("Asia/Tokyo", 2026, 12, 1, 19, 0),
        tzid: "Asia/Tokyo".into(),
        ..Default::default()
    };
    let berlin = CalEvent {
        uid: "berlin@studio.example".into(),
        summary: "Stand-up".into(),
        start: at("Europe/Berlin", 2026, 10, 20, 9, 0),
        end: at("Europe/Berlin", 2026, 10, 20, 9, 15),
        tzid: "Europe/Berlin".into(),
        rrule: Some("FREQ=DAILY;INTERVAL=2;UNTIL=20261115T235959Z".into()),
        ..Default::default()
    };
    let meeting = CalEvent {
        uid: "040000008200E00074C5B7101A82E00800000000A0B1C2D3E4F5A6010000000000000000100000004F1A2B3C4D5E6F708192A3B4C5D6E7F8".into(),
        summary: "Budget".into(),
        start: 1_793_000_000,
        end: 1_793_003_600,
        organizer: Some(person("riley@studio.example", "Riley", "", false)),
        attendees: vec![
            person("ana@studio.example", "Ana Lima", "ACCEPTED", false),
            person("ben@studio.example", "Ben", "DECLINED", false),
            person("cleo@posteo.example", "", "NEEDS-ACTION", true),
        ],
        ..Default::default()
    };
    let hourly = CalEvent {
        uid: "hourly@studio.example".into(),
        summary: "Render check".into(),
        start: 1_794_000_000,
        end: 1_794_000_600,
        rrule: Some("FREQ=HOURLY;INTERVAL=2;COUNT=5".into()),
        exdates: vec![1_794_000_000 + 2 * 7200],
        ..Default::default()
    };
    vec![single, all_day, weekly, moved, retitled, called_off, tokyo, berlin, meeting, hourly]
}

/// What an event is, as the import can give it back: its uid, times, zone, text, place, people,
/// how it shows its time, privacy, reminder, categories, files, and its occurrences.
fn shape(e: &CalEvent) -> String {
    format!(
        "{} {:?} {}–{} all_day={} tz={} {:?} {:?} {:?} org={:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?}",
        e.uid,
        e.recurrence_id,
        e.start,
        e.end,
        e.all_day,
        e.tzid,
        e.summary,
        e.description,
        e.location,
        e.organizer.as_ref().map(|o| &o.email),
        e.attendees,
        e.show_as,
        e.private,
        e.private.then_some(e.private_kind),
        ical::reminder_minutes(e),
        e.categories,
        event_file_lines(e),
        e.status,
        e.sequence,
    )
}

/// Every occurrence of `events` over three years from the first, as (uid, start, end, title).
fn occurrences(events: &[CalEvent]) -> Vec<(String, i64, i64, String)> {
    let from = 1_780_000_000;
    let mut out: Vec<_> = ical::occurrences(events, from, from + 3 * 366 * 86_400)
        .into_iter()
        .map(|o| (events[o.event].uid.clone(), o.start, o.end, events[o.event].summary.clone()))
        .collect();
    out.sort();
    out
}

#[test]
fn the_calendar_round_trips_inside_a_data_file() {
    use outlook_pst::ltp::prop_context::PropertyValue as V;
    let dir = scratch("round-trip");
    let store = Store::open(dir.join("cache")).unwrap();
    let events = calendar();
    let pim = Pim { events: events.iter().map(|e| StoredEvent { event: e.clone(), ..Default::default() }).collect(), ..Default::default() };
    store.save_account_doc("a1", quatrelle_model::pim::PIM_DOC, &pim).unwrap();
    let file = dir.join("Calendar.pst");
    let done = export(&store, "a1", "Calendar", &file, NONE, &mut |_| {}).unwrap();
    // Six appointments (a series one, its moved and retitled occurrences inside it, the one
    // called off deleted), and the hourly series' four occurrences, each one of its own.
    assert_eq!((done.events, done.occurrences), (10, 4), "{done:?}");
    assert!(!dir.join("Calendar Calendar.ics").exists());
    assert_eq!(UnicodePstFile::special_folder(&file, Special::Calendar).unwrap(), Some(vec!["Calendar".to_string()]));

    // As Outlook keeps them.
    let appt = |lid| named_id(&file, 0x0006_2002, lid).unwrap();
    let got = props_of(&file, &["Calendar"], "Edit review", &[0x001A, appt(0x8223), appt(0x8231), appt(0x8234), appt(0x8216), 0x0E1B]);
    assert_eq!(got[0].as_ref().and_then(crate::pst::text_of).as_deref(), Some("IPM.Appointment"));
    assert!(matches!(got[1], Some(V::Boolean(true))) && matches!(got[2], Some(V::Integer32(2))), "{got:?}");
    assert_eq!(got[3].as_ref().and_then(crate::pst::text_of).as_deref(), Some("America/New_York"));
    // Writer version 0x3009, two changed occurrences.
    let Some(V::Binary(p)) = &got[4] else { panic!("{got:?}") };
    assert!(p.buffer().windows(4).any(|w| w == 0x3009u32.to_le_bytes()));
    assert!(matches!(got[5], Some(V::Boolean(true))), "has its changed occurrences attached");
    let meeting = |lid| named_id(&file, 0x6ED8_DA90, lid).unwrap();
    let got = props_of(&file, &["Calendar"], "Screening — café", &[meeting(0x0003), 0x0036, appt(0x8205), appt(0x8215)]);
    let Some(V::Binary(id)) = &got[0] else { panic!("{got:?}") };
    assert!(id.buffer().windows(8).any(|w| w == b"vCal-Uid"));
    assert!(matches!(got[1..], [Some(V::Integer32(2)), Some(V::Integer32(1)), Some(V::Boolean(false))]), "{got:?}");

    // Read back by the import.
    let back = Store::open(dir.join("back")).unwrap();
    let imported = crate::pst::import(&file, &back, "b1", &mut |_| {}).unwrap();
    assert_eq!((imported.skipped, imported.files_left_out.len()), (0, 0), "{imported:?}");
    let got: Pim = back.load_account_doc("b1", quatrelle_model::pim::PIM_DOC).unwrap();
    let have: Vec<CalEvent> = got.events.iter().map(|e| e.event.clone()).collect();
    assert_eq!(have.len(), 12, "{have:#?}");
    // Each event as it went out (the occurrence called off is a deleted one now, as Outlook keeps
    // it; the hourly series is its occurrences, each with a uid of its own).
    let by_uid = |uid: &str, rid: Option<i64>| {
        have.iter().find(|e| e.uid == uid && e.recurrence_id == rid).unwrap_or_else(|| panic!("{uid} {rid:?}: {have:#?}"))
    };
    for e in events.iter().filter(|e| !e.uid.starts_with("hourly") && e.status != "CANCELLED") {
        let h = by_uid(&e.uid, e.recurrence_id);
        let mut want = e.clone();
        if want.uid.starts_with("weekly") && want.recurrence_id.is_none() {
            want.exdates.push(at("America/New_York", 2026, 11, 19, 9, 30));
        }
        assert_eq!(shape(h), shape(&want), "{}", e.summary);
        if let Some(rule) = &e.rrule {
            let mut ex = want.exdates.clone();
            let mut back_ex = h.exdates.clone();
            ex.sort_unstable();
            back_ex.sort_unstable();
            assert_eq!(back_ex, ex, "{rule}");
            assert_eq!(ical::starts(h, i64::MAX), ical::starts(e, i64::MAX), "{rule} → {:?}", h.rrule);
        }
    }
    // Every occurrence, over three years, where it was.
    let mut want = events.clone();
    for e in want.iter_mut().filter(|e| e.uid.starts_with("weekly") && e.recurrence_id.is_none()) {
        e.exdates.push(at("America/New_York", 2026, 11, 19, 9, 30));
    }
    want.retain(|e| e.status != "CANCELLED");
    let strip = |o: Vec<(String, i64, i64, String)>| {
        o.into_iter().map(|(uid, s, e, t)| (uid.split('/').next().unwrap_or_default().to_string(), s, e, t)).collect::<Vec<_>>()
    };
    assert_eq!(strip(occurrences(&have)), strip(occurrences(&want)));
    let hourly: Vec<&CalEvent> = have.iter().filter(|e| e.uid.starts_with("hourly@studio.example/")).collect();
    assert_eq!(hourly.len(), 4);
    assert!(hourly.iter().all(|e| e.rrule.is_none() && e.summary == "Render check"));

    // Exported again into the same file: nothing goes in twice.
    let again = export(&store, "a1", "Calendar", &file, NONE, &mut |_| {}).unwrap();
    assert_eq!((again.events, again.occurrences, again.pim_already), (0, 0, 10), "{again:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A meeting's request carries the meeting's own properties as its appointment has them: the
/// class Outlook gives meeting mail, the global object id, times, place and pattern.
#[test]
fn meeting_mail_carries_its_meeting() {
    use outlook_pst::ltp::prop_context::PropertyValue as V;
    let dir = scratch("meeting-mail");
    let store = Store::open(dir.join("cache")).unwrap();
    let mut invite = CalEvent {
        uid: "review@studio.example".into(),
        summary: "Review".into(),
        location: "Suite B".into(),
        start: at("Europe/Berlin", 2026, 11, 3, 10, 0),
        end: at("Europe/Berlin", 2026, 11, 3, 11, 0),
        tzid: "Europe/Berlin".into(),
        rrule: Some("FREQ=WEEKLY;COUNT=4".into()),
        organizer: Some(person("riley@studio.example", "Riley", "", false)),
        attendees: vec![person("ana@studio.example", "Ana", "NEEDS-ACTION", true)],
        ..Default::default()
    };
    ical::set_reminder(&mut invite, Some(15));
    let moved = CalEvent {
        recurrence_id: Some(at("Europe/Berlin", 2026, 11, 10, 10, 0)),
        start: at("Europe/Berlin", 2026, 11, 10, 12, 0),
        end: at("Europe/Berlin", 2026, 11, 10, 13, 0),
        rrule: None,
        ..invite.clone()
    };
    let ics = ical::write(&[invite.clone(), moved], Some("REQUEST"), 1_790_000_000);
    let reply = CalEvent { attendees: vec![person("ana@studio.example", "Ana", "ACCEPTED", false)], ..invite.clone() };
    let answer = ical::write(&[reply], Some("REPLY"), 1_790_000_000);
    let mk = |uid: u32, subject: &str, ics: &str, method: &str| {
        let d = mime::Draft {
            to: vec![EmailAddress::new("Ana", "ana@studio.example")],
            subject: subject.into(),
            body: "Join us.".into(),
            attachments: vec![mime::OutAttachment {
                name: "invite.ics".into(),
                mime: format!("text/calendar; method={method}"),
                data: ics.as_bytes().to_vec(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let raw = mime::build(&d, &EmailAddress::new("Riley", "riley@studio.example"), 1_790_000_000, &format!("m{uid}@studio.example")).unwrap();
        let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
        store.write_body(&key, &raw).unwrap();
        mime::summary_from_full(&raw, key, Flags::default(), 10, 0)
    };
    let inbox = FolderState {
        validity: 1,
        uid_next: 4,
        messages: vec![mk(1, "Review", &ics, "REQUEST"), mk(2, "Accepted: Review", &answer, "REPLY"), mk(3, "Plain", "", "x")],
        ..Default::default()
    };
    store.save_folder("a1", "INBOX", &inbox).unwrap();
    let folder =
        quatrelle_model::Folder { account: "a1".into(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
    store.save_folders("a1", &[folder]).unwrap();
    let account = quatrelle_model::AccountConfig { id: "a1".into(), email: "ana@studio.example".into(), ..Default::default() };
    store.save_accounts(&[account]).unwrap();
    let file = dir.join("Mail.pst");
    let done = export(&store, "a1", "Mail", &file, NONE, &mut |_| {}).unwrap();
    assert_eq!(done.messages, 3);
    let appt = |lid| named_id(&file, 0x0006_2002, lid).unwrap();
    let meeting = |lid| named_id(&file, 0x6ED8_DA90, lid).unwrap();
    let common = |lid| named_id(&file, 0x0006_2008, lid).unwrap();
    let ids = [0x001A, meeting(0x0003), appt(0x820D), appt(0x8208), appt(0x8216), appt(0x8234), common(0x8501), meeting(0x0024)];
    let got = props_of(&file, &["Inbox"], "Review", &ids);
    assert_eq!(got[0].as_ref().and_then(crate::pst::text_of).as_deref(), Some("IPM.Schedule.Meeting.Request"));
    let Some(V::Binary(id)) = &got[1] else { panic!("{got:?}") };
    assert_eq!(crate::pst_pim::uid_of(id.buffer()), "review@studio.example");
    assert!(matches!(got[2], Some(V::Time(t)) if t == item::filetime(invite.start)), "{got:?}");
    assert_eq!(got[3].as_ref().and_then(crate::pst::text_of).as_deref(), Some("Suite B"));
    let Some(V::Binary(p)) = &got[4] else { panic!("{got:?}") };
    let (rule, _) = crate::pst_pim::recurrence_in(p.buffer(), invite.start, "Europe/Berlin").unwrap();
    assert_eq!(rule, "FREQ=WEEKLY;INTERVAL=1;BYDAY=TU;COUNT=4");
    assert_eq!(got[5].as_ref().and_then(crate::pst::text_of).as_deref(), Some("Europe/Berlin"));
    assert!(matches!(got[6], Some(V::Integer32(15))));
    assert_eq!(got[7].as_ref().and_then(crate::pst::text_of).as_deref(), Some("IPM.Appointment"));
    let class = |subject: &str| props_of(&file, &["Inbox"], subject, &[0x001A]).remove(0).as_ref().and_then(crate::pst::text_of);
    assert_eq!(class("Accepted: Review").as_deref(), Some("IPM.Schedule.Meeting.Resp.Pos"));
    assert_eq!(class("Plain").as_deref(), Some("IPM.Note"));
    // Received (asfMeeting | asfReceived), not answered yet; its times, place, and when it changed.
    let ids = [appt(0x8217), appt(0x8218), 0x0060, 0x0061, 0x0063, meeting(0x0002), meeting(0x0001), meeting(0x001A)];
    let got = props_of(&file, &["Inbox"], "Review", &ids);
    assert!(matches!(got[..2], [Some(V::Integer32(3)), Some(V::Integer32(5))]), "{got:?}");
    assert!(matches!(got[2], Some(V::Time(t)) if t == item::filetime(invite.start)), "{got:?}");
    assert!(matches!(got[3], Some(V::Time(t)) if t == item::filetime(invite.end)), "{got:?}");
    assert!(matches!(got[4], Some(V::Boolean(true))), "{got:?}");
    assert_eq!(got[5].as_ref().and_then(crate::pst::text_of).as_deref(), Some("Suite B"));
    assert!(matches!(got[6..], [Some(V::Time(a)), Some(V::Time(b))] if a == b && a == item::filetime(1_790_000_000)), "{got:?}");
    // The guest's answer to it: the guest's own, accepted.
    let got = props_of(&file, &["Inbox"], "Accepted: Review", &[appt(0x8217), appt(0x8218)]);
    assert!(matches!(got[..], [Some(V::Integer32(3)), Some(V::Integer32(3))]), "{got:?}");

    // Read back, the request still carries its calendar.
    let back = Store::open(dir.join("back")).unwrap();
    crate::pst::import(&file, &back, "b1", &mut |_| {}).unwrap();
    let inbox = back.load_folder("b1", "INBOX").unwrap();
    let review = inbox.messages.iter().find(|m| m.subject == "Review").unwrap();
    let raw = back.read_body(&review.key).unwrap();
    let p = mime::parse(&raw).unwrap();
    let part = p.attachments.iter().find(|a| a.mime.starts_with("text/calendar")).unwrap();
    let cal = ical::parse(&mime::attachment_text(&raw, part.index).unwrap(), 0);
    assert_eq!(cal.method.as_deref(), Some("REQUEST"));
    assert_eq!(cal.events.len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

mod items;
