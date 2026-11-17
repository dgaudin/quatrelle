// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The bar over the message the pointer is on, and the question before a deletion. The bar is
//! drawn over the message (nothing moves), on its bubble's top edge at its outer end, while the
//! pointer is on the message's row or on the bar: the six quick reactions (`teams.react`), Copy
//! (the message's words, an edit going out's when there is one, on the clipboard), and on the
//! user's own message Edit (when the box can hold it: `TeamsState::editable`; the box loads it at
//! the next frame) and Delete (asked first). In a chat the user left, Copy alone. None on a system
//! line, a message deleted, unshowable or being deleted, nor under a question or a picture shown
//! big. Delete asks "Delete this message?": Delete asks the engine to (`teams.delete`), Cancel, Esc
//! or a click outside do nothing; it isn't asked of a message no longer to be deleted here.
//!
//! What is hostile here: the message (its words are copied as text, never read as anything else).
//! What it protects: only the user's own message is offered Edit or Delete, a deletion only after
//! its question, and what the bar offers is what the engine takes (decision 17).

use egui::{Align2, RichText, Sense, Ui, vec2};
use quatrelle_engine::model::pim::teams::{ChatMessage, MessageKind, OutKind, QUICK_REACTIONS};
use serde_json::json;

use super::{ConversationsUi, Pane};
use crate::icons::Icon;
use crate::theme::{self, Tokens};
use crate::{Action, widgets};

/// A button's side in the bar.
const BUTTON: f32 = 28.0;
/// How far the bar comes down over the bubble's top edge: less than the bubble's margin.
const OVER: f32 = 6.0;

/// What the bar offers on a message: reactions, and on the user's own Edit and Delete (Copy
/// always).
struct Offers {
    react: bool,
    edit: bool,
    delete: bool,
}

/// What the bar offers on `m`; none on a system line, a message deleted, unshowable or being
/// deleted.
fn offers(p: &Pane, m: &ChatMessage) -> Option<Offers> {
    let deleting = p.s.teams.change_of(&m.id).is_some_and(|o| matches!(o.what, OutKind::Delete { .. }));
    if m.kind != MessageKind::Message || m.deleted.is_some() || m.unshowable || deleting {
        return None;
    }
    let own = p.writable && p.is_own(m);
    Some(Offers { react: p.writable, edit: own && p.s.teams.editable(m, p.me), delete: own })
}

/// The bar's id: one at a time.
fn bar_id() -> egui::Id {
    egui::Id::unique("teams-message-bar")
}

/// The bar over `m`, whose bubble and time are `row`, while the pointer is on the row or on the
/// bar ([`ConversationsUi::hovered`]: where it was at the last frame).
pub(super) fn bar(p: &Pane, c: &mut ConversationsUi, ui: &mut Ui, m: &ChatMessage, row: egui::Rect, acts: &mut Vec<Action>) {
    let was = c.hovered.as_ref().filter(|(id, _)| *id == m.id).map(|(_, r)| *r);
    let ctx = ui.ctx().clone();
    let layer = egui::LayerId::new(egui::Order::Foreground, bar_id());
    let on_bar = was.zip(ctx.pointer_hover_pos()).is_some_and(|(r, at)| r.contains(at) && ctx.layer_id_at(at) == Some(layer));
    // Worked out for the message under the pointer only (whether the box can hold it is a parse).
    let hovered = (on_bar || ui.rect_contains_pointer(row)) && c.deleting.is_none() && c.viewing.is_none();
    let Some(offered) = hovered.then(|| offers(p, m)).flatten() else {
        if was.is_some() {
            c.hovered = None;
        }
        return;
    };
    let t = p.t;
    // At the bubble's outer end (the right of the user's own, the left of another's), over its top
    // edge by a few points: its first line stays in sight.
    let edge = row.top() + OVER;
    let (pivot, at) =
        if p.is_own(m) { (Align2::RIGHT_BOTTOM, egui::pos2(row.right(), edge)) } else { (Align2::LEFT_BOTTOM, egui::pos2(row.left(), edge)) };
    let frame = egui::Frame::new()
        .fill(t.surface)
        .stroke(egui::Stroke::new(1.0, t.border))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(4, 2))
        .shadow(egui::epaint::Shadow { offset: [0, 2], blur: 6, spread: 0, color: egui::Color32::from_black_alpha(if t.dark { 90 } else { 40 }) });
    let shown = egui::Area::new(bar_id()).order(egui::Order::Foreground).pivot(pivot).fixed_pos(at).constrain_to(ui.clip_rect()).show(&ctx, |ui| {
        frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                if offered.react {
                    for emoji in QUICK_REACTIONS {
                        if reaction(ui, &t, emoji).clicked() {
                            acts.push(Action::Run("teams.react", json!({"chat": m.chat, "message": m.id, "emoji": emoji})));
                        }
                    }
                    ui.add_space(2.0);
                    ui.separator();
                    ui.add_space(2.0);
                }
                if widgets::bar_icon_button(ui, Icon::Copy, "Copy", &t).clicked() {
                    let words = match p.s.teams.change_of(&m.id) {
                        Some(o) if matches!(o.what, OutKind::Edit { .. }) => o.body.plain(),
                        _ => m.body.plain(),
                    };
                    ui.ctx().copy_text(words);
                }
                if offered.edit && widgets::bar_icon_button(ui, Icon::Edit, "Edit", &t).clicked() {
                    c.edit_asked = Some(m.id.clone());
                }
                if offered.delete && widgets::bar_icon_button(ui, Icon::Delete, "Delete", &t).clicked() {
                    c.deleting = Some(m.id.clone());
                }
            });
        });
    });
    c.hovered = Some((m.id.clone(), shown.response.rect));
}

/// One of the quick reactions: the emoji, tinted under the pointer.
fn reaction(ui: &mut Ui, t: &Tokens, emoji: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(BUTTON, BUTTON), Sense::click());
    widgets::named(ui, &resp, emoji, true, None);
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, emoji, theme::regular(17.0), t.text);
    resp
}

/// The question before the user's message is deleted ([`ConversationsUi::deleting`]): Delete
/// asks the engine to; Cancel, Esc or a click outside let it be. Not asked of a message the window
/// no longer holds, or that can no longer be deleted here.
pub(super) fn delete_question(p: &Pane, c: &mut ConversationsUi, ctx: &egui::Context, acts: &mut Vec<Action>) {
    let Some(message) = c.deleting.clone() else { return };
    let deletable = p.s.teams.window.iter().find(|m| m.id == message).and_then(|m| offers(p, m)).is_some_and(|o| o.delete);
    let Some(chat) = p.s.teams.open.as_deref().filter(|_| deletable) else {
        c.deleting = None;
        return;
    };
    let t = &p.t;
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::unique("teams-delete-question")).frame(crate::dialogs::dialog_frame(t)).show(ctx, |ui| {
        ui.set_width(380.0);
        ui.label(RichText::new(tl!("Delete this message?")).font(theme::bold(18.0)).color(t.text));
        ui.add_space(4.0);
        ui.label(RichText::new(tl!("It will be deleted for everyone in this conversation.")).color(t.text));
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.button(tl!("Delete")).clicked() {
                answer = Some(true);
            }
            if ui.button(tl!("Cancel")).clicked() {
                answer = Some(false);
            }
        });
    });
    if answer.is_none() && modal.should_close() {
        answer = Some(false);
    }
    let Some(delete) = answer else { return };
    c.deleting = None;
    if delete {
        acts.push(Action::Run("teams.delete", json!({"chat": chat, "message": message})));
    }
}
