// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests, in light and dark: a calendar's first menu entry says what a click does (Hide this
//! calendar while it's shown, Show this calendar while it's hidden); Remove from the list takes
//! it out of the side pane and of the calendars an appointment is put in; the account's menu (on
//! its heading, or on My calendars when it's the only one) makes a calendar and brings removed
//! ones back, Removed calendars greyed while there's none.

use super::calendar_list_menu::{SIZE, calendars, click_text, point_at, right_click};
use super::calendar_menu::texts;
use super::*;
use crate::dialogs::CalendarAsk;

const TRIPS: &str = "AAMkTrips=";

/// How opaque `text` is drawn (its first glyph's alpha): a greyed entry's is fainter.
fn alpha(shapes: &[egui::Shape], text: &str) -> Option<u8> {
    shapes.iter().rev().find_map(|s| match s {
        egui::Shape::Text(t) if t.galley.text() == text => t.galley.rows.first()?.row.visuals.mesh.vertices.first().map(|v| v.color.a()),
        _ => None,
    })
}

/// An icon drawn just before `label`.
fn icon_before(shapes: &[egui::Shape], label: &str) -> bool {
    let Some(at) = text_at(shapes, label) else { return false };
    shapes.iter().any(|s| match s {
        egui::Shape::Mesh(m) => {
            let r = m.calc_bounds();
            r.max.x <= at.x && r.max.x > at.x - 120.0 && (r.center().y - at.y).abs() < 6.0
        }
        _ => false,
    })
}

fn has(menu: &[String], entry: &str) -> bool {
    menu.iter().any(|t| t == entry)
}

