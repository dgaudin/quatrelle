// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The scam check's banner: red from a score of 70, orange from 40, the model's reasons as plain
//! text (never a link, never markup), and the « Not a scam » link.

use quatrelle_engine::model::scam::{ALARM_SCORE, MAX_REASONS};

use super::*;

/// The banner's first line for a score of 40 or more (also the list's tooltip).
pub(crate) fn scam_headline(score: u8) -> &'static str {
    if score >= ALARM_SCORE { tl!("This message looks like a scam.") } else { tl!("This message could be a scam.") }
}

/// The banner. `advice`: say not to open links or attachments (left out when the sender check's
/// banner above already did). `true` when « Not a scam » was clicked.
pub(crate) fn scam_banner(t: &Tokens, ui: &mut Ui, score: u8, reasons: &[String], advice: bool) -> bool {
    let color = if score >= ALARM_SCORE { t.danger } else { t.warn };
    let mut clicked = false;
    egui::Frame::new().fill(color.gamma_multiply(0.12)).corner_radius(6).inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
            icons::paint(ui.painter(), r, Icon::Warning, color);
            ui.vertical(|ui| {
                ui.label(RichText::new(scam_headline(score)).color(t.text));
                for reason in reasons.iter().take(MAX_REASONS) {
                    ui.add(egui::Label::new(RichText::new(reason.as_str()).color(t.text).small()));
                }
                if advice {
                    ui.label(
                        RichText::new(tl!("Don't open its links or attachments, or answer it, unless you're sure who sent it.")).color(t.dim).small(),
                    );
                }
                clicked =
                    ui.add(egui::Label::new(RichText::new(tl!("Not a scam")).color(t.accent).small().underline()).sense(Sense::click())).clicked();
            });
        });
    });
    ui.add_space(8.0);
    clicked
}
