// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Teams meetings made from the calendar, as the session keeps them: the answer waiting for the
//! window that asked, a meeting nobody waits for any more deleted, a failure said, a new link
//! written where the old one was, and the permission as the Teams worker says it. The helpers
//! here serve the files of `teams_meetings/` too.

use std::collections::VecDeque;

use quatrelle_model::ical::{CalEvent, Conference};
use quatrelle_model::job::Secret;
use quatrelle_model::pim::teams::{self, Status, TEAMS_CONFERENCE_LABEL, TeamsMeeting, TeamsMeetingError, TeamsMeetingFields};

use super::*;
use crate::TeamsMeetingAsk;

mod exchange;
mod follow;
mod mailbox;
mod option;

/// The creation the tests answer.
const ASK: u64 = 7;
/// The meeting's join link, and the one Teams gives it later.
const LINK: &str = "https://teams.microsoft.com/l/meetup-join/19%3ameeting_a%40thread.v2/0";
const NEW_LINK: &str = "https://teams.microsoft.com/l/meetup-join/19%3ameeting_b%40thread.v2/0";
/// Another Teams meeting's link, the user's own.
const OTHER_LINK: &str = "https://teams.microsoft.com/l/meetup-join/19%3ameeting_c%40thread.v2/0";
const CLIENT: &str = "00000000-0000-0000-0000-00000000c11e";
const TENANT: &str = "contoso.example";

fn fields() -> TeamsMeetingFields {
    TeamsMeetingFields { title: "Budget".into(), start: NOW, end: NOW + 3600 }
}

/// The meeting `m1`, as Teams answers it.
fn meeting(link: &str) -> TeamsMeeting {
    TeamsMeeting {
        id: "m1".into(),
        join_url: link.into(),
        meeting_id: "1234567890".into(),
        passcode: Secret("aB3xY9".into()),
        toll_number: String::new(),
        conference_id: String::new(),
        options_url: String::new(),
    }
}

/// The sample session with Teams on, signed in, and its meetings allowed.
fn signed_in(name: &str) -> Session {
    let mut s = sample_session(name);
    let t = &mut s.prefs.teams;
    (t.on, t.signed_in, t.meetings) = (true, true, true);
    t.client_id = CLIENT.into();
    t.tenant = TENANT.into();
    s
}

/// What the Teams worker says.
fn teams(s: &mut Session, event: teams::Event) {
    s.apply(Event::Teams { event });
}

/// Only the jobs that go to the Teams worker: event commands push others (syncs, mail).
fn teams_jobs(s: &mut Session) -> Vec<teams::Job> {
    s.take_jobs()
        .into_iter()
        .filter_map(|j| match j {
            Job::Teams { job } => Some(job),
            _ => None,
        })
        .collect()
}

fn asked(s: &mut Session, abandoned: bool) {
    s.teams_meetings.asks.insert(ASK, TeamsMeetingAsk { fields: fields(), abandoned });
}

/// The stored appointment changed as a sync leaves it, without a save.
fn set(s: &mut Session, uid: &str, change: impl FnOnce(&mut CalEvent)) {
    let e = s.pim_mut("sample").events.iter_mut().find(|e| e.event.uid == uid).unwrap();
    change(&mut e.event);
}

/// A meeting with a guest, its Teams meeting `m1`; its uid.
fn appointment(s: &mut Session, location: &str, notes: &str) -> String {
    let made = s.run(
        "event.create",
        &json!({"summary": "Trailer review", "start": "2026-10-21T15:00", "location": location, "description": notes,
                "attendees": "Zoe Park <zoe@studio.example>"}),
    );
    let uid = made.unwrap()["uid"].as_str().unwrap().to_string();
    set(s, &uid, |e| e.teams_meeting_id = Some("m1".into()));
    uid
}

/// A Teams meeting's conference, joined at `url`.
fn conference(url: &str) -> Option<Conference> {
    Some(Conference { url: url.into(), label: TEAMS_CONFERENCE_LABEL.into(), features: vec![] })
}

/// The notes' lines of a meeting joined at `link`.
fn block(link: &str) -> String {
    format!("Join the Teams meeting: {link}\nMeeting ID: 1234567890")
}

/// The invitations gone to the guests so far.
fn invitations(s: &Session) -> usize {
    let folder = s.store.as_ref().unwrap().load_folder("sample", "Sent").unwrap();
    folder.messages.iter().filter(|m| m.subject == "Invitation: Trailer review").count()
}

