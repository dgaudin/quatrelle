// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: the message editor's right-click menu — an icon before each command that has one,
//! the paste entries, and the list toggles.

use super::markdown::{CLIPBOARD_HTML, clipboard_html, compose, writing};
use super::*;
use quatrelle_engine::mime::rich::List;

const SIZE: egui::Vec2 = egui::vec2(1280.0, 800.0);

/// A spot in the editor's text, where a right-click lands.
fn text_pos(a: &MailApp) -> egui::Pos2 {
    a.ui.editor.as_ref().and_then(|e| e.drawn.as_ref()).map(|d| d.rect.left_top() + egui::vec2(24.0, 8.0)).expect("the text is drawn")
}

/// Opens the right-click menu on the editor's text, at `at` (a spot in the text).
fn open_menu_at(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2) {
    let press = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE };
    frame_sized(ctx, a, vec![Event::PointerMoved(at)], SIZE);
    frame_sized(ctx, a, vec![press(true)], SIZE);
    frame_sized(ctx, a, vec![press(false)], SIZE);
    assert!(settled_text_at(ctx, a, "Select all", SIZE).is_some(), "the menu is open");
}

/// Opens the right-click menu on the editor's text.
fn open_menu(ctx: &egui::Context, a: &mut MailApp) {
    open_menu_at(ctx, a, text_pos(a));
}

/// Where `label` is drawn: the left edge of its line of text, at mid-height.
fn label_at(shapes: &[egui::Shape], label: &str) -> Option<egui::Pos2> {
    let find = |exact: bool| {
        shapes.iter().rev().find_map(|s| match s {
            egui::Shape::Text(t) if (exact && t.galley.text() == label) || (!exact && t.galley.text().starts_with(label)) => {
                Some(t.pos + egui::vec2(0.0, t.galley.rect.height() / 2.0))
            }
            _ => None,
        })
    };
    find(true).or_else(|| find(false))
}

/// The icon drawn just left of `label` (its texture), when there is one.
fn icon_left_of(shapes: &[egui::Shape], label: &str) -> Option<egui::TextureId> {
    let at = label_at(shapes, label)?;
    shapes.iter().find_map(|s| match s {
        egui::Shape::Mesh(m) if m.texture_id != egui::TextureId::default() => {
            let r = m.calc_bounds();
            (r.max.x <= at.x && r.max.x > at.x - 40.0 && (r.center().y - at.y).abs() < 6.0).then(|| m.texture_id)
        }
        _ => None,
    })
}

/// The accessibility tree of a frame drawn with AccessKit on.
fn a11y(ctx: &egui::Context, a: &mut MailApp) -> Vec<(egui::accesskit::NodeId, egui::accesskit::Node)> {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SIZE)), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default()
}

/// Whether the menu entry `label` is selected (checked) in the accessibility tree: the node
/// named like it whose bounds hold `at`, where the entry is drawn (the ribbon's toggles share
/// the name but sit elsewhere).
fn entry_selected(ctx: &egui::Context, a: &mut MailApp, label: &str, at: egui::Pos2) -> Option<bool> {
    let at = egui::accesskit::Point { x: at.x as f64, y: at.y as f64 };
    let nodes = a11y(ctx, a);
    nodes
        .iter()
        .rev()
        .find(|(_, n)| n.label().is_some_and(|l| l.starts_with(label)) && n.bounds().is_some_and(|b| b.contains(at)))
        .map(|(_, n)| matches!(n.toggled(), Some(egui::accesskit::Toggled::True)))
}

