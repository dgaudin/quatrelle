// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Reminders: those whose time has come, from the user's own calendars and tasks, as
//! Outlook's Reminders window lists them; dismissed and snoozed, and the server told (issue #40).

use super::*;

/// A reminder whose time has come (File › Reminders, as Outlook's Reminders window lists them).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DueReminder {
    /// Names it for snoozing and dismissing: `<account>|<uid>|<occurrence>`.
    pub key: String,
    pub account: String,
    pub uid: String,
    pub occurrence: i64,
    pub summary: String,
    pub location: String,
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
    /// When it was due (its start, less its reminder; or the end of a snooze).
    pub due: i64,
    /// A task's (`uid` names the task; `start` is the day it's due, or its reminder's time).
    pub task: bool,
}

/// How long after an event has ended its reminder still waits to be answered.
const REMINDER_KEPT_AFTER_END: i64 = 86_400;
/// Most reminders listed at once.
const REMINDERS_MAX: usize = 200;

/// An event's reminder, in minutes before its start: its own field (`CalEvent::reminder`, which
/// its `VALARM` is read into and written from, and which Exchange reads and writes); or, for one
/// made elsewhere with an alarm still among its `extra` lines, that alarm.
fn reminder_of(e: &CalEvent) -> Option<i64> {
    e.reminder.or_else(|| ical::reminder_minutes(e))
}

impl Session {
    /// The reminders due at `now`, soonest first: occurrences of the user's own calendars (shown
    /// or hidden; not others' shared with them) whose reminder time has passed, that haven't
    /// ended a day ago, weren't dismissed, and aren't snoozed past `now`; and the tasks not done
    /// whose reminder has come.
    pub fn due_reminders(&self, now: i64) -> Vec<DueReminder> {
        let ahead = ical::REMINDER_MAX_MINUTES * 60;
        let mut out = Vec::new();
        for (account, p) in &self.pim {
            if self.account(account).is_none() {
                continue;
            }
            // Not others' calendars, nor ones subscribed to (a holiday list's alarms are its own).
            let own = |c: &&quatrelle_model::pim::CalendarInfo| {
                quatrelle_model::pim::shared_calendar_owner(&c.id).is_none() && !quatrelle_model::pim::is_web_calendar(&c.id)
            };
            for cal in p.calendars.iter().filter(own) {
                // Only the series with a reminder somewhere (an exception may have its own).
                let with: std::collections::BTreeSet<&str> =
                    p.events.iter().filter(|e| e.calendar == cal.id && reminder_of(&e.event).is_some()).map(|e| e.event.uid.as_str()).collect();
                if with.is_empty() {
                    continue;
                }
                let events: Vec<&CalEvent> =
                    p.events.iter().filter(|e| e.calendar == cal.id && with.contains(e.event.uid.as_str())).map(|e| &e.event).collect();
                for o in ical::occurrences(&events, now - REMINDER_KEPT_AFTER_END - 86_400, now + ahead + 86_400) {
                    let Some(e) = events.get(o.event) else { continue };
                    let Some(minutes) = reminder_of(e) else { continue };
                    if e.status.eq_ignore_ascii_case("CANCELLED") {
                        continue;
                    }
                    let (start, end) = if e.all_day { (self.clock.utc(o.start), self.clock.utc(o.end)) } else { (o.start, o.end) };
                    let occurrence = e.recurrence_id.unwrap_or(o.start);
                    let key = format!("{account}|{}|{occurrence}", e.uid);
                    // Dismissed in another calendar program, which wrote so into the event
                    // (RFC 9074's `ACKNOWLEDGED`): not due here either, unless snoozed here.
                    if ical::acknowledged(e).is_some_and(|at| at >= start - minutes * 60) && !self.prefs.reminders.contains_key(&key) {
                        continue;
                    }
                    let due = match self.prefs.reminders.get(&key) {
                        Some(0) => continue,
                        Some(until) => *until,
                        None => start - minutes * 60,
                    };
                    if due > now || end.max(start) < now - REMINDER_KEPT_AFTER_END {
                        continue;
                    }
                    out.push(DueReminder {
                        key,
                        account: account.clone(),
                        uid: e.uid.clone(),
                        occurrence,
                        summary: e.summary.clone(),
                        location: e.location.clone(),
                        start,
                        end,
                        all_day: e.all_day,
                        due,
                        task: false,
                    });
                }
            }
            // Tasks not done whose reminder has come: its day due, or the reminder's time.
            for t in p.tasks.iter().map(|t| &t.task).filter(|t| !t.is_complete()) {
                let Some(at) = t.reminder else { continue };
                let key = format!("{account}|{}|{at}", t.uid);
                let due = match self.prefs.reminders.get(&key) {
                    Some(0) => continue,
                    Some(until) => *until,
                    None => at,
                };
                if due > now {
                    continue;
                }
                let (start, end, all_day) = match t.due {
                    Some(d) => {
                        let midnight = self.clock.utc(d.saturating_mul(86_400));
                        (midnight, midnight + 86_400, true)
                    }
                    None => (at, at, false),
                };
                out.push(DueReminder {
                    key,
                    account: account.clone(),
                    uid: t.uid.clone(),
                    occurrence: at,
                    summary: t.subject.clone(),
                    location: String::new(),
                    start,
                    end,
                    all_day,
                    due,
                    task: true,
                });
            }
        }
        out.sort_by(|a, b| (a.start, &a.summary, &a.key).cmp(&(b.start, &b.summary, &b.key)));
        out.dedup_by(|a, b| a.key == b.key);
        out.truncate(REMINDERS_MAX);
        out
    }

