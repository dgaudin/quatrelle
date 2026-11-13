// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: a calendar's right-click menu in the side pane, in light and dark — each entry with
//! its icon, the colour submenu, the dialogs it opens — and New, Rename and Delete greyed, with
//! why, where the account's server doesn't allow them.

use quatrelle_engine::model::pim::CalendarInfo;

use super::calendar_menu::texts;
use super::*;
use crate::dialogs::CalendarAsk;

/// Tall enough for the months and the calendars under them.
pub(super) const SIZE: egui::Vec2 = egui::vec2(1280.0, 1300.0);

/// The sample in the calendar, its account a Microsoft 365 one (`graph`) or one with a CalDAV
/// server, with a main calendar and Trips (and Work, the CalDAV one).
pub(super) fn calendars(name: &str, graph: bool, dark: bool) -> (MailApp, std::path::PathBuf, egui::Context) {
    let (mut a, dir) = app(name);
    a.ui.dark = Some(dark);
    let ctx = egui::Context::default();
    frame_sized(&ctx, &mut a, vec![], SIZE);
    if graph {
        a.session.accounts[0].config.graph = Some("https://graph.microsoft.com/v1.0".into());
    } else {
        a.session.accounts[0].config.imap.host = "imap.contoso.example".into();
    }
    let account = a.session.accounts[0].config.id.clone();
    let pim = a.session.pim_mut(&account);
    pim.calendars.push(CalendarInfo { id: "AAMkMain=".into(), name: "Main".into(), main: graph, ..Default::default() });
    pim.calendars.push(CalendarInfo { id: "AAMkTrips=".into(), name: "Trips".into(), color: 2, ..Default::default() });
    if !graph {
        pim.calendars.push(CalendarInfo { id: "https://dav.contoso.example/cal/work/".into(), name: "Work".into(), color: 3, ..Default::default() });
    }
    a.run("view.module", json!({"name": "calendar"}));
    for _ in 0..30 {
        frame_sized(&ctx, &mut a, vec![], SIZE);
    }
    (a, dir, ctx)
}

/// A right click on the calendar named `name` in the side pane: the texts the frame draws then
/// that it didn't before (the menu's entries).
pub(super) fn right_click(ctx: &egui::Context, a: &mut MailApp, name: &str) -> Vec<String> {
    let at = settled_text_at(ctx, a, name, SIZE).unwrap_or_else(|| panic!("{name} isn't in the side pane"));
    frame_sized(ctx, a, vec![Event::PointerMoved(at)], SIZE);
    let mut before = texts(&frame_shapes(ctx, a, vec![], SIZE));
    let button = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE };
    frame_sized(ctx, a, vec![button(true)], SIZE);
    frame_sized(ctx, a, vec![button(false)], SIZE);
    let mut after = Vec::new();
    for _ in 0..5 {
        after = texts(&frame_shapes(ctx, a, vec![], SIZE));
    }
    after.retain(|t| match before.iter().position(|b| b == t) {
        Some(i) => {
            before.swap_remove(i);
            false
        }
        None => true,
    });
    after
}

/// The pointer on `text` (a menu entry), held still a moment: a submenu opens, a tip shows.
pub(super) fn point_at(ctx: &egui::Context, a: &mut MailApp, text: &str) {
    let at = settled_text_at(ctx, a, text, SIZE).unwrap_or_else(|| panic!("no {text} in the menu"));
    frame_sized(ctx, a, vec![Event::PointerMoved(at)], SIZE);
    for _ in 0..60 {
        frame_sized(ctx, a, vec![], SIZE);
    }
}

pub(super) fn click_text(ctx: &egui::Context, a: &mut MailApp, text: &str) {
    let at = settled_text_at(ctx, a, text, SIZE).unwrap_or_else(|| panic!("no {text} drawn"));
    let press = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame_sized(ctx, a, vec![Event::PointerMoved(at)], SIZE);
    frame_sized(ctx, a, vec![press(true)], SIZE);
    frame_sized(ctx, a, vec![press(false)], SIZE);
    for _ in 0..5 {
        frame_sized(ctx, a, vec![], SIZE);
    }
}

