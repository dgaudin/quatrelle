// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: a note's window (drawn inside the page, as the browser and these frames draw it),
//! its Markdown editor and bar, and the grid's tiles.

use std::cell::RefCell;
use std::rc::Rc;

use egui::accesskit::{Node, Role};
use egui::text::CCursor;

use super::*;
use crate::notes::{self, MIN, STEADY, Sizer, USUAL};

/// The sample's Notes shown, with a note of `text` (sized when `size` says): its uid.
fn with_note(name: &str, text: &str, size: Option<(u16, u16)>) -> (MailApp, std::path::PathBuf, egui::Context, String) {
    let (mut a, dir) = app(name);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("view.module", json!({"name": "notes"}));
    let mut p = json!({"text": text});
    if let Some((w, h)) = size {
        p["width"] = json!(w);
        p["height"] = json!(h);
    }
    let uid = a.session.run("note.create", &p).unwrap()["uid"].as_str().unwrap().to_string();
    settle(&ctx, &mut a);
    (a, dir, ctx, uid)
}

fn settle(ctx: &egui::Context, a: &mut MailApp) {
    for _ in 0..5 {
        frame(ctx, a, vec![]);
    }
}

/// Open note `uid`'s window, as a double-click does.
fn open(ctx: &egui::Context, a: &mut MailApp, uid: &str) {
    let n = a.session.note(uid).cloned().unwrap();
    notes::open(&mut a.ui, &n);
    settle(ctx, a);
}

fn written(a: &MailApp, uid: &str) -> Option<String> {
    a.ui.note_windows.iter().find(|w| w.uid == uid).map(|w| w.editor.text.clone())
}

fn kept(a: &MailApp, uid: &str) -> String {
    a.session.note(uid).map(|n| n.text.clone()).unwrap_or_default()
}

fn window_rect(ctx: &egui::Context, uid: &str) -> egui::Rect {
    ctx.memory(|m| m.area_rect(notes::in_page_id(uid))).unwrap()
}

const SIZE: egui::Vec2 = egui::vec2(1280.0, 800.0);

/// A click at `at` with `modifiers` held, after a pause (no double-click with the last).
fn click_with(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2, modifiers: Modifiers) {
    for _ in 0..20 {
        frame(ctx, a, vec![]);
    }
    let press = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers };
    frame(ctx, a, vec![Event::ModifiersChanged(modifiers), Event::PointerMoved(at)]);
    frame(ctx, a, vec![press(true)]);
    frame(ctx, a, vec![press(false)]);
    frame(ctx, a, vec![Event::ModifiersChanged(Modifiers::NONE)]);
}

/// What a screen reader is told this frame: each node's role, name and ticked state, and where.
fn nodes(ctx: &egui::Context, a: &mut MailApp) -> Vec<(Role, String, Option<bool>, egui::Rect)> {
    ctx.enable_accesskit();
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SIZE)), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let tree = out.platform_output.accesskit_update.unwrap();
    let rect = |n: &Node| {
        n.bounds()
            .map_or(egui::Rect::NOTHING, |b| egui::Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32)))
    };
    tree.nodes
        .iter()
        .map(|(_, n)| {
            let ticked = n.toggled().map(|t| t == egui::accesskit::Toggled::True);
            (n.role(), n.label().unwrap_or_default().to_string(), ticked, rect(n))
        })
        .collect()
}

/// Where the control named `name` is (the last one so named: the window on top's).
fn named(ctx: &egui::Context, a: &mut MailApp, name: &str) -> egui::Pos2 {
    nodes(ctx, a).into_iter().rev().find(|(_, n, _, _)| n == name).map(|(_, _, _, r)| r.center()).unwrap()
}

