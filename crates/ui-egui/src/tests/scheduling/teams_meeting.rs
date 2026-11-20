// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: a Teams meeting in the appointment window. Its block in the notes, written in the
//! interface's language and found again in any; the place, Join and the conference it brings;
//! what removing it takes out, and what the window leaves to delete. The helpers here serve
//! `teams_button.rs` and `teams_option.rs` too.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use quatrelle_engine::model::Job;
use quatrelle_engine::model::ical::Conference;
use quatrelle_engine::model::job::Secret;
use quatrelle_engine::model::pim::teams::{self, TEAMS_CONFERENCE_LABEL, TeamsMeeting, TeamsMeetingError};
use serde_json::Value;

use super::*;
use crate::i18n::{Lang, in_every_language, set_current};
use crate::pim_dialogs::EventForm;
use crate::pim_dialogs::teams_meeting::{BLOCK_LINES, block_lines, find_block, leaving, options_link, remove, take_meeting, without_block};

/// The meeting's links: to join it, and to its options page.
pub(super) const LINK: &str = "https://teams.microsoft.com/l/meetup-join/19%3ameeting_a%40thread.v2/0";
pub(super) const OPTIONS: &str = "https://teams.microsoft.com/meetingOptions/?organizerId=o1&threadId=19_meeting_a";
/// Graph's id of the meeting.
pub(super) const ID: &str = "m1";
/// Another meeting's link to join it.
const OTHER: &str = "https://teams.microsoft.com/l/meetup-join/19%3ameeting_b%40thread.v2/0";

/// The meeting `ID` as Teams makes it, with every value.
pub(super) fn meeting() -> TeamsMeeting {
    TeamsMeeting {
        id: ID.into(),
        join_url: LINK.into(),
        meeting_id: "1234567890".into(),
        passcode: Secret("aB3xY9".into()),
        toll_number: "+1 323-555-0166".into(),
        conference_id: "9999999".into(),
        options_url: OPTIONS.into(),
    }
}

/// Another meeting, `m2`, with its own link and no options page.
fn second() -> TeamsMeeting {
    TeamsMeeting { id: "m2".into(), join_url: OTHER.into(), options_url: String::new(), ..meeting() }
}

/// A new appointment's form with `notes` and `place`, where a Teams meeting is the user's to keep.
pub(super) fn form(notes: &str, place: &str) -> EventForm {
    EventForm { teams_allowed: true, description: notes.into(), location: place.into(), ..Default::default() }
}

/// The size of the window the tests of its button draw.
pub(super) const SIZE: egui::Vec2 = egui::vec2(1280.0, 900.0);

pub(super) type Jobs = Rc<RefCell<Vec<Job>>>;

/// A backend that keeps the jobs it is given, counts its polls, and answers each meeting's delete
/// (Teams', with `error`, and Webex's) as the workers do.
struct Answering {
    jobs: Jobs,
    polls: Rc<Cell<usize>>,
    error: Option<TeamsMeetingError>,
    answers: Vec<MailEvent>,
}

impl crate::Backend for Answering {
    fn submit(&mut self, job: Job) {
        match &job {
            Job::Teams { job: teams::Job::MeetingDelete { id } } => {
                self.answers.push(MailEvent::Teams { event: teams::Event::MeetingDeleted { id: id.clone(), error: self.error.clone() } })
            }
            Job::WebexDelete { id, .. } => self.answers.push(MailEvent::WebexDeleted { id: id.clone(), error: None }),
            _ => {}
        }
        self.jobs.borrow_mut().push(job);
    }
    fn poll(&mut self) -> Vec<MailEvent> {
        self.polls.set(self.polls.get() + 1);
        std::mem::take(&mut self.answers)
    }
}

/// The app's jobs from now on, and its polls.
pub(super) fn backend(a: &mut MailApp, error: Option<TeamsMeetingError>) -> (Jobs, Rc<Cell<usize>>) {
    let (jobs, polls) = (Rc::new(RefCell::new(Vec::new())), Rc::new(Cell::new(0)));
    a.backend = Box::new(Answering { jobs: jobs.clone(), polls: polls.clone(), error, answers: Vec::new() });
    (jobs, polls)
}

/// Teams on and signed in (an organisation's application), its meetings allowed or not.
pub(super) fn signed_in(a: &mut MailApp, meetings: bool) {
    let t = &mut a.session.prefs.teams;
    (t.on, t.signed_in, t.meetings) = (true, true, meetings);
    t.client_id = "00000000-0000-0000-0000-00000000c11e".into();
    t.tenant = "contoso.example".into();
}