const ENTRIES: [&str; 10] = [
    "Hide this calendar",
    "Show only this calendar",
    "Overlay",
    "Color",
    "New calendar…",
    "Rename…",
    "Remove from the list",
    "Delete calendar…",
    "Move up",
    "Move down",
];

/// Every entry, an icon before each, in light and dark.
#[test]
fn a_calendars_menu_has_each_entry_with_its_icon() {
    for dark in [false, true] {
        let (mut a, dir, ctx) = calendars(&format!("calendar-list-menu-{dark}"), true, dark);
        let menu = right_click(&ctx, &mut a, "Trips");
        for entry in ENTRIES {
            assert!(menu.iter().any(|t| t == entry), "no {entry} in {menu:?} (dark: {dark})");
        }
        assert!(!menu.iter().any(|t| t == "Refresh" || t == "Remove this calendar"), "a calendar of the account's own: {menu:?}");
        let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
        for label in ENTRIES {
            let at = text_at(&shapes, label).unwrap_or_else(|| panic!("{label} in the menu"));
            let icon = shapes.iter().any(|s| match s {
                egui::Shape::Mesh(m) => {
                    let r = m.calc_bounds();
                    r.max.x <= at.x && r.max.x > at.x - 120.0 && (r.center().y - at.y).abs() < 6.0
                }
                _ => false,
            });
            assert!(icon, "an icon before {label} (dark: {dark})");
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Color opens Automatic and the palette's 16; one clicked is the calendar's, kept on this
/// computer, and Automatic lets it go.
#[test]
fn the_colour_submenu_offers_automatic_and_the_palette() {
    let (mut a, dir, ctx) = calendars("calendar-list-colour", true, false);
    right_click(&ctx, &mut a, "Trips");
    point_at(&ctx, &mut a, "Color");
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    assert!(text_at(&shapes, "Automatic").is_some(), "Automatic in the submenu");
    let swatch = |shapes: &[egui::Shape], i: u8| {
        shapes.iter().find_map(|s| match s {
            egui::Shape::Circle(c) if c.radius == 8.0 && c.fill == crate::theme::category_color(i, false) => Some(c.center),
            _ => None,
        })
    };
    for i in 0..quatrelle_engine::PALETTE_COLORS {
        assert!(swatch(&shapes, i).is_some(), "colour {i} in the submenu");
    }
    let at = swatch(&shapes, 9).unwrap();
    let press = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame_sized(&ctx, &mut a, vec![Event::PointerMoved(at)], SIZE);
    frame_sized(&ctx, &mut a, vec![press(true)], SIZE);
    frame_sized(&ctx, &mut a, vec![press(false)], SIZE);
    frame_sized(&ctx, &mut a, vec![], SIZE);
    let key = quatrelle_engine::Session::calendar_key(&a.session.accounts[0].config.id, "AAMkTrips=");
    assert_eq!(a.session.prefs.calendar_colors.get(&key), Some(&9));
    // Its box in the side pane takes it.
    for _ in 0..5 {
        frame_sized(&ctx, &mut a, vec![], SIZE);
    }
    let chosen = crate::theme::category_color(9, false);
    let boxed =
        frame_shapes(&ctx, &mut a, vec![], SIZE).iter().any(|s| matches!(s, egui::Shape::Rect(r) if r.fill == chosen && r.rect.width() < 20.0));
    assert!(boxed, "the calendar's box in its new colour");
    right_click(&ctx, &mut a, "Trips");
    point_at(&ctx, &mut a, "Color");
    click_text(&ctx, &mut a, "Automatic");
    assert!(a.session.prefs.calendar_colors.is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

/// Rename… and Delete calendar… open their dialogs; the name given renames it, Delete deletes it.
#[test]
fn rename_and_delete_ask_then_change_the_calendar() {
    let (mut a, dir, ctx) = calendars("calendar-list-dialogs", true, true);
    right_click(&ctx, &mut a, "Trips");
    click_text(&ctx, &mut a, "Rename…");
    match a.ui.dialog.as_mut() {
        Some(Dialog::CalendarChange(f)) => {
            assert_eq!((&f.ask, f.name.as_str()), (&CalendarAsk::Rename { calendar: "AAMkTrips=".into() }, "Trips"));
            f.name = "Travel".into();
        }
        other => panic!("{other:?}"),
    }
    click_text(&ctx, &mut a, "Rename");
    assert!(a.ui.dialog.is_none(), "closed once renamed");
    let account = a.session.accounts[0].config.id.clone();
    let name = |a: &MailApp| a.session.ordered_calendars(&account).iter().find(|c| c.id == "AAMkTrips=").map(|c| c.name.clone());
    assert_eq!(name(&a).as_deref(), Some("Travel"));

    right_click(&ctx, &mut a, "Travel");
    click_text(&ctx, &mut a, "Delete calendar…");
    assert!(matches!(a.ui.dialog.as_ref(), Some(Dialog::CalendarChange(f)) if f.deleting()), "the question");
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    assert!(texts(&shapes).iter().any(|t| t.contains("Travel") && t.contains("Deleted Items")), "{:?}", texts(&shapes));
    click_text(&ctx, &mut a, "Delete");
    assert!(a.ui.dialog.is_none());
    assert_eq!(name(&a), None, "deleted");

    // New calendar… asks for a name, saying which account it's made in.
    right_click(&ctx, &mut a, "Main");
    click_text(&ctx, &mut a, "New calendar…");
    assert!(matches!(a.ui.dialog.as_ref(), Some(Dialog::CalendarChange(f)) if f.ask == CalendarAsk::New));
    let email = a.session.accounts[0].config.email.clone();
    let shown = texts(&frame_shapes(&ctx, &mut a, vec![], SIZE));
    assert!(shown.iter().any(|t| *t == format!("Made in {email}")), "{shown:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// On a CalDAV account New, Rename and Delete are greyed, say why when pointed at, and do
/// nothing clicked; the rest of the menu works.
#[test]
fn a_caldav_calendar_greys_new_rename_and_delete_and_says_why() {
    for dark in [false, true] {
        let (mut a, dir, ctx) = calendars(&format!("calendar-list-caldav-{dark}"), false, dark);
        let menu = right_click(&ctx, &mut a, "Work");
        for entry in ENTRIES {
            assert!(menu.iter().any(|t| t == entry), "no {entry} in {menu:?}");
        }
        point_at(&ctx, &mut a, "New calendar…");
        let shown = texts(&frame_shapes(&ctx, &mut a, vec![], SIZE));
        assert!(shown.iter().any(|t| t == "This account's server doesn't allow it"), "why, when pointed at: {shown:?}");
        click_text(&ctx, &mut a, "Rename…");
        assert!(a.ui.dialog.is_none(), "greyed: nothing opens");
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// A colour kept for a calendar that isn't one of the menu's (a settings file from elsewhere): its
/// box is drawn as an untouched calendar's, as its events are.
#[test]
fn a_box_with_a_colour_that_isnt_one_is_drawn_as_the_servers() {
    let (mut a, dir, ctx) = calendars("calendar-list-junk-colour", true, false);
    let key = quatrelle_engine::Session::calendar_key(&a.session.accounts[0].config.id, "AAMkMain=");
    a.session.prefs.calendar_colors.insert(key, 200);
    for _ in 0..5 {
        frame_sized(&ctx, &mut a, vec![], SIZE);
    }
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    let at = text_at(&shapes, "Main").expect("Main in the side pane");
    let fill = shapes.iter().find_map(|s| match s {
        egui::Shape::Rect(r) if r.rect.width() < 20.0 && r.rect.max.x < at.x && (r.rect.center().y - at.y).abs() < 8.0 && r.fill.a() > 0 => {
            Some(r.fill)
        }
        _ => None,
    });
    assert_eq!(fill, Some(crate::theme::Tokens::light().accent), "the server's colour (0: the app's)");
    let _ = std::fs::remove_dir_all(dir);
}
