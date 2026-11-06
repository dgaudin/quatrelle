// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What a task brings in besides its subject, notes, due day and whether it's done (R23): its
//! categories, its importance, whether it's private (and which kind), and its reminder, as the
//! fork's tasks keep them (MS-OXOTASK, MS-OXORMDR); its start, status, how far along it is and the
//! day it was done (R25).

use outlook_pst::UnicodePstValue as V;
use outlook_pst::messaging::named_prop::PS_PUBLIC_STRINGS;
use quatrelle_model::ical::PrivateKind;
use quatrelle_model::pim::TaskStatus;

use super::*;

const TASK: GuidValue = GuidValue::new(0x0006_2003, 0, 0, OLE);

/// 2026-11-24 at midnight UTC: a task's due day, as Outlook writes it (its local midnight).
const DUE: i64 = 1_795_478_400;

/// A categorized, important, private task with a reminder, one of low importance kept
/// confidential whose reminder is off, and one done whose reminder went with it.
#[test]
fn a_task_keeps_its_categories_importance_privacy_and_reminder() {
    let names = ["Print", "Studio"];
    let reminder = DUE - 2 * 86_400 + 9 * 3600;
    let snoozed = reminder + 1800;
    let task = |importance: i32, sensitivity: i32, set: bool, done: bool| {
        vec![
            named(TASK, 0x8105, V::Time(filetime(DUE))),
            named(TASK, 0x811C, V::Boolean(done)),
            UnicodePstProperty::tagged(0x0017, V::Integer32(importance)),
            UnicodePstProperty::tagged(0x0036, V::Integer32(sensitivity)),
            UnicodePstProperty::named(PS_PUBLIC_STRINGS, UnicodePstName::String("Keywords"), V::MultipleUnicode(&names)),
            named(COMMON, 0x8503, V::Boolean(set)),
            named(COMMON, 0x8502, V::Time(filetime(reminder))),
            named(COMMON, 0x8560, V::Time(filetime(snoozed))),
        ]
    };
    let (a, b, c) = (task(2, 2, true, false), task(0, 3, false, false), task(1, 1, true, true));
    let brought = Brought::of(
        "tasks",
        &[
            Put::new(&["Tasks"], "IPF.Task", item("IPM.Task", "Send the proofs", "Both sizes.", &a)),
            Put::new(&["Tasks"], "IPF.Task", item("IPM.Task", "Book the stage", "", &b)),
            Put::new(&["Tasks"], "IPF.Task", item("IPM.Task", "Pay the printer", "", &c)),
        ],
    );
    assert_eq!(brought.done.tasks, 3, "{:?}", brought.done);
    let pim = brought.pim();
    let t = |s: &str| pim.tasks.iter().map(|t| &t.task).find(|t| t.subject == s).unwrap().clone();
    let proofs = t("Send the proofs");
    assert_eq!(proofs.categories, ["Print", "Studio"]);
    assert_eq!((proofs.importance, proofs.private, proofs.private_kind), (1, true, PrivateKind::Private));
    assert_eq!(proofs.reminder, Some(snoozed), "the reminder comes when it was put off to");
    assert_eq!((proofs.due, proofs.notes.as_str()), (Some(DUE.div_euclid(86_400)), "Both sizes."));
    let stage = t("Book the stage");
    assert_eq!((stage.importance, stage.private, stage.private_kind, stage.reminder), (-1, true, PrivateKind::Confidential, None));
    let paid = t("Pay the printer");
    assert!(paid.is_complete());
    assert_eq!((paid.importance, paid.private_kind, paid.reminder), (0, PrivateKind::Personal, None), "a task done reminds no more");
    assert_eq!(paid.categories, ["Print", "Studio"]);
}

/// A reminder set without its time is none (a task has no start to count from), and one at a
/// time no calendar has is none either.
#[test]
fn a_task_s_reminder_needs_its_time() {
    let without = [named(TASK, 0x8105, V::Time(filetime(DUE))), named(COMMON, 0x8503, V::Boolean(true))];
    let absurd = [named(COMMON, 0x8503, V::Boolean(true)), named(COMMON, 0x8502, V::Time(i64::MAX))];
    let brought = Brought::of(
        "task-reminders",
        &[
            Put::new(&["Tasks"], "IPF.Task", item("IPM.Task", "No time", "", &without)),
            Put::new(&["Tasks"], "IPF.Task", item("IPM.Task", "Absurd", "", &absurd)),
        ],
    );
    let pim = brought.pim();
    assert_eq!(pim.tasks.len(), 2);
    assert!(pim.tasks.iter().all(|t| t.task.reminder.is_none() && !t.task.private && t.task.importance == 0), "{:?}", pim.tasks);
}

