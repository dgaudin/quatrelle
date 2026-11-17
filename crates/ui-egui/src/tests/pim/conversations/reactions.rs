// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: reacting to a message, and the bar over it. The pointer on a message shows the six
//! quick reactions and Copy; on one's own, Edit (when the box can hold it) and Delete; in a chat
//! left, Copy alone; none on a system line, a message deleted, unshowable or being deleted. Delete
//! asks first (Esc or Cancel is a no, the search kept). A chip's click sets or takes back one's
//! own reaction, never on a kind that isn't an emoji nor in a chat left; a reaction going out
//! counts at once. Copy puts the words on the clipboard; Esc right after Edit lets the edit go.
//! With `QUATRELLE_FRAME_PNG` set to a folder, the bar, the chips and the question are painted
//! there, light and dark.

use quatrelle_engine::model::pim::teams::{Attachment, ChatMessage, MessageKind, OutKind, OutState, Outgoing, QUICK_REACTIONS, Reaction, Sender};

use super::chat::*;
use super::writing::{SIZE, asked, in_box, writing_app};
use super::*;
use crate::Action;

/// What a frame of the pane alone drew and asked for.
pub(super) struct Seen {
    pub(super) acts: Vec<Action>,
    pub(super) texts: Vec<(String, egui::Rect)>,
    /// The controls' accessible names, and where they are.
    pub(super) labels: Vec<(String, egui::Rect)>,
    pub(super) shapes: Vec<egui::Shape>,
    /// What was put on the clipboard.
    pub(super) copied: Vec<String>,
}

impl Seen {
    pub(super) fn text(&self, t: &str) -> Option<egui::Rect> {
        rect_of(&self.texts, |x| x == t)
    }

    pub(super) fn label(&self, name: &str) -> Option<egui::Rect> {
        self.labels.iter().find(|(l, _)| l == name).map(|(_, r)| *r)
    }

    /// Whether the bar is drawn: its six reactions.
    pub(super) fn bar(&self) -> bool {
        QUICK_REACTIONS.iter().any(|e| has_text(&self.texts, e))
    }
}

/// The commands `acts` asked for, by id (an `Action` has no `Debug`).
pub(super) fn ids(acts: &[Action]) -> Vec<&'static str> {
    acts.iter()
        .map(|x| match x {
            Action::Run(id, _) => *id,
            _ => "(not a command)",
        })
        .collect()
}

/// A frame of the pane alone with `events`.
pub(super) fn seen(ctx: &egui::Context, a: &mut MailApp, events: Vec<Event>) -> Seen {
    ctx.enable_accesskit();
    let input = RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SIZE.0 as f32, SIZE.1 as f32))),
        events,
        ..Default::default()
    };
    let mut acts = Vec::new();
    let mut out = ctx.run_ui(input, |ui| {
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
    let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
    let labels = nodes
        .iter()
        .filter_map(|(_, n)| {
            let b = n.bounds()?;
            Some((n.label()?.to_string(), egui::Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32))))
        })
        .collect();
    let copied = out
        .platform_output
        .commands
        .iter()
        .filter_map(|c| match c {
            egui::OutputCommand::CopyText(t) => Some(t.clone()),
            _ => None,
        })
        .collect();
    Seen { acts, texts: texts_of(&shapes), labels, shapes, copied }
}

/// The pane settled: its last frame.
pub(super) fn settle(ctx: &egui::Context, a: &mut MailApp) -> Seen {
    for _ in 0..6 {
        seen(ctx, a, vec![]);
    }
    seen(ctx, a, vec![])
}

/// The pointer gone from the window (a bar it left shown goes) and the pane settled, then the
/// pointer over `text`: the frame after.
pub(super) fn hover(ctx: &egui::Context, a: &mut MailApp, text: &str) -> Seen {
    seen(ctx, a, vec![Event::PointerGone]);
    let at = settle(ctx, a).text(text).unwrap_or_else(|| panic!("{text} drawn")).center();
    seen(ctx, a, vec![Event::PointerMoved(at)]);
    seen(ctx, a, vec![])
}

