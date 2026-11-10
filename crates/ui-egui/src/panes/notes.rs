// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Notes, as Outlook's Notes module: coloured sticky notes in a grid, each showing its text as
//! its Markdown sets it (the marks pale), cut at its edge. A click selects a note; a
//! double-click, Enter or New note opens it in a window of its own, where it is written
//! (`notes/window.rs`, `notes/editor.rs`); Delete deletes the one selected; right-click gives
//! its colours and Delete. Exchange accounts keep them in the mailbox's Notes folder, others on
//! this computer.

use egui::{Color32, Key, Margin, Modifiers, RichText, Sense, Ui, vec2};
use quatrelle_engine::Session;
use quatrelle_engine::model::dates;
use quatrelle_engine::model::pim::Note;
use serde_json::json;

use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{Action, UiState, widgets};

mod editor;
mod window;

#[cfg(test)]
pub(crate) use window::{MIN, STEADY, Sizer, USUAL, in_page_id, text_id};
pub(crate) use window::{NoteWindow, open, windows};

/// Outlook's note colours, in its order: blue, green, pink, yellow, white.
pub(crate) const NAMES: [&str; 5] = ["Blue", "Green", "Pink", "Yellow", "White"];

/// Most of a note's text a tile lays out: what fits on it, and more.
const TILE_TEXT: usize = 4096;

/// A note's paper (light in both themes, as sticky notes are), and its text.
pub(crate) fn paper(color: u8, dark: bool) -> Color32 {
    let c = match color {
        0 => [0xD4, 0xEC, 0xFB],
        1 => [0xD6, 0xF5, 0xD6],
        2 => [0xFB, 0xD8, 0xEE],
        4 => [0xFF, 0xFF, 0xFF],
        _ => [0xFF, 0xF4, 0xC2],
    };
    let c = Color32::from_rgb(c[0], c[1], c[2]);
    if dark { c.gamma_multiply(0.9) } else { c }
}

const INK: Color32 = Color32::from_rgb(0x1F, 0x23, 0x28);

/// The side pane: a new note, how many there are (searched from the box at the top).
pub fn nav(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    if widgets::accent_button(ui, Icon::Plus, "New note", &t, !s.accounts.is_empty()).clicked() {
        new_note(acts);
    }
    ui.add_space(8.0);
    let n = s.notes(&st.notes_query).len();
    ui.label(RichText::new(tf!("{n} notes", n = n)).color(t.faint).small());
}

/// Start a note and open its window at once (the app makes it, then opens it by its uid).
pub(crate) fn new_note(acts: &mut Vec<Action>) {
    acts.push(Action::NewNote);
}

/// A note's colours (its own ticked), as its right-click menu and its window's ⋯ give them.
pub(crate) fn colors_menu(ui: &mut Ui, note: &Note, t: &Tokens, acts: &mut Vec<Action>) {
    for (i, name) in NAMES.iter().enumerate() {
        let c = u8::try_from(i).unwrap_or(3);
        let clicked = ui
            .horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                ui.painter().rect(r, 3.0, paper(c, false), egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
                ui.selectable_label(note.color == c, tl!(name)).clicked()
            })
            .inner;
        if clicked {
            acts.push(Action::Run("note.update", json!({"uid": note.uid, "color": c})));
            ui.close();
        }
    }
}

