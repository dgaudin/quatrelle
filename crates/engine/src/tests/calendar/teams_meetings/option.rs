// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! File › Options › Calendar's option to make every meeting with guests a Teams meeting, as the
//! engine decides it for an appointment's window: off at first and kept; at the first guest of an
//! appointment of the user's own, in a calendar Quatrelle makes the meeting for, a meeting is
//! added once per window, unless it has one, or Webex, or the user removed Teams there; signed
//! out, or without the permission, the window says so once.

use super::*;
use crate::{TeamsAuto, TeamsAutoAsk, TeamsMeetingPath};

/// What the window of an appointment of the user's own says once its first guest is in, in a
/// calendar Quatrelle makes the meeting for.
fn ask() -> TeamsAutoAsk {
    TeamsAutoAsk { path: TeamsMeetingPath::Standalone, can_hold: true, guests: 1, has_teams: false, has_webex: false, removed: false, tried: false }
}

/// The session signed in to Teams, its meetings allowed, and the option on.
fn with_the_option(name: &str) -> Session {
    let mut s = signed_in(name);
    s.run("calendar.teamsMeetingForGuests", &json!({"on": true})).unwrap();
    s
}

#[test]
fn the_option_is_off_at_first_and_kept() {
    let mut s = signed_in("teams-option-kept");
    assert!(!s.prefs.teams_meeting_for_guests, "off at first");
    assert_eq!(s.run("calendar.teamsMeetingForGuests", &json!({"on": true})).unwrap(), json!({"on": true}));
    assert!(s.prefs.teams_meeting_for_guests);
    let back: crate::Prefs = serde_json::from_value(serde_json::to_value(&s.prefs).unwrap()).unwrap();
    assert!(back.teams_meeting_for_guests, "kept with the settings");
    assert_eq!(s.run("calendar.teamsMeetingForGuests", &json!({})).unwrap(), json!({"on": false}), "without `on`, it switches");
    assert_eq!(s.run("calendar.teamsMeetingForGuests", &json!({"on": false})).unwrap(), json!({"on": false}));
    let back: crate::Prefs = serde_json::from_value(serde_json::to_value(&s.prefs).unwrap()).unwrap();
    assert!(!back.teams_meeting_for_guests);
    // A settings file from before the option.
    let before: crate::Prefs = serde_json::from_str("{}").unwrap();
    assert!(!before.teams_meeting_for_guests);
    assert!(teams_jobs(&mut s).is_empty(), "switching it asks nothing of Teams");
    done(s);
}

#[test]
fn the_first_guest_asks_for_a_meeting() {
    let s = with_the_option("teams-option-first-guest");
    assert_eq!(s.teams_meeting_auto(&ask()), TeamsAuto::Add);
    assert_eq!(s.teams_meeting_auto(&TeamsAutoAsk { guests: 3, ..ask() }), TeamsAuto::Add, "the first guests together");
    assert_eq!(s.teams_meeting_path("sample", quatrelle_model::pim::LOCAL), TeamsMeetingPath::Standalone, "the path the window passes");
    done(s);
}

#[test]
fn not_without_a_guest_nor_on_someone_elses_event() {
    let s = with_the_option("teams-option-no-guest");
    assert_eq!(s.teams_meeting_auto(&ask()), TeamsAuto::Add, "the same window otherwise");
    assert_eq!(s.teams_meeting_auto(&TeamsAutoAsk { guests: 0, ..ask() }), TeamsAuto::No, "an appointment without guests");
    assert_eq!(s.teams_meeting_auto(&TeamsAutoAsk { can_hold: false, ..ask() }), TeamsAuto::No, "someone else's event");
    done(s);
}