/// A click at `at` on the pane: what its frames asked for and copied, with the last one's
/// drawing.
pub(super) fn click(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2) -> Seen {
    let press = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    let (mut acts, mut copied) = (Vec::new(), Vec::new());
    let mut last = None;
    for events in [vec![Event::PointerMoved(at)], vec![press(true)], vec![press(false)]] {
        let mut s = seen(ctx, a, events);
        acts.append(&mut s.acts);
        copied.append(&mut s.copied);
        last = Some(s);
    }
    let last = last.unwrap();
    Seen { acts, copied, ..last }
}

/// The smallest filled or stroked rectangle drawn around `inner`'s middle: (its stroke's colour,
/// its fill).
pub(super) fn frame_around(shapes: &[egui::Shape], inner: egui::Rect) -> Option<(egui::Color32, egui::Color32)> {
    shapes
        .iter()
        .filter_map(|s| match s {
            egui::Shape::Rect(r) if r.rect.contains(inner.center()) && r.rect.width() >= inner.width() => Some(r),
            _ => None,
        })
        .min_by(|x, y| x.rect.area().total_cmp(&y.rect.area()))
        .map(|r| (r.stroke.color, r.fill))
}

/// How opaque `text` is drawn: the most of its glyphs' alpha.
pub(super) fn alpha_of(shapes: &[egui::Shape], text: &str) -> Option<u8> {
    shapes.iter().find_map(|s| match s {
        egui::Shape::Text(t) if t.galley.text() == text => {
            t.galley.rows.iter().flat_map(|r| r.row.visuals.mesh.vertices.iter()).map(|v| v.color.a()).max()
        }
        _ => None,
    })
}

pub(super) fn ana(id: &str, secs: i64, text: &str) -> ChatMessage {
    said(id, user("u-ana", "Ana Lima"), secs, para(vec![words(text)]))
}

pub(super) fn mine(id: &str, secs: i64, text: &str) -> ChatMessage {
    said(id, user(ME, "Riley Moreno"), secs, para(vec![words(text)]))
}

pub(super) fn esc() -> Event {
    key(Key::Escape, Modifiers::NONE)
}

/// The open chat's changes going out, as the worker says them.
pub(super) fn outbox(a: &mut MailApp, items: Vec<Outgoing>) {
    a.session.teams_apply(teams::Event::Outbox { chat: CHAT.into(), items, took: None });
}

/// A change going out numbered `seq`, of `what`, in `state`; an edit's words `text`.
pub(super) fn change(seq: u64, what: OutKind, text: &str, state: OutState) -> Outgoing {
    let body = if text.is_empty() { teams::ChatText::default() } else { para(vec![words(text)]) };
    Outgoing { seq, local: format!("l{seq}"), chat: CHAT.into(), body, mentions: Vec::new(), what, state, parts: Vec::new() }
}

fn editing(a: &MailApp) -> Option<String> {
    a.ui.conversations.drafts.get(CHAT).and_then(|d| d.editing().map(str::to_string))
}

/// Ana's question: the bar offers the six quick reactions and Copy, drawn on the message's top
/// edge (none before the pointer comes, nor Edit or Delete: it isn't the user's); a click on 😮
/// sets that reaction.
#[test]
fn hovering_a_message_offers_six_reactions_and_copy() {
    let (mut a, dir) = writing_app("teams-bar-six", vec![ana("m1", NOW - 60, "Can you check the figures?")]);
    let ctx = egui::Context::default();
    let before = settle(&ctx, &mut a);
    assert!(!before.bar() && before.label("Copy").is_none(), "no bar before the pointer comes");
    let s = hover(&ctx, &mut a, "Can you check the figures?");
    for e in QUICK_REACTIONS {
        assert!(has_text(&s.texts, e), "{e}: {:?}", s.texts);
    }
    assert!(s.label("Copy").is_some(), "{:?}", s.labels);
    assert!(s.label("Edit").is_none() && s.label("Delete").is_none(), "not the user's: {:?}", s.labels);
    // Over the bubble's top edge, nothing moved.
    let words = s.text("Can you check the figures?").unwrap();
    let wow = s.text("\u{1F62E}").unwrap();
    assert!(wow.top() < words.top() && wow.bottom() > words.top() - 30.0, "on the top edge: {wow:?} {words:?}");
    assert_eq!(before.text("Can you check the figures?"), Some(words), "nothing moved");
    let c = click(&ctx, &mut a, wow.center());
    assert_eq!(asked(&c.acts, "teams.react"), vec![json!({"chat": CHAT, "message": "m1", "emoji": "\u{1F62E}"})]);
    let _ = std::fs::remove_dir_all(dir);
}