/// The menu opened on the text shows an icon before each command that has one, as Outlook's
/// menus do: Cut, Copy, Paste, Select all, the lists, the link entries, Clear formatting.
#[test]
fn the_editors_menu_has_an_icon_before_each_command() {
    let (mut a, dir, ctx) = writing("editor-menu-icons");
    a.run("compose.update", json!({"body": "Hello world"}));
    frame(&ctx, &mut a, vec![]);
    let id = crate::rich_edit::body_id(a.session.compose.as_ref().unwrap());
    crate::rich_edit::set_caret(&ctx, id, 0, 5);
    frame(&ctx, &mut a, vec![]);
    open_menu(&ctx, &mut a);
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    for label in ["Cut", "Copy", "Paste", "Select all", "Bullets", "Numbering", "Link…", "Clear formatting"] {
        assert!(label_at(&shapes, label).is_some(), "{label} in the menu");
        assert!(icon_left_of(&shapes, label).is_some(), "an icon before {label}");
    }
    // With a link on the selection, its entries show, each with its icon.
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    {
        let ed = a.ui.editor.as_mut().unwrap();
        let c = a.session.compose.as_mut().unwrap();
        crate::rich_edit::apply(ed, c, crate::rich_edit::Op::Link("https://contoso.example".into()));
    }
    crate::rich_edit::set_caret(&ctx, id, 0, 5);
    frame(&ctx, &mut a, vec![]);
    open_menu(&ctx, &mut a);
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    for label in ["Edit link…", "Open link", "Remove link"] {
        assert!(label_at(&shapes, label).is_some(), "{label} in the menu");
        assert!(icon_left_of(&shapes, label).is_some(), "an icon before {label}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// The menu's paste entries, as the ribbon's: "Paste" goes in with the look the clipboard's HTML
/// gives, "Keep text only" pastes the text alone.
#[test]
fn the_menus_paste_and_keep_text_only_pick_the_look_or_the_text() {
    crate::set_clipboard_html(clipboard_html);
    let (mut a, dir, ctx) = writing("editor-menu-paste");
    // Paste: the bold HTML on the clipboard goes in with its look.
    open_menu(&ctx, &mut a);
    let at = settled_text_at(&ctx, &mut a, "Paste", SIZE).expect("Paste in the menu");
    click_at(&ctx, &mut a, at, 1, SIZE);
    crate::rich_edit::forget_clipboard_html();
    CLIPBOARD_HTML.with(|c| *c.borrow_mut() = Some("<p><b>Bold</b> words</p>".into()));
    frame(&ctx, &mut a, vec![Event::Paste("Bold words".into())]);
    frame(&ctx, &mut a, vec![]);
    CLIPBOARD_HTML.with(|c| *c.borrow_mut() = None);
    let c = compose(&a);
    assert_eq!(c.body, "Bold words");
    assert!(c.styles.style(0).bold && !c.styles.style(5).bold, "Paste keeps the look");
    // Keep text only: the same clipboard goes in as plain text.
    a.run("compose.update", json!({"body": ""}));
    frame(&ctx, &mut a, vec![]);
    open_menu(&ctx, &mut a);
    let at = settled_text_at(&ctx, &mut a, "Keep text only", SIZE).expect("Keep text only in the menu");
    click_at(&ctx, &mut a, at, 1, SIZE);
    crate::rich_edit::forget_clipboard_html();
    CLIPBOARD_HTML.with(|c| *c.borrow_mut() = Some("<p><b>Bold</b> words</p>".into()));
    frame(&ctx, &mut a, vec![Event::Paste("Bold words".into())]);
    frame(&ctx, &mut a, vec![]);
    CLIPBOARD_HTML.with(|c| *c.borrow_mut() = None);
    let c = compose(&a);
    assert_eq!(c.body, "Bold words");
    assert!(!c.styles.style(0).bold, "Keep text only: no bold");
    let _ = std::fs::remove_dir_all(dir);
}

/// A "Keep text only" whose paste never comes (the clipboard couldn't be read) doesn't stick to
/// the next Ctrl+V: that one pastes with its look, as Ctrl+V does.
#[test]
fn a_keep_text_only_whose_paste_never_comes_does_not_stick() {
    crate::set_clipboard_html(clipboard_html);
    let (mut a, dir, ctx) = writing("editor-menu-paste-missed");
    a.ui.editor.as_mut().expect("the message is drawn").paste_text_only(0..0);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]); // the asked paste never came: told in the log, forgotten
    crate::rich_edit::forget_clipboard_html();
    CLIPBOARD_HTML.with(|c| *c.borrow_mut() = Some("<p><b>Bold</b> words</p>".into()));
    frame(&ctx, &mut a, vec![Event::Paste("Bold words".into())]);
    frame(&ctx, &mut a, vec![]);
    CLIPBOARD_HTML.with(|c| *c.borrow_mut() = None);
    let c = compose(&a);
    assert_eq!(c.body, "Bold words");
    assert!(c.styles.style(0).bold, "the next Ctrl+V keeps the look: paste_plain didn't stick");
    let _ = std::fs::remove_dir_all(dir);
}

/// The menu's Bullets and Numbering apply a list to the selection's paragraphs and take it off
/// again; while the selection is one, the entry is selected (checked), as the calendar's menus
/// mark theirs — and keeps its list icon.
#[test]
fn the_menus_bullets_and_numbering_toggle_the_selection() {
    let (mut a, dir, ctx) = writing("editor-menu-lists");
    ctx.enable_accesskit();
    a.run("compose.update", json!({"body": "First\nSecond"}));
    frame(&ctx, &mut a, vec![]);
    let n = compose(&a).body.chars().count();
    let id = crate::rich_edit::body_id(a.session.compose.as_ref().unwrap());
    let lists = |a: &MailApp| {
        let c = compose(a);
        (0..c.body.split('\n').count()).map(|p| c.styles.para(p).list).collect::<Vec<_>>()
    };
    let select_all = |ctx: &egui::Context, a: &mut MailApp| {
        crate::rich_edit::set_caret(ctx, id, 0, n);
        frame(ctx, a, vec![]);
    };
    // Bullets on; the entry is selected while the selection is a list, and keeps its icon.
    select_all(&ctx, &mut a);
    open_menu(&ctx, &mut a);
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    let at = label_at(&shapes, "Bullets").expect("Bullets in the menu");
    click_at(&ctx, &mut a, at, 1, SIZE);
    assert_eq!(lists(&a), [List::Bullet, List::Bullet], "a bullet list");
    select_all(&ctx, &mut a);
    open_menu(&ctx, &mut a);
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    let at = label_at(&shapes, "Bullets").expect("Bullets in the menu");
    assert_eq!(entry_selected(&ctx, &mut a, "Bullets", at), Some(true), "selected while a list");
    assert!(icon_left_of(&shapes, "Bullets").is_some(), "still the list icon, not a tick");
    click_at(&ctx, &mut a, at, 1, SIZE);
    assert_eq!(lists(&a), [List::None, List::None], "off again");
    // Numbering on: selected too; off again.
    select_all(&ctx, &mut a);
    open_menu(&ctx, &mut a);
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    let at = label_at(&shapes, "Numbering").expect("Numbering in the menu");
    click_at(&ctx, &mut a, at, 1, SIZE);
    assert_eq!(lists(&a), [List::Number, List::Number], "a numbered list");
    select_all(&ctx, &mut a);
    open_menu(&ctx, &mut a);
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    let at = label_at(&shapes, "Numbering").expect("Numbering in the menu");
    assert_eq!(entry_selected(&ctx, &mut a, "Numbering", at), Some(true), "selected while a numbered list");
    click_at(&ctx, &mut a, at, 1, SIZE);
    assert_eq!(lists(&a), [List::None, List::None], "off again");
    // A mixed selection (only the first paragraph a list) isn't selected; one click lists both.
    {
        let ed = a.ui.editor.as_mut().unwrap();
        let c = a.session.compose.as_mut().unwrap();
        ed.sel = 0..5;
        crate::rich_edit::apply(ed, c, crate::rich_edit::Op::Bullets);
    }
    assert_eq!(lists(&a), [List::Bullet, List::None], "only the first is a list");
    select_all(&ctx, &mut a);
    open_menu(&ctx, &mut a);
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    let at = label_at(&shapes, "Bullets").expect("Bullets in the menu");
    assert_eq!(entry_selected(&ctx, &mut a, "Bullets", at), Some(false), "a mixed selection isn't a list");
    click_at(&ctx, &mut a, at, 1, SIZE);
    assert_eq!(lists(&a), [List::Bullet, List::Bullet], "one click lists both");
    let _ = std::fs::remove_dir_all(dir);
}

/// The cursor alone on the second paragraph: only that paragraph becomes a list.
#[test]
fn the_menus_bullets_apply_to_the_cursors_paragraph_alone() {
    let (mut a, dir, ctx) = writing("editor-menu-list-cursor");
    a.run("compose.update", json!({"body": "First\nSecond"}));
    frame(&ctx, &mut a, vec![]);
    let id = crate::rich_edit::body_id(a.session.compose.as_ref().unwrap());
    crate::rich_edit::set_caret(&ctx, id, 6, 6);
    frame(&ctx, &mut a, vec![]);
    // Right-click on the second line, so the cursor is there.
    let d = a.ui.editor.as_ref().and_then(|e| e.drawn.as_ref()).expect("the text is drawn");
    let r = d.galley.pos_from_cursor(egui::text::CCursor::new(7));
    let line = d.pos + r.center().to_vec2();
    open_menu_at(&ctx, &mut a, line);
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    let at = label_at(&shapes, "Bullets").expect("Bullets in the menu");
    click_at(&ctx, &mut a, at, 1, SIZE);
    let c = compose(&a);
    assert_eq!((c.styles.para(0).list, c.styles.para(1).list), (List::None, List::Bullet), "only the second");
    let _ = std::fs::remove_dir_all(dir);
}
