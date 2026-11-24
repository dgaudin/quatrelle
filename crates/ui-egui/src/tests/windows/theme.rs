// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! UI tests: the title bars the system draws (the other windows', and the main window's when it
//! is chosen) are told Quatrelle's light or dark, again over a window's first frames.

use super::*;
use crate::WindowPlace;
use crate::app::title_bars_in;
use egui::{SystemTheme, ViewportCommand, ViewportId};

/// What one frame of the viewport `id` told the system about its title bar, `note` being called
/// as every window does each frame.
fn told_by_window(ctx: &egui::Context, id: ViewportId) -> Vec<SystemTheme> {
    let mut place = None;
    let viewports = std::iter::once((id, egui::ViewportInfo::default())).collect();
    let input = RawInput {
        viewport_id: id,
        viewports,
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))),
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| WindowPlace::note(&mut place, ui));
    out.textures_delta.clear();
    // egui's own sync says the system's theme over a viewport it has just taken up again, as
    // here, passes of other viewports between: not what's under test, and never said for a
    // chosen theme.
    themes(out.viewport_output.remove(&id).map(|v| v.commands).unwrap_or_default()).into_iter().filter(|t| *t != SystemTheme::SystemDefault).collect()
}

fn themes(commands: Vec<ViewportCommand>) -> Vec<SystemTheme> {
    commands
        .into_iter()
        .filter_map(|c| match c {
            ViewportCommand::SetTheme(t) => Some(t),
            _ => None,
        })
        .collect()
}

#[test]
fn a_window_is_told_the_theme_three_times_then_only_when_it_changes() {
    let ctx = egui::Context::default();
    let other = ViewportId::from_hash_of("a message window");
    // egui itself says its own default over a viewport's first frame: let it pass.
    for id in [ViewportId::ROOT, other] {
        (0..3).for_each(|_| _ = told_by_window(&ctx, id));
    }
    title_bars_in(&ctx, Some(true));
    for id in [ViewportId::ROOT, other] {
        let told: Vec<_> = (0..6).flat_map(|_| told_by_window(&ctx, id)).collect();
        assert_eq!(told, vec![SystemTheme::Dark; 3], "{id:?}");
    }
    // Changed: every window open is told again.
    title_bars_in(&ctx, Some(false));
    for id in [ViewportId::ROOT, other] {
        let told: Vec<_> = (0..6).flat_map(|_| told_by_window(&ctx, id)).collect();
        assert_eq!(told, vec![SystemTheme::Light; 3], "{id:?}");
    }
}

#[test]
fn nothing_is_told_before_the_theme_is_known() {
    let ctx = egui::Context::default();
    told_by_window(&ctx, ViewportId::ROOT);
    assert!(told_by_window(&ctx, ViewportId::ROOT).is_empty());
}

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn the_main_window_says_the_resolved_theme_on_linux_never_the_systems() {
    let (mut a, dir) = app("title-theme");
    let ctx = egui::Context::default();
    let told_in_frames = |a: &mut MailApp| -> Vec<SystemTheme> {
        let mut all = Vec::new();
        for _ in 0..6 {
            let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| a.ui(ui));
            out.textures_delta.clear();
            all.extend(themes(out.viewport_output.remove(&ViewportId::ROOT).map(|v| v.commands).unwrap_or_default()));
        }
        all
    };
    told_in_frames(&mut a);
    a.ui.dark = Some(true);
    let told = told_in_frames(&mut a);
    assert!(told.len() >= 3 && told.iter().all(|t| *t == SystemTheme::Dark), "{told:?}");
    a.ui.dark = Some(false);
    let told = told_in_frames(&mut a);
    assert!(told.len() >= 3 && told.iter().all(|t| *t == SystemTheme::Light), "{told:?}");
    // Following the system: the theme it resolves to (light here, no system theme known).
    a.ui.dark = Some(true);
    told_in_frames(&mut a);
    a.ui.dark = None;
    let told = told_in_frames(&mut a);
    assert!(told.len() >= 3 && told.iter().all(|t| *t == SystemTheme::Light), "{told:?}");
    let _ = std::fs::remove_dir_all(dir);
}
