// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! `event.teamsMeeting`, and the Teams meeting following its appointment: asked for in a calendar
//! of the user's own, sent what a save changed (its title, or its start or end as instants: a zone
//! alone changes nothing), deleted with the appointment or when another takes its place, never
//! acted on for a stranger, and waiting for Teams when Teams can't be used.

use quatrelle_model::pim::CalendarInfo;
use quatrelle_model::zone::Zone;

use super::*;
use crate::TeamsMeetingPath;

const EWS: &str = "https://mail.contoso.example/EWS/Exchange.asmx";
const DAV: &str = "https://dav.contoso.example/cal/work/";
pub(super) const LOCAL_CALENDAR: &str = quatrelle_model::pim::LOCAL;

pub(super) fn is_params<T: std::fmt::Debug>(r: Result<T, CmdError>) -> bool {
    matches!(r, Err(CmdError::Params(_)))
}

pub(super) fn is_disabled<T: std::fmt::Debug>(r: Result<T, CmdError>) -> bool {
    matches!(r, Err(CmdError::Disabled(_)))
}

/// What `event.teamsMeeting` is asked to add: "Budget", 15:00 to 16:30 on 21 October.
pub(super) fn add(account: &str, calendar: &str) -> Value {
    json!({"add": {"summary": "Budget", "start": "2026-10-21T15:00", "end": "2026-10-21T16:30", "account": account, "calendar": calendar}})
}

fn at(s: &Session, local: &str) -> i64 {
    crate::parse_local(local, &s.clock).unwrap().0
}

/// The sample account read as `kind` ("ews", "graph", anything else for as it is), with these
/// calendars too: (id, someone else's).
pub(super) fn calendars(s: &mut Session, kind: &str, more: &[(&str, bool)]) {
    let a = s.accounts.iter_mut().find(|a| a.config.id == "sample").unwrap();
    match kind {
        "ews" => a.config.ews = Some(EWS.into()),
        "graph" => a.config.graph = Some(quatrelle_model::GRAPH_URL.into()),
        _ => {}
    }
    let p = s.pim_mut("sample");
    for (id, not_own) in more {
        if !p.calendars.iter().any(|c| c.id == *id) {
            p.calendars.push(CalendarInfo { id: (*id).into(), name: format!("Calendar {id}"), not_own: *not_own, ..Default::default() });
        }
    }
}

/// An appointment without guests, 15:00 to 16:00 on 21 October, its Teams meeting `m1`; its uid.
fn alone(s: &mut Session) -> String {
    let made = s.run("event.create", &json!({"summary": "Budget", "start": "2026-10-21T15:00", "end": "2026-10-21T16:00", "teamsMeetingId": "m1"}));
    let uid = made.unwrap()["uid"].as_str().unwrap().to_string();
    assert_eq!(event(s, &uid).teams_meeting_id.as_deref(), Some("m1"));
    uid
}

pub(super) fn event(s: &Session, uid: &str) -> CalEvent {
    s.find_event(uid).unwrap().1.event.clone()
}

/// The fields `uid` has now, as Teams was last sent them.
fn sent_as_now(s: &mut Session, uid: &str, id: &str) -> TeamsMeetingFields {
    let now = TeamsMeetingFields::of(&event(s, uid));
    s.teams_meetings.sent.insert(id.into(), now.clone());
    now
}

fn update(id: &str, before: &TeamsMeetingFields, after: &TeamsMeetingFields) -> teams::Job {
    teams::Job::MeetingUpdate { id: id.into(), before: before.clone(), after: after.clone() }
}

fn delete(id: &str) -> teams::Job {
    teams::Job::MeetingDelete { id: id.into() }
}

/// The last notice: its template, its values, whether it's an error.
fn last_notice(s: &Session) -> Option<(&'static str, Vec<(&'static str, String)>, bool)> {
    s.notices.last().map(|n| (n.template, n.values.clone(), n.error))
}

#[test]
fn adding_needs_a_title_a_start_a_sign_in_and_the_permission() {
    let mut s = signed_in("teams-add-needs");
    let without = |k: &str| {
        let mut p = add("sample", LOCAL_CALENDAR);
        p["add"].as_object_mut().unwrap().remove(k);
        p
    };
    assert!(is_params(s.run("event.teamsMeeting", &without("summary"))));
    let mut blank = add("sample", LOCAL_CALENDAR);
    blank["add"]["summary"] = json!("  ");
    assert!(is_params(s.run("event.teamsMeeting", &blank)));
    assert!(is_params(s.run("event.teamsMeeting", &without("start"))));
    assert!(is_params(s.run("event.teamsMeeting", &json!({}))));
    for (on, signed_in, meetings) in [(false, true, true), (true, false, true), (true, true, false)] {
        let t = &mut s.prefs.teams;
        (t.on, t.signed_in, t.meetings) = (on, signed_in, meetings);
        assert!(is_disabled(s.run("event.teamsMeeting", &add("sample", LOCAL_CALENDAR))), "{on} {signed_in} {meetings}");
    }
    assert!(teams_jobs(&mut s).is_empty());
    assert!(s.teams_meetings.asks.is_empty());
    done(s);
}

