// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! `event.teamsMeeting`'s `remove`, what a window leaves, when Teams can't be used or the creation
//! it waited for was answered with an error.

use super::follow::{is_params, last_notice};
use super::*;

/// A meeting made here and left by its window while Teams can't be used: the window goes all the
/// same, nothing is asked of Teams and nothing waits, a notice says the meeting stays, and it is
/// forgotten.
#[test]
fn removing_a_fresh_meeting_with_teams_unusable_says_it_stays() {
    for (on, signed, meetings, why) in [
        (false, true, true, "Teams is signed out"),
        (true, false, true, "Teams is signed out"),
        (true, true, false, "Teams meetings need your permission"),
    ] {
        let mut s = signed_in("teams-remove-unusable");
        s.teams_meetings.sent.insert("m1".into(), fields());
        let t = &mut s.prefs.teams;
        (t.on, t.signed_in, t.meetings) = (on, signed, meetings);
        s.run("event.teamsMeeting", &json!({"remove": {"id": "m1"}})).unwrap();
        assert!(teams_jobs(&mut s).is_empty(), "{why}");
        assert_eq!(s.teams_meetings.deleting, 0, "nothing to wait for");
        assert!(s.teams_meetings.sent.is_empty(), "forgotten");
        let said = Some(("The Teams meeting couldn't be deleted: {error}", values(&[("error", why)]), true));
        assert_eq!(last_notice(&s), said);
        done(s);
    }
}

/// A creation answered with an error, its window gone before taking it: leaving it asks nothing
/// and says nothing more, once.
#[test]
fn removing_a_creation_answered_with_an_error_asks_nothing() {
    let mut s = signed_in("teams-remove-failed");
    asked(&mut s, false);
    teams(&mut s, teams::Event::MeetingCreated { ask: ASK, meeting: None, error: Some(TeamsMeetingError::Offline) });
    let notices = s.notices.len();
    s.run("event.teamsMeeting", &json!({"remove": {"ask": ASK}})).unwrap();
    assert!(teams_jobs(&mut s).is_empty());
    assert_eq!((s.teams_meetings.deleting, s.notices.len()), (0, notices));
    assert!(s.teams_meeting_take_answer(ASK).is_none(), "taken");
    assert!(is_params(s.run("event.teamsMeeting", &json!({"remove": {"ask": ASK}}))), "once");
    done(s);
}
