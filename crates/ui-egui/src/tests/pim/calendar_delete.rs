// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: the Delete key in the calendar's grid, as Outlook's: it deletes the appointment
//! selected (a series asks which first), only where the menu's Delete would, and never the
//! messages selected in Mail.

use quatrelle_engine::AgendaItem;

use super::calendar_clicks::chip_rect;
use super::calendar_menu::{SIZE, day_view, shown};
use super::*;
use crate::calendar::Pick;

/// Selects `item` with a click, then presses Delete.
fn select_and_delete(ctx: &egui::Context, a: &mut MailApp, item: &AgendaItem) {
    click_at(ctx, a, chip_rect(ctx, item).center(), 1, SIZE);
    assert_eq!(a.ui.cal_pick, Some(Pick::of(item)), "selected");
    press_delete(ctx, a);
}

fn press_delete(ctx: &egui::Context, a: &mut MailApp) {
    frame(ctx, a, vec![key(Key::Delete, Modifiers::NONE)]);
    for _ in 0..3 {
        frame(ctx, a, vec![]);
    }
}

/// A click on the button saying `text` (in the dialog open).
fn press(ctx: &egui::Context, a: &mut MailApp, text: &str) {
    let at = settled_text_at(ctx, a, text, SIZE).unwrap_or_else(|| panic!("no {text} shown"));
    click_at(ctx, a, at, 1, SIZE);
}

/// The messages in the Inbox (Delete in the Calendar must leave Mail's selection alone).
fn inbox(a: &MailApp) -> usize {
    a.session.store.as_ref().and_then(|s| s.load_folder("sample", "INBOX").ok()).map_or(0, |f| f.messages.len())
}

fn today_has(a: &MailApp, summary: &str) -> bool {
    let (d0, d1) = a.session.cal_days();
    a.session.agenda(a.session.local_midnight(d0), a.session.local_midnight(d1)).iter().any(|i| i.summary == summary)
}

/// Delete with an appointment selected deletes it, and nothing else; the selection goes.
#[test]
fn delete_deletes_the_appointment_selected() {
    let (mut a, dir, ctx) = day_view("delete-single", 0);
    // A message selected in Mail, before going to the Calendar.
    a.run("view.module", json!({"name": "mail"}));
    let k = a.session.list.first().map(|m| m.key.clone()).expect("a message in the Inbox");
    a.run("list.open", json!({"key": k.to_id()}));
    a.run("view.module", json!({"name": "calendar"}));
    for _ in 0..5 {
        frame(&ctx, &mut a, vec![]);
    }
    assert!(!a.session.selection.is_empty(), "a message still selected in Mail");
    let lunch = shown(&a, "Lunch with Jun");
    let mail = inbox(&a);
    select_and_delete(&ctx, &mut a, &lunch);
    assert!(a.ui.dialog.is_none(), "no question for a single appointment");
    assert!(a.session.find_event(&lunch.uid).is_none(), "it's gone");
    assert_eq!(a.ui.cal_pick, None, "the selection cleared");
    assert_eq!(inbox(&a), mail, "no message deleted");
    let _ = std::fs::remove_dir_all(dir);
}

/// Delete on a series asks which: this occurrence goes, the series stays.
#[test]
fn delete_on_a_series_asks_which() {
    let (mut a, dir, ctx) = day_view("delete-series", 0);
    let standup = shown(&a, "Team stand-up");
    assert!(standup.recurring);
    select_and_delete(&ctx, &mut a, &standup);
    assert!(a.ui.dialog.is_some(), "the question shows");
    assert!(today_has(&a, "Team stand-up"), "nothing deleted before the answer");
    press(&ctx, &mut a, "Delete this occurrence");
    assert!(a.ui.dialog.is_none(), "the question answered");
    assert!(!today_has(&a, "Team stand-up"), "this one is gone");
    let week = a.session.agenda(standup.start - 7 * 86_400, standup.start + 7 * 86_400);
    assert!(week.iter().any(|i| i.summary == "Team stand-up"), "the series stays");
    assert_eq!(a.ui.cal_pick, None, "the selection cleared");
    let _ = std::fs::remove_dir_all(dir);
}

/// Cancel on the question deletes nothing.
#[test]
fn cancel_on_the_series_question_deletes_nothing() {
    let (mut a, dir, ctx) = day_view("delete-series-cancel", 0);
    let standup = shown(&a, "Team stand-up");
    select_and_delete(&ctx, &mut a, &standup);
    assert!(a.ui.dialog.is_some(), "the question shows");
    press(&ctx, &mut a, "Cancel");
    assert!(a.ui.dialog.is_none(), "the question gone");
    assert!(today_has(&a, "Team stand-up"), "nothing deleted");
    let _ = std::fs::remove_dir_all(dir);
}

/// In a colleague's calendar, which can't be changed, Delete does nothing.
#[test]
fn delete_in_a_read_only_calendar_does_nothing() {
    let (mut a, dir, ctx) = day_view("delete-read-only", 0);
    let zoe = quatrelle_engine::model::pim::shared_calendar_id("zoe@studio.example");
    a.run("calendar.show", json!({"account": "sample", "calendar": zoe, "shown": true}));
    for _ in 0..5 {
        frame(&ctx, &mut a, vec![]);
    }
    let call = shown(&a, "Producers' call");
    assert!(a.session.pim["sample"].calendars.iter().any(|c| c.id == call.calendar && c.read_only));
    select_and_delete(&ctx, &mut a, &call);
    assert!(a.ui.dialog.is_none(), "no question");
    assert!(today_has(&a, "Producers' call"), "nothing deleted");
    let _ = std::fs::remove_dir_all(dir);
}

/// Delete typed in a text box (the calendar's search) deletes no appointment.
#[test]
fn delete_in_a_text_box_does_nothing() {
    let (mut a, dir, ctx) = day_view("delete-typing", 0);
    let lunch = shown(&a, "Lunch with Jun");
    click_at(&ctx, &mut a, chip_rect(&ctx, &lunch).center(), 1, SIZE);
    assert_eq!(a.ui.cal_pick, Some(Pick::of(&lunch)));
    a.ui.focus_search = true;
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    assert!(ctx.text_edit_focused(), "the search box has the keyboard");
    press_delete(&ctx, &mut a);
    assert!(a.session.find_event(&lunch.uid).is_some(), "nothing deleted");
    let _ = std::fs::remove_dir_all(dir);
}