#[test]
fn adding_without_its_calendar_is_refused() {
    let mut s = signed_in("teams-add-calendar");
    for k in ["account", "calendar"] {
        let mut p = add("sample", LOCAL_CALENDAR);
        p["add"].as_object_mut().unwrap().remove(k);
        assert!(is_params(s.run("event.teamsMeeting", &p)), "{k}");
        p["add"][k] = json!("");
        assert!(is_params(s.run("event.teamsMeeting", &p)), "{k} empty");
    }
    assert!(teams_jobs(&mut s).is_empty());
    assert!(s.teams_meetings.asks.is_empty());
    done(s);
}

#[test]
fn adding_asks_teams_with_the_appointments_fields() {
    for (kind, calendar) in [("ews", "calendar"), ("dav", DAV), ("local", LOCAL_CALENDAR)] {
        let mut s = signed_in("teams-add");
        calendars(&mut s, kind, &[(calendar, false)]);
        assert_eq!(s.teams_meeting_path("sample", calendar), TeamsMeetingPath::Standalone, "{kind}");
        let made = s.run("event.teamsMeeting", &add("sample", calendar)).unwrap();
        let ask = made["ask"].as_u64().unwrap();
        let asked = TeamsMeetingFields { title: "Budget".into(), start: at(&s, "2026-10-21T15:00"), end: at(&s, "2026-10-21T16:30") };
        assert_eq!(teams_jobs(&mut s), [teams::Job::MeetingCreate { ask, fields: asked.clone() }], "{kind}");
        assert_eq!(s.teams_meetings.asks.get(&ask).map(|a| (a.fields.clone(), a.abandoned)), Some((asked, false)), "{kind}");
        let again = s.run("event.teamsMeeting", &add("sample", calendar)).unwrap();
        assert_ne!(again["ask"].as_u64().unwrap(), ask, "{kind}");
        done(s);
    }
}

#[test]
fn adding_on_someone_elses_calendar_is_refused() {
    let theirs = [
        ("ews", "shared:boss@contoso.example", false),
        ("ews", "public:AQMkADAw", false),
        ("ews", "web:holidays", false),
        ("graph", "AAMkAGI2", true),
        ("dav", "https://dav.contoso.example/cal/boss/", true),
    ];
    for (kind, calendar, not_own) in theirs {
        let mut s = signed_in("teams-add-theirs");
        calendars(&mut s, kind, &[(calendar, not_own)]);
        assert_eq!(s.teams_meeting_path("sample", calendar), TeamsMeetingPath::Unavailable, "{calendar}");
        assert!(is_disabled(s.run("event.teamsMeeting", &add("sample", calendar))), "{calendar}");
        assert!(teams_jobs(&mut s).is_empty(), "{calendar}");
        assert!(s.teams_meetings.asks.is_empty(), "{calendar}");
        done(s);
    }
    // A calendar, or an account, nobody knows.
    let mut s = signed_in("teams-add-unknown");
    for (account, calendar) in [("sample", "nowhere"), ("nobody", LOCAL_CALENDAR)] {
        assert_eq!(s.teams_meeting_path(account, calendar), TeamsMeetingPath::Unavailable, "{account} {calendar}");
        assert!(is_disabled(s.run("event.teamsMeeting", &add(account, calendar))), "{account} {calendar}");
    }
    assert!(teams_jobs(&mut s).is_empty());
    done(s);
}

#[test]
fn too_many_asks_are_refused() {
    let mut s = signed_in("teams-add-many");
    for _ in 0..crate::TEAMS_MEETING_ASKS_MAX {
        s.run("event.teamsMeeting", &add("sample", LOCAL_CALENDAR)).unwrap();
    }
    assert_eq!(teams_jobs(&mut s).len(), crate::TEAMS_MEETING_ASKS_MAX);
    assert!(is_disabled(s.run("event.teamsMeeting", &add("sample", LOCAL_CALENDAR))), "the 17th");
    assert!(teams_jobs(&mut s).is_empty());
    assert_eq!(s.teams_meetings.asks.len(), crate::TEAMS_MEETING_ASKS_MAX);
    done(s);
}

