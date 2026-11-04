// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: the Webex rooms in the Webex Meeting button's menu, and the conference an appointment
//! is saved with, kept by its window.

use quatrelle_engine::WebexRoom;
use quatrelle_engine::model::WebexMeeting;
use quatrelle_engine::model::ical::Conference;
use quatrelle_engine::model::job::Secret;

use super::webex::{button, open_webex_menu};
use super::*;
use crate::pim_dialogs::EventForm;
use crate::pim_dialogs::webex::take_meeting;
use crate::pim_dialogs::webex_rooms::add_room;

const JITSI: &str = "https://jitsi.example.org/team";
const ANA: &str = "https://acme.webex.com/meet/ana";
const TEAM: &str = "https://acme.webex.com/meet/team";
const LINK: &str = "https://acme.webex.com/acme/j.php?MTID=m1";
const SIZE: egui::Vec2 = egui::vec2(1280.0, 900.0);

/// The room carried over from 1.0, unnamed.
fn ana() -> WebexRoom {
    WebexRoom { name: String::new(), url: ANA.into(), details: Vec::new() }
}

fn team() -> WebexRoom {
    WebexRoom { name: "Équipe".into(), url: TEAM.into(), details: vec!["N° 456".into()] }
}

/// A titled appointment in its window, with `rooms` in the settings.
fn with_rooms(name: &str, rooms: Vec<WebexRoom>) -> (MailApp, std::path::PathBuf, egui::Context) {
    let (mut a, dir) = app(name);
    let ctx = egui::Context::default();
    a.session.prefs.webex_rooms = rooms;
    frame_sized(&ctx, &mut a, vec![], SIZE);
    a.ui.dialog = Some(Dialog::new_event(&a.session, Some(NOW + 2 * 3600)));
    frame_sized(&ctx, &mut a, vec![], SIZE);
    a.ui.event_windows[0].1.summary = "Point".into();
    (a, dir, ctx)
}

/// The menu lists the scheduled meeting, then each room by its name (the unnamed one as the
/// personal room), then Add a room…; a room clicked is the appointment's place, Join and
/// conference, with its lines in the notes, and the menu closes.
#[test]
fn the_menu_lists_the_rooms_by_name() {
    let (mut a, dir, ctx) = with_rooms("webex-rooms-menu", vec![ana(), team()]);
    open_webex_menu(&ctx, &mut a, SIZE);
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    for label in ["New Webex meeting", "My personal room", "Équipe", "Add a room…"] {
        assert!(text_at(&shapes, label).is_some(), "{label}");
    }
    let at = settled_text_at(&ctx, &mut a, "Équipe", SIZE).expect("its line");
    click_at(&ctx, &mut a, at, 1, SIZE);
    let f = &a.ui.event_windows[0].1;
    assert_eq!((f.location.as_str(), f.join.as_deref()), (TEAM, Some(TEAM)));
    assert_eq!(f.conference.as_ref().map(|c| c.url.as_str()), Some(TEAM));
    assert!(f.description.ends_with(&format!("{TEAM}\nN° 456\n")), "{:?}", f.description);
    assert!(text_at(&frame_shapes(&ctx, &mut a, vec![], SIZE), "Add a room…").is_none(), "the menu closed");
    let _ = std::fs::remove_dir_all(dir);
}

