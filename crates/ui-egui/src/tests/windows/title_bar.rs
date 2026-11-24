// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: the rows at the top of the window, measured: the top bar at the very top, the ribbon
//! just under it (its tabs, then its buttons), in each module and on the File page, at the
//! smallest, a usual and a very wide window. The heights are those of the window as it is, within
//! two points (the fonts differ a little between systems): what changes them has to change them
//! here too.

use super::*;

const SIZES: [f32; 3] = [760.0, 1320.0, 2560.0];
const MODULES: [&str; 5] = ["mail", "calendar", "people", "tasks", "notes"];

/// The rows after a few frames (the ribbon fits itself to the window over the first ones):
/// the top bar's rectangle, the ribbon panel's, and its row of tabs'.
fn rows(a: &mut MailApp, ctx: &egui::Context, width: f32) -> (egui::Rect, Option<egui::Rect>, Option<egui::Rect>) {
    for _ in 0..6 {
        frame_sized(ctx, a, vec![], egui::vec2(width, 820.0));
    }
    (a.ui.top_bar_rect.expect("the top bar"), a.ui.ribbon_panel_rect, a.ui.ribbon_tabs_rect)
}

/// The top bar's height, the ribbon panel's (its margins and border, the row of tabs, the row of
/// buttons) and the row of tabs' (its top, under the panel's border and margin), in points, as
/// measured: the same in every module and at every width.
const BAR_H: f32 = 40.0;
const RIBBON_PANEL_H: f32 = 129.0;
const TABS_H: f32 = 26.0;
const TABS_TOP_IN_PANEL: f32 = 5.0;
/// How far a measure may be from the one written here: the fonts, and so the rows' text, can
/// differ a little from one system to another (the CI's Windows and macOS).
const SLACK: f32 = 2.0;

/// Whether each measure is within [`SLACK`] of the one wanted.
fn near<const N: usize>(got: [f32; N], want: [f32; N]) -> bool {
    got.iter().zip(want).all(|(g, w)| (g - w).abs() <= SLACK)
}

#[test]
fn the_rows_at_the_top_of_the_window_have_their_measured_heights() {
    let (mut a, dir) = app("title-bar-rows");
    let ctx = egui::Context::default();
    for module in MODULES {
        a.run("view.module", json!({"name": module}));
        for width in SIZES {
            let at = format!("{module} at {width} pt");
            let (bar, panel, tabs) = rows(&mut a, &ctx, width);
            // The bar is the top row: from the top edge, across the window.
            assert!(near([bar.min.x, bar.min.y, bar.max.x, bar.height()], [0.0, 0.0, width, BAR_H]), "the top bar, {at}: {bar:?}");
            // The ribbon just under it, to the window's right edge.
            let panel = panel.unwrap_or_else(|| panic!("the ribbon, {at}"));
            assert!(near([panel.min.y, panel.max.x, panel.height()], [BAR_H, width, RIBBON_PANEL_H]), "the ribbon panel, {at}: {panel:?}");
            let tabs = tabs.unwrap_or_else(|| panic!("the tabs, {at}"));
            assert!(near([tabs.min.y, tabs.height()], [BAR_H + TABS_TOP_IN_PANEL, TABS_H]), "the tabs, {at}: {tabs:?}");
        }
    }
    // The File page keeps the bar, and has no ribbon under it.
    a.run("view.module", json!({"name": "mail"}));
    a.act(crate::Action::Backstage(crate::backstage::Page::Info));
    for width in SIZES {
        let (bar, panel, tabs) = rows(&mut a, &ctx, width);
        assert!(near([bar.min.x, bar.min.y, bar.max.x, bar.height()], [0.0, 0.0, width, BAR_H]), "the top bar on the File page, {width} pt: {bar:?}");
        assert!(panel.is_none() && tabs.is_none(), "no ribbon on the File page, {width} pt");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// The labels of every node the Options page offers, after a few frames.
fn options_labels(a: &mut MailApp) -> Vec<String> {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
    let mut labels = Vec::new();
    for _ in 0..4 {
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 2400.0))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| a.ui(ui));
        out.textures_delta.clear();
        let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
        labels = nodes.iter().filter_map(|(_, n)| n.label().or_else(|| n.value()).map(str::to_string)).collect();
    }
    labels
}

#[test]
fn the_options_offer_the_system_title_bar_where_the_window_has_its_own() {
    let (mut a, dir) = app("title-bar-option");
    let labels = options_labels(&mut a);
    let offered = cfg!(any(windows, all(unix, not(target_os = "macos"))));
    assert_eq!(labels.iter().any(|l| l == "Use the system title bar"), offered, "the switch: {labels:?}");
    assert_eq!(labels.iter().any(|l| l == "Applies when Quatrelle starts again"), offered, "its note");
    a.run("view.systemTitleBar", json!({"on": true}));
    assert!(a.session.prefs.system_title_bar);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_window_has_no_own_title_bar_by_default() {
    let mut s = crate::NullServices(0);
    assert!(!crate::Services::own_title_bar(&s));
    crate::Services::keep_frame(&mut s);
    crate::Services::before_zoom(&mut s);
}
