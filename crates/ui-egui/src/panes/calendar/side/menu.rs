// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A calendar's right-click menu in the side pane, each entry with its icon: hidden or shown,
//! shown alone, overlaid, its colour; a calendar made, renamed or deleted where the account's
//! server allows it (greyed, with why, where it doesn't), removed from the list on this computer;
//! its place among the others; and for one subscribed to by its link or opened by hand, what
//! reads it again or removes it. And an account's heading's: a calendar made in it, and its
//! calendars removed from the list, brought back.

use egui::Button;
use quatrelle_engine::model::pim::{CalendarInfo, teams::one_line};
use quatrelle_engine::{AccountState, CalendarRefusal, PALETTE_COLORS, RemovalRefusal};

use super::*;

/// What else the menu offers for this calendar: one subscribed to by its link (read again,
/// removed), one opened by hand from its owner's address (removed).
pub(super) struct Extra {
    pub web: bool,
    pub opened: Option<String>,
}

/// Why a calendar can't be made, renamed or deleted, as the menu says it when pointed at.
pub(crate) fn refusal_text(r: CalendarRefusal) -> &'static str {
    match r {
        CalendarRefusal::Server => tl!("This account's server doesn't allow it"),
        CalendarRefusal::Main => tl!("The account's main calendar stays as it is"),
        CalendarRefusal::NotOwn => tl!("Only the account's own calendars can be changed"),
        CalendarRefusal::NotRead => tl!("Waiting for the server's list of calendars"),
    }
}

/// One entry: its icon and label, ticked (`selected`) or not, greyed when not `enabled` (`why`
/// then said when it's pointed at). Clicked: the menu closes.
fn entry(ui: &mut Ui, icon: Icon, label: &str, ticked: bool, enabled: bool, why: Option<&str>) -> bool {
    let r = ui.add_enabled(enabled, Button::new((widgets::menu_icon(Some(icon)), label)).selected(ticked));
    let r = match why {
        Some(why) if !enabled => r.on_disabled_hover_text(why),
        _ => r,
    };
    let clicked = r.clicked();
    if clicked {
        ui.close();
    }
    clicked
}

pub(super) fn calendar_menu(ui: &mut Ui, s: &Session, a: &AccountState, c: &CalendarInfo, extra: &Extra, acts: &mut Vec<Action>) {
    let (account, id) = (a.config.id.as_str(), c.id.as_str());
    let which = json!({"account": account, "calendar": id});
    let shown = s.calendar_shown(account, id);
    // Says what a click does.
    let show = if shown { tl!("Hide this calendar") } else { tl!("Show this calendar") };
    if entry(ui, Icon::Calendar, show, false, true, None) {
        acts.push(Action::Run("calendar.show", json!({"account": account, "calendar": id, "shown": !shown})));
    }
    if entry(ui, Icon::Funnel, tl!("Show only this calendar"), false, true, None) {
        acts.push(Action::Run("calendar.showOnly", which.clone()));
    }
    // Overlaid: every calendar together in day and week (not each in a column of its own).
    let overlaid = !s.prefs.calendars_side_by_side;
    if entry(ui, Icon::Layout, tl!("Overlay"), overlaid, true, None) {
        acts.push(Action::Run("calendar.sideBySide", json!({"on": overlaid})));
    }
    ui.menu_button((widgets::menu_icon(Some(Icon::Color)), tl!("Color")), |ui| color_menu(ui, s, account, id, acts));
    ui.separator();
    let new = s.calendar_change(account, None).err().map(refusal_text);
    if entry(ui, Icon::CalendarAdd, tl!("New calendar…"), false, new.is_none(), new) {
        acts.push(Action::Dialog(Dialog::new_calendar(account)));
    }
    let change = s.calendar_change(account, Some(id)).err().map(refusal_text);
    if entry(ui, Icon::Rename, tl!("Rename…"), false, change.is_none(), change) {
        acts.push(Action::Dialog(Dialog::rename_calendar(account, c)));
    }
    remove_entry(ui, s, account, id, acts);
    if entry(ui, Icon::Delete, tl!("Delete calendar…"), false, change.is_none(), change) {
        acts.push(Action::Dialog(Dialog::delete_calendar(account, c)));
    }
    ui.separator();
    if entry(ui, Icon::ChevronUp, tl!("Move up"), false, s.can_move_calendar(account, id, true), None) {
        acts.push(Action::Run("calendar.moveUp", which.clone()));
    }
    if entry(ui, Icon::ChevronDown, tl!("Move down"), false, s.can_move_calendar(account, id, false), None) {
        acts.push(Action::Run("calendar.moveDown", which.clone()));
    }
    // Subscribed to by its link: read again, or removed.
    if extra.web {
        ui.separator();
        if entry(ui, Icon::Sync, tl!("Refresh"), false, true, None) {
            acts.push(Action::Run("calendar.refreshSubscribed", json!({"calendar": id})));
        }
        if entry(ui, Icon::Delete, tl!("Remove this calendar"), false, true, None) {
            acts.push(Action::Run("calendar.unsubscribe", json!({"calendar": id})));
        }
    }
    // Someone else's, opened by hand from their address: closed.
    if let Some(email) = &extra.opened {
        ui.separator();
        if entry(ui, Icon::Delete, tl!("Remove this calendar"), false, true, None) {
            acts.push(Action::Run("calendar.closeShared", json!({"email": email})));
        }
    }
}

