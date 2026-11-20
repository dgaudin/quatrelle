// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: a Teams meeting in a Microsoft 365 mailbox's calendar, which Exchange Online makes as
//! the appointment is saved. Add a Teams meeting marks the appointment, Teams signed in or not,
//! and the window says a meeting will be added; Remove the Teams meeting unmarks it until the
//! first save, and is greyed after, saying why; the mark greys Webex; another calendar takes it
//! out; Join opens the link the server gave. File › Options › Calendar's option marks it too, and
//! Send goes with the mark at once: there is nothing to wait for.

use quatrelle_engine::model::Job;
use quatrelle_engine::model::pim::teams;
use quatrelle_engine::{TeamsMeetingPath, WebexRoom};

use super::teams_button::{calendar, choose, form, open_menu, opening, pressable, saved_and_opened, served};
use super::teams_meeting::{Jobs, LINK, SIZE, appointment, click, open_new};
use super::teams_option::{ZOE, frame_at, now, press_at, saved, type_guest, with_the_option};
use super::webex::{button, open_webex_menu};
use super::*;
use crate::i18n::{Lang, tr};
use crate::pim_dialogs::TeamsPrompt;
use crate::pim_dialogs::teams_meeting::{TeamsButton, button_state, remove};

/// The mailbox's calendar, and a colleague's the account lists.
const MAILBOX: &str = "AAMkAGI2";
const THEIRS: &str = "AAMkBOSS";
/// What the window says of a marked appointment not saved yet, and why the mark stays once saved.
const WILL_ADD: &str = "A Teams meeting will be added when the appointment is saved";
const KEEPS: &str = "Microsoft 365 keeps an online meeting once it is made";
/// What the window says once a calendar that can't hold the meeting is chosen.
const MOVED: &str = "The Teams meeting was removed: add it again for this calendar";
/// Why Webex is greyed, and why Add a Teams meeting is.
const TEAMS_FIRST: &str = "This appointment has a Teams meeting: remove it first";
const WEBEX_FIRST: &str = "This appointment has a Webex meeting or room: remove it first";
const ROOM: &str = "https://acme.webex.com/meet/ana";

/// A new appointment called "Budget" in its window, on the sample account's Microsoft 365 mailbox's
/// calendar, in a session signed in to Teams; the jobs kept.
fn on_the_mailbox(name: &str) -> (MailApp, std::path::PathBuf, egui::Context, Jobs) {
    let (mut a, dir, ctx, jobs) = appointment(name, "Budget");
    a.ui.event_windows.clear();
    served(&mut a, "sample", true);
    calendar(&mut a, "sample", MAILBOX, false);
    calendar(&mut a, "sample", THEIRS, true);
    open_new(&ctx, &mut a);
    form(&mut a).summary = "Budget".into();
    choose(&mut a, "sample", MAILBOX);
    (a, dir, ctx, jobs)
}

/// What was asked of Teams for its meetings: creations, updates and deletions.
fn meeting_jobs(jobs: &Jobs) -> usize {
    let meeting = |j: &&Job| {
        matches!(j, Job::Teams { job: teams::Job::MeetingCreate { .. } | teams::Job::MeetingUpdate { .. } | teams::Job::MeetingDelete { .. } })
    };
    jobs.borrow().iter().filter(meeting).count()
}

/// Whether `said` shows once the pointer rests on the text `label`.
pub(super) fn hover_says(ctx: &egui::Context, a: &mut MailApp, label: &str, said: &str) -> bool {
    let at = settled_text_at(ctx, a, label, SIZE).unwrap_or_else(|| panic!("{label} isn't drawn"));
    frame_sized(ctx, a, vec![Event::PointerMoved(at)], SIZE);
    (0..60).any(|_| text_at(&frame_shapes(ctx, a, vec![], SIZE), said).is_some())
}

