// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: a message that says something is attached and carries nothing asks its own question
//! (Attach a file, Send, Cancel), not the security check's "Send anyway": one clicked for nothing
//! at every message would be clicked the day a lookalike address is caught too. With a security
//! check as well, that one comes first, then this one.

use super::*;

/// Every text drawn.
fn texts(shapes: &[egui::Shape]) -> Vec<String> {
    shapes.iter().filter_map(|s| if let egui::Shape::Text(t) = s { Some(t.galley.text().to_string()) } else { None }).collect()
}

/// Where `text` is drawn under the dialog's title `title` (the form has its own buttons).
fn under(shapes: &[egui::Shape], title: &str, text: &str) -> Option<egui::Pos2> {
    let at = |t: &egui::epaint::TextShape| t.galley.rect.translate(t.pos.to_vec2());
    let top = shapes.iter().find_map(|s| match s {
        egui::Shape::Text(t) if t.galley.text() == title => Some(at(t)),
        _ => None,
    })?;
    shapes.iter().rev().find_map(|s| match s {
        egui::Shape::Text(t) if t.galley.text() == text && at(t).top() > top.bottom() && (at(t).center().x - top.center().x).abs() < 400.0 => {
            Some(at(t).center())
        }
        _ => None,
    })
}

#[test]
fn a_file_said_attached_and_missing_asks_its_own_question() {
    let (mut a, dir) = app("reminder-own");
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 800.0);
    let settled = |ctx: &egui::Context, a: &mut MailApp| {
        let mut shapes = Vec::new();
        for _ in 0..6 {
            shapes = frame_shapes(ctx, a, vec![], size);
        }
        shapes
    };
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"to": "ana@soundlab.example", "subject": "Plans", "body": "The plan is attached."}));
    a.run("compose.send", json!({}));
    let shapes = settled(&ctx, &mut a);
    let drawn = texts(&shapes);
    for own in ["Send without a file?", "You wrote that something is attached, but nothing is.", "Attach a file"] {
        assert!(drawn.iter().any(|t| t == own), "its own question: {own} in {drawn:?}");
    }
    assert!(!drawn.iter().any(|t| t == "Send anyway" || t == "Check before sending"), "not the security check's: {drawn:?}");
    // Its Send sends the message as it is.
    let send = under(&shapes, "Send without a file?", "Send").expect("the question's Send");
    click_at(&ctx, &mut a, send, 1, size);
    assert!(a.session.compose.is_none(), "sent as it is");
    // A lookalike address too: the security check first, without the file's line; then the file's question.
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"to": "orders@printsh0p.example", "subject": "Proof", "body": "The proof is attached."}));
    a.run("compose.send", json!({}));
    let shapes = settled(&ctx, &mut a);
    let drawn = texts(&shapes);
    assert!(drawn.iter().any(|t| t == "Check before sending") && drawn.iter().any(|t| t.contains("printsh0p.example")), "{drawn:?}");
    assert!(!drawn.iter().any(|t| t.contains("You wrote that something is attached")), "the file's question waits: {drawn:?}");
    let anyway = under(&shapes, "Check before sending", "Send anyway").expect("Send anyway");
    click_at(&ctx, &mut a, anyway, 1, size);
    let drawn = texts(&settled(&ctx, &mut a));
    assert!(drawn.iter().any(|t| t == "Send without a file?"), "then the file's question: {drawn:?}");
    assert!(a.session.compose.as_ref().is_some_and(|c| !c.sending), "not sent yet");
    let _ = std::fs::remove_dir_all(dir);
}
