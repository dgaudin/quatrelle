// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The reading pane's header as Outlook's: Reply | Reply all | Forward as one group, Assistant
//! and ⋯ beside it, at the sender's right in a wide pane; on a line of their own under the
//! header in a narrower one, their icons alone when even that is too narrow. Print and Save went
//! into ⋯, the assistant's three into Assistant.

use super::*;

/// The sample's first message open in a window `width` wide, with a language model set or not;
/// the sender's name as the header writes it.
fn opened(name: &str, width: f32, llm: bool) -> (MailApp, std::path::PathBuf, egui::Vec2, String) {
    let (mut a, dir) = app(name);
    if llm {
        a.run("assistant.settings", json!({"url": "http://localhost:11434/v1", "model": "llama3"}));
    }
    let k = a.session.list.first().map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    let sender = a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.from.display().to_string()).unwrap_or_default();
    (a, dir, egui::vec2(width, 800.0), sender)
}

#[test]
fn the_reply_group_sits_at_the_senders_right_in_a_wide_pane() {
    let (mut a, dir, size, sender) = opened("head-wide", 2000.0, true);
    let ctx = egui::Context::default();
    let group = settled_text_at(&ctx, &mut a, "Reply all", size).expect("the reply group");
    let shapes = frame_shapes(&ctx, &mut a, vec![], size);
    let from = text_at(&shapes, &sender).expect("the sender");
    assert!((group.y - from.y).abs() < 12.0 && group.x > from.x + 200.0, "at the sender's right: {group:?} {from:?}");
    let assistant = text_at(&shapes, "Assistant").expect("Assistant beside the group");
    assert!((assistant.y - group.y).abs() < 2.0 && assistant.x > group.x);
    for gone in ["Print", "Save", "Summarize"] {
        assert!(text_at(&shapes, gone).is_none(), "{gone} is in a menu now");
    }
    click_at(&ctx, &mut a, assistant, 1, size);
    assert!(settled_text_at(&ctx, &mut a, "Summarize", size).is_some(), "Assistant's menu");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_reply_group_goes_under_the_header_when_the_pane_is_narrower() {
    let (mut a, dir, size, sender) = opened("head-narrow", 1280.0, true);
    let ctx = egui::Context::default();
    let group = settled_text_at(&ctx, &mut a, "Reply all", size).expect("the reply group, with its words");
    let from = text_at(&frame_shapes(&ctx, &mut a, vec![], size), &sender).expect("the sender");
    assert!(group.y > from.y + 30.0, "under the header: {group:?} {from:?}");
    let _ = std::fs::remove_dir_all(dir);
}
