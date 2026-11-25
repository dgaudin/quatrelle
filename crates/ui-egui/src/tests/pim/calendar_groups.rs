// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: the calendar's side pane lists the server's calendar groups in its order, a group's
//! header folds and unfolds, and its tick shows or hides all the group's calendars at once.

use quatrelle_engine::model::pim::CalendarInfo;

use super::calendar_list_menu::{SIZE, calendars, click_text, point_at, right_click};
use super::calendar_menu::texts;
use super::*;
use crate::dialogs::GroupAsk;

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
    assert!(!a.session.prefs.folded_calendar_groups.is_empty(), "remembered folded");
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    assert!(text_at(&shapes, "Ana Lima").is_none(), "folded: its calendar isn't drawn");
    click_text(&ctx, &mut a, "Équipe");
    assert!(a.session.prefs.folded_calendar_groups.is_empty(), "unfolded");
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

/// The sample's groups on an Exchange account: the same calendars, the server groups kept there.
fn with_ews_groups(name: &str) -> (MailApp, std::path::PathBuf, egui::Context) {
    let (mut a, dir, ctx) = with_groups(name);
    a.session.accounts[0].config.ews = Some("https://ews.contoso.example/EWS/Exchange.asmx".into());
    for _ in 0..5 {
        frame_sized(&ctx, &mut a, vec![], SIZE);
    }
    (a, dir, ctx)
}

