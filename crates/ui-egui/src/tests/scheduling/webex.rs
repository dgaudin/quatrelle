// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: File › Options › Calendar › Webex, and the appointment window's Webex Meeting.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use egui::accesskit::Role;
use quatrelle_engine::model::job::Secret;
use quatrelle_engine::model::{Job, WEBEX_REDIRECT_URI, WebexError, WebexMeeting};

use super::*;
use crate::pim_dialogs::EventForm;
use crate::pim_dialogs::webex::{error_text, leaving, take_meeting, webex_params};

/// The File page on Options, drawn tall enough for every card.
fn options_page(name: &str) -> (MailApp, std::path::PathBuf, egui::Context, egui::Vec2) {
    let (mut a, dir) = app(name);
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 5000.0);
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
    frame_sized(&ctx, &mut a, vec![], size);
    (a, dir, ctx, size)
}

/// The card draws signed out (the sign-in waiting on a client id, a typed one enabling it) and
/// signed in, with or without a name; a sign-in's refusal is said under it.
#[test]
fn the_webex_card_draws_signed_out_and_in() {
    let (mut a, dir, ctx, size) = options_page("webex-card");
    for _ in 0..3 {
        frame_sized(&ctx, &mut a, vec![], size);
    }
    let shapes = frame_shapes(&ctx, &mut a, vec![], size);
    for t in ["Webex", "Site", "Client ID", "Client secret", "Redirect URI", "Sign in to Webex"] {
        assert!(text_at(&shapes, t).is_some(), "{t}");
    }
    assert!(text_at(&shapes, "Sign out").is_none());
    a.session.webex.signing_in = true;
    let shapes = frame_shapes(&ctx, &mut a, vec![], size);
    assert!(text_at(&shapes, "Waiting for the browser…").is_some());
    a.session.webex.signing_in = false;
    a.session.webex.sign_in_error = Some(quatrelle_engine::model::WebexError::SignIn);
    let shapes = frame_shapes(&ctx, &mut a, vec![], size);
    assert!(text_at(&shapes, "Sign in to Webex again").is_some(), "the refusal is said");
    a.session.webex.sign_in_error = None;
    a.session.prefs.webex_signed_in = true;
    let shapes = frame_shapes(&ctx, &mut a, vec![], size);
    assert!(text_at(&shapes, "Signed in").is_some() && text_at(&shapes, "Sign out").is_some());
    a.session.prefs.webex_user = "Ana Diaz".into();
    let shapes = frame_shapes(&ctx, &mut a, vec![], size);
    assert!(text_at(&shapes, "Signed in as Ana Diaz").is_some());
    let _ = std::fs::remove_dir_all(dir);
}

/// A backend that keeps the jobs it is given.
struct Recorder(Rc<RefCell<Vec<Job>>>);

impl crate::Backend for Recorder {
    fn submit(&mut self, job: Job) {
        self.0.borrow_mut().push(job);
    }
    fn poll(&mut self) -> Vec<MailEvent> {
        Vec::new()
    }
}

/// The secret goes to the keychain once the field is left, and the card forgets it.
#[test]
fn the_secret_is_never_shown_again() {
    let (mut a, dir, ctx, size) = options_page("webex-secret");
    // The secret's box lines up with the site's, which shows its hint.
    let x = settled_text_at(&ctx, &mut a, "acme.webex.com", size).expect("the site's hint").x;
    // Its row has a note under the name: the box is centred a little lower.
    let y = settled_text_at(&ctx, &mut a, "Client secret", size).expect("its row").y + 12.0;
    let jobs = Rc::new(RefCell::new(Vec::new()));
    a.backend = Box::new(Recorder(jobs.clone()));
    click_at(&ctx, &mut a, egui::pos2(x, y), 1, size);
    frame_sized(&ctx, &mut a, vec![egui::Event::Text("s3cret".into())], size);
    frame_sized(&ctx, &mut a, vec![key(Key::Enter, Modifiers::NONE)], size);
    frame_sized(&ctx, &mut a, vec![], size);
    let sent = jobs.borrow().iter().any(|j| matches!(j, Job::SetWebexSecret { secret } if secret.0 == "s3cret"));
    assert!(sent, "webex.settings ran with the secret");
    let kept = ctx.data_mut(|d| d.get_temp::<String>(egui::Id::unique(("options-webex", "secret"))));
    assert_eq!(kept.as_deref(), Some(""), "the typed text is cleared");
    let _ = std::fs::remove_dir_all(dir);
}