    /// Dismiss reminders: the one named by `key`, or every one due at `now` (`None`).
    pub fn dismiss_reminders(&mut self, key: Option<&str>, now: i64) -> usize {
        let keys: Vec<String> = match key {
            Some(k) => vec![k.to_string()],
            None => self.due_reminders(now).into_iter().map(|r| r.key).collect(),
        };
        for k in &keys {
            // A task's reminder is taken off the task, as Outlook does (and Exchange learns it).
            let uid = k.split('|').nth(1).unwrap_or_default();
            let off = (self.find_task(uid).is_some() && self.update_task(uid, TaskChange { reminder: Some(None), ..Default::default() }).is_ok())
                || self.turn_off_reminder_of_single(k);
            if off {
                self.prefs.reminders.remove(k);
                // A snooze still waiting for the server is overtaken.
                self.forget_reminder_answer(k);
            } else {
                self.prefs.reminders.insert(k.clone(), 0);
                self.tell_server_reminder(k, None, now);
            }
        }
        self.forget_old_reminders(now);
        keys.len()
    }

    /// An appointment that is no part of a series, in one of the user's own calendars, loses its
    /// reminder when it's dismissed, as in Outlook (Exchange's `ReminderIsSet`): it's on the item,
    /// so every device sees it and moving the appointment doesn't bring it back. A personal
    /// matter: the guests are never told, and its `SEQUENCE` isn't raised (a CalDAV server that
    /// schedules could take it for a change to the meeting). False for anything else (a series
    /// or one of its occurrences, an event not found, a calendar that can't be changed): that
    /// one is answered per occurrence.
    fn turn_off_reminder_of_single(&mut self, key: &str) -> bool {
        let mut parts = key.split('|');
        let (Some(account), Some(uid)) = (parts.next(), parts.next()) else { return false };
        let Some(p) = self.pim.get(account) else { return false };
        let Some(stored) = p.events.iter().find(|e| e.event.uid == uid) else { return false };
        if p.events.iter().any(|e| e.event.uid == uid && (e.event.rrule.is_some() || e.event.recurrence_id.is_some())) {
            return false;
        }
        if quatrelle_model::pim::shared_calendar_owner(&stored.calendar).is_some() || quatrelle_model::pim::is_web_calendar(&stored.calendar) {
            return false;
        }
        let Some((account, at)) = self.find_event_index(uid).map(|(a, at)| (a.to_string(), at)) else { return false };
        let p = self.pim_mut(&account);
        let read_only = |cal: &str| p.calendars.iter().any(|c| c.id == cal && c.read_only);
        let Some(remote) = p.events.get(at).filter(|e| !read_only(&e.calendar)).map(|e| e.calendar != quatrelle_model::pim::LOCAL) else {
            return false;
        };
        let Some(stored) = p.events.get_mut(at) else { return false };
        ical::set_reminder(&mut stored.event, None);
        stored.changed_here(remote);
        self.after_pim_change(&account, remote);
        true
    }

    /// Snooze a reminder until `until` (Unix seconds).
    pub fn snooze_reminder(&mut self, key: &str, until: i64) {
        self.prefs.reminders.insert(key.to_string(), until.max(1));
        self.tell_server_reminder(key, Some(until.max(1)), self.clock.now);
        self.forget_old_reminders(now_or(until));
    }