/// D15: on a Microsoft 365 mailbox's calendar, Add marks the appointment, Teams switched off and
/// signed out: the window says the meeting will be added, nothing is waited for, and the saved
/// appointment is marked. A colleague's calendar there still takes none.
#[test]
fn add_on_a_mailbox_says_it_will_be_added() {
    let (mut a, dir, ctx, jobs) = on_the_mailbox("teams-mailbox-add");
    let t = &mut a.session.prefs.teams;
    (t.on, t.signed_in, t.meetings) = (false, false, false);
    assert_eq!(a.session.teams_meeting_path("sample", MAILBOX), TeamsMeetingPath::Mailbox);
    assert_eq!(button_state(form(&mut a), TeamsMeetingPath::Mailbox), TeamsButton::Add);
    assert!(settled_text_at(&ctx, &mut a, WILL_ADD, SIZE).is_none(), "not before the click");
    click(&ctx, &mut a, "Teams Meeting");
    let f = form(&mut a);
    assert!(f.teams_online && !f.teams_online_saved, "marked");
    assert!(f.teams_wait.is_none() && f.teams_prompt.is_none() && f.error.is_none(), "{:?}", f.error);
    assert!(a.ui.backstage.is_none(), "Options don't open");
    settled_text_at(&ctx, &mut a, WILL_ADD, SIZE).expect("said under the ribbon");
    assert!(settled_text_at(&ctx, &mut a, "Creating the Teams meeting…", SIZE).is_none(), "nothing to wait for");
    assert_eq!(pressable(&ctx, &mut a, "Save & Close"), Some(true), "Save is on");
    assert_eq!(pressable(&ctx, &mut a, "Teams Meeting"), Some(false), "nothing more to add");
    assert_eq!(form(&mut a).params()["teamsOnline"], true);
    let w = a.ui.event_windows[0].0;
    let p = form(&mut a).params();
    a.for_event(w, "event.create", p);
    assert!(a.ui.event_windows.is_empty(), "saved");
    let e = saved(&a, "Budget").expect("saved");
    assert!(e.teams_online && e.teams_meeting_id.is_none());
    assert_eq!(meeting_jobs(&jobs), 0, "nothing asked of Teams");
    open_new(&ctx, &mut a);
    form(&mut a).summary = "Review".into();
    choose(&mut a, "sample", THEIRS);
    assert_eq!(pressable(&ctx, &mut a, "Teams Meeting"), Some(false), "someone else's calendar");
    let _ = std::fs::remove_dir_all(dir);
}

/// Before the first save, Remove takes the mark off: nothing to delete, the words gone, Add back.
#[test]
fn remove_before_the_first_save_unmarks() {
    let (mut a, dir, ctx, jobs) = on_the_mailbox("teams-mailbox-remove");
    click(&ctx, &mut a, "Teams Meeting");
    assert!(form(&mut a).teams_online);
    open_menu(&ctx, &mut a);
    assert_eq!(pressable(&ctx, &mut a, "Remove the Teams meeting"), Some(true));
    click(&ctx, &mut a, "Remove the Teams meeting");
    let f = form(&mut a);
    assert!(!f.teams_online && f.teams_removed, "unmarked by the user");
    assert!(f.params().get("teamsOnline").is_none());
    assert!(settled_text_at(&ctx, &mut a, WILL_ADD, SIZE).is_none(), "no longer said");
    assert_eq!(pressable(&ctx, &mut a, "Teams Meeting"), Some(true), "it can be added again");
    let w = a.ui.event_windows[0].0;
    let p = form(&mut a).params();
    a.for_event(w, "event.create", p);
    assert!(!saved(&a, "Budget").expect("saved").teams_online);
    assert_eq!(meeting_jobs(&jobs), 0, "nothing to delete");
    let _ = std::fs::remove_dir_all(dir);
}

/// Saved once, the mark stays: opened again, Remove the Teams meeting is greyed and says why, and
/// the words of the first save aren't said again.
#[test]
fn remove_after_is_greyed_with_its_reason() {
    let (mut a, dir, ctx, jobs) = on_the_mailbox("teams-mailbox-kept");
    click(&ctx, &mut a, "Teams Meeting");
    let uid = saved_and_opened(&ctx, &mut a);
    assert!(a.session.find_event(&uid).is_some_and(|(_, e)| e.event.teams_online), "saved marked");
    let f = form(&mut a);
    assert!(f.teams_online && f.teams_online_saved, "opened marked");
    assert_eq!(f.params()["teamsOnline"], true, "saved again with it");
    assert!(settled_text_at(&ctx, &mut a, WILL_ADD, SIZE).is_none(), "made already");
    assert_eq!(pressable(&ctx, &mut a, "Teams Meeting"), Some(false));
    open_menu(&ctx, &mut a);
    assert_eq!(pressable(&ctx, &mut a, "Add a Teams meeting"), Some(false));
    assert_eq!(pressable(&ctx, &mut a, "Remove the Teams meeting"), Some(false));
    assert!(hover_says(&ctx, &mut a, "Remove the Teams meeting", KEEPS), "its tooltip says why");
    assert_eq!(remove(form(&mut a)), None);
    assert!(form(&mut a).teams_online, "nor taken off another way");
    for key in [WILL_ADD, KEEPS] {
        for lang in Lang::all().skip(1) {
            assert_ne!(tr(lang, key), key, "{}: {key}", lang.code());
        }
    }
    assert_eq!(meeting_jobs(&jobs), 0);
    let _ = std::fs::remove_dir_all(dir);
}

