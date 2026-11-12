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

/// The conversation pane, and its pictures.
mod chat;
mod pictures;

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
/// Delete that copy.
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
        "Delete that copy",
    ] {
        assert!(has(&t, wanted), "{wanted}: {t:?}");
    }
    assert!(!t.iter().any(|x| x.contains("Teams-0a1b2c3d")), "no path shown: {t:?}");
    // Delete that copy: the worker is asked to delete that copy, and the notice goes.
    let size = egui::vec2(PAGE.0 as f32, PAGE.1 as f32);
    let at = settled_text_at(&ctx, &mut a, "Delete that copy", size).expect("Delete that copy");
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

/// In Japanese the rail says 会話 (conversations), not the mail's grouping by thread (スレッド).
#[test]
fn the_rail_names_conversations_as_the_module_in_japanese() {
    let (mut a, dir, _, _) = teams_app("teams-rail-ja", true);
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.ui.language = "ja".into();
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let seen = labelled(&ctx, &mut a);
    crate::i18n::set_current(crate::i18n::Lang::EN);
    assert!(seen.iter().any(|(l, x)| l == "会話" && *x < 48.0), "{seen:?}");
    let _ = std::fs::remove_dir_all(dir);
}

// The list pane (Task 13).

/// The app in the Conversations module, signed in, with `rows` listed and the status up to date.
fn module_app(name: &str, rows: Vec<ChatRow>) -> (MailApp, std::path::PathBuf, Shared<Job>) {
    let (mut a, dir, jobs, _) = teams_app(name, true);
    a.session.module = quatrelle_engine::Module::Conversations;
    a.session.teams_apply(teams::Event::Status { status: Status::UpToDate });
    for part in rows.chunks(teams::EVENT_ITEMS) {
        a.session.teams_apply(teams::Event::Chats { rows: part.to_vec() });
    }
    (a, dir, jobs)
}

/// A one-to-one row with `person`.
fn one_to_one(id: &str, name: &str, person: &str, at: i64) -> ChatRow {
    ChatRow { name: name.into(), kind: ChatKind::OneOnOne, members: 2, person: Some(person.into()), at: Some(at), ..row(id, 0) }
}

/// The text shapes of a frame.
fn texts_of(shapes: &[egui::Shape]) -> Vec<(String, egui::Rect)> {
    shapes
        .iter()
        .filter_map(|s| match s {
            egui::Shape::Text(t) => Some((t.galley.text().to_string(), t.galley.rect.translate(t.pos.to_vec2()))),
            _ => None,
        })
        .collect()
}

/// A few frames for the list to settle, then the texts of the last.
fn settled(ctx: &egui::Context, a: &mut MailApp) -> Vec<(String, egui::Rect)> {
    for _ in 0..4 {
        frame(ctx, a, vec![]);
    }
    texts_of(&frame_shapes(ctx, a, vec![], egui::vec2(1280.0, 800.0)))
}

/// 20,000 chats: only the rows on screen are drawn (a few dozen names at 800 px), and a frame
/// takes no longer for it.
#[test]
fn a_list_of_20000_rows_draws_only_what_is_visible() {
    let rows: Vec<ChatRow> = (0..teams::MAX_CHATS).map(|i| ChatRow { at: Some(NOW - i as i64), ..row(&format!("19:q{i}@thread.v2"), 0) }).collect();
    let (mut a, dir, _) = module_app("teams-list-big", rows);
    assert_eq!(a.session.teams.rows.len(), teams::MAX_CHATS);
    let ctx = egui::Context::default();
    settled(&ctx, &mut a);
    let started = std::time::Instant::now();
    let texts = texts_of(&frame_shapes(&ctx, &mut a, vec![], egui::vec2(1280.0, 800.0)));
    let took = started.elapsed();
    let names = texts.iter().filter(|(t, _)| t.starts_with("Chat 19:q")).count();
    assert!(names > 0, "some rows drawn: {texts:?}");
    assert!(names <= 60, "only the visible rows: {names}");
    assert!(has_text(&texts, "Chat 19:q0@thread.v2"), "the newest first");
    assert!(took < std::time::Duration::from_millis(500), "a frame took {took:?}");
    let _ = std::fs::remove_dir_all(dir);
}

