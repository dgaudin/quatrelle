// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: the message editor's right-click menu — an icon before each command that has one,
//! the paste options, and the list toggles.

use super::markdown::{CLIPBOARD_HTML, clipboard_html, compose, writing};
use super::*;
use quatrelle_engine::mime::rich::List;

const SIZE: egui::Vec2 = egui::vec2(1280.0, 800.0);

/// A spot in the editor's text, where a right-click lands.
fn text_pos(a: &MailApp) -> egui::Pos2 {
    a.ui.editor.as_ref().and_then(|e| e.drawn.as_ref()).map(|d| d.rect.left_top() + egui::vec2(24.0, 8.0)).expect("the text is drawn")
}

/// Opens the right-click menu on the editor's text.
fn open_menu(ctx: &egui::Context, a: &mut MailApp) {
    let at = text_pos(a);
    let press = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE };
    frame_sized(ctx, a, vec![Event::PointerMoved(at)], SIZE);
    frame_sized(ctx, a, vec![press(true)], SIZE);
    frame_sized(ctx, a, vec![press(false)], SIZE);
    assert!(settled_text_at(ctx, a, "Select all", SIZE).is_some(), "the menu is open");
}

/// Where `label` is drawn (its galley's top-left).
fn label_at(shapes: &[egui::Shape], label: &str) -> Option<egui::Pos2> {
    text_at(shapes, label).or_else(|| {
        shapes.iter().rev().find_map(|s| match s {
            egui::Shape::Text(t) if t.galley.text().starts_with(label) => Some(t.pos),
            _ => None,
        })
    })
}

/// The icon drawn just left of `label` (its texture), when there is one.
fn icon_left_of(shapes: &[egui::Shape], label: &str) -> Option<egui::TextureId> {
    let at = label_at(shapes, label)?;
    shapes.iter().find_map(|s| match s {
        egui::Shape::Mesh(m) if m.texture_id != egui::TextureId::default() => {
            let r = m.calc_bounds();
            (r.max.x <= at.x && r.max.x > at.x - 90.0 && (r.center().y - at.y).abs() < 6.0).then(|| m.texture_id)
        }
        _ => None,
    })
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

/// The menu's paste options: « Keep source formatting » pastes with the look the clipboard's
/// HTML gives, « Keep text only » pastes the text alone.
#[test]
fn the_menus_paste_options_pick_the_look_or_the_text() {
    crate::set_clipboard_html(clipboard_html);
    let (mut a, dir, ctx) = writing("editor-menu-paste");
    // Keep text only: the bold HTML on the clipboard goes in as plain text.
    open_menu(&ctx, &mut a);
    let at = settled_text_at(&ctx, &mut a, "Keep text only", SIZE).expect("Keep text only in the menu");
    click_at(&ctx, &mut a, at, 1, SIZE);
    frame(&ctx, &mut a, vec![]);
    crate::rich_edit::forget_clipboard_html();
    CLIPBOARD_HTML.with(|c| *c.borrow_mut() = Some("<p><b>Bold</b> words</p>".into()));
    frame(&ctx, &mut a, vec![Event::Paste("Bold words".into())]);
    frame(&ctx, &mut a, vec![]);
    CLIPBOARD_HTML.with(|c| *c.borrow_mut() = None);
    let c = compose(&a);
    assert_eq!(c.body, "Bold words");
    assert!(!c.styles.style(0).bold, "the text only, no bold");
    // Keep source formatting: the same paste goes in with its look.
    a.run("compose.update", json!({"body": ""}));
    frame(&ctx, &mut a, vec![]);
    open_menu(&ctx, &mut a);
    let at = settled_text_at(&ctx, &mut a, "Keep source formatting", SIZE).expect("Keep source formatting in the menu");
    click_at(&ctx, &mut a, at, 1, SIZE);
    frame(&ctx, &mut a, vec![]);
    crate::rich_edit::forget_clipboard_html();
    CLIPBOARD_HTML.with(|c| *c.borrow_mut() = Some("<p><b>Bold</b> words</p>".into()));
    frame(&ctx, &mut a, vec![Event::Paste("Bold words".into())]);
    frame(&ctx, &mut a, vec![]);
    CLIPBOARD_HTML.with(|c| *c.borrow_mut() = None);
    let c = compose(&a);
    assert_eq!(c.body, "Bold words");
    assert!(c.styles.style(0).bold && !c.styles.style(5).bold, "with its look");
    let _ = std::fs::remove_dir_all(dir);
}

/// The menu's Bullets and Numbering apply a list to the selection's paragraphs and take it off
/// again; while the selection is a list, the entry is checked (a tick, not the list icon).
#[test]
fn the_menus_bullets_and_numbering_toggle_the_selection() {
    let (mut a, dir, ctx) = writing("editor-menu-lists");
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
    // Bullets on.
    select_all(&ctx, &mut a);
    open_menu(&ctx, &mut a);
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    let list_icon = icon_left_of(&shapes, "Bullets").expect("an icon before Bullets");
    let at = label_at(&shapes, "Bullets").expect("Bullets in the menu");
    click_at(&ctx, &mut a, at, 1, SIZE);
    assert_eq!(lists(&a), [List::Bullet, List::Bullet], "a bullet list");
    // While the selection is one, the entry is checked; a second click takes the list off.
    select_all(&ctx, &mut a);
    open_menu(&ctx, &mut a);
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    let checked_icon = icon_left_of(&shapes, "Bullets").expect("an icon before Bullets");
    assert_ne!(list_icon, checked_icon, "checked: a tick, not the list icon");
    let at = label_at(&shapes, "Bullets").expect("Bullets in the menu");
    click_at(&ctx, &mut a, at, 1, SIZE);
    assert_eq!(lists(&a), [List::None, List::None], "off again");
    // Numbering on, then off.
    select_all(&ctx, &mut a);
    open_menu(&ctx, &mut a);
    let at = settled_text_at(&ctx, &mut a, "Numbering", SIZE).expect("Numbering in the menu");
    click_at(&ctx, &mut a, at, 1, SIZE);
    assert_eq!(lists(&a), [List::Number, List::Number], "a numbered list");
    select_all(&ctx, &mut a);
    open_menu(&ctx, &mut a);
    let at = settled_text_at(&ctx, &mut a, "Numbering", SIZE).expect("Numbering in the menu");
    click_at(&ctx, &mut a, at, 1, SIZE);
    assert_eq!(lists(&a), [List::None, List::None], "off again");
    let _ = std::fs::remove_dir_all(dir);
}
