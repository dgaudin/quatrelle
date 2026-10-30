//! A chip's card, open on a click: the name, the address and the kind, "Remove", and "Expand" for a
//! distribution list or a contact group when the account can expand one.

use egui::{Margin, RichText, Sense};

use super::*;
use crate::{bidi, icons, theme, widgets};

/// Draw the card of the chip `st.card` under it (`rects` are the pieces'); whether the pointer was
/// pressed on the card. A press anywhere else closes it.
pub(super) fn show(ui: &Ui, t: &Tokens, st: &mut FieldState, input: &FieldInput<'_>, rects: &[egui::Rect], ev: &mut Vec<PeopleEvent>) -> bool {
    let Some(i) = st.card else { return false };
    let (Some(Piece::Chip(a)), Some(&chip)) = (st.pieces.get(i), rects.get(i)) else {
        st.card = None;
        return false;
    };
    let mut asked = None;
    let area = egui::Area::new(input.id.with("people-card")).order(egui::Order::Foreground).fixed_pos(chip.left_bottom() + vec2(0.0, 4.0)).show(
        ui.ctx(),
        |ui| {
            egui::Frame::popup(ui.style()).inner_margin(Margin::same(12)).show(ui, |ui| {
                ui.set_min_width(220.0);
                ui.set_max_width(320.0);
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                    icons::paint(ui.painter(), r, kind_icon(a.kind), t.accent);
                    bidi::label(ui, a.display(), theme::bold(14.0), t.text);
                });
                if !a.address.is_empty() {
                    bidi::selectable_label(ui, &a.address, theme::regular(12.5), t.dim);
                }
                ui.label(RichText::new(kind_name(a.kind)).font(theme::regular(12.0)).color(t.faint));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if widgets::pill_button(ui, Icon::Delete, "Remove", t).clicked() {
                        asked = Some(PeopleEvent::RemoveChip(i));
                    }
                    if a.kind.is_list() && input.can_expand && widgets::pill_button(ui, Icon::People, "Expand", t).clicked() {
                        asked = Some(PeopleEvent::Expand(i));
                    }
                });
            });
        },
    );
    let card = area.response.rect;
    let (pressed, on_card, on_chip) = ui.input(|i| {
        let at = i.pointer.interact_pos();
        (i.pointer.any_pressed(), at.is_some_and(|p| card.contains(p)), at.is_some_and(|p| chip.contains(p)))
    });
    if let Some(e) = asked {
        ev.push(e);
        st.card = None;
        st.selected = None;
        return true;
    }
    if pressed && !on_card && !on_chip {
        st.card = None;
    }
    pressed && on_card
}
