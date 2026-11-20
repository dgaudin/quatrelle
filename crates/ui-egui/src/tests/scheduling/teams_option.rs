// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: File › Options › Calendar's option to make every meeting with guests a Teams meeting.
//! Its switch; the meeting asked for once the title and the first guest have been left alone a
//! second, or at once when Send is pressed (the invitation waiting for it); what the window says
//! when Teams can't make it; and what keeps it from coming back by itself.

use egui::accesskit::{Role, Toggled};
use quatrelle_engine::model::Job;
use quatrelle_engine::model::ical::CalEvent;
use quatrelle_engine::model::pim::teams::{self, TeamsMeetingError};

use super::teams_button::{answer, authorizing, calendar, choose, open_menu, served};
use super::teams_meeting::{ID, Jobs, LINK, SIZE, allowed, appointment, click, creates, deletes, meeting, open_new};
use super::*;
use crate::dialogs::people_field::{egui_id, meeting_field};
use crate::look::people_field::input_id;
use crate::pim_dialogs::{EventForm, TeamsPrompt};

/// The option, as its row and its switch say it, and the words under it.
const OPTION: &str = "Make every meeting with guests a Teams meeting";
const ABOUT: &str = "An appointment you organize gets a Teams meeting with its first guest";
/// What the window says when Teams can't make the meeting.
const NOT_ADDED: &str = "Teams meeting not added: sign in to Teams";
const ZOE: &str = "zoe@studio.example";
const SAM: &str = "sam@studio.example";

fn form(a: &mut MailApp) -> &mut EventForm {
    &mut a.ui.event_windows[0].1
}

/// An appointment called `title` in its window, its guests' lines shown, in a session signed in
/// to Teams with its meetings allowed and the option on; the jobs kept.
fn with_the_option(name: &str, title: &str) -> (MailApp, std::path::PathBuf, egui::Context, Jobs) {
    let (mut a, dir, ctx, jobs) = appointment(name, title);
    a.session.prefs.teams_meeting_for_guests = true;
    form(&mut a).inviting = true;
    frame_sized(&ctx, &mut a, vec![], SIZE);
    (a, dir, ctx, jobs)
}

/// The clock the frames are drawn at.
fn now(ctx: &egui::Context) -> f64 {
    ctx.input(|i| i.time)
}

/// A frame at `time` seconds, the clock the quiet second is measured by.
fn frame_at(ctx: &egui::Context, a: &mut MailApp, events: Vec<Event>, time: f64) {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SIZE)), time: Some(time), events, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
}

/// `who` typed at `time` in window 0's Required line, its input given the keyboard just before.
fn type_guest(ctx: &egui::Context, a: &mut MailApp, who: &str, time: f64) {
    let w = a.ui.event_windows[0].0;
    ctx.memory_mut(|m| m.request_focus(input_id(egui_id(meeting_field(w, "required")))));
    frame_at(ctx, a, vec![], time - 0.05);
    frame_at(ctx, a, vec![Event::Text(who.into())], time);
}

/// `title` typed at `time` in window 0's empty title, clicked first.
fn type_title(ctx: &egui::Context, a: &mut MailApp, title: &str, time: f64) {
    click(ctx, a, "Add a title");
    frame_at(ctx, a, vec![Event::Text(title.into())], time);
}

/// A press at `at` let go at `time`.
fn press_at(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2, time: f64) {
    let button = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame_at(ctx, a, vec![Event::PointerMoved(at)], time - 0.1);
    frame_at(ctx, a, vec![button(true)], time - 0.05);
    frame_at(ctx, a, vec![button(false)], time);
}

/// The appointment called `title` in the calendar.
fn saved(a: &MailApp, title: &str) -> Option<CalEvent> {
    let item = a.session.agenda(NOW, NOW + 86_400).into_iter().find(|i| i.summary == title)?;
    a.session.find_event(&item.uid).map(|(_, e)| e.event.clone())
}