/// The address to register with the integration is shown, for copying.
#[test]
fn the_redirect_uri_is_shown_to_copy() {
    let (mut a, dir, ctx, size) = options_page("webex-redirect");
    let shapes = frame_shapes(&ctx, &mut a, vec![], size);
    assert!(text_at(&shapes, WEBEX_REDIRECT_URI).is_some());
    let _ = std::fs::remove_dir_all(dir);
}

/// Task 8's way in: the action opens File › Options.
#[test]
fn webex_options_opens_the_card() {
    let (mut a, dir) = app("webex-options");
    a.act(crate::Action::WebexOptions);
    assert!(a.ui.backstage.as_ref().is_some_and(|b| b.page == crate::backstage::Page::Options));
    let _ = std::fs::remove_dir_all(dir);
}

const SIZE: egui::Vec2 = egui::vec2(1280.0, 900.0);
const LINK: &str = "https://acme.webex.com/acme/j.php?MTID=m1";
const ROOM: &str = "https://acme.webex.com/meet/ana";

type Jobs = Rc<RefCell<Vec<Job>>>;

/// An appointment called `title` in its window, in a session signed in to Webex, the jobs kept.
fn appointment(name: &str, title: &str) -> (MailApp, std::path::PathBuf, egui::Context, Jobs) {
    let (mut a, dir) = app(name);
    let ctx = egui::Context::default();
    let jobs = Rc::new(RefCell::new(Vec::new()));
    a.backend = Box::new(Recorder(jobs.clone()));
    a.session.prefs.webex_signed_in = true;
    a.session.prefs.webex_site = "acme.webex.com".into();
    frame_sized(&ctx, &mut a, vec![], SIZE);
    a.ui.dialog = Some(Dialog::new_event(&a.session, Some(NOW + 2 * 3600)));
    frame_sized(&ctx, &mut a, vec![], SIZE);
    form(&mut a).summary = title.into();
    (a, dir, ctx, jobs)
}

fn form(a: &mut MailApp) -> &mut EventForm {
    &mut a.ui.event_windows[0].1
}

fn meeting() -> WebexMeeting {
    WebexMeeting {
        id: "m1".into(),
        web_link: LINK.into(),
        meeting_number: "2623456701".into(),
        password: Secret("abc123".into()),
        sip_address: "2623456701@acme.webex.com".into(),
    }
}

/// Webex's jobs among those queued.
fn webex_jobs(jobs: &Jobs) -> usize {
    jobs.borrow().iter().filter(|j| matches!(j, Job::WebexCreate { .. } | Job::WebexUpdate { .. } | Job::WebexDelete { .. })).count()
}

/// The button named `label`, as a screen reader is told of it: where it is and whether it can be
/// pressed.
fn button(ctx: &egui::Context, a: &mut MailApp, label: &str) -> Option<(egui::Pos2, bool)> {
    ctx.enable_accesskit();
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SIZE)), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
    let (_, n) = nodes.iter().find(|(_, n)| n.role() == Role::Button && n.label() == Some(label))?;
    let r = n.bounds()?;
    Some((egui::pos2(((r.x0 + r.x1) / 2.0) as f32, ((r.y0 + r.y1) / 2.0) as f32), !n.is_disabled()))
}

/// Webex Meeting's plain click.
fn click_webex(ctx: &egui::Context, a: &mut MailApp) {
    let at = settled_text_at(ctx, a, "Webex Meeting", SIZE).expect("the button");
    click_at(ctx, a, at, 1, SIZE);
}

/// The menu beside Webex Meeting opened, in a window of `size`.
pub(super) fn open_webex_menu(ctx: &egui::Context, a: &mut MailApp, size: egui::Vec2) {
    settled_text_at(ctx, a, "Webex Meeting", size).expect("the button");
    let (at, _) = button(ctx, a, "Webex Meeting options").expect("its menu's button");
    click_at(ctx, a, at, 1, size);
}

/// A frame in which the window is closed.
fn close_window(ctx: &egui::Context, a: &mut MailApp) {
    let input = RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SIZE)),
        viewports: std::iter::once((egui::ViewportId::ROOT, egui::ViewportInfo { events: vec![egui::ViewportEvent::Close], ..Default::default() }))
            .collect(),
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    assert!(a.ui.event_windows.is_empty(), "closed");
    frame_sized(ctx, a, vec![], SIZE);
}

