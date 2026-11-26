// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! (Adapted from upstream ee0b46ad and 01208ca2.) Plain text out of the way: not a link over the message's text, where it was clicked by
//! mistake, but in the message's More actions (for that message), View › Plain text and File ›
//! Options (for all mail, where Outlook keeps reading all mail as plain text), with the way back
//! to one message's formatting above its text.

use super::*;

mod markdown;

/// A screen tall enough for File › Options' Reading group without scrolling.
const SCREEN: egui::Vec2 = egui::vec2(1280.0, 1400.0);

fn tall(ctx: &egui::Context, a: &mut MailApp, events: Vec<Event>) {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SCREEN)), events, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
}

/// What a screen reader is told is on screen: each node's name and where it is.
fn nodes(ctx: &egui::Context, a: &mut MailApp) -> Vec<(String, Option<egui::Pos2>)> {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SCREEN)), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let tree = out.platform_output.accesskit_update.expect("AccessKit is on");
    tree.nodes
        .iter()
        .filter_map(|(_, n)| {
            let name = n.label().or(n.value())?.to_string();
            let at = n.bounds().map(|b| egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32));
            Some((name, at))
        })
        .collect()
}

fn shows(ctx: &egui::Context, a: &mut MailApp, label: &str) -> bool {
    nodes(ctx, a).iter().any(|(n, _)| n == label)
}

/// Click what a screen reader names `label` (the last of that name: the reading pane's, below
/// the ribbon's).
fn click_named(ctx: &egui::Context, a: &mut MailApp, label: &str) {
    let all = nodes(ctx, a);
    let at = all.iter().filter(|(n, _)| n == label).filter_map(|(_, at)| *at).next_back();
    let at = at.unwrap_or_else(|| panic!("no {label} to click: {:?}", all.iter().map(|(n, _)| n).collect::<Vec<_>>()));
    press(ctx, a, at);
}

fn press(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2) {
    let button = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    tall(ctx, a, vec![Event::PointerMoved(at)]);
    tall(ctx, a, vec![button(true)]);
    tall(ctx, a, vec![button(false)]);
}

#[test]
fn plain_text_is_in_more_actions_the_view_ribbon_and_options() {
    let (mut a, dir) = app("plain-text");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    tall(&ctx, &mut a, vec![]);
    let key = a.session.list.iter().find(|m| m.subject == "Open call: winter group show").map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": key.to_id()}));
    for _ in 0..3 {
        tall(&ctx, &mut a, vec![]);
    }
    // Formatted, with nothing over its text to turn it plain.
    assert!(a.session.prefs.show_html);
    assert!(shows(&ctx, &mut a, "Tracking pictures, never loaded: 1 · Links that report clicks: 1"), "the HTML bar is drawn");
    assert!(!shows(&ctx, &mut a, "Show as plain text"), "no plain-text link over the message");
    assert!(!shows(&ctx, &mut a, "Show formatted"));

    // The message's More actions (⋯) has it, for that message: the setting stays.
    click_named(&ctx, &mut a, "More actions");
    assert!(shows(&ctx, &mut a, "Show as plain text"), "in More actions");
    click_named(&ctx, &mut a, "Show as plain text");
    tall(&ctx, &mut a, vec![]);
    assert!(a.session.prefs.show_html, "one message's More actions left the setting for all mail");
    // Shown as text, said, with the way back above the text.
    assert!(shows(&ctx, &mut a, "Shown as text."), "More actions turned it plain");
    click_named(&ctx, &mut a, "Show formatted");
    tall(&ctx, &mut a, vec![]);
    assert!(a.session.prefs.show_html);
    assert!(!shows(&ctx, &mut a, "Shown as text."), "Show formatted went back");

    // View › Plain text, both ways.
    a.ui.tab = crate::RibbonTab::View;
    tall(&ctx, &mut a, vec![]);
    click_named(&ctx, &mut a, "Plain text");
    assert!(!a.session.prefs.show_html, "View › Plain text");
    click_named(&ctx, &mut a, "Plain text");
    assert!(a.session.prefs.show_html, "View › Plain text, again");

    // File › Options: read all mail as plain text.
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
    for _ in 0..2 {
        tall(&ctx, &mut a, vec![]);
    }
    // Its switch: right of its label.
    let at = nodes(&ctx, &mut a)
        .into_iter()
        .filter(|(n, _)| n == "Read all mail as plain text")
        .filter_map(|(_, at)| at)
        .max_by(|p, q| p.x.total_cmp(&q.x));
    press(&ctx, &mut a, at.expect("the Options switch is shown"));
    assert!(!a.session.prefs.show_html, "the Options switch");

    // All mail as plain text: Show formatted above one message shows that one formatted, and
    // the setting stays on (as Outlook's Display as HTML does).
    a.ui.backstage = None;
    for _ in 0..2 {
        tall(&ctx, &mut a, vec![]);
    }
    assert!(shows(&ctx, &mut a, "Shown as text."), "read as plain text");
    click_named(&ctx, &mut a, "Show formatted");
    tall(&ctx, &mut a, vec![]);
    assert!(!shows(&ctx, &mut a, "Shown as text."), "this message formatted");
    assert!(!a.session.prefs.show_html, "Show formatted left the setting for all mail on");
    // Another message is still read as plain text.
    let other = a.session.list.iter().find(|m| m.key != key && m.subject != "Open call: winter group show").map(|m| m.key.clone()).unwrap();
    assert!(a.ui.shown_as.formatted(&key, false) && !a.ui.shown_as.formatted(&other, false), "only that one");

    // The command keeps its id, for the CLI and --cmd.
    a.run("view.html", json!({"on": true}));
    assert!(a.session.prefs.show_html);
    let _ = std::fs::remove_dir_all(dir);
}

