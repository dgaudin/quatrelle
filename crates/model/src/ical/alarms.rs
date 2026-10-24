//! Reminders: an event's alarms (`VALARM`) read as minutes before it starts, and set.

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