fn has_text(texts: &[(String, egui::Rect)], wanted: &str) -> bool {
    texts.iter().any(|(t, _)| t == wanted)
}

fn rect_of(texts: &[(String, egui::Rect)], pred: impl Fn(&str) -> bool) -> Option<egui::Rect> {
    texts.iter().find(|(t, _)| pred(t)).map(|(_, r)| *r)
}

/// A chat with someone from outside and a 200-character name: "External" comes first on the row,
/// whole, and the name is cut after it.
#[test]
fn external_is_drawn_before_the_name_and_never_cut() {
    let long = "Wide".repeat(50);
    let mut r = row("19:ext@thread.v2", 0);
    r.name = long.clone();
    r.external = true;
    r.last_line = "Sam: see you".into();
    r.last_external = true;
    let (mut a, dir, _) = module_app("teams-list-external", vec![r]);
    let ctx = egui::Context::default();
    let texts = settled(&ctx, &mut a);
    let name = rect_of(&texts, |t| t.starts_with("WideWide")).expect("the name drawn");
    let line = rect_of(&texts, |t| t == "Sam: see you").expect("the last line drawn");
    let on_row = |r: &egui::Rect, of: egui::Rect| (r.center().y - of.center().y).abs() < 8.0;
    let name_row: Vec<&(String, egui::Rect)> = texts.iter().filter(|(_, r)| on_row(r, name)).collect();
    let first = name_row.iter().min_by(|x, y| x.1.left().total_cmp(&y.1.left())).expect("texts on the row");
    assert_eq!(first.0, "External", "External first on the name's line: {name_row:?}");
    assert!(first.1.right() <= name.left(), "the name after it");
    let line_row: Vec<&(String, egui::Rect)> = texts.iter().filter(|(_, r)| on_row(r, line)).collect();
    let first = line_row.iter().min_by(|x, y| x.1.left().total_cmp(&y.1.left())).expect("texts on the line");
    assert_eq!(first.0, "External", "External before the sender: {line_row:?}");
    // The name is cut to the pane, External isn't.
    let pane_right = texts.iter().map(|(_, r)| r.right()).fold(0.0, f32::max);
    assert!(name.right() <= pane_right);
    assert!(name.width() < 400.0, "the name cut short: {name:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A group named like a person ("Paul Durand") still has the group's picture, not initials, and
/// its member count.
#[test]
fn a_group_named_like_a_person_still_shows_the_group_icon_and_count() {
    let r = ChatRow { name: "Paul Durand".into(), members: 4, ..row("19:grp@thread.v2", 0) };
    assert_eq!(crate::conversations::picture_of(&r), crate::conversations::RowPicture::Group);
    let one = one_to_one("19:one@thread.v2", "Paul Durand", "u-paul", NOW);
    assert_eq!(crate::conversations::picture_of(&one), crate::conversations::RowPicture::Person { user: "u-paul".into(), initials: "PD".into() });
    let meeting = ChatRow { kind: ChatKind::Meeting, ..row("19:meet@thread.v2", 0) };
    assert_eq!(crate::conversations::picture_of(&meeting), crate::conversations::RowPicture::Meeting);
    let (mut a, dir, _) = module_app("teams-list-group", vec![r]);
    let ctx = egui::Context::default();
    let texts = settled(&ctx, &mut a);
    assert!(has_text(&texts, "Paul Durand"), "{texts:?}");
    assert!(!has_text(&texts, "PD"), "no initials for a group: {texts:?}");
    assert!(has_text(&texts, "4"), "its member count: {texts:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A chat Graph won't serve is listed with its name, its preview and "Can't be read here"; a chat
/// left is listed too.
#[test]
fn an_unreadable_chat_is_listed_with_its_preview_and_its_mark() {
    let mut r = row("19:unr@thread.v2", 1);
    r.name = "Support desk".into();
    r.kind = ChatKind::Other { name: "unreadable".into() };
    r.unreadable = true;
    r.last_line = "Paul: it's fixed".into();
    let mut left = row("19:left@thread.v2", 0);
    left.name = "Old project".into();
    left.left = true;
    left.at = Some(NOW - 60);
    let (mut a, dir, _) = module_app("teams-list-unreadable", vec![r, left]);
    let ctx = egui::Context::default();
    let texts = settled(&ctx, &mut a);
    for t in ["Support desk", "Paul: it's fixed", "Can't be read here", "Old project"] {
        assert!(has_text(&texts, t), "{t}: {texts:?}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// In Conversations, Ctrl+E puts the keyboard in its search box (not the mail's), and Ctrl+F
/// (Forward in Mail) does nothing.
#[test]
fn ctrl_e_focuses_the_search_and_ctrl_f_does_nothing_here() {
    let (mut a, dir, _) = module_app("teams-list-ctrl-e", vec![row("19:a@thread.v2", 0)]);
    a.run("list.open", json!({"key": "sample:1:1:INBOX"}));
    assert!(a.session.reading.is_some(), "a message read in Mail");
    a.session.module = quatrelle_engine::Module::Conversations;
    let ctx = egui::Context::default();
    settled(&ctx, &mut a);
    frame(&ctx, &mut a, vec![key(Key::E, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("pa".into())]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.conversations.query, "pa", "typed in the conversations' box");
    assert!(a.ui.search_text.is_empty(), "not in the mail's");
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![key(Key::F, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.compose.is_none(), "Ctrl+F forwards nothing here");
    let _ = std::fs::remove_dir_all(dir);
}

/// Alt+↓ opens the next conversation and scrolls it into view; Alt+↑ the previous; in Mail they
/// don't.
#[test]
fn alt_down_opens_the_next_conversation() {
    let rows: Vec<ChatRow> = (0..200).map(|i| ChatRow { at: Some(NOW - i as i64), ..row(&format!("19:s{i}@thread.v2"), 0) }).collect();
    let (mut a, dir, _) = module_app("teams-list-alt", rows);
    let ctx = egui::Context::default();
    settled(&ctx, &mut a);
    frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::ALT)]);
    assert_eq!(a.session.teams.open.as_deref(), Some("19:s0@thread.v2"));
    frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::ALT)]);
    assert_eq!(a.session.teams.open.as_deref(), Some("19:s1@thread.v2"));
    frame(&ctx, &mut a, vec![key(Key::ArrowUp, Modifiers::ALT)]);
    assert_eq!(a.session.teams.open.as_deref(), Some("19:s0@thread.v2"));
    // Far down the list: the row opened is brought into view.
    a.run("teams.open", json!({"chat": "19:s150@thread.v2"}));
    frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::ALT)]);
    assert_eq!(a.session.teams.open.as_deref(), Some("19:s151@thread.v2"));
    let texts = settled(&ctx, &mut a);
    assert!(has_text(&texts, "Chat 19:s151@thread.v2"), "scrolled to it: {texts:?}");
    a.session.module = quatrelle_engine::Module::Mail;
    settled(&ctx, &mut a);
    frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::ALT)]);
    assert_eq!(a.session.teams.open.as_deref(), Some("19:s151@thread.v2"), "not in Mail");
    let _ = std::fs::remove_dir_all(dir);
}