fn stranger() -> quatrelle_model::ical::Attendee {
    quatrelle_model::ical::Attendee { email: "boss@elsewhere.example".into(), name: "Boss".into(), ..Default::default() }
}

#[test]
fn an_answer_waits_for_its_window() {
    let mut s = signed_in("teams-meeting-answer");
    asked(&mut s, false);
    teams(&mut s, teams::Event::MeetingCreated { ask: ASK, meeting: Some(meeting(LINK)), error: None });
    assert!(s.teams_meetings.asks.is_empty());
    assert_eq!(s.teams_meetings.sent.get("m1"), Some(&fields()), "sent with the fields asked for");
    assert!(s.take_jobs().is_empty());
    assert_eq!(s.teams_meeting_take_answer(ASK), Some(Ok(meeting(LINK))));
    assert_eq!(s.teams_meeting_take_answer(ASK), None, "once");
    // An answer to an ask nobody made is dropped: nothing is kept for it, nothing deleted.
    teams(&mut s, teams::Event::MeetingCreated { ask: 99, meeting: Some(meeting(NEW_LINK)), error: None });
    assert!(s.teams_meetings.answers.is_empty());
    assert!(s.take_jobs().is_empty());
    done(s);
}

#[test]
fn a_failed_creation_is_told_to_its_window() {
    let mut s = signed_in("teams-meeting-create-failed");
    asked(&mut s, false);
    teams(&mut s, teams::Event::MeetingCreated { ask: ASK, meeting: None, error: Some(TeamsMeetingError::NotAllowed) });
    assert_eq!(s.teams_meetings.answers.get(&ASK), Some(&Err(TeamsMeetingError::NotAllowed)));
    assert!(s.teams_meetings.asks.is_empty() && s.teams_meetings.sent.is_empty());
    assert!(s.take_jobs().is_empty());
    assert!(s.notices.is_empty(), "the window says it");
    // An answer with neither a meeting nor an error is no answer.
    asked(&mut s, false);
    teams(&mut s, teams::Event::MeetingCreated { ask: ASK, meeting: None, error: None });
    assert_eq!(s.teams_meeting_take_answer(ASK), Some(Err(TeamsMeetingError::Offline)));
    done(s);
}

#[test]
fn a_meeting_answered_after_its_window_closed_is_deleted() {
    let mut s = signed_in("teams-meeting-abandoned");
    asked(&mut s, true);
    assert!(s.teams_meetings_deleting(), "a creation given up may still bring a meeting");
    teams(&mut s, teams::Event::MeetingCreated { ask: ASK, meeting: Some(meeting(LINK)), error: None });
    assert_eq!(teams_jobs(&mut s), [teams::Job::MeetingDelete { id: "m1".into() }]);
    assert!(s.teams_meetings.answers.is_empty() && s.teams_meetings.asks.is_empty() && s.teams_meetings.sent.is_empty());
    assert_eq!(s.teams_meetings.deleting, 1);
    assert!(s.teams_meetings_deleting());
    teams(&mut s, teams::Event::MeetingDeleted { id: "m1".into(), error: None });
    assert!(!s.teams_meetings_deleting());
    done(s);
}

#[test]
fn an_abandoned_ask_that_failed_deletes_nothing() {
    let mut s = signed_in("teams-meeting-abandoned-failed");
    asked(&mut s, true);
    let refused = TeamsMeetingError::Refused { status: 503, code: String::new() };
    teams(&mut s, teams::Event::MeetingCreated { ask: ASK, meeting: None, error: Some(refused) });
    assert!(s.take_jobs().is_empty());
    assert!(s.teams_meetings.answers.is_empty() && s.teams_meetings.asks.is_empty());
    assert!(s.notices.is_empty(), "no one is waiting to hear of it");
    assert!(!s.teams_meetings_deleting(), "it brought nothing");
    done(s);
}

