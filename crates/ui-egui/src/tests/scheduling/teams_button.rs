// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: the appointment window's Teams Meeting button and its menu. What a click asks of the
//! engine (one request at a time), the answer and each refusal said in the window, the permission
//! asked for there and in File › Options › Teams, Webex and Teams greying each other, the calendars
//! that take a meeting, and what a window leaves unsaved when it closes, or Quatrelle quits.

use std::cell::RefCell;
use std::rc::Rc;

use quatrelle_engine::model::pim::teams::{self, TeamsMeeting, TeamsMeetingError};
use quatrelle_engine::model::pim::{CalendarInfo, LOCAL};
use quatrelle_engine::model::{Job, WebexMeeting};
use quatrelle_engine::{TeamsMeetingPath, WebexRoom};

use super::teams_meeting::{ID, Jobs, LINK, OPTIONS, SIZE, allowed, appointment, backend, click, creates, deletes, meeting, open_new, signed_in};
use super::webex::{button, open_webex_menu};
use super::*;
use crate::i18n::{Lang, tr};
use crate::pim_dialogs::teams_meeting::{TeamsButton, add_params, button_state, error_text};
use crate::pim_dialogs::{EventForm, TeamsPrompt};

/// What the window says once a calendar that can't hold its meeting is chosen.
const MOVED: &str = "The Teams meeting was removed: add it again for this calendar";
/// Why Add a Teams meeting is off, and why Webex is.
const WEBEX_FIRST: &str = "This appointment has a Webex meeting or room: remove it first";
const NOT_YOURS: &str = "A Teams meeting can only be added to an appointment you organize";
const TEAMS_FIRST: &str = "This appointment has a Teams meeting: remove it first";
const ROOM: &str = "https://acme.webex.com/meet/ana";

/// Services that keep the pages they open.
struct Opening(Rc<RefCell<Vec<String>>>);

impl crate::Services for Opening {
    fn clock(&self) -> (i64, i32) {
        (NOW, 0)
    }
    fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
        Err("refused".into())
    }
    fn open_url(&mut self, url: &str) -> Result<(), String> {
        self.0.borrow_mut().push(url.to_string());
        Ok(())
    }
}

/// The pages the app opens from now on.
fn opening(a: &mut MailApp) -> Rc<RefCell<Vec<String>>> {
    let opened = Rc::new(RefCell::new(Vec::new()));
    a.services = Box::new(Opening(opened.clone()));
    opened
}

fn form(a: &mut MailApp) -> &mut EventForm {
    &mut a.ui.event_windows[0].1
}

/// Whether Teams' sign-in was asked for with the meetings' permission.
fn authorizing(jobs: &Jobs) -> bool {
    jobs.borrow().iter().any(|j| matches!(j, Job::Teams { job: teams::Job::SignIn { meetings: true, .. } }))
}

/// The menu beside Teams Meeting opened.
fn open_menu(ctx: &egui::Context, a: &mut MailApp) {
    settled_text_at(ctx, a, "Teams Meeting", SIZE).expect("the button");
    let (at, _) = button(ctx, a, "Teams Meeting menu").expect("its menu's button");
    click_at(ctx, a, at, 1, SIZE);
}

/// Whether the button `label` can be pressed (`None`: it isn't drawn).
fn pressable(ctx: &egui::Context, a: &mut MailApp, label: &str) -> Option<bool> {
    button(ctx, a, label).map(|b| b.1)
}

/// The creation window 0 waits for, answered.
fn answer(ctx: &egui::Context, a: &mut MailApp, meeting: Option<TeamsMeeting>, error: Option<TeamsMeetingError>) {
    let ask = form(a).teams_wait.expect("a creation on its way");
    a.session.apply(MailEvent::Teams { event: teams::Event::MeetingCreated { ask, meeting, error } });
    frame_sized(ctx, a, vec![], SIZE);
}

/// The plain click, answered: a meeting made in the window and not saved.
fn made(ctx: &egui::Context, a: &mut MailApp) {
    click(ctx, a, "Teams Meeting");
    answer(ctx, a, Some(meeting()), None);
    assert_eq!(form(a).teams_meeting_id.as_deref(), Some(ID));
}