#[test]
fn the_first_save_sends_what_changed_since_the_click() {
    let mut s = signed_in("teams-first-save");
    let ask = s.run("event.teamsMeeting", &add("sample", LOCAL_CALENDAR)).unwrap()["ask"].as_u64().unwrap();
    let Some(teams::Job::MeetingCreate { fields: clicked, .. }) = teams_jobs(&mut s).pop() else { panic!("no creation") };
    teams(&mut s, teams::Event::MeetingCreated { ask, meeting: Some(meeting(LINK)), error: None });
    assert_eq!(s.teams_meetings.sent.get("m1"), Some(&clicked), "sent holds the click's fields");
    // The window's times moved before the save.
    let saved = json!({"summary": "Budget", "start": "2026-10-21T17:00", "end": "2026-10-21T18:00", "teamsMeetingId": "m1"});
    s.run("event.create", &saved).unwrap();
    let now = TeamsMeetingFields { title: "Budget".into(), start: at(&s, "2026-10-21T17:00"), end: at(&s, "2026-10-21T18:00") };
    assert_eq!(teams_jobs(&mut s), [update("m1", &clicked, &now)]);
    assert_eq!(s.teams_meetings.sent.get("m1"), Some(&now));
    assert_eq!(s.teams_meetings.unacknowledged.get("m1"), Some(&VecDeque::from([clicked])));
    done(s);
}

#[test]
fn a_save_that_changes_nothing_sends_nothing() {
    let mut s = signed_in("teams-same-save");
    let uid = alone(&mut s);
    // No creation of ours behind it and no earlier state: nothing to compare with.
    assert!(teams_jobs(&mut s).is_empty());
    sent_as_now(&mut s, &uid, "m1");
    s.run("event.create", &json!({"summary": "Budget", "start": "2026-10-21T15:00", "end": "2026-10-21T16:00", "teamsMeetingId": "m1"})).unwrap();
    s.run("event.update", &json!({"uid": uid, "location": "Room 2"})).unwrap();
    s.run("event.update", &json!({"uid": uid, "summary": "Budget"})).unwrap();
    assert!(teams_jobs(&mut s).is_empty());
    done(s);
}

#[test]
fn changing_the_title_or_the_times_updates_the_meeting() {
    let mut s = signed_in("teams-follow");
    let uid = appointment(&mut s, "", "");
    s.take_jobs();
    let first = TeamsMeetingFields::of(&event(&s, &uid));
    s.run("event.update", &json!({"uid": uid, "summary": "Trailer review 2"})).unwrap();
    let renamed = TeamsMeetingFields { title: "Trailer review 2".into(), ..first.clone() };
    assert_eq!(teams_jobs(&mut s), [update("m1", &first, &renamed)], "from what the appointment said");
    s.run("event.move", &json!({"uid": uid, "start": "2026-10-22T09:00"})).unwrap();
    let start = at(&s, "2026-10-22T09:00");
    let moved = TeamsMeetingFields { start, end: start + (first.end - first.start), ..renamed.clone() };
    assert_eq!(teams_jobs(&mut s), [update("m1", &renamed, &moved)], "from what Teams was sent");
    s.run("event.update", &json!({"uid": uid, "end": "2026-10-22T11:00"})).unwrap();
    let longer = TeamsMeetingFields { end: at(&s, "2026-10-22T11:00"), ..moved.clone() };
    assert_eq!(teams_jobs(&mut s), [update("m1", &moved, &longer)]);
    done(s);
}

/// D16: a meeting has no zone, only its start and end as instants.
#[test]
fn changing_only_the_zone_or_the_notes_sends_nothing() {
    let mut s = signed_in("teams-zone-alone");
    s.clock.zone = Zone::named("Europe/Paris");
    let uid = alone(&mut s);
    sent_as_now(&mut s, &uid, "m1");
    s.run("event.update", &json!({"uid": uid, "description": "The budget, line by line", "location": "Room 2", "reminder": 30})).unwrap();
    assert!(teams_jobs(&mut s).is_empty());
    let before = event(&s, &uid);
    assert_eq!(before.tzid, "Europe/Paris");
    // Another zone, the same instants: the meeting has nothing to change.
    let after = CalEvent { tzid: "America/New_York".into(), ..before.clone() };
    s.update_event(&uid, |e| *e = after.clone()).unwrap();
    s.teams_meeting_follow(Some(&before), &after);
    assert!(teams_jobs(&mut s).is_empty());
    done(s);
}

/// D16: the same hours shown in another zone are other instants, which Teams is sent.
#[test]
fn a_zone_that_moves_the_times_updates_the_meeting() {
    let mut s = signed_in("teams-zone-moves");
    s.clock.zone = Zone::named("Europe/Paris");
    let uid = alone(&mut s);
    let paris = sent_as_now(&mut s, &uid, "m1");
    // 21 October: Paris is two hours ahead of UTC, New York four behind.
    let later = |f: &TeamsMeetingFields| TeamsMeetingFields { start: f.start + 6 * 3600, end: f.end + 6 * 3600, ..f.clone() };
    let before = event(&s, &uid);
    let after = CalEvent { tzid: "America/New_York".into(), start: before.start + 6 * 3600, end: before.end + 6 * 3600, ..before.clone() };
    s.update_event(&uid, |e| *e = after.clone()).unwrap();
    s.teams_meeting_follow(Some(&before), &after);
    assert_eq!(teams_jobs(&mut s), [update("m1", &paris, &later(&paris))]);
    // The same through a save, the user now in New York.
    s.clock.zone = Zone::named("America/New_York");
    s.run("event.update", &json!({"uid": uid, "start": "2026-10-21T15:00", "end": "2026-10-21T16:00"})).unwrap();
    assert_eq!(teams_jobs(&mut s), [], "15:00 in New York: where Teams has it already");
    s.run("event.update", &json!({"uid": uid, "start": "2026-10-21T09:00", "end": "2026-10-21T10:00"})).unwrap();
    assert_eq!(teams_jobs(&mut s), [update("m1", &later(&paris), &paris)], "09:00 in New York is 15:00 in Paris");
    done(s);
}

