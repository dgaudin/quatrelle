// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! What the reading pane says about who sent a message: the warning when the sender doesn't add
//! up (a look-alike domain, an address in the name, a known person's name on another address,
//! a signature giving another address, a believed address the server couldn't confirm), and the
//! External tag beside the sender's name.

use super::*;

/// Who the message says it's from doesn't add up: said plainly, above everything it asks for.
pub(crate) fn sender_warning(t: &Tokens, ui: &mut Ui, w: &quatrelle_engine::model::lookalike::SenderWarning) {
    use quatrelle_engine::model::lookalike::SenderWarning;
    let what = match w {
        SenderWarning::Lookalike { domain, like } => {
            tf!("Careful: this comes from {domain}, which looks like {like} but isn't it.", domain = domain, like = like)
        }
        SenderWarning::NameShowsOther { shown, real } => {
            tf!("Careful: the sender's name shows {shown}, but this comes from {real}.", shown = shown, real = real)
        }
        SenderWarning::NameOfKnownPerson { name, known, real } => {
            tf!("Careful: you know {name} as {known}, but this comes from {real}.", name = name, known = known, real = real)
        }
        SenderWarning::SignatureShowsOther { shown, real } => {
            tf!("Careful: the signature gives {shown}, but this comes from {real}.", shown = shown, real = real)
        }
        SenderWarning::NotConfirmed { real } => {
            tf!("Careful: this says it comes from {real}, but the mail server couldn't confirm that (DMARC failed).", real = real)
        }
    };
    egui::Frame::new().fill(t.danger.gamma_multiply(0.12)).corner_radius(6).inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
            icons::paint(ui.painter(), r, Icon::Warning, t.danger);
            ui.vertical(|ui| {
                ui.label(RichText::new(what).color(t.text));
                ui.label(
                    RichText::new(tl!("Don't open its links or attachments, or answer it, unless you're sure who sent it.")).color(t.dim).small(),
                );
            });
        });
    });
    ui.add_space(8.0);
}

/// The External tag beside the sender's name: a sender from outside the organisation. One line
/// however narrow the pane (wrapped, its word stood in a column a letter wide).
pub(crate) fn external_tag(ui: &mut Ui, t: &Tokens) -> egui::Response {
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, t.border))
        .corner_radius(4)
        .inner_margin(Margin::symmetric(5, 1))
        .show(ui, |ui| {
            ui.add(egui::Label::new(RichText::new(tl!("External")).color(t.dim).small()).wrap_mode(egui::TextWrapMode::Extend));
        })
        .response
        .on_hover_text(tl!("This sender is outside your organisation."))
}
