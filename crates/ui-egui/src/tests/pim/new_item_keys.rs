// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: Ctrl+N makes the shown module's new item, as Outlook's: a message in Mail, an
//! appointment in the Calendar (at the half hour or the day selected), a contact in People, a
//! task in Tasks, a note in Notes.

use quatrelle_engine::model::dates::civil_from_days;

use super::calendar_menu::day_view;
use super::*;
use crate::calendar::Pick;
use crate::dialogs::pim_dialogs::EventForm;

fn ctrl_n(ctx: &egui::Context, a: &mut MailApp) {
    frame(ctx, a, vec![key(Key::N, Modifiers::COMMAND)]);
    for _ in 0..3 {
        frame(ctx, a, vec![]);
    }
}

/// The sample, `module` shown.
fn in_module(name: &str, module: &str) -> (MailApp, std::path::PathBuf, egui::Context) {
    let (mut a, dir, ctx) = day_view(name, 0);
    a.run("view.module", json!({"name": module}));
    for _ in 0..5 {
        frame(&ctx, &mut a, vec![]);
    }
    (a, dir, ctx)
}

/// The new appointment opened (as a dialog, or in a window of its own).
fn new_event(a: &MailApp) -> Option<&EventForm> {
    super::calendar_clicks::form(a).filter(|f| f.uid.is_none())
}

fn ymd(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

#[test]
fn ctrl_n_in_mail_writes_a_message() {
    let (mut a, dir, ctx) = in_module("new-mail", "mail");
    ctrl_n(&ctx, &mut a);
    assert!(a.session.compose.is_some(), "a new message");
    let _ = std::fs::remove_dir_all(dir);
}

/// Nothing selected: the appointment the ribbon's New event opens.
#[test]
fn ctrl_n_in_the_calendar_opens_a_new_appointment() {
    let (mut a, dir, ctx) = day_view("new-event", 0);
    let ribbon = EventForm::new(&a.session, None);
    ctrl_n(&ctx, &mut a);
    let f = new_event(&a).expect("a new appointment");
    assert!(a.session.compose.is_none(), "no message");
    assert_eq!((f.date.as_str(), f.start.as_str()), (ribbon.date.as_str(), ribbon.start.as_str()));
    let _ = std::fs::remove_dir_all(dir);
}

/// A half hour selected: the new appointment starts there.
#[test]
fn ctrl_n_on_a_selected_half_hour_starts_there() {
    let (mut a, dir, ctx) = day_view("new-event-slot", 0);
    let day = a.session.cal_days().0;
    a.ui.cal_pick = Some(Pick::Slot { day, half: 29, pane: 0 });
    ctrl_n(&ctx, &mut a);
    let f = new_event(&a).expect("a new appointment");
    assert_eq!((f.date.clone(), f.start.as_str()), (ymd(day), "14:30"));
    let _ = std::fs::remove_dir_all(dir);
}

/// A day selected in the Month view: the new appointment is that day, at 9:00.
#[test]
fn ctrl_n_on_a_selected_day_is_that_day() {
    let (mut a, dir, ctx) = day_view("new-event-day", 0);
    a.run("calendar.view", json!({"mode": "month"}));
    let day = a.session.cal_days().0 + 9;
    a.ui.cal_pick = Some(Pick::Day { day });
    ctrl_n(&ctx, &mut a);
    let f = new_event(&a).expect("a new appointment");
    assert_eq!((f.date.clone(), f.start.as_str()), (ymd(day), "09:00"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ctrl_n_in_people_opens_a_new_contact() {
    let (mut a, dir, ctx) = in_module("new-contact", "people");
    ctrl_n(&ctx, &mut a);
    assert!(matches!(&a.ui.dialog, Some(Dialog::Contact(_))), "a new contact");
    assert!(a.session.compose.is_none(), "no message");
    let _ = std::fs::remove_dir_all(dir);
}

/// In Tasks, the box to type a new task in takes the keyboard (as New task does).
#[test]
fn ctrl_n_in_tasks_starts_a_new_task() {
    let (mut a, dir, ctx) = in_module("new-task", "tasks");
    assert!(!ctx.text_edit_focused());
    ctrl_n(&ctx, &mut a);
    assert!(ctx.text_edit_focused(), "the new task's box has the keyboard");
    assert!(a.session.compose.is_none(), "no message");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ctrl_n_in_notes_starts_a_new_note() {
    let (mut a, dir, ctx) = in_module("new-note", "notes");
    let before = a.session.notes("").len();
    ctrl_n(&ctx, &mut a);
    assert_eq!(a.session.notes("").len(), before + 1, "a new note");
    assert_eq!(a.ui.note_windows.len(), 1, "its window open");
    assert!(ctx.text_edit_focused(), "written in at once");
    assert!(a.session.compose.is_none(), "no message");
    let _ = std::fs::remove_dir_all(dir);
}

/// Ctrl+Shift+M writes a message from any module, as Outlook's.
#[test]
fn ctrl_shift_m_writes_a_message_from_the_calendar() {
    let (mut a, dir, ctx) = day_view("new-mail-anywhere", 0);
    frame(&ctx, &mut a, vec![key(Key::M, Modifiers::COMMAND | Modifiers::SHIFT)]);
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    assert!(a.session.compose.is_some(), "a new message");
    let _ = std::fs::remove_dir_all(dir);
}