#[test]
fn a_group_header_menu_renames_and_deletes_the_group() {
    let (mut a, dir, ctx) = with_groups("calendar-groups-group-menu");
    let menu = right_click(&ctx, &mut a, "Équipe");
    for entry in ["New calendar group…", "Rename group…", "Delete group…", "Move up", "Move down"] {
        assert!(menu.iter().any(|t| t == entry), "no {entry} in {menu:?}");
    }
    click_text(&ctx, &mut a, "Rename group…");
    match a.ui.dialog.as_mut() {
        Some(Dialog::CalendarGroupChange(f)) => {
            assert!(matches!(f.ask, GroupAsk::Rename { .. }));
            f.name = "Team".into();
        }
        other => panic!("{other:?}"),
    }
    click_text(&ctx, &mut a, "Rename");
    assert!(a.session.pim.values().any(|p| p.calendar_groups.iter().any(|g| g == "Team")));
    assert!(a.session.pim.values().any(|p| p.calendars.iter().any(|c| c.name == "Ana Lima" && c.group == "Team")));
    // Delete asks first, then takes the group out of the pane.
    right_click(&ctx, &mut a, "Team");
    click_text(&ctx, &mut a, "Delete group…");
    assert!(matches!(&a.ui.dialog, Some(Dialog::CalendarGroupChange(f)) if f.deleting()));
    click_text(&ctx, &mut a, "Delete");
    assert!(!a.session.pim.values().any(|p| p.calendar_groups.iter().any(|g| g == "Team")));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_group_moves_its_calendar_to_another_group_on_exchange() {
    let (mut a, dir, ctx) = with_ews_groups("calendar-groups-move-calendar");
    right_click(&ctx, &mut a, "Ana Lima");
    point_at(&ctx, &mut a, "Move to group");
    click_text(&ctx, &mut a, "Salles");
    assert!(
        a.session.pim.values().any(|p| p.calendars.iter().any(|c| c.name == "Ana Lima" && c.group == "Salles")),
        "Ana's calendar moved to Salles"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_group_moves_up_and_down_on_exchange_but_not_on_microsoft_365() {
    let (mut a, dir, ctx) = with_ews_groups("calendar-groups-order-ews");
    right_click(&ctx, &mut a, "Salles");
    click_text(&ctx, &mut a, "Move down");
    let order = a.session.pim.values().next().map(|p| p.calendar_groups.clone()).unwrap_or_default();
    assert_eq!(order, ["Équipe", "Salles"]);
    let _ = std::fs::remove_dir_all(dir);

    let (mut a, dir, ctx) = with_groups("calendar-groups-order-graph");
    right_click(&ctx, &mut a, "Salles");
    point_at(&ctx, &mut a, "Move down");
    let shown = texts(&frame_shapes(&ctx, &mut a, vec![], SIZE));
    assert!(shown.iter().any(|t| t == "Microsoft 365 keeps no order for calendar groups"), "{shown:?}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn dragging_a_calendar_onto_a_groups_header_moves_it() {
    let (mut a, dir, ctx) = with_ews_groups("calendar-groups-drag");
    let from = settled_text_at(&ctx, &mut a, "Ana Lima", SIZE).expect("Ana Lima in the pane");
    let to = settled_text_at(&ctx, &mut a, "Salles", SIZE).expect("Salles in the pane");
    let press = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame_sized(&ctx, &mut a, vec![Event::PointerMoved(from)], SIZE);
    frame_sized(&ctx, &mut a, vec![press(from, true)], SIZE);
    frame_sized(&ctx, &mut a, vec![Event::PointerMoved(from + egui::vec2(12.0, 0.0))], SIZE);
    frame_sized(&ctx, &mut a, vec![Event::PointerMoved(to)], SIZE);
    frame_sized(&ctx, &mut a, vec![press(to, false)], SIZE);
    for _ in 0..6 {
        frame_sized(&ctx, &mut a, vec![], SIZE);
    }
    assert!(
        a.session.pim.values().any(|p| p.calendars.iter().any(|c| c.name == "Ana Lima" && c.group == "Salles")),
        "Ana's calendar moved onto Salles"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A colleague's calendar that Exchange gives the id `shared:<address>` is under the server
/// group it belongs to, not under Shared calendars.
#[test]
fn a_colleagues_calendar_stays_under_its_server_group() {
    let (mut a, dir, ctx) = with_groups("calendar-groups-shared");
    let account = a.session.accounts[0].config.id.clone();
    {
        let pim = a.session.pim_mut(&account);
        let ana = pim.calendars.iter_mut().find(|c| c.name == "Ana Lima").expect("Ana Lima");
        ana.id = "shared:ana@contoso.example".into();
    }
    for _ in 0..10 {
        frame_sized(&ctx, &mut a, vec![], SIZE);
    }
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    assert!(text_at(&shapes, "Shared calendars").is_none(), "the empty Shared calendars heading is gone");
    let y = |t: &str| text_at(&shapes, t).unwrap_or_else(|| panic!("{t} in the pane")).y;
    assert!(y("Équipe") < y("Ana Lima"), "Ana's calendar is under Équipe");
    let _ = std::fs::remove_dir_all(dir);
}

/// A calendar dragged onto a row of another group moves to that row's group.
#[test]
fn dragging_a_calendar_onto_a_groups_row_moves_it() {
    let (mut a, dir, ctx) = with_ews_groups("calendar-groups-drag-row");
    let from = settled_text_at(&ctx, &mut a, "Ana Lima", SIZE).expect("Ana Lima in the pane");
    let to = settled_text_at(&ctx, &mut a, "Salle Lyon", SIZE).expect("Salle Lyon in the pane");
    let press = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame_sized(&ctx, &mut a, vec![Event::PointerMoved(from)], SIZE);
    frame_sized(&ctx, &mut a, vec![press(from, true)], SIZE);
    frame_sized(&ctx, &mut a, vec![Event::PointerMoved(from + egui::vec2(12.0, 0.0))], SIZE);
    frame_sized(&ctx, &mut a, vec![Event::PointerMoved(to)], SIZE);
    frame_sized(&ctx, &mut a, vec![press(to, false)], SIZE);
    for _ in 0..6 {
        frame_sized(&ctx, &mut a, vec![], SIZE);
    }
    assert!(
        a.session.pim.values().any(|p| p.calendars.iter().any(|c| c.name == "Ana Lima" && c.group == "Salles")),
        "Ana's calendar moved onto Salle Lyon's group"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// Escape cancels the drag: the calendar stays where it was.
#[test]
fn escape_cancels_a_calendar_drag() {
    let (mut a, dir, ctx) = with_ews_groups("calendar-groups-drag-escape");
    let from = settled_text_at(&ctx, &mut a, "Ana Lima", SIZE).expect("Ana Lima in the pane");
    let to = settled_text_at(&ctx, &mut a, "Salles", SIZE).expect("Salles in the pane");
    let press = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame_sized(&ctx, &mut a, vec![Event::PointerMoved(from)], SIZE);
    frame_sized(&ctx, &mut a, vec![press(from, true)], SIZE);
    frame_sized(&ctx, &mut a, vec![Event::PointerMoved(from + egui::vec2(12.0, 0.0))], SIZE);
    frame_sized(&ctx, &mut a, vec![Event::PointerMoved(to)], SIZE);
    frame_sized(&ctx, &mut a, vec![key(egui::Key::Escape, Modifiers::NONE)], SIZE);
    frame_sized(&ctx, &mut a, vec![press(to, false)], SIZE);
    for _ in 0..6 {
        frame_sized(&ctx, &mut a, vec![], SIZE);
    }
    assert!(
        a.session.pim.values().any(|p| p.calendars.iter().any(|c| c.name == "Ana Lima" && c.group == "Équipe")),
        "Escape left Ana's calendar in Équipe"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A calendar opened by its owner's address has no shortcut in Outlook's groups: Move to group
/// is greyed with why, instead of offering a move that would fail.
#[test]
fn move_to_group_is_greyed_for_a_calendar_opened_by_address() {
    let (mut a, dir, ctx) = with_groups("calendar-groups-opened");
    let account = a.session.accounts[0].config.id.clone();
    {
        let pim = a.session.pim_mut(&account);
        pim.shared.push("ana@contoso.example".into());
        let ana = pim.calendars.iter_mut().find(|c| c.name == "Ana Lima").expect("Ana Lima");
        ana.id = "shared:ana@contoso.example".into();
    }
    for _ in 0..10 {
        frame_sized(&ctx, &mut a, vec![], SIZE);
    }
    right_click(&ctx, &mut a, "Ana Lima");
    point_at(&ctx, &mut a, "Move to group");
    let shown = texts(&frame_shapes(&ctx, &mut a, vec![], SIZE));
    assert!(shown.iter().any(|t| t == "This calendar isn't in Outlook's calendar groups yet"), "{shown:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Deleting a group puts its own calendars back under My Calendars, not out of the list.
#[test]
fn deleting_a_group_puts_its_own_calendars_back_under_my_calendars() {
    let (mut a, dir, ctx) = with_groups("calendar-groups-delete-own");
    right_click(&ctx, &mut a, "Salles");
    click_text(&ctx, &mut a, "Delete group…");
    click_text(&ctx, &mut a, "Delete");
    assert!(
        a.session.pim.values().any(|p| p.calendars.iter().any(|c| c.name == "Salle Lyon" && c.group.is_empty())),
        "Salle Lyon is back under My Calendars"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// An account whose server keeps no groups says so where a group would be made.
#[test]
fn a_server_without_calendar_groups_says_so() {
    let (mut a, dir, ctx) = calendars("calendar-groups-imap", false, false);
    let account = a.session.accounts[0].config.id.clone();
    {
        let pim = a.session.pim_mut(&account);
        pim.calendar_groups = vec!["Équipe".into()];
        pim.calendars.push(CalendarInfo { id: "AAMkAna=".into(), name: "Ana Lima".into(), group: "Équipe".into(), ..Default::default() });
    }
    for _ in 0..10 {
        frame_sized(&ctx, &mut a, vec![], SIZE);
    }
    right_click(&ctx, &mut a, "Équipe");
    point_at(&ctx, &mut a, "New calendar group…");
    let shown = texts(&frame_shapes(&ctx, &mut a, vec![], SIZE));
    assert!(shown.iter().any(|t| t == "This account's server doesn't keep calendar groups"), "{shown:?}");
    let _ = std::fs::remove_dir_all(dir);
}