#[test]
fn the_first_entry_says_what_a_click_does() {
    for dark in [false, true] {
        let (mut a, dir, ctx) = calendars(&format!("calendar-hide-show-{dark}"), true, dark);
        let account = a.session.accounts[0].config.id.clone();
        let menu = right_click(&ctx, &mut a, "Trips");
        assert!(has(&menu, "Hide this calendar") && !has(&menu, "Show this calendar"), "shown: {menu:?}");
        click_text(&ctx, &mut a, "Hide this calendar");
        assert!(!a.session.calendar_shown(&account, TRIPS), "hidden");
        let menu = right_click(&ctx, &mut a, "Trips");
        assert!(has(&menu, "Show this calendar") && !has(&menu, "Hide this calendar"), "hidden: {menu:?}");
        click_text(&ctx, &mut a, "Show this calendar");
        assert!(a.session.calendar_shown(&account, TRIPS), "shown again");
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// The main calendar's Remove from the list is greyed, and says why when pointed at.
#[test]
fn the_main_calendar_isnt_removed() {
    let (mut a, dir, ctx) = calendars("calendar-removed-main", true, false);
    let menu = right_click(&ctx, &mut a, "Main");
    assert!(has(&menu, "Remove from the list"), "{menu:?}");
    point_at(&ctx, &mut a, "Remove from the list");
    let shown = texts(&frame_shapes(&ctx, &mut a, vec![], SIZE));
    assert!(has(&shown, "The account's main calendar stays as it is"), "why, when pointed at: {shown:?}");
    click_text(&ctx, &mut a, "Remove from the list");
    assert!(a.session.prefs.removed_calendars.is_empty(), "greyed: nothing removed");
    let _ = std::fs::remove_dir_all(dir);
}

/// With one account, My calendars has its menu: New calendar… opens its dialog; Removed
/// calendars is greyed while there's none, then lists the one removed, whose click brings it back
/// into the side pane and among the calendars an appointment is put in.
#[test]
fn removed_from_the_list_then_brought_back_from_my_calendars() {
    for dark in [false, true] {
        let (mut a, dir, ctx) = calendars(&format!("calendar-removed-{dark}"), true, dark);
        let account = a.session.accounts[0].config.id.clone();
        let offered = |a: &MailApp| crate::pim_dialogs::EventForm::new(&a.session, None).calendars.iter().any(|c| c.1 == TRIPS);
        assert!(offered(&a), "Trips offered for a new appointment");

        let menu = right_click(&ctx, &mut a, "My calendars");
        assert!(has(&menu, "New calendar…") && has(&menu, "Removed calendars"), "{menu:?}");
        let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
        for label in ["New calendar…", "Removed calendars"] {
            assert!(icon_before(&shapes, label), "an icon before {label} (dark: {dark})");
        }
        let (new, removed) = (alpha(&shapes, "New calendar…"), alpha(&shapes, "Removed calendars"));
        assert!(removed < new, "Removed calendars greyed with none: {removed:?} / {new:?}");
        click_text(&ctx, &mut a, "New calendar…");
        assert!(matches!(a.ui.dialog.as_ref(), Some(Dialog::CalendarChange(f)) if f.ask == CalendarAsk::New && f.account == account));
        a.ui.dialog = None;

        right_click(&ctx, &mut a, "Trips");
        click_text(&ctx, &mut a, "Remove from the list");
        assert!(a.session.calendar_removed(&account, TRIPS));
        for _ in 0..5 {
            frame_sized(&ctx, &mut a, vec![], SIZE);
        }
        assert!(!has(&texts(&frame_shapes(&ctx, &mut a, vec![], SIZE)), "Trips"), "out of the side pane");
        assert!(!offered(&a), "not offered for a new appointment");

        right_click(&ctx, &mut a, "My calendars");
        let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
        assert_eq!(alpha(&shapes, "Removed calendars"), alpha(&shapes, "New calendar…"), "no longer greyed");
        point_at(&ctx, &mut a, "Removed calendars");
        let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
        assert!(icon_before(&shapes, "Trips"), "the removed one listed by its name, with an icon: {:?}", texts(&shapes));
        click_text(&ctx, &mut a, "Trips");
        assert!(!a.session.calendar_removed(&account, TRIPS), "brought back");
        assert!(offered(&a));
        for _ in 0..5 {
            frame_sized(&ctx, &mut a, vec![], SIZE);
        }
        assert!(has(&texts(&frame_shapes(&ctx, &mut a, vec![], SIZE)), "Trips"), "in the side pane again");
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// With two accounts each has its heading, and the heading its menu: the removed calendar listed
/// there, brought back from it.
#[test]
fn an_accounts_heading_has_its_menu() {
    for dark in [false, true] {
        let (mut a, dir, ctx) = calendars(&format!("calendar-removed-heading-{dark}"), true, dark);
        let account = a.session.accounts[0].config.id.clone();
        let email = a.session.accounts[0].config.email.clone();
        let mut two = a.session.accounts[0].clone();
        two.config.id = "two".into();
        two.config.email = "two@contoso.example".into();
        a.session.accounts.push(two);
        a.run("calendar.remove", json!({"account": account, "calendar": TRIPS}));
        assert!(a.session.calendar_removed(&account, TRIPS));
        for _ in 0..5 {
            frame_sized(&ctx, &mut a, vec![], SIZE);
        }
        let menu = right_click(&ctx, &mut a, &email);
        assert!(has(&menu, "New calendar…") && has(&menu, "Removed calendars"), "{menu:?}");
        point_at(&ctx, &mut a, "Removed calendars");
        click_text(&ctx, &mut a, "Trips");
        assert!(!a.session.calendar_removed(&account, TRIPS), "brought back from the heading's menu");
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// An account with every calendar removed (a CalDAV one: no main calendar stays) keeps its heading
/// in the side pane, and the heading its menu: Removed calendars lists them, one brought back.
#[test]
fn every_calendar_removed_keeps_the_accounts_heading() {
    for dark in [false, true] {
        let (mut a, dir, ctx) = calendars(&format!("calendar-removed-all-{dark}"), false, dark);
        let account = a.session.accounts[0].config.id.clone();
        let email = a.session.accounts[0].config.email.clone();
        a.session.pim_mut(&account).calendars.retain(|c| c.id != quatrelle_engine::model::pim::LOCAL);
        let ids: Vec<String> = a.session.ordered_calendars(&account).iter().map(|c| c.id.clone()).collect();
        assert!(ids.len() >= 3, "{ids:?}");
        for id in &ids {
            a.run("calendar.remove", json!({"account": account, "calendar": id}));
        }
        assert!(a.session.ordered_calendars(&account).is_empty(), "none listed");
        for _ in 0..5 {
            frame_sized(&ctx, &mut a, vec![], SIZE);
        }
        let menu = right_click(&ctx, &mut a, &email);
        assert!(has(&menu, "Removed calendars"), "{menu:?}");
        point_at(&ctx, &mut a, "Removed calendars");
        click_text(&ctx, &mut a, "Work");
        assert!(!a.session.calendar_removed(&account, "https://dav.contoso.example/cal/work/"), "brought back");
        let _ = std::fs::remove_dir_all(dir);
    }
}
