// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! A task as an Outlook task (`IPM.Task`, MS-OXOTASK 2.2.2): its subject and notes, status, how
//! far along it is, its start, due and done days, its priority (the item's importance), privacy,
//! reminder and categories, and how it repeats (PidLidTaskRecurrence, a RecurrencePattern:
//! [`super::pattern`]), as `pst_pim::tasks` reads them. A rule no pattern says the same as stays
//! in its text as a to-do of its own (`pst_pim::rest`), which the import reads back.

use quatrelle_model::ical::PrivateKind;
use quatrelle_model::pim::{Task, TaskStatus};

use super::item::{PSETID_COMMON, PSETID_TASK, filetime};
use super::pim::{Item, Value, crlf, keywords, lid, reminder, tag};

/// A task's day (days since 1970) as Outlook keeps one: its midnight, in UTC.
fn day(d: i64) -> i64 {
    filetime(d.saturating_mul(86_400))
}

/// The task as an item; `now` stands in for when it last changed when it doesn't say.
pub(super) fn item(t: &Task, now: i64) -> Item {
    let status = TaskStatus::ALL.iter().position(|s| *s == t.status).and_then(|i| i32::try_from(i).ok()).unwrap_or(0);
    let mut props = vec![
        lid(PSETID_TASK, 0x8101, Value::Int(status)),
        lid(PSETID_TASK, 0x8102, Value::Float(f64::from(t.percent.min(100)) / 100.0)),
        lid(PSETID_TASK, 0x811C, Value::Bool(t.is_complete())),
        // PidLidTaskMode: a task of the user's own, not assigned.
        lid(PSETID_COMMON, 0x8518, Value::Int(0)),
        tag(0x0017, Value::Int(i32::from(t.importance.clamp(-1, 1)) + 1)),
    ];
    // Its start and due days, also as PidLidCommonStart and PidLidCommonEnd, which Outlook sorts by.
    for (d, own, common) in [(t.start, 0x8104, 0x8516), (t.due, 0x8105, 0x8517)] {
        if let Some(d) = d {
            props.extend([lid(PSETID_TASK, own, Value::Time(day(d))), lid(PSETID_COMMON, common, Value::Time(day(d)))]);
        }
    }
    if let Some(done) = t.completed.filter(|_| t.is_complete()) {
        props.push(lid(PSETID_TASK, 0x810F, Value::Time(day(done))));
    }
    if let Some(at) = t.reminder {
        props.extend(reminder(at));
    }
    if t.private {
        let sensitivity = match t.private_kind {
            PrivateKind::Personal => 1,
            PrivateKind::Private => 2,
            PrivateKind::Confidential => 3,
        };
        props.push(tag(0x0036, Value::Int(sensitivity)));
    }
    props.extend(keywords(&t.categories));
    let mut rest: Vec<String> = Vec::new();
    if let Some(rule) = t.rrule.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
        // The pattern counts from the task's first day, as the import reads it.
        let from = t.start.or(t.due).map(|d| d.saturating_mul(86_400));
        match from.and_then(|from| super::pattern::write(rule, from).map(|p| (from, p))) {
            Some((from, p)) => {
                if !super::pattern::says(&p, from, rule) {
                    rest.push(format!("RRULE:{rule}"));
                }
                props.extend([lid(PSETID_TASK, 0x8116, Value::Bin(p)), lid(PSETID_TASK, 0x8126, Value::Bool(true))]);
            }
            None => rest.push(format!("RRULE:{rule}")),
        }
    }
    Item {
        class: "IPM.Task",
        subject: t.subject.clone(),
        body: crlf(&crate::pst_pim::with_rest(&t.notes, "VTODO", &rest)),
        modified: filetime(if t.modified > 0 { t.modified } else { now }),
        props,
        ..Default::default()
    }
}
