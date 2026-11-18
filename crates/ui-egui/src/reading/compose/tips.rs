// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The MailTips strip above the message form: what the server says about the current recipients,
//! drawn informational-only (nothing here holds a message back). The form asks again when the
//! recipients change (`mail_tips_wanted`), and the answer comes as `Event::MailTips`.

use quatrelle_engine::mail_tips_recipients;

use super::*;

/// The strip's lines are shown this high at most (a hundred recipients, each with an automatic
/// reply a letter long): the rest is scrolled to.
const STRIP_MAX_H: f32 = 90.0;

/// Draw the MailTips strip above the message body when the message's recipients match the ones
/// asked about and the server has something to say: nothing at all otherwise (a size limit the
/// message stays under, which Exchange gives for nearly every recipient, says nothing).
pub(super) fn draw_tips(s: &Session, st: &UiState, ui: &mut Ui, c: &quatrelle_engine::Compose) {
    let t = st.tokens;
    let recipients = mail_tips_recipients(c);
    let Some(state) = s.mail_tips_now(c.id) else { return };
    // The tips of a set of recipients since changed are let go (a new answer is on its way).
    if state.recipients != recipients || state.account != c.account {
        return;
    }
    let tips = match &state.found {
        quatrelle_engine::Fetched::Ready(t) if !t.is_empty() => t,
        _ => return,
    };
    let size = c.estimated_size();
    let lines: Vec<String> = tips
        .iter()
        .filter_map(|tip| {
            let mut parts = Vec::new();
            if !tip.automatic_replies.is_empty() {
                parts.push(tip.automatic_replies.clone());
            }
            if tip.mailbox_full {
                parts.push(tl!("Mailbox full").to_string());
            }
            if !tip.custom.is_empty() {
                parts.push(tip.custom.clone());
            }
            if tip.restricted {
                parts.push(tl!("Delivery restricted").to_string());
            }
            if tip.external_members > 0 {
                parts.push(tf!("Reaches {n} people outside the organisation", n = tip.external_members));
            }
            if tip.moderated {
                parts.push(tl!("Moderated").to_string());
            }
            if tip.invalid {
                parts.push(tl!("Recipient not found").to_string());
            }
            if tip.max_message_size > 0 && size > tip.max_message_size {
                parts.push(tf!("This message is larger than the {size} this recipient accepts", size = widgets::size_text(tip.max_message_size)));
            }
            (!parts.is_empty()).then(|| tf!("{email}: {line}", email = tip.email, line = parts.join(" — ")))
        })
        .collect();
    if lines.is_empty() {
        return;
    }
    ui.add_space(4.0);
    egui::Frame::new().fill(t.chrome.gamma_multiply(0.85)).corner_radius(6).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let icon_rect = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover()).0;
            icons::paint(ui.painter(), icon_rect, Icon::Info, t.accent);
            ui.add_space(8.0);
            ui.vertical(|ui| {
                egui::ScrollArea::vertical().id_salt(("mail-tips", c.id)).max_height(STRIP_MAX_H).auto_shrink([false, true]).show(ui, |ui| {
                    for line in &lines {
                        ui.add(egui::Label::new(RichText::new(line.as_str()).color(t.text).size(12.0)).wrap());
                    }
                });
            });
        });
    });
    ui.add_space(4.0);
}
