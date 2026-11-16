// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: a search's hit opened at its message (R38). The hit's row opens its chat at the
//! message found; the chat is scrolled to it once (the user has the scroll after) and the message
//! tinted two seconds; newer messages left out, "Go to the newest messages" under the last one
//! received (before the user's own going out) opens the chat again. With `QUATRELLE_FRAME_PNG`
//! set to a folder, the pane is painted there, light and dark.

use quatrelle_engine::model::pim::teams::{ChatMessage, OutState, Outgoing};

use super::chat::{CHAT, ME, TENANT_ID, group_row, members, para, said, user, words};
use super::*;

const SIZE: egui::Vec2 = egui::vec2(1280.0, 800.0);
/// Where the wheel turns: over the chat's messages.
const OVER: egui::Pos2 = egui::pos2(900.0, 400.0);

/// Messages `range` of a long chat, Ana's and the user's in turn, half a minute apart.
fn lines(range: std::ops::Range<i64>) -> Vec<ChatMessage> {
    range
        .map(|i| {
            let who = if i % 2 == 0 { user("u-ana", "Ana Lima") } else { user(ME, "Riley Moreno") };
            said(&format!("m{i:03}"), who, NOW - 7200 + i * 30, para(vec![words(&format!("Line {i}"))]))
        })
        .collect()
}

/// The window under test: its context, and the textures it was given from its first frame (to
/// paint it).
struct Window {
    ctx: egui::Context,
    tex: Textures,
}

impl Window {
    /// A frame with `events`: what it drew; with `png`, painted there.
    fn frame_painted(&mut self, a: &mut MailApp, events: Vec<Event>, png: Option<std::path::PathBuf>) -> Vec<egui::Shape> {
        frame_with_size(&self.ctx, a, &mut self.tex, events, png, (1280, 800)).1
    }

    fn frame(&mut self, a: &mut MailApp, events: Vec<Event>) -> Vec<egui::Shape> {
        self.frame_painted(a, events, None)
    }

    /// A few frames for the window to settle: the texts of the last.
    fn settled(&mut self, a: &mut MailApp) -> Vec<(String, egui::Rect)> {
        for _ in 0..6 {
            self.frame(a, vec![]);
        }
        texts_of(&self.frame(a, vec![]))
    }

    fn click(&mut self, a: &mut MailApp, at: egui::Pos2) {
        let press = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        for events in [vec![Event::PointerMoved(at)], vec![press(true)], vec![press(false)], vec![]] {
            self.frame(a, events);
        }
    }

    /// The wheel turned towards the newest, a notch a frame, until the end; then a few frames
    /// still.
    fn scroll_to_the_bottom(&mut self, a: &mut MailApp) {
        let notch = Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -1500.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
            modifiers: Modifiers::NONE,
        };
        self.frame(a, vec![Event::PointerMoved(OVER)]);
        for _ in 0..12 {
            self.frame(a, vec![Event::PointerMoved(OVER), notch.clone()]);
        }
        for _ in 0..20 {
            self.frame(a, vec![Event::PointerMoved(OVER)]);
        }
    }
}

/// A frame at `time` (the window's clock, in seconds): what it drew.
fn frame_at(ctx: &egui::Context, a: &mut MailApp, time: f64, events: Vec<Event>) -> Vec<egui::Shape> {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SIZE)), time: Some(time), events, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let mut shapes = Vec::new();
    fn flat(s: egui::Shape, out: &mut Vec<egui::Shape>) {
        match s {
            egui::Shape::Vec(v) => v.into_iter().for_each(|s| flat(s, out)),
            s => out.push(s),
        }
    }
    for s in out.shapes {
        flat(s.shape, &mut shapes);
    }
    shapes
}

/// Where the chat's messages show: under its header, above the box to write in.
fn in_view(texts: &[(String, egui::Rect)], text: &str) -> bool {
    let top = rect_of(texts, |t| t == "Members: 4").map_or(0.0, |r| r.bottom());
    let bottom = rect_of(texts, |t| t == "Write a message").map_or(SIZE.y, |r| r.top());
    rect_of(texts, |t| t == text).is_some_and(|r| r.top() >= top && r.bottom() <= bottom)
}

/// Whether the message saying `text` is drawn on the tint of a message found.
fn tinted(a: &MailApp, shapes: &[egui::Shape], text: &str) -> bool {
    let Some(at) = rect_of(&texts_of(shapes), |t| t == text) else { return false };
    let tint = a.ui.tokens.hover_row;
    shapes.iter().any(|s| matches!(s, egui::Shape::Rect(r) if r.fill == tint && r.rect.contains(at.center()) && r.rect.left() > 300.0))
}