/// The user's own "hello" offers Edit and Delete besides Copy; their message with a file, which
/// the box can't hold, Delete alone. Edit asks for the edit (`edit_asked`), and a frame later the
/// box edits it.
#[test]
fn ones_own_message_offers_edit_and_delete() {
    let mut with_file = mine("m4", NOW - 30, "Here it is");
    with_file.attachments = vec![Attachment::File { id: "f1".into(), name: "figures.xlsx".into(), url: String::new(), size: Some(2048) }];
    let (mut a, dir) = writing_app("teams-bar-own", vec![mine("m2", NOW - 300, "hello"), with_file]);
    let ctx = egui::Context::default();
    let s = hover(&ctx, &mut a, "hello");
    for name in ["Copy", "Edit", "Delete"] {
        assert!(s.label(name).is_some(), "{name}: {:?}", s.labels);
    }
    assert!(s.bar());
    let s = hover(&ctx, &mut a, "Here it is");
    assert!(s.label("Delete").is_some() && s.label("Copy").is_some(), "{:?}", s.labels);
    assert!(s.label("Edit").is_none(), "a file: the box can't hold it: {:?}", s.labels);
    let s = hover(&ctx, &mut a, "hello");
    let edit = s.label("Edit").expect("Edit").center();
    click(&ctx, &mut a, edit);
    assert_eq!(a.ui.conversations.edit_asked.as_deref(), Some("m2"));
    let s = seen(&ctx, &mut a, vec![]);
    assert!(has_text(&s.texts, "Editing message"), "{:?}", s.texts);
    assert_eq!(in_box(&a), "hello");
    let _ = std::fs::remove_dir_all(dir);
}

/// Delete asks "Delete this message?" first: Cancel does nothing, nor does Esc; Delete asks the
/// engine to delete it.
#[test]
fn delete_asks_first() {
    let (mut a, dir) = writing_app("teams-bar-delete", vec![mine("m2", NOW - 300, "hello")]);
    let ctx = egui::Context::default();
    let ask = |a: &mut MailApp| {
        let s = hover(&ctx, a, "hello");
        let c = click(&ctx, a, s.label("Delete").expect("Delete").center());
        assert!(asked(&c.acts, "teams.delete").is_empty(), "nothing before the answer");
        assert_eq!(a.ui.conversations.deleting.as_deref(), Some("m2"));
        let s = seen(&ctx, a, vec![]);
        assert!(has_text(&s.texts, "Delete this message?"), "{:?}", s.texts);
        assert!(has_text(&s.texts, "It will be deleted for everyone in this conversation."), "{:?}", s.texts);
        assert!(!s.bar(), "no bar under the question");
        s
    };
    let s = ask(&mut a);
    let c = click(&ctx, &mut a, s.text("Cancel").expect("Cancel").center());
    assert!(c.acts.is_empty(), "{:?}", ids(&c.acts));
    assert_eq!(a.ui.conversations.deleting, None);
    assert!(!has_text(&seen(&ctx, &mut a, vec![]).texts, "Delete this message?"));
    let s = ask(&mut a);
    let c = click(&ctx, &mut a, s.text("Delete").expect("the question's Delete").center());
    assert_eq!(asked(&c.acts, "teams.delete"), vec![json!({"chat": CHAT, "message": "m2"})]);
    assert_eq!(a.ui.conversations.deleting, None);
    ask(&mut a);
    let s = seen(&ctx, &mut a, vec![esc()]);
    assert!(s.acts.is_empty(), "{:?}", ids(&s.acts));
    assert_eq!(a.ui.conversations.deleting, None);
    let _ = std::fs::remove_dir_all(dir);
}

