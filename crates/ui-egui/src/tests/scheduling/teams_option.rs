// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: File › Options › Calendar's option to make every meeting with guests a Teams meeting.
//! Its switch; the meeting asked for once the title and the first guest have been left alone a
//! second, or at once when Send is pressed (the invitation waiting for it); what the window says
//! when Teams can't make it; and what keeps it from coming back by itself. The frame where the
//! quiet second ends is one where Send, or a Webex meeting, may be pressed too: whatever comes
//! first, one meeting is asked for, and the invitation waits for it.

use egui::accesskit::{Role, Toggled};
use quatrelle_engine::model::Job;
use quatrelle_engine::model::ical::CalEvent;
use quatrelle_engine::model::pim::teams::{self, TeamsMeetingError};

use super::teams_button::{answer, authorizing, calendar, choose, open_menu, served};
use super::teams_meeting::{ID, Jobs, LINK, SIZE, allowed, appointment, click, creates, deletes, meeting, open_new};
use super::webex::{button, open_webex_menu};
use super::*;
use crate::dialogs::people_field::{MEETING_WINDOWS, egui_id, meeting_field};
use crate::look::people_field::input_id;
use crate::pim_dialogs::{EventForm, TeamsPrompt};
use crate::tests::writing::{directory_answers, with_exchange};

/// The option, as its row and its switch say it, and the words under it.
const OPTION: &str = "Make every meeting with guests a Teams meeting";
const ABOUT: &str = "An appointment you organize gets a Teams meeting with its first guest";
/// What the window says when Teams can't make the meeting.
const NOT_ADDED: &str = "Teams meeting not added: sign in to Teams";
pub(super) const ZOE: &str = "zoe@studio.example";
const SAM: &str = "sam@studio.example";

fn form(a: &mut MailApp) -> &mut EventForm {
    &mut a.ui.event_windows[0].1
}

/// An appointment called `title` in its window, its guests' lines shown, in a session signed in
/// to Teams with its meetings allowed and the option on; the jobs kept.
pub(super) fn with_the_option(name: &str, title: &str) -> (MailApp, std::path::PathBuf, egui::Context, Jobs) {
    let (mut a, dir, ctx, jobs) = appointment(name, title);
    a.session.prefs.teams_meeting_for_guests = true;
    form(&mut a).inviting = true;
    frame_sized(&ctx, &mut a, vec![], SIZE);
    (a, dir, ctx, jobs)
}

/// The clock the frames are drawn at.
pub(super) fn now(ctx: &egui::Context) -> f64 {
    ctx.input(|i| i.time)
}

/// A whole second at least a second from the clock now: a quiet second counted from it ends at
/// exactly one more.
fn later(ctx: &egui::Context) -> f64 {
    (now(ctx) + 1.0).ceil()
}

/// A frame at `time` seconds, the clock the quiet second is measured by.
pub(super) fn frame_at(ctx: &egui::Context, a: &mut MailApp, events: Vec<Event>, time: f64) {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SIZE)), time: Some(time), events, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
}

/// `who` typed at `time` in window 0's Required line, its input given the keyboard just before.
pub(super) fn type_guest(ctx: &egui::Context, a: &mut MailApp, who: &str, time: f64) {
    let w = a.ui.event_windows[0].0;
    ctx.memory_mut(|m| m.request_focus(input_id(egui_id(meeting_field(w, "required")))));
    frame_at(ctx, a, vec![], time - 0.05);
    frame_at(ctx, a, vec![Event::Text(who.into())], time);
}

/// Window 0's Required line left at `time` (its entry closed, as a click elsewhere closes it), and
/// a few frames after: the first frame where the quiet second is up since its lines last changed,
/// the line no longer changing as a press elsewhere starts.
fn guests_left(ctx: &egui::Context, a: &mut MailApp, time: f64) -> f64 {
    let w = a.ui.event_windows[0].0;
    ctx.memory_mut(|m| m.surrender_focus(input_id(egui_id(meeting_field(w, "required")))));
    for at in [time, time + 0.05, time + 0.1] {
        frame_at(ctx, a, vec![], at);
    }
    let since = form(a).teams_auto_since.map_or(time, |(_, at)| at);
    since + crate::pim_dialogs::teams_meeting::TEAMS_AUTO_QUIET + 1e-6
}

/// A press at `at` begun just before `end` and let go at `end`, nothing asked of Teams before.
fn press_ending_at(ctx: &egui::Context, a: &mut MailApp, jobs: &Jobs, at: egui::Pos2, end: f64) {
    let button = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame_at(ctx, a, vec![Event::PointerMoved(at)], end - 0.1);
    frame_at(ctx, a, vec![button(true)], end - 0.05);
    assert!(creates(jobs).is_empty() && form(a).teams_wait.is_none(), "the quiet second isn't up yet");
    frame_at(ctx, a, vec![button(false)], end);
}