#[test]
fn undoing_a_move_updates_its_meeting() {
    let mut s = signed_in("teams-undo");
    let uid = alone(&mut s);
    let first = sent_as_now(&mut s, &uid, "m1");
    s.run("event.move", &json!({"uid": uid, "start": "2026-10-22T09:00"})).unwrap();
    let Some(teams::Job::MeetingUpdate { after: moved, .. }) = teams_jobs(&mut s).pop() else { panic!("no update") };
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(teams_jobs(&mut s), [update("m1", &moved, &first)]);
    done(s);
}

#[test]
fn deleting_or_cancelling_the_appointment_deletes_its_meeting() {
    for guests in [true, false] {
        let mut s = signed_in("teams-delete");
        let uid = if guests { appointment(&mut s, "", "") } else { alone(&mut s) };
        sent_as_now(&mut s, &uid, "m1");
        s.teams_meetings.unacknowledged.insert("m1".into(), VecDeque::from([fields()]));
        s.take_jobs();
        s.run("event.delete", &json!({"uid": uid})).unwrap();
        assert_eq!(teams_jobs(&mut s), [delete("m1")], "guests {guests}");
        assert_eq!(s.teams_meetings.deleting, 1);
        assert!(s.teams_meetings.sent.is_empty() && s.teams_meetings.unacknowledged.is_empty(), "forgotten");
        done(s);
    }
}

#[test]
fn saving_with_the_meeting_removed_deletes_it() {
    let mut s = signed_in("teams-removed");
    let uid = alone(&mut s);
    sent_as_now(&mut s, &uid, "m1");
    s.run("event.update", &json!({"uid": uid, "teamsMeetingId": ""})).unwrap();
    assert_eq!(event(&s, &uid).teams_meeting_id, None);
    assert_eq!(teams_jobs(&mut s), [delete("m1")]);
    assert!(s.teams_meetings.sent.is_empty());
    // Nothing more goes for it.
    s.run("event.update", &json!({"uid": uid, "summary": "Budget 2"})).unwrap();
    s.run("event.delete", &json!({"uid": uid})).unwrap();
    assert!(teams_jobs(&mut s).is_empty());
    done(s);
}

#[test]
fn saving_with_another_meeting_deletes_the_first() {
    let mut s = signed_in("teams-replaced");
    let made = s.run("event.create", &json!({"summary": "Budget", "start": "2026-10-21T15:00", "end": "2026-10-21T16:00", "teamsMeetingId": "A"}));
    let uid = made.unwrap()["uid"].as_str().unwrap().to_string();
    sent_as_now(&mut s, &uid, "A");
    // B was made by a click an hour before the appointment's time.
    let now = TeamsMeetingFields::of(&event(&s, &uid));
    let clicked = TeamsMeetingFields { start: now.start - 3600, end: now.end - 3600, ..now.clone() };
    s.teams_meetings.sent.insert("B".into(), clicked.clone());
    s.run("event.update", &json!({"uid": uid, "teamsMeetingId": "B"})).unwrap();
    assert_eq!(teams_jobs(&mut s), [delete("A"), update("B", &clicked, &now)]);
    assert!(!s.teams_meetings.sent.contains_key("A"));
    assert_eq!(s.teams_meetings.sent.get("B"), Some(&now), "B is followed");
    done(s);
}

#[test]
fn one_occurrence_leaves_the_series_meeting_alone() {
    let mut s = signed_in("teams-occurrence");
    let weekly = json!({"summary": "Weekly", "start": "2026-10-21T15:00", "end": "2026-10-21T16:00", "repeat": "weekly", "teamsMeetingId": "m1"});
    let uid = s.run("event.create", &weekly).unwrap()["uid"].as_str().unwrap().to_string();
    sent_as_now(&mut s, &uid, "m1");
    let second = event(&s, &uid).start + 7 * 86_400;
    s.run("event.move", &json!({"uid": uid, "occurrence": second, "start": second + 3600})).unwrap();
    s.run("event.delete", &json!({"uid": uid, "occurrence": second + 7 * 86_400})).unwrap();
    assert!(teams_jobs(&mut s).is_empty());
    s.run("event.move", &json!({"uid": uid, "start": "2026-10-22T09:00"})).unwrap();
    assert!(matches!(&teams_jobs(&mut s)[..], [teams::Job::MeetingUpdate { .. }]), "the series' own move follows");
    done(s);
}