/// In the whole window, Esc on the question is its no: the conversations' search stays.
#[test]
fn esc_on_the_delete_question_keeps_the_search() {
    let (mut a, dir) = writing_app("teams-bar-delete-esc", vec![mine("m2", NOW - 300, "hello")]);
    a.ui.conversations.query = "agenda".into();
    a.run("teams.search", json!({"query": "agenda"}));
    let ctx = egui::Context::default();
    let size = egui::vec2(SIZE.0 as f32, SIZE.1 as f32);
    assert!(settled_text_at(&ctx, &mut a, "hello", size).is_some());
    // As the bar's Delete asks.
    a.ui.conversations.deleting = Some("m2".into());
    assert!(settled_text_at(&ctx, &mut a, "Delete this message?", size).is_some());
    frame_sized(&ctx, &mut a, vec![esc()], size);
    frame_sized(&ctx, &mut a, vec![], size);
    assert_eq!(a.ui.conversations.deleting, None);
    assert_eq!(a.ui.conversations.query, "agenda", "the search kept");
    assert!(!a.session.teams.outgoing.iter().any(|o| matches!(o.what, OutKind::Delete { .. })), "nothing deleted");
    let _ = std::fs::remove_dir_all(dir);
}

/// A chip's click sets or takes back the user's own reaction of its kind; a kind that isn't an
/// emoji ("custom") takes none. In a chat the user left, no chip takes a click and the bar offers
/// Copy alone.
#[test]
fn a_chip_click_toggles_ones_reaction() {
    let mut m = ana("m1", NOW - 60, "Lunch at noon");
    m.reactions =
        vec![Reaction { emoji: "\u{1F44D}".into(), users: vec!["u-ana".into()] }, Reaction { emoji: "custom".into(), users: vec!["u-bruno".into()] }];
    let (mut a, dir) = writing_app("teams-chips-click", vec![m.clone()]);
    let ctx = egui::Context::default();
    let s = settle(&ctx, &mut a);
    let c = click(&ctx, &mut a, s.text("\u{1F44D} 1").expect("the chip").center());
    assert_eq!(asked(&c.acts, "teams.react"), vec![json!({"chat": CHAT, "message": "m1", "emoji": "\u{1F44D}"})]);
    let s = settle(&ctx, &mut a);
    let c = click(&ctx, &mut a, s.text("custom 1").expect("the custom chip").center());
    assert!(asked(&c.acts, "teams.react").is_empty(), "{:?}", ids(&c.acts));
    let _ = std::fs::remove_dir_all(dir);

    let (mut a, dir, _) = chat_app("teams-chips-left", ChatRow { left: true, ..group_row() }, TEAMS_LINK, vec![m]);
    let s = settle(&ctx, &mut a);
    let c = click(&ctx, &mut a, s.text("\u{1F44D} 1").expect("the chip").center());
    assert!(c.acts.is_empty(), "a chat left: {:?}", ids(&c.acts));
    let s = hover(&ctx, &mut a, "Lunch at noon");
    assert!(s.label("Copy").is_some(), "{:?}", s.labels);
    assert!(!s.bar() && s.label("Edit").is_none() && s.label("Delete").is_none(), "Copy alone: {:?} {:?}", s.texts, s.labels);
    let _ = std::fs::remove_dir_all(dir);
}

