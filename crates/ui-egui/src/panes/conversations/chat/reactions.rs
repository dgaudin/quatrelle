// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A message's reactions, as chips under it: the emoji and how many reacted so, the user's own
//! going out counted at once (`TeamsState::shown_reactions`), theirs in the accent on its soft
//! tint. A click sets the user's reaction of that kind, or takes it back (`teams.react`): only in
//! a chat they're still in, on a message that takes reactions, and of a kind that is an emoji
//! ([`reactable`]: one Teams names otherwise can't be sent back, so its chip only shows). Hovered,
//! a chip lists who reacted, by the chat's members' names, the user among them as the chip counts
//! them.
//!
//! What is hostile here: the reactions are the senders' and Teams' (at most [`MAX_REACTIONS`] a
//! message, the engine's cap): an emoji is drawn without control or bidi characters, a few
//! characters at most; the names come from the members, looked up only while a chip is hovered,
//! at most [`NAMES_MAX`] (the chips are drawn every frame).

use egui::{Align, Layout, Sense, Ui, vec2};
use quatrelle_engine::model::pim::teams::{ChatMessage, MAX_REACTIONS, is_bidi, one_line, reactable};
use serde_json::json;

use super::Pane;
use crate::Action;
use crate::theme::{self, Tokens};

/// The most names a reaction's hover lists.
const NAMES_MAX: usize = 20;
/// The most characters of a reaction's emoji drawn.
const EMOJI_CHARS: usize = 8;

/// The message's chips; `writable` when a click may set or take back the user's reaction (the
/// chat not left, the message neither deleted nor being deleted). The user's own message's chips
/// hug the right, as the message does.
pub(super) fn reactions(p: &Pane, ui: &mut Ui, m: &ChatMessage, writable: bool, acts: &mut Vec<Action>) {
    let shown = p.s.teams.shown_reactions(m, p.me);
    if shown.is_empty() {
        return;
    }
    let own = p.is_own(m);
    let mut chips: Vec<_> = shown.iter().take(MAX_REACTIONS).collect();
    // Right to left, the last first: they read in their order all the same.
    if own {
        chips.reverse();
    }
    // Their tops in a line (centred, they'd go to the middle of the pane's height).
    let layout = if own { Layout::right_to_left(Align::Min) } else { Layout::left_to_right(Align::Min) };
    ui.add_space(2.0);
    ui.with_layout(layout.with_main_wrap(true), |ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        for r in chips {
            let emoji: String = r.emoji.chars().filter(|&c| !(c.is_control() || is_bidi(c))).take(EMOJI_CHARS).collect();
            if emoji.is_empty() {
                continue;
            }
            let clickable = writable && reactable(r.emoji);
            let chip = chip(ui, &p.t, format!("{emoji} {}", r.count), r.mine, clickable);
            if chip.clicked() {
                acts.push(Action::Run("teams.react", json!({"chat": m.chat, "message": m.id, "emoji": r.emoji})));
            }
            if chip.hovered() {
                let names = names(p, r.users, r.mine);
                if !names.is_empty() {
                    chip.on_hover_text(names.join(", "));
                }
            }
        }
    });
}

/// A chip: its text in a rounded frame, the accent around its soft tint when it's the user's;
/// clickable, it's tinted under the pointer.
fn chip(ui: &mut Ui, t: &Tokens, text: String, mine: bool, clickable: bool) -> egui::Response {
    let g = ui.painter().layout_no_wrap(text, theme::regular(12.0), t.text);
    let size = g.size() + vec2(14.0, 4.0);
    let (rect, resp) = ui.allocate_exact_size(size, if clickable { Sense::click() } else { Sense::hover() });
    if clickable && resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if ui.is_rect_visible(rect) {
        let fill = match (mine, clickable && resp.hovered()) {
            (true, _) => t.accent_soft,
            (false, true) => t.hover,
            (false, false) => egui::Color32::TRANSPARENT,
        };
        let stroke = egui::Stroke::new(1.0, if mine { t.accent } else { t.border });
        ui.painter().rect(rect, 10.0, fill, stroke, egui::StrokeKind::Inside);
        ui.painter().galley(rect.center() - g.size() / 2.0, g, t.text);
    }
    resp
}

/// Who reacted, as the chat's members name them: Teams' list, the user put in when the chip counts
/// them (`mine`, their reaction maybe still going out) and left out when it doesn't (taken back).
fn names(p: &Pane, users: &[String], mine: bool) -> Vec<String> {
    let me = |u: &str| !p.me.is_empty() && u.eq_ignore_ascii_case(p.me);
    let name = |u: &str| p.s.teams.member(u).map(|x| one_line(&x.name)).filter(|n| !n.is_empty());
    let own = mine.then(|| name(p.me).unwrap_or_else(|| one_line(&p.s.prefs.teams.name))).filter(|n| !n.is_empty());
    own.into_iter().chain(users.iter().filter(|u| !me(u)).filter_map(|u| name(u))).take(NAMES_MAX).collect()
}