/// A new appointment in a window of its own.
pub(super) fn open_new(ctx: &egui::Context, a: &mut MailApp) {
    a.ui.dialog = Some(Dialog::new_event(&a.session, Some(NOW + 2 * 3600)));
    frame_sized(ctx, a, vec![], SIZE);
}

/// An appointment called `title` in its window, in a session signed in to Teams with its meetings
/// allowed; the jobs kept.
pub(super) fn appointment(name: &str, title: &str) -> (MailApp, std::path::PathBuf, egui::Context, Jobs) {
    let (mut a, dir) = app(name);
    let ctx = egui::Context::default();
    let (jobs, _) = backend(&mut a, None);
    signed_in(&mut a, true);
    frame_sized(&ctx, &mut a, vec![], SIZE);
    open_new(&ctx, &mut a);
    a.ui.event_windows[0].1.summary = title.into();
    (a, dir, ctx, jobs)
}

/// The Teams meetings asked for (their asks), and those asked to be deleted (their ids).
pub(super) fn creates(jobs: &Jobs) -> Vec<u64> {
    let created = |j: &Job| match j {
        Job::Teams { job: teams::Job::MeetingCreate { ask, .. } } => Some(*ask),
        _ => None,
    };
    jobs.borrow().iter().filter_map(created).collect()
}

pub(super) fn deletes(jobs: &Jobs) -> Vec<String> {
    let deleted = |j: &Job| match j {
        Job::Teams { job: teams::Job::MeetingDelete { id } } => Some(id.clone()),
        _ => None,
    };
    jobs.borrow().iter().filter_map(deleted).collect()
}

/// A click on the text `label`.
pub(super) fn click(ctx: &egui::Context, a: &mut MailApp, label: &str) {
    let at = settled_text_at(ctx, a, label, SIZE).unwrap_or_else(|| panic!("{label} isn't drawn"));
    click_at(ctx, a, at, 1, SIZE);
}

/// What the worker says of the meetings' permission.
pub(super) fn allowed(a: &mut MailApp, allowed: bool) {
    a.session.apply(MailEvent::Teams { event: teams::Event::Meetings { allowed } });
}

/// `m`'s block as the notes end with it.
fn block_of(m: &TeamsMeeting) -> String {
    format!("{}\n", block_lines(m).join("\n"))
}

/// A line for each of Teams' values, as it gave them, the ID and the passcode side by side.
#[test]
fn the_block_says_each_value_as_teams_gave_it() {
    let lines = block_lines(&meeting());
    assert_eq!(
        lines,
        [
            format!("Join the Teams meeting: {LINK}"),
            "Meeting ID: 1234567890   Passcode: aB3xY9".into(),
            "By phone: +1 323-555-0166, conference ID 9999999".into(),
            format!("Meeting options: {OPTIONS}"),
        ]
    );
    assert_eq!(lines.len(), BLOCK_LINES);
}

/// A line whose values Teams didn't give is left out; one of its two values alone keeps it.
#[test]
fn lines_without_a_value_are_left_out() {
    let join = format!("Join the Teams meeting: {LINK}");
    let options = format!("Meeting options: {OPTIONS}");
    let m = TeamsMeeting { toll_number: String::new(), conference_id: String::new(), ..meeting() };
    assert_eq!(block_lines(&m), [join.clone(), "Meeting ID: 1234567890   Passcode: aB3xY9".into(), options.clone()], "no audio: no phone line");
    let m = TeamsMeeting { conference_id: String::new(), ..meeting() };
    assert_eq!(block_lines(&m).get(2).map(String::as_str), Some("By phone: +1 323-555-0166"), "a number without a conference ID");
    let m = TeamsMeeting { toll_number: String::new(), ..meeting() };
    assert!(!block_lines(&m).iter().any(|l| l.starts_with("By phone")), "a conference ID without a number: nothing to call");
    let m = TeamsMeeting { passcode: Secret(String::new()), ..meeting() };
    assert_eq!(block_lines(&m).get(1).map(String::as_str), Some("Meeting ID: 1234567890"));
    let m = TeamsMeeting { meeting_id: String::new(), ..meeting() };
    assert_eq!(block_lines(&m).get(1).map(String::as_str), Some("Passcode: aB3xY9"));
    let m = TeamsMeeting { options_url: String::new(), ..meeting() };
    assert_eq!(block_lines(&m).last().map(String::as_str), Some("By phone: +1 323-555-0166, conference ID 9999999"), "no options line");
    let bare = TeamsMeeting { id: ID.into(), join_url: LINK.into(), meeting_id: String::new(), passcode: Secret(String::new()), ..m };
    let bare = TeamsMeeting { toll_number: String::new(), conference_id: String::new(), ..bare };
    assert_eq!(block_lines(&bare), [join]);
}

