// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Webex's answers: what the session keeps for the window that asked, what it does with a
//! meeting nobody waits for any more, and what a failure says.

use quatrelle_model::job::Secret;
use quatrelle_model::{WebexError, WebexFields, WebexMeeting};

use super::*;
use crate::WebexAsk;

/// The creation the tests answer.
const ASK: u64 = 7;
pub(super) const OLD_LINK: &str = "https://acme.webex.com/acme/j.php?MTID=old";
pub(super) const NEW_LINK: &str = "https://acme.webex.com/acme/j.php?MTID=new";
const ZOOM: &str = "https://us02web.zoom.us/j/1234567890";
const MEET: &str = "https://meet.google.com/abc-defg-hij";
const TEAMS: &str = "https://teams.microsoft.com/l/meetup-join/19%3ameeting_abc/0";

fn fields() -> WebexFields {
    WebexFields { title: "Budget".into(), start: NOW, end: NOW + 3600, timezone: "Europe/Paris".into(), recurrence: None }
}

/// The meeting `m1`, as Webex answers it.
pub(super) fn meeting(link: &str) -> WebexMeeting {
    WebexMeeting {
        id: "m1".into(),
        web_link: link.into(),
        meeting_number: "2623456701".into(),
        password: Secret("Xy7qT2".into()),
        sip_address: "2623456701@acme.webex.com".into(),
    }
}

fn asked(s: &mut Session, quiet: bool, abandoned: bool) {
    s.webex.asks.insert(ASK, WebexAsk { fields: fields(), quiet, abandoned });
}

/// A meeting with a guest, its Webex meeting `m1`; its uid.
pub(super) fn appointment(s: &mut Session, location: &str, notes: &str) -> String {
    let made = s.run(
        "event.create",
        &json!({"summary": "Trailer review", "start": "2026-10-21T15:00", "location": location, "description": notes,
                "attendees": "Zoe Park <zoe@studio.example>"}),
    );
    let uid = made.unwrap()["uid"].as_str().unwrap().to_string();
    let e = s.pim_mut("sample").events.iter_mut().find(|e| e.event.uid == uid).unwrap();
    e.event.webex_id = Some("m1".into());
    uid
}

/// The invitations gone to the guests so far.
pub(super) fn invitations(s: &Session) -> usize {
    let folder = s.store.as_ref().unwrap().load_folder("sample", "Sent").unwrap();
    folder.messages.iter().filter(|m| m.subject == "Invitation: Trailer review").count()
}

#[test]
fn an_answer_waits_for_its_window() {
    let mut s = sample_session("webex-answer");
    asked(&mut s, false, false);
    s.apply(Event::WebexCreated { ask: ASK, meeting: Some(meeting(OLD_LINK)), error: None });
    assert_eq!(s.webex.answers.get(&ASK), Some(&Ok(meeting(OLD_LINK))));
    assert!(s.webex.asks.is_empty());
    assert_eq!(s.webex.sent.get("m1"), Some(&fields()));
    assert!(s.take_jobs().is_empty());
    done(s);
}

#[test]
fn a_meeting_answered_after_its_window_closed_is_deleted() {
    for quiet in [true, false] {
        let mut s = sample_session("webex-abandoned");
        s.prefs.webex_client_id = "C1".into();
        asked(&mut s, quiet, true);
        s.apply(Event::WebexCreated { ask: ASK, meeting: Some(meeting(OLD_LINK)), error: None });
        assert_eq!(s.take_jobs(), [Job::WebexDelete { client_id: "C1".into(), id: "m1".into(), quiet }]);
        assert!(s.webex.answers.is_empty() && s.webex.asks.is_empty() && s.webex.sent.is_empty());
        done(s);
    }
}

#[test]
fn an_abandoned_ask_that_failed_deletes_nothing() {
    let mut s = sample_session("webex-abandoned-failed");
    asked(&mut s, false, true);
    s.apply(Event::WebexCreated { ask: ASK, meeting: None, error: Some(WebexError::Forbidden) });
    assert!(s.take_jobs().is_empty());
    assert!(s.webex.answers.is_empty() && s.webex.asks.is_empty());
    assert!(s.notices.is_empty(), "no one is waiting to hear of it");
    done(s);
}

#[test]
fn a_failed_creation_is_told_to_its_window() {
    let mut s = sample_session("webex-create-failed");
    asked(&mut s, false, false);
    s.apply(Event::WebexCreated { ask: ASK, meeting: None, error: Some(WebexError::SignIn) });
    assert_eq!(s.webex.answers.get(&ASK), Some(&Err(WebexError::SignIn)));
    assert!(s.webex.asks.is_empty() && s.webex.sent.is_empty());
    assert!(s.take_jobs().is_empty());
    done(s);
}

#[test]
fn a_failed_update_or_delete_is_a_notice() {
    let mut s = sample_session("webex-notices");
    let uid = appointment(&mut s, OLD_LINK, "");
    let before = s.find_event(&uid).unwrap().1.clone();
    s.take_jobs();
    let last = |s: &Session| s.notices.last().map(|n| (n.text.clone(), n.error));
    s.apply(Event::WebexUpdated { id: "m1".into(), meeting: None, error: Some(WebexError::Forbidden) });
    let refused = "The Webex meeting couldn't be updated: Your Webex account can't schedule meetings on this site";
    assert_eq!(last(&s), Some((refused.to_string(), true)));
    s.apply(Event::WebexDeleted { id: "m1".into(), error: Some(WebexError::SignIn) });
    assert_eq!(last(&s), Some(("The Webex meeting couldn't be deleted: Sign in to Webex again".to_string(), true)));
    assert_eq!(s.find_event(&uid).unwrap().1, &before);
    assert!(s.take_jobs().is_empty());
    let said = s.notices.len();
    s.apply(Event::WebexDeleted { id: "m1".into(), error: None });
    assert_eq!(s.notices.len(), said, "a delete that went says nothing");
    done(s);
}