/// A plain click on a titled appointment asks Webex for a meeting, and the window waits for it.
#[test]
fn the_plain_click_creates_a_meeting() {
    let (mut a, dir, ctx, jobs) = appointment("webex-click", "Budget");
    click_webex(&ctx, &mut a);
    let asked = jobs.borrow().iter().find_map(|j| match j {
        Job::WebexCreate { ask, site, fields, quiet, .. } => Some((*ask, site.clone(), fields.title.clone(), *quiet)),
        _ => None,
    });
    let (ask, site, title, quiet) = asked.expect("the meeting was asked for");
    assert_eq!((site.as_str(), title.as_str(), quiet), ("acme.webex.com", "Budget", false));
    assert_eq!(form(&mut a).webex_wait, Some(ask));
    assert!(a.session.webex.asks.contains_key(&ask));
    let _ = std::fs::remove_dir_all(dir);
}

/// The menu's New Webex meeting is the plain click, and the times and guests go along.
#[test]
fn the_menu_entry_is_the_plain_click() {
    let (mut a, dir, ctx, jobs) = appointment("webex-menu-new", "Budget");
    form(&mut a).inviting = true;
    let p = webex_params(form(&mut a));
    assert_eq!(p["summary"], "Budget");
    assert_eq!(p["quiet"], true, "its guests hear from the invitation alone");
    let (date, start) = (form(&mut a).date.clone(), form(&mut a).start.clone());
    assert_eq!(p["start"], format!("{date}T{start}"));
    assert!(p.get("end").is_some() && p.get("allDay").is_some() && p.get("repeat").is_some());
    open_webex_menu(&ctx, &mut a, SIZE);
    let at = settled_text_at(&ctx, &mut a, "New Webex meeting", SIZE).expect("the entry");
    click_at(&ctx, &mut a, at, 1, SIZE);
    assert_eq!(webex_jobs(&jobs), 1);
    assert!(form(&mut a).webex_wait.is_some());
    let _ = std::fs::remove_dir_all(dir);
}

/// No title, or no date: the window says so, and nothing goes to Webex.
#[test]
fn no_title_says_so_and_sends_nothing() {
    let (mut a, dir, ctx, jobs) = appointment("webex-no-title", "  ");
    click_webex(&ctx, &mut a);
    assert_eq!(form(&mut a).error.as_deref(), Some("A meeting needs a title and a start"));
    assert!(form(&mut a).webex_wait.is_none());
    form(&mut a).error = None;
    form(&mut a).summary = "Budget".into();
    form(&mut a).date.clear();
    click_webex(&ctx, &mut a);
    assert_eq!(form(&mut a).error.as_deref(), Some("A meeting needs a title and a start"));
    assert_eq!(webex_jobs(&jobs), 0);
    let _ = std::fs::remove_dir_all(dir);
}

/// Signed out, or with no site to schedule on, the click opens Options at the Webex card.
#[test]
fn signed_out_opens_options() {
    let (mut a, dir, ctx, jobs) = appointment("webex-signed-out", "Budget");
    a.session.prefs.webex_signed_in = false;
    click_webex(&ctx, &mut a);
    assert!(a.ui.backstage.as_ref().is_some_and(|b| b.page == crate::backstage::Page::Options));
    assert!(form(&mut a).webex_wait.is_none() && form(&mut a).error.is_none());
    a.ui.backstage = None;
    a.session.prefs.webex_signed_in = true;
    a.session.prefs.webex_site.clear();
    click_webex(&ctx, &mut a);
    assert!(a.ui.backstage.is_some(), "the site is asked for there");
    assert_eq!(webex_jobs(&jobs), 0);
    let _ = std::fs::remove_dir_all(dir);
}

/// Webex asked for a new sign-in: the window says so, and the next click goes to Options, the
/// refusal staying in the window.
#[test]
fn signing_in_again_goes_to_options_and_keeps_the_refusal() {
    let (mut a, dir, ctx, jobs) = appointment("webex-lapsed", "Budget");
    a.session.apply(quatrelle_engine::model::Event::WebexCreated { ask: 99, meeting: None, error: Some(WebexError::SignIn) });
    form(&mut a).error = Some(error_text(&WebexError::SignIn));
    a.session.prefs.webex_signed_in = false;
    click_webex(&ctx, &mut a);
    assert!(a.ui.backstage.as_ref().is_some_and(|b| b.page == crate::backstage::Page::Options));
    assert_eq!(form(&mut a).error.as_deref(), Some("Sign in to Webex again"));
    assert_eq!(webex_jobs(&jobs), 0);
    let _ = std::fs::remove_dir_all(dir);
}

