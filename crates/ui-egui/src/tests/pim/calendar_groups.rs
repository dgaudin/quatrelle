// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: the calendar's side pane lists the server's calendar groups in its order, a group's
//! header folds and unfolds, and its tick shows or hides all the group's calendars at once.

use quatrelle_engine::model::pim::CalendarInfo;

use super::calendar_list_menu::{SIZE, calendars, click_text};
use super::*;

/// The sample's calendar with two of the server's groups, each with a colleague's calendar.
fn with_groups(name: &str) -> (MailApp, std::path::PathBuf, egui::Context) {
    let (mut a, dir, ctx) = calendars(name, true, false);
    let account = a.session.accounts[0].config.id.clone();
    {
        let pim = a.session.pim_mut(&account);
        pim.calendar_groups = vec!["Salles".into(), "Équipe".into()];
        pim.calendars.push(CalendarInfo { id: "AAMkAna=".into(), name: "Ana Lima".into(), group: "Équipe".into(), ..Default::default() });
        pim.calendars.push(CalendarInfo { id: "AAMkRoom=".into(), name: "Salle Lyon".into(), group: "Salles".into(), ..Default::default() });
    }
    for _ in 0..10 {
        frame_sized(&ctx, &mut a, vec![], SIZE);
    }
    (a, dir, ctx)
}

/// The position of the group header's tick, left of its name.
fn header_tick(shapes: &[egui::Shape], title: &str) -> egui::Pos2 {
    let at = text_at(shapes, title).unwrap_or_else(|| panic!("{title} in the side pane"));
    shapes
        .iter()
        .find_map(|s| match s {
            egui::Shape::Rect(r) if (12.0..=16.0).contains(&r.rect.width()) && (r.rect.center().y - at.y).abs() < 8.0 && r.rect.max.x < at.x => {
                Some(r.rect.center())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("the tick left of {title}"))
}

/// A click of the primary button at `at`, then the frames that settle.
fn click_at(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2) {
    let press = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame_sized(ctx, a, vec![Event::PointerMoved(at)], SIZE);
    frame_sized(ctx, a, vec![press(true)], SIZE);
    frame_sized(ctx, a, vec![press(false)], SIZE);
    for _ in 0..5 {
        frame_sized(ctx, a, vec![], SIZE);
    }
}

#[test]
fn the_pane_lists_the_servers_groups_in_its_order() {
    let (mut a, dir, ctx) = with_groups("calendar-groups-order");
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    let y = |t: &str| text_at(&shapes, t).unwrap_or_else(|| panic!("{t} in the pane")).y;
    assert!(y("Salles") < y("Équipe"), "the server's order, not the order the calendars came in");
    assert!(y("Salles") < y("Salle Lyon") && y("Salle Lyon") < y("Équipe") && y("Équipe") < y("Ana Lima"), "each group's calendars under it");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_group_header_folds_and_unfolds() {
    let (mut a, dir, ctx) = with_groups("calendar-groups-fold");
    click_text(&ctx, &mut a, "Équipe");
    assert!(a.session.prefs.folded_calendar_groups.contains("Équipe"), "remembered folded");
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    assert!(text_at(&shapes, "Ana Lima").is_none(), "folded: its calendar isn't drawn");
    click_text(&ctx, &mut a, "Équipe");
    assert!(!a.session.prefs.folded_calendar_groups.contains("Équipe"), "unfolded");
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    assert!(text_at(&shapes, "Ana Lima").is_some(), "unfolded: its calendar is drawn");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_group_header_tick_shows_or_hides_all_its_calendars() {
    let (mut a, dir, ctx) = with_groups("calendar-groups-tick");
    let account = a.session.accounts[0].config.id.clone();
    let tick = header_tick(&frame_shapes(&ctx, &mut a, vec![], SIZE), "Équipe");
    click_at(&ctx, &mut a, tick);
    assert!(!a.session.calendar_shown(&account, "AAMkAna="), "hidden all at once");
    assert!(!a.session.prefs.calendars_side_by_side, "hiding doesn't change the view");
    let tick = header_tick(&frame_shapes(&ctx, &mut a, vec![], SIZE), "Équipe");
    click_at(&ctx, &mut a, tick);
    assert!(a.session.calendar_shown(&account, "AAMkAna="), "shown all at once");
    assert!(a.session.prefs.calendars_side_by_side, "shown together side by side");
    let _ = std::fs::remove_dir_all(dir);
}