/// A screen reader names each chip by what it shows, its emoji and its count: the one a click
/// takes as well as one of a kind that isn't an emoji.
#[test]
fn chips_are_named_for_a_screen_reader() {
    let mut m = ana("m1", NOW - 60, "Lunch at noon");
    m.reactions =
        vec![Reaction { emoji: "\u{1F44D}".into(), users: vec!["u-ana".into()] }, Reaction { emoji: "custom".into(), users: vec!["u-bruno".into()] }];
    let (mut a, dir) = writing_app("teams-chips-named", vec![m]);
    let ctx = egui::Context::default();
    let s = settle(&ctx, &mut a);
    for chip in ["\u{1F44D} 1", "custom 1"] {
        let drawn = s.text(chip).unwrap_or_else(|| panic!("{chip} drawn: {:?}", s.texts));
        let named = s.label(chip).unwrap_or_else(|| panic!("{chip} named: {:?}", s.labels));
        assert!(named.contains(drawn.center()), "{chip}: named at {named:?}, drawn at {drawn:?}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// The user's 👍 going out (shown at once, before the worker has it) counts on the chip, which is
/// drawn as theirs: the accent around its soft tint.
#[test]
fn a_reaction_going_out_counts_at_once() {
    let mut m = ana("m1", NOW - 60, "Lunch at noon");
    m.reactions = vec![Reaction { emoji: "\u{1F44D}".into(), users: vec!["u-ana".into()] }];
    let (mut a, dir) = writing_app("teams-chips-going", vec![m]);
    let ctx = egui::Context::default();
    let t = a.ui.tokens;
    let s = settle(&ctx, &mut a);
    let chip = s.text("\u{1F44D} 1").expect("Ana's");
    assert_eq!(frame_around(&s.shapes, chip).map(|f| f.0), Some(t.border), "not the user's");
    a.run("teams.react", json!({"chat": CHAT, "message": "m1", "emoji": "\u{1F44D}"}));
    assert!(a.session.teams.outgoing.iter().any(|o| o.seq == 0 && matches!(o.what, OutKind::React { on: true, .. })), "a provisional");
    let s = settle(&ctx, &mut a);
    let chip = s.text("\u{1F44D} 2").unwrap_or_else(|| panic!("counted at once: {:?}", s.texts));
    assert_eq!(frame_around(&s.shapes, chip), Some((t.accent, t.accent_soft)), "the user's");
    assert!(!has_text(&s.texts, "Sending…"), "a reaction isn't a message going out: {:?}", s.texts);
    let _ = std::fs::remove_dir_all(dir);
}

/// Copy puts the message's words on the clipboard; for one with an edit going out, the edit's.
#[test]
fn copy_puts_the_words_on_the_clipboard() {
    let (mut a, dir) = writing_app("teams-bar-copy", vec![ana("m1", NOW - 600, "Lunch at noon"), mine("m2", NOW - 300, "hello")]);
    let ctx = egui::Context::default();
    let s = hover(&ctx, &mut a, "Lunch at noon");
    let c = click(&ctx, &mut a, s.label("Copy").expect("Copy").center());
    assert_eq!(c.copied, vec!["Lunch at noon".to_string()]);
    let edit = OutKind::Edit { message: "m2".into(), created: (NOW - 300) * 1000 };
    outbox(&mut a, vec![change(7, edit, "hello there", OutState::Waiting)]);
    let s = hover(&ctx, &mut a, "hello there");
    let c = click(&ctx, &mut a, s.label("Copy").expect("Copy").center());
    assert_eq!(c.copied, vec!["hello there".to_string()]);
    let _ = std::fs::remove_dir_all(dir);
}

/// No bar on a system line, a message deleted or unshowable, nor on the user's message being
/// deleted.
#[test]
fn no_bar_for_system_deleted_or_unshowable_lines() {
    let mut system = said("m4", Sender::Nobody, NOW - 900, para(vec![words("Riley renamed the conversation to \"Q3 planning\"")]));
    system.kind = MessageKind::System;
    let mut deleted = ana("m6", NOW - 800, "gone");
    deleted.deleted = Some((NOW - 700) * 1000);
    let mut unshowable = ana("m7", NOW - 600, "");
    unshowable.unshowable = true;
    let (mut a, dir) = writing_app("teams-bar-none", vec![system, deleted, unshowable, mine("m8", NOW - 300, "I'll take it")]);
    outbox(&mut a, vec![change(9, OutKind::Delete { message: "m8".into(), created: (NOW - 300) * 1000 }, "", OutState::Waiting)]);
    let ctx = egui::Context::default();
    for text in ["Riley renamed the conversation to \"Q3 planning\"", "This message was deleted", "This message can't be shown", "I'll take it"] {
        let s = hover(&ctx, &mut a, text);
        assert!(!s.bar() && s.label("Copy").is_none(), "{text}: {:?}", s.labels);
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// Ruling 9: Edit from the bar gives the box the keyboard at the next frame (`edit_asked`), and
/// the window draws the frame after that one at once: an Esc then lets the edit go, and the
/// conversations' search stays as it is.
#[test]
fn esc_right_after_edit_cancels_the_edit_and_keeps_the_search() {
    let (mut a, dir) = writing_app("teams-bar-edit-esc", vec![mine("m2", NOW - 300, "hello")]);
    a.ui.conversations.query = "agenda".into();
    a.run("teams.search", json!({"query": "agenda"}));
    let ctx = egui::Context::default();
    // A frame of the whole window: whether it asked for the next one at once.
    let run = |a: &mut MailApp, events: Vec<Event>| {
        let input = RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SIZE.0 as f32, SIZE.1 as f32))),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| a.ui(ui));
        out.textures_delta.clear();
        out.viewport_output.values().any(|v| v.repaint_delay.is_zero())
    };
    for _ in 0..30 {
        run(&mut a, vec![]);
    }
    // As the bar's Edit asks.
    a.ui.conversations.edit_asked = Some("m2".into());
    let mut again = run(&mut a, vec![]);
    assert_eq!(editing(&a).as_deref(), Some("m2"));
    // The frames eframe draws at once when asked.
    for _ in 0..4 {
        if !again {
            break;
        }
        again = run(&mut a, vec![]);
    }
    run(&mut a, vec![esc()]);
    run(&mut a, vec![]);
    assert_eq!(editing(&a), None, "Esc let the edit go");
    assert_eq!(in_box(&a), "", "the draft back");
    assert_eq!(a.ui.conversations.query, "agenda", "the search kept");
    let _ = std::fs::remove_dir_all(dir);
}

/// The bar over Ana's message, its chips the user's (👍) and not (❤️), the chips of the user's own
/// message on its side, and the delete question, light and dark.
#[test]
fn the_bar_the_chips_and_the_question_in_light_and_dark() {
    let mut m = ana("m1", NOW - 600, "Can you check the figures?");
    m.reactions = vec![
        Reaction { emoji: "\u{1F44D}".into(), users: vec!["u-ana".into(), ME.into()] },
        Reaction { emoji: "\u{2764}\u{FE0F}".into(), users: vec!["u-bruno".into()] },
    ];
    let mut own = mine("m2", NOW - 300, "hello");
    own.reactions = vec![
        Reaction { emoji: "\u{1F606}".into(), users: vec!["u-ana".into()] },
        Reaction { emoji: "\u{1F44D}".into(), users: vec!["u-bruno".into(), "u-ana".into()] },
    ];
    let (mut a, dir) = writing_app("teams-bar-frames", vec![m, own]);
    let ctx = egui::Context::default();
    let size = egui::vec2(SIZE.0 as f32, SIZE.1 as f32);
    let at = settled_text_at(&ctx, &mut a, "Can you check the figures?", size).expect("the message");
    let pictures_dir = std::env::var_os("QUATRELLE_FRAME_PNG").map(std::path::PathBuf::from);
    let mut tex = Textures::default();
    for (dark, theme) in [(false, "light"), (true, "dark")] {
        a.ui.dark = Some(dark);
        frame_with_size(&ctx, &mut a, &mut tex, vec![Event::PointerMoved(at)], None, SIZE);
        for _ in 0..6 {
            frame_with_size(&ctx, &mut a, &mut tex, vec![], None, SIZE);
        }
        let png = pictures_dir.as_ref().map(|d| d.join(format!("teams-bar-{theme}.png")));
        let (_, shapes) = frame_with_size(&ctx, &mut a, &mut tex, vec![], png, SIZE);
        let texts = texts_of(&shapes);
        for w in QUICK_REACTIONS.iter().copied().chain(["\u{1F44D} 2", "\u{2764}\u{FE0F} 1"]) {
            assert!(has_text(&texts, w), "{theme}: {w}: {texts:?}");
        }
        let t = a.ui.tokens;
        let mine = frame_around(&shapes, rect_of(&texts, |x| x == "\u{1F44D} 2").unwrap());
        let theirs = frame_around(&shapes, rect_of(&texts, |x| x == "\u{2764}\u{FE0F} 1").unwrap());
        assert_eq!(mine, Some((t.accent, t.accent_soft)), "{theme}: the user's");
        assert_eq!(theirs.map(|f| f.0), Some(t.border), "{theme}: not the user's");
    }
    a.ui.conversations.deleting = Some("m2".into());
    for (dark, theme) in [(false, "light"), (true, "dark")] {
        a.ui.dark = Some(dark);
        for _ in 0..8 {
            frame_with_size(&ctx, &mut a, &mut tex, vec![], None, SIZE);
        }
        let png = pictures_dir.as_ref().map(|d| d.join(format!("teams-delete-question-{theme}.png")));
        let (_, shapes) = frame_with_size(&ctx, &mut a, &mut tex, vec![], png, SIZE);
        let texts = texts_of(&shapes);
        for w in ["Delete this message?", "It will be deleted for everyone in this conversation.", "Delete", "Cancel"] {
            assert!(has_text(&texts, w), "{theme}: {w}: {texts:?}");
        }
    }
    a.ui.dark = Some(false);
    let _ = std::fs::remove_dir_all(dir);
}