/// A search typed in the box goes to the worker 300 ms after the last key; its hits replace the
/// rows while the box has text, with "Indexing…" while the index is being built.
#[test]
fn the_search_is_sent_after_a_pause_and_its_hits_replace_the_rows() {
    let rows = vec![row("19:a@thread.v2", 0), ChatRow { at: Some(NOW - 5), ..row("19:b@thread.v2", 0) }];
    let (mut a, dir, jobs) = module_app("teams-list-search", rows);
    let ctx = egui::Context::default();
    let at = |t: f64, events: Vec<Event>| RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
        time: Some(t),
        events,
        ..Default::default()
    };
    let run = |a: &mut MailApp, input: RawInput| {
        let mut out = ctx.run_ui(input, |ui| a.ui(ui));
        out.textures_delta.clear();
    };
    for i in 0..4 {
        run(&mut a, at(f64::from(i) * 0.01, vec![]));
    }
    a.ui.conversations.focus_search = true;
    run(&mut a, at(0.1, vec![]));
    run(&mut a, at(0.11, vec![Event::Text("b".into())]));
    let searched =
        |jobs: &Shared<Job>| jobs.borrow().iter().filter(|j| matches!(j, Job::Teams { job: teams::Job::Search { query } } if query == "b")).count();
    run(&mut a, at(0.2, vec![]));
    assert_eq!(searched(&jobs), 0, "not at once");
    run(&mut a, at(0.5, vec![]));
    assert_eq!(searched(&jobs), 1, "after the pause");
    run(&mut a, at(0.6, vec![]));
    assert_eq!(searched(&jobs), 1, "once");
    a.session.teams_apply(teams::Event::Found {
        query: "b".into(),
        hits: vec![("19:b@thread.v2".into(), String::new())],
        indexing: true,
        partial: false,
    });
    let texts = settled(&ctx, &mut a);
    assert!(has_text(&texts, "Chat 19:b@thread.v2"), "{texts:?}");
    assert!(!has_text(&texts, "Chat 19:a@thread.v2"), "the hits replace the rows: {texts:?}");
    assert!(has_text(&texts, "Indexing…"), "{texts:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// The people of the one-to-one rows on screen are told to the worker once (their presence), and
/// each one's photo is asked once.
#[test]
fn the_people_shown_are_told_once_and_their_photos_asked_once() {
    let rows = vec![one_to_one("19:p1@thread.v2", "Paul Durand", "u-paul", NOW), one_to_one("19:p2@thread.v2", "Ana Lima", "u-ana", NOW - 5)];
    let (mut a, dir, jobs) = module_app("teams-list-people", rows);
    a.session.teams_apply(teams::Event::Presence { presences: vec![("u-paul".into(), teams::Presence::Busy)] });
    let ctx = egui::Context::default();
    for _ in 0..6 {
        frame(&ctx, &mut a, vec![]);
    }
    let jobs = jobs.borrow();
    let visible: Vec<&Vec<String>> = jobs
        .iter()
        .filter_map(|j| match j {
            Job::Teams { job: teams::Job::Visible { users } } => Some(users),
            _ => None,
        })
        .collect();
    assert_eq!(visible.len(), 1, "told once: {jobs:?}");
    let mut told = visible[0].clone();
    told.sort();
    assert_eq!(told, vec!["u-ana".to_string(), "u-paul".to_string()]);
    let photos = |u: &str| {
        jobs.iter().filter(|j| matches!(j, Job::Teams { job: teams::Job::Picture { picture: Picture::Photo { user } } } if user == u)).count()
    };
    assert_eq!((photos("u-paul"), photos("u-ana")), (1, 1), "{jobs:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Signed out, the rail's Conversations leads to File › Options, where Teams is signed in to
/// (the module itself can't be shown).
#[test]
fn signed_out_the_rail_leads_to_the_options() {
    let (mut a, dir, _, _) = teams_app("teams-rail-signed-out", false);
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
    let cell =
        nodes.iter().find_map(|(_, n)| n.bounds().filter(|b| n.label() == Some("Conversations") && b.x0 < 48.0)).expect("Conversations in the rail");
    let at = egui::pos2(((cell.x0 + cell.x1) / 2.0) as f32, ((cell.y0 + cell.y1) / 2.0) as f32);
    click_at(&ctx, &mut a, at, 1, egui::vec2(1280.0, 800.0));
    assert_eq!(a.session.module, quatrelle_engine::Module::Mail, "the module isn't shown signed out");
    assert_eq!(a.ui.backstage.as_ref().map(|b| b.page), Some(crate::backstage::Page::Options), "File › Options instead");
    let _ = std::fs::remove_dir_all(dir);
}

/// The list's states, light and dark: no conversations yet, signing in, copying (and the cap),
/// offline, sign in again, a permission missing, and the list.
#[test]
fn the_list_states_in_light_and_dark() {
    let pictures = std::env::var_os("QUATRELLE_FRAME_PNG").map(std::path::PathBuf::from);
    let states: Vec<(&str, Status, bool, Vec<&str>)> = vec![
        ("empty", Status::UpToDate, false, vec!["No conversations yet"]),
        ("signing-in", Status::SigningIn, false, vec!["Signing in…"]),
        ("copying", Status::Copying { done: 12, of: 40 }, false, vec!["Copying your conversations… 12 of 40 conversations"]),
        ("copying-cap", Status::Copying { done: 3, of: teams::MAX_CHATS as u32 }, true, vec!["Only the 20,000 most recent conversations are copied"]),
        ("offline", Status::Offline, true, vec!["Offline"]),
        ("sign-in-again", Status::SignInAgain, true, vec!["Teams: sign in again to go on", "Sign in"]),
        (
            "permission",
            Status::PermissionMissing { scope: "Chat.Read".into() },
            false,
            vec![
                "Quatrelle needs the permission Chat.Read: sign in again to accept it, or ask your administrator to grant admin consent in Microsoft Entra",
            ],
        ),
        ("list", Status::UpToDate, true, vec!["Paul Durand", "Paul: it's fixed", "External", "Can't be read here", "Sam: the files are ready"]),
    ];
    for (name, status, with_rows, wanted) in states {
        let rows = if with_rows {
            let mut g = row("19:g@thread.v2", 2);
            g.name = "Support team".into();
            g.last_line = "Paul: it's fixed".into();
            let mut x = row("19:x@thread.v2", 0);
            x.name = "Sam Okafor, Lee Chen".into();
            x.kind = ChatKind::Other { name: "unreadable".into() };
            x.unreadable = true;
            x.external = true;
            x.last_external = true;
            x.last_line = "Sam: the files are ready".into();
            x.at = Some(NOW - 90_000);
            let mut p = one_to_one("19:p@thread.v2", "Paul Durand", "u-paul", NOW);
            p.last_line = "See you at ten".into();
            vec![p, g, x]
        } else {
            vec![]
        };
        let (mut a, dir, _) = module_app(&format!("teams-list-{name}"), rows);
        a.session.teams_apply(teams::Event::Status { status });
        a.session.teams_apply(teams::Event::Presence { presences: vec![("u-paul".into(), teams::Presence::Available)] });
        let ctx = egui::Context::default();
        let mut tex = Textures::default();
        for (dark, theme) in [(false, "light"), (true, "dark")] {
            a.ui.dark = Some(dark);
            for _ in 0..6 {
                frame_with_size(&ctx, &mut a, &mut tex, vec![], None, (1280, 800));
            }
            let png = pictures.as_ref().map(|d| d.join(format!("teams-list-{name}-{theme}.png")));
            let (_, shapes) = frame_with_size(&ctx, &mut a, &mut tex, vec![], png, (1280, 800));
            let texts = texts_of(&shapes);
            for w in &wanted {
                assert!(has_text(&texts, w), "{name} {theme}: {w}: {texts:?}");
            }
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
