// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The menu a right click on an appointment opens, anywhere in the grid: open it, join its
//! meeting; in a calendar the user can write, answer it, how it shows, its categories, private,
//! delete it. How it shows, its categories and private tell no one (`"send": false`); on a
//! series they change the series, as its window does.

use egui::{RichText, Sense, Ui, vec2};
use quatrelle_engine::model::ical::ShowAs;
use quatrelle_engine::{AgendaItem, Session};
use serde_json::json;

use super::open;
use crate::dialogs::pim_dialogs::{all_categories, show_as_label};
use crate::icons::{self, Icon};
use crate::{Action, theme};

/// The menu's entries for `item` (drawn only while it's open: the event is read from here).
pub(super) fn menu(ui: &mut Ui, s: &Session, item: &AgendaItem, acts: &mut Vec<Action>) {
    let event = s.find_event(&item.uid).map(|(_, e)| &e.event);
    if ui.button(tl!("Open")).clicked() {
        acts.extend(open(s, item));
        ui.close();
    }
    if let Some(url) = event.and_then(|e| e.join_link())
        && ui.button(tl!("Join meeting")).clicked()
    {
        acts.push(Action::OpenMeeting(url));
        ui.close();
    }
    let writable = s.pim.get(&item.account).and_then(|p| p.calendars.iter().find(|c| c.id == item.calendar)).is_some_and(|c| !c.read_only);
    if !writable {
        return;
    }
    let uid = item.uid.as_str();
    // Someone else's meeting: the user's answer, ticked; another one changes it.
    if let Some(answer) = &item.answer {
        ui.separator();
        for (label, code, partstat) in
            [("Accept", "accepted", "ACCEPTED"), ("Tentative", "tentative", "TENTATIVE"), ("Decline", "declined", "DECLINED")]
        {
            let current = answer.eq_ignore_ascii_case(partstat);
            if ui.radio(current, tl!(label)).clicked() {
                if !current {
                    acts.push(Action::Run("event.respond", json!({"uid": uid, "answer": code})));
                }
                ui.close();
            }
        }
    }
    ui.separator();
    ui.menu_button(tl!("Show As"), |ui| {
        for x in ShowAs::ALL {
            if ui.radio(item.show_as == x, tl!(show_as_label(x))).clicked() {
                if x != item.show_as {
                    acts.push(Action::Run("event.update", json!({"uid": uid, "showAs": x.param(), "send": false})));
                }
                ui.close();
            }
        }
    });
    let has: Vec<String> = event.map(|e| e.categories.clone()).unwrap_or_default();
    let all = all_categories(s, &has);
    ui.add_enabled_ui(!all.is_empty(), |ui| {
        ui.menu_button(tl!("Categorize"), |ui| {
            let dark = ui.visuals().dark_mode;
            for (name, color) in &all {
                let mut on = has.iter().any(|c| c.eq_ignore_ascii_case(name));
                let c = theme::category_color(*color, dark);
                if ui.checkbox(&mut on, RichText::new(format!("● {name}")).color(c)).changed() {
                    let mut list: Vec<String> = has.iter().filter(|x| !x.eq_ignore_ascii_case(name)).cloned().collect();
                    if on {
                        list.push(name.clone());
                    }
                    acts.push(Action::Run("event.update", json!({"uid": uid, "categories": list, "send": false})));
                    ui.close();
                }
            }
        });
    });
    ui.horizontal(|ui| {
        let mut private = item.private;
        if ui.checkbox(&mut private, tl!("Private")).changed() {
            acts.push(Action::Run("event.update", json!({"uid": uid, "private": private, "send": false})));
            ui.close();
        }
        let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
        icons::paint(ui.painter(), r, Icon::Lock, ui.visuals().weak_text_color());
    });
    ui.separator();
    // A series asks which: this occurrence or all of it.
    if item.recurring {
        ui.menu_button(tl!("Delete"), |ui| {
            if ui.button(tl!("Delete this occurrence")).clicked() {
                acts.push(Action::Run("event.delete", json!({"uid": uid, "occurrence": item.occurrence})));
                ui.close();
            }
            if ui.button(tl!("Delete the series")).clicked() {
                acts.push(Action::Run("event.delete", json!({"uid": uid})));
                ui.close();
            }
        });
    } else if ui.button(tl!("Delete")).clicked() {
        acts.push(Action::Run("event.delete", json!({"uid": uid})));
        ui.close();
    }
}