/// A task begun, 40% along and in progress, one waiting on someone else, and one done on a day of
/// its own (R25): their start, status, progress and the day they were done come in.
#[test]
fn a_task_keeps_its_start_status_progress_and_day_done() {
    let begun = DUE - 7 * 86_400;
    let done_on = DUE - 3 * 86_400;
    let started = [
        named(TASK, 0x8105, V::Time(filetime(DUE))),
        named(TASK, 0x8104, V::Time(filetime(begun))),
        named(TASK, 0x8101, V::Integer32(1)),
        named(TASK, 0x8102, V::Floating64(0.4)),
    ];
    let waiting = [named(TASK, 0x8101, V::Integer32(3)), named(TASK, 0x8102, V::Floating64(0.25))];
    let done = [
        named(TASK, 0x8105, V::Time(filetime(DUE))),
        named(TASK, 0x811C, V::Boolean(true)),
        named(TASK, 0x8101, V::Integer32(2)),
        named(TASK, 0x8102, V::Floating64(1.0)),
        named(TASK, 0x810F, V::Time(filetime(done_on))),
    ];
    let brought = Brought::of(
        "task-progress",
        &[
            Put::new(&["Tasks"], "IPF.Task", item("IPM.Task", "Print the posters", "", &started)),
            Put::new(&["Tasks"], "IPF.Task", item("IPM.Task", "Hear from the venue", "", &waiting)),
            Put::new(&["Tasks"], "IPF.Task", item("IPM.Task", "Send the invoice", "", &done)),
        ],
    );
    let pim = brought.pim();
    let t = |s: &str| pim.tasks.iter().map(|t| &t.task).find(|t| t.subject == s).unwrap().clone();
    let posters = t("Print the posters");
    assert_eq!((posters.start, posters.due), (Some(begun.div_euclid(86_400)), Some(DUE.div_euclid(86_400))));
    assert_eq!((posters.status, posters.percent, posters.completed), (TaskStatus::InProgress, 40, None));
    let venue = t("Hear from the venue");
    assert_eq!((venue.status, venue.percent, venue.start), (TaskStatus::WaitingOnOthers, 25, None));
    let invoice = t("Send the invoice");
    assert!(invoice.is_complete());
    assert_eq!((invoice.percent, invoice.completed), (100, Some(done_on.div_euclid(86_400))), "done that day, not the day it was due");
}

/// A task that repeats every Monday, ten times (R25): the fork's tasks don't repeat, so its rule
/// is kept with it, as the data file had it, and nothing else changes.
#[test]
fn a_repeating_task_keeps_how_it_repeats() {
    use quatrelle_model::dates::days_from_civil;
    // Mondays from 2026-11-23, a task's pattern: no appointment part after its end date.
    let first = crate::pst_pim::tests::minutes(2026, 11, 23);
    let mut pattern = crate::pst_pim::tests::blob(0x200B, 0x0001, 1, &[0b10], (0x2022, 10), &[], &[], first);
    pattern.truncate(pattern.len() - 3 * 4);
    let start = days_from_civil(2026, 11, 23) * 86_400;
    let weekly = [
        named(TASK, 0x8104, V::Time(filetime(start))),
        named(TASK, 0x8105, V::Time(filetime(start))),
        named(TASK, 0x8116, V::Binary(&pattern)),
        named(TASK, 0x8126, V::Boolean(true)),
    ];
    let brought = Brought::of("task-recurrence", &[Put::new(&["Tasks"], "IPF.Task", item("IPM.Task", "Water the plants", "", &weekly))]);
    let pim = brought.pim();
    let t = &pim.tasks.first().unwrap().task;
    assert_eq!(t.rrule.as_deref(), Some("FREQ=WEEKLY;INTERVAL=1;BYDAY=MO;COUNT=10"));
    assert_eq!((t.start, t.due, t.status), (Some(start / 86_400), Some(start / 86_400), TaskStatus::NotStarted));
}
