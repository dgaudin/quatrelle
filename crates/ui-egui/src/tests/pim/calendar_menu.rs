// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: the right-click menu on an appointment in the calendar's grid (open it, answer it,
//! how it shows, private, delete), and only Open and Join in a calendar that can't be changed.

use quatrelle_engine::AgendaItem;
use quatrelle_engine::model::ical::ShowAs;

use super::*;
use crate::calendar::chip_id;

const SIZE: egui::Vec2 = egui::vec2(1280.0, 800.0);

/// The sample in the Day view, `step` days on from today, its chips settled (the day scrolled to
/// the working hours).
fn day_view(name: &str, step: i64) -> (MailApp, std::path::PathBuf, egui::Context) {
    let (mut a, dir) = app(name);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("view.module", json!({"name": "calendar"}));
    a.run("calendar.view", json!({"mode": "day"}));
    if step != 0 {
        a.run("calendar.goto", json!({"step": step}));
    }
    for _ in 0..30 {
        frame(&ctx, &mut a, vec![]);
    }
    (a, dir, ctx)
}

/// The appointment called `summary` in the days shown.
fn shown(a: &MailApp, summary: &str) -> AgendaItem {
    let (d0, d1) = a.session.cal_days();
    let items = a.session.agenda(a.session.local_midnight(d0), a.session.local_midnight(d1));
    items.into_iter().find(|i| i.summary == summary).unwrap_or_else(|| panic!("{summary} isn't in the view"))
}

fn texts(shapes: &[egui::Shape]) -> Vec<String> {
    shapes
        .iter()
        .filter_map(|s| match s {
            egui::Shape::Text(t) => Some(t.galley.text().to_string()),
            _ => None,
        })
        .collect()
}

/// A right click (pressed and let go where it was) on the appointment's chip: the texts the
/// frame draws then that it didn't before (the menu's entries).
fn right_click(ctx: &egui::Context, a: &mut MailApp, item: &AgendaItem) -> Vec<String> {
    let at = ctx.read_response(chip_id(item)).map(|r| r.rect.center()).expect("the chip is drawn");
    frame(ctx, a, vec![Event::PointerMoved(at)]);
    for _ in 0..60 {
        frame(ctx, a, vec![]);
    }
    let mut before = texts(&frame_shapes(ctx, a, vec![], SIZE));
    let button = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE };
    frame(ctx, a, vec![button(true)]);
    frame(ctx, a, vec![button(false)]);
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

/// Through the menu open: each of `path` but the last pointed at (a submenu opens), the last
/// clicked.
fn choose(ctx: &egui::Context, a: &mut MailApp, path: &[&str]) {
    let Some((last, through)) = path.split_last() else { return };
    for entry in through {
        let at = settled_text_at(ctx, a, entry, SIZE).unwrap_or_else(|| panic!("no {entry} in the menu"));
        frame(ctx, a, vec![Event::PointerMoved(at)]);
        for _ in 0..5 {
            frame(ctx, a, vec![]);
        }
    }
    let at = settled_text_at(ctx, a, last, SIZE).unwrap_or_else(|| panic!("no {last} in the menu"));
    let press = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame(ctx, a, vec![Event::PointerMoved(at)]);
    frame(ctx, a, vec![press(true)]);
    frame(ctx, a, vec![press(false)]);
    frame(ctx, a, vec![]);
}

fn sent(a: &MailApp) -> usize {
    a.session.store.as_ref().and_then(|s| s.load_folder("sample", "Sent").ok()).map_or(0, |f| f.messages.len())
}

/// A right click on an appointment opens its menu: no window opens, and it doesn't move (nor
/// start a drag).
#[test]
fn a_right_click_opens_the_appointments_menu() {
    let (mut a, dir, ctx) = day_view("menu-opens", 0);
    let lunch = shown(&a, "Lunch with Jun");
    let menu = right_click(&ctx, &mut a, &lunch);
    for entry in ["Open", "Show As", "Categorize", "Private", "Delete"] {
        assert!(menu.iter().any(|t| t == entry), "no {entry} in {menu:?}");
    }
    assert!(!menu.iter().any(|t| t == "Accept"), "the user's own appointment has no answer: {menu:?}");
    assert!(a.ui.dialog.is_none(), "nothing opened");
    assert!(a.ui.cal_drag.is_none(), "no drag");
    let e = a.session.find_event(&lunch.uid).map(|(_, e)| e.event.clone()).unwrap();
    assert_eq!((e.start, e.end), (lunch.start, lunch.end), "it hasn't moved");
    let _ = std::fs::remove_dir_all(dir);
}

/// Dragged with the right button, an appointment doesn't move (only the left one takes hold).
#[test]
fn a_right_button_drag_moves_nothing() {
    let (mut a, dir, ctx) = day_view("menu-no-drag", 0);
    let lunch = shown(&a, "Lunch with Jun");
    let from = ctx.read_response(chip_id(&lunch)).map(|r| r.rect.center()).expect("the chip is drawn");
    let to = from + egui::vec2(0.0, crate::calendar::HOUR);
    let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE };
    frame(&ctx, &mut a, vec![Event::PointerMoved(from)]);
    frame(&ctx, &mut a, vec![button(from, true)]);
    frame(&ctx, &mut a, vec![Event::PointerMoved(from + egui::vec2(0.0, 14.0))]);
    frame(&ctx, &mut a, vec![Event::PointerMoved(to)]);
    frame(&ctx, &mut a, vec![button(to, false)]);
    frame(&ctx, &mut a, vec![]);
    let e = a.session.find_event(&lunch.uid).map(|(_, e)| e.event.clone()).unwrap();
    assert_eq!((e.start, e.end), (lunch.start, lunch.end), "it hasn't moved");
    let _ = std::fs::remove_dir_all(dir);
}

