// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The reading pane's buttons: pills (opening a menu or not), the group of buttons side by side
//! (Reply | Reply all | Forward), and the icon before a menu command's label.

use super::*;

/// A "⋯" button in the style of [`pill_button`], for a menu of more actions.
pub fn more_pill(ui: &mut Ui, tip: &str, t: &Tokens) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(34.0, 28.0), Sense::click());
    named(ui, &resp, tl!(tip), true, None);
    let fill = if resp.hovered() { t.hover } else { t.surface };
    ui.painter().rect(rect, 6.0, fill, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    for dx in [-6.0, 0.0, 6.0] {
        ui.painter().circle_filled(rect.center() + vec2(dx, 0.0), 1.6, t.dim);
    }
    resp.on_hover_text(tl!(tip))
}

/// An icon before a menu command's label, as Outlook's menus show theirs:
/// `ui.button((widgets::menu_icon(Some(Icon::Reply)), tl!("Reply")))`, a submenu's
/// `ui.menu_button((…, label), …)` too. Drawn in the label's colour (greyed with it); `None`
/// keeps its room, so the labels line up.
pub fn menu_icon(icon: Option<Icon>) -> egui::Atom<'static> {
    egui::Atom::paint(vec2(16.0, 16.0), move |ui, args| {
        if let Some(icon) = icon {
            icons::paint(ui.painter(), args.rect, icon, args.fallback_text_color);
        }
    })
}

/// A [`pill_button`] that opens a menu (a ▾ after its label), grey and still when not `enabled`;
/// its icon alone when `compact` (its label then a tip).
pub fn menu_pill(ui: &mut Ui, icon: Icon, label: &str, t: &Tokens, enabled: bool, compact: bool) -> Response {
    let label = tl!(label);
    let ink = if enabled { t.text } else { t.dim };
    let galley = ui.painter().layout_no_wrap(label.to_string(), theme::regular(13.0), ink);
    let size = vec2(menu_pill_width(ui, label, compact), 28.0);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    named(ui, &resp, label, enabled, None);
    let fill = if enabled && resp.hovered() { t.hover } else { t.surface };
    ui.painter().rect(rect, 6.0, fill, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    let tint = if enabled { t.accent } else { t.dim };
    icons::paint(ui.painter(), egui::Rect::from_min_size(rect.min + vec2(9.0, 6.0), vec2(16.0, 16.0)), icon, tint);
    if !compact {
        ui.painter().galley_with_override_text_color(rect.min + vec2(29.0, (28.0 - galley.size().y) / 2.0), galley, ink);
    }
    let arrow = egui::Rect::from_min_size(rect.right_top() + vec2(-19.0, 9.0), vec2(10.0, 10.0));
    icons::paint(ui.painter(), arrow, Icon::ChevronDown, t.dim);
    if compact { resp.on_hover_text(label) } else { resp }
}

/// How wide a [`menu_pill`] with this (translated) label is.
pub fn menu_pill_width(ui: &Ui, label: &str, compact: bool) -> f32 {
    if compact { 50.0 } else { ui.painter().layout_no_wrap(label.to_string(), theme::regular(13.0), Color32::WHITE).size().x + 50.0 }
}

/// Buttons side by side in one frame, a thin line between each, as Outlook's Reply | Reply all |
/// Forward: each its icon and label, or its icon alone when `compact` (its label then a tip).
/// Which was clicked, if one was.
pub fn button_group(ui: &mut Ui, items: &[(Icon, &str)], t: &Tokens, compact: bool) -> Option<usize> {
    let widths: Vec<f32> = items.iter().map(|(_, l)| group_part_width(ui, tl!(l), compact)).collect();
    let (rect, _) = ui.allocate_exact_size(vec2(widths.iter().sum(), 28.0), Sense::hover());
    let mut clicked = None;
    let mut x = rect.left();
    let last = items.len().saturating_sub(1);
    for (i, ((icon, label), w)) in items.iter().zip(&widths).enumerate() {
        let label = tl!(label);
        let part = egui::Rect::from_min_size(egui::pos2(x, rect.top()), vec2(*w, 28.0));
        let resp = ui.interact(part, ui.scope_id().with(("button-group", i, label)), Sense::click());
        named(ui, &resp, label, true, None);
        if resp.hovered() {
            let round = |on: bool| if on { 6 } else { 0 };
            let corners = egui::CornerRadius { nw: round(i == 0), sw: round(i == 0), ne: round(i == last), se: round(i == last) };
            ui.painter().rect_filled(part.shrink(1.0), corners, t.hover);
        }
        if i > 0 {
            ui.painter().vline(x, part.y_range().shrink(5.0), Stroke::new(1.0, t.border));
        }
        icons::paint(ui.painter(), egui::Rect::from_min_size(part.min + vec2(9.0, 6.0), vec2(16.0, 16.0)), *icon, t.accent);
        if !compact {
            let galley = ui.painter().layout_no_wrap(label.to_string(), theme::regular(13.0), t.text);
            ui.painter().galley_with_override_text_color(part.min + vec2(29.0, (28.0 - galley.size().y) / 2.0), galley, t.text);
        }
        let resp = if compact { resp.on_hover_text(label) } else { resp };
        if resp.clicked() {
            clicked = Some(i);
        }
        x += w;
    }
    ui.painter().rect_stroke(rect, 6.0, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    clicked
}

/// How wide a [`button_group`] of these labels is.
pub fn button_group_width(ui: &Ui, labels: &[&str], compact: bool) -> f32 {
    labels.iter().map(|l| group_part_width(ui, tl!(l), compact)).sum()
}

fn group_part_width(ui: &Ui, label: &str, compact: bool) -> f32 {
    if compact { 34.0 } else { ui.painter().layout_no_wrap(label.to_string(), theme::regular(13.0), Color32::WHITE).size().x + 39.0 }
}

/// A labelled icon button for the reading pane's action row.
pub fn pill_button(ui: &mut Ui, icon: Icon, label: &str, t: &Tokens) -> Response {
    let label = tl!(label);
    let galley = ui.painter().layout_no_wrap(label.to_string(), theme::regular(13.0), t.text);
    let size = vec2(galley.size().x + 34.0, 28.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    named(ui, &resp, label, true, None);
    let fill = if resp.hovered() { t.hover } else { t.surface };
    ui.painter().rect(rect, 6.0, fill, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    icons::paint(ui.painter(), egui::Rect::from_min_size(rect.min + vec2(9.0, 6.0), vec2(16.0, 16.0)), icon, t.accent);
    ui.painter().galley_with_override_text_color(rect.min + vec2(29.0, (28.0 - galley.size().y) / 2.0), galley, t.text);
    resp
}