/// The option's switch on the Options page, drawn `size`, as a screen reader is told of it: where
/// it is, and whether it's on.
fn switch(ctx: &egui::Context, a: &mut MailApp, size: egui::Vec2) -> Option<(egui::Pos2, bool)> {
    ctx.enable_accesskit();
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
    let (_, n) = nodes.iter().find(|(_, n)| n.role() == Role::CheckBox && n.label() == Some(OPTION))?;
    let r = n.bounds()?;
    Some((egui::pos2(((r.x0 + r.x1) / 2.0) as f32, ((r.y0 + r.y1) / 2.0) as f32), n.toggled() == Some(Toggled::True)))
}

/// File › Options › Calendar: the option's row, off at first, with its words under it, after the
/// invitations' and before the Webex card; a click switches it on.
#[test]
fn the_option_draws_off_and_switches() {
    let (mut a, dir) = app("teams-option-switch");
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 5000.0);
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
    let tidy = settled_text_at(&ctx, &mut a, "Move meeting invitations to Deleted Items once answered", size).expect("the Calendar card");
    let option = settled_text_at(&ctx, &mut a, OPTION, size).expect("the option's row");
    let about = settled_text_at(&ctx, &mut a, ABOUT, size).expect("its words");
    let webex = settled_text_at(&ctx, &mut a, "Use Teams in Quatrelle", size).expect("the cards after");
    assert!(tidy.y < option.y && option.y < about.y && about.y < webex.y, "{tidy:?} {option:?} {about:?} {webex:?}");
    let (at, on) = switch(&ctx, &mut a, size).expect("its switch");
    assert!(!on && !a.session.prefs.teams_meeting_for_guests, "off at first");
    click_at(&ctx, &mut a, at, 1, size);
    assert!(a.session.prefs.teams_meeting_for_guests, "switched on");
    assert_eq!(switch(&ctx, &mut a, size).map(|s| s.1), Some(true));
    for key in [OPTION, ABOUT, NOT_ADDED] {
        for lang in crate::i18n::Lang::all().skip(1) {
            assert_ne!(crate::i18n::tr(lang, key), key, "{}: {key}", lang.code());
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// The title and a guest typed: nothing half a second after the guest, then, the lines left alone
/// a second, the meeting is asked for as Add a Teams meeting asks — once.
#[test]
fn a_first_guest_brings_a_meeting_after_a_quiet_second() {
    let (mut a, dir, ctx, jobs) = with_the_option("teams-option-quiet", "");
    let t = now(&ctx) + 1.0;
    type_title(&ctx, &mut a, "Budget", t);
    assert!(creates(&jobs).is_empty(), "no guest yet");
    type_guest(&ctx, &mut a, ZOE, t + 0.3);
    assert_eq!(form(&mut a).guests, ZOE, "typed into the form");
    frame_at(&ctx, &mut a, vec![], t + 0.8);
    assert!(creates(&jobs).is_empty() && form(&mut a).teams_wait.is_none(), "half a second after the guest: nothing yet");
    // What the window said of something else stays: the meeting comes without a click.
    form(&mut a).error = Some("Not found: Dupont".into());
    frame_at(&ctx, &mut a, vec![], t + 1.4);
    assert_eq!(form(&mut a).error.as_deref(), Some("Not found: Dupont"));
    let asked = jobs.borrow().iter().find_map(|j| match j {
        Job::Teams { job: teams::Job::MeetingCreate { ask, fields } } => Some((*ask, fields.title.clone())),
        _ => None,
    });
    let (ask, title) = asked.expect("asked a second after the guest");
    assert_eq!((title.as_str(), form(&mut a).teams_wait), ("Budget", Some(ask)));
    for at in [t + 2.0, t + 4.0] {
        frame_at(&ctx, &mut a, vec![], at);
    }
    assert_eq!(creates(&jobs), [ask], "once");
    answer(&ctx, &mut a, Some(meeting()), None);
    let f = form(&mut a);
    assert_eq!((f.teams_meeting_id.as_deref(), f.location.as_str(), f.teams_fresh), (Some(ID), LINK, true));
    assert!(f.teams_auto_tried && f.teams_send_held.is_none());
    let _ = std::fs::remove_dir_all(dir);
}

/// A guest without a title yet: the window waits, saying nothing; the title typed, the meeting
/// comes a quiet second later.
#[test]
fn no_title_yet_waits_without_a_word() {
    let (mut a, dir, ctx, jobs) = with_the_option("teams-option-no-title", "");
    let t = now(&ctx) + 1.0;
    type_guest(&ctx, &mut a, ZOE, t);
    for at in [t + 1.2, t + 3.0] {
        frame_at(&ctx, &mut a, vec![], at);
    }
    assert!(creates(&jobs).is_empty());
    let f = form(&mut a);
    assert!(f.error.is_none() && f.teams_prompt.is_none() && !f.teams_auto_tried, "not a word: {:?}", f.error);
    let t = now(&ctx) + 1.0;
    type_title(&ctx, &mut a, "Budget", t);
    frame_at(&ctx, &mut a, vec![], t + 0.5);
    assert!(creates(&jobs).is_empty(), "the title was just typed");
    frame_at(&ctx, &mut a, vec![], t + 1.1);
    assert_eq!(creates(&jobs).len(), 1);
    let _ = std::fs::remove_dir_all(dir);
}

/// Signed out, nothing goes to Teams: the window says so once, with Sign in, which opens File ›
/// Options at Teams; without the permission, with Authorize Teams meetings, gone once it's given.
#[test]
fn signed_out_says_it_once_with_its_button() {
    let (mut a, dir, ctx, jobs) = with_the_option("teams-option-signed-out", "Budget");
    a.session.prefs.teams.signed_in = false;
    let t = now(&ctx) + 1.0;
    type_guest(&ctx, &mut a, ZOE, t);
    frame_at(&ctx, &mut a, vec![], t + 0.5);
    assert_eq!(form(&mut a).teams_prompt, None, "not before the quiet second");
    frame_at(&ctx, &mut a, vec![], t + 1.1);
    assert_eq!(form(&mut a).teams_prompt, Some(TeamsPrompt::NotAdded));
    settled_text_at(&ctx, &mut a, NOT_ADDED, SIZE).expect("said in the window");
    click(&ctx, &mut a, "Sign in");
    assert!(a.ui.backstage.as_ref().is_some_and(|b| b.page == crate::backstage::Page::Options), "Options opens");
    a.ui.backstage = None;
    // Said once: the words gone, another guest brings them back no more.
    form(&mut a).teams_prompt = None;
    let t = now(&ctx) + 1.0;
    type_guest(&ctx, &mut a, &format!("; {SAM}"), t);
    for at in [t + 1.1, t + 3.0] {
        frame_at(&ctx, &mut a, vec![], at);
    }
    assert_eq!(form(&mut a).teams_prompt, None, "not said again");
    assert!(settled_text_at(&ctx, &mut a, NOT_ADDED, SIZE).is_none());

    // Signed in without the meetings' permission: its button asks for it.
    a.ui.event_windows.clear();
    a.session.prefs.teams.signed_in = true;
    a.session.prefs.teams.meetings = false;
    open_new(&ctx, &mut a);
    (form(&mut a).summary, form(&mut a).inviting) = ("Budget".into(), true);
    let t = now(&ctx) + 1.0;
    type_guest(&ctx, &mut a, ZOE, t);
    frame_at(&ctx, &mut a, vec![], t + 1.1);
    settled_text_at(&ctx, &mut a, NOT_ADDED, SIZE).expect("said");
    assert!(settled_text_at(&ctx, &mut a, "Sign in", SIZE).is_none(), "the permission is what's missing");
    click(&ctx, &mut a, "Authorize Teams meetings");
    assert!(authorizing(&jobs), "the sign-in asks for meetings too");
    allowed(&mut a, true);
    assert!(settled_text_at(&ctx, &mut a, NOT_ADDED, SIZE).is_none(), "gone once Teams can make meetings");
    assert!(creates(&jobs).is_empty(), "nothing went to Teams");
    let _ = std::fs::remove_dir_all(dir);
}

/// The user removed the meeting in this window: another guest brings none back, whether the option
/// or the button had added it.
#[test]
fn remove_then_another_guest_adds_nothing() {
    let (mut a, dir, ctx, jobs) = with_the_option("teams-option-removed", "Budget");
    let t = now(&ctx) + 1.0;
    type_guest(&ctx, &mut a, ZOE, t);
    frame_at(&ctx, &mut a, vec![], t + 1.1);
    assert_eq!(creates(&jobs).len(), 1);
    answer(&ctx, &mut a, Some(meeting()), None);
    open_menu(&ctx, &mut a);
    click(&ctx, &mut a, "Remove the Teams meeting");
    assert_eq!(deletes(&jobs), [ID]);
    assert!(form(&mut a).teams_removed);
    let t = now(&ctx) + 1.0;
    type_guest(&ctx, &mut a, &format!("; {SAM}"), t);
    for at in [t + 1.1, t + 3.0] {
        frame_at(&ctx, &mut a, vec![], at);
    }
    assert_eq!(creates(&jobs).len(), 1, "none comes back");

    // Added with the button, then removed: the option hasn't tried yet, and still adds none.
    a.ui.event_windows.clear();
    open_new(&ctx, &mut a);
    (form(&mut a).summary, form(&mut a).inviting) = ("Budget".into(), true);
    click(&ctx, &mut a, "Teams Meeting");
    answer(&ctx, &mut a, Some(meeting()), None);
    open_menu(&ctx, &mut a);
    click(&ctx, &mut a, "Remove the Teams meeting");
    assert_eq!(creates(&jobs).len(), 2);
    let t = now(&ctx) + 1.0;
    type_guest(&ctx, &mut a, ZOE, t);
    for at in [t + 1.1, t + 3.0] {
        frame_at(&ctx, &mut a, vec![], at);
    }
    assert!(!form(&mut a).teams_auto_tried);
    assert_eq!(creates(&jobs).len(), 2, "removed by the user: none by itself");
    let _ = std::fs::remove_dir_all(dir);
}

/// A meeting the calendar took out (a Microsoft 365 mailbox's chosen, then the first calendar
/// again) isn't one the user removed: a guest then brings the option's meeting, once.
#[test]
fn a_meeting_the_calendar_took_out_may_come_back_once() {
    let (mut a, dir, ctx, jobs) = with_the_option("teams-option-calendar", "Budget");
    a.ui.event_windows.clear();
    served(&mut a, "sample", false);
    calendar(&mut a, "sample", "AAMkEWS", false);
    let added = a.session.run("account.add", &json!({"email": "zoe@contoso.example", "password": "pw"})).expect("added");
    let zoe = added["account"].as_str().expect("its id").to_string();
    served(&mut a, &zoe, true);
    calendar(&mut a, &zoe, "AAMkGRAPH", false);
    open_new(&ctx, &mut a);
    (form(&mut a).summary, form(&mut a).inviting) = ("Budget".into(), true);
    choose(&mut a, "sample", "AAMkEWS");
    click(&ctx, &mut a, "Teams Meeting");
    answer(&ctx, &mut a, Some(meeting()), None);
    choose(&mut a, &zoe, "AAMkGRAPH");
    frame_sized(&ctx, &mut a, vec![], SIZE);
    assert_eq!(deletes(&jobs), [ID], "taken out for that calendar");
    assert!(form(&mut a).teams_meeting_id.is_none() && !form(&mut a).teams_removed, "not by the user");
    choose(&mut a, "sample", "AAMkEWS");
    let t = now(&ctx) + 1.0;
    type_guest(&ctx, &mut a, ZOE, t);
    frame_at(&ctx, &mut a, vec![], t + 1.1);
    assert_eq!(creates(&jobs).len(), 2, "the option adds one");
    answer(&ctx, &mut a, Some(meeting()), None);
    choose(&mut a, &zoe, "AAMkGRAPH");
    frame_sized(&ctx, &mut a, vec![], SIZE);
    choose(&mut a, "sample", "AAMkEWS");
    let t = now(&ctx) + 1.0;
    type_guest(&ctx, &mut a, &format!("; {SAM}"), t);
    for at in [t + 1.1, t + 3.0] {
        frame_at(&ctx, &mut a, vec![], at);
    }
    assert_eq!(creates(&jobs).len(), 2, "once per window");
    let _ = std::fs::remove_dir_all(dir);
}

/// A meeting saved with its guests already had its first one: opened again, it gets no meeting by
/// itself.
#[test]
fn a_meeting_opened_with_its_guests_gets_none_by_itself() {
    let (mut a, dir, ctx, jobs) = with_the_option("teams-option-opened", "Budget");
    a.ui.event_windows.clear();
    let made = a.session.run("event.create", &json!({"summary": "Review", "start": "2026-10-21T15:00", "attendees": ZOE})).expect("made");
    let uid = made["uid"].as_str().expect("its uid").to_string();
    let start = a.session.find_event(&uid).map(|(_, e)| e.event.start).expect("stored");
    a.ui.dialog = Dialog::open_event(&a.session, &uid, start);
    frame_sized(&ctx, &mut a, vec![], SIZE);
    assert!(form(&mut a).teams_auto_tried, "its first guest came before");
    let t = now(&ctx) + 1.0;
    for at in [t, t + 1.1, t + 3.0] {
        frame_at(&ctx, &mut a, vec![], at);
    }
    assert!(creates(&jobs).is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

/// Review Focus 5: a guest typed and Send pressed in the same second: no invitation goes before
/// Teams answers, then it goes with the meeting's block and id. Teams refusing it, nothing goes,
/// the window says why, and Send again sends the meeting without Teams.
#[test]
fn send_pressed_at_once_waits_for_the_meeting() {
    let (mut a, dir, ctx, jobs) = with_the_option("teams-option-send-at-once", "Budget");
    let send = settled_text_at(&ctx, &mut a, "Send", SIZE).expect("the meeting's Send");
    let t = now(&ctx) + 1.0;
    type_guest(&ctx, &mut a, ZOE, t);
    press_at(&ctx, &mut a, send, t + 0.3);
    let ask = form(&mut a).teams_wait.expect("the meeting asked for at once");
    assert_eq!(creates(&jobs), [ask]);
    assert_eq!(form(&mut a).teams_send_held, Some("event.create"));
    for at in [t + 1.5, t + 3.0] {
        frame_at(&ctx, &mut a, vec![], at);
    }
    assert!(saved(&a, "Budget").is_none() && a.ui.event_windows.len() == 1, "no invitation before the answer");
    assert_eq!(creates(&jobs), [ask], "one request");
    a.session.apply(MailEvent::Teams { event: teams::Event::MeetingCreated { ask, meeting: Some(meeting()), error: None } });
    frame_at(&ctx, &mut a, vec![], t + 3.5);
    assert!(a.ui.event_windows.is_empty(), "sent, and the window closed");
    let e = saved(&a, "Budget").expect("sent");
    assert_eq!(e.teams_meeting_id.as_deref(), Some(ID));
    assert!(e.description.contains(&format!("Join the Teams meeting: {LINK}")), "{:?}", e.description);
    assert_eq!(e.attendees.iter().map(|x| x.email.as_str()).collect::<Vec<_>>(), [ZOE]);
    assert!(deletes(&jobs).is_empty(), "the meeting is the appointment's");

    // Teams refuses: nothing goes, the window says why; Send again sends without Teams.
    open_new(&ctx, &mut a);
    (form(&mut a).summary, form(&mut a).inviting) = ("Review".into(), true);
    let send = settled_text_at(&ctx, &mut a, "Send", SIZE).expect("the meeting's Send");
    let t = now(&ctx) + 1.0;
    type_guest(&ctx, &mut a, ZOE, t);
    press_at(&ctx, &mut a, send, t + 0.3);
    let ask = form(&mut a).teams_wait.expect("asked for at once");
    a.session.apply(MailEvent::Teams { event: teams::Event::MeetingCreated { ask, meeting: None, error: Some(TeamsMeetingError::Offline) } });
    frame_at(&ctx, &mut a, vec![], t + 0.5);
    let f = form(&mut a);
    assert_eq!(f.error.as_deref(), Some("The Teams meeting couldn't be created: no connection"));
    assert!(f.teams_send_held.is_none() && f.teams_meeting_id.is_none());
    assert!(saved(&a, "Review").is_none(), "nothing went");
    settled_text_at(&ctx, &mut a, "The Teams meeting couldn't be created: no connection", SIZE).expect("said in the window");
    let t = now(&ctx) + 1.0;
    press_at(&ctx, &mut a, send, t);
    frame_at(&ctx, &mut a, vec![], t + 0.1);
    let e = saved(&a, "Review").expect("sent without Teams");
    assert_eq!(e.teams_meeting_id, None);
    assert_eq!(creates(&jobs).len(), 2, "not asked again");
    let _ = std::fs::remove_dir_all(dir);
}
