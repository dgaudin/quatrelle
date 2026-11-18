// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The conversation's header: "External" first when someone is from outside, the name, a
//! one-to-one chat's other person's presence, then, on the right, the members' button, "Open in
//! Teams" when the chat's link is one and "Call" (audio or video) when the chat can be called
//! (`TeamsState::callable`). A call is asked of the engine (`teams.call`), which hands the window
//! Teams' call link; the window opens it in the meetings browser's window.

use egui::{Align, Layout, RichText, Sense, Ui, vec2};
use quatrelle_engine::model::pim::teams::{ChatKind, ChatRow, is_internal, one_line};
use serde_json::json;

use super::{Pane, presence, tag};
use crate::icons::Icon;
use crate::theme;
use crate::{Action, widgets};

/// The header: "External" first when someone is from outside, the name, a one-to-one chat's
/// other person's presence, the members (their list on a click) and "Open in Teams" when the
/// chat's link is one; a chat the user is no longer in says so under it.
pub(super) fn header(p: &Pane, ui: &mut Ui, row: &ChatRow, acts: &mut Vec<Action>) {
    let t = &p.t;
    ui.horizontal(|ui| {
        if row.external {
            tag(ui, t, tl!("External")).on_hover_text(tl!("Someone in this conversation is outside your organisation"));
        }
        // Room for the members' button, and for "Open in Teams" and "Call" beside it.
        let callable = p.s.teams.callable(p.me).is_some();
        let buttons = 140.0 + if p.teams_link.is_some() { 120.0 } else { 0.0 } + if callable { 100.0 } else { 0.0 };
        let room = (ui.available_width() - buttons).max(60.0);
        let g = widgets::one_line(ui, &one_line(&row.name), theme::bold(17.0), t.text, room);
        ui.label(g);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            members(p, ui, row);
            p.open_in_teams(ui, acts);
            if callable {
                call(ui, acts);
            }
        });
    });
    let shown =
        row.person.as_deref().filter(|_| row.kind == ChatKind::OneOnOne).and_then(|u| p.s.teams.presence.get(u)).and_then(|&pr| presence(pr, t));
    if let Some((word, color)) = shown {
        ui.horizontal(|ui| {
            let (dot, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
            ui.painter().circle_filled(dot.center(), 4.5, color);
            ui.label(RichText::new(word).size(12.5).color(t.dim));
        });
    }
    if row.left {
        ui.label(RichText::new(tl!("You're no longer in this conversation")).size(12.5).color(t.dim));
    }
}

/// "Call": "Audio call" and "Video call", each asked of the engine (`teams.call`), which opens
/// Teams' call link in the window's own time.
fn call(ui: &mut Ui, acts: &mut Vec<Action>) {
    ui.menu_button((widgets::menu_icon(Some(Icon::Phone)), tl!("Call")), |ui| {
        for (label, video) in [(tl!("Audio call"), false), (tl!("Video call"), true)] {
            if ui.button(label).clicked() {
                acts.push(Action::Run("teams.call", json!({"video": video})));
                ui.close();
            }
        }
    });
}

/// "Members: n", and on a click their list, each from outside marked "External".
fn members(p: &Pane, ui: &mut Ui, row: &ChatRow) {
    let t = &p.t;
    let list = p.s.teams.members().as_slice();
    let n = if list.is_empty() { row.members as usize } else { list.len() };
    ui.menu_button(tf!("Members: {n}", n = n), |ui| {
        ui.set_min_width(240.0);
        egui::ScrollArea::vertical().max_height(320.0).show_rows(ui, 24.0, list.len(), |ui, range| {
            for m in list.get(range).unwrap_or_default() {
                ui.horizontal(|ui| {
                    ui.set_height(22.0);
                    if !is_internal(m, p.tenant) {
                        tag(ui, t, tl!("External"));
                    }
                    let g = widgets::one_line(ui, &one_line(&m.name), theme::regular(13.0), t.text, ui.available_width());
                    ui.label(g);
                });
            }
        });
    });
}
