// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The menu a right click on an appointment opens, anywhere in the grid: open it, join its
//! meeting; in a calendar the user can write, answer it, how it shows, its categories, private,
//! delete it. How it shows, its categories and private tell no one (`"send": false`); on a
//! series they change the series, as its window does.

use egui::{Button, RichText, Ui};
use quatrelle_engine::model::ical::ShowAs;
use quatrelle_engine::{AgendaItem, Session};
use serde_json::json;

use super::open;
use crate::dialogs::pim_dialogs::{all_categories, show_as_label};
use crate::icons::Icon;
use crate::{Action, theme, widgets};

/// Whether the user may change `item` here: its calendar isn't read-only, and it's the event the
/// uid finds (the user's own copy first), what the commands change. A colleague's copy of a
/// meeting the user also has is opened only.
pub(super) fn changeable(s: &Session, item: &AgendaItem) -> bool {
    let writable = s.pim.get(&item.account).and_then(|p| p.calendars.iter().find(|c| c.id == item.calendar)).is_some_and(|c| !c.read_only);
    let this_one = s.find_event(&item.uid).is_some_and(|(account, e)| account == item.account && e.calendar == item.calendar);
    writable && this_one
}

/// The menu's entries for `item` (drawn only while it's open: the event is read from here).
pub(super) fn menu(ui: &mut Ui, s: &Session, item: &AgendaItem, acts: &mut Vec<Action>) {
    let found = s.find_event(&item.uid);
    let event = found.map(|(_, e)| &e.event);
    if ui.button((widgets::menu_icon(Some(Icon::PopOut)), tl!("Open"))).clicked() {
        acts.extend(open(s, item));
        ui.close();
    }
    if let Some(url) = event.and_then(|e| e.join_link())
        && ui.button((widgets::menu_icon(Some(Icon::Video)), tl!("Join meeting"))).clicked()
    {
        acts.push(Action::OpenMeeting(url));
        ui.close();
    }
    if !changeable(s, item) {
        return;
    }
    let uid = item.uid.as_str();
    // Someone else's meeting: the user's answer, ticked; another one changes it.
    if let Some(answer) = &item.answer {
        ui.separator();
        for (label, code, partstat, icon) in [
            ("Accept", "accepted", "ACCEPTED", Icon::Done),
            ("Tentative", "tentative", "TENTATIVE", Icon::Question),
            ("Decline", "declined", "DECLINED", Icon::Close),
        ] {
            let current = answer.eq_ignore_ascii_case(partstat);
            if ui.add(Button::new((widgets::menu_icon(Some(icon)), tl!(label))).selected(current)).clicked() {
                if !current {
                    acts.push(Action::Run("event.respond", json!({"uid": uid, "answer": code})));
                }
                ui.close();
            }
        }
    }
    // Ticked as the event changed is (a series, not an occurrence changed on its own).
    let (show_as, is_private) = event.map_or((item.show_as, item.private), |e| (e.show_as, e.private));
    ui.separator();
    ui.menu_button((widgets::menu_icon(Some(Icon::Clock)), tl!("Show As")), |ui| {
        for x in ShowAs::ALL {
            if ui.add(Button::new((widgets::menu_icon(None), tl!(show_as_label(x)))).selected(show_as == x)).clicked() {
                if x != show_as {
                    acts.push(Action::Run("event.update", json!({"uid": uid, "showAs": x.param(), "send": false})));
                }
                ui.close();
            }
        }
    });
    let has: Vec<String> = event.map(|e| e.categories.clone()).unwrap_or_default();
    let all = all_categories(s, &has);
    ui.add_enabled_ui(!all.is_empty(), |ui| {
        ui.menu_button((widgets::menu_icon(Some(Icon::Tag)), tl!("Categorize")), |ui| {
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
    if ui.add(Button::new((widgets::menu_icon(Some(Icon::Lock)), tl!("Private"))).selected(is_private)).clicked() {
        acts.push(Action::Run("event.update", json!({"uid": uid, "private": !is_private, "send": false})));
        ui.close();
    }
    ui.separator();
    // A series asks which: this occurrence or all of it.
    if item.recurring {
        ui.menu_button((widgets::menu_icon(Some(Icon::Delete)), tl!("Delete")), |ui| {
            if ui.button((widgets::menu_icon(Some(Icon::Delete)), tl!("Delete this occurrence"))).clicked() {
                acts.push(Action::Run("event.delete", json!({"uid": uid, "occurrence": item.occurrence})));
                ui.close();
            }
            if ui.button((widgets::menu_icon(Some(Icon::Delete)), tl!("Delete the series"))).clicked() {
                acts.push(Action::Run("event.delete", json!({"uid": uid})));
                ui.close();
            }
        });
    } else if ui.button((widgets::menu_icon(Some(Icon::Delete)), tl!("Delete"))).clicked() {
        acts.push(Action::Run("event.delete", json!({"uid": uid})));
        ui.close();
    }
}
