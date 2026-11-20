// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: a stretch of half hours selected by dragging in the day and week views, as
//! Outlook's: green from the half hour pressed to the one under the pointer, Enter opens a
//! meeting over it, Ctrl+N and a double click inside it open an appointment over it.

use quatrelle_engine::model::dates::civil_from_days;
use quatrelle_engine::sample::STUDIO_CALENDAR;

use super::calendar_clicks::{chip_rect, drawn_away, form};
use super::calendar_menu::{SIZE, choose, day_view, right_click_at, shown};
use super::*;
use crate::calendar::{HOUR, Pick, chip_id, month_day_id};

/// A window tall enough for the whole day, so a drag can reach its last half hour.
const TALL: egui::Vec2 = egui::vec2(1280.0, 1600.0);
/// A window tall enough for a Month view's days to hold a few appointments.
const MONTH: egui::Vec2 = egui::vec2(1280.0, 1300.0);

fn press(at: egui::Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

/// Where the grid starts, and the middle of the day's column, worked out from the lunch's chip.
fn grid(ctx: &egui::Context, a: &MailApp) -> (f32, f32) {
    let lunch = shown(a, "Lunch with Jun");
    let chip = ctx.read_response(chip_id(&lunch)).map(|r| r.rect).expect("the chip is drawn");
    let day = a.session.local_midnight(a.session.cal_days().0);
    (chip.min.y - 1.0 - ((lunch.start - day) as f32 / 3600.0) * HOUR, chip.center().x)
}

/// The grid's point at `hour` (10.25 is 10:15), in the day's column.
fn point(top: f32, x: f32, hour: f32) -> egui::Pos2 {
    egui::pos2(x, top + hour * HOUR)
}

/// A press at `from`, dragged to `to` (a little way first, as a pointer does), then let go.
fn drag(ctx: &egui::Context, a: &mut MailApp, from: egui::Pos2, to: egui::Pos2, size: egui::Vec2) {
    frame_sized(ctx, a, vec![Event::PointerMoved(from)], size);
    frame_sized(ctx, a, vec![press(from, true)], size);
    frame_sized(ctx, a, vec![Event::PointerMoved(from + (to - from) * 0.1)], size);
    frame_sized(ctx, a, vec![Event::PointerMoved(to)], size);
    frame_sized(ctx, a, vec![press(to, false)], size);
    frame_sized(ctx, a, vec![], size);
}

/// A selection made of several half hours: its day, the first half hour and the last.
fn stretch(a: &MailApp) -> Option<(i64, i64, i64)> {
    match &a.ui.cal_pick {
        Some(Pick::Slot { day, half, last, pane: _ }) => Some((*day, (*half).min(*last), (*half).max(*last))),
        _ => None,
    }
}

fn ymd(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

/// A press at 16:30 dragged to 17:45 selects 16:30–18:00 as one block; Enter opens a meeting
/// covering exactly that, in that day, ready for its guests.
#[test]
fn a_drag_selects_a_stretch_and_enter_opens_a_meeting_over_it() {
    let (mut a, dir, ctx) = day_view("selection-enter", 0);
    let day = a.session.cal_days().0;
    let (top, x) = grid(&ctx, &a);
    drag(&ctx, &mut a, point(top, x, 16.5), point(top, x, 17.75), SIZE);
    let (d, first, last) = stretch(&a).expect("a stretch selected");
    assert_eq!((d, first, last), (day, 33, 35), "16:30 to 18:00");
    // One block, green, from 16:30 to 18:00, still drawn with the pointer away.
    let accent = a.ui.tokens.accent_soft;
    let shapes = drawn_away(&ctx, &mut a, SIZE);
    let block = shapes.iter().find_map(|s| match s {
        egui::Shape::Rect(r) if r.fill == accent && r.rect.width() > HOUR => Some(r.rect),
        _ => None,
    });
    let block = block.unwrap_or_else(|| panic!("the stretch is drawn green"));
    assert!((block.min.y - (top + 16.5 * HOUR)).abs() < 2.0, "from 16:30: {block:?}");
    assert!((block.max.y - (top + 18.0 * HOUR)).abs() < 2.0, "to 18:00: {block:?}");
    // Enter opens the meeting over it, with its guests' lines shown.
    frame_sized(&ctx, &mut a, vec![key(Key::Enter, Modifiers::NONE)], SIZE);
    frame_sized(&ctx, &mut a, vec![], SIZE);
    let f = form(&a).expect("a meeting opened");
    assert!(f.inviting, "as if Invite Attendees was pressed");
    assert_eq!(
        (f.date.as_str(), f.start.as_str(), f.end_date.as_str(), f.end.as_str()),
        (ymd(day).as_str(), "16:30", ymd(day).as_str(), "18:00"),
        "the whole stretch"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// Ctrl+N over the selection, and a double click inside it, open the same stretch as an
/// appointment (not a meeting).
#[test]
fn ctrl_n_and_a_double_click_inside_the_selection_keep_its_stretch() {
    let (mut a, dir, ctx) = day_view("selection-keys", 0);
    let day = a.session.cal_days().0;
    let (top, x) = grid(&ctx, &a);
    drag(&ctx, &mut a, point(top, x, 16.5), point(top, x, 17.75), SIZE);
    assert_eq!(stretch(&a), Some((day, 33, 35)));
    // Ctrl+N: the appointment over the whole selection.
    frame_sized(&ctx, &mut a, vec![key(Key::N, Modifiers::COMMAND)], SIZE);
    for _ in 0..3 {
        frame_sized(&ctx, &mut a, vec![], SIZE);
    }
    let f = form(&a).filter(|f| f.uid.is_none()).expect("a new appointment");
    assert!(!f.inviting, "an appointment");
    assert_eq!((f.date.as_str(), f.start.as_str(), f.end_date.as_str(), f.end.as_str()), (ymd(day).as_str(), "16:30", ymd(day).as_str(), "18:00"));
    (a.ui.dialog, a.ui.event_windows) = (None, Vec::new());
    // A double click inside it: the same stretch, not the half hour clicked.
    frame_sized(&ctx, &mut a, vec![], SIZE);
    click_at(&ctx, &mut a, point(top, x, 17.0), 2, SIZE);
    let f = form(&a).filter(|f| f.uid.is_none()).expect("a new appointment");
    assert_eq!(
        (f.date.as_str(), f.start.as_str(), f.end_date.as_str(), f.end.as_str()),
        (ymd(day).as_str(), "16:30", ymd(day).as_str(), "18:00"),
        "the selection's stretch"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A drag back up shrinks the selection while the button is still down; dragged past the
/// bottom of the day it stops at its last half hour.
#[test]
fn a_drag_back_shrinks_the_stretch_and_it_stops_at_the_days_ends() {
    let (mut a, dir, ctx) = day_view("selection-bounds", 0);
    let day = a.session.cal_days().0;
    // The tall window settles first: its height changes where the hours are scrolled to.
    for _ in 0..10 {
        frame_sized(&ctx, &mut a, vec![], TALL);
    }
    let (top, x) = grid(&ctx, &a);
    let from = point(top, x, 16.5);
    frame_sized(&ctx, &mut a, vec![Event::PointerMoved(from)], TALL);
    frame_sized(&ctx, &mut a, vec![press(from, true)], TALL);
    frame_sized(&ctx, &mut a, vec![Event::PointerMoved(point(top, x, 19.5))], TALL);
    assert_eq!(stretch(&a), Some((day, 33, 39)), "16:30 to 19:30 while dragging");
    frame_sized(&ctx, &mut a, vec![Event::PointerMoved(point(top, x, 17.5))], TALL);
    assert_eq!(stretch(&a), Some((day, 33, 35)), "back up to 17:30");
    frame_sized(&ctx, &mut a, vec![press(point(top, x, 17.5), false)], TALL);
    frame_sized(&ctx, &mut a, vec![], TALL);
    // From 22:00 dragged past the bottom: the last half hour, 23:30–24:00.
    let from = point(top, x, 22.0);
    frame_sized(&ctx, &mut a, vec![Event::PointerMoved(from)], TALL);
    frame_sized(&ctx, &mut a, vec![press(from, true)], TALL);
    let past = egui::pos2(x, top + 24.5 * HOUR);
    frame_sized(&ctx, &mut a, vec![Event::PointerMoved(past)], TALL);
    assert_eq!(stretch(&a), Some((day, 44, 47)), "22:00 to 24:00");
    frame_sized(&ctx, &mut a, vec![press(past, false)], TALL);
    frame_sized(&ctx, &mut a, vec![], TALL);
    // And past the top: the first half hour, 00:00–00:30.
    let from = point(top, x, 2.0);
    frame_sized(&ctx, &mut a, vec![Event::PointerMoved(from)], TALL);
    frame_sized(&ctx, &mut a, vec![press(from, true)], TALL);
    let above = egui::pos2(x, top - HOUR);
    frame_sized(&ctx, &mut a, vec![Event::PointerMoved(above)], TALL);
    assert_eq!(stretch(&a), Some((day, 0, 4)), "00:00 to 02:30");
    frame_sized(&ctx, &mut a, vec![press(above, false)], TALL);
    frame_sized(&ctx, &mut a, vec![], TALL);
    let _ = std::fs::remove_dir_all(dir);
}

/// A stretch selected goes when another week, another day or another view is shown.
#[test]
fn a_stretch_goes_when_another_week_or_view_is_shown() {
    let (mut a, dir, ctx) = day_view("selection-gone", 0);
    let (top, x) = grid(&ctx, &a);
    drag(&ctx, &mut a, point(top, x, 16.5), point(top, x, 17.75), SIZE);
    assert!(stretch(&a).is_some());
    a.run("calendar.goto", json!({"step": 1}));
    for _ in 0..2 {
        frame_sized(&ctx, &mut a, vec![], SIZE);
    }
    assert_eq!(stretch(&a), None, "another day: gone");
    // Selected again in that day, another view lets it go too.
    drag(&ctx, &mut a, point(top, x, 16.5), point(top, x, 17.75), SIZE);
    assert!(stretch(&a).is_some());
    a.run("calendar.view", json!({"mode": "week"}));
    for _ in 0..2 {
        frame_sized(&ctx, &mut a, vec![], SIZE);
    }
    assert_eq!(stretch(&a), None, "another view: gone");
    let _ = std::fs::remove_dir_all(dir);
}

/// With calendars side by side, a stretch stays in the pane (and calendar) it was pressed in:
/// dragged towards another pane, it doesn't move there, and Enter opens its meeting in that
/// calendar.
#[test]
fn a_stretch_stays_in_the_pane_and_calendar_it_was_pressed_in() {
    let (mut a, dir, ctx) = day_view("selection-panes", 0);
    a.run("calendar.sideBySide", json!({"on": true}));
    let others: Vec<(String, String, String, u8)> =
        a.session.shown_calendars().into_iter().filter(|(_, id, _, _)| id != STUDIO_CALENDAR && id != quatrelle_engine::model::pim::LOCAL).collect();
    for (account, calendar, _, _) in others {
        a.run("calendar.show", json!({"account": account, "calendar": calendar, "shown": false}));
    }
    for _ in 0..5 {
        frame_sized(&ctx, &mut a, vec![], SIZE);
    }
    let day = a.session.cal_days().0;
    let lunch = shown(&a, "Lunch with Jun");
    let top = chip_rect(&ctx, &lunch).min.y - 1.0 - ((lunch.start - a.session.local_midnight(day)) as f32 / 3600.0) * HOUR;
    let studio = shown(&a, "Edit session: act two");
    let x = chip_rect(&ctx, &studio).center().x;
    drag(&ctx, &mut a, point(top, x, 16.5), point(top, x, 17.75), SIZE);
    match &a.ui.cal_pick {
        Some(Pick::Slot { day: d, half, last, pane }) => assert_eq!((*d, *half, *last, *pane), (day, 33, 35, 1), "the studio's pane"),
        other => panic!("a stretch selected: {other:?}"),
    }
    frame_sized(&ctx, &mut a, vec![key(Key::Enter, Modifiers::NONE)], SIZE);
    frame_sized(&ctx, &mut a, vec![], SIZE);
    let f = form(&a).expect("a meeting opened");
    assert!(f.inviting);
    assert_eq!(f.calendars.get(f.calendar).map(|c| c.1.as_str()), Some(STUDIO_CALENDAR), "in the studio's calendar");
    let _ = std::fs::remove_dir_all(dir);
}

/// A press on an appointment dragged moves it, as before, and selects no stretch of time.
#[test]
fn an_appointment_dragged_still_moves_and_selects_no_stretch() {
    let (mut a, dir, ctx) = day_view("selection-appointment", 0);
    let lunch = shown(&a, "Lunch with Jun");
    let at = ctx.read_response(chip_id(&lunch)).map(|r| r.rect.center()).expect("the chip is drawn");
    drag(&ctx, &mut a, at, at + egui::vec2(0.0, HOUR), SIZE);
    let moved = shown(&a, "Lunch with Jun");
    assert_eq!((moved.start, moved.end), (lunch.start + 3600, lunch.end + 3600), "moved an hour on");
    assert!(stretch(&a).is_none(), "no stretch selected");
    let _ = std::fs::remove_dir_all(dir);
}

/// A double click outside any selection opens an appointment of the half hour clicked, as
/// before; a single click selects just that half hour.
#[test]
fn a_click_outside_a_selection_still_takes_one_half_hour() {
    let (mut a, dir, ctx) = day_view("selection-single", 0);
    let day = a.session.cal_days().0;
    let (top, x) = grid(&ctx, &a);
    let at = point(top, x, 16.5);
    click_at(&ctx, &mut a, at, 1, SIZE);
    assert_eq!(stretch(&a), Some((day, 33, 33)), "one half hour selected");
    click_at(&ctx, &mut a, at, 2, SIZE);
    let f = form(&a).filter(|f| f.uid.is_none()).expect("a new appointment");
    assert_eq!(
        (f.date.as_str(), f.start.as_str(), f.end_date.as_str(), f.end.as_str()),
        (ymd(day).as_str(), "16:30", ymd(day).as_str(), "17:00"),
        "half an hour"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// The sample in the Month view, its days settled.
fn month_view(name: &str) -> (MailApp, std::path::PathBuf, egui::Context) {
    let (mut a, dir, ctx) = day_view(name, 0);
    a.run("calendar.view", json!({"mode": "month"}));
    for _ in 0..10 {
        frame_sized(&ctx, &mut a, vec![], MONTH);
    }
    (a, dir, ctx)
}

/// Where a day's background is drawn in the Month view.
fn month_cell(ctx: &egui::Context, day: i64) -> egui::Rect {
    ctx.read_response(month_day_id(day)).map(|r| r.rect).unwrap_or_else(|| panic!("day {day} is drawn"))
}

/// A press in one day's background, dragged to another's, then let go.
fn drag_days(ctx: &egui::Context, a: &mut MailApp, from: i64, to: i64) {
    drag(ctx, a, month_cell(ctx, from).center(), month_cell(ctx, to).center(), MONTH);
}

/// A press on a Thursday dragged to the next Tuesday selects those six days, lit as one
/// selection; Enter opens an all-day meeting over them, guests shown.
#[test]
fn a_month_drag_selects_the_days_and_enter_opens_an_all_day_meeting() {
    let (mut a, dir, ctx) = month_view("selection-month-days");
    let d0 = a.session.cal_days().0;
    let (thu, tue) = (d0 + 3, d0 + 8);
    drag_days(&ctx, &mut a, thu, tue);
    assert_eq!(a.ui.cal_pick, Some(Pick::Days { first: thu, last: tue }), "Thursday to the next Tuesday");
    // Every day of the stretch is lit, the pointer away.
    let accent = a.ui.tokens.accent_soft;
    let shapes = drawn_away(&ctx, &mut a, MONTH);
    for d in thu..=tue {
        let cell = month_cell(&ctx, d);
        assert!(shapes.iter().any(|s| matches!(s, egui::Shape::Rect(x) if x.fill == accent && x.rect == cell)), "day {d} is lit: {shapes:?}");
    }
    // Enter: an all-day meeting from the first day to the last, included.
    frame_sized(&ctx, &mut a, vec![key(Key::Enter, Modifiers::NONE)], MONTH);
    frame_sized(&ctx, &mut a, vec![], MONTH);
    let f = form(&a).expect("a meeting opened");
    assert!(f.inviting, "as if Invite Attendees was pressed");
    assert!(f.all_day, "all day");
    assert_eq!((f.date.as_str(), f.end_date.as_str()), (ymd(thu).as_str(), ymd(tue).as_str()), "the whole stretch");
    let _ = std::fs::remove_dir_all(dir);
}

/// Ctrl+N over the days selected opens an all-day appointment over them; so does a double click
/// inside them. Outside them, a double click still opens a new appointment that day at 9:00.
#[test]
fn ctrl_n_and_a_double_click_inside_the_days_keep_their_stretch() {
    let (mut a, dir, ctx) = month_view("selection-month-keys");
    let d0 = a.session.cal_days().0;
    let (thu, tue) = (d0 + 3, d0 + 8);
    drag_days(&ctx, &mut a, thu, tue);
    assert_eq!(a.ui.cal_pick, Some(Pick::Days { first: thu, last: tue }));
    // Ctrl+N: an appointment over the whole stretch, all day.
    frame_sized(&ctx, &mut a, vec![key(Key::N, Modifiers::COMMAND)], MONTH);
    for _ in 0..3 {
        frame_sized(&ctx, &mut a, vec![], MONTH);
    }
    let f = form(&a).filter(|f| f.uid.is_none()).expect("a new appointment");
    assert!(!f.inviting, "an appointment");
    assert!(f.all_day, "all day");
    assert_eq!((f.date.as_str(), f.end_date.as_str()), (ymd(thu).as_str(), ymd(tue).as_str()), "the stretch");
    (a.ui.dialog, a.ui.event_windows) = (None, Vec::new());
    // A double click inside it: the same stretch, not the day clicked.
    frame_sized(&ctx, &mut a, vec![], MONTH);
    click_at(&ctx, &mut a, month_cell(&ctx, d0 + 5).center(), 2, MONTH);
    let f = form(&a).filter(|f| f.uid.is_none()).expect("a new appointment");
    assert!(f.all_day);
    assert_eq!((f.date.as_str(), f.end_date.as_str()), (ymd(thu).as_str(), ymd(tue).as_str()), "the selection's stretch");
    (a.ui.dialog, a.ui.event_windows) = (None, Vec::new());
    // Outside it: the day clicked, at 9:00, as before.
    frame_sized(&ctx, &mut a, vec![], MONTH);
    click_at(&ctx, &mut a, month_cell(&ctx, d0 + 1).center(), 2, MONTH);
    let f = form(&a).filter(|f| f.uid.is_none()).expect("a new appointment");
    assert!(!f.all_day && !f.inviting);
    assert_eq!((f.date.as_str(), f.start.as_str()), (ymd(d0 + 1).as_str(), "09:00"));
    let _ = std::fs::remove_dir_all(dir);
}

/// A click still selects one day; a double click on it keeps opening an appointment that day at
/// 9:00.
#[test]
fn a_click_in_the_month_still_takes_one_day() {
    let (mut a, dir, ctx) = month_view("selection-month-single");
    let day = a.session.cal_days().0 + 1;
    let at = month_cell(&ctx, day).center();
    click_at(&ctx, &mut a, at, 1, MONTH);
    assert_eq!(a.ui.cal_pick, Some(Pick::Day { day }), "one day selected");
    click_at(&ctx, &mut a, at, 2, MONTH);
    let f = form(&a).filter(|f| f.uid.is_none()).expect("a new appointment");
    assert!(!f.all_day && !f.inviting);
    assert_eq!((f.date.as_str(), f.start.as_str()), (ymd(day).as_str(), "09:00"));
    let _ = std::fs::remove_dir_all(dir);
}

/// A press on an appointment's chip and a drag still moves it, selecting no days.
#[test]
fn an_appointment_dragged_in_the_month_still_moves_and_selects_no_days() {
    let (mut a, dir, ctx) = month_view("selection-month-appointment");
    let lunch = shown(&a, "Lunch with Jun");
    let at = chip_rect(&ctx, &lunch).center();
    let cw = month_cell(&ctx, a.session.cal_days().0).width();
    drag(&ctx, &mut a, at, at + egui::vec2(cw, 0.0), MONTH);
    let moved = shown(&a, "Lunch with Jun");
    assert_eq!((moved.start, moved.end), (lunch.start + 86_400, lunch.end + 86_400), "moved a day on");
    assert!(!matches!(a.ui.cal_pick, Some(Pick::Days { .. })), "no days selected: {:?}", a.ui.cal_pick);
    let _ = std::fs::remove_dir_all(dir);
}

/// The days selected go when another month or another view is shown.
#[test]
fn the_days_selected_go_when_another_month_or_view_is_shown() {
    let (mut a, dir, ctx) = month_view("selection-month-gone");
    let d0 = a.session.cal_days().0;
    drag_days(&ctx, &mut a, d0 + 3, d0 + 8);
    assert!(matches!(a.ui.cal_pick, Some(Pick::Days { .. })));
    a.run("calendar.goto", json!({"step": 1}));
    for _ in 0..2 {
        frame_sized(&ctx, &mut a, vec![], MONTH);
    }
    assert_eq!(a.ui.cal_pick, None, "another month: gone");
    let d0 = a.session.cal_days().0;
    drag_days(&ctx, &mut a, d0 + 3, d0 + 8);
    assert!(matches!(a.ui.cal_pick, Some(Pick::Days { .. })));
    a.run("calendar.view", json!({"mode": "week"}));
    for _ in 0..2 {
        frame_sized(&ctx, &mut a, vec![], SIZE);
    }
    assert_eq!(a.ui.cal_pick, None, "another view: gone");
    let _ = std::fs::remove_dir_all(dir);
}

/// A right click on a stretch of half hours offers New event and New meeting, which open what
/// Ctrl+N and Enter open over it; a right click on a half hour outside it selects that one first.
#[test]
fn a_right_click_on_a_stretch_offers_new_event_and_new_meeting() {
    let (mut a, dir, ctx) = day_view("selection-menu-slot", 0);
    let day = a.session.cal_days().0;
    let (top, x) = grid(&ctx, &a);
    drag(&ctx, &mut a, point(top, x, 16.5), point(top, x, 17.75), SIZE);
    assert_eq!(stretch(&a), Some((day, 33, 35)));
    let menu = right_click_at(&ctx, &mut a, point(top, x, 17.0), SIZE);
    for entry in ["New event", "New meeting"] {
        assert!(menu.iter().any(|t| t == entry), "no {entry} in {menu:?}");
    }
    assert_eq!(stretch(&a), Some((day, 33, 35)), "the stretch stays selected");
    choose(&ctx, &mut a, &["New meeting"]);
    for _ in 0..2 {
        frame_sized(&ctx, &mut a, vec![], SIZE);
    }
    let f = form(&a).filter(|f| f.uid.is_none()).expect("a new meeting");
    assert!(f.inviting && !f.all_day);
    assert_eq!((f.date.as_str(), f.start.as_str(), f.end.as_str()), (ymd(day).as_str(), "16:30", "18:00"));
    (a.ui.dialog, a.ui.event_windows) = (None, Vec::new());
    // Outside the selection, on a half hour: it is selected first, and the menu opens over it.
    frame_sized(&ctx, &mut a, vec![], SIZE);
    let menu = right_click_at(&ctx, &mut a, point(top, x, 13.5), SIZE);
    for entry in ["New event", "New meeting"] {
        assert!(menu.iter().any(|t| t == entry), "no {entry} in {menu:?}");
    }
    assert_eq!(stretch(&a), Some((day, 27, 27)), "the half hour clicked, alone");
    choose(&ctx, &mut a, &["New event"]);
    for _ in 0..2 {
        frame_sized(&ctx, &mut a, vec![], SIZE);
    }
    let f = form(&a).filter(|f| f.uid.is_none()).expect("a new appointment");
    assert!(!f.inviting && !f.all_day);
    assert_eq!((f.start.as_str(), f.end.as_str()), ("13:30", "14:00"));
    let _ = std::fs::remove_dir_all(dir);
}

/// A right click on the days selected offers the same menu, over the whole stretch; a right
/// click on a day outside it selects that day and offers New event there at 9:00.
#[test]
fn a_right_click_on_days_offers_new_event_and_new_meeting() {
    let (mut a, dir, ctx) = month_view("selection-menu-days");
    let d0 = a.session.cal_days().0;
    let (thu, tue) = (d0 + 3, d0 + 8);
    drag_days(&ctx, &mut a, thu, tue);
    let menu = right_click_at(&ctx, &mut a, month_cell(&ctx, d0 + 5).center(), MONTH);
    for entry in ["New event", "New meeting"] {
        assert!(menu.iter().any(|t| t == entry), "no {entry} in {menu:?}");
    }
    assert_eq!(a.ui.cal_pick, Some(Pick::Days { first: thu, last: tue }), "the stretch stays selected");
    choose(&ctx, &mut a, &["New meeting"]);
    for _ in 0..2 {
        frame_sized(&ctx, &mut a, vec![], MONTH);
    }
    let f = form(&a).filter(|f| f.uid.is_none()).expect("a new meeting");
    assert!(f.inviting && f.all_day);
    assert_eq!((f.date.as_str(), f.end_date.as_str()), (ymd(thu).as_str(), ymd(tue).as_str()));
    (a.ui.dialog, a.ui.event_windows) = (None, Vec::new());
    // A day outside the stretch: selected first, then the menu over it.
    frame_sized(&ctx, &mut a, vec![], MONTH);
    let menu = right_click_at(&ctx, &mut a, month_cell(&ctx, d0 + 1).center(), MONTH);
    for entry in ["New event", "New meeting"] {
        assert!(menu.iter().any(|t| t == entry), "no {entry} in {menu:?}");
    }
    assert_eq!(a.ui.cal_pick, Some(Pick::Day { day: d0 + 1 }), "the day clicked, alone");
    choose(&ctx, &mut a, &["New event"]);
    for _ in 0..2 {
        frame_sized(&ctx, &mut a, vec![], MONTH);
    }
    let f = form(&a).filter(|f| f.uid.is_none()).expect("a new appointment");
    assert!(!f.inviting && !f.all_day);
    assert_eq!((f.date.as_str(), f.start.as_str()), (ymd(d0 + 1).as_str(), "09:00"));
    let _ = std::fs::remove_dir_all(dir);
}