#[test]
fn signing_in_keeps_who_and_stops_waiting() {
    let mut s = sample_session("webex-sign-in");
    s.webex.signing_in = true;
    let refused = WebexError::Failed("the client secret isn't set".into());
    s.apply(Event::WebexSignedIn { user: None, error: Some(refused.clone()) });
    assert!(!s.webex.signing_in);
    assert_eq!(s.webex.sign_in_error, Some(refused));
    assert!(!s.prefs.webex_signed_in);
    s.webex.signing_in = true;
    s.apply(Event::WebexSignedIn { user: Some("zoe@acme.example".into()), error: None });
    assert!(!s.webex.signing_in);
    assert_eq!(s.webex.sign_in_error, None, "a sign-in that works ends the last one's error");
    assert!(s.prefs.webex_signed_in);
    assert_eq!(s.prefs.webex_user, "zoe@acme.example");
    done(s);
}

#[test]
fn every_webex_job_goes_to_no_account() {
    let (client_id, id, f) = (String::from("C1"), String::from("m1"), WebexFields::default());
    let jobs = [
        Job::WebexSignIn { client_id: client_id.clone() },
        Job::WebexSignOut,
        Job::SetWebexSecret { secret: Secret("s".into()) },
        Job::WebexCreate { ask: ASK, client_id: client_id.clone(), site: "acme.webex.com".into(), fields: f.clone(), quiet: false },
        Job::WebexUpdate { client_id: client_id.clone(), id: id.clone(), before: f.clone(), after: f, quiet: false },
        Job::WebexDelete { client_id, id, quiet: false },
    ];
    for j in jobs {
        assert_eq!(j.account(), "", "{j:?}");
    }
}

#[test]
fn changing_the_link_reaches_the_notes() {
    let mut s = sample_session("webex-link");
    let notes = |link: &str| format!("Join the Webex meeting: {link}\nMeeting number: 2623456701");
    let uid = appointment(&mut s, OLD_LINK, &notes(OLD_LINK));
    let sequence = s.find_event(&uid).unwrap().1.event.sequence;
    let told = invitations(&s);
    s.apply(Event::WebexUpdated { id: "m1".into(), meeting: Some(meeting(NEW_LINK)), error: None });
    let e = &s.find_event(&uid).unwrap().1.event;
    assert_eq!((e.location.as_str(), e.description.as_str()), (NEW_LINK, notes(NEW_LINK).as_str()));
    assert_eq!(e.join_link().as_deref(), Some(NEW_LINK));
    assert_eq!(e.sequence, sequence + 1);
    assert_eq!(invitations(&s), told + 1, "the guests hear of the new link");
    // The same link again: nothing to change, no one to tell.
    s.apply(Event::WebexUpdated { id: "m1".into(), meeting: Some(meeting(NEW_LINK)), error: None });
    assert_eq!(s.find_event(&uid).unwrap().1.event.sequence, sequence + 1);
    assert_eq!(invitations(&s), told + 1);
    done(s);
}

/// A join address the notes and the place don't hold, the server's own or Webex's: there is
/// nothing there to replace, so the guests aren't sent an update that changes nothing.
#[test]
fn a_link_the_notes_dont_hold_changes_nothing() {
    for join_url in [TEAMS, OLD_LINK] {
        let mut s = sample_session("webex-no-link");
        let uid = appointment(&mut s, "Room 2", "");
        let e = s.pim_mut("sample").events.iter_mut().find(|e| e.event.uid == uid).unwrap();
        e.event.join_url = join_url.into();
        let before = s.find_event(&uid).unwrap().1.clone();
        let told = invitations(&s);
        s.apply(Event::WebexUpdated { id: "m1".into(), meeting: Some(meeting(NEW_LINK)), error: None });
        s.apply(Event::WebexUpdated { id: "elsewhere".into(), meeting: Some(meeting(NEW_LINK)), error: None });
        assert_eq!(s.find_event(&uid).unwrap().1, &before, "{join_url}");
        assert_eq!(invitations(&s), told, "{join_url}");
        assert!(s.notices.iter().all(|n| !n.error));
        done(s);
    }
}

/// Webex answers every change with the meeting's link, most often the one it had: a title
/// changed mustn't touch a meeting link of another kind that comes first, nor raise the
/// appointment's sequence, nor mail its guests.
#[test]
fn the_same_link_answered_changes_nothing() {
    let block = format!("Join the Webex meeting: {OLD_LINK}");
    for (place, notes, join_url) in [
        (ZOOM, block.clone(), ""),
        ("https://acme.webex.com/meet/ana", block.clone(), ""),
        ("Room 2", format!("{MEET}\n{block}"), ""),
        ("Room 2", format!("{TEAMS}\n{block}"), TEAMS),
        ("Room 2", format!("Last time: https://acme.webex.com/acme/j.php?MTID=last\n{block}"), ""),
    ] {
        let mut s = sample_session("webex-same-link");
        let uid = appointment(&mut s, place, &notes);
        let e = s.pim_mut("sample").events.iter_mut().find(|e| e.event.uid == uid).unwrap();
        e.event.join_url = join_url.into();
        let before = s.find_event(&uid).unwrap().1.clone();
        let told = invitations(&s);
        s.apply(Event::WebexUpdated { id: "m1".into(), meeting: Some(meeting(OLD_LINK)), error: None });
        assert_eq!(s.find_event(&uid).unwrap().1, &before, "{place}: {notes}");
        assert_eq!(invitations(&s), told, "{place}: {notes}");
        done(s);
    }
}