    /// Tell the event's server a reminder answered here that it can't learn from the event
    /// itself, so the user's other programs (Outlook, Outlook on the web, the phone) don't show
    /// it again (issue #40). Exchange learns it through its own reminder action at the next
    /// sync, as Outlook in Cached Exchange Mode tells it: the answer waits with the account's
    /// calendar ([`quatrelle_model::pim::ReminderAnswer`]), so one given offline isn't lost. A CalDAV server learns a
    /// dismissal from the event, its alarm acknowledged at `now` (RFC 9074), which every client
    /// reads; its `SEQUENCE` isn't raised (no guest has anything to learn); a snooze stays here.
    /// A single appointment's dismissal isn't for this: its reminder is turned off on the item
    /// ([`Self::turn_off_reminder_of_single`]). Nor is a calendar this account can't change.
    fn tell_server_reminder(&mut self, key: &str, snooze_until: Option<i64>, now: i64) {
        use quatrelle_model::pim::{LOCAL, MAX_REMINDER_ANSWERS, ReminderAnswer, is_web_calendar, shared_calendar_owner};
        let mut parts = key.rsplitn(3, '|');
        let (Some(occurrence), Some(uid), Some(account)) = (parts.next().and_then(|o| o.parse::<i64>().ok()), parts.next(), parts.next()) else {
            return;
        };
        let Some(config) = self.account(account).map(|a| a.config.clone()) else { return };
        if config.is_local() {
            return;
        }
        let exchange = config.graph.is_some() || config.ews.is_some();
        let p = self.pim_mut(account);
        let writable = |e: &StoredEvent| {
            e.calendar != LOCAL
                && shared_calendar_owner(&e.calendar).is_none()
                && !is_web_calendar(&e.calendar)
                && !p.calendars.iter().any(|c| c.id == e.calendar && c.read_only)
                && (!exchange || !e.href.is_empty())
        };
        // The occurrence's own copy (one changed on its own), and its series.
        let own = p.events.iter().position(|e| e.event.uid == uid && e.event.recurrence_id == Some(occurrence));
        let series = p.events.iter().position(|e| e.event.uid == uid && e.event.recurrence_id.is_none());
        if exchange {
            // Exchange finds an occurrence's reminder by its series and start; a single
            // appointment's (or an occurrence whose series isn't here) by its own item.
            let (at, of_series) = match series.filter(|&i| p.events.get(i).is_some_and(|e| e.event.rrule.is_some() && writable(e))) {
                Some(i) => (i, true),
                None => match own.or(series).filter(|&i| p.events.get(i).is_some_and(writable)) {
                    Some(i) => (i, false),
                    None => return,
                },
            };
            let Some(stored) = p.events.get(at) else { return };
            let answer = ReminderAnswer {
                calendar: stored.calendar.clone(),
                href: stored.href.clone(),
                uid: uid.to_string(),
                occurrence,
                series: of_series,
                snooze_until,
            };
            // Answered again (snoozed, then dismissed): the later answer stands. Those for
            // occurrences long past go, and the list is kept small.
            p.reminder_answers.retain(|a| !(a.uid == uid && a.occurrence == occurrence) && a.occurrence >= now.saturating_sub(30 * 86_400));
            if p.reminder_answers.len() >= MAX_REMINDER_ANSWERS {
                p.reminder_answers.remove(0);
            }
            p.reminder_answers.push(answer);
        } else if snooze_until.is_none() {
            let Some(i) = own.or(series).filter(|&i| p.events.get(i).is_some_and(writable)) else { return };
            let Some(e) = p.events.get_mut(i) else { return };
            ical::acknowledge(&mut e.event, now);
            e.changed_here(true);
        } else {
            return;
        }
        self.after_pim_change(account, true);
    }

    /// The answer waiting for the server for this reminder, if any, goes.
    fn forget_reminder_answer(&mut self, key: &str) {
        let mut parts = key.rsplitn(3, '|');
        let (Some(occurrence), Some(uid), Some(account)) = (parts.next().and_then(|o| o.parse::<i64>().ok()), parts.next(), parts.next()) else {
            return;
        };
        if let Some(p) = self.pim.get_mut(account)
            && p.reminder_answers.iter().any(|a| a.uid == uid && a.occurrence == occurrence)
        {
            p.reminder_answers.retain(|a| !(a.uid == uid && a.occurrence == occurrence));
            self.write_pim(account);
        }
    }

    /// Answers for occurrences long past go, except a snooze still running (a task's reminder
    /// long overdue, snoozed); and the list is kept small.
    fn forget_old_reminders(&mut self, now: i64) {
        let cutoff = now - 30 * 86_400;
        self.prefs.reminders.retain(|k, until| *until > now || k.rsplit('|').next().and_then(|t| t.parse::<i64>().ok()).is_none_or(|t| t >= cutoff));
        while self.prefs.reminders.len() > 5000 {
            let Some(first) = self.prefs.reminders.keys().next().cloned() else { break };
            self.prefs.reminders.remove(&first);
        }
    }
}

/// The time to judge old answers by: `t` when it's a real time.
fn now_or(t: i64) -> i64 {
    t.max(0)
}
