// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: File › Options › Calendar › Webex rooms, a row per room (its name, its link, its lines
//! and ×) and a row to add one, each box saved once it's left.

use egui::accesskit::Role;
use quatrelle_engine::{WEBEX_ROOM_LINE_MAX, WEBEX_ROOM_LINES_MAX, WEBEX_ROOM_NAME_MAX, WebexRoom};

use super::*;

const ANA: &str = "https://acme.webex.com/meet/ana";
const TEAM: &str = "https://acme.webex.com/meet/team";
const TEAM2: &str = "https://acme.webex.com/meet/team2";
const SALLE_B: &str = "https://acme.webex.com/meet/b";
const LINES: &str = "Meeting number, video address, dial-in numbers… copied into the notes";
const NOT_WEBEX: &str = "Not a Webex personal room: https://<site>.webex.com/meet/<name>";
const TAKEN: &str = "Another room has this link";
const REMOVE: &str = "Remove this Webex room";
/// Tall enough for every card of Options.
const SIZE: egui::Vec2 = egui::vec2(1280.0, 5000.0);

/// The room carried over from 1.0, unnamed, with a line; and a named one with none.
fn rooms() -> Vec<WebexRoom> {
    vec![
        WebexRoom { name: String::new(), url: ANA.into(), details: vec!["N° 123".into()] },
        WebexRoom { name: "Équipe".into(), url: TEAM.into(), details: Vec::new() },
    ]
}

/// File › Options, with `rooms` kept.
fn options_with(name: &str, rooms: Vec<WebexRoom>) -> (MailApp, std::path::PathBuf, egui::Context) {
    let (mut a, dir) = app(name);
    let ctx = egui::Context::default();
    a.session.prefs.webex_rooms = rooms;
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
    frame_sized(&ctx, &mut a, vec![], SIZE);
    (a, dir, ctx)
}

/// Where each `text` is drawn, from the top down.
fn every_text_at(shapes: &[egui::Shape], text: &str) -> Vec<egui::Pos2> {
    let mut at: Vec<egui::Pos2> = shapes
        .iter()
        .filter_map(|s| match s {
            egui::Shape::Text(t) if t.galley.text() == text => Some(t.pos + t.galley.rect.center().to_vec2()),
            _ => None,
        })
        .collect();
    at.sort_by(|a, b| a.y.total_cmp(&b.y));
    at
}

/// Where the buttons a screen reader is told are called `label` are, from the top down.
fn buttons_at(ctx: &egui::Context, a: &mut MailApp, label: &str) -> Vec<egui::Pos2> {
    ctx.enable_accesskit();
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SIZE)), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
    let mut at: Vec<egui::Pos2> = nodes
        .iter()
        .filter(|(_, n)| n.role() == Role::Button && n.label() == Some(label))
        .filter_map(|(_, n)| n.bounds())
        .map(|r| egui::pos2(((r.x0 + r.x1) / 2.0) as f32, ((r.y0 + r.y1) / 2.0) as f32))
        .collect();
    at.sort_by(|a, b| a.y.total_cmp(&b.y));
    at
}

/// `text` typed in the box drawn at `at`, in place of what it had, the box not left.
fn type_over(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2, text: &str) {
    click_at(ctx, a, at, 1, SIZE);
    frame_sized(ctx, a, vec![key(Key::A, Modifiers::COMMAND)], SIZE);
    frame_sized(ctx, a, vec![Event::Paste(text.into())], SIZE);
}

/// The box being typed in left: a click on the rooms' title.
fn leave(ctx: &egui::Context, a: &mut MailApp) {
    let away = settled_text_at(ctx, a, "Webex rooms", SIZE).expect("the rooms' title");
    click_at(ctx, a, away, 1, SIZE);
}

/// `text` typed in the box drawn at `at`, in place of what it had, then the box left.
fn type_in(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2, text: &str) {
    type_over(ctx, a, at, text);
    leave(ctx, a);
}

