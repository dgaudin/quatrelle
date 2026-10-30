//! The field's body: its pieces (chips with their kind's icon, text not yet recognised) laid line
//! after line with the input after them, growing to [`MAX_LINES`] lines, then scrolling.

use egui::{Align, Layout, Response, Sense, Stroke, StrokeKind, TextWrapMode, UiBuilder, pos2};

use super::*;
use crate::{bidi, icons, theme, widgets};

/// The widest a chip gets (a longer name ends in "…").
const MAX_CHIP_W: f32 = 240.0;
/// The narrowest the input gets on the line of the last piece (else it starts a line).
const MIN_INPUT_W: f32 = 90.0;
const GAP: f32 = 4.0;
/// A chip's icon, and the room around its name.
const ICON: f32 = 14.0;
const PAD_LEFT: f32 = 6.0;
const PAD_RIGHT: f32 = 9.0;

/// The body as it was laid out.
pub(super) struct Laid {
    /// The input.
    pub(super) edit: Response,
    /// The lines in view.
    pub(super) view: egui::Rect,
    /// Each piece's rect.
    pub(super) rects: Vec<egui::Rect>,
    /// A piece was clicked: the input takes the focus back.
    pub(super) refocus: bool,
}

/// The body; `label_id` is the book button, which names the input to a screen reader.
pub(super) fn body(ui: &mut Ui, t: &Tokens, st: &mut FieldState, input: &FieldInput<'_>, edit_id: Id, label_id: Id) -> Laid {
    let FieldState { pieces, input: typed, selected, card, .. } = st;
    let card_open = card.is_some();
    let mut rects = Vec::with_capacity(pieces.len());
    let mut clicked = None;
    let out = egui::ScrollArea::vertical()
        .id_salt(input.id.with("people-lines"))
        .max_height(MAX_LINES as f32 * CHIP_ROW_H)
        .auto_shrink([false, true])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(GAP, GAP);
                for (i, piece) in pieces.iter().enumerate() {
                    let on = *selected == Some(i);
                    let resp = match piece {
                        Piece::Chip { who, refused } => chip(ui, t, who, refused.as_deref(), input.id.with(("people-chip", i)), on, card_open),
                        Piece::Text { text, error } => text_piece(ui, t, text, error.as_ref(), input.id.with(("people-text", i)), on),
                    };
                    if resp.clicked() {
                        clicked = Some(i);
                    }
                    rects.push(resp.rect);
                }
                if !pieces.is_empty() && ui.available_size_before_wrap().x < MIN_INPUT_W {
                    ui.end_row();
                }
                let spinner = if input.pending > 0 { ICON + GAP } else { 0.0 };
                let w = (ui.available_size_before_wrap().x - spinner).max(MIN_INPUT_W);
                let hint = if pieces.is_empty() { tl!(input.hint) } else { "" };
                let edit = egui::TextEdit::singleline(typed)
                    .id(edit_id)
                    .frame(egui::Frame::NONE)
                    .hint_text(hint)
                    .desired_width(w)
                    .align(egui::Align2::LEFT_CENTER);
                // Named by the field's button ("To"): its hint goes once there are chips.
                let r = ui.add_sized(vec2(w, CHIP_H), edit).labelled_by(label_id);
                if input.pending > 0 {
                    ui.add(egui::Spinner::new().size(ICON).color(t.dim)).on_hover_text(tl!("Checking…"));
                }
                r
            })
            .inner
        });
    let view = out.inner_rect;
    let edit = out.inner;
    let line = if edit.has_focus() { t.accent } else { t.border };
    ui.painter().line_segment([pos2(view.left(), view.bottom() + 1.0), pos2(view.right(), view.bottom() + 1.0)], Stroke::new(1.0, line));
    ui.add_space(2.0);
    if let Some(i) = clicked {
        *selected = Some(i);
        // A chip's card opens on a click, and closes on a second.
        *card = match pieces.get(i) {
            Some(Piece::Chip { .. }) if *card != Some(i) => Some(i),
            _ => None,
        };
    }
    Laid { edit, view, rects, refocus: clicked.is_some() }
}

/// A slot for a piece whose text is `w` wide, and its response. It takes clicks but isn't a Tab
/// stop (a field holds up to 500): the keyboard selects pieces from the input, with Backspace.
fn slot(ui: &mut Ui, id: Id, w: f32) -> (egui::Rect, Response) {
    let (rect, _) = ui.allocate_exact_size(vec2(w, CHIP_H), Sense::hover());
    let resp = ui.interact(rect, id, Sense::CLICK);
    (rect, resp)
}

