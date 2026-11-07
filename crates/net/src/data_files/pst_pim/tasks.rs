// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A task (`IPM.Task`, MS-OXOTASK) as the fork's tasks keep one: its subject and notes, the days
//! it starts and is due, where it stands and how far along it is, the day it was done, its
//! priority (the item's importance), categories, privacy and reminder (R23, R25), read as
//! `712d7336` of the project it was forked from reads them, into the fork's own task. How it
//! repeats (PidLidTaskRecurrence, the RecurrencePattern a repeating appointment's starts with) is
//! read into an RFC 5545 rule and kept with it: the fork's tasks don't repeat, so it isn't shown.
//! A rule no pattern says, which an export keeps in the task's text (`rest`), comes back from there.

use quatrelle_model::pim::{Task, TaskStatus};

use super::*;

/// A task's day: its local midnight as Outlook writes it, the nearest UTC midnight.
fn day(t: i64) -> i64 {
    t.saturating_add(43_200).div_euclid(86_400)
}

/// Where it stands, by PidLidTaskStatus's number.
fn status(n: i64) -> Option<TaskStatus> {
    Some(match n {
        0 => TaskStatus::NotStarted,
        1 => TaskStatus::InProgress,
        2 => TaskStatus::Completed,
        3 => TaskStatus::WaitingOnOthers,
        4 => TaskStatus::Deferred,
        _ => return None,
    })
}

/// The task, `importance` its item's (-1 low, 0 normal, 1 high), `date` when the item was (its day
/// done when the task is done and says no day of its own, nor a due day).
pub(crate) fn task_of(x: &Extra, subject: &str, body: &str, importance: i8, date: i64, uid: String) -> Task {
    let due = x.due.map(day);
    let start = x.task_start.map(day);
    let mut status = x.task_status.and_then(status).unwrap_or_default();
    if x.done {
        status = TaskStatus::Completed;
    }
    let (notes, rest) = split_rest(&body.replace("\r\n", "\n"), "VTODO");
    let said = rest.iter().find_map(|l| l.get(..6).filter(|h| h.eq_ignore_ascii_case("RRULE:")).and_then(|_| l.get(6..))).map(str::trim);
    // The pattern counts from the task's first day: its start, else the day it's due.
    let from = start.or(due).unwrap_or(0).saturating_mul(86_400);
    let rrule = match said.filter(|r| !r.is_empty()) {
        Some(rule) => Some(rule.to_string()),
        None => (!x.task_recurrence.is_empty()).then(|| recur::pattern(&x.task_recurrence, from, "").map(|p| p.rule)).flatten(),
    };
    let mut task = Task {
        uid,
        subject: subject.to_string(),
        notes,
        // A start after the day it's due isn't one Outlook keeps: the due day stands.
        start: match (start, due) {
            (Some(s), Some(d)) if s > d => Some(d),
            (s, _) => s,
        },
        due,
        status,
        percent: x.percent.unwrap_or(0),
        importance,
        reminder: x.reminder_at,
        categories: x.categories.clone(),
        private: x.sensitivity != 0,
        private_kind: private_kind(x.sensitivity),
        rrule,
        modified: x.modified.unwrap_or(date),
        ..Default::default()
    };
    // Its status and progress in step, as Outlook keeps them; one done, on its day (else, as
    // before, the day it was due); its reminder done with.
    let done_on = x.completed.map(day).or(due).unwrap_or(date.div_euclid(86_400));
    task.settle(false, done_on);
    task
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tasks_numbers_are_kept_in_step() {
        let x = |status: Option<i64>, percent: Option<u8>, done: bool| Extra { task_status: status, percent, done, ..Default::default() };
        let t = task_of(&x(Some(0), Some(40), false), "a", "", 0, 0, "u".into());
        assert_eq!((t.status, t.percent), (TaskStatus::NotStarted, 0), "not started is 0%");
        let t = task_of(&x(Some(4), Some(10), false), "a", "", 0, 0, "u".into());
        assert_eq!((t.status, t.percent), (TaskStatus::Deferred, 10));
        let t = task_of(&x(Some(9), None, true), "a", "", 0, 86_400 * 3 + 5, "u".into());
        assert_eq!((t.status, t.percent, t.completed), (TaskStatus::Completed, 100, Some(3)), "done, on the item's day");
        let late = Extra { task_start: Some(10 * 86_400), due: Some(5 * 86_400), ..Default::default() };
        assert_eq!(task_of(&late, "a", "", 0, 0, "u".into()).start, Some(5));
        let none = task_of(&Extra { task_recurrence: vec![1, 2, 3], ..Default::default() }, "a", "", 0, 0, "u".into());
        assert_eq!(none.rrule, None, "a damaged pattern is none");
    }
}