/// The answer's place, notes, Join and id, as the Scheduler writes them; a place typed stays.
#[test]
fn the_answer_goes_into_the_appointment() {
    let mut f = EventForm { description: "Agenda\n".into(), ..Default::default() };
    take_meeting(&mut f, &meeting());
    assert_eq!(f.location, LINK);
    let notes =
        format!("Agenda\n\nJoin the Webex meeting:\n{LINK}\nMeeting number: 2623456701\nPassword: abc123\nVideo: 2623456701@acme.webex.com\n");
    assert_eq!(f.description, notes);
    assert_eq!((f.join.as_deref(), f.webex_id.as_deref(), f.webex_fresh), (Some(LINK), Some("m1"), true));
    let mut f = EventForm { location: "Salle B".into(), ..Default::default() };
    take_meeting(&mut f, &meeting());
    assert_eq!(f.location, "Salle B");
    assert!(f.description.starts_with("Join the Webex meeting:\n"), "{:?}", f.description);
    let e = quatrelle_engine::model::ical::CalEvent { location: f.location.clone(), description: f.description.clone(), ..Default::default() };
    assert_eq!(e.join_link().as_deref(), Some(LINK), "what Join finds once it's saved");
}

/// A value Webex didn't give writes no line.
#[test]
fn empty_values_write_no_line() {
    let m = WebexMeeting { password: Secret(String::new()), ..meeting() };
    let mut f = EventForm::default();
    take_meeting(&mut f, &m);
    assert!(!f.description.contains("Password") && f.description.contains("Meeting number: 2623456701"), "{:?}", f.description);
    let m = WebexMeeting { meeting_number: String::new(), password: Secret(String::new()), sip_address: String::new(), ..meeting() };
    let mut f = EventForm::default();
    take_meeting(&mut f, &m);
    assert_eq!(f.description, format!("Join the Webex meeting:\n{LINK}\n"));
}

/// In Arabic and Hebrew a value filled in with `tf!` is wrapped in invisible marks: in the notes
/// they'd go out in the invitation, and along with a password a guest copies.
#[test]
fn the_notes_hold_the_values_as_webex_gave_them() {
    let notes = ["he", "ar"].map(|code| {
        crate::i18n::set_current(crate::i18n::Lang::from_code(code).unwrap_or(crate::i18n::Lang::EN));
        let mut f = EventForm::default();
        take_meeting(&mut f, &meeting());
        f.description
    });
    crate::i18n::set_current(crate::i18n::Lang::EN);
    for n in notes {
        assert!(!n.contains(['\u{2068}', '\u{2069}']), "{n:?}");
        for value in ["2623456701", "abc123", "2623456701@acme.webex.com"] {
            assert!(n.contains(value), "{n:?}");
        }
        assert!(!n.contains("Password"), "said in the language: {n:?}");
    }
}

/// Whatever Webex refuses is said in the window, and the appointment stays as it was.
#[test]
fn a_refused_creation_shows_and_changes_nothing() {
    let (mut a, dir, ctx, _) = appointment("webex-refused", "Budget");
    form(&mut a).location = "Salle B".into();
    form(&mut a).description = "Agenda".into();
    let failures = [
        (WebexError::SignIn, "Sign in to Webex again"),
        (WebexError::Forbidden, "Your Webex account can't schedule meetings on this site"),
        (WebexError::Refused { status: 429, message: "Too many requests".into() }, "Webex answered 429: Too many requests"),
        (WebexError::Failed("the connection was reset".into()), "the connection was reset"),
    ];
    for (ask, (e, said)) in failures.into_iter().enumerate() {
        let ask = ask as u64 + 1;
        assert_eq!(error_text(&e), said);
        a.session.webex.answers.insert(ask, Err(e));
        form(&mut a).webex_wait = Some(ask);
        form(&mut a).error = None;
        frame_sized(&ctx, &mut a, vec![], SIZE);
        let f = form(&mut a);
        assert_eq!(f.error.as_deref(), Some(said));
        assert_eq!((f.location.as_str(), f.description.as_str(), f.join.as_deref()), ("Salle B", "Agenda", None));
        assert!(f.webex_wait.is_none() && f.webex_id.is_none() && !f.webex_fresh);
    }
    assert!(a.session.webex.answers.is_empty(), "each taken once");
    let _ = std::fs::remove_dir_all(dir);
}

