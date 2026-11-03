// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: clicks in the calendar's grid, as Outlook's: a click selects an appointment or an
//! empty half hour, a double click opens it (or a new appointment there), Enter opens the
//! appointment selected, a right click selects it too; in the Month view a click on a day's
//! background selects the day.

use quatrelle_engine::AgendaItem;

use super::calendar_menu::{SIZE, day_view, right_click, shown};
use super::*;
use crate::calendar::{HOUR, Pick, chip_id, month_day_id};

pub(super) fn chip_rect(ctx: &egui::Context, item: &AgendaItem) -> egui::Rect {
    ctx.read_response(chip_id(item)).map(|r| r.rect).expect("the chip is drawn")
}

/// The appointment's form opened (as a dialog, or already in a window of its own).
pub(super) fn form(a: &MailApp) -> Option<&crate::dialogs::pim_dialogs::EventForm> {
    match &a.ui.dialog {
        Some(Dialog::Event(f)) => Some(&**f),
        _ => a.ui.event_windows.last().map(|(_, f)| &**f),
    }
}

/// The subject of the appointment opened.
fn opened(a: &MailApp) -> Option<String> {
    form(a).map(|f| f.summary.clone())
}

/// Nothing opened: no dialog, no appointment's window.
pub(super) fn nothing_opened(a: &MailApp) -> bool {
    a.ui.dialog.is_none() && a.ui.event_windows.is_empty()
}

/// The pointer away from the grid (on the window's title), a frame drawn: what it drew.
pub(super) fn drawn_away(ctx: &egui::Context, a: &mut MailApp, size: egui::Vec2) -> Vec<egui::Shape> {
    frame_sized(ctx, a, vec![Event::PointerMoved(egui::pos2(640.0, 2.0))], size);
    let mut shapes = Vec::new();
    for _ in 0..3 {
        shapes = frame_shapes(ctx, a, vec![], size);
    }
    shapes
}

/// A box drawn round `r` with a border at least 2 points thick: how the selected chip is drawn.
fn outlined(shapes: &[egui::Shape], r: egui::Rect) -> bool {
    shapes.iter().any(|s| matches!(s, egui::Shape::Rect(x) if x.rect == r && x.stroke.width >= 2.0))
}

/// One click on an appointment, the 3 views it's drawn in: nothing opens, it's selected (drawn
/// with a thick border); a double click opens it (in a window of `size`).
fn click_then_double_click(ctx: &egui::Context, a: &mut MailApp, item: &AgendaItem, size: egui::Vec2) {
    let r = chip_rect(ctx, item);
    assert!(!outlined(&drawn_away(ctx, a, size), r), "not selected yet");
    click_at(ctx, a, r.center(), 1, size);
    assert!(nothing_opened(a), "a click opens nothing: {:?}", opened(a));
    assert_eq!(a.ui.cal_pick, Some(Pick::of(item)), "a click selects it");
    assert!(outlined(&drawn_away(ctx, a, size), r), "drawn selected, the pointer away");
    click_at(ctx, a, r.center(), 2, size);
    assert_eq!(opened(a).as_deref(), Some(item.summary.as_str()), "a double click opens it");
}

