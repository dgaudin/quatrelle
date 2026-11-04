// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: the conference an appointment is saved with, kept by its window.

use quatrelle_engine::WebexRoom;
use quatrelle_engine::model::ical::Conference;

use super::*;
use crate::pim_dialogs::EventForm;
use crate::pim_dialogs::webex_rooms::add_room;

const JITSI: &str = "https://jitsi.example.org/team";

/// The event `uid`'s conference, as stored.
fn stored(a: &MailApp, uid: &str) -> Option<Conference> {
    a.session.find_event(uid).and_then(|(_, e)| e.event.conference.clone())
}

/// A new event made with `notes` and the conference `JITSI`, opened in the form.
fn made_and_opened(a: &mut MailApp, notes: &str) -> (String, EventForm) {
    let p = json!({"summary": "Point", "start": "2026-10-23T11:00", "end": "2026-10-23T12:00", "description": notes,
        "conference": {"url": JITSI, "label": "Jitsi"}});
    let made = a.session.run("event.create", &p).expect("made");
    let uid = made["uid"].as_str().unwrap_or_default().to_string();
    let start = a.session.find_event(&uid).map(|(_, e)| e.event.start).expect("stored");
    let f = EventForm::open(&a.session, &uid, start).expect("opened");
    (uid, f)
}

/// Another app's conference, its link in neither the place nor the notes, stays whatever the
/// window changes, with its label and features; one whose link is in the notes is the window's,
/// and goes once the link is erased.
#[test]
fn a_foreign_conference_stays() {
    let (mut a, dir) = app("webex-rooms-foreign");
    let (uid, mut f) = made_and_opened(&mut a, "Ordre du jour");
    assert!(!f.conference_shown);
    let features = vec!["VIDEO".to_string(), "SCREEN".to_string()];
    let account = a.session.find_event(&uid).map(|(acc, _)| acc.to_string()).unwrap_or_default();
    if let Some(e) = a.session.pim_mut(&account).events.iter_mut().find(|e| e.event.uid == uid)
        && let Some(c) = &mut e.event.conference
    {
        c.features = features.clone();
    }
    f.reminder = Some(30);
    a.session.run("event.update", &f.params()).expect("saved");
    let reminder = a.session.find_event(&uid).and_then(|(_, e)| quatrelle_engine::model::ical::reminder_minutes(&e.event));
    assert_eq!(reminder, Some(30), "saved");
    assert_eq!(stored(&a, &uid), Some(Conference { url: JITSI.into(), label: "Jitsi".into(), features }));

    let (uid, mut f) = made_and_opened(&mut a, &format!("Ordre du jour\nRejoindre : {JITSI}\n"));
    assert!(f.conference_shown);
    a.session.run("event.update", &f.params()).expect("saved");
    assert!(stored(&a, &uid).is_some(), "kept while its link is there");
    f.description = "Ordre du jour".into();
    a.session.run("event.update", &f.params()).expect("saved");
    assert_eq!(stored(&a, &uid), None, "taken out with its link");
    let _ = std::fs::remove_dir_all(dir);
}

/// A new appointment given a room is made with the room as its conference.
#[test]
fn a_new_appointment_is_made_with_its_rooms_conference() {
    let (mut a, dir) = app("webex-rooms-new");
    let ana = WebexRoom { name: "Ana".into(), url: "https://acme.webex.com/meet/ana".into(), details: vec!["N° 123".into()] };
    let mut f = EventForm::new(&a.session, Some(NOW + 2 * 3600));
    f.summary = "Point".into();
    add_room(&mut f, &ana, std::slice::from_ref(&ana));
    let made = a.session.run("event.create", &f.params()).expect("made");
    let uid = made["uid"].as_str().unwrap_or_default();
    let features = vec!["AUDIO".to_string(), "VIDEO".to_string()];
    assert_eq!(stored(&a, uid), Some(Conference { url: ana.url.clone(), label: "Webex".into(), features }));
    let _ = std::fs::remove_dir_all(dir);
}