/// Use my personal room puts the room in as ever: nothing goes to Webex.
#[test]
fn the_personal_room_stays_a_choice() {
    let (mut a, dir, ctx, jobs) = appointment("webex-room-choice", "Budget");
    a.session.prefs.webex_room = ROOM.into();
    open_webex_menu(&ctx, &mut a, SIZE);
    let at = settled_text_at(&ctx, &mut a, "Use my personal room", SIZE).expect("the entry");
    click_at(&ctx, &mut a, at, 1, SIZE);
    let f = form(&mut a);
    assert_eq!((f.join.as_deref(), f.location.as_str()), (Some(ROOM), ROOM));
    assert!(f.description.contains(ROOM) && f.webex_wait.is_none() && f.webex_id.is_none());
    assert_eq!(webex_jobs(&jobs), 0);
    let _ = std::fs::remove_dir_all(dir);
}

/// One meeting at a time, and only for the user's own appointment: Webex Meeting is off while one
/// is on its way, while the appointment holds one, and on someone else's event, whose menu keeps
/// the personal room.
#[test]
fn a_meeting_goes_only_where_it_can_be_kept() {
    let (mut a, dir, ctx, jobs) = appointment("webex-one-only", "Budget");
    settled_text_at(&ctx, &mut a, "Webex Meeting", SIZE).expect("the button");
    assert_eq!(button(&ctx, &mut a, "Webex Meeting").map(|b| b.1), Some(true));
    form(&mut a).webex_wait = Some(7);
    assert_eq!(button(&ctx, &mut a, "Webex Meeting").map(|b| b.1), Some(false));
    assert_eq!(button(&ctx, &mut a, "Webex Meeting options").map(|b| b.1), Some(false), "the personal room waits too");
    a.act(crate::Action::ForWebex(a.ui.event_windows[0].0, webex_params(&a.ui.event_windows[0].1)));
    form(&mut a).webex_wait = None;
    form(&mut a).webex_id = Some("m0".into());
    assert_eq!(button(&ctx, &mut a, "Webex Meeting").map(|b| b.1), Some(false));
    form(&mut a).webex_id = None;
    form(&mut a).can_invite = false;
    assert_eq!(button(&ctx, &mut a, "Webex Meeting").map(|b| b.1), Some(false));
    open_webex_menu(&ctx, &mut a, SIZE);
    let at = text_at(&frame_shapes(&ctx, &mut a, vec![], SIZE), "New Webex meeting").expect("the entry");
    click_at(&ctx, &mut a, at, 1, SIZE);
    a.session.prefs.webex_room = ROOM.into();
    let at = text_at(&frame_shapes(&ctx, &mut a, vec![], SIZE), "Use my personal room").expect("the room stays");
    click_at(&ctx, &mut a, at, 1, SIZE);
    assert_eq!(form(&mut a).join.as_deref(), Some(ROOM));
    assert_eq!(webex_jobs(&jobs), 0, "none of these asked Webex");
    let _ = std::fs::remove_dir_all(dir);
}

/// A colleague's meeting from a server that names no organizer reads as the user's to invite to,
/// but no meeting made for it could be kept: Webex Meeting is off, as the engine would never
/// follow it. A lone appointment opened again can have one.
#[test]
fn a_colleagues_meeting_with_no_organizer_schedules_nothing() {
    let (mut a, dir, ctx, _) = appointment("webex-colleague", "Budget");
    let uid = saved_and_opened(&ctx, &mut a);
    assert_eq!(button(&ctx, &mut a, "Webex Meeting").map(|b| b.1), Some(true));
    a.ui.event_windows.clear();
    let guest = quatrelle_engine::model::ical::Attendee { email: "zoe@studio.example".into(), ..Default::default() };
    let e = a.session.pim_mut("sample").events.iter_mut().find(|e| e.event.uid == uid).expect("saved");
    e.event.attendees = vec![guest];
    let start = e.event.start;
    a.ui.dialog = Dialog::open_event(&a.session, &uid, start);
    frame_sized(&ctx, &mut a, vec![], SIZE);
    assert!(form(&mut a).can_invite && form(&mut a).join.is_none());
    assert_eq!(button(&ctx, &mut a, "Webex Meeting").map(|b| b.1), Some(false));
    let _ = std::fs::remove_dir_all(dir);
}

