// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What the user sent in the open chat that Teams doesn't have yet, after the chat's messages, on
//! the user's side and dimmed: "Sending…" while it waits or goes, with Delete while the outbox
//! holds it waiting its turn (one stuck offline needn't fail first); "Not sent" in the danger
//! colour, why, and Teams' code after it as Teams gave it, with Retry and Delete. Nothing is
//! offered while it's being sent, or before the outbox has taken it: it may be in Teams already
//! (the engine refuses Delete then, and Retry acts only on a message not sent).
//!
//! What is hostile here: the code came from Teams' answer (the worker keeps at most 64 characters
//! of it): drawn on one line, after the window's own sentence, never in its place.

use egui::{Align, Layout, RichText, Ui};
use quatrelle_engine::model::pim::teams::{OutState, Outgoing, Unsent, one_line};
use serde_json::json;

use super::{Pane, text};
use crate::Action;
use crate::theme::Tokens;

/// How much a message going out is dimmed.
const DIMMED: f32 = 0.6;

/// The open chat's messages going out, each under the last.
pub(super) fn outgoing(p: &Pane, ui: &mut Ui, chat: &str, acts: &mut Vec<Action>) {
    let t = &p.t;
    for o in p.s.teams.outgoing.iter().filter(|o| o.chat == chat) {
        ui.add_space(10.0);
        let width = (ui.available_width() * 0.7).clamp(180.0, 620.0);
        ui.with_layout(Layout::top_down(Align::Max), |ui| {
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                ui.multiply_opacity(DIMMED);
                egui::Frame::new().fill(t.accent_soft).corner_radius(8).inner_margin(egui::Margin::symmetric(10, 7)).show(ui, |ui| {
                    ui.set_max_width(width);
                    ui.with_layout(Layout::top_down(Align::Max), |ui| {
                        let cx = text::Context { me: p.me, members: p.s.teams.members(), color: t.text };
                        text::body(ui, t, &o.body, &[], &cx, width - 20.0, acts);
                    });
                });
            });
            state(ui, t, o, acts);
        });
    }
}

/// Why a message wasn't sent, in the window's words.
fn why(reason: Unsent) -> &'static str {
    match reason {
        Unsent::Refused => tl!("Teams refused this message"),
        Unsent::TooLong => tl!("Too long for Teams"),
        Unsent::TooMany => tl!("Too many messages are waiting"),
        Unsent::Left => tl!("You're no longer in this conversation"),
        Unsent::Unconfirmed => tl!("Quatrelle couldn't confirm it was sent"),
    }
}

/// Under the message: "Sending…", and Delete while the outbox holds it waiting its turn; or "Not
/// sent", why, and Retry and Delete, which name it by its number, or by its local id for one the
/// outbox didn't take (numbered 0).
fn state(ui: &mut Ui, t: &Tokens, o: &Outgoing, acts: &mut Vec<Action>) {
    let OutState::NotSent { reason, code } = &o.state else {
        ui.label(RichText::new(tl!("Sending…")).size(11.0).color(t.dim));
        // Not yet taken (numbered 0), or being sent: it may be in Teams already.
        if o.seq != 0 && o.state == OutState::Waiting {
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                if ui.button(tl!("Delete")).clicked() {
                    acts.push(Action::Run("teams.discard", json!({"seq": o.seq})));
                }
            });
        }
        return;
    };
    let why = match code.as_deref().map(one_line).filter(|c| !c.is_empty()) {
        Some(code) => format!("{} ({code})", why(*reason)),
        None => why(*reason).to_string(),
    };
    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        ui.label(RichText::new(why).size(11.5).color(t.dim));
        ui.label(RichText::new(tl!("Not sent")).size(11.5).strong().color(t.danger));
    });
    let which = if o.seq == 0 { json!({"seq": 0, "local": o.local}) } else { json!({"seq": o.seq}) };
    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        if ui.button(tl!("Delete")).clicked() {
            acts.push(Action::Run("teams.discard", which.clone()));
        }
        if ui.button(tl!("Retry")).clicked() {
            acts.push(Action::Run("teams.retry", which));
        }
    });
}
