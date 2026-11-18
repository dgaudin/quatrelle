// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What the user sent in the open chat that Teams doesn't have yet, after the chat's messages, on
//! the user's side and dimmed, its files and pictures listed under its words: "Sending…" while it
//! waits or goes, with Delete while the outbox holds it waiting its turn (one stuck offline needn't
//! fail first); "Uploading… 42 %" while its files go up, with Cancel; "Not sent" in the danger
//! colour, why, and Teams' code after it as Teams gave it, with Retry and Delete. Nothing is
//! offered while it's being sent, or before the outbox has taken it: it may be in Teams already
//! (the engine refuses Delete then, and Retry acts only on a message not sent). One the outbox
//! refused with its files can't be sent again as it is (the engine kept only their names): it
//! says to attach them again, and offers Delete alone.
//!
//! An edit or a deletion going out is drawn under its message the same way ([`change_state`]):
//! "Editing…" or "Deleting…", with Cancel while it waits its turn; "Not edited" or "Not deleted",
//! why, Retry and Cancel. One whose message the window doesn't hold (it went back to older
//! messages, a refusal kept for the chat) still counts in the chat's share: it is drawn here,
//! after the messages, so it can be seen and let go. A reaction going out shows on its chip only.
//!
//! What is hostile here: the code came from Teams' answer (the worker keeps at most 64 characters
//! of it): drawn on one line, after the window's own sentence, never in its place. The parts'
//! names (the user's files', as the worker keeps them) are drawn on one line, without bidi
//! characters, at most [`PARTS_MAX`] of them.

use egui::{Align, Layout, RichText, Sense, Ui, vec2};
use quatrelle_engine::model::pim::teams::{MessageKind, OutKind, OutPart, OutState, Outgoing, PARTS_MAX, Unsent, is_bidi, one_line};
use serde_json::json;

use super::{DIMMED, Pane, text};
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{Action, widgets};

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

/// What goes out, dimmed in a bubble on the user's side: its words, then its parts.
fn going_out(p: &Pane, ui: &mut Ui, o: &Outgoing, acts: &mut Vec<Action>) {
    let t = &p.t;
    let width = (ui.available_width() * 0.7).clamp(180.0, 620.0);
    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        ui.multiply_opacity(DIMMED);
        egui::Frame::new().fill(t.accent_soft).corner_radius(8).inner_margin(egui::Margin::symmetric(10, 7)).show(ui, |ui| {
            ui.set_max_width(width);
            ui.with_layout(Layout::top_down(Align::Max), |ui| {
                let cx = text::Context { me: p.me, members: p.s.teams.members(), color: t.text, clean_links: p.s.prefs.clean_links };
                text::body(ui, t, &o.body, &o.mentions, &cx, width - 20.0, acts);
                parts(ui, t, &o.parts, width - 20.0);
            });
        });
    });
}

/// A message's files and pictures, a line each: its icon, its name on one line, its size once the
/// worker has said it.
fn parts(ui: &mut Ui, t: &Tokens, parts: &[OutPart], width: f32) {
    for part in parts.iter().take(PARTS_MAX) {
        let name: String = one_line(&part.name).chars().filter(|&c| !is_bidi(c)).collect();
        let size = (part.size > 0).then(|| widgets::size_text(part.size));
        // On the user's side as the words are: from the right, the size, the name, the icon.
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            if let Some(size) = &size {
                ui.label(RichText::new(size).size(11.5).color(t.dim));
            }
            let room = width - 24.0 - if size.is_some() { 64.0 } else { 0.0 };
            ui.label(widgets::one_line(ui, &name, theme::regular(12.5), t.text, room));
            let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
            icons::paint(ui.painter(), r, if part.picture { Icon::Image } else { Icon::File }, t.dim);
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
        Unsent::FileGone => tl!("A file is no longer there, or has changed"),
        Unsent::FileTooBig => tl!("A file is bigger than Teams takes here (100 MB)"),
        Unsent::NotShared => tl!("OneDrive didn't share the file with everyone in this conversation"),
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

/// How far a message's files have gone up: the bytes sent over their sizes (its pictures go
/// inside the message, not up), "Uploading… 42 %"; "Uploading…" before any size is known.
fn uploading(parts: &[OutPart]) -> String {
    let files = parts.iter().filter(|p| !p.picture);
    let total = files.clone().fold(0u64, |n, p| n.saturating_add(p.size));
    let sent = files.fold(0u64, |n, p| n.saturating_add(p.sent.min(p.size)));
    if total == 0 {
        return tl!("Uploading…").to_string();
    }
    let percent = (u128::from(sent) * 100 / u128::from(total)).min(100);
    tf!("Uploading… {percent} %", percent = percent)
}

/// Under the message: "Sending…", and Delete while the outbox holds it waiting its turn;
/// "Uploading… 42 %" and Cancel while its files go up; or "Not sent", why, and Retry and Delete.
fn state(ui: &mut Ui, t: &Tokens, o: &Outgoing, acts: &mut Vec<Action>) {
    match &o.state {
        OutState::NotSent { reason, code } => not_made(ui, t, o, (tl!("Not sent"), why(*reason), code.as_deref()), tl!("Delete"), acts),
        OutState::Uploading => going(ui, t, o, &uploading(&o.parts), tl!("Cancel"), acts),
        OutState::Waiting | OutState::Sending => going(ui, t, o, tl!("Sending…"), tl!("Delete"), acts),
    }
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

/// `said` ("Sending…"), and `let_go` (Delete, Cancel) while the outbox holds it waiting its turn
/// or its files go up: not yet taken (numbered 0), or being sent, it may be in Teams already.
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
/// it, then `let_go` (Delete, Cancel) and Retry. A message the outbox refused with its files
/// (numbered 0) isn't sent again as it is: it says to attach them again, and has no Retry.
fn not_made(ui: &mut Ui, t: &Tokens, o: &Outgoing, (headline, why, code): (&str, &str, Option<&str>), let_go: &str, acts: &mut Vec<Action>) {
    let why = match code.map(one_line).filter(|c| !c.is_empty()) {
        Some(code) => format!("{why} ({code})"),
        None => why.to_string(),
    };
    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        ui.label(RichText::new(why).size(11.5).color(t.dim));
        ui.label(RichText::new(headline).size(11.5).strong().color(t.danger));
    });
    let attach_again = o.seq == 0 && !o.parts.is_empty();
    if attach_again {
        ui.label(RichText::new(tl!("Attach the files again to send it")).size(11.5).color(t.dim));
    }
    let which = which(o);
    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        if ui.button(let_go).clicked() {
            acts.push(Action::Run("teams.discard", which.clone()));
        }
        if !attach_again && ui.button(tl!("Retry")).clicked() {
            acts.push(Action::Run("teams.retry", which));
        }
    });
}