/// The mark greys Webex's button, new meeting and rooms, as a Teams meeting does, and keeps a
/// Webex meeting from being asked; taken off, Webex is back. A Webex meeting greys Add here too.
#[test]
fn a_marked_event_greys_webex() {
    let (mut a, dir, ctx, jobs) = on_the_mailbox("teams-mailbox-webex");
    a.session.prefs.webex_rooms = vec![WebexRoom { name: "Ana".into(), url: ROOM.into(), details: Vec::new() }];
    (a.session.prefs.webex_signed_in, a.session.prefs.webex_site) = (true, "acme.webex.com".into());
    assert_eq!(pressable(&ctx, &mut a, "Webex Meeting"), Some(true), "before the mark");
    click(&ctx, &mut a, "Teams Meeting");
    assert!(form(&mut a).teams_online);
    assert_eq!(pressable(&ctx, &mut a, "Webex Meeting"), Some(false));
    assert!(hover_says(&ctx, &mut a, "Webex Meeting", TEAMS_FIRST), "its tooltip says why");
    open_webex_menu(&ctx, &mut a, SIZE);
    assert_eq!(pressable(&ctx, &mut a, "New Webex meeting"), Some(false));
    assert_eq!(pressable(&ctx, &mut a, "Ana"), Some(false), "the rooms too");
    let w = a.ui.event_windows[0].0;
    let webex = crate::pim_dialogs::webex::webex_params(form(&mut a));
    a.act(crate::Action::ForWebex(w, webex));
    frame_sized(&ctx, &mut a, vec![], SIZE);
    assert!(!jobs.borrow().iter().any(|j| matches!(j, Job::WebexCreate { .. })), "no Webex meeting asked beside it");
    open_menu(&ctx, &mut a);
    click(&ctx, &mut a, "Remove the Teams meeting");
    assert_eq!(pressable(&ctx, &mut a, "Webex Meeting"), Some(true), "Webex is back");
    form(&mut a).webex_id = Some("w1".into());
    assert_eq!(button_state(form(&mut a), TeamsMeetingPath::Mailbox), TeamsButton::Off(WEBEX_FIRST));
    assert_eq!(pressable(&ctx, &mut a, "Teams Meeting"), Some(false));
    let _ = std::fs::remove_dir_all(dir);
}

/// Marked on a Microsoft 365 mailbox's calendar, then an Exchange one chosen: the mark goes and the
/// window says to add the meeting again; the calendar took it out, not the user.
#[test]
fn changing_the_calendar_unmarks() {
    let (mut a, dir, ctx, jobs) = appointment("teams-mailbox-calendar", "Budget");
    a.ui.event_windows.clear();
    served(&mut a, "sample", false);
    calendar(&mut a, "sample", "AAMkEWS", false);
    let added = a.session.run("account.add", &json!({"email": "zoe@contoso.example", "password": "pw"})).expect("added");
    let zoe = added["account"].as_str().expect("its id").to_string();
    served(&mut a, &zoe, true);
    calendar(&mut a, &zoe, MAILBOX, false);
    open_new(&ctx, &mut a);
    form(&mut a).summary = "Budget".into();
    choose(&mut a, &zoe, MAILBOX);
    click(&ctx, &mut a, "Teams Meeting");
    assert!(form(&mut a).teams_online);
    choose(&mut a, "sample", "AAMkEWS");
    frame_sized(&ctx, &mut a, vec![], SIZE);
    let f = form(&mut a);
    assert!(!f.teams_online && !f.teams_removed, "taken off, not by the user");
    assert_eq!(f.error.as_deref(), Some(MOVED));
    assert!(f.params().get("teamsOnline").is_none());
    settled_text_at(&ctx, &mut a, MOVED, SIZE).expect("said in the window");
    assert!(settled_text_at(&ctx, &mut a, WILL_ADD, SIZE).is_none());
    choose(&mut a, &zoe, MAILBOX);
    frame_sized(&ctx, &mut a, vec![], SIZE);
    assert!(!form(&mut a).teams_online, "it doesn't come back by itself");
    assert_eq!(pressable(&ctx, &mut a, "Teams Meeting"), Some(true));
    assert_eq!(meeting_jobs(&jobs), 0);
    let _ = std::fs::remove_dir_all(dir);
}

