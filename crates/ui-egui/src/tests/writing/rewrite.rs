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
