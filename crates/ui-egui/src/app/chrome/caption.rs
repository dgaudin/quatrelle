// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The window's own title bar ([`crate::Services::own_title_bar`]): Quatrelle's top bar is the
//! title bar, as Outlook's is, instead of a bar under the system's. From PhotoCraft's own title
//! bar, as MailCraft took it: the window's buttons at the bar's end, the bar's free space that
//! drags the window (a double-click maximizes it), and thin zones along the window's edges that
//! resize it. Windows keeps the window borderless meanwhile ([`crate::Services::keep_frame`]);
//! elsewhere it opens so. While a dialog holds the window, the bar's drag and buttons are laid
//! again above it ([`over_a_modal`], PhotoCraft's `modal_drag_overlay`).

use egui::{Color32, CursorIcon, Id, LayerId, Order, PointerButton, Rect, ResizeDirection, Sense, Ui, ViewportCommand, pos2, vec2};

use crate::look::theme::Tokens;
use crate::*;

/// The top bar's height, and its margins across and down.
pub(crate) const TOP_BAR_H: f32 = 40.0;
pub(crate) const TOP_BAR_MARGIN: (i8, i8) = (10, 5);
/// One of the window's buttons: as wide as Windows' own.
pub(crate) const BUTTON_W: f32 = 46.0;
/// The three of them.
pub(crate) const CAPTIONS_W: f32 = 3.0 * BUTTON_W;
/// Resize zone thickness along an edge, and the side of a corner zone (PhotoCraft found 5 pt hard
/// to find; 8 pt is about a native resize border).
const EDGE: f32 = 8.0;
const CORNER: f32 = 12.0;
/// The main window's smallest inner size: the system's own resize stops there.
pub const MIN_WINDOW_SIZE: egui::Vec2 = vec2(760.0, 480.0);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Caption {
    Minimize,
    Maximize,
    Close,
}

fn maximized(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false))
}

/// The window's Maximize or Restore, noted for the frame's end ([`take_zooming`]) so the window
/// is told first.
fn toggle_maximized(ctx: &egui::Context) {
    let max = maximized(ctx);
    ctx.data_mut(|d| d.insert_temp(zooming_id(), true));
    ctx.send_viewport_cmd(ViewportCommand::Maximized(!max));
}

fn zooming_id() -> Id {
    Id::unique("title-bar-zooming")
}

/// Whether this frame's title bar asked to maximize or restore the window, the once: the command
/// runs after the frame, so what must come before it ([`crate::Services::before_zoom`]) goes here.
fn take_zooming(ctx: &egui::Context) -> bool {
    ctx.data_mut(|d| d.remove_temp::<bool>(zooming_id())).unwrap_or(false)
}

/// What a frame does first while the window draws its own title bar: the window kept borderless
/// (winit puts the system's frame back when it's maximized or restored), and the scroll bars kept
/// clear of its resize edges.
pub(crate) fn begin_frame(ctx: &egui::Context, services: &mut dyn Services) {
    if services.own_title_bar() {
        services.keep_frame();
        clear_scroll_bars(ctx);
    }
}

/// What a frame lays last while the window draws its own title bar, over everything else: the
/// bar again over a dialog holding the window, and the window's resize edges.
pub(crate) fn end_frame(ui: &mut Ui, t: &Tokens, services: &dyn Services) {
    if services.own_title_bar() {
        over_a_modal(ui.ctx(), t);
        resize_zones(ui);
    }
}

/// The window is told before this frame's maximize or restore by its title bar, if there is one:
/// the command runs after the frame.
pub(crate) fn tell_before_zooming(ctx: &egui::Context, services: &mut dyn Services) {
    if take_zooming(ctx) && services.own_title_bar() {
        services.before_zoom();
        // A frame after Windows' zoom animation, whatever else is going on then.
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }
}

