//! Reminders: those whose time has come, from the user's own calendars and tasks, as
//! Outlook's Reminders window lists them; dismissed and snoozed.

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
                let events: Vec<CalEvent> =
                    p.events.iter().filter(|e| e.calendar == cal.id && with.contains(e.event.uid.as_str())).map(|e| e.event.clone()).collect();
                for o in ical::occurrences(&events, now - REMINDER_KEPT_AFTER_END - 86_400, now + ahead + 86_400) {
                    let Some(e) = events.get(o.event) else { continue };
                    let Some(minutes) = reminder_of(e) else { continue };
                    if e.status.eq_ignore_ascii_case("CANCELLED") {
                        continue;
                    }
                    let (start, end) = if e.all_day { (self.clock.utc(o.start), self.clock.utc(o.end)) } else { (o.start, o.end) };
                    let occurrence = e.recurrence_id.unwrap_or(o.start);
                    let key = format!("{account}|{}|{occurrence}", e.uid);
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
            if self.find_task(uid).is_some() && self.update_task(uid, TaskChange { reminder: Some(None), ..Default::default() }).is_ok() {
                self.prefs.reminders.remove(k);
            } else {
                self.prefs.reminders.insert(k.clone(), 0);
            }
        }
        self.forget_old_reminders(now);
        keys.len()
    }

    /// Snooze a reminder until `until` (Unix seconds).
    pub fn snooze_reminder(&mut self, key: &str, until: i64) {
        self.prefs.reminders.insert(key.to_string(), until.max(1));
        self.forget_old_reminders(now_or(until));
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