/// Webex's link did change: its own is replaced, and the links around it stay.
#[test]
fn a_new_link_replaces_webexs_own_alone() {
    let mut s = sample_session("webex-own-link");
    let uid = appointment(&mut s, ZOOM, &format!("{MEET}\nJoin the Webex meeting: {OLD_LINK}"));
    let told = invitations(&s);
    s.apply(Event::WebexUpdated { id: "m1".into(), meeting: Some(meeting(NEW_LINK)), error: None });
    let e = &s.find_event(&uid).unwrap().1.event;
    assert_eq!(e.location, ZOOM);
    assert_eq!(e.description, format!("{MEET}\nJoin the Webex meeting: {NEW_LINK}"));
    assert_eq!(invitations(&s), told + 1);
    done(s);
}

/// Only the jobs that go to Webex: event commands push others (syncs, mail).
fn webex_jobs(s: &mut Session) -> Vec<Job> {
    s.take_jobs()
        .into_iter()
        .filter(|j| {
            matches!(
                j,
                Job::WebexSignIn { .. }
                    | Job::WebexSignOut
                    | Job::SetWebexSecret { .. }
                    | Job::WebexCreate { .. }
                    | Job::WebexUpdate { .. }
                    | Job::WebexDelete { .. }
            )
        })
        .collect()
}

/// A session signed in to Webex on `acme.webex.com`.
fn signed_in(name: &str) -> Session {
    let mut s = sample_session(name);
    s.prefs.webex_site = "acme.webex.com".into();
    s.prefs.webex_client_id = "C1".into();
    s.prefs.webex_signed_in = true;
    s
}

fn is_params<T: std::fmt::Debug>(r: Result<T, CmdError>) -> bool {
    matches!(r, Err(CmdError::Params(_)))
}

/// An appointment without guests, its Webex meeting `m1`; its uid.
fn alone(s: &mut Session) -> String {
    let made = s.run("event.create", &json!({"summary": "Budget", "start": "2026-10-21T15:00", "end": "2026-10-21T16:00", "webexId": "m1"}));
    made.unwrap()["uid"].as_str().unwrap().to_string()
}

fn stranger() -> quatrelle_model::ical::Attendee {
    quatrelle_model::ical::Attendee { email: "boss@elsewhere.example".into(), name: "Boss".into(), ..Default::default() }
}

fn set_organizer(s: &mut Session, uid: &str, organizer: Option<quatrelle_model::ical::Attendee>) {
    let e = s.pim_mut("sample").events.iter_mut().find(|e| e.event.uid == uid).unwrap();
    e.event.organizer = organizer;
}

#[test]
fn settings_keep_the_site_as_a_host() {
    let mut s = sample_session("webex-settings");
    s.run("webex.settings", &json!({"site": "https://acme.webex.com/", "clientId": "C1.x-y_z"})).unwrap();
    assert_eq!((s.prefs.webex_site.as_str(), s.prefs.webex_client_id.as_str()), ("acme.webex.com", "C1.x-y_z"));
    for bad in ["not a site", "localhost"] {
        assert!(is_params(s.run("webex.settings", &json!({"site": bad}))), "{bad}");
    }
    for bad in ["has space", &"x".repeat(201), "é"] {
        assert!(is_params(s.run("webex.settings", &json!({"clientId": bad}))), "{bad}");
    }
    assert_eq!(s.prefs.webex_site, "acme.webex.com", "a refused one changes nothing");
    assert!(webex_jobs(&mut s).is_empty());
    s.run("webex.settings", &json!({"secret": "hunter2"})).unwrap();
    assert_eq!(webex_jobs(&mut s), [Job::SetWebexSecret { secret: Secret("hunter2".into()) }]);
    s.run("webex.settings", &json!({"secret": ""})).unwrap();
    assert_eq!(webex_jobs(&mut s), [Job::SetWebexSecret { secret: Secret(String::new()) }]);
    done(s);
}

#[test]
fn signing_in_needs_a_client_id_and_waits() {
    let mut s = sample_session("webex-sign-in-cmd");
    assert!(s.run("webex.signIn", &json!({})).is_err());
    assert!(webex_jobs(&mut s).is_empty());
    s.prefs.webex_client_id = "C1".into();
    s.webex.sign_in_error = Some(WebexError::SignIn);
    s.run("webex.signIn", &json!({})).unwrap();
    assert_eq!(webex_jobs(&mut s), [Job::WebexSignIn { client_id: "C1".into() }]);
    assert!(s.webex.signing_in);
    assert_eq!(s.webex.sign_in_error, None);
    assert!(s.run("webex.signIn", &json!({})).is_err(), "one at a time");
    assert!(webex_jobs(&mut s).is_empty());
    done(s);
}

#[test]
fn signing_out_forgets_who() {
    let mut s = signed_in("webex-sign-out");
    s.prefs.webex_user = "zoe@acme.example".into();
    s.run("webex.signOut", &json!({})).unwrap();
    assert_eq!(webex_jobs(&mut s), [Job::WebexSignOut]);
    assert!(!s.prefs.webex_signed_in && s.prefs.webex_user.is_empty());
    done(s);
}

