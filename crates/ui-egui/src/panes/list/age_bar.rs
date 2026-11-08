// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The bar above the list that offers old mail for the Archives account (archiving by age), and
//! the line that says how far it has got. The engine counts and the worker archives: the window
//! only shows and asks.

use crate::*;

/// The bar for the first connected account with mail to offer, and the progress line while an
/// archiving by age runs.
pub(super) fn show(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    if let Some((done, total)) = s.archive_progress() {
        ui.add_space(4.0);
        ui.label(RichText::new(tf!("Archiving: {done} of {total}", done = done, total = total)).color(t.dim).small());
    }
    let Some((account, count, months)) = s.accounts.iter().find_map(|a| s.archive_offer(&a.config.id).map(|(n, m)| (a, n, m))) else {
        return;
    };
    let id = account.config.id.clone();
    ui.add_space(4.0);
    egui::Frame::new().fill(t.accent_soft).corner_radius(4).inner_margin(egui::Margin::symmetric(8, 5)).show(ui, |ui| {
        ui.vertical(|ui| {
            let said = tf!(
                "{count} messages older than {months} months in {account} can go to Archives.",
                count = count,
                months = months,
                account = account.config.name
            );
            ui.add(egui::Label::new(RichText::new(said).color(t.text).small()).wrap());
            ui.horizontal(|ui| {
                if ui.small_button(tl!("Archive")).clicked() {
                    acts.push(Action::Run("archive.byAge", json!({"account": id})));
                }
                if ui.small_button(tl!("Not now")).clicked() {
                    acts.push(Action::Run("archive.notNow", json!({"account": id})));
                }
            });
        });
    });
    ui.add_space(2.0);
}