/// A line of text in `rect` through `bidi::label`: a right-to-left name where it starts, a long
/// one ending in "…".
fn name_in(ui: &mut Ui, rect: egui::Rect, text: &str, color: egui::Color32) -> Response {
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::left_to_right(Align::Center)));
    child.style_mut().wrap_mode = Some(TextWrapMode::Truncate);
    child.style_mut().interaction.selectable_labels = false;
    bidi::label(&mut child, text, theme::regular(13.0), color)
}

fn text_width(ui: &Ui, text: &str) -> f32 {
    ui.painter().layout_no_wrap(text.to_string(), theme::regular(13.0), egui::Color32::WHITE).size().x.ceil() + 1.0
}

/// A chip: a pill with the kind's icon and the name; the address shows on hover (not while a card
/// is open: one at a time). A refused one is red, and says why on hover and to a screen reader.
fn chip(ui: &mut Ui, t: &Tokens, a: &Addressee, refused: Option<&str>, id: Id, selected: bool, card_open: bool) -> Response {
    let name = a.display();
    let text_w = text_width(ui, name).min(MAX_CHIP_W - PAD_LEFT - ICON - 5.0 - PAD_RIGHT);
    let (rect, resp) = slot(ui, id, PAD_LEFT + ICON + 5.0 + text_w + PAD_RIGHT);
    let kind = kind_name(a.kind);
    let said = match refused {
        Some(why) => tf!("{name}, {kind}, {why}, remove with Delete", name = name, kind = kind, why = why),
        None => tf!("{name}, {kind}, remove with Delete", name = name, kind = kind),
    };
    widgets::named(ui, &resp, &said, true, Some(selected));
    // A field holds up to 500: those scrolled out of its lines take their place and aren't drawn.
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let pill = rect.shrink2(vec2(0.0, 2.0));
    let fill = if selected {
        t.accent_soft
    } else if resp.hovered() {
        t.hover_row
    } else {
        t.hover
    };
    let (ink, text_color) = match refused {
        Some(_) => {
            let red = if selected { t.danger.gamma_multiply(0.22) } else { t.danger.gamma_multiply(0.1) };
            ui.painter().rect(pill, pill.height() / 2.0, red, Stroke::new(1.0, t.danger), StrokeKind::Inside);
            (t.danger, t.danger)
        }
        None => {
            ui.painter().rect_filled(pill, pill.height() / 2.0, fill);
            (t.accent, t.text)
        }
    };
    let icon = egui::Rect::from_center_size(pos2(pill.left() + PAD_LEFT + ICON / 2.0, pill.center().y), vec2(ICON, ICON));
    icons::paint(ui.painter(), icon, kind_icon(a.kind), ink);
    let text = egui::Rect::from_min_max(pos2(icon.right() + 5.0, pill.top()), pos2(pill.right() - PAD_RIGHT, pill.bottom()));
    name_in(ui, text, name, text_color);
    let tip = match refused {
        Some(why) => why,
        None if a.address.is_empty() => kind,
        None => a.address.as_str(),
    };
    if card_open { resp } else { resp.on_hover_text(tip) }
}

/// Text not recognised: as typed, red-underlined with its reason on hover when it has an error.
fn text_piece(ui: &mut Ui, t: &Tokens, text: &str, error: Option<&PieceError>, id: Id, selected: bool) -> Response {
    let text_w = text_width(ui, text).min(MAX_CHIP_W - 2.0 * PAD_LEFT);
    let (rect, resp) = slot(ui, id, PAD_LEFT + text_w + PAD_LEFT);
    let message = error.map(|e| error_text(text, e));
    // The reason is in what's read out when it names the text; otherwise the text is.
    let said = match (error, &message) {
        (Some(PieceError::NotAnAddress | PieceError::NotFound), Some(m)) => m.as_str(),
        _ => text,
    };
    widgets::named(ui, &resp, said, true, Some(selected));
    let pill = rect.shrink2(vec2(0.0, 2.0));
    if selected || resp.hovered() {
        ui.painter().rect_filled(pill, 4.0, if selected { t.accent_soft } else { t.hover_row });
    }
    let inner = egui::Rect::from_min_max(pos2(pill.left() + PAD_LEFT, pill.top()), pos2(pill.right() - PAD_LEFT, pill.bottom()));
    let label = name_in(ui, inner, text, t.text);
    match message {
        Some(m) => {
            let y = label.rect.bottom() + 0.5;
            ui.painter().line_segment([pos2(label.rect.left(), y), pos2(label.rect.right(), y)], Stroke::new(1.5, t.danger));
            resp.on_hover_text(m)
        }
        None => resp,
    }
}
