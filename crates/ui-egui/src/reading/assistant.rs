// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The reading pane: Summarize, with the language model set in Options, and its answer above
//! the message.

use quatrelle_engine::AssistKind;

use super::*;

/// Assistant, beside the reply group once a language model is set: Summarize, Suggest a reply
/// and File it in its menu (greyed while a question is out); its icon alone when `compact`.
pub(super) fn button(s: &Session, t: &Tokens, ui: &mut Ui, compact: bool, acts: &mut Vec<Action>) {
    if s.prefs.llm_url.trim().is_empty() {
        return;
    }
    let busy = s.assistant.as_ref().is_some_and(|a| a.answer.is_none());
    let r = widgets::menu_pill(ui, Icon::Sparkle, "Assistant", t, true, compact);
    egui::Popup::menu(&r).show(|ui| {
        let asks = [
            ("assistant.summarize", Icon::ListBullet, "Summarize", "Ask the language model set in Options for the gist of this message"),
            ("assistant.suggestReply", Icon::Drafts, "Suggest a reply", "Ask the language model set in Options to write a reply to this message"),
            (
                "assistant.suggestFiling",
                Icon::Move,
                "File it",
                "Ask the language model set in Options which of your folders and categories this message belongs in",
            ),
        ];
        for (id, icon, label, tip) in asks {
            if ui.add_enabled(!busy, egui::Button::new((widgets::menu_icon(Some(icon)), tl!(label)))).on_hover_text(tl!(tip)).clicked() {
                acts.push(Action::Run(id, json!({})));
                ui.close();
            }
        }
    });
}

/// The room [`button`] takes beside the reply group, its gap with it: none without a language
/// model.
pub(super) fn width(ui: &Ui, s: &Session, compact: bool) -> f32 {
    if s.prefs.llm_url.trim().is_empty() { 0.0 } else { widgets::menu_pill_width(ui, tl!("Assistant"), compact) + 6.0 }
}

/// The language model's answer about this message, or that it's working on it.
pub(super) fn panel(s: &Session, t: &Tokens, ui: &mut Ui, key: &MessageKey, acts: &mut Vec<Action>) {
    let Some(a) = s.assistant.as_ref().filter(|a| a.key == *key) else { return };
    egui::Frame::new().fill(t.accent_soft).corner_radius(8).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let title = match a.kind {
            AssistKind::Summary => "Summary",
            AssistKind::Reply => "Suggested reply",
            AssistKind::Filing => "Where it goes",
        };
        // Its name, and a cross to put it away unused (a question still out with it).
        ui.horizontal(|ui| {
            ui.label(RichText::new(tl!(title)).font(theme::bold(13.0)).color(t.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::tiny_icon_button(ui, Icon::Close, "Close", t).clicked() {
                    acts.push(Action::Run("assistant.close", json!({})));
                }
            });
        });
        ui.add_space(2.0);
        match &a.answer {
            None => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(tl!("Asking the language model…")).color(t.dim));
                });
            }
            Some(Ok(_)) if a.kind == AssistKind::Filing => {
                let f = a.filing.clone().unwrap_or_default();
                if f.folder.is_none() && f.category.is_none() {
                    ui.label(RichText::new(tl!("The language model found none of your folders or categories for it.")).color(t.dim));
                }
                ui.horizontal_wrapped(|ui| {
                    if let Some((path, shown)) = &f.folder
                        && widgets::pill_button(ui, Icon::Move, &tf!("Move to {folder}", folder = shown), t).clicked()
                    {
                        acts.push(Action::Run("mail.move", json!({"folder": path})));
                    }
                    if let Some(c) = &f.category
                        && widgets::pill_button(ui, Icon::Tag, &tf!("Categorize as {category}", category = c), t).clicked()
                    {
                        acts.push(Action::Run("mail.categorize", json!({"category": c, "on": true})));
                    }
                });
            }
            Some(Ok(text)) => {
                ui.add(egui::Label::new(RichText::new(text).color(t.text)).selectable(true));
                if a.kind == AssistKind::Reply {
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
