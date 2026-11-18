// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: a new conversation, written in the open chat's place (decision 14). Ctrl+N or the
//! list's button opens it with the keyboard in "To:", which asks the directory 300 ms after the
//! last key; the people found are picked as chips (↑ and ↓ choose, Enter picks, Backspace takes the
//! last away, a guest marked External, a name drawn as its text on one line, at most 20), a group
//! name from two people on. Enter in the box creates the conversation with its first message;
//! while it's created everything waits; a failure, a refusal or a first message refused keeps the
//! people, the text and the files, and says why; the chat made (or the one-to-one chat there
//! already) takes the pane's place. Esc or Cancel closes it, the list's search kept. With
//! `QUATRELLE_FRAME_PNG` set to a folder, the pane is painted there, light and dark.

use std::path::PathBuf;

use quatrelle_engine::model::pim::teams::{NEW_CHAT_PEOPLE, News, NotCreated, OUTBOX_MAX, Person};
use quatrelle_engine::{TeamsCreating, TeamsPeople};
use serde_json::Value;

use super::super::list::follow_up::{Textures, frame_with_size};
use super::chat::{TEAMS_LINK, chat_app, group_row, settled_at};
use super::writing::{SIZE, asked, enter};
use super::*;
use crate::Action;
use crate::conversations::{NEW, add_files, draft};

const ANA: &str = "00000000-0000-0000-0000-0000000000a1";
const BRUNO: &str = "00000000-0000-0000-0000-0000000000b2";
const SAM: &str = "00000000-0000-0000-0000-0000000000c3";
/// The one-to-one chat with Ana.
const WITH_ANA: &str = "19:00000000-0000-0000-0000-0000000000a1_riley@unq.gbl.spaces";

fn person(id: &str, name: &str, address: &str, guest: bool) -> Person {
    Person { id: id.into(), name: name.into(), address: address.into(), guest }
}

fn ana() -> Person {
    person(ANA, "Ana Lima", "ana@contoso.example", false)
}

fn bruno() -> Person {
    person(BRUNO, "Bruno Reis", "bruno@contoso.example", false)
}

/// A guest of the organisation.
fn sam() -> Person {
    person(SAM, "Sam Okafor", "sam@fabrikam.example", true)
}

/// Person `i` of many.
fn many(i: usize) -> Person {
    person(&format!("00000000-0000-0000-0000-{:012x}", 0x100 + i), &format!("Person {i}"), &format!("p{i}@contoso.example"), false)
}

/// The new conversation's pane.
fn pane_of(a: &MailApp) -> &crate::conversations::NewChat {
    a.ui.conversations.new_chat.as_ref().expect("the new conversation's pane")
}

fn picked(a: &MailApp) -> Vec<String> {
    pane_of(a).picked.iter().map(|p| p.name.clone()).collect()
}

/// What the new conversation's box holds.
fn in_new_box(a: &MailApp) -> String {
    a.ui.conversations.drafts.get(NEW).map(|d| d.text.clone()).unwrap_or_default()
}

/// The params of the conversations asked to be created.
fn creates(acts: &[Action]) -> Vec<Value> {
    asked(acts, "teams.create")
}

/// The directory asked for `query` (as the pane asks) and answering `people`.
fn answered(a: &mut MailApp, query: &str, people: Vec<Person>) {
    a.run("teams.people", json!({"query": query}));
    a.session.teams_apply(teams::Event::People { query: query.into(), people, failed: false });
}

/// The window, and the pane's clock (in seconds).
struct Win {
    ctx: egui::Context,
    time: f64,
}

impl Win {
    /// The support chat open in Conversations, a window over it (its fonts from frames of it whole),
    /// and the new conversation opened (as Ctrl+N opens it) when `open`; the jobs the app sends.
    fn app(name: &str, rows: Vec<ChatRow>, open: bool) -> (MailApp, PathBuf, Win, Shared<Job>) {
        let (mut a, dir, jobs) = chat_app(name, group_row(), TEAMS_LINK, vec![]);
        if !rows.is_empty() {
            a.session.teams_apply(teams::Event::Chats { rows: [vec![group_row()], rows].concat() });
        }
        let ctx = egui::Context::default();
        settled_at(&ctx, &mut a, SIZE);
        if open {
            crate::conversations::open_new_chat(&a.session, &mut a.ui.conversations);
        }
        let mut w = Win { ctx, time: 100.0 };
        w.settled(&mut a);
        (a, dir, w, jobs)
    }

