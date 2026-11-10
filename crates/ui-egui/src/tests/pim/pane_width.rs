// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The folder pane in the calendar: a calendar's long name doesn't hold the pane wide.

use super::*;

/// A read-only calendar with a long name put "read-only" past the pane's edge, so the pane could
/// not be dragged narrower than that row: the pane's colour stopped where the pointer was, the
/// days stayed where they were, and nothing was drawn between the two (a black block).
#[test]
fn a_long_calendar_name_leaves_the_pane_its_width() {
    let (mut a, dir) = app("calendar-pane-width");
    let ctx = egui::Context::default();
    let mut root = egui::Id::NULL;
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| {
        root = ui.scope_id();
        a.ui(ui);
    });
    out.textures_delta.clear();
    let account = a.session.accounts[0].config.id.clone();
    let pim = a.session.pim.entry(account).or_default();
    pim.ensure_defaults();
    pim.calendars.push(quatrelle_engine::model::pim::CalendarInfo {
        id: "team".into(),
        name: "Studio › The calendar of every crew on call this winter".into(),
        read_only: true,
        ..Default::default()
    });
    a.run("view.calendar", json!({}));
    // Tall enough for the months and the calendars under them.
    let size = egui::vec2(1280.0, 1400.0);
    for _ in 0..30 {
        frame_sized(&ctx, &mut a, vec![], size);
    }
    let shapes = frame_shapes(&ctx, &mut a, vec![], size);
    let nav = root.with_salt(egui::IdSalt::new("nav"));
    let pane = egui::containers::panel::PanelState::load(&ctx, nav).map(|p| p.outer_rect).expect("the pane");
    // The days begin where the pane ends: the white of the days' side starts at its edge.
    let days = shapes
        .iter()
        .filter_map(|s| match s {
            egui::Shape::Rect(r) if r.rect.height() > 400.0 && r.fill.a() > 0 && r.rect.max.x > 1000.0 => Some(r.rect.min.x),
            _ => None,
        })
        .fold(f32::MAX, f32::min);
    assert!(pane.width() <= 273.0, "the pane is as wide as it is made (260 and its margins), not {}", pane.width());
    assert!(days <= pane.max.x + 2.0, "the days begin at {days}, the pane ends at {}", pane.max.x);
    // "read-only" is drawn whole, inside the pane.
    let tag = shapes.iter().find_map(|s| match s {
        egui::Shape::Text(t) if t.galley.text() == "read-only" => Some(t.pos.x + t.galley.rect.max.x),
        _ => None,
    });
    assert!(tag.is_some_and(|x| x <= pane.max.x), "read-only ends at {tag:?} in a pane that ends at {}", pane.max.x);
    let _ = std::fs::remove_dir_all(dir);
}