#[test]
fn a_failed_update_or_delete_is_a_notice() {
    let mut s = signed_in("teams-meeting-notices");
    let uid = appointment(&mut s, LINK, "");
    let before = s.find_event(&uid).unwrap().1.clone();
    s.take_jobs();
    let last = |s: &Session| s.notices.last().map(|n| (n.text.clone(), n.error));
    let refused = TeamsMeetingError::Refused { status: 400, code: "BadRequest".into() };
    teams(&mut s, teams::Event::MeetingUpdated { id: "m1".into(), meeting: None, error: Some(refused) });
    assert_eq!(last(&s), Some(("The Teams meeting couldn't be updated: Teams answered 400: BadRequest".to_string(), true)));
    s.teams_meeting_delete("m1".into());
    assert_eq!(teams_jobs(&mut s), [teams::Job::MeetingDelete { id: "m1".into() }]);
    teams(&mut s, teams::Event::MeetingDeleted { id: "m1".into(), error: Some(TeamsMeetingError::Offline) });
    assert_eq!(last(&s), Some(("The Teams meeting couldn't be deleted: no connection".to_string(), true)));
    assert_eq!(s.teams_meetings.deleting, 0, "answered, however it went");
    assert_eq!(s.find_event(&uid).unwrap().1, &before);
    assert!(s.take_jobs().is_empty());
    let said = s.notices.len();
    teams(&mut s, teams::Event::MeetingDeleted { id: "m1".into(), error: None });
    assert_eq!(s.notices.len(), said, "a delete that went says nothing");
    assert_eq!(s.teams_meetings.deleting, 0, "an answer nobody counted");
    done(s);
}

/// A notice's values as the engine keeps them.
fn values(pairs: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
    pairs.iter().map(|(k, v)| (*k, (*v).to_string())).collect()
}

/// Graph's refusal has a wording of its own, its status and code put in as they came, so that the
/// window says the rest in its language; any other error goes in as `{error}`, which the window
/// translates as Quatrelle's words.
#[test]
fn a_refusal_is_said_by_its_status_and_code() {
    let mut s = signed_in("teams-meeting-refusal");
    let refused = |code: &str| Some(TeamsMeetingError::Refused { status: 400, code: code.into() });
    let last = |s: &Session| s.notices.last().map(|n| (n.template, n.values.clone(), n.text.clone(), n.error));
    let updated = |s: &mut Session, error| teams(s, teams::Event::MeetingUpdated { id: "m1".into(), meeting: None, error });
    let deleted = |s: &mut Session, error| teams(s, teams::Event::MeetingDeleted { id: "m1".into(), error });
    updated(&mut s, refused("BadRequest"));
    assert_eq!(
        last(&s),
        Some((
            "The Teams meeting couldn't be updated: Teams answered {status}: {code}",
            values(&[("status", "400"), ("code", "BadRequest")]),
            "The Teams meeting couldn't be updated: Teams answered 400: BadRequest".to_string(),
            true
        ))
    );
    updated(&mut s, refused(""));
    assert_eq!(
        last(&s),
        Some((
            "The Teams meeting couldn't be updated: Teams answered {status}",
            values(&[("status", "400")]),
            "The Teams meeting couldn't be updated: Teams answered 400".to_string(),
            true
        ))
    );
    deleted(&mut s, refused("BadRequest"));
    assert_eq!(
        last(&s),
        Some((
            "The Teams meeting couldn't be deleted: Teams answered {status}: {code}",
            values(&[("status", "400"), ("code", "BadRequest")]),
            "The Teams meeting couldn't be deleted: Teams answered 400: BadRequest".to_string(),
            true
        ))
    );
    deleted(&mut s, refused(""));
    assert_eq!(
        last(&s),
        Some((
            "The Teams meeting couldn't be deleted: Teams answered {status}",
            values(&[("status", "400")]),
            "The Teams meeting couldn't be deleted: Teams answered 400".to_string(),
            true
        ))
    );
    deleted(&mut s, Some(TeamsMeetingError::Offline));
    assert_eq!(
        last(&s),
        Some((
            "The Teams meeting couldn't be deleted: {error}",
            values(&[("error", "no connection")]),
            "The Teams meeting couldn't be deleted: no connection".to_string(),
            true
        ))
    );
    updated(&mut s, Some(TeamsMeetingError::NotSignedIn));
    assert_eq!(
        last(&s),
        Some((
            "The Teams meeting couldn't be updated: {error}",
            values(&[("error", "Teams is signed out")]),
            "The Teams meeting couldn't be updated: Teams is signed out".to_string(),
            true
        ))
    );
    done(s);
}

