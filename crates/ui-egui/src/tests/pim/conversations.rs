// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: Teams' frame. File › Options › Teams (its fields, the sign-in, the copy set aside,
//! the sign-in that won't be remembered), the Conversations entry in the rail and its badge, and
//! the Teams events taken a few a frame. With `QUATRELLE_FRAME_PNG` set to a folder, the card is
//! painted there, light and dark.

use std::cell::RefCell;
use std::rc::Rc;

use quatrelle_engine::model::Job;
use quatrelle_engine::model::pim::teams::{self, ChatKind, ChatRow, Picture, Status};

use super::super::list::follow_up::{Textures, frame_with_size};
use super::*;

/// The File page drawn whole.
const PAGE: (usize, usize) = (1280, 6000);

/// A backend that keeps the jobs it is given and hands out `events` at its next poll.
struct Teams {
    jobs: Rc<RefCell<Vec<Job>>>,
    events: Rc<RefCell<Vec<MailEvent>>>,
}

impl crate::Backend for Teams {
    fn submit(&mut self, job: Job) {
        self.jobs.borrow_mut().push(job);
    }
    fn poll(&mut self) -> Vec<MailEvent> {
        std::mem::take(&mut *self.events.borrow_mut())
    }
}

type Shared<T> = Rc<RefCell<Vec<T>>>;

/// The app over the sample mailbox with Teams on (signed in when `signed_in`), and the jobs it
/// sends and the events it will be given.
fn teams_app(name: &str, signed_in: bool) -> (MailApp, std::path::PathBuf, Shared<Job>, Shared<MailEvent>) {
    let (mut a, dir) = app(name);
    let (jobs, events) = (Rc::new(RefCell::new(Vec::new())), Rc::new(RefCell::new(Vec::new())));
    a.backend = Box::new(Teams { jobs: jobs.clone(), events: events.clone() });
    let t = &mut a.session.prefs.teams;
    t.on = true;
    t.client_id = "11111111-2222-3333-4444-555555555555".into();
    t.tenant = "contoso.example".into();
    if signed_in {
        t.signed_in = true;
        t.name = "Riley Moreno".into();
        t.address = "riley@contoso.example".into();
        t.object_id = "00000000-0000-0000-0000-00000000000a".into();
    }
    (a, dir, jobs, events)
}

fn row(id: &str, unread: u32) -> ChatRow {
    ChatRow {
        id: id.into(),
        name: format!("Chat {id}"),
        kind: ChatKind::Group,
        last_line: String::new(),
        last_external: false,
        at: Some(NOW),
        unread,
        hidden: false,
        left: false,
        unreadable: false,
        external: false,
        members: 3,
        person: None,
    }
}

fn chats(rows: Vec<ChatRow>) -> MailEvent {
    MailEvent::Teams { event: teams::Event::Chats { rows } }
}

/// The accessible labels of the frame's controls, with where they are.
fn labelled(ctx: &egui::Context, a: &mut MailApp) -> Vec<(String, f64)> {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
    nodes.iter().filter_map(|(_, n)| Some((n.label()?.to_string(), n.bounds().map_or(f64::MAX, |b| b.x0)))).collect()
}

/// The texts drawn, light and dark, once the page has settled (each painted as `name` when
/// pictures are asked for); the light ones.
fn drawn(ctx: &egui::Context, a: &mut MailApp, tex: &mut Textures, name: &str) -> Vec<String> {
    let pictures = std::env::var_os("QUATRELLE_FRAME_PNG").map(std::path::PathBuf::from);
    let mut light = Vec::new();
    for (dark, theme) in [(false, "light"), (true, "dark")] {
        a.ui.dark = Some(dark);
        for _ in 0..8 {
            frame_with_size(ctx, a, tex, vec![], None, PAGE);
        }
        let png = pictures.as_ref().map(|d| d.join(format!("teams-card-{name}-{theme}.png")));
        let (_, shapes) = frame_with_size(ctx, a, tex, vec![], png, PAGE);
        let texts: Vec<String> = shapes
            .iter()
            .filter_map(|s| match s {
                egui::Shape::Text(t) => Some(t.galley.text().to_string()),
                _ => None,
            })
            .collect();
        if !dark {
            light = texts;
        }
    }
    a.ui.dark = Some(false);
    light
}

fn has(texts: &[String], wanted: &str) -> bool {
    texts.iter().any(|t| t == wanted)
}