#[test]
fn creating_needs_a_title_a_start_a_site_and_a_sign_in() {
    let ok = json!({"summary": "Budget", "start": "2026-10-21T15:00"});
    let mut s = signed_in("webex-create-needs");
    assert!(is_params(s.run("webex.createMeeting", &json!({"summary": " ", "start": "2026-10-21T15:00"}))));
    assert!(is_params(s.run("webex.createMeeting", &json!({"summary": "Budget"}))));
    s.prefs.webex_site.clear();
    assert!(matches!(s.run("webex.createMeeting", &ok), Err(CmdError::Disabled(_))));
    s.prefs.webex_site = "acme.webex.com".into();
    s.prefs.webex_signed_in = false;
    assert!(matches!(s.run("webex.createMeeting", &ok), Err(CmdError::Disabled(_))));
    assert!(webex_jobs(&mut s).is_empty());
    assert!(s.webex.asks.is_empty());
    done(s);
}

#[test]
fn creating_asks_webex_with_the_appointments_fields() {
    let mut s = signed_in("webex-create");
    s.clock.zone = quatrelle_model::zone::Zone::named("Europe/Paris");
    let made = s
        .run(
            "webex.createMeeting",
            &json!({"summary": "Budget", "start": "2026-10-21T15:00", "end": "2026-10-21T16:30", "repeat": "weekly", "quiet": true}),
        )
        .unwrap();
    let ask = made["ask"].as_u64().unwrap();
    let jobs = webex_jobs(&mut s);
    let [Job::WebexCreate { ask: a, client_id, site, fields, quiet }] = &jobs[..] else { panic!("{jobs:?}") };
    assert_eq!((*a, client_id.as_str(), site.as_str(), *quiet), (ask, "C1", "acme.webex.com", true));
    assert_eq!(fields.title, "Budget");
    assert_eq!(fields.end - fields.start, 5400);
    assert_eq!(fields.timezone, "Europe/Paris");
    assert_eq!(fields.recurrence.as_deref(), Some("FREQ=WEEKLY"));
    assert_eq!(s.webex.asks.get(&ask).map(|x| (x.fields.clone(), x.quiet, x.abandoned)), Some((fields.clone(), true, false)));
    // The answer is the window's to take, once.
    s.apply(Event::WebexCreated { ask, meeting: Some(meeting(OLD_LINK)), error: None });
    assert_eq!(s.webex_take_answer(ask), Some(Ok(meeting(OLD_LINK))));
    assert_eq!(s.webex_take_answer(ask), None);
    let again = s.run("webex.createMeeting", &json!({"summary": "Budget", "start": "2026-10-21T15:00"})).unwrap();
    assert_ne!(again["ask"].as_u64().unwrap(), ask);
    assert!(matches!(&webex_jobs(&mut s)[..], [Job::WebexCreate { quiet: false, .. }]));
    done(s);
}

#[test]
fn the_first_save_sends_what_changed_since_the_click() {
    let mut s = signed_in("webex-first-save");
    let made = s.run("webex.createMeeting", &json!({"summary": "Budget", "start": "2026-10-21T15:00", "end": "2026-10-21T16:00"})).unwrap();
    let ask = made["ask"].as_u64().unwrap();
    let Some(Job::WebexCreate { fields: clicked, .. }) = webex_jobs(&mut s).pop() else { panic!("no creation") };
    s.apply(Event::WebexCreated { ask, meeting: Some(meeting(OLD_LINK)), error: None });
    assert_eq!(s.webex.sent.get("m1"), Some(&clicked), "sent holds the click's fields");
    // The window's times moved before the save.
    s.run("event.create", &json!({"summary": "Budget", "start": "2026-10-21T17:00", "end": "2026-10-21T18:00", "webexId": "m1"})).unwrap();
    let jobs = webex_jobs(&mut s);
    let [Job::WebexUpdate { client_id, id, before, after, quiet }] = &jobs[..] else { panic!("{jobs:?}") };
    assert_eq!((client_id.as_str(), id.as_str(), *quiet), ("C1", "m1", false));
    assert_eq!(before, &clicked);
    assert_eq!(after.start, clicked.start + 7200);
    assert_eq!(s.webex.sent.get("m1"), Some(after));
    done(s);
}

#[test]
fn a_save_that_changes_nothing_sends_nothing() {
    let mut s = signed_in("webex-same-save");
    let uid = alone(&mut s);
    // No creation of ours behind it and no earlier state: nothing to compare with.
    assert!(webex_jobs(&mut s).is_empty());
    s.webex.sent.insert("m1".into(), WebexFields::of(&s.find_event(&uid).unwrap().1.event));
    s.run("event.create", &json!({"summary": "Budget", "start": "2026-10-21T15:00", "end": "2026-10-21T16:00", "webexId": "m1"})).unwrap();
    s.run("event.update", &json!({"uid": uid, "location": "Room 2"})).unwrap();
    s.run("event.update", &json!({"uid": uid, "summary": "Budget"})).unwrap();
    assert!(webex_jobs(&mut s).is_empty());
    done(s);
}

#[test]
fn changing_an_appointment_updates_its_meeting() {
    let mut s = signed_in("webex-follow");
    let uid = appointment(&mut s, "", "");
    s.take_jobs();
    let before = WebexFields::of(&s.find_event(&uid).unwrap().1.event);
    s.run("event.update", &json!({"uid": uid, "summary": "Trailer review 2"})).unwrap();
    let jobs = webex_jobs(&mut s);
    let [Job::WebexUpdate { id, before: b, after, quiet, .. }] = &jobs[..] else { panic!("{jobs:?}") };
    assert_eq!((id.as_str(), *quiet), ("m1", true), "with guests");
    assert_eq!((b, after.title.as_str()), (&before, "Trailer review 2"));
    s.run("event.move", &json!({"uid": uid, "start": "2026-10-22T09:00"})).unwrap();
    let jobs = webex_jobs(&mut s);
    let [Job::WebexUpdate { before: b, after, .. }] = &jobs[..] else { panic!("{jobs:?}") };
    assert_eq!(b.title, "Trailer review 2");
    assert_eq!(after.start - b.start, 18 * 3600);
    assert_eq!(after.end - after.start, b.end - b.start);
    done(s);
}