/// An update Teams didn't take leaves `sent` as Teams still has it, so that the next save sends
/// the difference again; the updates still out started from it too.
#[test]
fn a_failed_update_is_sent_again_by_the_next_save() {
    let mut s = signed_in("teams-meeting-resend");
    let original = fields();
    let renamed = TeamsMeetingFields { title: "Budget 2".into(), ..fields() };
    let moved = TeamsMeetingFields { start: NOW + 7200, end: NOW + 10_800, ..renamed.clone() };
    // Two updates on their way, as two saves leave them.
    s.teams_meetings.sent.insert("m1".into(), moved.clone());
    s.teams_meetings.unacknowledged.insert("m1".into(), VecDeque::from([original.clone(), renamed.clone()]));
    teams(&mut s, teams::Event::MeetingUpdated { id: "m1".into(), meeting: None, error: Some(TeamsMeetingError::Offline) });
    assert_eq!(s.teams_meetings.sent.get("m1"), Some(&original), "Teams still has what it had");
    assert_eq!(s.teams_meetings.unacknowledged.get("m1"), Some(&VecDeque::from([original.clone()])));
    teams(&mut s, teams::Event::MeetingUpdated { id: "m1".into(), meeting: None, error: Some(TeamsMeetingError::Offline) });
    assert_eq!(s.teams_meetings.sent.get("m1"), Some(&original), "a later failure doesn't move it forward");
    assert!(s.teams_meetings.unacknowledged.is_empty());
    // One Teams took stays sent.
    s.teams_meetings.sent.insert("m1".into(), renamed.clone());
    s.teams_meetings.unacknowledged.insert("m1".into(), VecDeque::from([original.clone()]));
    teams(&mut s, teams::Event::MeetingUpdated { id: "m1".into(), meeting: Some(meeting(LINK)), error: None });
    assert_eq!(s.teams_meetings.sent.get("m1"), Some(&renamed));
    assert!(s.teams_meetings.unacknowledged.is_empty());
    assert_eq!(s.notices.iter().filter(|n| n.error).count(), 2, "each failure said once");
    done(s);
}

#[test]
fn a_new_link_replaces_the_old_in_the_notes_the_place_and_the_conference() {
    let mut s = signed_in("teams-meeting-link");
    let uid = appointment(&mut s, LINK, &block(LINK));
    set(&mut s, &uid, |e| e.conference = conference(LINK));
    let sequence = s.find_event(&uid).unwrap().1.event.sequence;
    let told = invitations(&s);
    teams(&mut s, teams::Event::MeetingUpdated { id: "m1".into(), meeting: Some(meeting(NEW_LINK)), error: None });
    let e = &s.find_event(&uid).unwrap().1.event;
    assert_eq!((e.location.as_str(), e.description.as_str()), (NEW_LINK, block(NEW_LINK).as_str()));
    assert_eq!(e.conference, conference(NEW_LINK));
    assert_eq!(e.join_link().as_deref(), Some(NEW_LINK));
    assert_eq!(e.sequence, sequence + 1);
    assert_eq!(invitations(&s), told + 1, "the guests hear of the new link");
    // The same link again: nothing to change, no one to tell.
    teams(&mut s, teams::Event::MeetingUpdated { id: "m1".into(), meeting: Some(meeting(NEW_LINK)), error: None });
    assert_eq!(s.find_event(&uid).unwrap().1.event.sequence, sequence + 1);
    assert_eq!(invitations(&s), told + 1);
    assert!(s.notices.iter().all(|n| !n.error));
    done(s);
}

/// Another Teams meeting's link in the notes is the user's: the one replaced is the meeting's own,
/// its conference's.
#[test]
fn another_teams_link_in_the_notes_stays() {
    let mut s = signed_in("teams-meeting-other-link");
    let notes = |link: &str| format!("Last time: {OTHER_LINK}\n{}", block(link));
    let uid = appointment(&mut s, "Room 2", &notes(LINK));
    set(&mut s, &uid, |e| e.conference = conference(LINK));
    teams(&mut s, teams::Event::MeetingUpdated { id: "m1".into(), meeting: Some(meeting(NEW_LINK)), error: None });
    let e = &s.find_event(&uid).unwrap().1.event;
    assert_eq!((e.location.as_str(), e.description.as_str()), ("Room 2", notes(NEW_LINK).as_str()));
    assert_eq!(e.conference, conference(NEW_LINK));
    done(s);
}

