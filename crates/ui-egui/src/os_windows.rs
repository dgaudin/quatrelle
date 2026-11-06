// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The windows Quatrelle opens of its own on the system: their place asked for at their
//! opening only, and what each says about itself when traced.

use egui::Ui;

/// Whether this is the first frame of a window: its place is asked for once, at its opening, and
/// left to the window manager from then on. A place handed in at every frame made a window the
/// window manager had just placed jump between the two places, frame after frame (GNOME put a
/// compose window at one spot, the app asked for another, and the window flashed between them).
pub(super) fn first_frame(ctx: &egui::Context, viewport: egui::ViewportId) -> bool {
    let id = egui::Id::unique(("first-frame", viewport));
    let seen = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    if !seen {
        ctx.data_mut(|d| d.insert_temp(id, true));
    }
    !seen
}

/// `QUATRELLE_TRACE_WINDOWS=1`: what a window of the app says about itself — its outer and inner
/// rectangles, its monitor, its scale — logged each time it changes, with the window's id, so a
/// window that the window manager and the app seem to fight over (one that jumps between two
/// places, flashes, or comes in two) can be followed in `quatrelle.log` frame by frame.
pub fn trace_window(ui: &Ui, what: &str) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*ON.get_or_init(|| std::env::var_os("QUATRELLE_TRACE_WINDOWS").is_some_and(|v| !v.is_empty())) {
        return;
    }
    let ctx = ui.ctx();
    let id = ctx.viewport_id();
    let v = ui.input(|i| i.viewport().clone());
    let rect = |r: Option<egui::Rect>| r.map(|r| format!("{:.0},{:.0} {:.0}x{:.0}", r.min.x, r.min.y, r.width(), r.height()));
    let say = format!(
        "outer {} inner {} monitor {:?} scale {:.2} maximized {:?} minimized {:?} focused {:?}",
        rect(v.outer_rect).unwrap_or_else(|| "-".into()),
        rect(v.inner_rect).unwrap_or_else(|| "-".into()),
        v.monitor_size.map(|m| format!("{:.0}x{:.0}", m.x, m.y)),
        v.native_pixels_per_point.unwrap_or(1.0),
        v.maximized,
        v.minimized,
        v.focused,
    );
    let key = egui::Id::unique(("trace-window", id));
    let last: Option<String> = ctx.data(|d| d.get_temp(key));
    if last.as_deref() != Some(say.as_str()) {
        log::info!("window {what} {id:?}: {say}");
        ctx.data_mut(|d| d.insert_temp(key, say));
    }
}
