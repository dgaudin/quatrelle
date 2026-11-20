// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A Teams meeting in a Microsoft 365 mailbox's calendar: Exchange Online makes it when the
//! appointment is saved marked (`teamsOnline`). `event.teamsMeeting` only answers that the
//! appointment is to be marked, Teams signed in or not (D15); the mark is kept only in such a
//! calendar, never goes with a meeting Quatrelle asked Teams for, can't be taken off once saved,
//! and never makes Quatrelle ask anything of Teams. File › Options › Calendar's option marks one
//! at its first guest.

use super::follow::{LOCAL_CALENDAR, add, calendars, event, is_disabled, is_params};
use super::*;
use crate::{TeamsAuto, TeamsAutoAsk, TeamsMeetingPath};

/// The mailbox's calendar, and a colleague's the account lists.
const MAILBOX: &str = "AAMkAGI2";
const THEIRS: &str = "AAMkBOSS";
/// Why the mark can't be taken off.
const KEEPS: &str = "Microsoft 365 keeps an online meeting once it is made";

/// The session signed in to Teams, the sample account read from Microsoft 365 with its mailbox's
/// calendar and a colleague's.
fn mailbox(name: &str) -> Session {
    let mut s = signed_in(name);
    calendars(&mut s, "graph", &[(MAILBOX, false), (THEIRS, true)]);
    s
}

/// An appointment with a guest, 15:00 to 16:00 on 21 October, in `calendar` (none: the account's
/// first); its uid.
fn made(s: &mut Session, calendar: Option<&str>, more: Value) -> String {
    let mut p = json!({"summary": "Budget", "start": "2026-10-21T15:00", "end": "2026-10-21T16:00", "account": "sample",
                       "attendees": "Zoe Park <zoe@studio.example>"});
    if let Some(c) = calendar {
        p["calendar"] = json!(c);
    }
    for (k, x) in more.as_object().into_iter().flatten() {
        p[k] = x.clone();
    }
    s.run("event.create", &p).unwrap()["uid"].as_str().unwrap().to_string()
}

/// A marked appointment in the mailbox's calendar; its uid.
fn marked(s: &mut Session) -> String {
    let uid = made(s, Some(MAILBOX), json!({"teamsOnline": true}));
    assert!(event(s, &uid).teams_online, "marked");
    uid
}

/// Whether a notice speaks of Teams.
fn said_of_teams(s: &Session) -> bool {
    s.notices.iter().any(|n| n.template.contains("Teams"))
}

#[test]
fn adding_on_a_graph_mailbox_only_marks() {
    let mut s = mailbox("teams-mailbox-add");
    assert_eq!(s.teams_meeting_path("sample", MAILBOX), TeamsMeetingPath::Mailbox);
    // D15: signed in or not, Teams on or off, its meetings allowed or not.
    for (on, signed_in, meetings) in [(true, true, true), (true, true, false), (true, false, false), (false, false, false)] {
        let t = &mut s.prefs.teams;
        (t.on, t.signed_in, t.meetings) = (on, signed_in, meetings);
        assert_eq!(s.run("event.teamsMeeting", &add("sample", MAILBOX)), Ok(json!({"path": "mailbox"})), "{on} {signed_in} {meetings}");
        assert!(teams_jobs(&mut s).is_empty(), "{on} {signed_in} {meetings}");
    }
    assert!(s.teams_meetings.asks.is_empty() && s.teams_meetings.answers.is_empty());
    assert!(!said_of_teams(&s));
    // As many as asked: nothing waits for Teams.
    for _ in 0..=crate::TEAMS_MEETING_ASKS_MAX {
        assert_eq!(s.run("event.teamsMeeting", &add("sample", MAILBOX)), Ok(json!({"path": "mailbox"})));
    }
    // A title and a start all the same; a colleague's calendar takes none.
    let mut untitled = add("sample", MAILBOX);
    untitled["add"]["summary"] = json!("  ");
    assert!(is_params(s.run("event.teamsMeeting", &untitled)));
    let mut no_start = add("sample", MAILBOX);
    no_start["add"].as_object_mut().unwrap().remove("start");
    assert!(is_params(s.run("event.teamsMeeting", &no_start)));
    assert_eq!(s.teams_meeting_path("sample", THEIRS), TeamsMeetingPath::Unavailable);
    assert!(is_disabled(s.run("event.teamsMeeting", &add("sample", THEIRS))));
    // The account's calendar on this computer is the user's alone: Quatrelle asks Teams there.
    assert_eq!(s.teams_meeting_path("sample", LOCAL_CALENDAR), TeamsMeetingPath::Standalone);
    assert!(teams_jobs(&mut s).is_empty());
    done(s);
}