#[test]
fn undoing_a_move_updates_its_meeting() {
    let mut s = signed_in("webex-undo");
    let uid = alone(&mut s);
    let start = s.find_event(&uid).unwrap().1.event.start;
    s.run("event.move", &json!({"uid": uid, "start": "2026-10-22T09:00"})).unwrap();
    assert_eq!(webex_jobs(&mut s).len(), 1);
    s.run("edit.undo", &json!({})).unwrap();
    let jobs = webex_jobs(&mut s);
    let [Job::WebexUpdate { after, .. }] = &jobs[..] else { panic!("{jobs:?}") };
    assert_eq!(after.start, start);
    done(s);
}

#[test]
fn deleting_an_appointment_deletes_its_meeting() {
    for (guests, quiet) in [(true, true), (false, false)] {
        let mut s = signed_in("webex-delete");
        let uid = if guests { appointment(&mut s, "", "") } else { alone(&mut s) };
        webex_jobs(&mut s);
        s.run("event.delete", &json!({"uid": uid})).unwrap();
        assert_eq!(webex_jobs(&mut s), [Job::WebexDelete { client_id: "C1".into(), id: "m1".into(), quiet }]);
        done(s);
    }
}

#[test]
fn a_strangers_webex_id_is_never_acted_on() {
    let mut s = signed_in("webex-stranger");
    let uid = alone(&mut s);
    set_organizer(&mut s, &uid, Some(stranger()));
    webex_jobs(&mut s);
    s.run("event.update", &json!({"uid": uid, "summary": "Mine now"})).unwrap();
    s.run("event.move", &json!({"uid": uid, "start": "2026-10-22T09:00"})).unwrap();
    s.run("event.delete", &json!({"uid": uid})).unwrap();
    assert!(webex_jobs(&mut s).is_empty());
    done(s);
}

#[test]
fn a_strangers_event_takes_no_new_link() {
    let mut s = signed_in("webex-stranger-link");
    let uid = appointment(&mut s, OLD_LINK, &format!("Join the Webex meeting: {OLD_LINK}"));
    set_organizer(&mut s, &uid, Some(stranger()));
    let before = s.find_event(&uid).unwrap().1.clone();
    let told = invitations(&s);
    s.apply(Event::WebexUpdated { id: "m1".into(), meeting: Some(meeting(NEW_LINK)), error: None });
    assert_eq!(s.find_event(&uid).unwrap().1, &before);
    assert_eq!(invitations(&s), told);
    done(s);
}

#[test]
fn one_occurrence_leaves_the_series_meeting_alone() {
    let mut s = signed_in("webex-occurrence");
    let made = s
        .run(
            "event.create",
            &json!({"summary": "Weekly", "start": "2026-10-21T15:00", "end": "2026-10-21T16:00", "repeat": "weekly", "webexId": "m1"}),
        )
        .unwrap();
    let uid = made["uid"].as_str().unwrap().to_string();
    let start = s.find_event(&uid).unwrap().1.event.start;
    let second = start + 7 * 86_400;
    webex_jobs(&mut s);
    s.run("event.move", &json!({"uid": uid, "occurrence": second, "start": second + 3600})).unwrap();
    s.run("event.delete", &json!({"uid": uid, "occurrence": second + 7 * 86_400})).unwrap();
    assert!(webex_jobs(&mut s).is_empty());
    s.run("event.move", &json!({"uid": uid, "start": "2026-10-22T09:00"})).unwrap();
    assert!(matches!(&webex_jobs(&mut s)[..], [Job::WebexUpdate { .. }]), "the series' own move follows");
    done(s);
}

#[test]
fn no_meeting_no_webex() {
    let mut s = signed_in("webex-none");
    let made = s.run("event.create", &json!({"summary": "Lunch", "start": "2026-10-21T12:00"})).unwrap();
    let uid = made["uid"].as_str().unwrap().to_string();
    s.run("event.update", &json!({"uid": uid, "summary": "Late lunch"})).unwrap();
    s.run("event.move", &json!({"uid": uid, "start": "2026-10-22T09:00"})).unwrap();
    s.run("event.delete", &json!({"uid": uid})).unwrap();
    assert!(webex_jobs(&mut s).is_empty());
    done(s);
}

#[test]
fn closing_during_a_creation_marks_it_abandoned() {
    let mut s = signed_in("webex-close");
    asked(&mut s, false, false);
    s.run("webex.deleteMeeting", &json!({"ask": ASK})).unwrap();
    assert!(s.webex.asks.get(&ASK).is_some_and(|a| a.abandoned));
    assert!(webex_jobs(&mut s).is_empty());
    // Its answer was there already, and nobody took it: the meeting goes.
    s.webex.answers.insert(9, Ok(meeting(OLD_LINK)));
    s.run("webex.deleteMeeting", &json!({"ask": 9})).unwrap();
    assert_eq!(webex_jobs(&mut s), [Job::WebexDelete { client_id: "C1".into(), id: "m1".into(), quiet: true }]);
    assert!(s.webex.answers.is_empty());
    // An ask nobody knows is not an error to the window closing.
    s.run("webex.deleteMeeting", &json!({"ask": 12345})).unwrap();
    assert!(webex_jobs(&mut s).is_empty());
    done(s);
}

