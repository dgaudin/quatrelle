// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What the user sent in the open chat that Teams doesn't have yet, after the chat's messages, on
//! the user's side and dimmed: "Sending…" while it waits or goes, with Delete while the outbox
//! holds it waiting its turn (one stuck offline needn't fail first); "Not sent" in the danger
//! colour, why, and Teams' code after it as Teams gave it, with Retry and Delete. Nothing is
//! offered while it's being sent, or before the outbox has taken it: it may be in Teams already
//! (the engine refuses Delete then, and Retry acts only on a message not sent).
//!
//! An edit or a deletion going out is drawn under its message the same way ([`change_state`]):
//! "Editing…" or "Deleting…", with Cancel while it waits its turn; "Not edited" or "Not deleted",
//! why, Retry and Cancel. One whose message the window doesn't hold (it went back to older
//! messages, a refusal kept for the chat) still counts in the chat's share: it is drawn here,
//! after the messages, so it can be seen and let go. A reaction going out shows on its chip only.
//!
//! What is hostile here: the code came from Teams' answer (the worker keeps at most 64 characters
//! of it): drawn on one line, after the window's own sentence, never in its place.

use egui::{Align, Layout, RichText, Ui};
use quatrelle_engine::model::pim::teams::{MessageKind, OutKind, OutState, Outgoing, Unsent, one_line};
use serde_json::json;

use super::{DIMMED, Pane, text};
use crate::Action;
use crate::theme::Tokens;

/// The open chat's messages going out, each under the last; then the edits and deletions of
/// messages the window doesn't hold.
pub(super) fn outgoing(p: &Pane, ui: &mut Ui, chat: &str, acts: &mut Vec<Action>) {
    let t = &p.t;
    let mine = p.s.teams.outgoing.iter().filter(|o| o.chat == chat);
    for o in mine.clone().filter(|o| o.what == OutKind::Send) {
        ui.add_space(10.0);
        ui.with_layout(Layout::top_down(Align::Max), |ui| {
            going_out(p, ui, o, acts);
            state(ui, t, o, acts);
        });
    }
    // Drawn under its message when the window holds it (as a message, not a system line).
    let held = |message: &str| p.s.teams.window.iter().any(|m| m.id == message && m.kind != MessageKind::System);
    let elsewhere = mine.filter(|o| matches!(o.what, OutKind::Edit { .. } | OutKind::Delete { .. }) && !o.what.message().is_some_and(held));
    for o in elsewhere {
        ui.add_space(10.0);
        ui.with_layout(Layout::top_down(Align::Max), |ui| {
            if matches!(o.what, OutKind::Edit { .. }) {
                going_out(p, ui, o, acts);
            }
            change_state(p, ui, o, acts);
        });
    }
}

/// What goes out, dimmed in a bubble on the user's side.
fn going_out(p: &Pane, ui: &mut Ui, o: &Outgoing, acts: &mut Vec<Action>) {
    let t = &p.t;
    let width = (ui.available_width() * 0.7).clamp(180.0, 620.0);
    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        ui.multiply_opacity(DIMMED);
        egui::Frame::new().fill(t.accent_soft).corner_radius(8).inner_margin(egui::Margin::symmetric(10, 7)).show(ui, |ui| {
            ui.set_max_width(width);
            ui.with_layout(Layout::top_down(Align::Max), |ui| {
                let cx = text::Context { me: p.me, members: p.s.teams.members(), color: t.text };
                text::body(ui, t, &o.body, &o.mentions, &cx, width - 20.0, acts);
            });
        });
    });
}

/// Why a message wasn't sent, in the window's words.
fn why(reason: Unsent) -> &'static str {
    match reason {
        // Until the window has a sentence for each of the files' reasons, they say what a refusal says.
        Unsent::Refused | Unsent::FileGone | Unsent::FileTooBig | Unsent::NotShared => tl!("Teams refused this message"),
        Unsent::TooLong => tl!("Too long for Teams"),
        Unsent::TooMany => tl!("Too many messages are waiting"),
        Unsent::Left => tl!("You're no longer in this conversation"),
        Unsent::Unconfirmed => tl!("Quatrelle couldn't confirm it was sent"),
    }
}