/// Remove from the list: on this computer only (Delete deletes it on the server). Not offered
/// for one with a Remove of its own (subscribed to, opened by hand); greyed, with why, for the
/// account's main calendar.
fn remove_entry(ui: &mut Ui, s: &Session, account: &str, id: &str, acts: &mut Vec<Action>) {
    let why = match s.calendar_removable(account, id) {
        Err(RemovalRefusal::OwnRemove) => return,
        Err(RemovalRefusal::Main) => Some(tl!("The account's main calendar stays as it is")),
        Ok(()) => None,
    };
    let r = ui.add_enabled(why.is_none(), Button::new((widgets::menu_icon(Some(Icon::Close)), tl!("Remove from the list"))));
    let r = r.on_hover_text(tl!("On this computer only: it stays on the server"));
    let r = match why {
        Some(why) => r.on_disabled_hover_text(why),
        None => r,
    };
    if r.clicked() {
        acts.push(Action::Run("calendar.remove", json!({"account": account, "calendar": id})));
        ui.close();
    }
}

/// An account's heading's right-click menu: New calendar… as the calendars' menu has it, and
/// the account's calendars removed from the list, by name, a click bringing one back (greyed
/// when there's none).
pub(super) fn account_menu(ui: &mut Ui, s: &Session, a: &AccountState, acts: &mut Vec<Action>) {
    let account = a.config.id.as_str();
    let new = s.calendar_change(account, None).err().map(refusal_text);
    if entry(ui, Icon::CalendarAdd, tl!("New calendar…"), false, new.is_none(), new) {
        acts.push(Action::Dialog(Dialog::new_calendar(account)));
    }
    let removed = s.removed_calendars(account);
    ui.add_enabled_ui(!removed.is_empty(), |ui| {
        ui.menu_button((widgets::menu_icon(Some(Icon::Undo)), tl!("Removed calendars")), |ui| {
            // Every colleague's of a large mailbox may be among them.
            egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                for c in &removed {
                    if entry(ui, Icon::Calendar, &one_line(&c.name), false, true, None) {
                        acts.push(Action::Run("calendar.restore", json!({"account": account, "calendar": c.id})));
                    }
                }
            });
        });
    });
}

/// The colour submenu: Automatic (the server's, or the account's for its own calendar), then the
/// palette's 16, four by four, the one chosen ringed.
fn color_menu(ui: &mut Ui, s: &Session, account: &str, id: &str, acts: &mut Vec<Action>) {
    let chosen = s.chosen_color(account, id);
    if ui.add(Button::new((widgets::menu_icon(None), tl!("Automatic"))).selected(chosen.is_none())).clicked() {
        acts.push(Action::Run("calendar.color", json!({"account": account, "calendar": id, "color": null})));
        ui.close();
    }
    let dark = ui.visuals().dark_mode;
    let ink = ui.visuals().text_color();
    let hover = ui.visuals().widgets.hovered.weak_bg_fill;
    for row in 0..PALETTE_COLORS / 4 {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for i in row * 4..row * 4 + 4 {
                let (rect, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
                if resp.hovered() {
                    ui.painter().circle_filled(rect.center(), 12.0, hover);
                }
                ui.painter().circle_filled(rect.center(), 8.0, theme::category_color(i, dark));
                if chosen == Some(i) {
                    ui.painter().circle_stroke(rect.center(), 10.5, egui::Stroke::new(2.0, ink));
                }
                if resp.clicked() {
                    acts.push(Action::Run("calendar.color", json!({"account": account, "calendar": id, "color": i})));
                    ui.close();
                }
            }
        });
    }
}
