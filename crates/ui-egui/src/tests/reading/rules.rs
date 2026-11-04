// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A line across the message between the exchanges a reply quotes, as Outlook draws it.

use super::*;

/// Outlook's reply puts a thin line over each message it quotes (a top border over « De : »):
/// drawn across the pane, between the text written and the message quoted. They ran together.
#[test]
fn a_line_is_drawn_between_the_text_and_the_message_quoted() {
    let html = r#"<html><body><p class=MsoNormal>Cordialement,</p>
        <div style='border:none;border-top:solid #E1E1E1 1.0pt;padding:3.0pt 0cm 0cm 0cm'>
        <p class=MsoNormal><b>De&nbsp;:</b> Fred Lopes<br><b>Envoyé&nbsp;:</b> mercredi 4 novembre 2026</p></div>
        <p class=MsoNormal>Le texte cité</p></body></html>"#;
    let (mut a, dir) = app_with_html("quote-line", "Re: Planning", html);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list.iter().find(|m| m.subject == "Re: Planning").map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    let size = egui::vec2(1280.0, 800.0);
    let above = settled_text_at(&ctx, &mut a, "Cordialement,", size).expect("the text written");
    let shapes = frame_shapes(&ctx, &mut a, vec![], size);
    let below = text_at(&shapes, "Le texte cité").expect("the message quoted");
    let line = shapes.iter().any(|s| match s {
        // Across the message's column (the list's rows have lines of their own beside it).
        egui::Shape::LineSegment { points: [a, b], .. } => {
            (a.y - b.y).abs() < 0.5 && (b.x - a.x).abs() > 300.0 && a.y > above.y && a.y < below.y && a.x.min(b.x) < above.x && a.x.max(b.x) > above.x
        }
        _ => false,
    });
    assert!(line, "a line across, between them");
    let _ = std::fs::remove_dir_all(dir);
}