/// Teams' answer in the appointment: the block after the notes, the link as an empty place (one
/// typed stays), Join, the conference, and the id saved with it.
#[test]
fn the_answer_goes_into_the_appointment() {
    let mut f = form("Agenda", "");
    take_meeting(&mut f, &meeting());
    assert_eq!(f.description, format!("Agenda\n\n{}", block_of(&meeting())));
    assert_eq!(f.location, LINK, "an empty place takes the link");
    assert_eq!(f.join.as_deref(), Some(LINK));
    assert!(f.conference.is_some() && f.conference_shown);
    assert_eq!(f.conference, Conference::new(LINK, TEAMS_CONFERENCE_LABEL));
    assert_eq!((f.teams_meeting_id.as_deref(), f.teams_fresh), (Some(ID), true));
    let p = f.params();
    assert_eq!(p["teamsMeetingId"], ID);
    assert_eq!(p["conference"], json!({"url": LINK, "label": "Microsoft Teams"}));
    let mut f = form("", "Room 4");
    take_meeting(&mut f, &meeting());
    assert_eq!(f.location, "Room 4", "a place typed stays");
    assert_eq!(f.description, block_of(&meeting()), "no notes: the block alone");
    let f = EventForm { teams_allowed: false, ..f };
    assert!(f.params().get("teamsMeetingId").is_none(), "not the user's to keep: the engine keeps what the event has");
    assert_eq!(form("", "").params()["teamsMeetingId"], "", "none: the engine takes any out");
}

/// Another meeting's answer takes the first one's place in the notes, the place, Join and the
/// conference; a place typed stays through both.
#[test]
fn a_second_meeting_replaces_the_block_and_its_place() {
    let mut f = form("Agenda", "");
    take_meeting(&mut f, &meeting());
    take_meeting(&mut f, &second());
    assert_eq!(f.description, format!("Agenda\n\n{}", block_of(&second())));
    assert_eq!(f.description.matches("Join the Teams meeting:").count(), 1, "one block");
    assert_eq!(f.location, OTHER, "it was the first meeting's link");
    assert_eq!((f.join.as_deref(), f.teams_meeting_id.as_deref()), (Some(OTHER), Some("m2")));
    assert_eq!(f.conference.as_ref().map(|c| c.url.as_str()), Some(OTHER));
    assert_eq!(options_link(&f.description), None, "the second has no options page");
    let mut f = form("Agenda", "Room 4");
    take_meeting(&mut f, &meeting());
    take_meeting(&mut f, &second());
    assert_eq!(f.location, "Room 4");
}

/// A block written in French, then replaced by one in Japanese, is found and taken out with the
/// interface in English; what the user wrote stays.
#[test]
fn the_block_is_found_whatever_language_wrote_it() {
    let mut f = form("Agenda", "");
    set_current(Lang::from_code("fr").expect("French"));
    take_meeting(&mut f, &meeting());
    let french = f.description.clone();
    set_current(Lang::from_code("ja").expect("Japanese"));
    take_meeting(&mut f, &TeamsMeeting { options_url: OPTIONS.into(), ..second() });
    let japanese = f.description.clone();
    set_current(Lang::EN);
    assert!(french.contains("Rejoindre la réunion Teams : ") && french.contains("Code secret : aB3xY9"), "{french:?}");
    assert!(!japanese.contains("Rejoindre") && japanese.matches(OTHER).count() == 1, "the French block replaced: {japanese:?}");
    assert_eq!(f.location, OTHER, "the place was the French block's link");
    for (notes, link) in [(&french, LINK), (&japanese, OTHER)] {
        let b = find_block(notes).expect("found in English");
        assert_eq!((b.at, b.lines, b.join.as_str(), b.options.as_deref()), (2, BLOCK_LINES, link, Some(OPTIONS)), "{notes:?}");
        assert_eq!(options_link(notes).as_deref(), Some(OPTIONS));
        assert_eq!(without_block(notes), "Agenda");
    }
    assert_eq!(remove(&mut f), Some(json!({"remove": {"id": "m2"}})));
    assert_eq!((f.description.as_str(), f.location.as_str()), ("Agenda", ""));
}