/// `title` typed at `time` in window 0's empty title, clicked first.
fn type_title(ctx: &egui::Context, a: &mut MailApp, title: &str, time: f64) {
    click(ctx, a, "Add a title");
    frame_at(ctx, a, vec![Event::Text(title.into())], time);
}

/// A press at `at` let go at `time`.
pub(super) fn press_at(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2, time: f64) {
    let button = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame_at(ctx, a, vec![Event::PointerMoved(at)], time - 0.1);
    frame_at(ctx, a, vec![button(true)], time - 0.05);
    frame_at(ctx, a, vec![button(false)], time);
}

/// Teams' answer to the creation window 0 waits for, at `time`.
fn answered_at(ctx: &egui::Context, a: &mut MailApp, time: f64) {
    let ask = form(a).teams_wait.expect("a creation on its way");
    a.session.apply(MailEvent::Teams { event: teams::Event::MeetingCreated { ask, meeting: Some(meeting()), error: None } });
    frame_at(ctx, a, vec![], time);
}

/// What was asked of Webex: its creations.
fn webex_creates(jobs: &Jobs) -> usize {
    jobs.borrow().iter().filter(|j| matches!(j, Job::WebexCreate { .. })).count()
}

/// The appointment called `title` in the calendar.
pub(super) fn saved(a: &MailApp, title: &str) -> Option<CalEvent> {
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

/// I1: Send let go in the very frame the quiet second ends: the one meeting asked for comes first,
/// and the invitation waits for it, then goes with its block and id.
#[test]
fn send_let_go_as_the_quiet_second_ends_waits_for_the_meeting() {
    let (mut a, dir, ctx, jobs) = with_the_option("teams-option-send-at-the-second", "Budget");
    let send = settled_text_at(&ctx, &mut a, "Send", SIZE).expect("the meeting's Send");
    let t = later(&ctx);
    type_guest(&ctx, &mut a, ZOE, t);
    let end = guests_left(&ctx, &mut a, t + 0.05);
    press_ending_at(&ctx, &mut a, &jobs, send, end);
    assert_eq!(a.ui.event_windows.len(), 1, "no invitation before the answer");
    assert!(saved(&a, "Budget").is_none());
    assert_eq!((creates(&jobs).len(), form(&mut a).teams_send_held), (1, Some("event.create")));
    frame_at(&ctx, &mut a, vec![], end + 1.0);
    assert_eq!(creates(&jobs).len(), 1, "one request");
    answered_at(&ctx, &mut a, end + 1.5);
    let e = saved(&a, "Budget").expect("sent once the meeting is in");
    assert_eq!(e.teams_meeting_id.as_deref(), Some(ID));
    assert!(e.description.contains(&format!("Join the Teams meeting: {LINK}")), "{:?}", e.description);
    let _ = std::fs::remove_dir_all(dir);
}

/// I1: New Webex meeting chosen in the frame the quiet second ends: only Webex is asked.
#[test]
fn a_webex_meeting_chosen_as_the_quiet_second_ends_is_the_only_one() {
    let (mut a, dir, ctx, jobs) = with_the_option("teams-option-webex-at-the-second", "Budget");
    (a.session.prefs.webex_signed_in, a.session.prefs.webex_site) = (true, "acme.webex.com".into());
    open_webex_menu(&ctx, &mut a, SIZE);
    let entry = settled_text_at(&ctx, &mut a, "New Webex meeting", SIZE).expect("its menu's entry");
    let t = later(&ctx);
    type_guest(&ctx, &mut a, ZOE, t);
    let end = guests_left(&ctx, &mut a, t + 0.05);
    press_ending_at(&ctx, &mut a, &jobs, entry, end);
    for at in [end + 0.5, end + 2.0] {
        frame_at(&ctx, &mut a, vec![], at);
    }
    assert_eq!((webex_creates(&jobs), creates(&jobs).len()), (1, 0), "Webex alone");
    assert!(form(&mut a).webex_wait.is_some() && form(&mut a).teams_wait.is_none());
    let _ = std::fs::remove_dir_all(dir);
}

/// I1: the option's meeting on its way leaves Send on; pressed, its names checked meanwhile, the
/// invitation waits for the meeting once they're found, then goes with it.
#[test]
fn names_checked_while_the_meeting_is_on_its_way_wait_for_it() {
    let (mut a, dir, ctx, jobs) = with_the_option("teams-option-names", "Budget");
    let gal = with_exchange(&mut a);
    let w = a.ui.event_windows[0].0;
    let t = later(&ctx);
    type_guest(&ctx, &mut a, "Dupont", t);
    frame_at(&ctx, &mut a, vec![], t + 1.1);
    assert_eq!(creates(&jobs).len(), 1, "the option's meeting is on its way");
    let (send, on) = button(&ctx, &mut a, "Send").expect("the meeting's Send");
    assert!(on, "Send stays on while the option's meeting is made");
    let t = later(&ctx);
    press_at(&ctx, &mut a, send, t);
    assert!(a.ui.people.holding(MEETING_WINDOWS | w), "its names are checked first");
    directory_answers(&mut a, &gal, "dupont", &[("Anne Dupont", "anne.dupont@contoso.example")]);
    for at in [t + 0.1, t + 0.2, t + 0.3] {
        frame_at(&ctx, &mut a, vec![], at);
    }
    assert!(!a.ui.people.holding(MEETING_WINDOWS | w), "found");
    assert!(saved(&a, "Budget").is_none(), "not before the meeting");
    assert_eq!(form(&mut a).teams_send_held, Some("event.create"));
    answered_at(&ctx, &mut a, t + 0.5);
    let e = saved(&a, "Budget").expect("sent with it");
    assert_eq!(e.teams_meeting_id.as_deref(), Some(ID));
    assert_eq!(e.attendees.iter().map(|x| x.email.as_str()).collect::<Vec<_>>(), ["anne.dupont@contoso.example"]);
    assert_eq!(creates(&jobs).len(), 1);
    let _ = std::fs::remove_dir_all(dir);
}

/// m1: Send pressed just before the quiet second ends and let go just after: the press counts, and
/// the invitation waits for the meeting the option asked for in between.
#[test]
fn a_press_across_the_quiet_seconds_end_waits_for_the_meeting() {
    let (mut a, dir, ctx, jobs) = with_the_option("teams-option-press-across", "Budget");
    let send = settled_text_at(&ctx, &mut a, "Send", SIZE).expect("the meeting's Send");
    let t = later(&ctx);
    type_guest(&ctx, &mut a, ZOE, t);
    let press = |pressed| Event::PointerButton { pos: send, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame_at(&ctx, &mut a, vec![Event::PointerMoved(send)], t + 0.9);
    frame_at(&ctx, &mut a, vec![press(true)], t + 0.95);
    frame_at(&ctx, &mut a, vec![], t + 1.0);
    assert_eq!(creates(&jobs).len(), 1, "the option asked for its meeting");
    frame_at(&ctx, &mut a, vec![press(false)], t + 1.05);
    assert_eq!(form(&mut a).teams_send_held, Some("event.create"), "the press counted");
    assert!(saved(&a, "Budget").is_none());
    answered_at(&ctx, &mut a, t + 1.5);
    assert_eq!(saved(&a, "Budget").and_then(|e| e.teams_meeting_id).as_deref(), Some(ID));
    assert_eq!(creates(&jobs).len(), 1);
    let _ = std::fs::remove_dir_all(dir);
}

/// m2: a send waiting for the meeting, then a calendar chosen that can't hold it: the send is let
/// go with the meeting, and nothing goes later by itself, the given-up meeting's answer or another
/// meeting's.
#[test]
fn a_held_send_is_let_go_when_the_calendar_takes_the_meeting_out() {
    let (mut a, dir, ctx, jobs) = with_the_option("teams-option-held-moved", "Budget");
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
    let send = settled_text_at(&ctx, &mut a, "Send", SIZE).expect("the meeting's Send");
    let t = later(&ctx);
    type_guest(&ctx, &mut a, ZOE, t);
    press_at(&ctx, &mut a, send, t + 0.3);
    let ask = form(&mut a).teams_wait.expect("asked for at once");
    assert_eq!(form(&mut a).teams_send_held, Some("event.create"));
    choose(&mut a, &zoe, "AAMkGRAPH");
    frame_at(&ctx, &mut a, vec![], t + 0.5);
    assert_eq!((form(&mut a).teams_wait, form(&mut a).teams_send_held), (None, None), "let go with it");
    assert!(a.session.teams_meetings.asks.get(&ask).is_some_and(|x| x.abandoned));
    a.session.apply(MailEvent::Teams { event: teams::Event::MeetingCreated { ask, meeting: Some(meeting()), error: None } });
    frame_at(&ctx, &mut a, vec![], t + 0.6);
    assert_eq!(deletes(&jobs), [ID], "the meeting given up is deleted");
    choose(&mut a, "sample", "AAMkEWS");
    click(&ctx, &mut a, "Teams Meeting");
    answer(&ctx, &mut a, Some(meeting()), None);
    frame_sized(&ctx, &mut a, vec![], SIZE);
    assert_eq!(a.ui.event_windows.len(), 1, "nothing went by itself");
    assert!(saved(&a, "Budget").is_none());
    assert_eq!(creates(&jobs).len(), 2);
    let _ = std::fs::remove_dir_all(dir);
}

/// m3 (decision 3): Send pressed with Teams signed out: the first stops, the window saying why;
/// the second sends without Teams.
#[test]
fn send_signed_out_stops_once_then_goes_without_teams() {
    let (mut a, dir, ctx, jobs) = with_the_option("teams-option-send-signed-out", "Budget");
    a.session.prefs.teams.signed_in = false;
    let send = settled_text_at(&ctx, &mut a, "Send", SIZE).expect("the meeting's Send");
    let t = later(&ctx);
    type_guest(&ctx, &mut a, ZOE, t);
    press_at(&ctx, &mut a, send, t + 0.3);
    assert_eq!(a.ui.event_windows.len(), 1, "the first press stops");
    assert!(saved(&a, "Budget").is_none());
    assert_eq!((form(&mut a).teams_prompt, form(&mut a).teams_send_held), (Some(TeamsPrompt::NotAdded), None));
    settled_text_at(&ctx, &mut a, NOT_ADDED, SIZE).expect("said in the window");
    let t = later(&ctx);
    press_at(&ctx, &mut a, send, t);
    let e = saved(&a, "Budget").expect("the second sends");
    assert_eq!((e.teams_meeting_id, a.ui.event_windows.len()), (None, 0));
    assert!(creates(&jobs).is_empty(), "nothing went to Teams");
    let _ = std::fs::remove_dir_all(dir);
}

/// m4: an appointment saved without guests, opened again: its first guest brings the meeting.
#[test]
fn a_reopened_appointment_gets_one_with_its_first_guest() {
    let (mut a, dir, ctx, jobs) = with_the_option("teams-option-reopened", "Budget");
    a.ui.event_windows.clear();
    let made = a.session.run("event.create", &json!({"summary": "Review", "start": "2026-10-21T15:00"})).expect("made");
    let uid = made["uid"].as_str().expect("its uid").to_string();
    let start = a.session.find_event(&uid).map(|(_, e)| e.event.start).expect("stored");
    a.ui.dialog = Dialog::open_event(&a.session, &uid, start);
    frame_sized(&ctx, &mut a, vec![], SIZE);
    assert!(!form(&mut a).teams_auto_tried, "no guest yet");
    form(&mut a).inviting = true;
    let t = later(&ctx);
    type_guest(&ctx, &mut a, ZOE, t);
    frame_at(&ctx, &mut a, vec![], t + 0.5);
    assert!(creates(&jobs).is_empty());
    frame_at(&ctx, &mut a, vec![], t + 1.1);
    assert_eq!(creates(&jobs).len(), 1, "its first guest brought one");
    let _ = std::fs::remove_dir_all(dir);
}

/// Whatever asks first in a frame, a window waits for one meeting at a time: Webex isn't asked
/// beside a Teams meeting, nor one on its way; Teams isn't asked beside a Webex meeting on its way.
#[test]
fn webex_and_teams_are_never_asked_together() {
    use crate::pim_dialogs::teams_meeting::add_params;
    use crate::pim_dialogs::webex::webex_params;
    let (mut a, dir, ctx, jobs) = with_the_option("teams-option-one-at-a-time", "Budget");
    (a.session.prefs.webex_signed_in, a.session.prefs.webex_site) = (true, "acme.webex.com".into());
    let w = a.ui.event_windows[0].0;
    let (teams, webex) = (add_params(form(&mut a)), webex_params(form(&mut a)));
    a.act(crate::Action::ForTeamsMeeting(w, teams.clone()));
    a.act(crate::Action::ForWebex(w, webex.clone()));
    frame_sized(&ctx, &mut a, vec![], SIZE);
    assert_eq!((creates(&jobs).len(), webex_creates(&jobs)), (1, 0), "Teams on its way");
    answer(&ctx, &mut a, Some(meeting()), None);
    a.act(crate::Action::ForWebex(w, webex.clone()));
    frame_sized(&ctx, &mut a, vec![], SIZE);
    assert_eq!(webex_creates(&jobs), 0, "a Teams meeting in it");
    a.ui.event_windows.clear();
    open_new(&ctx, &mut a);
    form(&mut a).summary = "Budget".into();
    let w = a.ui.event_windows[0].0;
    a.act(crate::Action::ForWebex(w, webex));
    a.act(crate::Action::ForTeamsMeeting(w, teams));
    frame_sized(&ctx, &mut a, vec![], SIZE);
    assert_eq!((creates(&jobs).len(), webex_creates(&jobs)), (1, 1), "Webex on its way: Teams isn't asked");
    let _ = std::fs::remove_dir_all(dir);
}
