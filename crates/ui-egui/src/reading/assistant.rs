//! The reading pane: Summarize, with the language model set in Options, and its answer above
//! the message.

use super::*;

/// Summarize, beside Forward, once a language model is set.
pub(super) fn button(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    if s.prefs.llm_url.trim().is_empty() {
        return;
    }
    let busy = s.assistant.as_ref().is_some_and(|a| a.answer.is_none());
    let summary = widgets::pill_button(ui, Icon::ListBullet, "Summarize", t)
        .on_hover_text(tl!("Ask the language model set in Options for the gist of this message"));
    if summary.clicked() && !busy {
        acts.push(Action::Run("assistant.summarize", json!({})));
    }
    let reply = widgets::pill_button(ui, Icon::Drafts, "Suggest a reply", t)
        .on_hover_text(tl!("Ask the language model set in Options to write a reply to this message"));
    if reply.clicked() && !busy {
        acts.push(Action::Run("assistant.suggestReply", json!({})));
    }
}

/// The language model's answer about this message, or that it's working on it.
pub(super) fn panel(s: &Session, t: &Tokens, ui: &mut Ui, key: &MessageKey, acts: &mut Vec<Action>) {
    let Some(a) = s.assistant.as_ref().filter(|a| a.key == *key) else { return };
    egui::Frame::new().fill(t.accent_soft).corner_radius(8).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(tl!(if a.reply { "Suggested reply" } else { "Summary" })).font(theme::bold(13.0)).color(t.text));
        ui.add_space(2.0);
        match &a.answer {
            None => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(tl!("Asking the language model…")).color(t.dim));
                });
            }
            Some(Ok(text)) => {
                ui.add(egui::Label::new(RichText::new(text).color(t.text)).selectable(true));
                if a.reply {
                    ui.add_space(4.0);
                    if widgets::pill_button(ui, Icon::Reply, "Reply with this text", t).clicked() {
                        acts.push(Action::Run("assistant.replyWithSuggestion", json!({})));
                    }
                }
            }
            Some(Err(e)) => {
                ui.label(RichText::new(tf!("The language model couldn't answer: {error}", error = e)).color(t.warn));
            }
        }
    });
    ui.add_space(8.0);
}

/// The text written, rewritten by the language model: to put in its place, or let go.
pub(crate) fn rewrite_panel(t: &Tokens, ui: &mut Ui, c: &quatrelle_engine::Compose, acts: &mut Vec<Action>) {
    let Some(r) = &c.rewrite else { return };
    ui.add_space(6.0);
    egui::Frame::new().fill(t.accent_soft).corner_radius(8).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(tl!("Rewritten text")).font(theme::bold(13.0)).color(t.text));
        ui.add_space(2.0);
        match &r.answer {
            None => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(tl!("Asking the language model…")).color(t.dim));
                });
            }
            Some(Ok(text)) => {
                egui::ScrollArea::vertical().id_salt(("rewrite", c.id)).max_height(220.0).show(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(text).color(t.text)).selectable(true));
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if widgets::pill_button(ui, Icon::Complete, "Use this text", t)
                        .on_hover_text(tl!("In place of what you wrote (its formatting goes)"))
                        .clicked()
                    {
                        acts.push(Action::Run("assistant.endRewrite", json!({"use": true})));
                    }
                    if widgets::pill_button(ui, Icon::Close, "Keep mine", t).clicked() {
                        acts.push(Action::Run("assistant.endRewrite", json!({"use": false})));
                    }
                });
            }
            Some(Err(e)) => {
                ui.label(RichText::new(tf!("The language model couldn't answer: {error}", error = e)).color(t.warn));
                if widgets::pill_button(ui, Icon::Close, "Close", t).clicked() {
                    acts.push(Action::Run("assistant.endRewrite", json!({"use": false})));
                }
            }
        }
    });
}