/// Join opens the link the server gave the appointment, before any other meeting link in it.
#[test]
fn join_opens_the_servers_link() {
    let (mut a, dir, ctx, _) = on_the_mailbox("teams-mailbox-join");
    a.ui.event_windows.clear();
    let opened = opening(&mut a);
    let p = json!({"summary": "Budget", "start": "2026-10-21T15:00", "account": "sample", "calendar": MAILBOX, "teamsOnline": true,
                   "location": "https://contoso.zoom.us/j/123456789", "description": "Agenda"});
    let uid = a.session.run("event.create", &p).expect("made")["uid"].as_str().unwrap_or_default().to_string();
    let e = a.session.pim_mut("sample").events.iter_mut().find(|e| e.event.uid == uid).expect("stored");
    e.event.join_url = LINK.into();
    let start = e.event.start;
    a.ui.dialog = Dialog::open_event(&a.session, &uid, start);
    frame_sized(&ctx, &mut a, vec![], SIZE);
    assert_eq!(form(&mut a).join.as_deref(), Some(LINK));
    click(&ctx, &mut a, "Join meeting");
    assert_eq!(opened.borrow().as_slice(), [LINK]);
    let _ = std::fs::remove_dir_all(dir);
}

/// File › Options › Calendar's option on a Microsoft 365 mailbox's calendar, Teams signed out
/// (D15): the first guest marks the appointment after the quiet second; Send pressed at once goes
/// with the mark, nothing held.
#[test]
fn the_option_marks_it_and_send_goes_at_once() {
    let (mut a, dir, ctx, jobs) = with_the_option("teams-mailbox-option", "Budget");
    a.ui.event_windows.clear();
    served(&mut a, "sample", true);
    calendar(&mut a, "sample", MAILBOX, false);
    a.session.prefs.teams.signed_in = false;
    open_new(&ctx, &mut a);
    (form(&mut a).summary, form(&mut a).inviting) = ("Budget".into(), true);
    choose(&mut a, "sample", MAILBOX);
    let t = now(&ctx) + 1.0;
    type_guest(&ctx, &mut a, ZOE, t);
    frame_at(&ctx, &mut a, vec![], t + 0.5);
    assert!(!form(&mut a).teams_online, "not before the quiet second");
    frame_at(&ctx, &mut a, vec![], t + 1.1);
    let f = form(&mut a);
    assert!(f.teams_online && f.teams_auto_tried, "marked at the first guest");
    assert!(f.teams_wait.is_none() && f.teams_prompt.is_none() && f.teams_send_held.is_none());
    settled_text_at(&ctx, &mut a, WILL_ADD, SIZE).expect("said under the ribbon");

    a.ui.event_windows.clear();
    open_new(&ctx, &mut a);
    (form(&mut a).summary, form(&mut a).inviting) = ("Review".into(), true);
    choose(&mut a, "sample", MAILBOX);
    let (send, on) = button(&ctx, &mut a, "Send").expect("the meeting's Send");
    assert!(on);
    let t = now(&ctx) + 1.0;
    type_guest(&ctx, &mut a, ZOE, t);
    press_at(&ctx, &mut a, send, t + 0.3);
    assert!(a.ui.event_windows.is_empty(), "sent at once, the window closed");
    let e = saved(&a, "Review").expect("sent");
    assert!(e.teams_online, "with the mark");
    assert_eq!(e.attendees.iter().map(|x| x.email.as_str()).collect::<Vec<_>>(), [ZOE]);
    assert_eq!(meeting_jobs(&jobs), 0, "nothing asked of Teams");
    let _ = std::fs::remove_dir_all(dir);
}

/// T15: the option's "Teams meeting not added", said for a calendar Quatrelle asks Teams for
/// (Teams signed out), goes once a Microsoft 365 mailbox's calendar is chosen, where Exchange
/// Online makes the meeting without Teams; and the option marks the appointment there.
#[test]
fn not_added_elsewhere_then_the_mailbox_chosen_marks_it() {
    let (mut a, dir, ctx, jobs) = with_the_option("teams-mailbox-not-added", "Budget");
    a.ui.event_windows.clear();
    served(&mut a, "sample", true);
    calendar(&mut a, "sample", MAILBOX, false);
    a.session.prefs.teams.signed_in = false;
    open_new(&ctx, &mut a);
    (form(&mut a).summary, form(&mut a).inviting) = ("Budget".into(), true);
    choose(&mut a, "sample", quatrelle_engine::model::pim::LOCAL);
    let t = now(&ctx) + 1.0;
    type_guest(&ctx, &mut a, ZOE, t);
    frame_at(&ctx, &mut a, vec![], t + 1.1);
    assert_eq!(form(&mut a).teams_prompt, Some(TeamsPrompt::NotAdded), "said for this computer's calendar");
    choose(&mut a, "sample", MAILBOX);
    frame_at(&ctx, &mut a, vec![], t + 1.2);
    let f = form(&mut a);
    assert!(f.teams_prompt.is_none(), "not said where Teams isn't needed");
    assert!(f.teams_online, "marked there");
    assert!(settled_text_at(&ctx, &mut a, "Teams meeting not added: sign in to Teams", SIZE).is_none());
    assert_eq!(meeting_jobs(&jobs), 0, "nothing asked of Teams");
    let _ = std::fs::remove_dir_all(dir);
}