#[test]
fn updating_and_deleting_a_meeting_by_id() {
    let mut s = signed_in("webex-by-id");
    s.webex.sent.insert("m1".into(), fields());
    s.run("webex.updateMeeting", &json!({"id": "m1", "summary": "Budget", "start": "2026-10-21T15:00", "quiet": true})).unwrap();
    let jobs = webex_jobs(&mut s);
    let [Job::WebexUpdate { id, before, after, quiet, .. }] = &jobs[..] else { panic!("{jobs:?}") };
    assert_eq!((id.as_str(), before, *quiet), ("m1", &fields(), true));
    assert_eq!(s.webex.sent.get("m1"), Some(after));
    assert!(is_params(s.run("webex.updateMeeting", &json!({"id": "a b", "summary": "x", "start": "2026-10-21T15:00"}))));
    assert!(is_params(s.run("webex.deleteMeeting", &json!({"id": "a b"}))));
    s.run("webex.deleteMeeting", &json!({"id": "m1"})).unwrap();
    assert_eq!(webex_jobs(&mut s), [Job::WebexDelete { client_id: "C1".into(), id: "m1".into(), quiet: false }]);
    assert!(s.webex.sent.is_empty());
    assert!(is_params(s.run("webex.deleteMeeting", &json!({}))));
    done(s);
}

/// Every delete asked of Webex is counted until it answers, however it went, and however it was
/// asked for: this is what closing Quatrelle waits on.
#[test]
fn a_delete_is_counted_until_webex_answers() {
    for error in [None, Some(WebexError::Forbidden), Some(WebexError::SignIn)] {
        let mut s = signed_in("webex-deleting");
        let uid = alone(&mut s);
        s.run("event.delete", &json!({"uid": uid})).unwrap();
        assert_eq!(s.webex.deleting, 1, "the appointment's meeting");
        s.run("webex.deleteMeeting", &json!({"id": "m2"})).unwrap();
        assert_eq!(s.webex.deleting, 2, "a meeting its window left");
        asked(&mut s, true, true);
        s.apply(Event::WebexCreated { ask: ASK, meeting: Some(meeting(NEW_LINK)), error: None });
        assert_eq!(s.webex.deleting, 3, "one nobody waits for any more");
        s.webex.answers.insert(9, Ok(meeting(NEW_LINK)));
        s.run("webex.deleteMeeting", &json!({"ask": 9})).unwrap();
        assert_eq!(s.webex.deleting, 4, "an answer nobody took");
        assert!(s.webex_deleting());
        for left in [3, 2, 1, 0] {
            s.apply(Event::WebexDeleted { id: "m1".into(), error: error.clone() });
            assert_eq!(s.webex.deleting, left);
        }
        assert!(!s.webex_deleting());
        s.apply(Event::WebexDeleted { id: "m1".into(), error: None });
        assert_eq!(s.webex.deleting, 0, "an answer nobody counted");
        done(s);
    }
}

/// A creation given up may still bring a meeting to delete; refused or signed out, nothing is out.
#[test]
fn only_what_is_on_its_way_counts_as_deleting() {
    let mut s = signed_in("webex-deleting-asks");
    asked(&mut s, true, false);
    assert!(!s.webex_deleting(), "its window still waits for it");
    s.run("webex.deleteMeeting", &json!({"ask": ASK})).unwrap();
    assert!(s.webex_deleting(), "given up");
    s.apply(Event::WebexCreated { ask: ASK, meeting: None, error: Some(WebexError::Forbidden) });
    assert!(!s.webex_deleting(), "it brought nothing");
    s.run("webex.signOut", &json!({})).unwrap();
    assert!(s.run("webex.deleteMeeting", &json!({"id": "m1"})).is_err());
    assert!(!s.webex_deleting() && s.webex.deleting == 0, "signed out, nothing went");
    done(s);
}

#[test]
fn a_foreign_id_is_refused() {
    let mut s = signed_in("webex-foreign-id");
    for bad in ["a b", "x/y", &"x".repeat(201)] {
        let made = s.run("event.create", &json!({"summary": "x", "start": "2026-10-21T15:00", "webexId": bad}));
        assert!(is_params(made), "{bad:?}");
    }
    done(s);
}

/// A lone appointment given guests is still the user's: its meeting follows the change, and goes
/// with it.
#[test]
fn a_lone_appointment_given_guests_keeps_its_meeting() {
    let mut s = signed_in("webex-gains-guests");
    let uid = alone(&mut s);
    webex_jobs(&mut s);
    s.run("event.update", &json!({"uid": uid, "attendees": "Zoe Park <zoe@studio.example>", "start": "2026-10-22T09:00"})).unwrap();
    let jobs = webex_jobs(&mut s);
    let [Job::WebexUpdate { id, quiet, .. }] = &jobs[..] else { panic!("{jobs:?}") };
    assert_eq!((id.as_str(), *quiet), ("m1", true), "its guests hear from the invitation");
    s.run("event.delete", &json!({"uid": uid})).unwrap();
    assert_eq!(webex_jobs(&mut s), [Job::WebexDelete { client_id: "C1".into(), id: "m1".into(), quiet: true }]);
    done(s);
}