/// The conference holds the meeting's link: only that link gives way, wherever it still is. Gone
/// from the notes and the place, it isn't looked for there in another Teams meeting's link the
/// user typed.
#[test]
fn the_meetings_own_link_alone_gives_way() {
    let mut s = signed_in("teams-meeting-own-link");
    let notes = format!("Last time: {OTHER_LINK}");
    let uid = appointment(&mut s, "Room 2", &notes);
    set(&mut s, &uid, |e| e.conference = conference(LINK));
    let told = invitations(&s);
    teams(&mut s, teams::Event::MeetingUpdated { id: "m1".into(), meeting: Some(meeting(NEW_LINK)), error: None });
    let e = &s.find_event(&uid).unwrap().1.event;
    assert_eq!((e.location.as_str(), e.description.as_str()), ("Room 2", notes.as_str()), "the user's link stays");
    assert_eq!(e.conference, conference(NEW_LINK));
    assert_eq!(invitations(&s), told + 1, "the guests hear of the new link");
    done(s);
}

/// A new link that isn't a Teams meeting's join link is never written; nor is a link the
/// appointment held that isn't one replaced (a Teams link of another kind, another service's).
#[test]
fn a_link_that_is_not_a_teams_meetings_changes_nothing() {
    let not_teams = [
        "https://teams.live.com/meet/9876543210",
        "http://teams.microsoft.com/l/meetup-join/19%3ameeting_b%40thread.v2/0",
        "https://teams.microsoft.com/l/chat/0/0?users=zoe@studio.example",
        "https://teams.microsoft.com.evil.example/l/meetup-join/19%3ameeting_b%40thread.v2/0",
        "https://teams.microsoft.com:8443/l/meetup-join/19%3ameeting_b%40thread.v2/0",
        "javascript:alert(1)",
    ];
    for new in not_teams {
        let mut s = signed_in("teams-meeting-untrusted-link");
        let uid = appointment(&mut s, LINK, &block(LINK));
        set(&mut s, &uid, |e| e.conference = conference(LINK));
        let before = s.find_event(&uid).unwrap().1.clone();
        let told = invitations(&s);
        teams(&mut s, teams::Event::MeetingUpdated { id: "m1".into(), meeting: Some(meeting(new)), error: None });
        assert_eq!(s.find_event(&uid).unwrap().1, &before, "{new}");
        assert_eq!(invitations(&s), told, "{new}");
        done(s);
    }
    for old in ["https://teams.live.com/meet/9876543210", "https://us02web.zoom.us/j/1234567890"] {
        let mut s = signed_in("teams-meeting-other-old-link");
        let uid = appointment(&mut s, old, &block(old));
        set(&mut s, &uid, |e| e.conference = conference(old));
        let before = s.find_event(&uid).unwrap().1.clone();
        let told = invitations(&s);
        teams(&mut s, teams::Event::MeetingUpdated { id: "m1".into(), meeting: Some(meeting(NEW_LINK)), error: None });
        assert_eq!(s.find_event(&uid).unwrap().1, &before, "{old}");
        assert_eq!(invitations(&s), told, "{old}");
        done(s);
    }
}

#[test]
fn a_strangers_event_takes_no_new_link() {
    let mut s = signed_in("teams-meeting-stranger-link");
    let uid = appointment(&mut s, LINK, &block(LINK));
    set(&mut s, &uid, |e| {
        e.organizer = Some(stranger());
        e.conference = conference(LINK);
    });
    let before = s.find_event(&uid).unwrap().1.clone();
    let told = invitations(&s);
    teams(&mut s, teams::Event::MeetingUpdated { id: "m1".into(), meeting: Some(meeting(NEW_LINK)), error: None });
    assert_eq!(s.find_event(&uid).unwrap().1, &before);
    assert_eq!(invitations(&s), told);
    done(s);
}