/// D10: every catalog starts each of the block's lines with words of its own, before the value,
/// as that's how the block is found again; the join line's are never the options line's.
#[test]
fn the_blocks_labels_lead_in_every_language() {
    const KEYS: [&str; 6] = [
        "Join the Teams meeting: {link}",
        "Meeting ID: {id}",
        "Passcode: {passcode}",
        "By phone: {number}, conference ID {conference}",
        "By phone: {number}",
        "Meeting options: {link}",
    ];
    let label = |t: &str| t.split('{').next().unwrap_or_default().trim_end().to_string();
    for lang in Lang::all() {
        for key in KEYS {
            let t = crate::i18n::tr(lang, key);
            assert!(!label(t).trim().is_empty(), "{}: {t:?} has no words before its value", lang.code());
        }
        let (join, options) = (label(crate::i18n::tr(lang, KEYS[0])), label(crate::i18n::tr(lang, KEYS[5])));
        assert_ne!(join, options, "{}", lang.code());
    }
    let join: Vec<String> = in_every_language(KEYS[0]).iter().map(|t| label(t)).collect();
    let options: Vec<String> = in_every_language(KEYS[5]).iter().map(|t| label(t)).collect();
    assert!(join.len() > 1 && join.iter().all(|j| !options.contains(j)), "{join:?} {options:?}");
}

/// Removed, a meeting made in this window is deleted at once; the saved appointment's is deleted
/// by the engine when the appointment is saved without it. Either way the block, the place that
/// was its link, Join, the conference and the id go.
#[test]
fn removing_takes_the_block_the_place_and_the_id() {
    let mut f = form("Agenda", "");
    take_meeting(&mut f, &meeting());
    assert_eq!(remove(&mut f), Some(json!({"remove": {"id": ID}})), "made here: deleted now");
    assert_eq!((f.description.as_str(), f.location.as_str()), ("Agenda", ""));
    assert_eq!((f.join.as_deref(), f.conference.as_ref(), f.teams_meeting_id.as_deref()), (None, None, None));
    assert!(f.teams_removed && !f.teams_fresh);
    assert_eq!(leaving(&f), None, "nothing left to delete");
    assert_eq!(f.params()["teamsMeetingId"], "");
    assert!(f.params().get("conference").is_none(), "none was saved");

    let (mut a, dir) = app("teams-meeting-remove");
    let mut f = EventForm::new(&a.session, Some(NOW + 2 * 3600));
    f.summary = "Budget".into();
    f.description = "Agenda".into();
    take_meeting(&mut f, &meeting());
    let made = a.session.run("event.create", &f.params()).expect("made");
    let uid = made["uid"].as_str().unwrap_or_default().to_string();
    let start = a.session.find_event(&uid).map(|(_, e)| e.event.start).expect("stored");
    let mut f = EventForm::open(&a.session, &uid, start).expect("opened");
    assert!(f.teams_allowed);
    assert_eq!((f.teams_meeting_id.as_deref(), f.teams_fresh), (Some(ID), false));
    assert_eq!(f.params()["teamsMeetingId"], ID, "saved again, the meeting stays");
    assert_eq!(remove(&mut f), None, "the saved appointment's: the engine deletes it on save");
    assert!(f.teams_removed);
    assert_eq!((f.description.as_str(), f.location.as_str(), f.join.as_deref()), ("Agenda", "", None));
    let p = f.params();
    assert_eq!(p["teamsMeetingId"], "");
    assert_eq!(p.get("conference"), Some(&Value::Null), "the engine takes the conference out");
    a.session.run("event.update", &p).expect("saved");
    let e = a.session.find_event(&uid).map(|(_, e)| e.event.clone()).expect("stored");
    assert_eq!((e.teams_meeting_id, e.conference, e.location.as_str(), e.description.as_str()), (None, None, "", "Agenda"));
    let _ = std::fs::remove_dir_all(dir);
}

/// A place typed by hand isn't the meeting's: it stays when the meeting goes.
#[test]
fn a_place_typed_by_hand_stays_when_removing() {
    let mut f = form("Agenda", "Room 4");
    take_meeting(&mut f, &meeting());
    assert_eq!(f.location, "Room 4");
    remove(&mut f);
    assert_eq!((f.location.as_str(), f.description.as_str()), ("Room 4", "Agenda"));
    assert_eq!(f.params()["teamsMeetingId"], "");
}