/// Right-click in the message's text, as Outlook's menu: Copy the selection — greyed out, and
/// copying nothing, when nothing is selected — and the whole message as it is written.
#[test]
fn right_click_in_the_message_copies_only_with_a_selection() {
    let (mut a, dir) = app("reading-copy-menu");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    let key = a.session.list.iter().find(|m| m.subject == "Storyboard review on Thursday").map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": key.to_id()}));
    let body = "The animatic is in good shape.";
    let mut at = None;
    for _ in 0..30 {
        let shapes = frame_shapes(&ctx, &mut a, vec![], SCREEN);
        let found = shapes.iter().rev().find_map(|s| match s {
            egui::Shape::Text(t) if t.galley.text().contains(body) => {
                Some(t.pos + egui::vec2(t.galley.rect.width() * 0.5, t.galley.rect.height() - 3.0))
            }
            _ => None,
        });
        if found.is_some() {
            at = found;
            break;
        }
    }
    let at = at.expect("the body is drawn");
    let right_click = |a: &mut MailApp, at: egui::Pos2| {
        let b = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE };
        tall(&ctx, a, vec![Event::PointerMoved(at)]);
        tall(&ctx, a, vec![b(true)]);
        tall(&ctx, a, vec![b(false)]);
        tall(&ctx, a, vec![]);
    };
    // Click the menu's `label` (the last node of that name: the popup's), and what the release
    // frame put on the clipboard.
    let click_menu = |a: &mut MailApp, label: &str| -> Option<String> {
        let all = nodes(&ctx, a);
        let at = all.iter().filter(|(n, _)| n == label).filter_map(|(_, at)| *at).next_back();
        let at = at.unwrap_or_else(|| panic!("no {label} to click: {:?}", all.iter().map(|(n, _)| n).collect::<Vec<_>>()));
        let press = Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE };
        let release = Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE };
        tall(&ctx, a, vec![Event::PointerMoved(at)]);
        tall(&ctx, a, vec![press]);
        let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SCREEN)), events: vec![release], ..Default::default() };
        let out = ctx.run_ui(input, |ui| a.ui(ui));
        out.platform_output.commands.into_iter().find_map(|c| match c {
            egui::OutputCommand::CopyText(t) => Some(t),
            _ => None,
        })
    };
    // Nothing selected: the menu offers Copy, greyed out — clicking it copies nothing.
    right_click(&mut a, at);
    assert!(shows(&ctx, &mut a, "Copy"), "the menu opens with Copy");
    // The Copy item has its icon: a small painted shape beside the label, as Outlook's menu.
    let all = nodes(&ctx, &mut a);
    let copy_at = all.iter().filter(|(n, _)| n == "Copy").filter_map(|(_, at)| *at).next_back().expect("Copy");
    let shapes = frame_shapes(&ctx, &mut a, vec![], SCREEN);
    let icon = shapes.iter().any(|s| match s {
        egui::Shape::Text(_) => false,
        sh => {
            let r = sh.visual_bounding_rect();
            let c = r.center();
            r.width() <= 24.0 && r.height() <= 24.0 && c.x < copy_at.x && c.x > copy_at.x - 110.0 && (c.y - copy_at.y).abs() < 12.0
        }
    });
    assert!(icon, "the Copy item has its icon beside the label");
    assert!(click_menu(&mut a, "Copy").is_none(), "greyed out: nothing is copied");
    // Something selected (Ctrl+A here): Copy is offered and copies it.
    press(&ctx, &mut a, at);
    tall(&ctx, &mut a, vec![Event::Key { key: Key::A, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND }]);
    right_click(&mut a, at);
    let text = click_menu(&mut a, "Copy").expect("with a selection, Copy copies it");
    assert!(text.contains(body), "the selection is copied: {text:?}");
    let _ = std::fs::remove_dir_all(dir);
}