#[test]
fn a_click_selects_an_appointment_a_double_click_opens_it() {
    let (mut a, dir, ctx) = day_view("clicks-day", 0);
    let lunch = shown(&a, "Lunch with Jun");
    click_then_double_click(&ctx, &mut a, &lunch, SIZE);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_click_selects_a_month_chip_a_double_click_opens_it() {
    let (mut a, dir, ctx) = day_view("clicks-month", 0);
    a.run("calendar.view", json!({"mode": "month"}));
    // Tall enough for a day's two appointments (at 800 points, one and "1 more").
    let tall = egui::vec2(1280.0, 1300.0);
    for _ in 0..10 {
        frame_sized(&ctx, &mut a, vec![], tall);
    }
    let lunch = shown(&a, "Lunch with Jun");
    click_then_double_click(&ctx, &mut a, &lunch, tall);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_click_selects_an_all_day_chip_a_double_click_opens_it() {
    let (mut a, dir, ctx) = day_view("clicks-all-day", 2);
    let poster = shown(&a, "Poster to the print shop");
    assert!(poster.all_day);
    click_then_double_click(&ctx, &mut a, &poster, SIZE);
    let _ = std::fs::remove_dir_all(dir);
}

/// One click on an empty half hour: nothing opens, it's selected (and stays lit with the pointer
/// away), the appointment selected before isn't any more; a double click opens a new appointment
/// at that time; a click on an appointment then unselects the half hour.
#[test]
fn a_click_selects_an_empty_half_hour_a_double_click_adds_an_appointment() {
    let (mut a, dir, ctx) = day_view("clicks-slot", 0);
    let lunch = shown(&a, "Lunch with Jun");
    let chip = chip_rect(&ctx, &lunch);
    let day = a.session.cal_days().0;
    let midnight = a.session.local_midnight(day);
    // Where the grid starts, from where the chip is drawn.
    let top = chip.min.y - 1.0 - ((lunch.start - midnight) as f32 / 3600.0) * HOUR;
    let (d0, d1) = a.session.cal_days();
    let items = a.session.agenda(a.session.local_midnight(d0), a.session.local_midnight(d1));
    let half = (20..30_i64)
        .find(|h| {
            let (from, to) = (midnight + h * 1800, midnight + (h + 1) * 1800);
            !items.iter().any(|i| !i.all_day && i.start < to && i.end > from)
        })
        .expect("an empty half hour between 10:00 and 15:00");
    let at = egui::pos2(chip.center().x, top + (half as f32 + 0.5) * HOUR / 2.0);
    click_at(&ctx, &mut a, chip.center(), 1, SIZE);
    assert_eq!(a.ui.cal_pick, Some(Pick::of(&lunch)));
    click_at(&ctx, &mut a, at, 1, SIZE);
    assert!(nothing_opened(&a), "a click opens nothing: {:?}", opened(&a));
    assert_eq!(a.ui.cal_pick, Some(Pick::Slot { day, half, pane: 0 }), "the half hour selected, the appointment not");
    let accent_soft = a.ui.tokens.accent_soft;
    let lit = |shapes: &[egui::Shape]| {
        shapes.iter().any(|s| matches!(s, egui::Shape::Rect(x) if x.fill == accent_soft && x.rect.contains(at) && x.rect.height() < HOUR))
    };
    assert!(lit(&drawn_away(&ctx, &mut a, SIZE)), "the half hour stays lit, the pointer away");
    assert!(!outlined(&drawn_away(&ctx, &mut a, SIZE), chip), "the appointment isn't drawn selected");
    click_at(&ctx, &mut a, at, 2, SIZE);
    let f = form(&a).expect("a new appointment opened");
    assert!(f.summary.is_empty(), "a new appointment: {}", f.summary);
    assert_eq!(f.start, format!("{:02}:{:02}", half / 2, (half % 2) * 30));
    (a.ui.dialog, a.ui.event_windows) = (None, Vec::new());
    click_at(&ctx, &mut a, chip.center(), 1, SIZE);
    assert_eq!(a.ui.cal_pick, Some(Pick::of(&lunch)), "the appointment selected, the half hour not");
    assert!(!lit(&drawn_away(&ctx, &mut a, SIZE)), "the half hour no longer lit");
    let _ = std::fs::remove_dir_all(dir);
}

/// Enter opens the appointment selected (nothing when none is).
#[test]
fn enter_opens_the_appointment_selected() {
    let (mut a, dir, ctx) = day_view("clicks-enter", 0);
    let lunch = shown(&a, "Lunch with Jun");
    frame(&ctx, &mut a, vec![key(Key::Enter, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    assert!(nothing_opened(&a), "nothing selected, nothing opens");
    click_at(&ctx, &mut a, chip_rect(&ctx, &lunch).center(), 1, SIZE);
    assert!(nothing_opened(&a));
    frame(&ctx, &mut a, vec![key(Key::Enter, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(opened(&a).as_deref(), Some("Lunch with Jun"));
    let _ = std::fs::remove_dir_all(dir);
}

/// A right click selects the appointment too, and opens its menu (no window).
#[test]
fn a_right_click_selects_the_appointment() {
    let (mut a, dir, ctx) = day_view("clicks-right", 0);
    let lunch = shown(&a, "Lunch with Jun");
    let menu = right_click(&ctx, &mut a, &lunch);
    assert!(menu.iter().any(|t| t == "Open"), "{menu:?}");
    assert_eq!(a.ui.cal_pick, Some(Pick::of(&lunch)));
    assert!(nothing_opened(&a));
    let _ = std::fs::remove_dir_all(dir);
}

/// In the Month view, a click on an empty day's background selects the day (lit, the pointer
/// away; the appointment selected before isn't any more); a click on an appointment then
/// unselects it; a double click on the day still opens a new appointment there, at 9:00.
#[test]
fn a_click_selects_a_month_day() {
    let (mut a, dir, ctx) = day_view("clicks-month-day", 0);
    a.run("calendar.view", json!({"mode": "month"}));
    let tall = egui::vec2(1280.0, 1300.0);
    for _ in 0..10 {
        frame_sized(&ctx, &mut a, vec![], tall);
    }
    let lunch = shown(&a, "Lunch with Jun");
    let (d0, d1) = a.session.cal_days();
    let items = a.session.agenda(a.session.local_midnight(d0), a.session.local_midnight(d1));
    let day = (d0..d1)
        .find(|d| {
            let (from, to) = (a.session.local_midnight(*d), a.session.local_midnight(d + 1));
            !items.iter().any(|i| i.start < to && i.end.max(i.start + 1) > from)
        })
        .expect("an empty day in the month");
    let cell = ctx.read_response(month_day_id(day)).map(|r| r.rect).expect("the day is drawn");
    let at = cell.center();
    click_at(&ctx, &mut a, chip_rect(&ctx, &lunch).center(), 1, tall);
    assert_eq!(a.ui.cal_pick, Some(Pick::of(&lunch)));
    click_at(&ctx, &mut a, at, 1, tall);
    assert!(nothing_opened(&a), "a click opens nothing: {:?}", opened(&a));
    assert_eq!(a.ui.cal_pick, Some(Pick::Day { day }), "the day selected, the appointment not");
    let accent_soft = a.ui.tokens.accent_soft;
    let lit = |shapes: &[egui::Shape]| shapes.iter().any(|s| matches!(s, egui::Shape::Rect(x) if x.fill == accent_soft && x.rect.contains(at)));
    assert!(lit(&drawn_away(&ctx, &mut a, tall)), "the day stays lit, the pointer away");
    click_at(&ctx, &mut a, chip_rect(&ctx, &lunch).center(), 1, tall);
    assert_eq!(a.ui.cal_pick, Some(Pick::of(&lunch)), "the appointment selected, the day not");
    assert!(!lit(&drawn_away(&ctx, &mut a, tall)), "the day no longer lit");
    click_at(&ctx, &mut a, at, 2, tall);
    let f = form(&a).expect("a new appointment opened");
    assert!(f.summary.is_empty(), "a new appointment: {}", f.summary);
    assert_eq!(f.start, "09:00");
    let _ = std::fs::remove_dir_all(dir);
}