/// A series has one meeting for all its occurrences: its start and end are the first one's.
#[test]
fn a_series_sends_its_first_occurrences_times() {
    let mut s = signed_in("teams-series");
    let weekly = json!({"summary": "Weekly", "start": "2026-10-21T15:00", "end": "2026-10-21T16:00", "repeat": "weekly", "teamsMeetingId": "m1"});
    let uid = s.run("event.create", &weekly).unwrap()["uid"].as_str().unwrap().to_string();
    let first = sent_as_now(&mut s, &uid, "m1");
    assert_eq!((first.start, first.end), (at(&s, "2026-10-21T15:00"), at(&s, "2026-10-21T16:00")));
    s.run("event.update", &json!({"uid": uid, "summary": "Weekly sync"})).unwrap();
    let renamed = TeamsMeetingFields { title: "Weekly sync".into(), ..first.clone() };
    assert_eq!(teams_jobs(&mut s), [update("m1", &first, &renamed)]);
    s.run("event.update", &json!({"uid": uid, "start": "2026-10-28T10:00", "end": "2026-10-28T11:00"})).unwrap();
    let later = TeamsMeetingFields { start: at(&s, "2026-10-28T10:00"), end: at(&s, "2026-10-28T11:00"), ..renamed.clone() };
    assert_eq!(teams_jobs(&mut s), [update("m1", &renamed, &later)], "the series starting later");
    done(s);
}

/// T3, T6: an event the user doesn't organize may carry any id, one made here among them.
#[test]
fn a_strangers_teams_id_is_never_acted_on() {
    let mut s = signed_in("teams-stranger");
    let uid = alone(&mut s);
    sent_as_now(&mut s, &uid, "m1");
    set(&mut s, &uid, |e| e.organizer = Some(stranger()));
    teams_jobs(&mut s);
    s.run("event.update", &json!({"uid": uid, "summary": "Mine now"})).unwrap();
    s.run("event.move", &json!({"uid": uid, "start": "2026-10-22T09:00"})).unwrap();
    s.run("event.update", &json!({"uid": uid, "teamsMeetingId": "m2"})).unwrap();
    s.run("event.delete", &json!({"uid": uid})).unwrap();
    assert!(teams_jobs(&mut s).is_empty());
    assert_eq!(s.teams_meetings.deleting, 0);
    done(s);
}

/// An event read with no organizer and with guests, or an answer of the user's, is a colleague's:
/// a change made here doesn't claim it.
#[test]
fn an_event_read_with_no_organizer_is_not_claimed() {
    let guest = quatrelle_model::ical::Attendee { email: "zoe@studio.example".into(), ..Default::default() };
    for (attendees, answer) in [(vec![guest.clone()], ""), (vec![guest.clone()], "ACCEPTED"), (vec![], "NEEDS-ACTION")] {
        let mut s = signed_in("teams-not-claimed");
        let uid = alone(&mut s);
        sent_as_now(&mut s, &uid, "m1");
        set(&mut s, &uid, |e| (e.attendees, e.my_answer) = (attendees, answer.into()));
        s.run("event.update", &json!({"uid": uid, "summary": "Again", "attendees": "Sam Lee <sam@studio.example>"})).unwrap();
        assert!(event(&s, &uid).organizer.is_none(), "answer {answer:?}");
        s.run("event.delete", &json!({"uid": uid})).unwrap();
        assert_eq!(teams_jobs(&mut s), [], "answer {answer:?}");
        done(s);
    }
}