/// The conference another app gave the appointment, which the meeting's replaced, comes back
/// when the meeting goes: saved, the appointment keeps it as it was.
#[test]
fn removing_gives_back_the_conference_the_meeting_replaced() {
    let jitsi = Conference::new("https://jitsi.example.org/team", "Jitsi");
    let mut f = EventForm { conference: jitsi.clone(), conference_opened: jitsi.clone(), ..form("Agenda", "") };
    take_meeting(&mut f, &meeting());
    assert_eq!(f.conference, Conference::new(LINK, TEAMS_CONFERENCE_LABEL));
    remove(&mut f);
    assert_eq!(f.conference, jitsi);
    assert!(f.params().get("conference").is_none(), "unchanged: the engine keeps it");
}

/// Only what's still as Quatrelle wrote it is the block: with its link's line erased, the other
/// lines are the user's text (removing still takes the id and the place); a line typed between
/// two of its lines ends it there, and what follows stays.
#[test]
fn a_block_edited_by_hand_loses_only_what_is_recognised() {
    let lines = block_lines(&meeting());
    let mut f = form("Agenda", "");
    take_meeting(&mut f, &meeting());
    let rest = format!("Agenda\n\n{}\n", lines[1..].join("\n"));
    f.description = rest.clone();
    assert!(find_block(&rest).is_none());
    assert_eq!(options_link(&rest), None);
    assert_eq!(without_block(&rest), rest);
    assert_eq!(remove(&mut f), Some(json!({"remove": {"id": ID}})), "the id goes all the same");
    assert_eq!(f.description, rest, "the lines left are the user's");
    assert_eq!((f.location.as_str(), f.join.as_deref(), f.teams_meeting_id.as_deref()), ("", None, None), "the place was the meeting's link");
    assert!(f.conference.is_none());

    let typed = format!("Agenda\n\n{}\n{}\nBring the figures\n{}\n{}\n", lines[0], lines[1], lines[2], lines[3]);
    let b = find_block(&typed).expect("the block up to the line typed");
    assert_eq!((b.at, b.lines, b.join.as_str(), b.options), (2, 2, LINK, None));
    assert_eq!(without_block(&typed), format!("Agenda\n\nBring the figures\n{}\n{}", lines[2], lines[3]));
}

/// A link that isn't the block's first line as Quatrelle writes it, or isn't Teams' own, is no
/// block, nor are its options; an options link that isn't Teams' own isn't the block's.
#[test]
fn a_link_elsewhere_in_the_notes_is_no_block() {
    for notes in [
        "Agenda\n\nJoin the Teams meeting: https://teams.microsoft.com.evil.example/x\nMeeting options: https://teams.microsoft.com.evil.example/o\n"
            .to_string(),
        format!("Join the Teams meeting: {LINK} today\nMeeting ID: 1234567890\n"),
        format!("Join the Teams meeting:\n{LINK}\n"),
        "Join the Teams meeting: http://teams.microsoft.com/l/meetup-join/x\n".to_string(),
        format!("See {LINK}\nMeeting options: {OPTIONS}\n"),
        format!("Meeting options: {OPTIONS}\n"),
    ] {
        assert!(find_block(&notes).is_none(), "{notes:?}");
        assert_eq!(options_link(&notes), None, "{notes:?}");
        assert_eq!(without_block(&notes), notes);
    }
    let notes = format!("Join the Teams meeting: {LINK}\nMeeting options: https://evil.example/o\n");
    let b = find_block(&notes).expect("a block");
    assert_eq!((b.lines, b.options), (2, None));
}

/// What the window leaves to delete when it goes without saving: the meeting made in it, or the
/// one on its way; never the saved appointment's.
#[test]
fn what_the_window_leaves_unsaved() {
    let mut f = form("", "");
    assert_eq!(leaving(&f), None, "nothing asked");
    f.teams_wait = Some(7);
    assert_eq!(leaving(&f), Some(json!({"remove": {"ask": 7}})), "on its way");
    f.teams_wait = None;
    take_meeting(&mut f, &meeting());
    assert_eq!(leaving(&f), Some(json!({"remove": {"id": ID}})), "made here, not saved");
    let saved = EventForm { teams_meeting_id: Some(ID.into()), ..form("", "") };
    assert_eq!(leaving(&saved), None, "the saved appointment's stays");
}