/// Where character `ch` of the note's text is drawn in its window.
fn char_at(ctx: &egui::Context, a: &mut MailApp, text: &str, ch: usize) -> egui::Pos2 {
    let shapes = frame_shapes(ctx, a, vec![], SIZE);
    shapes
        .iter()
        .rev()
        .find_map(|s| match s {
            egui::Shape::Text(t) if t.galley.text() == text => {
                Some(t.pos + t.galley.pos_from_cursor(CCursor::new(ch)).center().to_vec2() + egui::vec2(3.0, 0.0))
            }
            _ => None,
        })
        .unwrap()
}

/// New note opens its window with the keyboard in its text; what's typed stays in the window
/// (not saved at each key) and is kept when Escape closes it.
#[test]
fn a_new_note_is_written_in_its_own_window_and_kept_on_closing() {
    let (mut a, dir) = app("note-window-new");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("view.module", json!({"name": "notes"}));
    frame(&ctx, &mut a, vec![]);
    a.act(crate::Action::NewNote);
    settle(&ctx, &mut a);
    let uid = a.ui.note_windows.first().map(|w| w.uid.clone()).expect("its window opened");
    assert!(ctx.memory(|m| m.has_focus(notes::text_id(&uid))), "its text has the keyboard");
    frame(&ctx, &mut a, vec![Event::Text("Water the plants".into())]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(written(&a, &uid).as_deref(), Some("Water the plants"));
    assert_eq!(kept(&a, &uid), "", "not saved at each key");
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.note_windows.is_empty(), "Escape closed it");
    assert_eq!(kept(&a, &uid), "Water the plants");
    // Ctrl+1 goes back to Mail.
    frame(&ctx, &mut a, vec![key(Key::Num1, Modifiers::COMMAND)]);
    assert_eq!(a.session.module, quatrelle_engine::Module::Mail);
    let _ = std::fs::remove_dir_all(dir);
}

/// One click on a tile selects it (typing goes nowhere: the tile isn't written on any more);
/// a double-click opens its window, Enter too; Delete deletes the note selected, its window
/// with it. The text is kept when the keyboard leaves it.
#[test]
fn a_tile_is_selected_with_a_click_and_opened_with_a_double_click() {
    let text = "Shopping\n- [ ] milk";
    let (mut a, dir, ctx, uid) = with_note("note-window-grid", text, None);
    let tile = text_at(&frame_shapes(&ctx, &mut a, vec![], SIZE), text).expect("the tile shows the note");
    click_at(&ctx, &mut a, tile, 1, SIZE);
    assert_eq!(a.ui.note_selected.as_deref(), Some(uid.as_str()));
    assert!(a.ui.note_windows.is_empty(), "one click opens nothing");
    frame(&ctx, &mut a, vec![Event::Text("zz".into())]);
    settle(&ctx, &mut a);
    assert_eq!(kept(&a, &uid), text, "nothing typed on the tile");
    click_at(&ctx, &mut a, tile, 2, SIZE);
    assert_eq!(a.ui.note_windows.len(), 1, "a double-click opens it");
    // Typed in, then a click back on the grid: the keyboard leaves the text, which is kept.
    frame(&ctx, &mut a, vec![Event::Text(" eggs".into())]);
    click_at(&ctx, &mut a, tile, 1, SIZE);
    assert_eq!(kept(&a, &uid), "Shopping\n- [ ] milk eggs", "kept when the keyboard left it");
    frame(&ctx, &mut a, vec![key(Key::Enter, Modifiers::NONE)]);
    settle(&ctx, &mut a);
    assert_eq!(a.ui.note_windows.len(), 1, "Enter on the one selected brings its window (still one)");
    // Escape closes it; Enter opens it again; Delete from the grid deletes the note.
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    settle(&ctx, &mut a);
    assert!(a.ui.note_windows.is_empty());
    frame(&ctx, &mut a, vec![key(Key::Enter, Modifiers::NONE)]);
    settle(&ctx, &mut a);
    assert_eq!(a.ui.note_windows.len(), 1, "Enter opens the note selected");
    click_at(&ctx, &mut a, tile, 1, SIZE);
    frame(&ctx, &mut a, vec![key(Key::Delete, Modifiers::NONE)]);
    settle(&ctx, &mut a);
    assert!(a.session.note(&uid).is_none(), "Delete deleted it");
    assert!(a.ui.note_windows.is_empty(), "its window went with it");
    let _ = std::fs::remove_dir_all(dir);
}