/// Why an edit or a deletion wasn't made: as a message's, but of a change.
fn change_why(reason: Unsent) -> &'static str {
    match reason {
        Unsent::Refused => tl!("Teams refused this change"),
        Unsent::Unconfirmed => tl!("Quatrelle couldn't confirm the change was made"),
        other => why(other),
    }
}

/// How Retry and Delete (or Cancel) name it: by its number, or by its local id for one the outbox
/// didn't take (numbered 0).
fn which(o: &Outgoing) -> serde_json::Value {
    if o.seq == 0 { json!({"seq": 0, "local": o.local}) } else { json!({"seq": o.seq}) }
}

/// Under the message: "Sending…", and Delete while the outbox holds it waiting its turn; or "Not
/// sent", why, and Retry and Delete.
fn state(ui: &mut Ui, t: &Tokens, o: &Outgoing, acts: &mut Vec<Action>) {
    let OutState::NotSent { reason, code } = &o.state else {
        going(ui, t, o, tl!("Sending…"), tl!("Delete"), acts);
        return;
    };
    not_made(ui, t, o, (tl!("Not sent"), why(*reason), code.as_deref()), tl!("Delete"), acts);
}

/// Under an edit or a deletion going out (`o`): "Editing…" or "Deleting…", and Cancel while the
/// outbox holds it waiting its turn; or "Not edited" or "Not deleted", why, and Retry and Cancel.
pub(super) fn change_state(p: &Pane, ui: &mut Ui, o: &Outgoing, acts: &mut Vec<Action>) {
    let edit = matches!(o.what, OutKind::Edit { .. });
    let OutState::NotSent { reason, code } = &o.state else {
        going(ui, &p.t, o, if edit { tl!("Editing…") } else { tl!("Deleting…") }, tl!("Cancel"), acts);
        return;
    };
    let headline = if edit { tl!("Not edited") } else { tl!("Not deleted") };
    not_made(ui, &p.t, o, (headline, change_why(*reason), code.as_deref()), tl!("Cancel"), acts);
}

/// `said` ("Sending…"), and `let_go` (Delete, Cancel) while the outbox holds it waiting its turn:
/// not yet taken (numbered 0), or being sent, it may be in Teams already.
fn going(ui: &mut Ui, t: &Tokens, o: &Outgoing, said: &str, let_go: &str, acts: &mut Vec<Action>) {
    ui.label(RichText::new(said).size(11.0).color(t.dim));
    if o.seq != 0 && matches!(o.state, OutState::Waiting | OutState::Uploading) {
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            if ui.button(let_go).clicked() {
                acts.push(Action::Run("teams.discard", json!({"seq": o.seq})));
            }
        });
    }
}

/// What wasn't made: its headline ("Not sent") in the danger colour, why with Teams' code after
/// it, then `let_go` (Delete, Cancel) and Retry.
fn not_made(ui: &mut Ui, t: &Tokens, o: &Outgoing, (headline, why, code): (&str, &str, Option<&str>), let_go: &str, acts: &mut Vec<Action>) {
    let why = match code.map(one_line).filter(|c| !c.is_empty()) {
        Some(code) => format!("{why} ({code})"),
        None => why.to_string(),
    };
    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        ui.label(RichText::new(why).size(11.5).color(t.dim));
        ui.label(RichText::new(headline).size(11.5).strong().color(t.danger));
    });
    let which = which(o);
    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        if ui.button(let_go).clicked() {
            acts.push(Action::Run("teams.discard", which.clone()));
        }
        if ui.button(tl!("Retry")).clicked() {
            acts.push(Action::Run("teams.retry", which));
        }
    });
}
