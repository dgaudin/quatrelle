// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Reminders answered in Quatrelle, told to Exchange (issue #40), as Outlook in Cached Exchange
//! Mode does at its next sync: EWS's `GetReminders` finds the server's reminder of a series'
//! occurrence (or of a single appointment snoozed), and `PerformReminderAction` dismisses or
//! snoozes it (Exchange 2013 and later), so Outlook, Outlook on the web and the phone don't show
//! it again. A single appointment's dismissal isn't among them: the engine turns its reminder
//! off on the item (`ReminderIsSet`), as Outlook does, which every version takes. An older
//! server doesn't know the operations: it isn't asked, and those answers stay on this computer.

use quatrelle_model::ical::REMINDER_MAX_MINUTES;
use quatrelle_model::pim::{MAX_REMINDER_ANSWERS, ReminderAnswer};

use super::*;
use crate::ews::{Listed, MAX_REMINDERS, check_message, reminders_listed};

/// How far from the occurrence's start its reminder is looked for: a day either way (an all-day
/// occurrence is kept as its local day).
const AROUND: i64 = 86_400;

/// The server's reminder for the answer: the event's own, or its series' occurrence that starts
/// nearest the answer's, within a day.
fn pick<'a>(listed: &'a [Listed], a: &ReminderAnswer) -> Option<&'a Listed> {
    listed
        .iter()
        .filter(|r| (if a.series { r.master == a.href } else { r.item == a.href }) || (!r.uid.is_empty() && r.uid == a.uid))
        .filter(|r| r.start.abs_diff(a.occurrence) <= AROUND.unsigned_abs())
        .min_by_key(|r| r.start.abs_diff(a.occurrence))
}

/// The reminders the server lists around `at`, from four weeks and a day before (the longest
/// reminder) to a day after.
fn listed_around(c: &Client, at: i64) -> Result<Vec<Listed>> {
    let from = at.saturating_sub(REMINDER_MAX_MINUTES * 60).saturating_sub(AROUND);
    let doc = c.call_2013(&format!(
        "<m:GetReminders><m:BeginTime>{}</m:BeginTime><m:EndTime>{}</m:EndTime><m:MaxItems>{MAX_REMINDERS}</m:MaxItems>\
         <m:ReminderType>All</m:ReminderType></m:GetReminders>",
        utc(from),
        utc(at.saturating_add(AROUND))
    ))?;
    match doc.find("GetRemindersResponse") {
        Some(m) => check_message(m)?,
        None => return Err(NetError::Protocol("an empty answer".into())),
    }
    Ok(reminders_listed(&doc))
}

/// Tell Exchange one answer. `NetError::No`: it can't apply there (the server doesn't list that
/// reminder any more, or refuses the operation).
fn answer(c: &Client, a: &ReminderAnswer) -> Result<()> {
    let listed = listed_around(c, a.occurrence)?;
    let Some(r) = pick(&listed, a) else {
        return Err(NetError::No("that reminder isn't on the server any more".into()));
    };
    let (action, time) = match a.snooze_until {
        None => ("Dismiss", String::new()),
        Some(until) => ("Snooze", format!("<t:NewReminderTime>{}</t:NewReminderTime>", utc(until))),
    };
    let doc = c.call_2013(&format!(
        "<m:PerformReminderAction><m:ReminderItemActions><t:ReminderItemAction><t:ActionType>{action}</t:ActionType>\
         <t:ItemId Id=\"{}\" ChangeKey=\"{}\"/>{time}</t:ReminderItemAction></m:ReminderItemActions></m:PerformReminderAction>",
        esc(&r.item),
        esc(&r.change_key)
    ))?;
    match doc.find("PerformReminderActionResponse") {
        Some(m) => check_message(m),
        None => Err(NetError::Protocol("an empty answer".into())),
    }
}

/// Every answer waiting: those the server took, or that can't apply there, are said answered;
/// the rest (the server out of reach, or busy) wait for the next sync.
pub(super) fn answer_all(c: &Client, pim: &Pim, out: &mut DavSync) {
    let waiting: Vec<&ReminderAnswer> = pim.reminder_answers.iter().take(MAX_REMINDER_ANSWERS).collect();
    if waiting.is_empty() {
        return;
    }
    if let Some(v) = c.server_major().filter(|v| *v < 15) {
        log::info!("Exchange {v} (older than 2013) has no reminders to answer: {} answer(s) stay on this computer", waiting.len());
        out.answered.extend(waiting.into_iter().cloned());
        return;
    }
    for a in waiting {
        match answer(c, a) {
            Ok(()) => out.answered.push(a.clone()),
            Err(NetError::No(why)) => {
                log::info!("a reminder answer can't be told to Exchange: {why}");
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