/// Once a room is in, Webex Meeting stays beside Join: another room chosen there takes the first
/// one's place in the notes, the place, Join and the conference.
#[test]
fn another_room_takes_the_first_ones_place() {
    let (mut a, dir, ctx) = with_rooms("webex-rooms-another", vec![ana(), team()]);
    open_webex_menu(&ctx, &mut a, SIZE);
    let at = settled_text_at(&ctx, &mut a, "My personal room", SIZE).expect("its line");
    click_at(&ctx, &mut a, at, 1, SIZE);
    assert_eq!(a.ui.event_windows[0].1.join.as_deref(), Some(ANA));
    assert!(settled_text_at(&ctx, &mut a, "Join meeting", SIZE).is_some(), "Join shows");
    open_webex_menu(&ctx, &mut a, SIZE);
    let at = settled_text_at(&ctx, &mut a, "Équipe", SIZE).expect("its line, the menu open again");
    click_at(&ctx, &mut a, at, 1, SIZE);
    let f = &a.ui.event_windows[0].1;
    assert_eq!(f.description, format!("Join the Webex meeting:\n{TEAM}\nN° 456\n"), "the first room's block gone");
    assert_eq!((f.location.as_str(), f.join.as_deref()), (TEAM, Some(TEAM)));
    assert_eq!(f.conference.as_ref().map(|c| c.url.as_str()), Some(TEAM));
    let _ = std::fs::remove_dir_all(dir);
}

/// While the appointment has its scheduled Webex meeting (Join beside the button), its menu still
/// opens, the rooms and Add a room… greyed: a click changes nothing. With as many rooms as are
/// kept, Add a room… alone is.
#[test]
fn rooms_are_greyed_with_a_scheduled_meeting() {
    let (mut a, dir, ctx) = with_rooms("webex-rooms-greyed", vec![ana(), team()]);
    let m = WebexMeeting {
        id: "m1".into(),
        web_link: LINK.into(),
        meeting_number: "2623456701".into(),
        password: Secret(String::new()),
        sip_address: String::new(),
    };
    let rooms = a.session.prefs.webex_rooms.clone();
    let f = &mut a.ui.event_windows[0].1;
    f.description = "Ordre du jour".into();
    take_meeting(f, &m, &rooms);
    let before = |f: &EventForm| (f.description.clone(), f.location.clone(), f.join.clone(), f.conference.clone());
    let scheduled = before(f);
    assert!(settled_text_at(&ctx, &mut a, "Join meeting", SIZE).is_some(), "Join shows");
    open_webex_menu(&ctx, &mut a, SIZE);
    for label in ["My personal room", "Équipe", "Add a room…"] {
        assert_eq!(button(&ctx, &mut a, label).map(|b| b.1), Some(false), "{label}");
    }
    for label in ["Équipe", "Add a room…"] {
        let at = settled_text_at(&ctx, &mut a, label, SIZE).expect(label);
        click_at(&ctx, &mut a, at, 1, SIZE);
        assert_eq!(before(&a.ui.event_windows[0].1), scheduled, "{label}");
        assert!(text_at(&frame_shapes(&ctx, &mut a, vec![], SIZE), "New Webex room").is_none(), "no room asked for");
    }
    // As many rooms as are kept: they can be chosen, but none added.
    a.ui.event_windows[0].1.webex_id = None;
    a.session.prefs.webex_rooms = (0..quatrelle_engine::WEBEX_ROOMS_MAX)
        .map(|i| WebexRoom { name: format!("Salle {i}"), url: format!("https://acme.webex.com/meet/s{i}"), details: Vec::new() })
        .collect();
    frame_sized(&ctx, &mut a, vec![], SIZE);
    assert_eq!(button(&ctx, &mut a, "Salle 0").map(|b| b.1), Some(true));
    assert_eq!(button(&ctx, &mut a, "Add a room…").map(|b| b.1), Some(false));
    let _ = std::fs::remove_dir_all(dir);
}

/// A window that can't be changed shows Join alone: no Webex Meeting to put a room in.
#[test]
fn a_read_only_window_shows_join_alone() {
    let (mut a, dir, ctx) = with_rooms("webex-rooms-read-only", vec![ana()]);
    let f = &mut a.ui.event_windows[0].1;
    f.read_only = true;
    f.join = Some(LINK.into());
    settled_text_at(&ctx, &mut a, "Join meeting", SIZE).expect("Join drawn");
    // Told of the ribbon's buttons as a screen reader is: Join is there, and nothing else.
    assert_eq!(button(&ctx, &mut a, "Join meeting").map(|b| b.1), Some(true));
    assert_eq!(button(&ctx, &mut a, "Webex Meeting"), None);
    assert_eq!(button(&ctx, &mut a, "Webex Meeting options"), None);
    let _ = std::fs::remove_dir_all(dir);
}

