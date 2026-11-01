//! Webex's answers: what the session keeps for the window that asked, what it does with a
//! meeting nobody waits for any more, and what a failure says.

use quatrelle_model::job::Secret;
use quatrelle_model::{WebexError, WebexFields, WebexMeeting};

use super::*;
use crate::WebexAsk;

/// The creation the tests answer.
const ASK: u64 = 7;
const OLD_LINK: &str = "https://acme.webex.com/acme/j.php?MTID=old";
const NEW_LINK: &str = "https://acme.webex.com/acme/j.php?MTID=new";
const ZOOM: &str = "https://us02web.zoom.us/j/1234567890";
const MEET: &str = "https://meet.google.com/abc-defg-hij";
const TEAMS: &str = "https://teams.microsoft.com/l/meetup-join/19%3ameeting_abc/0";

fn fields() -> WebexFields {
    WebexFields { title: "Budget".into(), start: NOW, end: NOW + 3600, timezone: "Europe/Paris".into(), recurrence: None }
}

/// The meeting `m1`, as Webex answers it.
fn meeting(link: &str) -> WebexMeeting {
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
fn appointment(s: &mut Session, location: &str, notes: &str) -> String {
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
fn invitations(s: &Session) -> usize {
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