#[test]
fn not_on_someone_elses_calendar_nor_yet_on_a_graph_mailbox() {
    let s = with_the_option("teams-option-calendars");
    assert_eq!(s.teams_meeting_auto(&ask()), TeamsAuto::Add, "the same window otherwise");
    assert_eq!(s.teams_meetings_usable(), Ok(()), "signed in, its meetings allowed");
    assert_eq!(s.teams_meeting_auto(&TeamsAutoAsk { path: TeamsMeetingPath::Unavailable, ..ask() }), TeamsAuto::No);
    // D13: Exchange Online is to make its own there, at step 2.
    assert_eq!(s.teams_meeting_auto(&TeamsAutoAsk { path: TeamsMeetingPath::Mailbox, ..ask() }), TeamsAuto::No);
    done(s);
}

#[test]
fn not_with_a_teams_or_webex_meeting_or_room() {
    let s = with_the_option("teams-option-has-one");
    assert_eq!(s.teams_meeting_auto(&ask()), TeamsAuto::Add, "the same window otherwise");
    assert_eq!(s.teams_meeting_auto(&TeamsAutoAsk { has_teams: true, ..ask() }), TeamsAuto::No, "a Teams meeting already");
    assert_eq!(s.teams_meeting_auto(&TeamsAutoAsk { has_webex: true, ..ask() }), TeamsAuto::No, "a Webex meeting or room");
    done(s);
}

#[test]
fn not_after_remove_in_this_window() {
    let s = with_the_option("teams-option-removed");
    assert_eq!(s.teams_meeting_auto(&ask()), TeamsAuto::Add, "the same window otherwise");
    assert_eq!(s.teams_meeting_auto(&TeamsAutoAsk { removed: true, ..ask() }), TeamsAuto::No);
    assert_eq!(s.teams_meeting_auto(&TeamsAutoAsk { removed: true, guests: 2, ..ask() }), TeamsAuto::No, "another guest neither");
    done(s);
}

#[test]
fn only_once_per_window() {
    let s = with_the_option("teams-option-once");
    assert_eq!(s.teams_meeting_auto(&ask()), TeamsAuto::Add);
    assert_eq!(s.teams_meeting_auto(&TeamsAutoAsk { tried: true, ..ask() }), TeamsAuto::No, "tried in this window");
    done(s);
}

#[test]
fn signed_out_or_without_the_permission_says_so_once() {
    let mut s = with_the_option("teams-option-signed-out");
    for (on, signed_in, meetings) in [(true, false, true), (false, true, true), (true, true, false)] {
        let t = &mut s.prefs.teams;
        (t.on, t.signed_in, t.meetings) = (on, signed_in, meetings);
        assert_eq!(s.teams_meeting_auto(&ask()), TeamsAuto::SayOnce, "{on} {signed_in} {meetings}");
        assert_eq!(s.teams_meeting_auto(&TeamsAutoAsk { tried: true, ..ask() }), TeamsAuto::No, "said once");
        assert_eq!(s.teams_meeting_auto(&TeamsAutoAsk { path: TeamsMeetingPath::Unavailable, ..ask() }), TeamsAuto::No, "nothing to say there");
    }
    assert!(teams_jobs(&mut s).is_empty());
    done(s);
}

#[test]
fn with_the_option_off_nothing_is_added() {
    let mut s = signed_in("teams-option-off");
    s.prefs.teams_meeting_for_guests = true;
    assert_eq!(s.teams_meeting_auto(&ask()), TeamsAuto::Add, "the same window with the option on");
    s.prefs.teams_meeting_for_guests = false;
    for path in [TeamsMeetingPath::Standalone, TeamsMeetingPath::Mailbox, TeamsMeetingPath::Unavailable] {
        for signed_in in [true, false] {
            s.prefs.teams.signed_in = signed_in;
            assert_eq!(s.teams_meeting_auto(&TeamsAutoAsk { path, ..ask() }), TeamsAuto::No, "{path:?} {signed_in}");
            assert_eq!(s.teams_meeting_auto(&TeamsAutoAsk { path, guests: 5, ..ask() }), TeamsAuto::No);
        }
    }
    done(s);
}
