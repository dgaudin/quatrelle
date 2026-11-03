// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: what the agenda says of each appointment (private, how it shows, the user's
//! answer), and changing those without telling the guests.

use quatrelle_model::ical::{Attendee, CalEvent, ShowAs};
use quatrelle_model::pim::{CalendarInfo, DavSync, StoredEvent};

use super::*;

const DAY: i64 = 86_400;

fn around(s: &Session) -> Vec<crate::AgendaItem> {
    s.agenda(NOW - 30 * DAY, NOW + 30 * DAY)
}

fn item(s: &Session, summary: &str) -> crate::AgendaItem {
    around(s).into_iter().find(|i| i.summary == summary).unwrap_or_else(|| panic!("`{summary}` is not in the agenda"))
}

fn sent(s: &Session) -> usize {
    s.store.as_ref().unwrap().load_folder("sample", "Sent").unwrap().messages.len()
}

fn guest(email: &str, partstat: &str) -> Attendee {
    Attendee { email: email.into(), name: email.into(), partstat: partstat.into(), rsvp: true, ..Default::default() }
}

/// Put an event in the agenda: in the sample's own calendar, or in a server calendar.
fn put_event(s: &mut Session, read_only: bool, event: CalEvent) {
    let cal = "https://dav.example.org/cal/work/".to_string();
    let sync = DavSync {
        caldav_home: Some("https://dav.example.org/cal/".into()),
        calendars: Some(vec![CalendarInfo { id: cal.clone(), name: "Work".into(), color: 1, read_only, ctag: "a".into(), group: String::new() }]),
        events: vec![StoredEvent { calendar: cal.clone(), href: format!("{cal}x.ics"), etag: "\"1\"".into(), event, ..Default::default() }],
        synced_calendars: vec![cal],
        checked: NOW,
        ..Default::default()
    };
    s.apply(Event::DavSynced { account: "sample".into(), sync: Box::new(sync) });
}

fn meeting(uid: &str, organizer: Option<&str>, my_answer: &str, guests: Vec<Attendee>) -> CalEvent {
    CalEvent {
        uid: uid.into(),
        summary: "Planning".into(),
        start: NOW + DAY,
        end: NOW + DAY + 3600,
        organizer: organizer.map(|o| guest(o, "ACCEPTED")),
        attendees: guests,
        my_answer: my_answer.into(),
        ..Default::default()
    }
}

#[test]
fn the_agenda_says_private_show_as_and_answer() {
    let mut s = sample_session("showing-agenda");
    let lunch = item(&s, "Lunch with Jun");
    assert!(lunch.private && lunch.answer.is_none(), "{lunch:?}");
    assert_eq!(item(&s, "Winter group show opening").show_as, ShowAs::OutOfOffice);
    let mix = item(&s, "Sound mix playback");
    assert_eq!(mix.answer.as_deref(), Some("NEEDS-ACTION"));
    s.run("event.respond", &json!({"uid": mix.uid, "answer": "accepted"})).unwrap();
    assert_eq!(item(&s, "Sound mix playback").answer.as_deref(), Some("ACCEPTED"));

    let listed = s.run("calendar.events", &json!({"from": "2026-09-15", "to": "2026-11-30"})).unwrap();
    let find = |name: &str| listed.as_array().unwrap().iter().find(|e| e["summary"] == name).unwrap().clone();
    assert_eq!(find("Lunch with Jun")["private"], json!(true));
    assert_eq!(find("Winter group show opening")["showAs"], json!("outOfOffice"));
    assert_eq!(find("Sound mix playback")["answer"], json!("ACCEPTED"));
    assert_eq!(find("Lunch with Jun")["answer"], json!(null));
}

#[test]
fn an_organizer_without_a_known_address_is_answered_by_my_answer() {
    let mut s = sample_session("showing-no-organizer");
    put_event(&mut s, false, meeting("a@x", None, "TENTATIVE", vec![guest("zoe@studio.example", "ACCEPTED")]));
    assert_eq!(item(&s, "Planning").answer.as_deref(), Some("TENTATIVE"));
    put_event(&mut s, false, meeting("a@x", None, "", vec![guest("zoe@studio.example", "ACCEPTED")]));
    assert_eq!(item(&s, "Planning").answer, None);
}

#[test]
fn a_read_only_calendars_meeting_has_no_answer() {
    let mut s = sample_session("showing-read-only");
    let invitation = meeting("b@x", Some("ana@soundlab.example"), "", vec![guest("riley@studio.example", "NEEDS-ACTION")]);
    put_event(&mut s, false, invitation.clone());
    assert_eq!(item(&s, "Planning").answer.as_deref(), Some("NEEDS-ACTION"), "writable: the answer is awaited");
    put_event(&mut s, true, invitation);
    assert_eq!(item(&s, "Planning").answer, None);
}

#[test]
fn private_and_show_as_change_without_telling_the_guests() {
    let mut s = sample_session("showing-silent");
    let start = "2026-10-20T10:00";
    let uid = s
        .run("event.create", &json!({"summary": "Kick-off", "start": start, "end": "2026-10-20T11:00", "attendees": "zoe@studio.example"}))
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    let before = sent(&s);
    s.run("event.update", &json!({"uid": uid, "private": true, "showAs": "free", "send": false})).unwrap();
    let e = s.find_event(&uid).unwrap().1.event.clone();
    assert!(e.private && e.show_as == ShowAs::Free, "{e:?}");
    assert_eq!(sent(&s), before, "the guests heard nothing");
    s.run("event.update", &json!({"uid": uid, "summary": "Renamed"})).unwrap();
    assert_eq!(sent(&s), before + 1);
}

