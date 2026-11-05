// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Reminders answered in Quatrelle, told to Exchange Online (issue #40): Graph's
//! `dismissReminder` and `snoozeReminder` on the event, or on the one occurrence of a series
//! (found among the series' `instances` around its start), so Outlook doesn't show them again.
//! A single appointment's dismissal isn't among them: the engine turns its reminder off on the
//! event (`isReminderOn`), as Outlook does.

use quatrelle_model::pim::{MAX_REMINDER_ANSWERS, ReminderAnswer};

use super::*;

/// How far from the occurrence's start its instance is looked for: a day either way, for an
/// all-day occurrence kept as its local day.
const AROUND: i64 = 86_400;
/// The most instances asked for around it.
const MAX_INSTANCES: usize = 50;

/// Graph's date-time in a query (`2026-11-03T10:00:00Z`).
fn query_time(t: i64) -> String {
    let (y, m, d) = quatrelle_model::dates::civil_from_days(t.div_euclid(86_400));
    let s = t.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", s / 3600, s / 60 % 60, s % 60)
}

/// The event to answer: the item itself, or the series' occurrence nearest its start.
fn item(c: &Client, a: &ReminderAnswer) -> Result<String> {
    if !a.series {
        return Ok(a.href.clone());
    }
    let path = format!(
        "/me/events/{}/instances?startDateTime={}&endDateTime={}&$select=id,start&$top={MAX_INSTANCES}",
        seg(&a.href),
        query_time(a.occurrence.saturating_sub(AROUND)),
        query_time(a.occurrence.saturating_add(AROUND))
    );
    let v = c.get(&path)?;
    let list = v.get("value").and_then(Value::as_array).cloned().unwrap_or_default();
    list.iter()
        .take(MAX_INSTANCES)
        .filter_map(|i| Some((when(i.get("start"))?, s(i, "id"))))
        .filter(|(start, id)| !id.is_empty() && start.abs_diff(a.occurrence) <= AROUND.unsigned_abs())
        .min_by_key(|(start, _)| start.abs_diff(a.occurrence))
        .map(|(_, id)| id)
        .ok_or_else(|| NetError::No("that occurrence isn't in the calendar any more".into()))
}

/// Tell Graph one answer.
fn answer(c: &Client, a: &ReminderAnswer) -> Result<()> {
    let id = item(c, a)?;
    match a.snooze_until {
        None => c.post(&format!("/me/events/{}/dismissReminder", seg(&id)), &json!({})),
        Some(until) => c.post(&format!("/me/events/{}/snoozeReminder", seg(&id)), &json!({ "newReminderTime": date_time(until) })),
    }
    .map(|_| ())
}

/// Every answer waiting: those the server took, or that no longer apply there (the event is
/// gone), are said answered; the rest (out of reach) wait for the next sync.
pub(super) fn answer_all(c: &Client, pim: &Pim, out: &mut DavSync) {
    for a in pim.reminder_answers.iter().take(MAX_REMINDER_ANSWERS) {
        match answer(c, a) {
            Ok(()) => out.answered.push(a.clone()),
            Err(NetError::No(why) | NetError::Gone(why)) => {
                log::info!("a reminder answer can't be told to Exchange Online: {why}");
                out.answered.push(a.clone());
            }
            // Out of reach: the others wait too, rather than each failing in turn.
            Err(e) => {
                out.errors.push(format!("a reminder: {e}"));
                break;
            }
        }
    }
}