#[test]
fn the_permission_follows_what_the_worker_says() {
    let mut s = signed_in("teams-meeting-permission");
    s.prefs.teams.meetings = false;
    assert_eq!(s.teams_meetings_usable(), Err(TeamsMeetingError::Consent));
    teams(&mut s, teams::Event::Meetings { allowed: true });
    assert!(s.prefs.teams.meetings);
    assert_eq!(s.teams_meetings_usable(), Ok(()));
    assert_eq!(s.run("teams.settings", &json!({})).unwrap()["meetings"], json!(true), "said back");
    teams(&mut s, teams::Event::Meetings { allowed: false });
    assert!(!s.prefs.teams.meetings);
    assert_eq!(s.teams_meetings_usable(), Err(TeamsMeetingError::Consent));
    assert_eq!(s.run("teams.settings", &json!({})).unwrap()["meetings"], json!(false));
    // Signed out, Teams is what's missing, and the worker's word isn't taken.
    s.prefs.teams.signed_in = false;
    teams(&mut s, teams::Event::Meetings { allowed: true });
    assert!(!s.prefs.teams.meetings);
    assert_eq!(s.teams_meetings_usable(), Err(TeamsMeetingError::NotSignedIn));
    // Switched off, the same, the permission given or not.
    (s.prefs.teams.signed_in, s.prefs.teams.meetings, s.prefs.teams.on) = (true, true, false);
    assert_eq!(s.teams_meetings_usable(), Err(TeamsMeetingError::NotSignedIn));
    done(s);
}

/// Teams switched off or signed out answers nothing more: the window waiting hears that Teams
/// is signed out, nothing counts as being deleted, and an update still out counts as not taken.
#[test]
fn signing_out_or_switching_off_answers_what_waits() {
    for how in ["signOut", "off", "worker"] {
        let mut s = signed_in("teams-meeting-stopped");
        asked(&mut s, false);
        s.teams_meetings.asks.insert(ASK + 1, TeamsMeetingAsk { fields: fields(), abandoned: true });
        s.teams_meeting_delete("m2".into());
        let renamed = TeamsMeetingFields { title: "Budget 2".into(), ..fields() };
        s.teams_meetings.sent.insert("m1".into(), renamed);
        s.teams_meetings.unacknowledged.insert("m1".into(), VecDeque::from([fields()]));
        assert!(s.teams_meetings_deleting());
        match how {
            "signOut" => {
                s.run("teams.signOut", &json!({})).unwrap();
            }
            "off" => {
                s.run("teams.settings", &json!({"on": false})).unwrap();
            }
            _ => teams(&mut s, teams::Event::Status { status: Status::SignedOut }),
        }
        assert_eq!(s.teams_meeting_take_answer(ASK), Some(Err(TeamsMeetingError::NotSignedIn)), "{how}");
        assert_eq!(s.teams_meeting_take_answer(ASK + 1), None, "{how}: nobody waits for the one given up");
        assert!(s.teams_meetings.asks.is_empty(), "{how}");
        assert!(!s.teams_meetings_deleting() && s.teams_meetings.deleting == 0, "{how}");
        assert_eq!(s.teams_meetings.sent.get("m1"), Some(&fields()), "{how}: what Teams surely has");
        assert!(s.teams_meetings.unacknowledged.is_empty(), "{how}");
        done(s);
    }
}

/// « Authorize » asks for the meetings' permission with the sign-in; a plain sign-in asks for what
/// the settings say was given.
#[test]
fn authorizing_asks_the_wider_sign_in() {
    let mut s = sample_session("teams-meeting-sign-in");
    let t = &mut s.prefs.teams;
    (t.on, t.client_id, t.tenant) = (true, CLIENT.into(), TENANT.into());
    let sign_in = |s: &mut Session, p: Value| -> bool {
        s.teams.status = Status::SignedOut;
        s.run("teams.signIn", &p).unwrap();
        match &teams_jobs(s)[..] {
            [teams::Job::SignIn { client_id, tenant, meetings }] if client_id == CLIENT && tenant == TENANT => *meetings,
            jobs => panic!("{jobs:?}"),
        }
    };
    assert!(sign_in(&mut s, json!({"meetings": true})));
    assert!(!sign_in(&mut s, json!({})));
    assert!(!sign_in(&mut s, json!({"meetings": false})));
    s.prefs.teams.meetings = true;
    assert!(sign_in(&mut s, json!({})), "the settings say they're allowed");
    assert!(sign_in(&mut s, json!({"meetings": false})), "the settings say they're allowed");
    // D6: signed in before, "sign in again" (the settings still say who), and that sign-in refused
    // with nothing to go on from: the worker lets the permission go, and the next plain sign-in
    // asks for the chats' permissions alone.
    s.prefs.teams.signed_in = true;
    teams(&mut s, teams::Event::Meetings { allowed: false });
    assert!(!sign_in(&mut s, json!({})), "a refused wide sign-in isn't asked again by itself");
    done(s);
}