/// Private, from the menu: the appointment is no longer private, and no one is told.
#[test]
fn private_from_the_menu_tells_no_one() {
    let (mut a, dir, ctx) = day_view("menu-private", 0);
    let lunch = shown(&a, "Lunch with Jun");
    assert!(lunch.private);
    let before = sent(&a);
    right_click(&ctx, &mut a, &lunch);
    choose(&ctx, &mut a, &["Private"]);
    assert!(!shown(&a, "Lunch with Jun").private);
    assert_eq!(sent(&a), before, "nothing sent");
    let _ = std::fs::remove_dir_all(dir);
}

/// Show As › Free, from the menu.
#[test]
fn show_as_from_the_menu() {
    let (mut a, dir, ctx) = day_view("menu-show-as", 0);
    let lunch = shown(&a, "Lunch with Jun");
    assert_eq!(lunch.show_as, ShowAs::Busy);
    right_click(&ctx, &mut a, &lunch);
    choose(&ctx, &mut a, &["Show As", "Free"]);
    assert_eq!(shown(&a, "Lunch with Jun").show_as, ShowAs::Free);
    let _ = std::fs::remove_dir_all(dir);
}

/// An invitation not answered yet: Accept, from the menu, answers it (the reply goes).
#[test]
fn an_invitation_is_answered_from_the_menu() {
    let (mut a, dir, ctx) = day_view("menu-answer", 1);
    let mix = shown(&a, "Sound mix playback");
    assert_eq!(mix.answer.as_deref(), Some("NEEDS-ACTION"));
    let before = sent(&a);
    let menu = right_click(&ctx, &mut a, &mix);
    for entry in ["Accept", "Tentative", "Decline"] {
        assert!(menu.iter().any(|t| t == entry), "no {entry} in {menu:?}");
    }
    choose(&ctx, &mut a, &["Accept"]);
    assert_eq!(shown(&a, "Sound mix playback").answer.as_deref(), Some("ACCEPTED"));
    assert_eq!(sent(&a), before + 1, "the reply is in Sent");
    let _ = std::fs::remove_dir_all(dir);
}

/// A series' Delete asks which: this occurrence goes, the series stays.
#[test]
fn a_series_asks_which_to_delete() {
    let (mut a, dir, ctx) = day_view("menu-delete-series", 0);
    let standup = shown(&a, "Team stand-up");
    assert!(standup.recurring);
    right_click(&ctx, &mut a, &standup);
    choose(&ctx, &mut a, &["Delete", "Delete this occurrence"]);
    let (d0, d1) = a.session.cal_days();
    let today = a.session.agenda(a.session.local_midnight(d0), a.session.local_midnight(d1));
    assert!(!today.iter().any(|i| i.summary == "Team stand-up"), "this one is gone: {today:?}");
    let week = a.session.agenda(standup.start - 7 * 86_400, standup.start + 7 * 86_400);
    assert!(week.iter().any(|i| i.summary == "Team stand-up"), "the series stays");
    let _ = std::fs::remove_dir_all(dir);
}

/// In a colleague's calendar, which can't be changed: Open, and Join for a meeting with a link,
/// nothing else.
#[test]
fn a_read_only_calendar_offers_only_open() {
    const TEAMS: &str = "https://teams.microsoft.com/l/meetup-join/19%3ameeting_x%40thread.v2/0?context=%7b%7d";
    let (mut a, dir, ctx) = day_view("menu-read-only", 0);
    let zoe = quatrelle_engine::model::pim::shared_calendar_id("zoe@studio.example");
    a.run("calendar.show", json!({"account": "sample", "calendar": zoe, "shown": true}));
    let call = a.session.pim_mut("sample").events.iter_mut().find(|e| e.event.summary == "Producers' call").unwrap();
    call.event.location = TEAMS.into();
    a.session.pim_revision += 1;
    for _ in 0..5 {
        frame(&ctx, &mut a, vec![]);
    }
    let call = shown(&a, "Producers' call");
    assert!(a.session.pim["sample"].calendars.iter().any(|c| c.id == call.calendar && c.read_only));
    let menu = right_click(&ctx, &mut a, &call);
    assert!(menu.iter().any(|t| t == "Open"), "{menu:?}");
    assert!(menu.iter().any(|t| t == "Join meeting"), "{menu:?}");
    for entry in ["Show As", "Categorize", "Private", "Delete", "Accept"] {
        assert!(!menu.iter().any(|t| t == entry), "{entry} in {menu:?}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// Categorize, from the menu: a category ticked comes off, the others stay.
#[test]
fn categorize_from_the_menu() {
    let (mut a, dir, ctx) = day_view("menu-categorize", 0);
    let lunch = shown(&a, "Lunch with Jun");
    let categories = |a: &MailApp| a.session.find_event(&lunch.uid).map(|(_, e)| e.event.categories.clone()).unwrap_or_default();
    assert_eq!(categories(&a), ["Personal"]);
    let before = sent(&a);
    right_click(&ctx, &mut a, &lunch);
    choose(&ctx, &mut a, &["Categorize", "● Personal"]);
    assert!(categories(&a).is_empty(), "{:?}", categories(&a));
    assert_eq!(sent(&a), before, "nothing sent");
    let _ = std::fs::remove_dir_all(dir);
}
