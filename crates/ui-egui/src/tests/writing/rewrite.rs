// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Rewrite where it's seen: beside Discard on a reply written in the reading pane, grey until
//! something is typed above the signature and the message quoted.

use super::*;

/// Rewrite was under the Review tab only, and on before anything was typed.
#[test]
fn rewrite_is_beside_discard_and_grey_until_something_is_typed() {
    let (mut a, dir) = app("rewrite-bar");
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 800.0);
    a.run("assistant.settings", json!({"url": "http://localhost:11434/v1", "model": "llama3"}));
    let k = a.session.list.first().map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    a.run("mail.reply", json!({}));
    let rewrite = settled_text_at(&ctx, &mut a, "Rewrite", size).expect("Rewrite on the reply's bar");
    let discard = settled_text_at(&ctx, &mut a, "Discard", size).expect("Discard");
    assert!((rewrite.y - discard.y).abs() < 4.0 && rewrite.x > discard.x, "beside Discard");
    // Nothing typed yet (the reply holds the signature and the message quoted): no choices.
    click_at(&ctx, &mut a, rewrite, 1, size);
    assert!(settled_text_at(&ctx, &mut a, "Shorter", size).is_none(), "grey until something is typed");
    // Something typed: its choices.
    if let Some(c) = a.session.compose.as_mut() {
        c.body = format!("Thanks, noted\n\n{}", c.body);
        c.styles = quatrelle_engine::mime::rich::Styles::plain(&c.body);
    }
    click_at(&ctx, &mut a, rewrite, 1, size);
    assert!(settled_text_at(&ctx, &mut a, "Shorter", size).is_some(), "its choices");
    let _ = std::fs::remove_dir_all(dir);
}

/// Asking a rewrite takes the keyboard from the menu; the text must get it back, or Ctrl+Z,
/// Ctrl+Y, typing and the right-click menu stop working until it's clicked again.
#[test]
fn asking_a_rewrite_leaves_the_editor_alive() {
    let (mut a, dir) = app("rewrite-alive");
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 800.0);
    a.run("assistant.settings", json!({"url": "http://localhost:11434/v1", "model": "llama3"}));
    a.run("list.open", json!({"key": "sample:1:1:INBOX"}));
    a.run("mail.reply", json!({}));
    if let Some(c) = a.session.compose.as_mut() {
        c.body = format!("Thanks, noted\n\n{}", c.body);
        c.styles = quatrelle_engine::mime::rich::Styles::plain(&c.body);
    }
    let id = crate::rich_edit::body_id(a.session.compose.as_ref().unwrap());
    ctx.memory_mut(|m| m.request_focus(id));
    frame(&ctx, &mut a, vec![]);
    let rewrite = settled_text_at(&ctx, &mut a, "Rewrite", size).expect("Rewrite");
    click_at(&ctx, &mut a, rewrite, 1, size);
    let shorter = settled_text_at(&ctx, &mut a, "Shorter", size).expect("Shorter");
    click_at(&ctx, &mut a, shorter, 1, size);
    frame(&ctx, &mut a, vec![]);
    assert!(ctx.memory(|m| m.has_focus(id)), "the text has the keyboard after asking Rewrite");
    // Typing goes in, rather than nowhere, and Ctrl+Z/Ctrl+Y still work.
    let before = a.session.compose.as_ref().unwrap().body.clone();
    frame(&ctx, &mut a, vec![Event::Text("!".into())]);
    let typed = a.session.compose.as_ref().unwrap().body.clone();
    assert_ne!(typed, before, "typing still reaches the text");
    frame(&ctx, &mut a, vec![key(Key::Z, Modifiers::COMMAND)]);
    assert_eq!(a.session.compose.as_ref().unwrap().body, before, "Ctrl+Z still works");
    frame(&ctx, &mut a, vec![key(Key::Y, Modifiers::COMMAND)]);
    assert_eq!(a.session.compose.as_ref().unwrap().body, typed, "Ctrl+Y still works");
    // The right-click menu opens on the text too.
    let at = a.ui.editor.as_ref().and_then(|ed| ed.drawn.clone()).map_or(egui::pos2(300.0, 300.0), |d| d.rect.center());
    frame(&ctx, &mut a, vec![Event::PointerMoved(at)]);
    let button = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE };
    frame(&ctx, &mut a, vec![button(true)]);
    frame(&ctx, &mut a, vec![button(false)]);
    assert!(settled_text_at(&ctx, &mut a, "Select all", size).is_some(), "the right-click menu still opens");
    let _ = std::fs::remove_dir_all(dir);
}

/// Using the rewritten text is one undo step in the editor: Ctrl+Z gives back the text, its
/// styles and its picture; Ctrl+Y puts the rewrite back.
#[test]
fn using_a_rewrite_is_one_undo_step() {
    let (mut a, dir) = app("rewrite-undo");
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 800.0);
    a.run("assistant.settings", json!({"url": "http://localhost:11434/v1", "model": "llama3"}));
    a.run("mail.new", json!({}));
    let (text, styles) = quatrelle_engine::mime::rich::from_markup("**Bonjour**\n\n![logo](cid:logo@studio.example)");
    if let Some(c) = a.session.compose.as_mut() {
        c.signature.clear();
        c.body = text;
        c.styles = styles;
        c.inline = vec![quatrelle_engine::mime::InlinePicture {
            cid: "logo@studio.example".into(),
            name: "logo.png".into(),
            mime: "image/png".into(),
            data: Vec::new(),
        }];
    }
    let id = crate::rich_edit::body_id(a.session.compose.as_ref().unwrap());
    ctx.memory_mut(|m| m.request_focus(id));
    frame(&ctx, &mut a, vec![]);
    let original = a.session.compose.as_ref().unwrap().clone();
    a.run("assistant.rewrite", json!({"how": "shorter"}));
    let ask = a.session.take_jobs().into_iter().find_map(|j| match j {
        quatrelle_engine::model::Job::AskLlm { id, .. } => Some(id),
        _ => None,
    });
    let ask = ask.expect("the rewrite was asked");
    a.session.apply(quatrelle_engine::model::Event::LlmAnswered { id: ask, text: "Salut".into(), error: None });
    frame(&ctx, &mut a, vec![]);
    let use_it = settled_text_at(&ctx, &mut a, "Use this text", size).expect("Use this text");
    click_at(&ctx, &mut a, use_it, 1, size);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    assert!(ctx.memory(|m| m.has_focus(id)), "the text has the keyboard after using the rewrite");
    let after = a.session.compose.as_ref().unwrap().clone();
    assert_ne!(after.body, original.body, "the rewrite went in");
    // Ctrl+Z: exactly the text, styles and picture of before.
    frame(&ctx, &mut a, vec![key(Key::Z, Modifiers::COMMAND)]);
    let back = a.session.compose.as_ref().unwrap();
    assert_eq!((back.body.clone(), back.styles.clone()), (original.body.clone(), original.styles.clone()), "one step back");
    assert_eq!(back.inline, original.inline, "the picture is the same");
    // Ctrl+Y: the rewrite again.
    frame(&ctx, &mut a, vec![key(Key::Y, Modifiers::COMMAND)]);
    let again = a.session.compose.as_ref().unwrap();
    assert_eq!((again.body.clone(), again.styles.clone()), (after.body.clone(), after.styles.clone()), "one step forward");
    let _ = std::fs::remove_dir_all(dir);
}