/// Save waits for the meeting on its way, and goes with it once it's in.
#[test]
fn save_waits_for_the_webex_answer() {
    let (mut a, dir, ctx, _) = appointment("webex-save-waits", "Budget");
    let w = a.ui.event_windows[0].0;
    form(&mut a).webex_wait = Some(7);
    let p = form(&mut a).params();
    a.for_event(w, "event.create", p.clone());
    assert_eq!(a.ui.event_windows.len(), 1, "nothing saved, the window stays");
    assert!(a.session.agenda(NOW, NOW + 86_400).iter().all(|i| i.summary != "Budget"));
    settled_text_at(&ctx, &mut a, "Creating the Webex meeting…", SIZE).expect("said in the window");
    assert_eq!(button(&ctx, &mut a, "Save & Close").map(|b| b.1), Some(false), "Save is off");
    a.session.webex.answers.insert(7, Ok(meeting()));
    frame_sized(&ctx, &mut a, vec![], SIZE);
    assert!(form(&mut a).webex_wait.is_none());
    assert_eq!(button(&ctx, &mut a, "Save & Close").map(|b| b.1), Some(true), "Save is back");
    let p = form(&mut a).params();
    a.for_event(w, "event.create", p);
    assert!(a.ui.event_windows.is_empty(), "saved");
    let _ = std::fs::remove_dir_all(dir);
}

/// Saved, the appointment holds the id and the meeting is the appointment's: nothing deletes it.
#[test]
fn saving_sends_the_id_and_keeps_the_meeting() {
    let (mut a, dir, ctx, jobs) = appointment("webex-saved", "Budget");
    let w = a.ui.event_windows[0].0;
    take_meeting(form(&mut a), &meeting());
    let p = form(&mut a).params();
    assert_eq!(p["webexId"], "m1");
    a.for_event(w, "event.create", p);
    assert!(a.ui.event_windows.is_empty());
    frame_sized(&ctx, &mut a, vec![], SIZE);
    assert!(jobs.borrow().iter().all(|j| !matches!(j, Job::WebexDelete { .. })));
    let saved = a.session.agenda(NOW, NOW + 86_400).into_iter().find(|i| i.summary == "Budget").expect("saved");
    assert_eq!(a.session.find_event(&saved.uid).and_then(|(_, e)| e.event.webex_id.clone()).as_deref(), Some("m1"));
    let _ = std::fs::remove_dir_all(dir);
}

/// The appointment in the window saved, and opened again in a window of its own: its uid.
fn saved_and_opened(ctx: &egui::Context, a: &mut MailApp) -> String {
    let w = a.ui.event_windows[0].0;
    let p = form(a).params();
    a.for_event(w, "event.create", p);
    let saved = a.session.agenda(NOW, NOW + 86_400).into_iter().find(|i| i.summary == "Budget").expect("saved");
    a.ui.dialog = Dialog::open_event(&a.session, &saved.uid, saved.occurrence);
    frame_sized(ctx, a, vec![], SIZE);
    saved.uid
}

/// An appointment opened again holds its meeting, and closing it leaves that alone.
#[test]
fn a_saved_meeting_outlives_its_window() {
    let (mut a, dir, ctx, jobs) = appointment("webex-reopened", "Budget");
    take_meeting(form(&mut a), &meeting());
    saved_and_opened(&ctx, &mut a);
    let f = form(&mut a);
    assert_eq!((f.webex_id.as_deref(), f.webex_fresh), (Some("m1"), false));
    assert_eq!(leaving(f), None);
    close_window(&ctx, &mut a);
    assert_eq!(webex_jobs(&jobs), 0);
    let _ = std::fs::remove_dir_all(dir);
}

/// Closed unsaved, the window takes its meeting with it.
#[test]
fn closing_unsaved_deletes_the_meeting() {
    let (mut a, dir, ctx, jobs) = appointment("webex-close", "Budget");
    take_meeting(form(&mut a), &meeting());
    form(&mut a).inviting = true;
    close_window(&ctx, &mut a);
    let deleted = jobs.borrow().iter().find_map(|j| match j {
        Job::WebexDelete { id, quiet, .. } => Some((id.clone(), *quiet)),
        _ => None,
    });
    assert_eq!(deleted, Some(("m1".to_string(), true)));
    let _ = std::fs::remove_dir_all(dir);
}

/// Closed while its meeting is on its way, the window gives it up: the answer has nobody to go
/// to, and the engine deletes what it brings.
#[test]
fn closing_during_the_creation_abandons_it() {
    let (mut a, dir, ctx, _) = appointment("webex-close-waiting", "Budget");
    click_webex(&ctx, &mut a);
    let ask = form(&mut a).webex_wait.expect("waiting");
    assert_eq!(leaving(form(&mut a)), Some(json!({"ask": ask})));
    close_window(&ctx, &mut a);
    assert!(a.session.webex.asks.get(&ask).is_some_and(|x| x.abandoned));
    let _ = std::fs::remove_dir_all(dir);
}