/// A stranger's invitation, its file carrying a Teams id, in the Inbox and opened.
fn deliver_invitation(s: &mut Session) {
    let ics = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:planted@elsewhere.example\r\nSEQUENCE:0\r\n\
               SUMMARY:Planted\r\nDTSTART:20261021T140000Z\r\nDTEND:20261021T150000Z\r\nORGANIZER;CN=Boss:mailto:boss@elsewhere.example\r\n\
               ATTENDEE;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:riley@studio.example\r\nX-QUATRELLE-TEAMS-ID:MSpkYzE3\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let d = quatrelle_mime::Draft {
        to: vec![EmailAddress::new("Riley", "riley@studio.example")],
        subject: "Invitation: Planted".into(),
        body: "See the calendar.".into(),
        attachments: vec![quatrelle_mime::OutAttachment {
            name: "invite.ics".into(),
            mime: "text/calendar; method=REQUEST".into(),
            data: ics.as_bytes().to_vec(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let raw = quatrelle_mime::build(&d, &EmailAddress::new("Boss", "boss@elsewhere.example"), NOW, "<planted@elsewhere.example>").unwrap();
    let store = s.store.clone().unwrap();
    let mut state = store.load_folder("sample", "INBOX").unwrap();
    let key = MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: state.validity, uid: 9700 };
    state.messages.insert(0, quatrelle_mime::summary_from_full(&raw, key.clone(), quatrelle_model::Flags::default(), 0, NOW));
    store.save_folder("sample", "INBOX", &state).unwrap();
    store.write_body(&key, &raw).unwrap();
    s.forget_echoes("sample");
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
}

/// T3: a Teams id comes only from the user's own appointment; a file opened makes the user's note
/// of an event, never their meeting.
#[test]
fn a_strangers_invitation_and_files_bring_no_teams_id() {
    let ics = |method: &str| {
        format!(
            "BEGIN:VCALENDAR\r\n{method}BEGIN:VEVENT\r\nUID:inv1\r\nSUMMARY:Planted\r\nDTSTART:20261016T140000Z\r\nDTEND:20261016T150000Z\r\nX-QUATRELLE-TEAMS-ID:MSpkYzE3\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
        )
    };
    for (n, method) in ["", "METHOD:PUBLISH\r\n"].into_iter().enumerate() {
        let mut s = signed_in("teams-import");
        let path = std::env::temp_dir().join(format!("quatrelle-teams-import-{n}-{}.ics", std::process::id()));
        std::fs::write(&path, ics(method)).unwrap();
        s.run("file.add", &json!({"path": path.to_string_lossy()})).unwrap();
        let _ = std::fs::remove_file(&path);
        let planted = s.pim_mut("sample").events.iter().find(|e| e.event.summary == "Planted").map(|e| e.event.clone()).expect("imported");
        assert_eq!(planted.teams_meeting_id, None, "{method:?}");
        s.run("event.update", &json!({"uid": &planted.uid, "summary": "Planted 2", "start": "2026-10-22T09:00"})).unwrap();
        s.run("event.delete", &json!({"uid": &planted.uid})).unwrap();
        assert!(teams_jobs(&mut s).is_empty(), "{method:?}");
        done(s);
    }
    // An invitation received, accepted into the calendar.
    let mut s = signed_in("teams-invitation");
    deliver_invitation(&mut s);
    s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    let e = event(&s, "planted@elsewhere.example");
    assert_eq!(e.teams_meeting_id, None);
    s.run("event.delete", &json!({"uid": e.uid})).unwrap();
    assert!(teams_jobs(&mut s).is_empty());
    assert!(s.run("event.teamsMeeting", &json!({"remove": {"id": "MSpkYzE3"}})).is_err());
    done(s);
}

#[test]
fn an_appointment_sent_as_a_file_carries_no_teams_id() {
    let mut s = signed_in("teams-attach");
    let uid = s.run("event.create", &json!({"summary": "Budget", "start": "2026-10-21T15:00"})).unwrap()["uid"].as_str().unwrap().to_string();
    set(&mut s, &uid, |e| e.teams_meeting_id = Some("m1".into()));
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.attachItem", &json!({"kind": "event", "id": uid})).unwrap();
    let data = s.compose.as_ref().unwrap().attachments[0].data.clone();
    let text = String::from_utf8_lossy(&data);
    assert!(text.contains("SUMMARY:Budget"), "{text}");
    assert!(!text.contains("TEAMS"), "{text}");
    done(s);
}

#[test]
fn closing_during_a_creation_marks_it_abandoned() {
    let mut s = signed_in("teams-close");
    let ask = s.run("event.teamsMeeting", &add("sample", LOCAL_CALENDAR)).unwrap()["ask"].as_u64().unwrap();
    teams_jobs(&mut s);
    s.run("event.teamsMeeting", &json!({"remove": {"ask": ask}})).unwrap();
    assert!(s.teams_meetings.asks.get(&ask).is_some_and(|a| a.abandoned));
    assert!(teams_jobs(&mut s).is_empty(), "its answer deletes it");
    assert!(s.teams_meetings_deleting());
    teams(&mut s, teams::Event::MeetingCreated { ask, meeting: Some(meeting(LINK)), error: None });
    assert_eq!(teams_jobs(&mut s), [delete("m1")]);
    // Its answer was there already, and nobody took it: the meeting goes.
    asked(&mut s, false);
    teams(&mut s, teams::Event::MeetingCreated { ask: ASK, meeting: Some(meeting(LINK)), error: None });
    s.run("event.teamsMeeting", &json!({"remove": {"ask": ASK}})).unwrap();
    assert_eq!(teams_jobs(&mut s), [delete("m1")]);
    assert!(s.teams_meetings.answers.is_empty() && s.teams_meetings.sent.is_empty());
    // An ask nobody knows, or that isn't one.
    for junk in [json!(12345), json!("7"), json!(-1)] {
        assert!(is_params(s.run("event.teamsMeeting", &json!({"remove": {"ask": junk}}))), "{junk}");
    }
    assert!(teams_jobs(&mut s).is_empty());
    done(s);
}

#[test]
fn removing_a_fresh_meeting_deletes_it() {
    let mut s = signed_in("teams-remove-fresh");
    let ask = s.run("event.teamsMeeting", &add("sample", LOCAL_CALENDAR)).unwrap()["ask"].as_u64().unwrap();
    teams_jobs(&mut s);
    teams(&mut s, teams::Event::MeetingCreated { ask, meeting: Some(meeting(LINK)), error: None });
    assert!(s.teams_meeting_take_answer(ask).is_some_and(|a| a.is_ok()));
    s.run("event.teamsMeeting", &json!({"remove": {"id": "m1"}})).unwrap();
    assert_eq!(teams_jobs(&mut s), [delete("m1")]);
    assert_eq!(s.teams_meetings.deleting, 1);
    assert!(s.teams_meetings.sent.is_empty());
    assert!(is_params(s.run("event.teamsMeeting", &json!({"remove": {"id": "m1"}}))), "once");
    done(s);
}

/// S3, T6: only a meeting made here is removed by its id, never one taken from another's event.
#[test]
fn removing_an_id_not_made_here_is_refused() {
    let mut s = signed_in("teams-remove-foreign");
    let uid = appointment(&mut s, "", "");
    set(&mut s, &uid, |e| (e.organizer, e.teams_meeting_id) = (Some(stranger()), Some("MSpkYzE3".into())));
    s.take_jobs();
    for junk in [json!({"remove": {"id": "MSpkYzE3"}}), json!({"remove": {"id": "a b"}}), json!({"remove": {}}), json!({"remove": {"id": 5}})] {
        assert!(is_params(s.run("event.teamsMeeting", &junk)), "{junk}");
    }
    assert!(is_params(s.run("event.teamsMeeting", &json!({"add": {}, "remove": {"id": "m1"}}))), "one at a time");
    assert!(teams_jobs(&mut s).is_empty());
    assert_eq!(s.teams_meetings.deleting, 0);
    done(s);
}

/// Every delete asked of Teams is counted until it answers, however it went, and however it was
/// asked for: this is what closing Quatrelle waits on.
#[test]
fn a_delete_is_counted_until_teams_answers() {
    for error in [None, Some(TeamsMeetingError::Offline), Some(TeamsMeetingError::SignIn)] {
        let mut s = signed_in("teams-deleting");
        let uid = alone(&mut s);
        s.run("event.delete", &json!({"uid": uid})).unwrap();
        assert_eq!(s.teams_meetings.deleting, 1, "the appointment's meeting");
        s.teams_meetings.sent.insert("m2".into(), fields());
        s.run("event.teamsMeeting", &json!({"remove": {"id": "m2"}})).unwrap();
        assert_eq!(s.teams_meetings.deleting, 2, "a meeting its window left");
        asked(&mut s, true);
        teams(&mut s, teams::Event::MeetingCreated { ask: ASK, meeting: Some(meeting(NEW_LINK)), error: None });
        assert_eq!(s.teams_meetings.deleting, 3, "one nobody waits for any more");
        asked(&mut s, false);
        teams(&mut s, teams::Event::MeetingCreated { ask: ASK, meeting: Some(meeting(NEW_LINK)), error: None });
        s.run("event.teamsMeeting", &json!({"remove": {"ask": ASK}})).unwrap();
        assert_eq!(s.teams_meetings.deleting, 4, "an answer nobody took");
        assert!(s.teams_meetings_deleting());
        for left in [3, 2, 1, 0] {
            teams(&mut s, teams::Event::MeetingDeleted { id: "m1".into(), error: error.clone() });
            assert_eq!(s.teams_meetings.deleting, left);
        }
        assert!(!s.teams_meetings_deleting());
        done(s);
    }
}

#[test]
fn an_id_that_cannot_be_one_is_refused() {
    let mut s = signed_in("teams-bad-id");
    for bad in ["a b", "x?y", &"x".repeat(513)] {
        let made = s.run("event.create", &json!({"summary": "x", "start": "2026-10-21T15:00", "teamsMeetingId": bad}));
        assert!(is_params(made), "{bad:?}");
    }
    let uid = alone(&mut s);
    assert!(is_params(s.run("event.update", &json!({"uid": uid, "summary": "Changed", "teamsMeetingId": "a b"}))));
    let e = event(&s, &uid);
    assert_eq!((e.summary.as_str(), e.teams_meeting_id.as_deref()), ("Budget", Some("m1")), "a refused save changes nothing");
    assert!(teams_jobs(&mut s).is_empty());
    done(s);
}

/// A Teams meeting and a Webex meeting or room exclude each other: nothing falls back to the other.
#[test]
fn teams_and_webex_ids_never_go_together() {
    let mut s = signed_in("teams-and-webex");
    let both = json!({"summary": "x", "start": "2026-10-21T15:00", "teamsMeetingId": "m1", "webexId": "w1"});
    assert!(is_params(s.run("event.create", &both)));
    let room = json!({"url": "https://acme.webex.com/meet/riley"});
    assert!(is_params(s.run("event.create", &json!({"summary": "x", "start": "2026-10-21T15:00", "teamsMeetingId": "m1", "conference": room}))));
    let uid = alone(&mut s);
    assert!(is_params(s.run("event.update", &json!({"uid": uid, "webexId": "w1"}))));
    assert!(is_params(s.run("event.update", &json!({"uid": uid, "conference": room}))));
    let webex =
        s.run("event.create", &json!({"summary": "y", "start": "2026-10-21T15:00", "webexId": "w1"})).unwrap()["uid"].as_str().unwrap().to_string();
    assert!(is_params(s.run("event.update", &json!({"uid": webex, "teamsMeetingId": "m2"}))));
    assert_eq!(event(&s, &webex).teams_meeting_id, None);
    // One for the other: the Webex meeting taken out, the Teams one put in.
    s.run("event.update", &json!({"uid": webex, "webexId": "", "teamsMeetingId": "m2"})).unwrap();
    let e = event(&s, &webex);
    assert_eq!((e.webex_id, e.teams_meeting_id.as_deref()), (None, Some("m2")));
    assert!(teams_jobs(&mut s).is_empty());
    done(s);
}

/// Review Focus 2: signed out, the appointment is saved all the same; Teams is told nothing, a
/// notice says the meeting wasn't changed, and `sent` keeps what Teams has, so that the next save,
/// signed in again, sends the whole difference.
#[test]
fn signed_out_the_appointment_is_saved_and_the_meeting_waits() {
    let mut s = signed_in("teams-signed-out-update");
    let uid = alone(&mut s);
    let original = sent_as_now(&mut s, &uid, "m1");
    s.run("teams.signOut", &json!({})).unwrap();
    teams_jobs(&mut s);
    s.run("event.update", &json!({"uid": uid, "start": "2026-10-21T17:00", "end": "2026-10-21T18:00"})).unwrap();
    assert_eq!(event(&s, &uid).start, at(&s, "2026-10-21T17:00"), "saved");
    assert!(teams_jobs(&mut s).is_empty());
    let said = Some(("The Teams meeting couldn't be updated: {error}", values(&[("error", "Teams is signed out")]), true));
    assert_eq!(last_notice(&s), said);
    assert_eq!(s.teams_meetings.sent.get("m1"), Some(&original), "what Teams still has");
    let t = &mut s.prefs.teams;
    (t.signed_in, t.meetings) = (true, true);
    s.run("event.update", &json!({"uid": uid, "summary": "Budget 2"})).unwrap();
    let now = TeamsMeetingFields { title: "Budget 2".into(), start: at(&s, "2026-10-21T17:00"), end: at(&s, "2026-10-21T18:00") };
    assert_eq!(teams_jobs(&mut s), [update("m1", &original, &now)]);
    done(s);
}

/// A meeting of an earlier session isn't in `sent`: changed while signed out, it goes in with the
/// fields from before the change, and the next save sends that change with its own.
#[test]
fn signed_out_a_meeting_of_an_earlier_session_gets_the_whole_change() {
    let mut s = signed_in("teams-signed-out-earlier");
    let uid = alone(&mut s);
    let before = TeamsMeetingFields::of(&event(&s, &uid));
    assert!(s.teams_meetings.sent.is_empty(), "read with its id, as a new session has it");
    s.prefs.teams.meetings = false;
    s.run("event.update", &json!({"uid": uid, "start": "2026-10-21T17:00", "end": "2026-10-21T18:00"})).unwrap();
    assert!(teams_jobs(&mut s).is_empty());
    let said = Some(("The Teams meeting couldn't be updated: {error}", values(&[("error", "Teams meetings need your permission")]), true));
    assert_eq!(last_notice(&s), said);
    assert_eq!(s.teams_meetings.sent.get("m1"), Some(&before));
    s.prefs.teams.meetings = true;
    s.run("event.update", &json!({"uid": uid, "summary": "Budget 2"})).unwrap();
    let now = TeamsMeetingFields { title: "Budget 2".into(), start: at(&s, "2026-10-21T17:00"), end: at(&s, "2026-10-21T18:00") };
    assert_eq!(teams_jobs(&mut s), [update("m1", &before, &now)], "the time comes with the title");
    done(s);
}

#[test]
fn signed_out_a_deleted_appointment_says_its_meeting_stays() {
    let mut s = signed_in("teams-signed-out-delete");
    let uid = alone(&mut s);
    sent_as_now(&mut s, &uid, "m1");
    s.prefs.teams.on = false;
    s.run("event.delete", &json!({"uid": uid})).unwrap();
    assert!(s.find_event(&uid).is_none(), "deleted");
    assert!(teams_jobs(&mut s).is_empty());
    assert_eq!(s.teams_meetings.deleting, 0, "nothing to wait for");
    let said = Some(("The Teams meeting couldn't be deleted: {error}", values(&[("error", "Teams is signed out")]), true));
    assert_eq!(last_notice(&s), said);
    done(s);
}
