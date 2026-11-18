// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A message window's File page, as Outlook's message window has its own: the message's details,
//! and what's done with the message as a whole — Save As, Save Attachments, Print, its headers —
//! each one of the commands the main window's File page offers for the message read, acting here
//! on the window's message. It takes the window's place until Back (or one of them) is chosen.

use crate::*;

/// Where egui keeps whether `key`'s window shows its File page.
fn id(key: &MessageKey) -> egui::Id {
    egui::Id::unique(("message-file-page", key.to_id()))
}

/// Whether `key`'s window shows its File page.
pub(super) fn shown(ctx: &egui::Context, key: &MessageKey) -> bool {
    ctx.data(|d| d.get_temp::<bool>(id(key))).unwrap_or(false)
}

/// Show `key`'s window's File page, or the message again.
pub(super) fn show(ctx: &egui::Context, key: &MessageKey, on: bool) {
    ctx.data_mut(|d| d.insert_temp(id(key), on));
}

/// The page: its rail at the left (Back, Info, what's done with the message, Close), the message's
/// details at the right. Drawn while the window's message stands in as the one selected and read.
pub(super) fn page(ui: &mut Ui, s: &Session, t: &Tokens, w: &quatrelle_engine::Reading, acts: &mut Vec<Action>) {
    let key = &w.key;
    let p = w.parsed.as_ref();
    let mut back = false;
    egui::Panel::left(egui::Id::unique(("message-file-rail", key.to_id())))
        .resizable(false)
        .exact_size(220.0)
        .frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(0, 12)).stroke(egui::Stroke::new(1.0, t.border)))
        .show(ui, |ui| {
            if backstage::rail_item(ui, t, Icon::Back, "Back", false).clicked() {
                back = true;
            }
            ui.add_space(8.0);
            let _ = backstage::rail_item(ui, t, Icon::Info, "Info", true);
            ui.add_space(8.0);
            let attachments = p.is_some_and(|p| !p.attachments.is_empty());
            for (icon, label, on, action) in [
                (Icon::File, "Save As", p.is_some(), Action::SaveMessage),
                (Icon::Attach, "Save Attachments", attachments, Action::SaveAll),
                (Icon::Print, "Print", p.is_some(), Action::Print),
                (Icon::Info, "Message headers", p.is_some(), Action::Run("message.headers", json!({}))),
            ] {
                let r = ui.add_enabled_ui(on, |ui| backstage::rail_item(ui, t, icon, label, false)).inner;
                if on && r.clicked() {
                    acts.push(action);
                    back = true;
                }
            }
            ui.add_space(8.0);
            if backstage::rail_item(ui, t, Icon::Close, "Close", false).clicked() {
                acts.push(Action::Run("message.closeWindow", json!({"key": key.to_id()})));
            }
        });
    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.bg)).show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink(false).id_salt(("message-file-info", key.to_id())).show(ui, |ui| {
            egui::Frame::new().inner_margin(Margin { left: 36, right: 36, top: 24, bottom: 32 }).show(ui, |ui| info(ui, s, t, w));
        });
    });
    if back {
        show(ui.ctx(), key, false);
    }
}

/// The message's details: its subject, then who it's from and to, when, how big, and where.
fn info(ui: &mut Ui, s: &Session, t: &Tokens, w: &quatrelle_engine::Reading) {
    ui.label(RichText::new(tl!("Info")).font(theme::bold(22.0)).color(t.text));
    ui.add_space(10.0);
    let Some(p) = w.parsed.as_ref() else { return };
    let subject = if p.subject.trim().is_empty() { tl!("(no subject)") } else { p.subject.as_str() };
    crate::bidi::label(ui, subject, theme::bold(16.0), t.text);
    ui.add_space(8.0);
    let summary = s.summary(&w.key);
    let when = if p.date > 0 { p.date } else { summary.map_or(0, |m| m.date) };
    let folder = s.folder(&w.key.account, &w.key.folder).map(crate::i18n::folder).unwrap_or_default();
    let account = s.account(&w.key.account).map(|a| a.config.email.clone()).unwrap_or_default();
    let rows = [
        (tl!("From"), p.from.to_string()),
        (tl!("To"), quatrelle_engine::model::addr::display_list(&p.to)),
        (tl!("Date"), if when > 0 { crate::i18n::date(&quatrelle_engine::model::dates::full_time(when, &s.clock)) } else { String::new() }),
        (tl!("Size"), summary.map(|m| widgets::size_text(u64::from(m.size))).unwrap_or_default()),
        (tl!("Folder"), if account.is_empty() { folder } else { format!("{folder} ({account})") }),
    ];
    egui::Grid::new(("message-file-details", w.key.to_id())).num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
        for (label, value) in rows.into_iter().filter(|(_, v)| !v.trim().is_empty()) {
            ui.label(RichText::new(label).color(t.dim));
            ui.label(RichText::new(value).color(t.text));
            ui.end_row();
        }
    });
}