#[test]
fn the_mark_is_kept_only_on_a_mailbox_calendar() {
    // An Exchange calendar, or one on this computer: the mark is let go, made or changed.
    for (kind, calendar) in [("ews", "calendar"), ("local", LOCAL_CALENDAR)] {
        let mut s = signed_in("teams-mark-where");
        calendars(&mut s, kind, &[(calendar, false)]);
        let uid = made(&mut s, Some(calendar), json!({"teamsOnline": true}));
        assert!(!event(&s, &uid).teams_online, "{kind}");
        s.run("event.update", &json!({"uid": uid, "teamsOnline": true})).unwrap();
        assert!(!event(&s, &uid).teams_online, "{kind}: changed");
        assert!(teams_jobs(&mut s).is_empty(), "{kind}");
        done(s);
    }
    let mut s = mailbox("teams-mark-where-graph");
    // A colleague's calendar, and the account's first (the one on this computer) when none is named.
    for calendar in [Some(THEIRS), None] {
        let uid = made(&mut s, calendar, json!({"teamsOnline": true}));
        assert!(!event(&s, &uid).teams_online, "{calendar:?}");
    }
    // The mailbox's own: kept, as it's made or once it's saved.
    marked(&mut s);
    let plain = made(&mut s, Some(MAILBOX), json!({}));
    assert!(!event(&s, &plain).teams_online);
    s.run("event.update", &json!({"uid": plain, "teamsOnline": true})).unwrap();
    assert!(event(&s, &plain).teams_online, "marked once saved");
    assert!(teams_jobs(&mut s).is_empty());
    done(s);
}

#[test]
fn once_saved_the_mark_cannot_be_taken_off() {
    let mut s = mailbox("teams-mark-kept");
    let uid = marked(&mut s);
    let refused = s.run("event.update", &json!({"uid": uid, "summary": "Changed", "teamsOnline": false}));
    assert_eq!(refused, Err(CmdError::Params(KEEPS.into())));
    let e = event(&s, &uid);
    assert_eq!((e.summary.as_str(), e.teams_online), ("Budget", true), "a refused save changes nothing");
    // A change that doesn't name it, or names it again, keeps it.
    s.run("event.update", &json!({"uid": uid, "summary": "Budget 2"})).unwrap();
    s.run("event.update", &json!({"uid": uid, "summary": "Budget 3", "teamsOnline": true})).unwrap();
    let e = event(&s, &uid);
    assert_eq!((e.summary.as_str(), e.teams_online), ("Budget 3", true));
    assert!(is_params(s.run("event.update", &json!({"uid": uid, "teamsOnline": "no"}))), "a yes or a no");
    assert!(is_params(s.run("event.create", &json!({"summary": "x", "start": "2026-10-21T15:00", "teamsOnline": 1}))));
    // `false` on an appointment that hasn't it changes nothing.
    let plain = made(&mut s, Some(MAILBOX), json!({"teamsOnline": false}));
    s.run("event.update", &json!({"uid": plain, "teamsOnline": false})).unwrap();
    assert!(!event(&s, &plain).teams_online);
    assert!(teams_jobs(&mut s).is_empty());
    done(s);
}

