// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Menus and marks for organising mail: Categorize, Snooze, rules, Find related.

use egui::{RichText, Sense, Ui, vec2};
use quatrelle_engine::Session;
use quatrelle_engine::model::dates;
use quatrelle_engine::model::organize::SnoozeUntil;
use serde_json::json;

use crate::dialogs::FollowUpForm;
use crate::theme::{self, Tokens};
use crate::{Action, Dialog};

/// A coloured dot for a category.
pub fn dot(ui: &mut Ui, color: u8, dark: bool) {
    let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
    ui.painter().circle_filled(r.center(), 5.0, theme::category_color(color, dark));
}

/// The Categorize menu: tick or untick a category on the selection, or make a new one.
pub fn categorize_menu(ui: &mut Ui, s: &Session, t: &Tokens, acts: &mut Vec<Action>) {
    let first = s.first_selected();
    for c in &s.prefs.categories {
        let on = first.as_ref().is_some_and(|m| c.marks(&m.flags));
        let clicked = ui
            .horizontal(|ui| {
                dot(ui, c.color, t.dark);
                ui.selectable_label(on, tl!(&c.name)).clicked()
            })
            .inner;
        if clicked {
            acts.push(Action::Run("mail.categorize", json!({"category": c.name, "on": !on})));
            ui.close();
        }
    }
    ui.separator();
    if ui.button(tl!("New category…")).clicked() {
        acts.push(Action::Dialog(Dialog::NewCategory {
            name: String::new(),
            color: u8::try_from(s.prefs.categories.len() % 8).unwrap_or(0),
            error: None,
        }));
        ui.close();
    }
}

/// The Snooze menu: the usual times, shown in local time.
pub fn snooze_menu(ui: &mut Ui, s: &Session, t: &Tokens, acts: &mut Vec<Action>) {
    for p in SnoozeUntil::ALL {
        let at = p.at(&s.clock);
        let when = dates::full_time(at, &s.clock);
        let clicked = ui
            .horizontal(|ui| {
                let r = ui.selectable_label(false, tl!(p.label()));
                ui.label(RichText::new(crate::i18n::date(&when)).color(t.faint).small());
                r.clicked()
            })
            .inner;
        if clicked {
            acts.push(Action::Run("mail.snooze", json!({"preset": p.code()})));
            ui.close();
        }
    }
}

/// The Follow Up menu, as Outlook's: flag for today, tomorrow, this week, next week or no day
/// (each with its date), Mark complete, Clear flag. `key`: the message a To-Do List row names
/// (the list's selection when none).
pub fn follow_up_menu(ui: &mut Ui, s: &Session, t: &Tokens, acts: &mut Vec<Action>, key: Option<&str>) {
    let params = |mut p: serde_json::Value| {
        if let (Some(k), Some(o)) = (key, p.as_object_mut()) {
            o.insert("key".into(), json!(k));
        }
        p
    };
    for f in dates::FollowUp::ALL {
        let label = crate::i18n::date(&dates::day_label(f.due(&s.clock)));
        let clicked = ui
            .horizontal(|ui| {
                let r = ui.selectable_label(false, tl!(f.label()));
                ui.label(RichText::new(label).color(t.faint).small());
                r.clicked()
            })
            .inner;
        if clicked {
            acts.push(Action::Run("mail.flag", params(json!({"due": f.code()}))));
            ui.close();
        }
    }
    if ui.selectable_label(false, tl!("No date")).clicked() {
        acts.push(Action::Run("mail.flag", params(json!({"due": "none"}))));
        ui.close();
    }
    if ui.button(tl!("Custom…")).clicked() {
        acts.push(Action::Dialog(Dialog::FollowUp(Box::new(FollowUpForm::of(s, true, key)))));
        ui.close();
    }
    if ui.button(tl!("Add Reminder…")).clicked() {
        acts.push(Action::Dialog(Dialog::FollowUp(Box::new(FollowUpForm::of(s, false, key)))));
        ui.close();
    }
    ui.separator();
    let flagged = match key {
        Some(k) => s.message_by_id(k).is_some_and(|m| m.flags.flagged),
        None => s.selected_iter().any(|m| m.flags.flagged),
    };
    if ui.add_enabled(flagged, egui::Button::new(tl!("Mark complete"))).clicked() {
        acts.push(Action::Run("mail.flag", params(json!({"complete": true}))));
        ui.close();
    }
    if ui.add_enabled(flagged, egui::Button::new(tl!("Clear flag"))).clicked() {
        acts.push(Action::Run("mail.flag", params(json!({"on": false}))));
        ui.close();
    }
}

/// Every organising item of the message context menu.
pub fn context_items(ui: &mut Ui, s: &Session, t: &Tokens, acts: &mut Vec<Action>, pinned: bool) {
    use crate::icons::Icon;
    use crate::widgets::menu_icon;
    ui.menu_button((menu_icon(Some(Icon::Tag)), tl!("Categorize")), |ui| categorize_menu(ui, s, t, acts));
    if ui.button((menu_icon(Some(Icon::Pin)), tl!(if pinned { "Unpin" } else { "Pin" }))).clicked() {
        acts.push(Action::Run("mail.pin", json!({"on": !pinned})));
        ui.close();
    }
    ui.menu_button((menu_icon(Some(Icon::Clock)), tl!("Snooze")), |ui| snooze_menu(ui, s, t, acts));
    if s.focus_split() {
        let (id, label) = if s.focus_other { ("mail.moveToFocused", "Always move to Focused") } else { ("mail.moveToOther", "Always move to Other") };
        if ui.button((menu_icon(Some(Icon::Inbox)), tl!(label))).clicked() {
            acts.push(Action::Run(id, json!({})));
            ui.close();
        }
    }
    if let Some(m) = s.first_selected()
        && ui.button((menu_icon(Some(Icon::MailRules)), tl!("Create rule…"))).clicked()
    {
        // A colleague's address may be Exchange's internal one, kept as a name alone: the rule
        // goes by the name then.
        let sender = if m.from.email.trim().is_empty() { m.from.name.trim() } else { m.from.email.trim() };
        let server = s.server_kind(&m.key.account).is_some();
        acts.push(Action::Dialog(Dialog::rule_from(sender, Some(&m.key.account), server, Some(&m.key.folder))));
        ui.close();
    }
}

/// Find related's searches, as Outlook's: the messages of the conversation (by its subject), or
/// everything from the sender.
pub fn find_related(ui: &mut Ui, subject: &str, sender: &str, acts: &mut Vec<Action>) {
    let topic = quatrelle_engine::model::thread::base_subject(subject);
    let searches =
        [("Messages in this conversation", format!("subject:\"{}\"", topic.replace('"', ""))), ("Messages from sender", format!("from:{sender}"))];
    for (label, query) in searches {
        if ui.button(tl!(label)).clicked() {
            acts.push(Action::Search(query));
            ui.close();
        }
    }
}