/// Send Update on a lone appointment given guests does what Send does on a new meeting: the user
/// organizes it, and the guests are invited, here by mail. Only its sequence tells them apart.
#[test]
fn a_lone_appointment_given_guests_invites_them_as_a_new_meeting_does() {
    let mut s = signed_in("webex-gains-guests-invited");
    let new = appointment(&mut s, "", "");
    assert_eq!(invitations(&s), 1);
    let new = s.find_event(&new).unwrap().1.event.clone();
    let made = s.run("event.create", &json!({"summary": "Trailer review", "start": "2026-10-21T15:00"})).unwrap();
    let uid = made["uid"].as_str().unwrap().to_string();
    assert_eq!(invitations(&s), 1, "nobody to invite yet");
    s.run("event.update", &json!({"uid": uid, "attendees": "Zoe Park <zoe@studio.example>"})).unwrap();
    assert_eq!(invitations(&s), 2);
    let given = s.find_event(&uid).unwrap().1.event.clone();
    assert_eq!(given.organizer, new.organizer);
    assert_eq!((new.sequence, given.sequence), (0, 1));
    done(s);
}

/// A room is a guest for this: booked on a lone appointment, it makes the user the organizer too.
#[test]
fn a_lone_appointment_given_a_room_keeps_its_meeting() {
    let mut s = signed_in("webex-gains-a-room");
    let uid = alone(&mut s);
    webex_jobs(&mut s);
    s.run("event.update", &json!({"uid": uid, "rooms": "Sound Lab <sound.lab@studio.example>", "summary": "Budget 2"})).unwrap();
    assert_eq!(webex_jobs(&mut s).len(), 1);
    let me = s.find_event(&uid).unwrap().1.event.organizer.clone().expect("the user organizes it");
    assert_eq!(me.email, "riley@studio.example");
    done(s);
}

/// An event read with no organizer and with guests, or an answer of the user's, is a colleague's:
/// a change made here doesn't claim it.
#[test]
fn an_event_read_with_no_organizer_is_not_claimed() {
    let guest = quatrelle_model::ical::Attendee { email: "zoe@studio.example".into(), ..Default::default() };
    for (attendees, answer) in [(vec![guest.clone()], ""), (vec![guest.clone()], "ACCEPTED"), (vec![], "NEEDS-ACTION")] {
        let mut s = signed_in("webex-not-claimed");
        let uid = alone(&mut s);
        webex_jobs(&mut s);
        let e = s.pim_mut("sample").events.iter_mut().find(|e| e.event.uid == uid).unwrap();
        (e.event.attendees, e.event.my_answer) = (attendees, answer.into());
        s.run("event.update", &json!({"uid": uid, "summary": "Again", "attendees": "Sam Lee <sam@studio.example>"})).unwrap();
        assert!(s.find_event(&uid).unwrap().1.event.organizer.is_none(), "answer {answer:?}");
        assert_eq!(webex_jobs(&mut s), [], "answer {answer:?}");
        done(s);
    }
}

#[test]
fn no_organizer_is_the_users_only_for_a_lone_appointment() {
    let guest = quatrelle_model::ical::Attendee { email: "zoe@studio.example".into(), ..Default::default() };
    // (organizer, guests, the user's answer, whose it is): what the sync leaves of the user's own
    // appointment, and of a colleague's meeting from a server that names no organizer.
    let cases = [
        (None, vec![], "", true),
        (None, vec![guest.clone()], "ACCEPTED", false),
        (None, vec![guest.clone()], "", false),
        (None, vec![], "NEEDS-ACTION", false),
        (Some(stranger()), vec![], "", false),
        (Some(quatrelle_model::ical::Attendee::default()), vec![], "", false),
    ];
    for (n, (organizer, attendees, answer, mine)) in cases.into_iter().enumerate() {
        let mut s = signed_in("webex-no-organizer");
        let uid = alone(&mut s);
        webex_jobs(&mut s);
        let e = s.pim_mut("sample").events.iter_mut().find(|e| e.event.uid == uid).unwrap();
        (e.event.organizer, e.event.attendees, e.event.my_answer) = (organizer, attendees, answer.into());
        let without_id = quatrelle_model::ical::CalEvent { webex_id: None, ..e.event.clone() };
        assert_eq!(s.webex_can_hold(&without_id), mine, "case {n}");
        let occurrence = quatrelle_model::ical::CalEvent { recurrence_id: Some(NOW), ..without_id };
        assert!(!s.webex_can_hold(&occurrence), "case {n}");
        s.run("event.move", &json!({"uid": uid, "start": "2026-10-22T09:00", "send": false})).unwrap();
        assert_eq!(webex_jobs(&mut s).len(), usize::from(mine), "case {n}");
        done(s);
    }
    // The user's own meeting, read back with its organizer filled in.
    let mut s = signed_in("webex-own-organizer");
    let uid = appointment(&mut s, "", "");
    let me = s.find_event(&uid).unwrap().1.event.organizer.clone();
    assert!(me.is_some());
    webex_jobs(&mut s);
    s.run("event.update", &json!({"uid": uid, "summary": "Again"})).unwrap();
    assert_eq!(webex_jobs(&mut s).len(), 1);
    done(s);
}

#[test]
fn a_strangers_invitation_and_files_bring_no_webex_id() {
    let ics = |method: &str| {
        format!(
            "BEGIN:VCALENDAR\r\n{method}BEGIN:VEVENT\r\nUID:inv1\r\nSUMMARY:Planted\r\nDTSTART:20261016T140000Z\r\nDTEND:20261016T150000Z\r\nX-QUATRELLE-WEBEX-ID:evil\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
        )
    };
    // An imported file, with or without a method.
    for (n, method) in ["", "METHOD:PUBLISH\r\n"].into_iter().enumerate() {
        let mut s = signed_in("webex-import");
        let path = std::env::temp_dir().join(format!("quatrelle-webex-import-{n}-{}.ics", std::process::id()));
        std::fs::write(&path, ics(method)).unwrap();
        s.run("file.open", &json!({"path": path.to_string_lossy()})).unwrap();
        let _ = std::fs::remove_file(&path);
        let planted = s.pim_mut("sample").events.iter().find(|e| e.event.summary == "Planted").map(|e| e.event.clone()).expect("imported");
        assert_eq!(planted.webex_id, None, "{method:?}");
        done(s);
    }
    // An appointment attached to a message leaves its meeting out.
    let mut s = signed_in("webex-attach");
    let uid = alone(&mut s);
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.attachItem", &json!({"kind": "event", "id": uid})).unwrap();
    let data = s.compose.as_ref().unwrap().attachments[0].data.clone();
    let text = String::from_utf8_lossy(&data);
    assert!(!text.contains("WEBEX"), "{text}");
    done(s);
}