/// Exchange Online follows the appointment itself: saving, moving and deleting it ask nothing of
/// Teams and say nothing of it, Teams signed in or not.
#[test]
fn a_marked_event_asks_nothing_of_teams() {
    for signed_in in [true, false] {
        let mut s = mailbox("teams-mark-follows");
        s.prefs.teams.signed_in = signed_in;
        let uid = marked(&mut s);
        s.run("event.update", &json!({"uid": uid, "summary": "Budget 2", "start": "2026-10-21T17:00", "end": "2026-10-21T18:00"})).unwrap();
        s.run("event.move", &json!({"uid": uid, "start": "2026-10-22T09:00"})).unwrap();
        assert!(event(&s, &uid).teams_online, "kept through a change and a move");
        s.run("event.delete", &json!({"uid": uid})).unwrap();
        assert!(teams_jobs(&mut s).is_empty(), "{signed_in}");
        let m = &s.teams_meetings;
        assert!(m.sent.is_empty() && m.unacknowledged.is_empty() && m.deleting == 0, "{signed_in}");
        assert!(!said_of_teams(&s), "{signed_in}");
        done(s);
    }
}

/// Exchange Online's meeting and one Quatrelle asked Teams for: one or the other, whatever the
/// calendar.
#[test]
fn a_mark_and_an_id_never_go_together() {
    let mut s = mailbox("teams-mark-and-id");
    for calendar in [MAILBOX, LOCAL_CALENDAR] {
        let both = json!({"summary": "x", "start": "2026-10-21T15:00", "account": "sample", "calendar": calendar, "teamsOnline": true, "teamsMeetingId": "m1"});
        assert!(is_params(s.run("event.create", &both)), "{calendar}");
    }
    let uid = marked(&mut s);
    assert!(is_params(s.run("event.update", &json!({"uid": uid, "teamsMeetingId": "m1"}))));
    let e = event(&s, &uid);
    assert_eq!((e.teams_online, e.teams_meeting_id), (true, None));
    let with_id = made(&mut s, Some(MAILBOX), json!({"teamsMeetingId": "m2"}));
    assert!(is_params(s.run("event.update", &json!({"uid": with_id, "teamsOnline": true}))));
    let e = event(&s, &with_id);
    assert_eq!((e.teams_online, e.teams_meeting_id.as_deref()), (false, Some("m2")));
    assert!(teams_jobs(&mut s).is_empty());
    // One for the other: Quatrelle's meeting taken out goes from Teams, and the mark is put in.
    s.run("event.update", &json!({"uid": with_id, "teamsMeetingId": "", "teamsOnline": true})).unwrap();
    let e = event(&s, &with_id);
    assert_eq!((e.teams_online, e.teams_meeting_id), (true, None));
    assert_eq!(teams_jobs(&mut s), [teams::Job::MeetingDelete { id: "m2".into() }]);
    done(s);
}

/// D15: in a Microsoft 365 mailbox's calendar the option marks the appointment at its first guest,
/// whether Teams can make meetings or not; never twice, nor beside Webex.
#[test]
fn the_option_marks_a_mailbox_event_at_its_first_guest() {
    let mut s = signed_in("teams-option-mailbox");
    s.run("calendar.teamsMeetingForGuests", &json!({"on": true})).unwrap();
    let ask =
        TeamsAutoAsk { path: TeamsMeetingPath::Mailbox, can_hold: true, guests: 1, has_teams: false, has_webex: false, removed: false, tried: false };
    for (on, signed_in, meetings) in [(true, true, true), (true, false, true), (false, true, true), (true, true, false), (false, false, false)] {
        let t = &mut s.prefs.teams;
        (t.on, t.signed_in, t.meetings) = (on, signed_in, meetings);
        assert_eq!(s.teams_meeting_auto(&ask), TeamsAuto::Add, "{on} {signed_in} {meetings}");
        for (no, why) in [
            (TeamsAutoAsk { has_teams: true, ..ask }, "marked already"),
            (TeamsAutoAsk { tried: true, ..ask }, "tried in this window"),
            (TeamsAutoAsk { guests: 0, ..ask }, "no guest"),
            (TeamsAutoAsk { has_webex: true, ..ask }, "Webex"),
            (TeamsAutoAsk { removed: true, ..ask }, "removed"),
            (TeamsAutoAsk { can_hold: false, ..ask }, "someone else's"),
        ] {
            assert_eq!(s.teams_meeting_auto(&no), TeamsAuto::No, "{why}");
        }
    }
    s.prefs.teams_meeting_for_guests = false;
    assert_eq!(s.teams_meeting_auto(&ask), TeamsAuto::No, "the option off");
    assert!(teams_jobs(&mut s).is_empty());
    done(s);
}
