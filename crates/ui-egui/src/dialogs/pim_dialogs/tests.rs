// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Tests of the forms' rows: a label longer than its column wraps inside it.

use super::*;

/// A label too long for its column (French's "Téléphone professionnel", "Anniversaire de
/// mariage") goes on two lines inside it, the row as tall as that: the field after it never runs
/// over its end.
#[test]
fn a_long_label_wraps_inside_its_column() {
    let ctx = egui::Context::default();
    let t = theme::Tokens::light();
    let mut row = None;
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        let top = ui.cursor().top();
        let left = ui.cursor().left();
        form_row(ui, &t, false, None, "Wedding anniversary of the person", |ui| {
            ui.add(egui::TextEdit::singleline(&mut String::new()).desired_width(200.0));
        });
        row = Some((left, ui.cursor().top() - top));
    });
    out.textures_delta.clear();
    let (left, height) = row.expect("drawn");
    let label = out
        .shapes
        .iter()
        .find_map(|c| match &c.shape {
            egui::Shape::Text(s) if s.galley.text().starts_with("Wedding") => Some((s.galley.rect.translate(s.pos.to_vec2()), s.galley.rows.len())),
            _ => None,
        })
        .expect("the label drawn");
    assert!(label.1 >= 2, "on two lines: {label:?}");
    assert!(label.0.max.x <= left + LABEL_W, "inside its column: {label:?}");
    assert!(height >= label.0.height(), "the row as tall as the label: {height} {label:?}");
}