    /// The pane alone, `after` seconds after the last frame, given `events`: what it asked for, and
    /// what it drew.
    fn pane(&mut self, a: &mut MailApp, after: f64, events: Vec<Event>) -> (Vec<Action>, Vec<egui::Shape>) {
        // After the frames of the window whole, which keep their own clock.
        self.time = self.time.max(self.ctx.input(|i| i.time)) + after;
        let input = RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SIZE.0 as f32, SIZE.1 as f32))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut acts = Vec::new();
        let mut out = self.ctx.run_ui(input, |ui| {
            egui::CentralPanel::default().show(ui, |ui| crate::panes::conversations::chat(&a.session, &mut a.ui, ui, &mut acts));
        });
        out.textures_delta.clear();
        fn flat(s: egui::Shape, out: &mut Vec<egui::Shape>) {
            match s {
                egui::Shape::Vec(v) => v.into_iter().for_each(|s| flat(s, out)),
                s => out.push(s),
            }
        }
        let mut shapes = Vec::new();
        for s in out.shapes {
            flat(s.shape, &mut shapes);
        }
        (acts, shapes)
    }

    /// `events` given to the pane a frame each, a moment apart, then one more: what it asked for.
    fn typed(&mut self, a: &mut MailApp, events: Vec<Event>) -> Vec<Action> {
        let mut acts = Vec::new();
        for e in events {
            acts.extend(self.pane(a, 0.02, vec![e]).0);
        }
        acts.extend(self.pane(a, 0.02, vec![]).0);
        acts
    }

    /// A few frames of the pane alone, a moment apart: what they asked for, and the last one's
    /// texts.
    fn settled(&mut self, a: &mut MailApp) -> (Vec<Action>, Vec<(String, egui::Rect)>) {
        let mut acts = Vec::new();
        for _ in 0..5 {
            acts.extend(self.pane(a, 0.02, vec![]).0);
        }
        let (more, shapes) = self.pane(a, 0.02, vec![]);
        acts.extend(more);
        (acts, texts_of(&shapes))
    }

    fn texts(&mut self, a: &mut MailApp) -> Vec<(String, egui::Rect)> {
        self.settled(a).1
    }

    /// A click on the pane alone at `at`: what it asked for.
    fn click(&mut self, a: &mut MailApp, at: egui::Pos2) -> Vec<Action> {
        let press = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        let mut acts = Vec::new();
        for events in [vec![Event::PointerMoved(at)], vec![press(true)], vec![press(false)], vec![]] {
            acts.extend(self.pane(a, 0.02, events).0);
        }
        acts
    }

    /// A click on the text `text` drawn in the pane.
    fn click_text(&mut self, a: &mut MailApp, text: &str) -> Vec<Action> {
        let texts = self.texts(a);
        let at = rect_of(&texts, |t| t == text).unwrap_or_else(|| panic!("{text} drawn: {texts:?}")).center();
        self.click(a, at)
    }

    /// `text` typed in "To:" (which has the keyboard), then the pause: the directory asked, as
    /// the pane asks it, and answering `people`.
    fn look_up(&mut self, a: &mut MailApp, text: &str, people: Vec<Person>) {
        a.ui.conversations.new_chat.as_mut().expect("open").focus = true;
        self.pane(a, 0.02, vec![]);
        let mut acts = self.typed(a, vec![Event::Text(text.into())]);
        acts.extend(self.pane(a, 0.35, vec![]).0);
        let asked = asked(&acts, "teams.people");
        assert_eq!(asked, [json!({"query": text})]);
        for act in acts {
            a.act(act);
        }
        a.session.teams_apply(teams::Event::People { query: text.into(), people, failed: false });
    }

    /// `p` looked up by the first word of their name and picked with Enter.
    fn pick(&mut self, a: &mut MailApp, p: Person) {
        let word = p.name.split(' ').next().unwrap_or_default().to_lowercase();
        self.look_up(a, &word, vec![p]);
        self.settled(a);
        self.typed(a, vec![enter()]);
    }

    /// `text` written in the box (a click on it first).
    fn write(&mut self, a: &mut MailApp, text: &str) {
        self.click_text(a, "Write a message");
        self.typed(a, vec![Event::Text(text.into())]);
    }

    /// Enter in the box: what the pane asked for, done by the window.
    fn send(&mut self, a: &mut MailApp) -> Vec<Value> {
        let acts = self.typed(a, vec![enter()]);
        let asked = creates(&acts);
        for act in acts {
            a.act(act);
        }
        asked
    }
}

/// Where the window's control named `label` is, in the middle of it.
fn window_control(ctx: &egui::Context, a: &mut MailApp, label: &str) -> Option<egui::Pos2> {
    ctx.enable_accesskit();
    let input =
        RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SIZE.0 as f32, SIZE.1 as f32))), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
    let b = nodes.iter().find(|(_, n)| n.label() == Some(label)).and_then(|(_, n)| n.bounds())?;
    Some(egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32))
}

/// The colour `text` is drawn in.
fn colour_of(shapes: &[egui::Shape], text: &str) -> Option<egui::Color32> {
    shapes.iter().find_map(|s| match s {
        egui::Shape::Text(t) if t.galley.text() == text => t.galley.job.sections.first().map(|x| x.format.color),
        _ => None,
    })
}