/// The main pane: the notes, side by side.
pub fn show(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let notes = s.notes(&st.notes_query);
    if notes.is_empty() {
        ui.centered_and_justified(|ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.35);
                let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
                icons::paint(ui.painter(), r, Icon::Note, t.border);
                ui.add_space(8.0);
                let empty = st.notes_query.trim().is_empty();
                ui.label(RichText::new(tl!(if empty { "No notes yet: New note starts one." } else { "No note matches." })).color(t.faint));
            });
        });
        return;
    }
    // The grid's keys, while no text box has the keyboard: Enter opens the note selected,
    // Delete deletes it.
    let ctx = ui.ctx().clone();
    let free = !ctx.text_edit_focused() && st.dialog.is_none() && !ctx.any_popup_open();
    if free && let Some(n) = st.note_selected.as_ref().and_then(|u| notes.iter().find(|n| &n.note.uid == u)) {
        let (enter, delete) = ui.input_mut(|i| (i.consume_key(Modifiers::NONE, Key::Enter), i.consume_key(Modifiers::NONE, Key::Delete)));
        if delete {
            acts.push(Action::Run("note.delete", json!({"uid": n.note.uid})));
            st.note_selected = None;
        } else if enter {
            open(st, &n.note);
        }
    }
    egui::ScrollArea::vertical().auto_shrink(false).id_salt("notes").show(ui, |ui| {
        egui::Frame::new().inner_margin(Margin::same(18)).show(ui, |ui| {
            let size = 210.0;
            let gap = 14.0;
            let cols = (((ui.available_width() + gap) / (size + gap)).floor() as usize).max(1);
            for row in notes.chunks(cols) {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for n in row {
                        tile(s, &t, st, ui, &n.note, size, acts);
                    }
                });
                ui.add_space(gap);
            }
        });
    });
}

/// One note: its paper, its text as its window sets it (cut at the edge), when it last changed.
/// A click selects it, a double-click opens it; right-click gives its colours and Delete.
fn tile(s: &Session, t: &Tokens, st: &mut UiState, ui: &mut Ui, note: &Note, size: f32, acts: &mut Vec<Action>) {
    let uid = &note.uid;
    let selected = st.note_selected.as_ref() == Some(uid);
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let title = editor::title(&note.text);
    let name = if title.is_empty() { tl!("Untitled").to_string() } else { title };
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, &name));
    let fill = paper(note.color, t.dark);
    // A shadow, the paper, its folded corner.
    let lifted = resp.hovered() || selected;
    ui.painter().rect_filled(rect.translate(vec2(0.0, 2.0)), 4.0, Color32::from_black_alpha(if lifted { 40 } else { 22 }));
    ui.painter().rect(
        rect,
        4.0,
        fill,
        egui::Stroke::new(if selected { 2.0 } else { 1.0 }, if selected { t.accent } else { fill.gamma_multiply(0.85) }),
        egui::StrokeKind::Inside,
    );
    let fold = 18.0;
    let corner = [rect.right_bottom() - vec2(0.0, fold), rect.right_bottom() - vec2(fold, 0.0), rect.right_bottom() - vec2(fold, fold)];
    ui.painter().add(egui::Shape::convex_polygon(corner.to_vec(), fill.gamma_multiply(0.8), egui::Stroke::NONE));
    let inner = rect.shrink2(vec2(12.0, 10.0));
    let when = crate::i18n::date(&dates::list_time(note.modified, &s.clock));
    ui.painter().text(egui::pos2(inner.left(), inner.bottom()), egui::Align2::LEFT_BOTTOM, when, theme::regular(11.0), INK.gamma_multiply(0.55));
    let text_rect = egui::Rect::from_min_max(inner.min, egui::pos2(inner.right(), inner.bottom() - 18.0));
    // What fits on it: its first few thousand bytes, laid out as in its window.
    let mut end = note.text.len().min(TILE_TEXT);
    while end > 0 && !note.text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    let shown = note.text.get(..end).unwrap_or("");
    let galley = ui.painter().layout_job(editor::job(shown, text_rect.width(), editor::Look::TILE));
    ui.painter().with_clip_rect(text_rect).galley(text_rect.min, galley, INK);
    if resp.clicked() {
        st.note_selected = Some(uid.clone());
    }
    if resp.double_clicked() {
        open(st, note);
    }
    resp.context_menu(|ui| {
        if ui.button((widgets::menu_icon(Some(Icon::Note)), tl!("Open"))).clicked() {
            open(st, note);
            ui.close();
        }
        ui.separator();
        colors_menu(ui, note, t, acts);
        ui.separator();
        if ui.button((widgets::menu_icon(Some(Icon::Delete)), RichText::new(tl!("Delete")).color(t.danger))).clicked() {
            acts.push(Action::Run("note.delete", json!({"uid": uid})));
            ui.close();
        }
    });
}