/// The bar's free space, `rect`: a press and a move drag the window (the system snaps it at the
/// screen's edges), a double-click maximizes or restores it. Laid down before the bar's own
/// controls, so they keep their clicks; where it's laid under or over them (`except`: the bar's
/// controls, the window's buttons), a press there does nothing (a button takes no drag, so the
/// drag would have gone to the bar behind it). Not in the Tab order, as a system title bar isn't.
fn drag_area(ui: &mut Ui, rect: Rect, except: &[Rect]) {
    let resp = ui.interact(rect, ui.scope_id().with("title-bar-drag"), Sense::CLICK | Sense::DRAG);
    // Where the press began, the pointer having moved on since (or where it clicked).
    let at = ui.input(|i| i.pointer.press_origin()).or_else(|| resp.interact_pointer_pos());
    if at.is_some_and(|p| except.iter().any(|r| r.contains(p))) {
        return;
    }
    if resp.drag_started_by(PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
    if resp.double_clicked() {
        toggle_maximized(ui.ctx());
    }
}

/// The title bar's top row, where the window's buttons are, of `bar` (its whole rect).
pub(crate) fn top_row(bar: Rect) -> Rect {
    Rect::from_min_size(bar.min, vec2(bar.width(), TOP_BAR_H))
}

/// Where the window's three buttons are, at the end of `top` (the title bar's top row), flush
/// with the window's edge.
fn buttons_rect(ctx: &egui::Context, top: Rect) -> Rect {
    let right = top.right().min(ctx.content_rect().right());
    Rect::from_min_max(pos2(right - CAPTIONS_W, top.top()), pos2(right, top.bottom()))
}

/// Quatrelle's own title bar over `bar`, its whole rect (the top bar's panel, margins and all): its
/// free space to drag the window by, laid first so what's drawn on it keeps its clicks, and the
/// window's buttons at the end of its top row, flush with the window's corner (named to a screen
/// reader unless they're laid again over a dialog). Where the bar's own contents go: inside its
/// margins, clear of the buttons.
pub(crate) fn title_bar(ui: &mut Ui, t: &Tokens, bar: Rect) -> Rect {
    let top = top_row(bar);
    let mut except = noted_bar(ui.ctx()).map(|(_, _, controls)| controls).unwrap_or_default();
    except.push(buttons_rect(ui.ctx(), top));
    drag_area(ui, bar, &except);
    let live = !laid_over_a_dialog(ui.ctx());
    caption_buttons(ui, t, top, live);
    let margin = vec2(f32::from(TOP_BAR_MARGIN.0), f32::from(TOP_BAR_MARGIN.1));
    let inner = bar.shrink2(margin);
    Rect::from_min_max(inner.min, pos2(inner.max.x - (CAPTIONS_W - margin.x), inner.max.y))
}

/// The minimize, maximize (restore) and close buttons at the right end of `top` (the title bar's
/// top row), as tall as it and flush with the window's edge, so a maximized window's corner hits
/// Close. Close asks the window to close, as the system's button does (Quatrelle then asks about
/// messages being written). Not in the Tab order, as the system's aren't; named to a screen
/// reader when `live` (not the ones a dialog has dimmed, laid again over it).
fn caption_buttons(ui: &mut Ui, t: &Tokens, top: Rect, live: bool) {
    let max = maximized(ui.ctx());
    let all = buttons_rect(ui.ctx(), top);
    let mut clicked = None;
    for (i, c) in [Caption::Minimize, Caption::Maximize, Caption::Close].into_iter().enumerate() {
        let left = all.left() + i as f32 * BUTTON_W;
        let r = Rect::from_min_max(pos2(left, all.top()), pos2(left + BUTTON_W, all.bottom()));
        let resp = ui.interact(r, ui.scope_id().with(("title-bar-caption", i)), Sense::CLICK);
        let close = c == Caption::Close;
        // Close turns red under the pointer, its cross white, as the system's does.
        let fill = match (resp.is_pointer_button_down_on(), resp.hovered()) {
            (true, _) if close => t.danger.gamma_multiply(0.85),
            (_, true) if close => t.danger,
            (true, _) => t.accent_soft,
            (_, true) => t.hover,
            _ => Color32::TRANSPARENT,
        };
        ui.painter().rect_filled(r, 0.0, fill);
        let ink = match (close, resp.hovered()) {
            (true, true) => Color32::WHITE,
            (false, true) => t.text,
            _ => t.dim,
        };
        let (icon, tip) = match c {
            Caption::Minimize => (Icon::Minimize, tl!("Minimize")),
            Caption::Maximize if max => (Icon::Restore, tl!("Restore")),
            Caption::Maximize => (Icon::Maximize, tl!("Maximize")),
            Caption::Close => (Icon::Close, tl!("Close")),
        };
        icons::paint(ui.painter(), icons::centered(r, 14.0), icon, ink);
        if live {
            widgets::named(ui, &resp, tip, true, None);
        }
        if resp.on_hover_text(tip).clicked() {
            clicked = Some(c);
        }
    }
    match clicked {
        Some(Caption::Minimize) => ui.ctx().send_viewport_cmd(ViewportCommand::Minimized(true)),
        Some(Caption::Maximize) => toggle_maximized(ui.ctx()),
        Some(Caption::Close) => ui.ctx().send_viewport_cmd(ViewportCommand::Close),
        None => {}
    }
}

/// Where the title bar was drawn: its whole rect (what drags the window), its top row (where the
/// window's buttons are) and its controls (the mark, the search box, its buttons), for
/// [`over_a_modal`] and the next frame's drag.
fn bar_id() -> Id {
    Id::unique("title-bar-rects")
}

/// Note where the title bar was drawn this frame.
pub(crate) fn note_bar(ctx: &egui::Context, whole: Rect, top: Rect, controls: Vec<Rect>) {
    ctx.data_mut(|d| d.insert_temp(bar_id(), (whole, top, controls)));
}

/// Where the title bar was last drawn: its whole rect, its top row and its controls.
fn noted_bar(ctx: &egui::Context) -> Option<(Rect, Rect, Vec<Rect>)> {
    ctx.data(|d| d.get_temp::<(Rect, Rect, Vec<Rect>)>(bar_id()))
}

/// The title bar laid again over a dialog holding the window, as last drawn: `None` without a
/// dialog, or when one, or a menu, reaches into the bar.
fn over_a_dialog(ctx: &egui::Context) -> Option<(Rect, Rect, Vec<Rect>)> {
    let modal = ctx.memory(|m| m.top_modal_layer())?;
    let (whole, top, controls) = noted_bar(ctx)?;
    let clear = whole.is_positive() && !egui::Popup::is_any_open(ctx) && !ctx.memory(|m| m.area_rect(modal.id)).is_some_and(|r| r.intersects(whole));
    clear.then_some((whole, top, controls))
}

/// Whether the title bar is laid again over a dialog: the dimmed one under it is then not named
/// to a screen reader (one Close, not two).
fn laid_over_a_dialog(ctx: &egui::Context) -> bool {
    over_a_dialog(ctx).is_some()
}

/// While a dialog holds the window (an `egui::Modal`: the question as Quatrelle closes, Add
/// account…), nothing under it takes a click, the title bar neither: the window couldn't be
/// moved, minimized or closed with the mouse. Its drag and buttons are laid again above
/// everything, as a system title bar stays usable over a dialog, unless the dialog itself, or a
/// menu, reaches into the bar. PhotoCraft's `modal_drag_overlay` (its #4913).
fn over_a_modal(ctx: &egui::Context, t: &Tokens) {
    let Some((whole, top, mut controls)) = over_a_dialog(ctx) else { return };
    egui::Area::new(Id::unique("title-bar-over-modal")).order(Order::Tooltip).fixed_pos(whole.min).movable(false).sense(Sense::hover()).show(
        ctx,
        |ui| {
            ui.allocate_rect(whole, Sense::hover());
            // Its free space only (PhotoCraft's gap): a double-click on the search box doesn't
            // maximize.
            controls.push(buttons_rect(ctx, top));
            drag_area(ui, whole, &controls);
            caption_buttons(ui, t, top, true);
        },
    );
}

/// The edge and corner zones of a window with content rect `w`: (zone, direction, cursor), the
/// corners last so they win where they overlap the edges.
fn zones(w: Rect) -> [(Rect, ResizeDirection, CursorIcon); 8] {
    use ResizeDirection::*;
    let (e, c) = (EDGE, CORNER);
    [
        (Rect::from_min_max(w.left_top(), pos2(w.right(), w.top() + e)), North, CursorIcon::ResizeNorth),
        (Rect::from_min_max(pos2(w.left(), w.bottom() - e), w.right_bottom()), South, CursorIcon::ResizeSouth),
        (Rect::from_min_max(w.left_top(), pos2(w.left() + e, w.bottom())), West, CursorIcon::ResizeWest),
        (Rect::from_min_max(pos2(w.right() - e, w.top()), w.right_bottom()), East, CursorIcon::ResizeEast),
        (Rect::from_min_size(w.left_top(), vec2(c, c)), NorthWest, CursorIcon::ResizeNwSe),
        (Rect::from_min_size(w.right_top() - vec2(c, 0.0), vec2(c, c)), NorthEast, CursorIcon::ResizeNeSw),
        (Rect::from_min_size(w.left_bottom() - vec2(0.0, c), vec2(c, c)), SouthWest, CursorIcon::ResizeNeSw),
        (Rect::from_min_size(w.right_bottom() - vec2(c, c), vec2(c, c)), SouthEast, CursorIcon::ResizeNwSe),
    ]
}

/// Scroll bars kept clear of the window's resize edges: the reading pane's lay in the last 10 pt
/// of the window, under the right edge's zone, so pressing it resized the window instead
/// (MailCraft found it in review). Every scroll bar keeps the zone's width from its area's edge.
fn clear_scroll_bars(ctx: &egui::Context) {
    if ctx.global_style().spacing.scroll.bar_outer_margin != EDGE {
        ctx.all_styles_mut(|s| s.spacing.scroll.bar_outer_margin = EDGE);
    }
}

/// Invisible resize zones along the window's edges, above everything else; a press in one starts
/// a resize by the system in that direction. None while the window is maximized or full screen.
fn resize_zones(ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    if maximized(&ctx) || ctx.input(|i| i.viewport().fullscreen.unwrap_or(false)) {
        return;
    }
    let w = ctx.content_rect();
    let layer = LayerId::new(Order::Foreground, Id::unique("title-bar-resize"));
    let zui = ui.new_child(egui::UiBuilder::new().layer_id(layer).max_rect(w));
    for (i, (r, dir, cursor)) in zones(w).into_iter().enumerate() {
        let resp = zui.interact(r, Id::unique(("title-bar-resize", i)), Sense::DRAG).on_hover_cursor(cursor);
        if resp.drag_started_by(PointerButton::Primary) {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
        }
    }
}
