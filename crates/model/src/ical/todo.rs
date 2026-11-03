// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A task as a calendar file (`VTODO`, RFC 5545 §3.6.2): the copy of one kept when the server
//! no longer had it (Sync Issues › Conflicts).

use super::{day, escape, fold, stamp};
use crate::pim::{Task, TaskStatus};

/// A calendar file published with this one task, stamped `now`. Its days (start, due,
/// completed) are written as dates.
pub fn write_task(t: &Task, now: i64) -> String {
    let mut lines: Vec<String> = vec![
        "BEGIN:VCALENDAR".into(),
        "VERSION:2.0".into(),
        "PRODID:-//Quatrelle//Quatrelle//EN".into(),
        "METHOD:PUBLISH".into(),
        "BEGIN:VTODO".into(),
        format!("UID:{}", t.uid.replace(['\r', '\n'], "")),
        format!("DTSTAMP:{}", stamp(now)),
        format!("SUMMARY:{}", escape(&t.subject)),
    ];
    if !t.notes.is_empty() {
        lines.push(format!("DESCRIPTION:{}", escape(&t.notes)));
    }
    let date = |d: i64| day(d.saturating_mul(86_400));
    if let Some(d) = t.start {
        lines.push(format!("DTSTART;VALUE=DATE:{}", date(d)));
    }
    if let Some(d) = t.due {
        lines.push(format!("DUE;VALUE=DATE:{}", date(d)));
    }
    if let Some(d) = t.completed {
        lines.push(format!("COMPLETED:{}", stamp(d.saturating_mul(86_400))));
    }
    let status = match t.status {
        TaskStatus::Completed => "COMPLETED",
        TaskStatus::InProgress => "IN-PROCESS",
        _ => "NEEDS-ACTION",
    };
    lines.push(format!("STATUS:{status}"));
    lines.push(format!("PERCENT-COMPLETE:{}", t.percent.min(100)));
    // RFC 5545's priorities: 1 the highest, 9 the lowest, 5 the middle.
    lines.push(format!(
        "PRIORITY:{}",
        match t.importance.signum() {
            1 => 1,
            -1 => 9,
            _ => 5,
        }
    ));
    if !t.categories.is_empty() {
        lines.push(format!("CATEGORIES:{}", t.categories.iter().map(|c| escape(c)).collect::<Vec<_>>().join(",")));
    }
    if t.private {
        lines.push("CLASS:PRIVATE".into());
    }
    lines.push("END:VTODO".into());
    lines.push("END:VCALENDAR".into());
    let mut out = String::new();
    for l in lines {
        fold(&l, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_task_is_written_as_a_vtodo() {
        let t = Task {
            uid: "task:T1".into(),
            subject: "Devis, Martin".into(),
            notes: "Relancer\nvendredi".into(),
            due: Some(crate::dates::days_from_civil(2026, 10, 27)),
            importance: 1,
            ..Default::default()
        };
        let ics = write_task(&t, 1_792_000_000);
        for line in [
            "METHOD:PUBLISH",
            "BEGIN:VTODO",
            "UID:task:T1",
            "SUMMARY:Devis\\, Martin",
            "DESCRIPTION:Relancer\\nvendredi",
            "DUE;VALUE=DATE:20261027",
            "PRIORITY:1",
        ] {
            assert!(ics.lines().any(|l| l == line), "{line} in {ics}");
        }
    }
}