/// The appointment in window 0 saved, and opened again in a window of its own: its uid.
fn saved_and_opened(ctx: &egui::Context, a: &mut MailApp) -> String {
    let w = a.ui.event_windows[0].0;
    let p = form(a).params();
    a.for_event(w, "event.create", p);
    assert!(a.ui.event_windows.is_empty(), "saved");
    let saved = a.session.agenda(NOW, NOW + 86_400).into_iter().find(|i| i.summary == "Budget").expect("saved");
    a.ui.dialog = Dialog::open_event(&a.session, &saved.uid, saved.occurrence);
    frame_sized(ctx, a, vec![], SIZE);
    saved.uid
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

/// The stored appointment's Teams meeting.
fn kept_id(a: &MailApp, uid: &str) -> Option<String> {
    a.session.find_event(uid).and_then(|(_, e)| e.event.teams_meeting_id.clone())
}

/// Calendar `id` added to `account` (someone else's when `not_own`).
fn calendar(a: &mut MailApp, account: &str, id: &str, not_own: bool) {
    let info = CalendarInfo { id: id.into(), name: format!("Calendar {id}"), not_own, ..Default::default() };
    a.session.pim_mut(account).calendars.push(info);
}

/// Window 0's calendar made `account`'s `id`.
fn choose(a: &mut MailApp, account: &str, id: &str) {
    let f = form(a);
    f.calendar = f.calendars.iter().position(|c| c.0 == account && c.1 == id).expect("listed");
}

/// Account `id` made a Microsoft 365 mailbox, or an Exchange one.
fn served(a: &mut MailApp, id: &str, graph: bool) {
    let config = &mut a.session.accounts.iter_mut().find(|x| x.config.id == id).expect("the account").config;
    if graph {
        config.graph = Some(quatrelle_engine::model::GRAPH_URL.into());
    } else {
        config.ews = Some("https://mail.contoso.example/EWS/Exchange.asmx".into());
    }
}

/// A plain click on a titled appointment asks Teams for a meeting: the window waits for it, says
/// so, and Save is off meanwhile.
#[test]
fn the_plain_click_asks_for_a_meeting() {
    let (mut a, dir, ctx, jobs) = appointment("teams-click", "Budget");
    let webex = settled_text_at(&ctx, &mut a, "Webex Meeting", SIZE).expect("Webex Meeting");
    let teams = settled_text_at(&ctx, &mut a, "Teams Meeting", SIZE).expect("Teams Meeting");
    assert!(teams.x > webex.x && (teams.y - webex.y).abs() < 12.0, "right after Webex Meeting in the ribbon: {webex:?} {teams:?}");
    click(&ctx, &mut a, "Teams Meeting");
    let asked = jobs.borrow().iter().find_map(|j| match j {
        Job::Teams { job: teams::Job::MeetingCreate { ask, fields } } => Some((*ask, fields.title.clone())),
        _ => None,
    });
    let (ask, title) = asked.expect("the meeting was asked for");
    assert_eq!(title, "Budget");
    assert_eq!(form(&mut a).teams_wait, Some(ask));
    assert!(a.session.teams_meetings.asks.contains_key(&ask));
    settled_text_at(&ctx, &mut a, "Creating the Teams meeting…", SIZE).expect("said in the window");
    assert_eq!(pressable(&ctx, &mut a, "Save & Close"), Some(false), "Save is off");
    let _ = std::fs::remove_dir_all(dir);
}

/// The menu's Add a Teams meeting is the plain click.
#[test]
fn the_menu_entry_is_the_plain_click() {
    let (mut a, dir, ctx, jobs) = appointment("teams-menu-add", "Budget");
    open_menu(&ctx, &mut a);
    click(&ctx, &mut a, "Add a Teams meeting");
    assert_eq!(creates(&jobs).len(), 1);
    assert!(form(&mut a).teams_wait.is_some());
    settled_text_at(&ctx, &mut a, "Creating the Teams meeting…", SIZE).expect("said in the window");
    assert_eq!(pressable(&ctx, &mut a, "Save & Close"), Some(false));
    let _ = std::fs::remove_dir_all(dir);
}

/// No title, or no date: the window says so, and nothing goes to Teams.
#[test]
fn no_title_says_so_and_sends_nothing() {
    let (mut a, dir, ctx, jobs) = appointment("teams-no-title", "  ");
    click(&ctx, &mut a, "Teams Meeting");
    assert_eq!(form(&mut a).error.as_deref(), Some("A meeting needs a title and a start"));
    assert!(form(&mut a).teams_wait.is_none());
    form(&mut a).error = None;
    form(&mut a).summary = "Budget".into();
    form(&mut a).date.clear();
    click(&ctx, &mut a, "Teams Meeting");
    assert_eq!(form(&mut a).error.as_deref(), Some("A meeting needs a title and a start"));
    assert!(creates(&jobs).is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

/// Teams switched off, or signed out: the click opens File › Options, scrolled to the Teams card.
#[test]
fn switched_off_or_signed_out_opens_options_at_teams() {
    let (mut a, dir, ctx, jobs) = appointment("teams-options", "Budget");
    for (on, signed_in) in [(false, true), (true, false)] {
        (a.session.prefs.teams.on, a.session.prefs.teams.signed_in) = (on, signed_in);
        a.ui.backstage = None;
        click(&ctx, &mut a, "Teams Meeting");
        assert!(a.ui.backstage.as_ref().is_some_and(|b| b.page == crate::backstage::Page::Options), "{on} {signed_in}");
        let card = settled_text_at(&ctx, &mut a, "Use Teams in Quatrelle", SIZE).expect("the Teams card");
        assert!(card.y > 0.0 && card.y < SIZE.y, "scrolled to it: {card:?}");
        assert!(!a.ui.teams_scroll.get(), "once");
        let f = form(&mut a);
        assert!(f.teams_wait.is_none() && f.error.is_none() && f.teams_prompt.is_none());
    }
    assert!(creates(&jobs).is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

/// Without the meetings' permission the window asks for it, and Authorize Teams meetings opens
/// Microsoft's sign-in for it; once it's given the words go (D17), whether the answer that asked
/// came before the worker's word that it was missing or after.
#[test]
fn without_the_permission_the_window_offers_to_authorize() {
    let (mut a, dir, ctx, jobs) = appointment("teams-consent", "Budget");
    a.session.prefs.teams.meetings = false;
    click(&ctx, &mut a, "Teams Meeting");
    assert!(creates(&jobs).is_empty());
    assert_eq!(form(&mut a).teams_prompt, Some(TeamsPrompt::Consent));
    settled_text_at(&ctx, &mut a, "Teams meetings need your permission", SIZE).expect("said");
    click(&ctx, &mut a, "Authorize Teams meetings");
    assert!(authorizing(&jobs), "the sign-in asks for meetings too");
    allowed(&mut a, true);
    assert!(settled_text_at(&ctx, &mut a, "Teams meetings need your permission", SIZE).is_none(), "gone once given");
    // Teams answers that the permission is missing before the worker's word is applied.
    click(&ctx, &mut a, "Teams Meeting");
    answer(&ctx, &mut a, None, Some(TeamsMeetingError::Consent));
    assert_eq!(form(&mut a).teams_prompt, Some(TeamsPrompt::Consent));
    allowed(&mut a, false);
    settled_text_at(&ctx, &mut a, "Teams meetings need your permission", SIZE).expect("said once it's known missing");
    allowed(&mut a, true);
    assert!(settled_text_at(&ctx, &mut a, "Teams meetings need your permission", SIZE).is_none());
    assert!(settled_text_at(&ctx, &mut a, "Authorize Teams meetings", SIZE).is_none());
    let _ = std::fs::remove_dir_all(dir);
}

/// T5: while one request is on its way the button and its menu are off, and another ask for the
/// window sends nothing.
#[test]
fn a_second_press_while_one_is_on_its_way_does_nothing() {
    let (mut a, dir, ctx, jobs) = appointment("teams-twice", "Budget");
    click(&ctx, &mut a, "Teams Meeting");
    let ask = form(&mut a).teams_wait.expect("on its way");
    assert_eq!(pressable(&ctx, &mut a, "Teams Meeting"), Some(false));
    assert_eq!(pressable(&ctx, &mut a, "Teams Meeting menu"), Some(false));
    let (w, p) = (a.ui.event_windows[0].0, add_params(form(&mut a)));
    a.act(crate::Action::ForTeamsMeeting(w, p));
    frame_sized(&ctx, &mut a, vec![], SIZE);
    assert_eq!(creates(&jobs), [ask], "one request");
    assert_eq!(form(&mut a).teams_wait, Some(ask));
    assert_eq!(a.session.teams_meetings.asks.len(), 1);
    let _ = std::fs::remove_dir_all(dir);
}

/// The answer puts the link as the place, the block after the notes, and Join opens the meeting.
#[test]
fn the_answer_fills_the_appointment_and_join_works() {
    let (mut a, dir, ctx, _) = appointment("teams-answer", "Budget");
    let opened = opening(&mut a);
    form(&mut a).description = "Agenda".into();
    made(&ctx, &mut a);
    let f = form(&mut a);
    assert_eq!((f.location.as_str(), f.join.as_deref(), f.teams_fresh), (LINK, Some(LINK), true));
    assert!(f.description.starts_with(&format!("Agenda\n\nJoin the Teams meeting: {LINK}\n")), "{:?}", f.description);
    assert!(f.error.is_none() && f.teams_wait.is_none());
    click(&ctx, &mut a, "Join meeting");
    assert_eq!(opened.borrow().as_slice(), [LINK]);
    let _ = std::fs::remove_dir_all(dir);
}

/// Save waits for the meeting on its way, and goes with it once it's in.
#[test]
fn save_waits_for_the_teams_answer() {
    let (mut a, dir, ctx, _) = appointment("teams-save-waits", "Budget");
    let w = a.ui.event_windows[0].0;
    click(&ctx, &mut a, "Teams Meeting");
    let p = form(&mut a).params();
    a.for_event(w, "event.create", p);
    assert_eq!(a.ui.event_windows.len(), 1, "nothing saved, the window stays");
    assert!(a.session.agenda(NOW, NOW + 86_400).iter().all(|i| i.summary != "Budget"));
    settled_text_at(&ctx, &mut a, "Creating the Teams meeting…", SIZE).expect("said in the window");
    assert_eq!(pressable(&ctx, &mut a, "Save & Close"), Some(false), "Save is off");
    answer(&ctx, &mut a, Some(meeting()), None);
    assert_eq!(pressable(&ctx, &mut a, "Save & Close"), Some(true), "Save is back");
    let p = form(&mut a).params();
    a.for_event(w, "event.create", p);
    assert!(a.ui.event_windows.is_empty(), "saved");
    let _ = std::fs::remove_dir_all(dir);
}

/// Saved, the appointment holds the id and the meeting is the appointment's: nothing deletes it.
#[test]
fn saving_sends_the_id_and_keeps_the_meeting() {
    let (mut a, dir, ctx, jobs) = appointment("teams-saved", "Budget");
    made(&ctx, &mut a);
    let w = a.ui.event_windows[0].0;
    let p = form(&mut a).params();
    assert_eq!(p["teamsMeetingId"], ID);
    a.for_event(w, "event.create", p);
    assert!(a.ui.event_windows.is_empty());
    frame_sized(&ctx, &mut a, vec![], SIZE);
    assert!(deletes(&jobs).is_empty());
    let saved = a.session.agenda(NOW, NOW + 86_400).into_iter().find(|i| i.summary == "Budget").expect("saved");
    assert_eq!(kept_id(&a, &saved.uid).as_deref(), Some(ID));
    let _ = std::fs::remove_dir_all(dir);
}

/// Each refusal is said in the window in its words, and the appointment stays as it was: the
/// permission and a new sign-in are asked for with their buttons, signed out opens Options.
#[test]
fn each_refusal_says_its_words() {
    use TeamsMeetingError::*;
    let (mut a, dir, ctx, jobs) = appointment("teams-refused", "Budget");
    (form(&mut a).location, form(&mut a).description) = ("Room 4".into(), "Agenda".into());
    let said = [
        (SignIn, "Sign in to Teams again"),
        (NotAllowed, "Your Teams account can't create meetings"),
        (Offline, "The Teams meeting couldn't be created: no connection"),
        (Untrusted, "Teams gave a link Quatrelle doesn't trust"),
        (Refused { status: 400, code: "BadRequest".into() }, "Teams answered 400: BadRequest"),
        (Refused { status: 503, code: String::new() }, "Teams answered 503"),
        (NotSignedIn, "Teams is signed out"),
        (Consent, "Teams meetings need your permission"),
    ];
    for (ask, (e, words)) in said.into_iter().enumerate() {
        assert_eq!(error_text(&e), words);
        let ask = ask as u64 + 1;
        a.ui.backstage = None;
        // The worker says the permission went along with its refusal.
        a.session.prefs.teams.meetings = e != Consent;
        a.session.teams_meetings.answers.insert(ask, Err(e.clone()));
        (form(&mut a).teams_wait, form(&mut a).error, form(&mut a).teams_prompt) = (Some(ask), None, None);
        frame_sized(&ctx, &mut a, vec![], SIZE);
        let f = form(&mut a);
        assert_eq!((f.location.as_str(), f.description.as_str(), f.join.as_deref()), ("Room 4", "Agenda", None), "{words}");
        assert!(f.teams_wait.is_none() && f.teams_meeting_id.is_none() && !f.teams_fresh, "{words}");
        match e {
            NotSignedIn => {
                assert!(f.error.is_none() && f.teams_prompt.is_none());
                assert!(a.ui.backstage.as_ref().is_some_and(|b| b.page == crate::backstage::Page::Options), "Options opens");
            }
            SignIn | Consent => {
                assert!(f.error.is_none());
                assert_eq!(f.teams_prompt, Some(if e == SignIn { TeamsPrompt::SignIn } else { TeamsPrompt::Consent }));
                settled_text_at(&ctx, &mut a, words, SIZE).unwrap_or_else(|| panic!("{words} said"));
            }
            _ => {
                assert_eq!(f.error.as_deref(), Some(words));
                settled_text_at(&ctx, &mut a, words, SIZE).unwrap_or_else(|| panic!("{words} said"));
            }
        }
        if e == SignIn {
            click(&ctx, &mut a, "Sign in");
            assert!(jobs.borrow().iter().any(|j| matches!(j, Job::Teams { job: teams::Job::SignIn { .. } })), "signing in again");
            assert_eq!(form(&mut a).teams_prompt, None);
            a.session.teams.status = teams::Status::UpToDate;
        }
    }
    assert!(a.session.teams_meetings.answers.is_empty(), "each taken once");
    assert!(creates(&jobs).is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

/// On a saved appointment Remove takes the meeting out of the window at once and sends nothing;
/// saved without it, the engine deletes it.
#[test]
fn remove_takes_it_out_at_once_and_deletes_on_save() {
    let (mut a, dir, ctx, jobs) = appointment("teams-remove-saved", "Budget");
    form(&mut a).description = "Agenda".into();
    made(&ctx, &mut a);
    let uid = saved_and_opened(&ctx, &mut a);
    let f = form(&mut a);
    assert_eq!((f.teams_meeting_id.as_deref(), f.teams_fresh), (Some(ID), false));
    open_menu(&ctx, &mut a);
    click(&ctx, &mut a, "Remove the Teams meeting");
    let f = form(&mut a);
    assert_eq!((f.description.as_str(), f.location.as_str(), f.join.as_deref(), f.teams_meeting_id.as_deref()), ("Agenda", "", None, None));
    assert!(deletes(&jobs).is_empty(), "nothing before the save");
    let w = a.ui.event_windows[0].0;
    let p = form(&mut a).params();
    assert_eq!(p["teamsMeetingId"], "");
    a.for_event(w, "event.update", p);
    frame_sized(&ctx, &mut a, vec![], SIZE);
    assert_eq!(deletes(&jobs), [ID]);
    assert_eq!(kept_id(&a, &uid), None);
    let _ = std::fs::remove_dir_all(dir);
}

/// Removed, then the window closed without saving: the appointment and its meeting stay as they
/// were.
#[test]
fn a_window_closed_after_remove_leaves_the_meeting() {
    let (mut a, dir, ctx, jobs) = appointment("teams-remove-unsaved", "Budget");
    made(&ctx, &mut a);
    let uid = saved_and_opened(&ctx, &mut a);
    open_menu(&ctx, &mut a);
    click(&ctx, &mut a, "Remove the Teams meeting");
    assert!(form(&mut a).teams_meeting_id.is_none());
    close_window(&ctx, &mut a);
    assert!(deletes(&jobs).is_empty());
    assert_eq!(kept_id(&a, &uid).as_deref(), Some(ID));
    let _ = std::fs::remove_dir_all(dir);
}

/// A meeting made in this window and never saved is deleted as soon as it's removed.
#[test]
fn removing_a_fresh_meeting_deletes_it_now() {
    let (mut a, dir, ctx, jobs) = appointment("teams-remove-fresh", "Budget");
    form(&mut a).description = "Agenda".into();
    made(&ctx, &mut a);
    open_menu(&ctx, &mut a);
    click(&ctx, &mut a, "Remove the Teams meeting");
    assert_eq!(deletes(&jobs), [ID]);
    let f = form(&mut a);
    assert_eq!((f.description.as_str(), f.location.as_str(), f.teams_meeting_id.as_deref(), f.teams_fresh), ("Agenda", "", None, false));
    assert_eq!(pressable(&ctx, &mut a, "Teams Meeting"), Some(true), "another can be added");
    let _ = std::fs::remove_dir_all(dir);
}

/// Meeting options opens the meeting's options page; it's off without an options line, or with a
/// link there that isn't Teams' own.
#[test]
fn meeting_options_opens_teams_page() {
    let (mut a, dir, ctx, _) = appointment("teams-options-page", "Budget");
    let opened = opening(&mut a);
    open_menu(&ctx, &mut a);
    assert_eq!(pressable(&ctx, &mut a, "Meeting options"), Some(false), "no meeting yet");
    assert_eq!(pressable(&ctx, &mut a, "Remove the Teams meeting"), Some(false));
    click(&ctx, &mut a, "Add a Teams meeting");
    answer(&ctx, &mut a, Some(meeting()), None);
    open_menu(&ctx, &mut a);
    click(&ctx, &mut a, "Meeting options");
    assert_eq!(opened.borrow().as_slice(), [OPTIONS]);
    form(&mut a).description = format!("Join the Teams meeting: {LINK}\n");
    open_menu(&ctx, &mut a);
    assert_eq!(pressable(&ctx, &mut a, "Remove the Teams meeting"), Some(true));
    assert_eq!(pressable(&ctx, &mut a, "Meeting options"), Some(false), "no options line");
    form(&mut a).description = format!("Join the Teams meeting: {LINK}\nMeeting options: https://teams.microsoft.com.evil.example/o\n");
    assert_eq!(pressable(&ctx, &mut a, "Meeting options"), Some(false), "a link that isn't Teams'");
    assert_eq!(opened.borrow().len(), 1);
    let _ = std::fs::remove_dir_all(dir);
}

/// A Teams meeting greys Webex's new meeting and rooms; a Webex meeting or a room's block greys
/// Add a Teams meeting; taking either out gives the other back. The reasons are said in every
/// language.
#[test]
fn teams_and_webex_grey_each_other() {
    let (mut a, dir, ctx, _) = appointment("teams-webex", "Budget");
    a.session.prefs.webex_rooms = vec![WebexRoom { name: "Ana".into(), url: ROOM.into(), details: Vec::new() }];
    made(&ctx, &mut a);
    assert_eq!(pressable(&ctx, &mut a, "Webex Meeting"), Some(false));
    open_webex_menu(&ctx, &mut a, SIZE);
    assert_eq!(pressable(&ctx, &mut a, "New Webex meeting"), Some(false));
    assert_eq!(pressable(&ctx, &mut a, "Ana"), Some(false), "the rooms too");
    open_menu(&ctx, &mut a);
    click(&ctx, &mut a, "Remove the Teams meeting");
    assert_eq!(pressable(&ctx, &mut a, "Webex Meeting"), Some(true), "Webex is back");
    form(&mut a).webex_id = Some("w1".into());
    assert_eq!(button_state(form(&mut a), TeamsMeetingPath::Standalone), TeamsButton::Off(WEBEX_FIRST));
    assert_eq!(pressable(&ctx, &mut a, "Teams Meeting"), Some(false));
    form(&mut a).webex_id = None;
    assert_eq!(pressable(&ctx, &mut a, "Teams Meeting"), Some(true));
    open_webex_menu(&ctx, &mut a, SIZE);
    click(&ctx, &mut a, "Ana");
    assert_eq!(form(&mut a).join.as_deref(), Some(ROOM), "the room is in");
    assert_eq!(button_state(form(&mut a), TeamsMeetingPath::Standalone), TeamsButton::Off(WEBEX_FIRST));
    assert_eq!(pressable(&ctx, &mut a, "Teams Meeting"), Some(false));
    (form(&mut a).description, form(&mut a).location) = ("Agenda".into(), String::new());
    assert_eq!(button_state(form(&mut a), TeamsMeetingPath::Standalone), TeamsButton::Add, "its block erased");
    assert_eq!(pressable(&ctx, &mut a, "Teams Meeting"), Some(true));
    for key in [WEBEX_FIRST, TEAMS_FIRST, NOT_YOURS, MOVED] {
        for lang in Lang::all().skip(1) {
            assert_ne!(tr(lang, key), key, "{}: {key}", lang.code());
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// Only an appointment of the user's takes a meeting: one they can't invite to, or one a meeting
/// of theirs couldn't be kept on (a colleague's with no organizer named), has the button off, and
/// says why.
#[test]
fn only_an_appointment_of_the_users_takes_one() {
    let (mut a, dir, ctx, jobs) = appointment("teams-not-yours", "Budget");
    assert_eq!(pressable(&ctx, &mut a, "Teams Meeting"), Some(true));
    form(&mut a).can_invite = false;
    assert_eq!(button_state(form(&mut a), TeamsMeetingPath::Standalone), TeamsButton::Off(NOT_YOURS));
    assert_eq!(pressable(&ctx, &mut a, "Teams Meeting"), Some(false));
    form(&mut a).can_invite = true;
    let uid = saved_and_opened(&ctx, &mut a);
    assert_eq!(pressable(&ctx, &mut a, "Teams Meeting"), Some(true), "a lone appointment of the user's");
    a.ui.event_windows.clear();
    let guest = quatrelle_engine::model::ical::Attendee { email: "zoe@studio.example".into(), ..Default::default() };
    let e = a.session.pim_mut("sample").events.iter_mut().find(|e| e.event.uid == uid).expect("saved");
    e.event.attendees = vec![guest];
    e.event.teams_meeting_id = Some("m0".into());
    let start = e.event.start;
    a.ui.dialog = Dialog::open_event(&a.session, &uid, start);
    frame_sized(&ctx, &mut a, vec![], SIZE);
    assert!(form(&mut a).can_invite && !form(&mut a).teams_allowed);
    assert_eq!(button_state(form(&mut a), TeamsMeetingPath::Standalone), TeamsButton::Off(NOT_YOURS), "its id isn't the user's");
    assert_eq!(pressable(&ctx, &mut a, "Teams Meeting"), Some(false));
    open_menu(&ctx, &mut a);
    assert_eq!(pressable(&ctx, &mut a, "Remove the Teams meeting"), Some(false));
    assert!(creates(&jobs).is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

/// D13: a Microsoft 365 mailbox's calendar is greyed for now (Exchange Online will make its own),
/// someone else's always; the account's calendar on this computer takes one.
#[test]
fn a_graph_mailbox_is_greyed_for_now() {
    let (mut a, dir, ctx, jobs) = appointment("teams-graph", "Budget");
    a.ui.event_windows.clear();
    served(&mut a, "sample", true);
    calendar(&mut a, "sample", "AAMkAGI2", false);
    calendar(&mut a, "sample", "AAMkBOSS", true);
    open_new(&ctx, &mut a);
    form(&mut a).summary = "Budget".into();
    assert_eq!(a.session.teams_meeting_path("sample", "AAMkAGI2"), TeamsMeetingPath::Mailbox);
    assert_eq!(button_state(form(&mut a), TeamsMeetingPath::Mailbox), TeamsButton::Off(""));
    for (id, path) in [("AAMkAGI2", TeamsMeetingPath::Mailbox), ("AAMkBOSS", TeamsMeetingPath::Unavailable)] {
        choose(&mut a, "sample", id);
        assert_eq!(a.session.teams_meeting_path("sample", id), path);
        assert_eq!(pressable(&ctx, &mut a, "Teams Meeting"), Some(false), "{id}");
    }
    assert_eq!(button_state(form(&mut a), TeamsMeetingPath::Unavailable), TeamsButton::Off(NOT_YOURS));
    choose(&mut a, "sample", LOCAL);
    assert_eq!(pressable(&ctx, &mut a, "Teams Meeting"), Some(true), "on this computer");
    assert!(creates(&jobs).is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

/// Closed unsaved, the window takes the meeting it made with it.
#[test]
fn closing_unsaved_deletes_the_meeting() {
    let (mut a, dir, ctx, jobs) = appointment("teams-close", "Budget");
    made(&ctx, &mut a);
    close_window(&ctx, &mut a);
    assert_eq!(deletes(&jobs), [ID]);
    let _ = std::fs::remove_dir_all(dir);
}

/// Closed while its meeting is on its way, the window gives it up: the engine deletes what it
/// brings.
#[test]
fn closing_during_the_creation_abandons_it() {
    let (mut a, dir, ctx, _) = appointment("teams-close-waiting", "Budget");
    click(&ctx, &mut a, "Teams Meeting");
    let ask = form(&mut a).teams_wait.expect("waiting");
    close_window(&ctx, &mut a);
    assert!(a.session.teams_meetings.asks.get(&ask).is_some_and(|x| x.abandoned));
    let _ = std::fs::remove_dir_all(dir);
}

/// Deleting the appointment from its window takes the meeting just made with it, though the
/// event never held its id; a meeting on its way is given up the same way.
#[test]
fn deleting_the_event_with_a_fresh_meeting_leaves_nothing() {
    let (mut a, dir, ctx, jobs) = appointment("teams-delete", "Budget");
    let uid = saved_and_opened(&ctx, &mut a);
    made(&ctx, &mut a);
    let w = a.ui.event_windows[0].0;
    a.for_event(w, "event.delete", json!({"uid": uid}));
    assert!(a.ui.event_windows.is_empty());
    frame_sized(&ctx, &mut a, vec![], SIZE);
    assert_eq!(deletes(&jobs), [ID]);

    open_new(&ctx, &mut a);
    form(&mut a).summary = "Budget".into();
    let uid = saved_and_opened(&ctx, &mut a);
    click(&ctx, &mut a, "Teams Meeting");
    let (w, ask) = (a.ui.event_windows[0].0, form(&mut a).teams_wait.expect("waiting"));
    a.for_event(w, "event.delete", json!({"uid": uid}));
    assert!(a.ui.event_windows.is_empty());
    assert!(a.session.teams_meetings.asks.get(&ask).is_some_and(|x| x.abandoned));
    let _ = std::fs::remove_dir_all(dir);
}

/// Quitting deletes the fresh meeting of an open window, once however often it's asked, and stops
/// waiting as soon as Teams answers, deleted or refused; the window is left to Webex's turn.
#[test]
fn quitting_deletes_a_fresh_meeting_once() {
    for (n, error) in [None, Some(TeamsMeetingError::NotAllowed)].into_iter().enumerate() {
        let (mut a, dir, ctx, _) = appointment(&format!("teams-quit-{n}"), "Budget");
        made(&ctx, &mut a);
        let (jobs, _) = backend(&mut a, error);
        assert!(a.teams_meetings_closing(), "the delete is on its way");
        assert_eq!(deletes(&jobs), [ID]);
        assert_eq!(a.ui.event_windows.len(), 1, "read, not taken");
        assert!(!a.teams_meetings_closing(), "answered");
        assert!(!a.teams_meetings_closing());
        assert_eq!(deletes(&jobs), [ID], "once");
        assert_eq!(a.session.teams_meetings.deleting, 0);
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Quitting during a creation gives it up, and waits for what it brings: a meeting is deleted.
#[test]
fn quitting_during_a_creation_deletes_what_it_brings() {
    let (mut a, dir, ctx, _) = appointment("teams-quit-waiting", "Budget");
    click(&ctx, &mut a, "Teams Meeting");
    let ask = form(&mut a).teams_wait.expect("waiting");
    let (jobs, _) = backend(&mut a, None);
    assert!(a.teams_meetings_closing(), "its answer is awaited");
    assert!(deletes(&jobs).is_empty());
    a.session.apply(MailEvent::Teams { event: teams::Event::MeetingCreated { ask, meeting: Some(meeting()), error: None } });
    assert!(a.teams_meetings_closing(), "what it brought is being deleted");
    assert_eq!(deletes(&jobs), [ID]);
    assert!(!a.teams_meetings_closing());
    let _ = std::fs::remove_dir_all(dir);
}

/// Signed out, the delete can't go: quitting waits for nothing, not a look at the workers.
#[test]
fn quitting_signed_out_waits_for_nothing() {
    let (mut a, dir, ctx, _) = appointment("teams-quit-signed-out", "Budget");
    made(&ctx, &mut a);
    let (jobs, polls) = backend(&mut a, None);
    a.session.prefs.teams.signed_in = false;
    assert!(!a.teams_meetings_closing());
    assert_eq!((deletes(&jobs).len(), polls.get()), (0, 0));
    let _ = std::fs::remove_dir_all(dir);
}

/// No window with a meeting to leave and nothing on its way: quitting adds no wait.
#[test]
fn quitting_with_nothing_on_its_way_adds_no_wait() {
    let (mut a, dir, _ctx, _) = appointment("teams-quit-nothing", "Budget");
    let (jobs, polls) = backend(&mut a, None);
    assert!(!a.teams_meetings_closing());
    assert_eq!((jobs.borrow().len(), polls.get()), (0, 0));
    let _ = std::fs::remove_dir_all(dir);
}

/// One window with a fresh Webex meeting, another with a fresh Teams meeting: closing as
/// `main.rs` does (Teams first, both each turn) deletes each once.
#[test]
fn quitting_with_webex_and_teams_windows_deletes_each_once() {
    let (mut a, dir, ctx, _) = appointment("teams-webex-quit", "Budget");
    (a.session.prefs.webex_signed_in, a.session.prefs.webex_site) = (true, "acme.webex.com".into());
    let webex = WebexMeeting {
        id: "w1".into(),
        web_link: "https://acme.webex.com/acme/j.php?MTID=m1".into(),
        meeting_number: String::new(),
        password: quatrelle_engine::model::job::Secret(String::new()),
        sip_address: String::new(),
    };
    crate::pim_dialogs::webex::take_meeting(form(&mut a), &webex, &[]);
    open_new(&ctx, &mut a);
    let (w, f) = a.ui.event_windows.last_mut().expect("the second window");
    f.summary = "Budget".into();
    let (w, p) = (*w, add_params(f));
    a.act(crate::Action::ForTeamsMeeting(w, p));
    let ask = a.ui.event_windows.last().and_then(|(_, f)| f.teams_wait).expect("on its way");
    a.session.apply(MailEvent::Teams { event: teams::Event::MeetingCreated { ask, meeting: Some(meeting()), error: None } });
    frame_sized(&ctx, &mut a, vec![], SIZE);
    assert_eq!(a.ui.event_windows.last().and_then(|(_, f)| f.teams_meeting_id.as_deref()), Some(ID));
    let (jobs, _) = backend(&mut a, None);
    for _ in 0..3 {
        let _ = a.teams_meetings_closing() | a.webex_closing();
    }
    assert_eq!(deletes(&jobs), [ID]);
    let webex_deletes: Vec<String> = jobs
        .borrow()
        .iter()
        .filter_map(|j| match j {
            Job::WebexDelete { id, .. } => Some(id.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(webex_deletes, ["w1"]);
    let _ = std::fs::remove_dir_all(dir);
}

/// A3: an appointment saved already, opened again, asks with its own account and calendar, which
/// its save doesn't name.
#[test]
fn a_saved_appointment_asks_with_its_calendar() {
    let (mut a, dir, ctx, jobs) = appointment("teams-saved-asks", "Budget");
    let uid = saved_and_opened(&ctx, &mut a);
    let stored = a.session.find_event(&uid).map(|(account, e)| (account.to_string(), e.calendar.clone())).expect("stored");
    assert!(form(&mut a).params().get("account").is_none(), "a save names only its uid");
    let p = add_params(form(&mut a));
    assert_eq!((p["add"]["account"].as_str(), p["add"]["calendar"].as_str()), (Some(stored.0.as_str()), Some(stored.1.as_str())));
    assert_eq!(p["add"]["summary"], "Budget");
    click(&ctx, &mut a, "Teams Meeting");
    assert_eq!(creates(&jobs).len(), 1, "the engine took it");
    let _ = std::fs::remove_dir_all(dir);
}

/// Review Focus 3: a meeting made on an Exchange calendar, then a Microsoft 365 mailbox's
/// calendar chosen: the meeting is taken out of the window and deleted, and the window says to
/// add one again.
#[test]
fn changing_the_calendar_takes_the_meeting_out() {
    let (mut a, dir, ctx, jobs) = appointment("teams-calendar-changed", "Budget");
    a.ui.event_windows.clear();
    served(&mut a, "sample", false);
    calendar(&mut a, "sample", "AAMkEWS", false);
    let added = a.session.run("account.add", &json!({"email": "zoe@contoso.example", "password": "pw"})).expect("added");
    let zoe = added["account"].as_str().expect("its id").to_string();
    served(&mut a, &zoe, true);
    calendar(&mut a, &zoe, "AAMkGRAPH", false);
    open_new(&ctx, &mut a);
    form(&mut a).summary = "Budget".into();
    form(&mut a).description = "Agenda".into();
    choose(&mut a, "sample", "AAMkEWS");
    made(&ctx, &mut a);
    assert_eq!(form(&mut a).location, LINK);
    choose(&mut a, &zoe, "AAMkGRAPH");
    frame_sized(&ctx, &mut a, vec![], SIZE);
    frame_sized(&ctx, &mut a, vec![], SIZE);
    let f = form(&mut a);
    assert_eq!((f.description.as_str(), f.location.as_str(), f.join.as_deref(), f.teams_meeting_id.as_deref()), ("Agenda", "", None, None));
    assert_eq!(f.error.as_deref(), Some(MOVED));
    assert_eq!(deletes(&jobs), [ID]);
    settled_text_at(&ctx, &mut a, MOVED, SIZE).expect("said in the window");
    let _ = std::fs::remove_dir_all(dir);
}

/// File › Options › Teams: signed in, a Teams meetings line offers to authorize them until they
/// are, then says they are; nothing while Teams is signed out.
#[test]
fn the_teams_card_offers_to_authorize_meetings() {
    let (mut a, dir) = app("teams-card-meetings");
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 5000.0);
    let (jobs, _) = backend(&mut a, None);
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
    settled_text_at(&ctx, &mut a, "Use Teams in Quatrelle", size).expect("the card");
    assert!(settled_text_at(&ctx, &mut a, "Teams meetings", size).is_none(), "signed out: nothing");
    signed_in(&mut a, false);
    settled_text_at(&ctx, &mut a, "Teams meetings", size).expect("the line");
    assert!(settled_text_at(&ctx, &mut a, "Teams meetings are allowed", size).is_none());
    let at = settled_text_at(&ctx, &mut a, "Authorize Teams meetings", size).expect("its button");
    click_at(&ctx, &mut a, at, 1, size);
    assert!(authorizing(&jobs));
    allowed(&mut a, true);
    settled_text_at(&ctx, &mut a, "Teams meetings are allowed", size).expect("said");
    assert!(settled_text_at(&ctx, &mut a, "Authorize Teams meetings", size).is_none());
    let _ = std::fs::remove_dir_all(dir);
}