/// Deleting the appointment from its window takes the meeting just made with it, though the
/// event never held its id; a meeting on its way is given up the same way.
#[test]
fn deleting_the_event_with_a_fresh_meeting_leaves_nothing() {
    let (mut a, dir, ctx, jobs) = appointment("webex-delete", "Budget");
    let uid = saved_and_opened(&ctx, &mut a);
    let w = a.ui.event_windows[0].0;
    take_meeting(form(&mut a), &meeting());
    a.for_event(w, "event.delete", json!({"uid": uid}));
    assert!(a.ui.event_windows.is_empty());
    frame_sized(&ctx, &mut a, vec![], SIZE);
    assert!(jobs.borrow().iter().any(|j| matches!(j, Job::WebexDelete { id, .. } if id == "m1")));

    a.ui.dialog = Some(Dialog::new_event(&a.session, Some(NOW + 2 * 3600)));
    frame_sized(&ctx, &mut a, vec![], SIZE);
    form(&mut a).summary = "Budget".into();
    let uid = saved_and_opened(&ctx, &mut a);
    click_webex(&ctx, &mut a);
    let (w, ask) = (a.ui.event_windows[0].0, form(&mut a).webex_wait.expect("waiting"));
    a.for_event(w, "event.delete", json!({"uid": uid}));
    assert!(a.ui.event_windows.is_empty());
    assert!(a.session.webex.asks.get(&ask).is_some_and(|x| x.abandoned));
    let _ = std::fs::remove_dir_all(dir);
}

/// A backend that keeps the jobs it is given, counts its polls, and answers each Webex delete as
/// the worker does, with `error`.
struct Answering {
    jobs: Jobs,
    polls: Rc<Cell<usize>>,
    error: Option<WebexError>,
    answers: Vec<MailEvent>,
}

impl crate::Backend for Answering {
    fn submit(&mut self, job: Job) {
        if let Job::WebexDelete { id, .. } = &job {
            self.answers.push(MailEvent::WebexDeleted { id: id.clone(), error: self.error.clone() });
        }
        self.jobs.borrow_mut().push(job);
    }
    fn poll(&mut self) -> Vec<MailEvent> {
        self.polls.set(self.polls.get() + 1);
        std::mem::take(&mut self.answers)
    }
}

/// The deletes queued.
fn deletes(jobs: &Jobs) -> usize {
    jobs.borrow().iter().filter(|j| matches!(j, Job::WebexDelete { .. })).count()
}

/// The app answering Webex's deletes with `error`, the jobs kept and the polls counted.
fn answering(a: &mut MailApp, error: Option<WebexError>) -> (Jobs, Rc<Cell<usize>>) {
    let (jobs, polls) = (Rc::new(RefCell::new(Vec::new())), Rc::new(Cell::new(0)));
    a.backend = Box::new(Answering { jobs: jobs.clone(), polls: polls.clone(), error, answers: Vec::new() });
    (jobs, polls)
}