/// Mail, Calendar, People, Tasks and Notes always; Conversations only once Teams is on.
#[test]
fn the_rail_shows_conversations_only_when_teams_is_on() {
    let (mut a, dir, _, _) = teams_app("teams-rail", true);
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let in_rail = |seen: &[(String, f64)], label: &str| seen.iter().any(|(l, x)| l == label && *x < 48.0);
    a.session.prefs.teams.on = false;
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let seen = labelled(&ctx, &mut a);
    assert!(in_rail(&seen, "Mail") && in_rail(&seen, "Notes"), "{seen:?}");
    assert!(!in_rail(&seen, "Conversations"), "Teams is off: {seen:?}");
    a.session.prefs.teams.on = true;
    frame(&ctx, &mut a, vec![]);
    let seen = labelled(&ctx, &mut a);
    assert!(in_rail(&seen, "Conversations"), "Teams is on: {seen:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// The badge counts the unread messages of the listed chats; past 99 it says `99+`, and none
/// shows at 0.
#[test]
fn the_badge_says_99_plus() {
    let (mut a, dir, _, _) = teams_app("teams-badge", true);
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 800.0);
    let badge = |shapes: &[egui::Shape], text: &str| text_at(shapes, text).is_some_and(|at| at.x < 48.0);
    frame(&ctx, &mut a, vec![]);
    a.session.teams_apply(teams::Event::Chats { rows: vec![row("19:a@thread.v2", 3), row("19:b@thread.v2", 4)] });
    let shapes = frame_shapes(&ctx, &mut a, vec![], size);
    assert!(badge(&shapes, "7"), "7 unread");
    a.session.teams_apply(teams::Event::Chats { rows: vec![row("19:a@thread.v2", 100)] });
    let shapes = frame_shapes(&ctx, &mut a, vec![], size);
    assert!(badge(&shapes, "99+") && !badge(&shapes, "104"), "past 99");
    if let Some(d) = std::env::var_os("QUATRELLE_FRAME_PNG").map(std::path::PathBuf::from) {
        let mut tex = Textures::default();
        for (dark, theme) in [(false, "light"), (true, "dark")] {
            a.ui.dark = Some(dark);
            for _ in 0..8 {
                frame_with_size(&ctx, &mut a, &mut tex, vec![], None, (1280, 800));
            }
            frame_with_size(&ctx, &mut a, &mut tex, vec![], Some(d.join(format!("teams-rail-{theme}.png"))), (1280, 800));
        }
        a.ui.dark = Some(false);
    }
    a.session.teams_apply(teams::Event::Chats { rows: vec![row("19:a@thread.v2", 0), row("19:b@thread.v2", 0)] });
    let shapes = frame_shapes(&ctx, &mut a, vec![], size);
    assert!(!badge(&shapes, "0") && !badge(&shapes, "99+"), "nothing unread, no badge");
    let _ = std::fs::remove_dir_all(dir);
}

/// Ten Teams events in one poll: four are applied in the first frame, the rest in the next two,
/// and a repaint is asked for while some wait; the mail's events in the same poll are applied at
/// once.
#[test]
fn at_most_four_teams_events_a_frame() {
    let (mut a, dir, _, events) = teams_app("teams-four", true);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let mut given: Vec<MailEvent> = (0..10).map(|i| chats(vec![row(&format!("19:c{i}@thread.v2"), 1)])).collect();
    given.insert(5, MailEvent::Quota { account: "sample".into(), used: 5, limit: 10 });
    *events.borrow_mut() = given;
    let run = |a: &mut MailApp| {
        let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| a.ui(ui));
        out.textures_delta.clear();
        out.viewport_output.values().any(|v| v.repaint_delay.is_zero())
    };
    let repaint = run(&mut a);
    assert_eq!(a.session.teams.rows.len(), 4, "four the first frame");
    assert_eq!(a.session.account("sample").and_then(|x| x.quota), Some((5, 10)), "the mail's at once");
    assert!(repaint, "drawn again while some wait");
    assert!(run(&mut a));
    assert_eq!(a.session.teams.rows.len(), 8);
    run(&mut a);
    assert_eq!(a.session.teams.rows.len(), 10, "all after three frames");
    let _ = std::fs::remove_dir_all(dir);
}

/// The queue never holds more than 10,000: the oldest rows and messages go (the worker sends
/// them again), a status stays.
#[test]
fn the_teams_queue_is_bounded_and_keeps_a_status() {
    let mut q = crate::conversations::TeamsQueue::default();
    let mut events = vec![MailEvent::Teams { event: teams::Event::Status { status: Status::UpToDate } }];
    events.extend((0..12_000).map(|i| chats(vec![row(&format!("19:q{i}@thread.v2"), 0)])));
    events.push(MailEvent::Quota { account: "sample".into(), used: 1, limit: 2 });
    q.take(&mut events);
    assert_eq!(events.len(), 1, "the mail's event stays to be applied");
    assert_eq!(q.len(), 10_000);
    assert!(q.holds(|e| matches!(e, teams::Event::Status { status: Status::UpToDate })), "the status is kept");
    assert!(q.holds(|e| matches!(e, teams::Event::Chats { rows } if rows[0].id == "19:q11999@thread.v2")), "the newest kept");
    assert!(!q.holds(|e| matches!(e, teams::Event::Chats { rows } if rows[0].id == "19:q0@thread.v2")), "the oldest gone");
}

/// Restored settings with Teams on and signed in start the Teams worker (the settings come after
/// the session opened).
#[test]
fn restoring_the_settings_starts_teams() {
    let (mut a, dir, jobs, _) = teams_app("teams-restore", true);
    let saved = a.saved_state();
    a.session.prefs.teams = teams::TeamsSettings::default();
    let _ = a.session.take_jobs();
    a.restore_state(&saved);
    let ctx = egui::Context::default();
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let started = jobs.borrow().iter().any(|j| matches!(j, Job::Teams { job: teams::Job::Start { .. } }));
    assert!(started, "{:?}", jobs.borrow());
    let _ = std::fs::remove_dir_all(dir);
}