/// Webex says the sign-in is over, to a create, an update or a delete alike: the session is signed
/// out, who it was stays, and Options offers the sign-in.
#[test]
fn an_answer_that_says_sign_in_signs_out() {
    for which in 0..3 {
        let mut s = signed_in("webex-lapsed");
        s.prefs.webex_user = "Zoe".into();
        asked(&mut s, false, false);
        let signed_in_error = Some(WebexError::SignIn);
        match which {
            0 => s.apply(Event::WebexCreated { ask: ASK, meeting: None, error: signed_in_error }),
            1 => s.apply(Event::WebexUpdated { id: "m1".into(), meeting: None, error: signed_in_error }),
            _ => s.apply(Event::WebexDeleted { id: "m1".into(), error: signed_in_error }),
        }
        assert!(!s.prefs.webex_signed_in, "{which}");
        assert_eq!(s.prefs.webex_user, "Zoe", "{which}");
        assert_eq!(s.webex.sign_in_error, Some(WebexError::SignIn), "{which}");
        done(s);
    }
    // Any other refusal leaves the sign-in alone.
    let mut s = signed_in("webex-refused");
    s.apply(Event::WebexUpdated { id: "m1".into(), meeting: None, error: Some(WebexError::Forbidden) });
    assert!(s.prefs.webex_signed_in && s.webex.sign_in_error.is_none());
    done(s);
}

/// An update Webex didn't take is sent again by the next save, with what Webex last had as its
/// starting point.
#[test]
fn a_failed_update_is_sent_again_by_the_next_save() {
    let mut s = signed_in("webex-resend");
    let uid = alone(&mut s);
    let original = WebexFields::of(&s.find_event(&uid).unwrap().1.event);
    s.webex.sent.insert("m1".into(), original.clone());
    s.run("event.update", &json!({"uid": uid, "summary": "Budget 2"})).unwrap();
    let jobs = webex_jobs(&mut s);
    let [Job::WebexUpdate { after, .. }] = &jobs[..] else { panic!("{jobs:?}") };
    let after = after.clone();
    s.apply(Event::WebexUpdated { id: "m1".into(), meeting: None, error: Some(WebexError::Failed("no network".into())) });
    assert_eq!(s.webex.sent.get("m1"), Some(&original));
    s.run("event.update", &json!({"uid": uid, "location": "Room 2"})).unwrap();
    let jobs = webex_jobs(&mut s);
    let [Job::WebexUpdate { before, after: again, .. }] = &jobs[..] else { panic!("{jobs:?}") };
    assert_eq!((before, again), (&original, &after));
    // One Webex took stays sent.
    s.apply(Event::WebexUpdated { id: "m1".into(), meeting: Some(meeting(OLD_LINK)), error: None });
    s.run("event.update", &json!({"uid": uid, "location": "Room 3"})).unwrap();
    assert!(webex_jobs(&mut s).is_empty());
    done(s);
}

/// Two updates on their way and both refused: Webex still has the first start, and the next save
/// sends both changes.
#[test]
fn two_failed_updates_are_both_sent_again() {
    let mut s = signed_in("webex-resend-two");
    let uid = alone(&mut s);
    let original = WebexFields::of(&s.find_event(&uid).unwrap().1.event);
    s.webex.sent.insert("m1".into(), original.clone());
    s.run("event.update", &json!({"uid": uid, "summary": "Budget 2"})).unwrap();
    s.run("event.update", &json!({"uid": uid, "summary": "Budget 3"})).unwrap();
    assert_eq!(webex_jobs(&mut s).len(), 2);
    for _ in 0..2 {
        s.apply(Event::WebexUpdated { id: "m1".into(), meeting: None, error: Some(WebexError::Failed("no network".into())) });
    }
    assert_eq!(s.webex.sent.get("m1"), Some(&original));
    s.run("event.update", &json!({"uid": uid, "location": "Room 2"})).unwrap();
    let jobs = webex_jobs(&mut s);
    let [Job::WebexUpdate { before, after, .. }] = &jobs[..] else { panic!("{jobs:?}") };
    assert_eq!((before, after.title.as_str()), (&original, "Budget 3"));
    done(s);
}

/// The command's updates leave a start too, and a delete or a sign-out forgets them.
#[test]
fn updates_by_command_leave_a_start_until_deleted_or_signed_out() {
    let mut s = signed_in("webex-unacknowledged");
    let p = json!({"id": "m1", "summary": "Budget", "start": "2026-10-21T15:00"});
    s.run("webex.updateMeeting", &p).unwrap();
    assert_eq!(s.webex.unacknowledged.get("m1").map(|q| q.len()), Some(1));
    s.run("webex.deleteMeeting", &json!({"id": "m1"})).unwrap();
    assert!(s.webex.unacknowledged.is_empty());
    s.run("webex.updateMeeting", &p).unwrap();
    s.run("webex.signOut", &json!({})).unwrap();
    assert!(s.webex.unacknowledged.is_empty());
    done(s);
}