/// Add a room… asks for its name and its link under the button (in the appointment's window, not
/// in Options behind it): nothing goes in until both are right, a link that isn't a personal
/// room's or that another room has saying so; then the room is kept and in the appointment.
#[test]
fn add_a_room_asks_its_name_and_link() {
    let (mut a, dir, ctx) = with_rooms("webex-rooms-add", Vec::new());
    open_webex_menu(&ctx, &mut a, SIZE);
    let at = settled_text_at(&ctx, &mut a, "Add a room…", SIZE).expect("the menu's entry");
    click_at(&ctx, &mut a, at, 1, SIZE);
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    assert!(text_at(&shapes, "New Webex room").is_some(), "asked for under the button");
    assert!(a.ui.backstage.is_none(), "not in Options");
    let typed = egui::Id::unique(("webex-room-typed", a.ui.event_windows[0].0));
    let enter = |ctx: &egui::Context, a: &mut MailApp| {
        frame_sized(ctx, a, vec![key(Key::Enter, Modifiers::NONE)], SIZE);
        frame_sized(ctx, a, vec![], SIZE);
    };
    // A link with no name.
    ctx.data_mut(|d| d.insert_temp(typed, (String::new(), ANA.to_string())));
    enter(&ctx, &mut a);
    assert!(a.session.prefs.webex_rooms.is_empty() && a.ui.event_windows[0].1.join.is_none(), "a room needs a name");
    // The name typed (the name's box has the focus), with a link that isn't a personal room's.
    ctx.data_mut(|d| d.insert_temp(typed, (String::new(), "https://evil.example/meet/ana".to_string())));
    frame_sized(&ctx, &mut a, vec![Event::Text("Ana".into())], SIZE);
    assert_eq!(ctx.data_mut(|d| d.get_temp::<(String, String)>(typed)).map(|t| t.0).as_deref(), Some("Ana"));
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    assert!(text_at(&shapes, "Not a Webex personal room: https://<site>.webex.com/meet/<name>").is_some());
    enter(&ctx, &mut a);
    assert!(a.session.prefs.webex_rooms.is_empty() && a.ui.event_windows[0].1.join.is_none(), "refused");
    // A link another room has, whatever its case.
    a.session.prefs.webex_rooms = vec![team()];
    ctx.data_mut(|d| d.insert_temp(typed, ("Ana".to_string(), "HTTPS://acme.webex.com/meet/TEAM".to_string())));
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    assert!(text_at(&shapes, "Another room has this link").is_some());
    enter(&ctx, &mut a);
    assert!(a.session.prefs.webex_rooms == [team()] && a.ui.event_windows[0].1.join.is_none(), "refused");
    // Right this time.
    a.session.prefs.webex_rooms.clear();
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    assert!(text_at(&shapes, "New Webex room").is_some(), "still asked");
    ctx.data_mut(|d| d.insert_temp(typed, (" Ana ".to_string(), ANA.to_string())));
    enter(&ctx, &mut a);
    assert_eq!(a.session.prefs.webex_rooms, [WebexRoom { name: "Ana".into(), url: ANA.into(), details: Vec::new() }], "kept");
    let f = &a.ui.event_windows[0].1;
    assert_eq!((f.join.as_deref(), f.location.as_str()), (Some(ANA), ANA));
    assert_eq!(f.description, format!("Join the Webex meeting:\n{ANA}\n"));
    assert_eq!(f.conference.as_ref().map(|c| c.url.as_str()), Some(ANA));
    let _ = std::fs::remove_dir_all(dir);
}

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