/// Nothing was refused: no notice, so no toast.
fn nothing_refused(a: &MailApp) {
    assert!(a.session.notices.is_empty(), "{:?}", a.session.notices);
    assert!(a.ui.toast.is_none(), "{:?}", a.ui.toast);
}

/// Each room has its row: its name (the unnamed one hinting it's the personal room), its link and
/// its lines; then a row to add one.
#[test]
fn options_lists_the_rooms() {
    let (mut a, dir, ctx) = options_with("room-options-list", rooms());
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    for text in ["Webex rooms", "My personal room", "Équipe", ANA, TEAM, "N° 123", LINES, "Add a room"] {
        assert!(text_at(&shapes, text).is_some(), "{text}");
    }
    assert!(text_at(&shapes, "Webex personal room").is_none(), "the single field is gone");
    assert_eq!(buttons_at(&ctx, &mut a, REMOVE).len(), 2, "a × per room");
    let _ = std::fs::remove_dir_all(dir);
}

/// A room added in the add row goes at the end; a row's lines are saved once their box is left; a
/// link that can't be the room's says why under its row and stays in its box, nothing changed,
/// and a right one changes the room's link in its place; × takes a room out, and the row that
/// moves up shows that room's own text, not what was typed in the row before.
#[test]
fn options_adds_edits_and_removes_rooms() {
    let (mut a, dir, ctx) = options_with("room-options-edit", rooms());
    let at = settled_text_at(&ctx, &mut a, "Room name", SIZE).expect("the add row's name");
    type_in(&ctx, &mut a, at, "Salle B");
    let at = settled_text_at(&ctx, &mut a, "https://site.webex.com/meet/name", SIZE).expect("the add row's link");
    type_in(&ctx, &mut a, at, SALLE_B);
    assert_eq!(a.session.prefs.webex_rooms.len(), 2, "nothing added before the button");
    let at = settled_text_at(&ctx, &mut a, "Add a room", SIZE).expect("the button");
    click_at(&ctx, &mut a, at, 1, SIZE);
    let added = WebexRoom { name: "Salle B".into(), url: SALLE_B.into(), details: Vec::new() };
    assert_eq!(a.session.prefs.webex_rooms.get(2), Some(&added));
    assert!(text_at(&frame_shapes(&ctx, &mut a, vec![], SIZE), "Room name").is_some(), "the add row emptied");

    // Équipe's lines (its box is the first with none).
    let at = every_text_at(&frame_shapes(&ctx, &mut a, vec![], SIZE), LINES);
    assert_eq!(at.len(), 2, "Équipe's and Salle B's");
    type_in(&ctx, &mut a, at[0], "N° 999");
    assert_eq!(a.session.prefs.webex_rooms[1].details, ["N° 999"]);

    // Its link: one that isn't a personal room's, then one another room has (case aside).
    let before = a.session.prefs.webex_rooms.clone();
    let at = settled_text_at(&ctx, &mut a, TEAM, SIZE).expect("Équipe's link");
    type_in(&ctx, &mut a, at, "https://evil.example/x");
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    assert!(text_at(&shapes, NOT_WEBEX).is_some(), "why, under its row");
    assert!(text_at(&shapes, "https://evil.example/x").is_some(), "still in the box");
    assert_eq!(a.session.prefs.webex_rooms, before, "nothing changed");
    let at = settled_text_at(&ctx, &mut a, "https://evil.example/x", SIZE).expect("the box");
    type_in(&ctx, &mut a, at, "HTTPS://acme.webex.com/meet/ANA");
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    assert!(text_at(&shapes, TAKEN).is_some() && text_at(&shapes, NOT_WEBEX).is_none());
    assert_eq!(a.session.prefs.webex_rooms, before, "nothing changed");
    let at = settled_text_at(&ctx, &mut a, "HTTPS://acme.webex.com/meet/ANA", SIZE).expect("the box");
    type_in(&ctx, &mut a, at, TEAM2);
    let team = WebexRoom { name: "Équipe".into(), url: TEAM2.into(), details: vec!["N° 999".into()] };
    assert_eq!(a.session.prefs.webex_rooms.get(1), Some(&team), "changed in its place");
    assert_eq!(a.session.prefs.webex_rooms.len(), 3);
    assert!(text_at(&frame_shapes(&ctx, &mut a, vec![], SIZE), TAKEN).is_none());

    // Ana's row given a wrong link, then ana's × clicked: Équipe moves up, its own link shown.
    let at = settled_text_at(&ctx, &mut a, ANA, SIZE).expect("ana's link");
    type_in(&ctx, &mut a, at, "https://acme.webex.com/x");
    assert!(text_at(&frame_shapes(&ctx, &mut a, vec![], SIZE), NOT_WEBEX).is_some());
    let at = buttons_at(&ctx, &mut a, REMOVE);
    assert_eq!(at.len(), 3);
    click_at(&ctx, &mut a, at[0], 1, SIZE);
    assert_eq!(a.session.prefs.webex_rooms, [team, added], "ana gone");
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    for gone in ["https://acme.webex.com/x", NOT_WEBEX, "N° 123", "My personal room"] {
        assert!(text_at(&shapes, gone).is_none(), "{gone}");
    }
    assert!(text_at(&shapes, TEAM2).is_some() && text_at(&shapes, "N° 999").is_some());
    nothing_refused(&a);
    let _ = std::fs::remove_dir_all(dir);
}