/// The stand-up's occurrences around now, as `(occurrence, start)`.
fn stand_ups(s: &Session) -> Vec<(i64, i64)> {
    around(s).into_iter().filter(|i| i.summary == "Team stand-up").map(|i| (i.occurrence, i.start)).collect()
}

/// An edit of something other than the time must not move a series: giving a zoneless series the
/// user's zone makes it repeat at its first occurrence's local time, an hour off across a
/// summer/winter change.
#[test]
fn an_edit_that_keeps_the_time_keeps_the_series_in_place() {
    for (what, params) in [
        ("showAs", json!({"uid": "standup@sample.quatrelle", "showAs": "tentative", "send": false})),
        ("private", json!({"uid": "standup@sample.quatrelle", "private": true, "send": false})),
        ("categories", json!({"uid": "standup@sample.quatrelle", "categories": "Team", "send": false})),
    ] {
        let mut s = sample_session("showing-in-place");
        s.set_clock(NOW, 2 * 3600);
        s.set_zone(Some("Europe/Paris"));
        assert!(s.clock.zone.is_some());
        let (before, tzid) = (stand_ups(&s), s.find_event("standup@sample.quatrelle").unwrap().1.event.tzid.clone());
        assert!(before.len() > 8, "{before:?}");
        s.run("event.update", &params).unwrap();
        let e = &s.find_event("standup@sample.quatrelle").unwrap().1.event;
        assert_eq!(e.tzid, tzid, "{what}: the zone is as it was");
        match what {
            "showAs" => assert_eq!(e.show_as, ShowAs::Tentative),
            "private" => assert!(e.private),
            _ => assert_eq!(e.categories, vec!["Team".to_string()]),
        }
        assert_eq!(stand_ups(&s), before, "{what}: every occurrence starts when it did");
    }
}

#[test]
fn a_new_time_gives_a_zoneless_event_the_users_zone() {
    let mut s = sample_session("showing-new-time");
    s.set_clock(NOW, 2 * 3600);
    s.set_zone(Some("Europe/Paris"));
    assert_eq!(s.find_event("standup@sample.quatrelle").unwrap().1.event.tzid, "");
    s.run("event.update", &json!({"uid": "standup@sample.quatrelle", "start": "2026-10-12T09:30", "send": false})).unwrap();
    assert_eq!(s.find_event("standup@sample.quatrelle").unwrap().1.event.tzid, "Europe/Paris");
}

/// On an account whose server tells the guests, a change made without telling them ("send":
/// false: Show As, Private, Categorize from the grid's menu) doesn't take back an invitation, or
/// an update, still to go: the meeting created and made private before the sync runs is still
/// sent to its guests. One already on the server stays quiet, and is told when asked to.
#[test]
fn a_silent_edit_keeps_a_notification_still_to_go() {
    let mut s = Session::open(Some(temp("showing-notify")), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s
        .run("account.add", &json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.pim_mut(&id).calendars = vec![CalendarInfo { id: "ews:calendar".into(), name: "Calendar".into(), ..Default::default() }];
    let uid = s
        .run("event.create", &json!({"summary": "Budget", "start": "2026-10-21T15:00", "attendees": "Ana <ana@contoso.example>", "account": id}))
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    let stored = |s: &Session| s.pim[&id].events.iter().find(|e| e.event.uid == uid).cloned().unwrap();
    assert!(stored(&s).notify, "the invitation is to go");
    s.run("event.update", &json!({"uid": uid, "private": true, "send": false})).unwrap();
    let e = stored(&s);
    assert!(e.event.private && e.notify, "still to go: {e:?}");
    // On the server: a silent edit stays silent, an edit that tells is marked again.
    let pushed = StoredEvent { href: "I1".into(), etag: "ck".into(), dirty: false, notify: false, ..stored(&s) };
    s.apply_dav(&id, &DavSync { pushed_events: vec![pushed], ..Default::default() });
    assert!(!stored(&s).notify);
    s.run("event.update", &json!({"uid": uid, "showAs": "free", "send": false})).unwrap();
    assert!(!stored(&s).notify && stored(&s).dirty, "{:?}", stored(&s));
    s.run("event.update", &json!({"uid": uid, "showAs": "busy", "send": true})).unwrap();
    assert!(stored(&s).notify);
    done(s);
}

/// Put an event in a calendar of the sample's account by its id (a colleague's: `shared:…`).
fn put_in(s: &mut Session, cal: &str, read_only: bool, event: CalEvent) {
    let sync = DavSync {
        calendars: Some(vec![CalendarInfo { id: cal.into(), name: "Ana".into(), color: 1, read_only, ctag: "a".into(), group: String::new() }]),
        events: vec![StoredEvent { calendar: cal.into(), href: format!("{cal}/x"), etag: "\"1\"".into(), event, ..Default::default() }],
        synced_calendars: vec![cal.into()],
        checked: NOW,
        ..Default::default()
    };
    s.apply(Event::DavSynced { account: "sample".into(), sync: Box::new(sync) });
}

/// An invitation in a colleague's calendar the user may change isn't the user's to answer: no
/// answer in the agenda (else Accept would answer in the user's name, or for the colleague).
#[test]
fn a_colleagues_writable_calendars_meeting_has_no_answer() {
    let mut s = sample_session("showing-colleague");
    let cal = quatrelle_model::pim::shared_calendar_id("ana@soundlab.example");
    let invitation = meeting("c@x", Some("ana@soundlab.example"), "", vec![guest("riley@studio.example", "NEEDS-ACTION")]);
    put_in(&mut s, &cal, false, invitation);
    let shown = around(&s).into_iter().find(|i| i.calendar == cal).expect("the colleague's calendar is shown");
    assert_eq!(shown.answer, None, "{shown:?}");
}