/// A tile shows the note as its window sets it: a title larger and bold, its mark pale, bold,
/// code in a fixed face, a tick; cut at the tile's edge.
#[test]
fn a_tile_shows_the_note_formatted() {
    let text = format!("# Plan\n**bold** and `code`\n- [x] done{}", "\nmore".repeat(40));
    let (mut a, dir, ctx, _) = with_note("note-window-tile", &text, None);
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SIZE)), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let (clip, galley) = out
        .shapes
        .iter()
        .find_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text() == text => Some((c.clip_rect, t.galley.clone())),
            _ => None,
        })
        .expect("the tile's text");
    let format = |needle: &str| {
        let at = text.find(needle).unwrap();
        galley.job.sections.iter().find(|s| s.byte_range.start == egui::text::ByteIndex(at)).map(|s| s.format.clone()).unwrap()
    };
    let ink = format("Plan");
    assert!(ink.font_id.size > 15.0 && ink.font_id.family == egui::FontFamily::Name("bold".into()), "a title, larger and bold");
    assert_ne!(format("# ").color, ink.color, "its mark pale");
    assert_eq!(format("bold").font_id.family, egui::FontFamily::Name("bold".into()));
    assert_eq!(format("code").font_id.family, egui::FontFamily::Monospace);
    assert_eq!(format("[x]").font_id.family, egui::FontFamily::Monospace, "a tick's box");
    assert!(clip.height() <= 210.0, "cut at the tile's edge: {clip:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// The bar and the keys write their marks: Ctrl+B, the list button, Enter going on with the
/// list, Tab and Shift+Tab, the tick button, and a click on the box ticking it.
#[test]
fn the_bar_and_the_keys_write_their_marks() {
    let (mut a, dir, ctx, uid) = with_note("note-window-bar", "milk", None);
    open(&ctx, &mut a, &uid);
    let id = notes::text_id(&uid);
    crate::rich_edit::set_caret(&ctx, id, 0, 4);
    frame(&ctx, &mut a, vec![key(Key::B, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(written(&a, &uid).as_deref(), Some("**milk**"), "Ctrl+B");
    let list = named(&ctx, &mut a, "Bulleted list");
    click_at(&ctx, &mut a, list, 1, SIZE);
    assert_eq!(written(&a, &uid).as_deref(), Some("- **milk**"), "the bar's list");
    assert!(ctx.memory(|m| m.has_focus(id)), "the text has the keyboard back");
    crate::rich_edit::set_caret(&ctx, id, 10, 10);
    frame(&ctx, &mut a, vec![key(Key::Enter, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![Event::Text("eggs".into())]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(written(&a, &uid).as_deref(), Some("- **milk**\n- eggs"), "Enter goes on with the list");
    frame(&ctx, &mut a, vec![key(Key::Tab, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(written(&a, &uid).as_deref(), Some("- **milk**\n  - eggs"), "Tab takes it in");
    frame(&ctx, &mut a, vec![key(Key::Tab, Modifiers::SHIFT)]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(written(&a, &uid).as_deref(), Some("- **milk**\n- eggs"), "Shift+Tab out");
    let tick = named(&ctx, &mut a, "Tick box");
    click_at(&ctx, &mut a, tick, 1, SIZE);
    assert_eq!(written(&a, &uid).as_deref(), Some("- **milk**\n- [ ] eggs"), "the bar's tick box");
    let boxed = nodes(&ctx, &mut a).into_iter().find(|(r, n, _, _)| *r == Role::CheckBox && n == "eggs").expect("the box is a check box");
    assert_eq!(boxed.2, Some(false));
    click_at(&ctx, &mut a, boxed.3.center(), 1, SIZE);
    assert_eq!(written(&a, &uid).as_deref(), Some("- **milk**\n- [x] eggs"), "a click ticks it");
    let boxed = nodes(&ctx, &mut a).into_iter().find(|(r, n, _, _)| *r == Role::CheckBox && n == "eggs");
    assert_eq!(boxed.map(|b| b.2), Some(Some(true)));
    let _ = std::fs::remove_dir_all(dir);
}

/// The size is sent once a resize ends, once only: never while it goes on, nor for a size that
/// came back to the one sent.
#[test]
fn a_sizer_sends_a_size_once_it_stays_put() {
    let mut s = Sizer::new(USUAL);
    let v = |x: f32, y: f32| egui::vec2(x, y);
    assert_eq!(s.step(v(200.0, 166.0), 0.0, false), None, "the size it opened at");
    assert_eq!(s.step(v(200.0, 166.0), 9.0, false), None, "still that one");
    let mut sent = Vec::new();
    for (i, x) in [220.0, 240.0, 260.0, 260.0, 260.0].into_iter().enumerate() {
        sent.extend(s.step(v(x, 180.0), 10.0 + 0.1 * i as f64, true));
    }
    assert!(sent.is_empty(), "not while it's held");
    for i in 0..60 {
        sent.extend(s.step(v(260.0, 180.0), 11.0 + f64::from(i) / 60.0, false));
    }
    assert_eq!(sent, [(260, 180)], "once, at the end");
    assert_eq!(s.left(), None, "nothing left at the closing");
    // Too small, past the largest, not a number.
    assert_eq!(s.step(v(10.0, 10.0), 20.0, false), None);
    assert_eq!(s.step(v(10.0, 10.0), 20.0 + STEADY, false), Some(MIN));
    assert_eq!(s.step(v(f32::NAN, 10.0), 30.0, false), None);
    assert_eq!(s.step(v(9_000.0, 9_000.0), 31.0, false), None);
    assert_eq!(s.left(), Some((4000, 4000)), "closed before it was sent: sent then");
}

/// Its window opens at the note's size (the usual one for a note never sized, nothing sent for
/// it); dragging its corner sends the new size once, after the drag.
#[test]
fn a_window_opens_at_its_notes_size_and_sends_a_resize_once_done() {
    let (mut a, dir, ctx, sized) = with_note("note-window-size", "sized", Some((320, 240)));
    open(&ctx, &mut a, &sized);
    let r = window_rect(&ctx, &sized);
    assert!((r.width() - 320.0).abs() <= 1.0 && (r.height() - 240.0).abs() <= 1.0, "at its own size: {r:?}");
    let plain = a.session.run("note.create", &json!({"text": "plain"})).unwrap()["uid"].as_str().unwrap().to_string();
    open(&ctx, &mut a, &plain);
    let r = window_rect(&ctx, &plain);
    assert!((r.width() - f32::from(USUAL.0)).abs() <= 1.0 && (r.height() - f32::from(USUAL.1)).abs() <= 1.0, "the usual size: {r:?}");
    for _ in 0..60 {
        frame(&ctx, &mut a, vec![]);
    }
    assert_eq!(a.session.note(&plain).and_then(|n| n.size()), None, "nothing sent for opening");
    // Drag its corner 100 right and 60 down.
    let from = r.max - egui::vec2(2.0, 2.0);
    let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame(&ctx, &mut a, vec![Event::PointerMoved(from)]);
    frame(&ctx, &mut a, vec![button(from, true)]);
    for i in 1..=40 {
        let at = from + egui::vec2(100.0, 60.0) * (i as f32 / 40.0);
        frame(&ctx, &mut a, vec![Event::PointerMoved(at)]);
        assert_eq!(a.session.note(&plain).and_then(|n| n.size()), None, "not while it's dragged");
    }
    let to = from + egui::vec2(100.0, 60.0);
    frame(&ctx, &mut a, vec![button(to, false)]);
    for _ in 0..60 {
        frame(&ctx, &mut a, vec![]);
    }
    let r = window_rect(&ctx, &plain);
    let size = a.session.note(&plain).and_then(|n| n.size()).expect("sent once it stayed put");
    assert!((f32::from(size.0) - r.width()).abs() <= 1.0 && (f32::from(size.1) - r.height()).abs() <= 1.0, "{size:?} for {r:?}");
    assert!(size.0 >= 290 && size.1 >= 216, "the size dragged to: {size:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Services that remember the addresses opened.
struct Opening(Rc<RefCell<Vec<String>>>);

impl crate::Services for Opening {
    fn clock(&self) -> (i64, i32) {
        (NOW, 0)
    }
    fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
        Ok(())
    }
    fn open_url(&mut self, url: &str) -> Result<(), String> {
        self.0.borrow_mut().push(url.into());
        Ok(())
    }
}

/// Ctrl+click on a link opens it through the check a message's links go through (here its
/// tracker taken off); a plain click only puts the caret there.
#[test]
fn ctrl_click_on_a_link_goes_through_the_usual_check() {
    let text = "see [site](https://studio.example/a?utm_source=note) now";
    let (mut a, dir, ctx, uid) = with_note("note-window-link", text, None);
    let opened = Rc::new(RefCell::new(Vec::new()));
    a.services = Box::new(Opening(opened.clone()));
    open(&ctx, &mut a, &uid);
    let at = char_at(&ctx, &mut a, text, 6);
    click_with(&ctx, &mut a, at, Modifiers::NONE);
    assert!(opened.borrow().is_empty(), "a plain click opens nothing");
    click_with(&ctx, &mut a, at, Modifiers::COMMAND);
    assert_eq!(*opened.borrow(), ["https://studio.example/a"], "opened as a message's link is, its tracker taken off");
    let _ = std::fs::remove_dir_all(dir);
}

/// At its smallest, in every colour, light and dark: the window stays 160 × 120 (nothing in it
/// pushes it wider or taller, a long word and a long line neither), its bar inside it, its
/// paper the note's colour.
#[test]
fn a_window_at_its_smallest_holds_in_every_colour_light_and_dark() {
    let long = format!("{}\n{}", "x".repeat(300), "word ".repeat(80));
    let (mut a, dir, ctx, _) = with_note("note-window-small", &long, None);
    for dark in [false, true] {
        a.ui.dark = Some(dark);
        for color in 0..5u8 {
            let p = json!({"text": long, "color": color, "width": MIN.0, "height": MIN.1});
            let uid = a.session.run("note.create", &p).unwrap()["uid"].as_str().unwrap().to_string();
            open(&ctx, &mut a, &uid);
            for _ in 0..40 {
                frame(&ctx, &mut a, vec![]);
            }
            let r = window_rect(&ctx, &uid);
            assert!(r.width() <= 161.0 && r.height() <= 121.0, "colour {color}, dark {dark}: {r:?}");
            assert_eq!(a.session.note(&uid).and_then(|n| n.size()), Some(MIN), "nothing grew it");
            let inside: Vec<String> = nodes(&ctx, &mut a)
                .into_iter()
                .filter(|(role, name, _, b)| {
                    *role == Role::Button && ["Bold (Ctrl+B)", "Link", "More actions", "Close"].contains(&name.as_str()) && r.intersects(*b)
                })
                .filter(|(_, _, _, b)| !r.expand(1.0).contains_rect(*b))
                .map(|(_, n, _, b)| format!("{n} at {b:?}"))
                .collect();
            assert!(inside.is_empty(), "the bar spills out of {r:?}: {inside:?}");
            let paper = crate::notes::paper(color, a.ui.tokens.dark);
            let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
            assert!(
                shapes.iter().any(|s| matches!(s, egui::Shape::Rect(rs) if rs.fill == paper && (rs.rect.width() - r.width()).abs() < 2.0)),
                "its paper: colour {color}, dark {dark}"
            );
            frame(&ctx, &mut a, vec![]);
            // Closed by its own Close button.
            let close = named(&ctx, &mut a, "Close");
            click_at(&ctx, &mut a, close, 1, SIZE);
            assert!(a.ui.note_windows.is_empty(), "Close closed it");
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// A screen reader names the window (its title), each button of its bar, and each tick's box by
/// its words, ticked or not.
#[test]
fn a_note_window_names_its_parts() {
    let (mut a, dir, ctx, uid) = with_note("note-window-names", "# Shopping\n- [ ] milk\n- [x] eggs", None);
    open(&ctx, &mut a, &uid);
    let seen = nodes(&ctx, &mut a);
    let has = |role: Role, name: &str| seen.iter().find(|(r, n, _, _)| *r == role && n == name).map(|(_, _, t, _)| *t);
    assert!(has(Role::Window, "Shopping – Note").is_some(), "the window");
    for b in ["Bold (Ctrl+B)", "Italic (Ctrl+I)", "Heading", "Bulleted list", "Tick box", "Link", "More actions", "Close"] {
        assert!(has(Role::Button, b).is_some(), "{b}");
    }
    assert_eq!(has(Role::CheckBox, "milk"), Some(Some(false)));
    assert_eq!(has(Role::CheckBox, "eggs"), Some(Some(true)));
    let _ = std::fs::remove_dir_all(dir);
}

/// Hostile text (256 KiB of others' characters and marks everywhere): its tile and its window
/// draw, the keys and the bar work on it, and nothing panics.
#[test]
fn a_hostile_note_draws_and_edits_without_a_panic() {
    let piece = "é中**[ ](\t> - [x] `~~_ 1) ```\r\n# ";
    let text: String = piece.repeat(256 * 1024 / piece.len() + 1);
    // A note is kept to 100,000 characters; a window's text can be longer (pasted in).
    let (mut a, dir, ctx, uid) = with_note("note-window-hostile", &text, None);
    open(&ctx, &mut a, &uid);
    if let Some(w) = a.ui.note_windows.first_mut() {
        w.editor.text = text.clone();
    }
    settle(&ctx, &mut a);
    let id = notes::text_id(&uid);
    let n = written(&a, &uid).map(|t| t.chars().count()).unwrap_or(0);
    for (from, to) in [(0, n), (3, 3), (n / 2, n / 2 + 7), (n, n)] {
        crate::rich_edit::set_caret(&ctx, id, from, to);
        for k in [
            key(Key::B, Modifiers::COMMAND),
            key(Key::I, Modifiers::COMMAND),
            key(Key::Enter, Modifiers::NONE),
            key(Key::Tab, Modifiers::NONE),
            key(Key::Tab, Modifiers::SHIFT),
        ] {
            frame(&ctx, &mut a, vec![k]);
        }
    }
    for b in ["Heading", "Bulleted list", "Tick box", "Link"] {
        let at = named(&ctx, &mut a, b);
        click_at(&ctx, &mut a, at, 1, SIZE);
    }
    if let Some(tick) = nodes(&ctx, &mut a).into_iter().find(|(r, _, _, _)| *r == Role::CheckBox) {
        click_at(&ctx, &mut a, tick.3.center(), 1, SIZE);
    }
    // (Enter over the whole text selected put a line break in its place, as a text box does.)
    let last = written(&a, &uid).expect("still open");
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    settle(&ctx, &mut a);
    assert!(a.ui.note_windows.is_empty());
    assert_eq!(kept(&a, &uid), last.chars().take(100_000).collect::<String>(), "what it held, kept");
    let _ = std::fs::remove_dir_all(dir);
}