/// A × clicked while its row's link box holds a right link not saved yet: the box is left on
/// that click, its link saved first, and the room then goes by that link, nothing refused.
#[test]
fn a_rooms_x_removes_it_with_its_link_still_being_typed() {
    let (mut a, dir, ctx) = options_with("room-options-x-typing", rooms());
    let at = settled_text_at(&ctx, &mut a, TEAM, SIZE).expect("Équipe's link");
    type_over(&ctx, &mut a, at, TEAM2);
    assert_eq!(a.session.prefs.webex_rooms[1].url, TEAM, "not saved while it's typed");
    let at = buttons_at(&ctx, &mut a, REMOVE);
    click_at(&ctx, &mut a, at[1], 1, SIZE);
    assert_eq!(a.session.prefs.webex_rooms, rooms()[..1], "Équipe gone");
    nothing_refused(&a);
    let _ = std::fs::remove_dir_all(dir);
}

/// The last room's ×, its lines being typed: the lines go first, then the room; no rooms are left,
/// and the add row is still there.
#[test]
fn the_last_rooms_x_leaves_the_add_row() {
    let (mut a, dir, ctx) = options_with("room-options-last", rooms()[..1].to_vec());
    let at = settled_text_at(&ctx, &mut a, "N° 123", SIZE).expect("its lines");
    type_over(&ctx, &mut a, at, "N° 456");
    let at = buttons_at(&ctx, &mut a, REMOVE);
    assert_eq!(at.len(), 1);
    click_at(&ctx, &mut a, at[0], 1, SIZE);
    assert!(a.session.prefs.webex_rooms.is_empty());
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    for text in ["Webex rooms", "Room name", "https://site.webex.com/meet/name", "Add a room"] {
        assert!(text_at(&shapes, text).is_some(), "{text}");
    }
    assert!(text_at(&shapes, "N° 456").is_none() && text_at(&shapes, LINES).is_none(), "no row left");
    assert!(buttons_at(&ctx, &mut a, REMOVE).is_empty());
    nothing_refused(&a);
    let _ = std::fs::remove_dir_all(dir);
}