/// The app in Conversations, a search for "budget" having found two of the chat's messages, the
/// hit's row clicked; the worker's answer: the found message's page and the next (messages 0 to
/// 199 of many more), the newest left out, and a message of the user's going out.
fn opened_at_a_hit(name: &str) -> (MailApp, std::path::PathBuf, Shared<Job>, Window) {
    let (mut a, dir, jobs) = module_app(name, vec![group_row()]);
    let account = teams::Event::Account {
        name: "Riley Moreno".into(),
        address: "riley@contoso.example".into(),
        object_id: ME.into(),
        tenant_id: TENANT_ID.into(),
        drive_host: None,
    };
    a.session.teams_apply(account);
    a.ui.conversations.query = "budget".into();
    a.run("teams.search", json!({"query": "budget"}));
    let hits = vec![(CHAT.to_string(), "m040".to_string()), (CHAT.to_string(), "m150".to_string())];
    a.session.teams_apply(teams::Event::Found { query: "budget".into(), hits, indexing: false, partial: false });
    let mut w = Window { ctx: egui::Context::default(), tex: Textures::default() };
    let texts = w.settled(&mut a);
    let row = rect_of(&texts, |t| t == "Support team").expect("the hit's row");
    w.click(&mut a, row.center());
    let finds: Vec<Job> = jobs.borrow().iter().filter(|j| matches!(j, Job::Teams { job: teams::Job::Find { .. } })).cloned().collect();
    assert_eq!(finds, [Job::Teams { job: teams::Job::Find { chat: CHAT.into(), message: "m040".into() } }], "the chat's first hit");
    a.session.teams_apply(teams::Event::Members { chat: CHAT.into(), members: members(), web_url: None, first: true });
    for part in lines(0..200).chunks(teams::EVENT_ITEMS) {
        a.session.teams_apply(teams::Event::Messages { chat: CHAT.into(), messages: part.to_vec(), fresh: false, older: false });
    }
    a.session.teams_apply(teams::Event::Placed { chat: CHAT.into(), message: "m040".into(), newest: false });
    let going = Outgoing { seq: 3, local: "100-1".into(), chat: CHAT.into(), body: para(vec![words("On its way")]), state: OutState::Waiting };
    a.session.teams_apply(teams::Event::Outbox { chat: CHAT.into(), items: vec![going], took: None });
    (a, dir, jobs, w)
}

/// The window painted light and dark as it is (`QUATRELLE_FRAME_PNG`), each pass checked by
/// `check`.
fn painted(a: &mut MailApp, w: &mut Window, name: &str, check: impl Fn(&MailApp, &[egui::Shape]) -> bool) {
    let Some(d) = std::env::var_os("QUATRELLE_FRAME_PNG").map(std::path::PathBuf::from) else { return };
    for (dark, theme) in [(false, "light"), (true, "dark")] {
        a.ui.dark = Some(dark);
        for _ in 0..4 {
            w.frame(a, vec![]);
        }
        let shapes = w.frame_painted(a, vec![], Some(d.join(format!("{name}-{theme}.png"))));
        assert!(check(a, &shapes), "{name}, {theme}");
    }
    a.ui.dark = Some(false);
}

#[test]
fn a_hit_scrolls_to_its_message() {
    let (mut a, dir, _, mut w) = opened_at_a_hit("teams-find-scroll");
    w.settled(&mut a);
    let shapes = w.frame(&mut a, vec![]);
    let texts = texts_of(&shapes);
    assert!(in_view(&texts, "Line 40"), "scrolled to the message found: {:?}", rect_of(&texts, |t| t == "Line 40"));
    assert!(!in_view(&texts, "Line 199"), "not opened at the last");
    assert!(tinted(&a, &shapes, "Line 40") && !tinted(&a, &shapes, "Line 41"), "the message found tinted");
    painted(&mut a, &mut w, "teams-chat-found", |a, shapes| tinted(a, shapes, "Line 40") && in_view(&texts_of(shapes), "Line 40"));
    // Two seconds later: no tint, still there.
    let now = w.ctx.input(|i| i.time);
    frame_at(&w.ctx, &mut a, now + 2.1, vec![]);
    let shapes = frame_at(&w.ctx, &mut a, now + 2.2, vec![]);
    assert!(!tinted(&a, &shapes, "Line 40"), "the tint lasts two seconds");
    assert!(in_view(&texts_of(&shapes), "Line 40"));
    // Once: the user scrolls away, and the chat stays where they put it.
    w.scroll_to_the_bottom(&mut a);
    let texts = w.settled(&mut a);
    assert!(!in_view(&texts, "Line 40") && in_view(&texts, "Line 199"), "the user's scroll stays");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn go_to_the_newest_messages_opens_the_chat_again() {
    let (mut a, dir, jobs, mut w) = opened_at_a_hit("teams-find-newest");
    w.scroll_to_the_bottom(&mut a);
    let texts = texts_of(&w.frame(&mut a, vec![]));
    let link = rect_of(&texts, |t| t == "Go to the newest messages").expect("the link, newer messages being left out");
    let (last, going) = (rect_of(&texts, |t| t == "Line 199").unwrap(), rect_of(&texts, |t| t == "On its way").unwrap());
    assert!(link.top() > last.bottom() && link.bottom() < going.top(), "under the last message received, before the user's going out");
    assert!(in_view(&texts, "Go to the newest messages"));
    painted(&mut a, &mut w, "teams-chat-newest-link", |_, shapes| in_view(&texts_of(shapes), "Go to the newest messages"));
    jobs.borrow_mut().clear();
    w.click(&mut a, link.center());
    let opened: Vec<Job> =
        jobs.borrow().iter().filter(|j| matches!(j, Job::Teams { job: teams::Job::Open { .. } | teams::Job::Find { .. } })).cloned().collect();
    assert_eq!(opened, [Job::Teams { job: teams::Job::Open { chat: Some(CHAT.into()) } }]);
    assert!(!a.session.teams.newer_left_out && a.session.teams.go_to.is_none());
    // The worker's answer: the last pages, the newest at the foot, no link any more.
    for part in lines(100..300).chunks(teams::EVENT_ITEMS) {
        a.session.teams_apply(teams::Event::Messages { chat: CHAT.into(), messages: part.to_vec(), fresh: false, older: false });
    }
    let texts = w.settled(&mut a);
    assert!(!has_text(&texts, "Go to the newest messages"));
    assert!(in_view(&texts, "Line 299"), "at the newest: {:?}", rect_of(&texts, |t| t == "Line 299"));
    let _ = std::fs::remove_dir_all(dir);
}