/// Quitting deletes the fresh meeting of an open window, once however often it's asked, and stops
/// waiting as soon as Webex answers, deleted or refused.
#[test]
fn quitting_deletes_a_fresh_meeting_once() {
    for (n, error) in [None, Some(WebexError::Forbidden)].into_iter().enumerate() {
        let (mut a, dir, _ctx, _) = appointment(&format!("webex-quit-{n}"), "Budget");
        let (jobs, _) = answering(&mut a, error);
        take_meeting(form(&mut a), &meeting());
        assert!(a.webex_closing(), "the delete is on its way");
        assert_eq!(deletes(&jobs), 1);
        assert!(a.ui.event_windows.is_empty());
        assert!(!a.webex_closing(), "answered");
        assert!(!a.webex_closing());
        assert_eq!(deletes(&jobs), 1, "once");
        assert_eq!(a.session.webex.deleting, 0);
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Quitting during a creation gives it up, and waits for what it brings: a meeting is deleted.
#[test]
fn quitting_during_a_creation_deletes_what_it_brings() {
    let (mut a, dir, ctx, _) = appointment("webex-quit-waiting", "Budget");
    click_webex(&ctx, &mut a);
    let ask = form(&mut a).webex_wait.expect("waiting");
    let (jobs, _) = answering(&mut a, None);
    assert!(a.webex_closing(), "its answer is awaited");
    assert_eq!(deletes(&jobs), 0);
    a.session.apply(MailEvent::WebexCreated { ask, meeting: Some(meeting()), error: None });
    assert!(a.webex_closing(), "what it brought is being deleted");
    assert_eq!(deletes(&jobs), 1);
    assert!(!a.webex_closing());
    let _ = std::fs::remove_dir_all(dir);
}

/// A creation that fails while Quatrelle closes leaves nothing to wait for.
#[test]
fn quitting_stops_at_a_failed_creation() {
    let (mut a, dir, ctx, _) = appointment("webex-quit-failed", "Budget");
    click_webex(&ctx, &mut a);
    let ask = form(&mut a).webex_wait.expect("waiting");
    let (jobs, _) = answering(&mut a, None);
    assert!(a.webex_closing());
    a.session.apply(MailEvent::WebexCreated { ask, meeting: None, error: Some(WebexError::Forbidden) });
    assert!(!a.webex_closing());
    assert_eq!(deletes(&jobs), 0);
    let _ = std::fs::remove_dir_all(dir);
}

/// Signed out, the delete is refused and nothing goes: quitting waits for nothing.
#[test]
fn quitting_signed_out_waits_for_nothing() {
    let (mut a, dir, _ctx, _) = appointment("webex-quit-signed-out", "Budget");
    let (jobs, polls) = answering(&mut a, None);
    take_meeting(form(&mut a), &meeting());
    a.session.prefs.webex_signed_in = false;
    assert!(!a.webex_closing());
    assert_eq!((deletes(&jobs), polls.get()), (0, 0));
    let _ = std::fs::remove_dir_all(dir);
}

/// No window with a meeting to leave and nothing on its way: quitting adds no wait, not a look at
/// the workers.
#[test]
fn quitting_with_nothing_on_its_way_adds_no_wait() {
    let (mut a, dir, _ctx, _) = appointment("webex-quit-nothing", "Budget");
    let (jobs, polls) = answering(&mut a, None);
    assert!(!a.webex_closing());
    assert_eq!((jobs.borrow().len(), polls.get()), (0, 0));
    let _ = std::fs::remove_dir_all(dir);
}

/// A delete already on its way (an appointment deleted just before quitting) is waited for like
/// a window's, until Webex answers.
#[test]
fn quitting_waits_for_a_delete_already_on_its_way() {
    let (mut a, dir, _ctx, _) = appointment("webex-quit-delete", "Budget");
    let (jobs, _) = answering(&mut a, None);
    a.session.run("webex.deleteMeeting", &json!({"id": "m0"})).expect("asked");
    assert_eq!(a.session.webex.deleting, 1);
    assert!(a.webex_closing(), "its answer is awaited");
    assert_eq!(deletes(&jobs), 1);
    assert!(!a.webex_closing(), "answered");
    assert_eq!(a.session.webex.deleting, 0);
    let _ = std::fs::remove_dir_all(dir);
}

/// A bad date is the engine's to refuse: the window says so, and waits for nothing.
#[test]
fn a_refused_creation_says_why_in_the_window() {
    let (mut a, dir, ctx, jobs) = appointment("webex-engine-refuses", "Budget");
    form(&mut a).date = "someday".into();
    click_webex(&ctx, &mut a);
    let f = form(&mut a);
    assert!(f.error.as_deref().is_some_and(|e| e.contains("someday") && !e.starts_with("bad parameters")), "{:?}", f.error);
    assert!(f.webex_wait.is_none());
    assert_eq!(webex_jobs(&jobs), 0);
    let _ = std::fs::remove_dir_all(dir);
}

/// A save refused keeps the window and its meeting; closing it then takes the meeting along.
#[test]
fn closing_after_a_refused_save_deletes_the_meeting() {
    let (mut a, dir, ctx, jobs) = appointment("webex-save-refused", "Budget");
    let w = a.ui.event_windows[0].0;
    take_meeting(form(&mut a), &meeting());
    form(&mut a).date = "someday".into();
    let p = form(&mut a).params();
    a.for_event(w, "event.create", p);
    assert_eq!(a.ui.event_windows.len(), 1, "the window stays");
    let f = form(&mut a);
    assert!(f.error.is_some() && f.webex_fresh && f.webex_id.is_some());
    frame_sized(&ctx, &mut a, vec![], SIZE);
    assert_eq!(webex_jobs(&jobs), 0, "the meeting waits for what the window does next");
    close_window(&ctx, &mut a);
    assert_eq!(deletes(&jobs), 1);
    let _ = std::fs::remove_dir_all(dir);
}