/// A link refused because another room has it, once that room is gone, stays in its box with no
/// reason shown, and is saved when the box is left again.
#[test]
fn a_refused_link_is_saved_once_it_can_be() {
    let (mut a, dir, ctx) = options_with("room-options-freed", rooms());
    let at = settled_text_at(&ctx, &mut a, ANA, SIZE).expect("ana's link");
    type_in(&ctx, &mut a, at, TEAM);
    assert!(text_at(&frame_shapes(&ctx, &mut a, vec![], SIZE), TAKEN).is_some());
    let at = buttons_at(&ctx, &mut a, REMOVE);
    click_at(&ctx, &mut a, at[1], 1, SIZE);
    assert_eq!(a.session.prefs.webex_rooms.len(), 1, "Équipe gone");
    assert_eq!(a.session.prefs.webex_rooms[0].url, ANA, "nothing sent yet");
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    assert!(text_at(&shapes, TEAM).is_some(), "still in the box");
    assert!(text_at(&shapes, TAKEN).is_none() && text_at(&shapes, NOT_WEBEX).is_none(), "no reason now");
    let at = settled_text_at(&ctx, &mut a, TEAM, SIZE).expect("the box");
    click_at(&ctx, &mut a, at, 1, SIZE);
    leave(&ctx, &mut a);
    let ana = WebexRoom { name: String::new(), url: TEAM.into(), details: vec!["N° 123".into()] };
    assert_eq!(a.session.prefs.webex_rooms, [ana], "saved, in its place");
    nothing_refused(&a);
    let _ = std::fs::remove_dir_all(dir);
}

/// The room carried over from 1.0 is named in its row.
#[test]
fn the_unnamed_room_takes_a_name() {
    let (mut a, dir, ctx) = options_with("room-options-name", rooms());
    let at = settled_text_at(&ctx, &mut a, "My personal room", SIZE).expect("its name's box");
    type_in(&ctx, &mut a, at, "Ana");
    assert_eq!(a.session.prefs.webex_rooms[0].name, "Ana");
    assert_eq!(a.session.prefs.webex_rooms[0].url, ANA);
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    assert!(text_at(&shapes, "Ana").is_some() && text_at(&shapes, "My personal room").is_none());
    let _ = std::fs::remove_dir_all(dir);
}

/// What's typed is cut to what a room keeps before it's sent, so it's never refused for its size:
/// a name of 80 characters, 20 lines of 200 (the blank ones left out), lines ended by `\r\n` kept
/// without the `\r`; a named room's name taken off isn't sent (nothing refused), and the box shows
/// it again.
#[test]
fn what_is_typed_is_cut_to_what_a_room_keeps() {
    let (mut a, dir, ctx) = options_with("room-options-caps", rooms());
    let at = settled_text_at(&ctx, &mut a, "My personal room", SIZE).expect("its name's box");
    type_in(&ctx, &mut a, at, &"é".repeat(WEBEX_ROOM_NAME_MAX + 10));
    assert_eq!(a.session.prefs.webex_rooms[0].name, "é".repeat(WEBEX_ROOM_NAME_MAX));
    let line = "x".repeat(WEBEX_ROOM_LINE_MAX + 50);
    let typed = vec![line.as_str(); WEBEX_ROOM_LINES_MAX + 5].join("\n\n");
    let at = settled_text_at(&ctx, &mut a, LINES, SIZE).expect("Équipe's lines");
    type_in(&ctx, &mut a, at, &typed);
    let kept = vec!["x".repeat(WEBEX_ROOM_LINE_MAX); WEBEX_ROOM_LINES_MAX];
    assert_eq!(a.session.prefs.webex_rooms[1].details, kept);
    let at = settled_text_at(&ctx, &mut a, "N° 123", SIZE).expect("ana's lines");
    type_in(&ctx, &mut a, at, "N° 1\r\nVidéo : ana@acme.webex.com\r\n");
    assert_eq!(a.session.prefs.webex_rooms[0].details, ["N° 1", "Vidéo : ana@acme.webex.com"]);
    let at = settled_text_at(&ctx, &mut a, "Équipe", SIZE).expect("its name's box");
    click_at(&ctx, &mut a, at, 1, SIZE);
    frame_sized(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)], SIZE);
    frame_sized(&ctx, &mut a, vec![key(Key::Delete, Modifiers::NONE)], SIZE);
    leave(&ctx, &mut a);
    assert_eq!(a.session.prefs.webex_rooms[1].name, "Équipe", "kept");
    assert!(text_at(&frame_shapes(&ctx, &mut a, vec![], SIZE), "Équipe").is_some(), "shown again");
    nothing_refused(&a);
    let _ = std::fs::remove_dir_all(dir);
}
