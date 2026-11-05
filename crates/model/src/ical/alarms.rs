// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Reminders: an event's alarms (`VALARM`) read as minutes before it starts, and set; and
//! dismissed, as RFC 9074 writes it into the alarm (`ACKNOWLEDGED`).

use super::*;

/// The longest reminder taken (four weeks before), as Outlook's own list goes.
pub const REMINDER_MAX_MINUTES: i64 = 4 * 7 * 24 * 60;

/// An event's reminder, in minutes before it starts: its own (`reminder`, what the parser and
/// the servers' readers fill in), else its first alarm (`VALARM`) still among its extra lines
/// with a trigger relative to the start (`TRIGGER:-PT15M`, `TRIGGER;RELATED=START:-P1D`) or at
/// a set time (`TRIGGER;VALUE=DATE-TIME:…Z`). `None`: no reminder. A trigger after the start
/// counts as "at the start".
pub fn reminder_minutes(e: &CalEvent) -> Option<i64> {
    if let Some(m) = e.reminder {
        return Some(m.clamp(0, REMINDER_MAX_MINUTES));
    }
    let mut in_alarm = false;
    for raw in e.extra.iter().take(400) {
        let line = raw.trim();
        let upper = line.to_ascii_uppercase();
        if upper == "BEGIN:VALARM" {
            in_alarm = true;
            continue;
        }
        if upper == "END:VALARM" {
            in_alarm = false;
            continue;
        }
        if !in_alarm || !upper.starts_with("TRIGGER") {
            continue;
        }
        let Some((params, value)) = line.split_once(':') else { continue };
        let params = params.to_ascii_uppercase();
        if params.contains("RELATED=END") {
            continue;
        }
        // One that can't be read is passed over for the next.
        let before = if params.contains("VALUE=DATE-TIME") {
            let Some((t, _, _)) = date_time(value) else { continue };
            e.start.saturating_sub(t)
        } else {
            let Some(d) = duration(value) else { continue };
            d.saturating_neg()
        };
        return Some((before / 60).clamp(0, REMINDER_MAX_MINUTES));
    }
    None
}

/// Give an event one reminder `minutes` before it starts, in place of any it had, or none
/// (`None`). It's the event's own field (written out as one display alarm); any alarm still
/// among its extra lines goes, so it isn't written twice.
pub fn set_reminder(e: &mut CalEvent, minutes: Option<i64>) {
    e.reminder = minutes.map(|m| m.clamp(0, REMINDER_MAX_MINUTES));
    // A reminder set again rings again.
    e.acknowledged = None;
    // Every alarm goes, nested lines included.
    let mut kept = Vec::with_capacity(e.extra.len());
    let mut depth = 0usize;
    for raw in std::mem::take(&mut e.extra) {
        let upper = raw.trim().to_ascii_uppercase();
        if upper == "BEGIN:VALARM" {
            depth += 1;
            continue;
        }
        if depth > 0 {
            if upper == "END:VALARM" {
                depth -= 1;
            }
            continue;
        }
        kept.push(raw);
    }
    e.extra = kept;
}

/// The event's own reminder as an alarm: Outlook's, a display alarm whose trigger is a plain
/// duration from the start; dismissed, it says when (`ACKNOWLEDGED`, RFC 9074).
pub(super) fn alarm_lines(e: &CalEvent) -> Vec<String> {
    let Some(m) = e.reminder else { return Vec::new() };
    let mut lines = vec![
        "BEGIN:VALARM".to_string(),
        "ACTION:DISPLAY".into(),
        format!("DESCRIPTION:{}", escape(if e.summary.is_empty() { "Reminder" } else { &e.summary })),
        format!("TRIGGER:{}", trigger(m)),
    ];
    if let Some(at) = e.acknowledged {
        lines.push(format!("ACKNOWLEDGED:{}", stamp(at.max(0))));
    }
    lines.push("END:VALARM".into());
    lines
}

/// The latest `ACKNOWLEDGED` among an alarm's lines (a UTC date-time, as RFC 9074 has it).
pub(super) fn acknowledged_in(lines: &[String]) -> Option<i64> {
    lines
        .iter()
        .take(400)
        .filter_map(|raw| {
            let line = raw.trim();
            let named = line.get(..13).is_some_and(|n| n.eq_ignore_ascii_case("ACKNOWLEDGED:") || n.eq_ignore_ascii_case("ACKNOWLEDGED;"));
            match line.split_once(':').and_then(|(_, v)| date_time(v)) {
                Some((t, false, true)) if named => Some(t),
                _ => None,
            }
        })
        .max()
}

/// When the event's reminder was last answered (dismissed) by a calendar program: the latest
/// `ACKNOWLEDGED` of its alarms (RFC 9074 §6), its own reminder's or one still among its extra
/// lines. An occurrence whose reminder came at or before then has been dealt with.
pub fn acknowledged(e: &CalEvent) -> Option<i64> {
    let mut in_alarm: Option<Vec<String>> = None;
    let mut latest = e.acknowledged;
    for raw in e.extra.iter().take(400) {
        let upper = raw.trim().to_ascii_uppercase();
        if upper == "BEGIN:VALARM" {
            in_alarm = Some(Vec::new());
        } else if upper == "END:VALARM" {
            latest = latest.max(in_alarm.take().and_then(|lines| acknowledged_in(&lines)));
        } else if let Some(lines) = in_alarm.as_mut() {
            lines.push(raw.clone());
        }
    }
    latest
}

/// Mark the event's reminder answered at `at` (UTC seconds), as RFC 9074 §6 says: its own
/// reminder's alarm, and each alarm still among its extra lines, gets `ACKNOWLEDGED` in place of
/// the one it had, so every calendar program reading it knows the reminder was dismissed.
/// Snooze alarms (`RELATED-TO;RELTYPE=SNOOZE`) go: they were answered too. The rest of the event
/// is left as written, and an alarm left open (a damaged event) as it was.
pub fn acknowledge(e: &mut CalEvent, at: i64) {
    let at = at.max(0);
    if e.reminder.is_some() {
        e.acknowledged = Some(at);
    }
    let stamp = format!("ACKNOWLEDGED:{}", stamp(at));
    let mut out = Vec::with_capacity(e.extra.len().saturating_add(2));
    let mut alarm: Option<Vec<String>> = None;
    for raw in std::mem::take(&mut e.extra) {
        let upper = raw.trim().to_ascii_uppercase();
        if upper == "BEGIN:VALARM" && alarm.is_none() {
            alarm = Some(vec![raw]);
            continue;
        }
        let Some(lines) = alarm.as_mut() else {
            out.push(raw);
            continue;
        };
        if upper == "END:VALARM" {
            let snooze = lines.iter().any(|l| {
                let u = l.to_ascii_uppercase();
                u.starts_with("RELATED-TO") && u.contains("RELTYPE=SNOOZE")
            });
            if !snooze {
                out.append(lines);
                out.push(stamp.clone());
                out.push(raw);
            }
            alarm = None;
            continue;
        }
        if !(upper.starts_with("ACKNOWLEDGED:") || upper.starts_with("ACKNOWLEDGED;")) {
            lines.push(raw);
        }
    }
    if let Some(lines) = alarm {
        out.extend(lines);
    }
    e.extra = out;
}