/// Pictures answered are taken every frame, whatever the module: the queue never fills.
#[test]
fn pictures_are_taken_every_frame() {
    let (mut a, dir, _, _) = teams_app("teams-pictures", true);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let picture = Picture::Photo { user: "u-1".into() };
    a.run("teams.picture", json!({"kind": "photo", "user": "u-1"}));
    a.session.teams_apply(teams::Event::Picture { picture, bytes: vec![1, 2, 3] });
    assert_eq!(a.session.teams.pictures.len(), 1);
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.teams.pictures.is_empty(), "taken in the Mail module too");
    let _ = std::fs::remove_dir_all(dir);
}

/// The card, light and dark: its switch, the two ids, the guide, the sign-in, the
/// announcements; signed in, who and Sign out; the sign-in not remembered; a copy set aside with
/// Delete it.
#[test]
fn the_options_card_shows_its_fields_in_light_and_dark() {
    let (mut a, dir, jobs, _) = teams_app("teams-card", false);
    let ctx = egui::Context::default();
    let mut tex = Textures::default();
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
    let t = drawn(&ctx, &mut a, &mut tex, "signed-out");
    for wanted in [
        "Teams",
        "Use Teams in Quatrelle",
        "Application (client) ID",
        "Directory (tenant) ID",
        "How to register the application",
        "Sign in",
        "Announce new messages",
        "Only messages sent to me alone or naming me",
    ] {
        assert!(has(&t, wanted), "{wanted}: {t:?}");
    }
    assert!(!has(&t, "Sign out"), "not signed in: {t:?}");
    // Signed in, the sign-in not remembered, a copy set aside.
    let s = &mut a.session;
    s.prefs.teams.signed_in = true;
    s.prefs.teams.name = "Riley Moreno".into();
    s.prefs.teams.address = "riley@contoso.example".into();
    s.prefs.teams.object_id = "00000000-0000-0000-0000-00000000000a".into();
    s.teams_apply(teams::Event::Status { status: Status::NoKeychain });
    s.teams_apply(teams::Event::Status { status: Status::Copying { done: 12, of: 40 } });
    s.teams_apply(teams::Event::SetAside { path: "/data/files/Teams-0a1b2c3d.quatrelle.old".into() });
    let t = drawn(&ctx, &mut a, &mut tex, "signed-in");
    for wanted in [
        "Signed in as Riley Moreno (riley@contoso.example)",
        "Sign out",
        "Your sign-in won't be remembered: no keychain is available",
        "Copying your conversations… 12 of 40 conversations",
        "A copy made for another account was set aside",
        "Delete it",
    ] {
        assert!(has(&t, wanted), "{wanted}: {t:?}");
    }
    assert!(!t.iter().any(|x| x.contains("Teams-0a1b2c3d")), "no path shown: {t:?}");
    // Delete it: the worker is asked to delete that copy, and the notice goes.
    let size = egui::vec2(PAGE.0 as f32, PAGE.1 as f32);
    let at = settled_text_at(&ctx, &mut a, "Delete it", size).expect("Delete it");
    click_at(&ctx, &mut a, at, 1, size);
    assert!(jobs.borrow().iter().any(|j| matches!(j, Job::Teams { job: teams::Job::DeleteSetAside { .. } })), "{:?}", jobs.borrow());
    assert!(a.session.teams.set_aside.is_none());
    let _ = std::fs::remove_dir_all(dir);
}

/// Sign out asks whether to delete the local copy: Keep the copy signs out and keeps it, Delete
/// the copy deletes it too.
#[test]
fn signing_out_asks_whether_to_delete_the_copy() {
    for (choice, delete_copy) in [("Keep the copy", false), ("Delete the copy", true)] {
        let (mut a, dir, jobs, _) = teams_app("teams-sign-out", true);
        let ctx = egui::Context::default();
        let size = egui::vec2(PAGE.0 as f32, PAGE.1 as f32);
        a.act(crate::Action::Backstage(crate::backstage::Page::Options));
        let at = settled_text_at(&ctx, &mut a, "Sign out", size).expect("Sign out");
        // Webex's card has none while it's signed out: this is Teams'.
        click_at(&ctx, &mut a, at, 1, size);
        assert!(a.session.prefs.teams.signed_in, "nothing done before the answer");
        let at = settled_text_at(&ctx, &mut a, choice, size).expect(choice);
        click_at(&ctx, &mut a, at, 1, size);
        assert!(!a.session.prefs.teams.signed_in, "{choice}: signed out");
        let asked = jobs.borrow().iter().any(|j| matches!(j, Job::Teams { job: teams::Job::SignOut { delete_copy: d } } if *d == delete_copy));
        assert!(asked, "{choice}: {:?}", jobs.borrow());
        let _ = std::fs::remove_dir_all(dir);
    }
}
