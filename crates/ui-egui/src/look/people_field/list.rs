//! The suggestions under the input: at most [`quatrelle_engine::MAX_SUGGESTIONS`] rows, each the
//! kind's icon, the name and the address, and the kind and where it comes from; a Recent one has
//! a × that forgets it.

use egui::{Pos2, Rect, Sense, pos2};

use super::*;
use crate::{icons, theme, widgets};

const ROW_H: f32 = 40.0;
/// The width the × at a row's end takes.
const X_W: f32 = 26.0;

/// The suggestions shown (their indices in `suggestions`): while the list is wanted, the input has
/// the focus and something typed that Esc didn't close it for, the kinds the field takes.
pub(super) fn rows(st: &FieldState, input: &FieldInput<'_>, focused: bool) -> Vec<usize> {
    if !st.open || !focused || st.input.trim().is_empty() || st.dismissed_for.as_deref() == Some(st.input.as_str()) {
        return Vec::new();
    }
    input
        .suggestions
        .iter()
        .enumerate()
        .filter(|(_, s)| input.accepts.takes(s.who.kind))
        .map(|(i, _)| i)
        .take(quatrelle_engine::MAX_SUGGESTIONS)
        .collect()
}

/// Draw the list at `at` (under the input, `edit_w` wide); whether a suggestion was taken, and
/// whether the pointer was pressed on the list.
pub(super) fn show(
    ui: &Ui,
    t: &Tokens,
    st: &mut FieldState,
    input: &FieldInput<'_>,
    rows: &[usize],
    at: Pos2,
    edit_w: f32,
    ev: &mut Vec<PeopleEvent>,
) -> (bool, bool) {
    let w = edit_w.clamp(300.0, 480.0);
    let mut picked = None;
    let area = egui::Area::new(super::list_id(input.id)).order(egui::Order::Foreground).fixed_pos(at).show(ui.ctx(), |ui| {
        egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (n, &k) in rows.iter().enumerate() {
                let Some(s) = input.suggestions.get(k) else { continue };
                if let Some(e) = row(ui, t, s, n == st.highlighted, w, input.id.with(("people-row", k))) {
                    picked = Some(match e {
                        Pick::Take => PeopleEvent::Take(k),
                        Pick::Forget => PeopleEvent::Forget(s.who.address.clone()),
                    });
                }
            }
        });
    });
    let hit = ui.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| area.response.rect.contains(p)));
    let took = matches!(picked, Some(PeopleEvent::Take(_)));
    ev.extend(picked);
    (took, hit)
}

enum Pick {
    Take,
    Forget,
}

/// One suggestion's row; what a press on it asks for.
fn row(ui: &mut Ui, t: &Tokens, s: &Suggestion, on: bool, w: f32, id: Id) -> Option<Pick> {
    // Rows take clicks without being Tab stops: the keyboard moves in the list with the arrows.
    let (rect, resp) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::CLICK);
    let name = s.who.display().to_string();
    widgets::named(ui, &resp, &name, true, None);
    let forgets = s.source == Source::Recent;
    // The × (Recent ones only), its own target so it's the one under the pointer, not the row.
    let x = Rect::from_center_size(pos2(rect.right() - X_W / 2.0, rect.center().y), vec2(20.0, 20.0));
    let on_x = forgets && {
        let r = ui.interact(x, id.with("x"), Sense::CLICK).on_hover_text(tl!("Remove from suggestions"));
        widgets::named(ui, &r, tl!("Remove from suggestions"), true, None);
        r.hovered()
    };
    let pointed = resp.hovered() || on_x;
    if on || pointed {
        ui.painter().rect_filled(rect, 4.0, if on { t.accent_soft } else { t.hover_row });
    }
    let p = ui.painter();
    let icon = Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), vec2(18.0, 18.0));
    icons::paint(p, icon, kind_icon(s.who.kind), t.accent);
    // The kind and the source on the right, the name and the address in what's left.
    let small = theme::regular(11.5);
    let right = rect.right() - X_W - 4.0;
    let source = p.layout_no_wrap(source_name(s.source).to_string(), small.clone(), t.faint);
    let kind = p.layout_no_wrap(kind_name(s.who.kind).to_string(), small, t.faint);
    let side = source.size().x.max(kind.size().x);
    let left = icon.right() + 10.0;
    let text_w = (right - side - 12.0 - left).max(40.0);
    let top = rect.top() + 3.0;
    let mid = rect.center().y + 1.0;
    let name_g = widgets::one_line(ui, &name, theme::regular(13.5), t.text, text_w);
    p.galley(pos2(left, top), name_g, t.text);
    let address = if s.who.address.is_empty() { kind_name(s.who.kind) } else { s.who.address.as_str() };
    let mail_g = widgets::one_line(ui, address, theme::regular(12.0), t.dim, text_w);
    p.galley(pos2(left, mid), mail_g, t.dim);
    p.galley(pos2(right - source.size().x, top + 2.0), source, t.faint);
    p.galley(pos2(right - kind.size().x, mid + 1.0), kind, t.faint);
    if forgets && (on || pointed) {
        if on_x {
            p.rect_filled(x, 4.0, t.danger.gamma_multiply(0.15));
        }
        icons::paint(p, x.shrink(3.0), Icon::Close, t.danger);
    }
    // Taken on the press, as the list answers at once; the field holds the input's focus and text
    // until the click is over (`people_field`).
    if pointed && ui.input(|i| i.pointer.primary_pressed()) {
        return Some(if on_x { Pick::Forget } else { Pick::Take });
    }
    None
}