/// Ctrl+N in Conversations opens the new conversation in the open chat's place, the keyboard in
/// "To:"; Esc closes it, and the list's button opens it the same; in Mail, Ctrl+N still writes a
/// message.
#[test]
fn ctrl_n_and_the_button_open_a_new_conversation() {
    let (mut a, dir, w, _) = Win::app("teams-new-open", vec![], false);
    let size = egui::vec2(SIZE.0 as f32, SIZE.1 as f32);
    frame(&w.ctx, &mut a, vec![key(Key::N, Modifiers::COMMAND)]);
    let texts = settled_at(&w.ctx, &mut a, SIZE);
    for t in ["New conversation", "Cancel", "To:", "A name or an address", "Write a message"] {
        assert!(has_text(&texts, t), "{t}: {texts:?}");
    }
    assert!(!has_text(&texts, "Members: 4"), "in the open chat's place: {texts:?}");
    frame(&w.ctx, &mut a, vec![Event::Text("ana".into())]);
    assert_eq!(pane_of(&a).to, "ana", "the keyboard in To:");
    frame(&w.ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert_eq!(pane_of(&a).to, "", "Esc empties To: first");
    frame(&w.ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(a.ui.conversations.new_chat.is_none(), "then Esc closes it");
    let texts = settled_at(&w.ctx, &mut a, SIZE);
    assert!(has_text(&texts, "Members: 4"), "the chat back: {texts:?}");
    let plus = window_control(&w.ctx, &mut a, "New conversation").expect("the list's button");
    assert!(plus.x < 400.0, "beside the list's search: {plus:?}");
    click_at(&w.ctx, &mut a, plus, 1, size);
    assert!(a.ui.conversations.new_chat.is_some(), "the button opens it");
    settled_at(&w.ctx, &mut a, SIZE);
    frame(&w.ctx, &mut a, vec![Event::Text("bru".into())]);
    assert_eq!(pane_of(&a).to, "bru", "the keyboard in To:");
    a.run("view.module", json!({"name": "mail"}));
    settled_at(&w.ctx, &mut a, SIZE);
    assert!(a.session.compose.is_none());
    frame(&w.ctx, &mut a, vec![key(Key::N, Modifiers::COMMAND)]);
    frame(&w.ctx, &mut a, vec![]);
    assert!(a.session.compose.is_some(), "in Mail, a new message");
    let _ = std::fs::remove_dir_all(dir);
}

/// "an", then "ana" 50 ms later: nothing asked while typing; 300 ms after the last key, the
/// directory is asked once, for "ana"; and "Searching…" until it answers.
#[test]
fn typing_asks_the_directory_after_a_pause() {
    let (mut a, dir, mut w, _) = Win::app("teams-new-pause", vec![], true);
    let mut acts = w.pane(&mut a, 0.02, vec![Event::Text("an".into())]).0;
    acts.extend(w.pane(&mut a, 0.05, vec![Event::Text("a".into())]).0);
    acts.extend(w.pane(&mut a, 0.05, vec![]).0);
    assert_eq!(pane_of(&a).to, "ana");
    assert!(asked(&acts, "teams.people").is_empty(), "nothing yet");
    let (later, shapes) = w.pane(&mut a, 0.26, vec![]);
    assert!(has_text(&texts_of(&shapes), "Searching…"));
    let mut acts = later;
    for _ in 0..5 {
        acts.extend(w.pane(&mut a, 0.1, vec![]).0);
    }
    assert_eq!(asked(&acts, "teams.people"), [json!({"query": "ana"})], "once");
    // The directory not reached: said.
    for act in acts {
        a.act(act);
    }
    a.session.teams_apply(teams::Event::People { query: "ana".into(), people: vec![], failed: true });
    assert!(has_text(&w.texts(&mut a), "The directory couldn't be searched"));
    let _ = std::fs::remove_dir_all(dir);
}

/// Ana found, Enter: she's a chip, "To:" is empty and the people found forgotten
/// (`teams.people` of nothing); looked up again, she isn't offered twice.
#[test]
fn a_person_found_is_picked_as_a_chip() {
    let (mut a, dir, mut w, _) = Win::app("teams-new-pick", vec![], true);
    w.look_up(&mut a, "ana", vec![ana()]);
    let texts = w.texts(&mut a);
    assert!(has_text(&texts, "Ana Lima") && has_text(&texts, "ana@contoso.example"), "{texts:?}");
    let acts = w.typed(&mut a, vec![enter()]);
    assert_eq!(picked(&a), ["Ana Lima"]);
    assert_eq!(pane_of(&a).to, "");
    assert_eq!(asked(&acts, "teams.people"), [json!({"query": ""})]);
    for act in acts {
        a.act(act);
    }
    let texts = w.texts(&mut a);
    assert!(has_text(&texts, "Ana Lima") && !has_text(&texts, "ana@contoso.example"), "the chip alone: {texts:?}");
    w.look_up(&mut a, "ana", vec![ana()]);
    let texts = w.texts(&mut a);
    assert_eq!(texts.iter().filter(|(t, _)| t == "Ana Lima").count(), 1, "the chip, not a second offer: {texts:?}");
    assert!(has_text(&texts, "No one found"), "{texts:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// ↓ chooses the next one found, ↑ the one before; Enter picks the one chosen. A click picks too.
#[test]
fn arrows_choose_among_the_people_found() {
    let (mut a, dir, mut w, _) = Win::app("teams-new-arrows", vec![], true);
    let down = || key(Key::ArrowDown, Modifiers::NONE);
    w.look_up(&mut a, "re", vec![ana(), bruno(), sam()]);
    w.settled(&mut a);
    w.typed(&mut a, vec![down(), down(), down(), key(Key::ArrowUp, Modifiers::NONE)]);
    assert_eq!(pane_of(&a).chosen, 1, "the second");
    w.typed(&mut a, vec![enter()]);
    assert_eq!(picked(&a), ["Bruno Reis"]);
    w.look_up(&mut a, "sam", vec![sam()]);
    let acts = w.click_text(&mut a, "Sam Okafor");
    assert_eq!(picked(&a), ["Bruno Reis", "Sam Okafor"], "a click picks");
    assert_eq!(asked(&acts, "teams.people"), [json!({"query": ""})]);
    let _ = std::fs::remove_dir_all(dir);
}

/// Backspace in an empty "To:" takes the last chip away, one at a time; with text in it, it's the
/// text's.
#[test]
fn backspace_takes_the_last_chip() {
    let (mut a, dir, mut w, _) = Win::app("teams-new-backspace", vec![], true);
    w.pick(&mut a, ana());
    w.pick(&mut a, bruno());
    assert_eq!(picked(&a), ["Ana Lima", "Bruno Reis"]);
    let back = || key(Key::Backspace, Modifiers::NONE);
    w.typed(&mut a, vec![Event::Text("x".into()), back()]);
    assert_eq!((pane_of(&a).to.as_str(), picked(&a).len()), ("", 2), "the text's first");
    w.typed(&mut a, vec![back()]);
    assert_eq!(picked(&a), ["Ana Lima"]);
    w.typed(&mut a, vec![back(), back()]);
    assert!(picked(&a).is_empty());
    let texts = w.texts(&mut a);
    assert!(!has_text(&texts, "Ana Lima") && has_text(&texts, "A name or an address"), "{texts:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A guest is marked "External", before the name, among the people found and on the chip; a
/// chip's cross takes them away.
#[test]
fn a_guest_is_marked_external() {
    let (mut a, dir, mut w, _) = Win::app("teams-new-guest", vec![], true);
    w.look_up(&mut a, "sam", vec![sam(), bruno()]);
    let texts = w.texts(&mut a);
    let (mark, name) = (rect_of(&texts, |t| t == "External").expect("External"), rect_of(&texts, |t| t == "Sam Okafor").expect("Sam"));
    assert!(mark.right() <= name.left() && (mark.center().y - name.center().y).abs() < 6.0, "before the name: {mark:?} {name:?}");
    let bruno_at = rect_of(&texts, |t| t == "Bruno Reis").expect("Bruno");
    assert_eq!(texts.iter().filter(|(t, _)| t == "External").count(), 1, "Bruno isn't marked: {texts:?} {bruno_at:?}");
    w.typed(&mut a, vec![enter()]);
    let texts = w.texts(&mut a);
    let (mark, name) = (rect_of(&texts, |t| t == "External").expect("External"), rect_of(&texts, |t| t == "Sam Okafor").expect("Sam"));
    assert!(mark.right() <= name.left() && (mark.center().y - name.center().y).abs() < 6.0, "on the chip: {mark:?} {name:?}");
    let cross = pane_control(&mut w, &mut a, "Remove: Sam Okafor").expect("the chip's cross");
    w.click(&mut a, cross);
    assert!(picked(&a).is_empty(), "the cross takes them away");
    let _ = std::fs::remove_dir_all(dir);
}

/// Where the pane's control named `label` is, in the middle of it.
fn pane_control(w: &mut Win, a: &mut MailApp, label: &str) -> Option<egui::Pos2> {
    w.ctx.enable_accesskit();
    let input = RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SIZE.0 as f32, SIZE.1 as f32))),
        time: Some(w.time.max(w.ctx.input(|i| i.time)) + 0.02),
        ..Default::default()
    };
    w.time = w.time.max(w.ctx.input(|i| i.time)) + 0.02;
    let mut acts = Vec::new();
    let mut out = w.ctx.run_ui(input, |ui| {
        egui::CentralPanel::default().show(ui, |ui| crate::panes::conversations::chat(&a.session, &mut a.ui, ui, &mut acts));
    });
    out.textures_delta.clear();
    let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
    let b = nodes.iter().find(|(_, n)| n.label() == Some(label)).and_then(|(_, n)| n.bounds())?;
    Some(egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32))
}

/// H13: a name the directory gives with markup, a new line and a bidi override is drawn as its
/// text, on one line, without the override: among the people found and on the chip.
#[test]
fn a_name_with_markup_or_bidi_is_drawn_plain() {
    let (mut a, dir, mut w, _) = Win::app("teams-new-markup", vec![], true);
    let hostile = person(ANA, "Ana\n<b>Lima</b>\u{202E}", "ana@contoso.example\u{202E}", false);
    a.ui.conversations.new_chat.as_mut().expect("open").to = "ana".into();
    a.session.teams.people = Some(TeamsPeople { query: "ana".into(), people: vec![hostile], failed: false, waiting: false });
    let check = |texts: &[(String, egui::Rect)], with_address: bool| {
        let name = rect_of(texts, |t| t == "Ana <b>Lima</b>").unwrap_or_else(|| panic!("the name as its text: {texts:?}"));
        assert!(name.height() < 24.0, "on one line: {name:?}");
        assert!(!texts.iter().any(|(t, _)| t.contains('\u{202E}') || t.contains('\n')), "{texts:?}");
        assert_eq!(has_text(texts, "ana@contoso.example"), with_address, "{texts:?}");
    };
    check(&w.texts(&mut a), true);
    w.typed(&mut a, vec![enter()]);
    assert_eq!(pane_of(&a).picked.len(), 1);
    check(&w.texts(&mut a), false);
    let _ = std::fs::remove_dir_all(dir);
}

/// A group name is asked for from two people on, not for one.
#[test]
fn the_group_name_shows_from_two_people() {
    let (mut a, dir, mut w, _) = Win::app("teams-new-group-name", vec![], true);
    w.pick(&mut a, ana());
    assert!(!has_text(&w.texts(&mut a), "Group name (optional)"));
    w.pick(&mut a, bruno());
    assert!(has_text(&w.texts(&mut a), "Group name (optional)"));
    w.click_text(&mut a, "Group name (optional)");
    w.typed(&mut a, vec![Event::Text("Launch".into())]);
    assert_eq!(pane_of(&a).topic, "Launch");
    a.ui.conversations.new_chat.as_mut().expect("open").picked.pop();
    let texts = w.texts(&mut a);
    assert!(!has_text(&texts, "Group name (optional)") && !has_text(&texts, "Launch"), "{texts:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Ana and Bruno picked, "Launch" named, "hello" bold on "hel" and a file: Enter asks for the
/// conversation with them, its name and that first message; nothing is sent to the open chat, and
/// what was written stays until the conversation is made.
#[test]
fn enter_creates_with_the_first_message() {
    let (mut a, dir, mut w, _) = Win::app("teams-new-create", vec![], true);
    w.pick(&mut a, ana());
    w.pick(&mut a, bruno());
    w.click_text(&mut a, "Group name (optional)");
    w.typed(&mut a, vec![Event::Text("Launch".into())]);
    w.click_text(&mut a, "Write a message");
    let bold = || key(Key::B, Modifiers::COMMAND);
    w.typed(&mut a, vec![bold(), Event::Text("hel".into()), bold(), Event::Text("lo".into())]);
    let files = dir.join("files");
    std::fs::create_dir_all(&files).unwrap();
    let pdf = files.join("a.pdf");
    std::fs::write(&pdf, b"%PDF-1.7").unwrap();
    add_files(draft(&mut a.ui.conversations, NEW), vec![pdf.clone()]);
    let acts = w.typed(&mut a, vec![enter()]);
    let pdf = pdf.to_string_lossy().into_owned();
    let wanted = json!({"people": [ANA, BRUNO], "topic": "Launch", "text": "hello", "bold": [[0, 3]], "italic": [], "files": [pdf]});
    assert_eq!(creates(&acts), [wanted]);
    assert!(asked(&acts, "teams.send").is_empty(), "nothing to the open chat");
    assert_eq!(in_new_box(&a), "hello", "kept until it's made");
    let _ = std::fs::remove_dir_all(dir);
}

/// One person, no group name sent; no one picked: nothing is created, the box says so and keeps
/// the text.
#[test]
fn no_one_picked_sends_nothing_and_says_so() {
    let (mut a, dir, mut w, _) = Win::app("teams-new-no-one", vec![], true);
    w.write(&mut a, "hello");
    let acts = w.typed(&mut a, vec![enter()]);
    assert!(creates(&acts).is_empty());
    let texts = w.texts(&mut a);
    let (said, hint) = (rect_of(&texts, |t| t == "Choose who to write to first"), rect_of(&texts, |t| t == "hello"));
    assert!(said.zip(hint).is_some_and(|(s, h)| s.top() >= h.bottom()), "under the box: {texts:?}");
    assert_eq!(in_new_box(&a), "hello");
    w.pick(&mut a, ana());
    assert!(!has_text(&w.texts(&mut a), "Choose who to write to first"), "someone picked: no longer said");
    w.click_text(&mut a, "hello");
    let acts = w.typed(&mut a, vec![enter()]);
    assert_eq!(creates(&acts), [json!({"people": [ANA], "text": "hello", "bold": [], "italic": []})]);
    let _ = std::fs::remove_dir_all(dir);
}

/// While the conversation is created: "Creating the conversation…", and what's typed, clicked or
/// pressed changes nothing (no second creation).
#[test]
fn while_creating_everything_waits() {
    let (mut a, dir, mut w, jobs) = Win::app("teams-new-waiting", vec![], true);
    w.pick(&mut a, ana());
    w.write(&mut a, "hello");
    assert_eq!(w.send(&mut a).len(), 1);
    assert!(matches!(a.session.teams.creating, Some(TeamsCreating::Waiting { .. })));
    // Enter again at once, the box still with the keyboard: nothing more is asked.
    assert!(creates(&w.pane(&mut a, 0.02, vec![enter()]).0).is_empty(), "a second Enter");
    // The jobs go out at the end of a frame of the window.
    frame(&w.ctx, &mut a, vec![]);
    assert!(jobs.borrow().iter().any(|j| matches!(j, Job::Teams { job: teams::Job::Create { .. } })), "asked of the worker");
    let texts = w.texts(&mut a);
    assert!(has_text(&texts, "Creating the conversation…") && has_text(&texts, "hello"), "{texts:?}");
    let acts = w.typed(&mut a, vec![Event::Text("x".into()), key(Key::Backspace, Modifiers::NONE), enter()]);
    assert!(creates(&acts).is_empty());
    assert_eq!((in_new_box(&a), picked(&a)), ("hello".to_string(), vec!["Ana Lima".to_string()]));
    w.click_text(&mut a, "Ana Lima");
    w.typed(&mut a, vec![Event::Text("bru".into())]);
    w.pane(&mut a, 0.5, vec![]);
    assert_eq!(pane_of(&a).to, "", "To: waits too");
    let _ = std::fs::remove_dir_all(dir);
}

/// Review Focus 6: Teams can't be reached when the conversation is created: the pane says it
/// wasn't, and why, the person, the text and the file still there; sent again offline, the engine
/// refuses it and the box says so in the same words, nothing lost.
#[test]
fn a_failure_keeps_the_people_and_the_text() {
    let (mut a, dir, mut w, _) = Win::app("teams-new-failed", vec![], true);
    w.pick(&mut a, ana());
    w.write(&mut a, "hello");
    let pdf = dir.join("a.pdf");
    std::fs::write(&pdf, b"%PDF-1.7").unwrap();
    add_files(draft(&mut a.ui.conversations, NEW), vec![pdf]);
    w.send(&mut a);
    let Some(TeamsCreating::Waiting { local }) = a.session.teams.creating.clone() else { panic!("waiting") };
    a.session.teams_apply(teams::Event::NotCreated { local, why: NotCreated::Offline, code: None });
    w.settled(&mut a);
    let (_, shapes) = w.pane(&mut a, 0.02, vec![]);
    let texts = texts_of(&shapes);
    for t in ["Teams didn't create the conversation", "Teams can't be reached: try again", "Ana Lima", "hello", "a.pdf"] {
        assert!(has_text(&texts, t), "{t}: {texts:?}");
    }
    assert_eq!(colour_of(&shapes, "Teams didn't create the conversation"), Some(a.ui.tokens.danger));
    a.session.teams.creating = Some(TeamsCreating::Failed { why: NotCreated::TooMany, code: None });
    assert!(has_text(&w.texts(&mut a), "Another conversation is being created"));
    a.session.teams_apply(teams::Event::Status { status: Status::Offline });
    w.click_text(&mut a, "hello");
    assert_eq!(w.send(&mut a).len(), 1, "asked again");
    let said = a.ui.conversations.drafts.get(NEW).and_then(|d| d.said());
    assert_eq!(said, Some("Teams can't be reached: try again"));
    let texts = w.texts(&mut a);
    for t in ["Ana Lima", "hello", "a.pdf"] {
        assert!(has_text(&texts, t), "nothing lost: {t}: {texts:?}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// A creation that failed before the pane was opened (left by one closed since) isn't said in a new
/// one; nor once the engine refuses what this one asks.
#[test]
fn a_failure_from_before_isnt_said() {
    let (mut a, dir, mut w, _) = Win::app("teams-new-stale", vec![], true);
    a.session.teams.creating = Some(TeamsCreating::Failed { why: NotCreated::Offline, code: None });
    assert!(!has_text(&w.texts(&mut a), "Teams didn't create the conversation"));
    a.ui.conversations.new_chat.as_mut().expect("open").picked.push(ana());
    w.write(&mut a, "hello");
    w.send(&mut a);
    let texts = w.texts(&mut a);
    assert_eq!(texts.iter().filter(|(t, _)| t == "Teams didn't create the conversation").count(), 1, "the refusal only: {texts:?}");
    assert!(!has_text(&texts, "Teams can't be reached: try again"), "{texts:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// P-15: the engine's refusals of a creation are said under the box in the window's words: a
/// person the directory didn't give, and a creation already going.
#[test]
fn the_engines_refusals_of_a_creation_are_said() {
    let (mut a, dir, mut w, _) = Win::app("teams-new-refusals", vec![], true);
    let said = |a: &MailApp| a.ui.conversations.drafts.get(NEW).and_then(|d| d.said());
    // Picked without the directory having given her.
    a.ui.conversations.new_chat.as_mut().expect("open").picked.push(ana());
    w.write(&mut a, "hello");
    assert_eq!(w.send(&mut a).len(), 1);
    assert_eq!(said(&a), Some("Teams didn't create the conversation"));
    assert!(a.session.teams.creating.is_none());
    assert_eq!((in_new_box(&a), picked(&a)), ("hello".to_string(), vec!["Ana Lima".to_string()]));
    answered(&mut a, "bru", vec![bruno()]);
    a.run("teams.create", json!({"people": [BRUNO], "text": "first"}));
    assert!(matches!(a.session.teams.creating, Some(TeamsCreating::Waiting { .. })));
    a.act(Action::Run("teams.create", json!({"people": [BRUNO], "text": "hello"})));
    assert_eq!(said(&a), Some("Teams didn't create the conversation"), "one at a time");
    assert_eq!(in_new_box(&a), "hello");
    let _ = std::fs::remove_dir_all(dir);
}

/// P-14: the conversation is made but its first message refused (the outbox full): the pane stays
/// with the person, the text and the file, and says Teams refused it.
#[test]
fn a_first_message_refused_keeps_what_was_typed() {
    let (mut a, dir, mut w, _) = Win::app("teams-new-first-refused", vec![], true);
    w.pick(&mut a, ana());
    w.write(&mut a, "hello");
    let pdf = dir.join("a.pdf");
    std::fs::write(&pdf, b"%PDF-1.7").unwrap();
    add_files(draft(&mut a.ui.conversations, NEW), vec![pdf]);
    w.send(&mut a);
    let Some(TeamsCreating::Waiting { local }) = a.session.teams.creating.clone() else { panic!("waiting") };
    a.session.teams_apply(teams::Event::Unsent { count: OUTBOX_MAX as u32 });
    a.session.teams_apply(teams::Event::Chats { rows: vec![group_row(), one_to_one(WITH_ANA, "Ana Lima", ANA, NOW)] });
    a.session.teams_apply(teams::Event::Created { local, chat: WITH_ANA.into() });
    for _ in 0..3 {
        frame(&w.ctx, &mut a, vec![]);
    }
    assert!(a.ui.conversations.new_chat.is_some(), "the pane stays");
    let texts = w.texts(&mut a);
    for t in ["Teams didn't create the conversation", "Teams refused it", "Ana Lima", "hello", "a.pdf"] {
        assert!(has_text(&texts, t), "{t}: {texts:?}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// The conversation made: the pane closes, its draft goes, and the chat shows in its place.
#[test]
fn created_closes_the_pane_and_shows_the_chat() {
    let (mut a, dir, mut w, _) = Win::app("teams-new-created", vec![], true);
    w.pick(&mut a, ana());
    w.write(&mut a, "hello");
    w.send(&mut a);
    let Some(TeamsCreating::Waiting { local }) = a.session.teams.creating.clone() else { panic!("waiting") };
    a.session.teams_apply(teams::Event::Chats { rows: vec![group_row(), one_to_one(WITH_ANA, "Ana Lima", ANA, NOW)] });
    a.session.teams_apply(teams::Event::Created { local, chat: WITH_ANA.into() });
    let texts = settled_at(&w.ctx, &mut a, SIZE);
    assert!(a.ui.conversations.new_chat.is_none(), "closed");
    assert!(!a.ui.conversations.drafts.contains_key(NEW), "its draft gone");
    assert_eq!(a.session.teams.open.as_deref(), Some(WITH_ANA));
    assert!(!has_text(&texts, "New conversation") && has_text(&texts, "Ana Lima"), "{texts:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Review Focus 5: Ana already chatted with: Enter opens that chat with the message, and no
/// conversation is asked of the worker.
#[test]
fn a_person_already_chatted_with_opens_that_chat() {
    let (mut a, dir, mut w, jobs) = Win::app("teams-new-existing", vec![one_to_one(WITH_ANA, "Ana Lima", ANA, NOW)], true);
    w.pick(&mut a, ana());
    w.write(&mut a, "hello");
    w.send(&mut a);
    settled_at(&w.ctx, &mut a, SIZE);
    assert!(a.ui.conversations.new_chat.is_none(), "closed");
    assert_eq!(a.session.teams.open.as_deref(), Some(WITH_ANA));
    assert!(!jobs.borrow().iter().any(|j| matches!(j, Job::Teams { job: teams::Job::Create { .. } })), "nothing created");
    let _ = std::fs::remove_dir_all(dir);
}

/// Esc closes it and lets go of what was typed; the list's search stays as it was. Cancel the
/// same.
#[test]
fn esc_closes_it_and_keeps_the_lists_search() {
    let (mut a, dir, mut w, _) = Win::app("teams-new-esc", vec![], true);
    a.ui.conversations.query = "agenda".into();
    w.write(&mut a, "hello");
    frame(&w.ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(a.ui.conversations.new_chat.is_none(), "closed");
    assert_eq!(a.ui.conversations.query, "agenda", "the search kept");
    assert!(!a.ui.conversations.drafts.contains_key(NEW), "what was typed let go");
    crate::conversations::open_new_chat(&a.session, &mut a.ui.conversations);
    assert_eq!(in_new_box(&a), "", "a new one starts empty");
    w.click_text(&mut a, "Cancel");
    assert!(a.ui.conversations.new_chat.is_none(), "Cancel closes it");
    assert_eq!(a.ui.conversations.query, "agenda");
    let _ = std::fs::remove_dir_all(dir);
}

/// The twentieth person is picked, and then "To:" says no more are: the twenty-first isn't added.
#[test]
fn the_twenty_first_person_isnt_added() {
    let (mut a, dir, mut w, _) = Win::app("teams-new-twenty", vec![], true);
    a.ui.conversations.new_chat.as_mut().expect("open").picked = (1..NEW_CHAT_PEOPLE).map(many).collect();
    w.look_up(&mut a, "person", vec![many(NEW_CHAT_PEOPLE), many(NEW_CHAT_PEOPLE + 1)]);
    w.settled(&mut a);
    w.typed(&mut a, vec![enter()]);
    assert_eq!(pane_of(&a).picked.len(), NEW_CHAT_PEOPLE);
    let texts = w.texts(&mut a);
    assert!(has_text(&texts, "At most 20 people") && !has_text(&texts, "A name or an address"), "{texts:?}");
    answered(&mut a, "person", vec![many(NEW_CHAT_PEOPLE + 1)]);
    a.ui.conversations.new_chat.as_mut().expect("open").to = "person".into();
    w.typed(&mut a, vec![enter()]);
    assert_eq!(pane_of(&a).picked.len(), NEW_CHAT_PEOPLE, "no twenty-first");
    assert!(!has_text(&w.texts(&mut a), &format!("Person {}", NEW_CHAT_PEOPLE + 1)));
    let _ = std::fs::remove_dir_all(dir);
}

/// The pane in light and dark: empty, people found, two chips with a group name, being created,
/// and not created.
#[test]
fn the_new_conversation_in_light_and_dark() {
    let (mut a, dir, mut w, _) = Win::app("teams-new-frames", vec![], true);
    let pictures = std::env::var_os("QUATRELLE_FRAME_PNG").map(PathBuf::from);
    let mut tex = Textures::default();
    let mut painted = |a: &mut MailApp, ctx: &egui::Context, name: &str, wanted: &[&str]| {
        let mut light = Vec::new();
        for (dark, theme) in [(false, "light"), (true, "dark")] {
            a.ui.dark = Some(dark);
            for _ in 0..6 {
                frame_with_size(ctx, a, &mut tex, vec![], None, SIZE);
            }
            let png = pictures.as_ref().map(|d| d.join(format!("teams-new-chat-{name}-{theme}.png")));
            let (_, shapes) = frame_with_size(ctx, a, &mut tex, vec![], png, SIZE);
            let mut texts: Vec<String> = texts_of(&shapes).into_iter().map(|(t, _)| t).collect();
            for want in wanted {
                assert!(texts.iter().any(|t| t == want), "{name}, {theme}: {want}: {texts:?}");
            }
            texts.sort();
            if dark {
                assert_eq!(texts, light, "{name}: the dark pass draws the light one's texts");
            } else {
                light = texts;
            }
        }
        a.ui.dark = Some(false);
    };
    painted(&mut a, &w.ctx, "empty", &["New conversation", "To:", "A name or an address", "Write a message"]);
    w.look_up(&mut a, "a", vec![ana(), sam()]);
    painted(&mut a, &w.ctx, "found", &["Ana Lima", "ana@contoso.example", "External", "Sam Okafor"]);
    w.typed(&mut a, vec![enter()]);
    w.pick(&mut a, sam());
    let n = a.ui.conversations.new_chat.as_mut().expect("open");
    n.topic = "Launch".into();
    painted(&mut a, &w.ctx, "group", &["Ana Lima", "Sam Okafor", "External", "Launch"]);
    w.write(&mut a, "hello");
    assert_eq!(w.send(&mut a).len(), 1);
    painted(&mut a, &w.ctx, "creating", &["Creating the conversation…", "hello", "Launch"]);
    let Some(TeamsCreating::Waiting { local }) = a.session.teams.creating.clone() else { panic!("waiting") };
    a.session.teams_apply(teams::Event::NotCreated { local, why: NotCreated::Refused, code: Some("HTTP 403 Forbidden".into()) });
    painted(&mut a, &w.ctx, "failed", &["Teams didn't create the conversation", "Teams refused it (HTTP 403 Forbidden)", "Launch", "hello"]);
    let _ = std::fs::remove_dir_all(dir);
}

/// Fix round 1, Important 1: Esc meant for the emoji picker open over the new conversation's box
/// closes the picker only: the people, the text and the pane stay. A second Esc closes the pane.
#[test]
fn esc_closes_the_emoji_picker_not_the_pane() {
    let (mut a, dir, mut w, _) = Win::app("teams-new-esc-picker", vec![], true);
    let size = egui::vec2(SIZE.0 as f32, SIZE.1 as f32);
    w.pick(&mut a, ana());
    w.write(&mut a, "hello");
    let emoji = window_control(&w.ctx, &mut a, "Emoji").expect("the box's emoji button");
    click_at(&w.ctx, &mut a, emoji, 1, size);
    assert!(egui::Popup::is_any_open(&w.ctx), "the picker open");
    frame(&w.ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    frame(&w.ctx, &mut a, vec![]);
    assert!(!egui::Popup::is_any_open(&w.ctx), "the picker closed");
    assert!(a.ui.conversations.new_chat.is_some(), "the pane stays");
    assert_eq!((picked(&a), in_new_box(&a)), (vec!["Ana Lima".to_string()], "hello".to_string()), "nothing lost");
    frame(&w.ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(a.ui.conversations.new_chat.is_none(), "then Esc closes it");
    let _ = std::fs::remove_dir_all(dir);
}

/// Fix round 1, Important 1: Esc while people are listed under "To:" empties "To:" (the list goes,
/// the keyboard stays there); the pane, its chips and its text stay. Once "To:" is empty, Esc
/// closes the pane.
#[test]
fn esc_with_people_listed_empties_to_first() {
    let (mut a, dir, mut w, _) = Win::app("teams-new-esc-list", vec![], true);
    w.pick(&mut a, bruno());
    w.write(&mut a, "hello");
    w.look_up(&mut a, "ana", vec![ana()]);
    assert!(has_text(&w.texts(&mut a), "ana@contoso.example"), "Ana listed");
    frame(&w.ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(a.ui.conversations.new_chat.is_some(), "the pane stays");
    assert_eq!((pane_of(&a).to.as_str(), picked(&a), in_new_box(&a)), ("", vec!["Bruno Reis".to_string()], "hello".to_string()));
    let texts = settled_at(&w.ctx, &mut a, SIZE);
    assert!(!has_text(&texts, "ana@contoso.example"), "the list gone: {texts:?}");
    frame(&w.ctx, &mut a, vec![Event::Text("b".into())]);
    assert_eq!(pane_of(&a).to, "b", "the keyboard still in To:");
    frame(&w.ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert_eq!(pane_of(&a).to, "", "emptied again");
    frame(&w.ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(a.ui.conversations.new_chat.is_none(), "then Esc closes it");
    let _ = std::fs::remove_dir_all(dir);
}

/// The marks read the app sent.
fn marks_read(jobs: &Shared<Job>) -> usize {
    jobs.borrow().iter().filter(|j| matches!(j, Job::Teams { job: teams::Job::Op { op: teams::Op::MarkRead { .. } } })).count()
}

/// Fix round 1, Important 2 (ruled): the chat the pane hides isn't on screen for the engine: its
/// news are announced, it isn't marked read. Its row clicked in the list, the chat takes the pane's
/// place and is marked read; New conversation brings the pane back as it was (the people, the group
/// name, the text, the file).
#[test]
fn the_chat_under_the_pane_is_away_and_a_row_takes_its_place() {
    let (mut a, dir, mut w, jobs) = Win::app("teams-new-hidden", vec![], true);
    let shown = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    a.services = Box::new(Recorder(shown.clone()));
    let size = egui::vec2(SIZE.0 as f32, SIZE.1 as f32);
    w.pick(&mut a, ana());
    w.pick(&mut a, bruno());
    w.click_text(&mut a, "Group name (optional)");
    w.typed(&mut a, vec![Event::Text("Launch".into())]);
    w.write(&mut a, "hello");
    let pdf = dir.join("a.pdf");
    std::fs::write(&pdf, b"%PDF-1.7").unwrap();
    add_files(draft(&mut a.ui.conversations, NEW), vec![pdf]);
    for _ in 0..2 {
        frame(&w.ctx, &mut a, vec![]);
    }
    jobs.borrow_mut().clear();
    a.session.teams_apply(teams::Event::Chats { rows: vec![ChatRow { unread: 1, ..group_row() }] });
    let news = News {
        chat: group_row().id,
        message: "m1".into(),
        sender: "Ana Lima".into(),
        external: false,
        app: false,
        mentions_me: false,
        chat_name: Some("Support team".into()),
        direct: false,
        hidden: false,
        text: "Lunch?".into(),
        at: NOW * 1_000,
    };
    a.session.teams_apply(teams::Event::News { news: vec![news] });
    for _ in 0..2 {
        frame(&w.ctx, &mut a, vec![]);
    }
    assert_eq!(shown.lock().unwrap().as_slice(), [("Ana Lima in Support team".to_string(), "Lunch?".to_string())], "announced");
    assert_eq!(marks_read(&jobs), 0, "not marked read under the pane");
    let row = settled_text_at(&w.ctx, &mut a, "Support team", size).expect("its row in the list");
    assert!(row.x < 400.0, "the list's row: {row:?}");
    click_at(&w.ctx, &mut a, row, 1, size);
    let texts = settled_at(&w.ctx, &mut a, SIZE);
    assert!(a.ui.conversations.new_chat.is_none(), "the chat takes the pane's place");
    assert!(has_text(&texts, "Members: 4") && !has_text(&texts, "To:"), "{texts:?}");
    assert!(marks_read(&jobs) >= 1, "marked read once shown");
    frame(&w.ctx, &mut a, vec![key(Key::N, Modifiers::COMMAND)]);
    let texts = settled_at(&w.ctx, &mut a, SIZE);
    assert_eq!((picked(&a), pane_of(&a).topic.as_str()), (vec!["Ana Lima".to_string(), "Bruno Reis".to_string()], "Launch"));
    assert_eq!(in_new_box(&a), "hello");
    for t in ["Ana Lima", "Bruno Reis", "Launch", "hello", "a.pdf"] {
        assert!(has_text(&texts, t), "back as it was: {t}: {texts:?}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// Fix round 1, Minor 1: Enter in the box and a click on Cancel in the same frame: the pane closes
/// and nothing is created.
#[test]
fn enter_and_cancel_in_the_same_frame_create_nothing() {
    let (mut a, dir, mut w, _) = Win::app("teams-new-cancel-enter", vec![], true);
    w.pick(&mut a, ana());
    w.write(&mut a, "hello");
    let cancel = rect_of(&w.texts(&mut a), |t| t == "Cancel").expect("Cancel").center();
    let press = |pressed| Event::PointerButton { pos: cancel, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    let mut acts = w.pane(&mut a, 0.02, vec![Event::PointerMoved(cancel)]).0;
    acts.extend(w.pane(&mut a, 0.02, vec![press(true)]).0);
    acts.extend(w.pane(&mut a, 0.02, vec![press(false), enter()]).0);
    assert!(a.ui.conversations.new_chat.is_none(), "Cancel closes it");
    assert!(creates(&acts).is_empty(), "nothing created");
    let _ = std::fs::remove_dir_all(dir);
}
