// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The session: following mail up (Home › Follow Up): flagged, due by a day, or marked
//! complete, as Outlook keeps it: a follow-up marked complete stays so, with the day it was
//! done, until it's flagged again or cleared (adapted from upstream f1eb8a3). The mark is the
//! [`FOLLOW_UP_COMPLETE`] keyword, which each server keeps its own way (Exchange as its flag
//! status, IMAP and JMAP as the keyword); the days are kept in the preferences, but for the day
//! Exchange says it was done, which wins.

use quatrelle_model::FOLLOW_UP_COMPLETE;

use super::*;

/// The most due or done days kept in the preferences.
pub(crate) const MAX_FOLLOW_UP: usize = 10_000;

/// One message's follow-up as it was, for Undo: start, due, reminder and done (local days, or
/// the reminder's time).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FollowUpWas {
    pub id: String,
    pub start: Option<i64>,
    pub due: Option<i64>,
    pub reminder: Option<i64>,
    pub done: Option<i64>,
}

/// The follow-ups of the messages a change touched, as they were.
pub type FollowUpDays = Vec<FollowUpWas>;

impl Session {
    /// Flag `sel` (or clear the flag): one marked complete no longer is, a fresh flag has no due
    /// day, and a cleared one neither. The changes that put the flags back, by account.
    ///
    /// A server that keeps one status for both (Exchange's flag status) gets the
    /// [`FOLLOW_UP_COMPLETE`] keyword taken off before the flag is set, and put on after it.
    pub(crate) fn set_flagged(&mut self, sel: &[Arc<Summary>], on: bool) -> Vec<(String, Vec<Op>)> {
        let mut by_account: BTreeMap<String, Vec<MessageKey>> = BTreeMap::new();
        let mut changed: BTreeMap<String, Vec<MessageKey>> = BTreeMap::new();
        // Carrying the keyword: complete, or flagged again elsewhere with it left behind.
        let mut completed: BTreeMap<String, Vec<MessageKey>> = BTreeMap::new();
        for m in sel {
            by_account.entry(m.key.account.clone()).or_default().push(m.key.clone());
            if m.flags.flagged != on {
                changed.entry(m.key.account.clone()).or_default().push(m.key.clone());
            }
            if m.flags.has(FOLLOW_UP_COMPLETE) {
                completed.entry(m.key.account.clone()).or_default().push(m.key.clone());
            }
        }
        for (a, keys) in &completed {
            self.apply_op(a, Op::SetKeyword { keys: keys.clone(), keyword: FOLLOW_UP_COMPLETE.into(), on: false, name: None });
        }
        for (a, keys) in by_account {
            self.apply_op(&a, Op::SetFlag { keys, flag: Flag::Flagged, on, all: false });
        }
        for m in sel {
            let id = follow_up_id(m);
            // Unflagged: no longer due. Flagged again once complete: a fresh flag, not the one
            // completed (one still flagged keeps its day).
            if !on || m.flags.follow_up_complete() {
                self.prefs.follow_up.remove(&id);
                self.prefs.follow_up_start.remove(&id);
                self.prefs.follow_up_reminder.remove(&id);
                self.prefs.reminders.remove(&format!("{}|mail|{}", m.key.account, m.key.to_id()));
            }
            self.prefs.follow_up_done.remove(&id);
        }
        let mut undo: BTreeMap<String, Vec<Op>> = BTreeMap::new();
        for (a, keys) in changed {
            undo.entry(a).or_default().push(Op::SetFlag { keys, flag: Flag::Flagged, on: !on, all: false });
        }
        for (a, keys) in completed {
            undo.entry(a).or_default().push(Op::SetKeyword { keys, keyword: FOLLOW_UP_COMPLETE.into(), on: true, name: None });
        }
        undo.into_iter().collect()
    }

    /// Each of `sel`'s follow-up days, start and reminder as they are, for Undo to put back.
    pub(crate) fn follow_up_days_of(&self, sel: &[Arc<Summary>]) -> FollowUpDays {
        sel.iter()
            .map(|m| {
                let id = follow_up_id(m);
                FollowUpWas {
                    start: self.prefs.follow_up_start.get(&id).copied(),
                    due: self.prefs.follow_up.get(&id).copied(),
                    reminder: self.prefs.follow_up_reminder.get(&id).copied(),
                    done: self.prefs.follow_up_done.get(&id).copied(),
                    id,
                }
            })
            .collect()
    }

    /// Remember a change to follow-ups for Undo, when it changed anything: `ops` put the flags
    /// back, `was` the days.
    pub(crate) fn remember_follow_up(&mut self, ops: Vec<(String, Vec<Op>)>, was: FollowUpDays) {
        let days_changed = was.iter().any(|w| {
            self.prefs.follow_up_start.get(&w.id).copied() != w.start
                || self.prefs.follow_up.get(&w.id).copied() != w.due
                || self.prefs.follow_up_reminder.get(&w.id).copied() != w.reminder
                || self.prefs.follow_up_done.get(&w.id).copied() != w.done
        });
        if ops.iter().any(|(_, o)| !o.is_empty()) || days_changed {
            self.remember(Undo::FollowUp { ops, days: was });
        }
    }

    /// Undo of a follow-up change: the flags and keywords put back, then the days.
    pub(crate) fn undo_follow_up(&mut self, ops: Vec<(String, Vec<Op>)>, days: FollowUpDays) {
        for (account, ops) in ops {
            for op in ops {
                self.apply_op(&account, op);
            }
        }
        for w in days {
            put_day(&mut self.prefs.follow_up_start, w.id.clone(), w.start);
            put_day(&mut self.prefs.follow_up, w.id.clone(), w.due);
            put_day(&mut self.prefs.follow_up_reminder, w.id.clone(), w.reminder);
            put_day(&mut self.prefs.follow_up_done, w.id, w.done);
        }
    }

    /// Flag the selection for follow-up by a day (`None`: no date), or (`complete`) mark it
    /// done ([`Self::complete_selection`]).
    pub fn follow_up_selection(&mut self, due: Option<i64>, complete: bool) -> CmdResult {
        if complete {
            return self.complete_selection();
        }
        let sel = self.selected();
        if sel.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        let was = self.follow_up_days_of(&sel);
        let undo = self.set_flagged(&sel, true);
        self.set_follow_up_fields(&sel, None, Some(due), None, was, undo);
        Ok(serde_json::json!({ "flagged": true, "due": due.map(quatrelle_model::dates::day_string), "count": sel.len() }))
    }

    /// Follow Up › Custom…: the selection's start and due days and its reminder as given (`None`
    /// takes each off), flagged for follow-up. Exchange keeps all three; Graph keeps the dates
    /// and the reminder stays here; the other servers keep all three here.
    pub fn follow_up_custom(&mut self, start: Option<i64>, due: Option<i64>, reminder: Option<i64>) -> CmdResult {
        self.follow_up_set(Some(start), Some(due), Some(reminder))
    }

    /// The follow-up fields named (`None`: leave as it is; `Some(None)`: take it off), flagged.
    pub fn follow_up_set(&mut self, start: Option<Option<i64>>, due: Option<Option<i64>>, reminder: Option<Option<i64>>) -> CmdResult {
        let sel = self.selected();
        if sel.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        let was = self.follow_up_days_of(&sel);
        let undo = self.set_flagged(&sel, true);
        self.set_follow_up_fields(&sel, start, due, reminder, was, undo);
        Ok(serde_json::json!({
            "flagged": true,
            "start": start.flatten().map(quatrelle_model::dates::day_string),
            "due": due.flatten().map(quatrelle_model::dates::day_string),
            "reminder": reminder.flatten(),
            "count": sel.len(),
        }))
    }

    /// Add Reminder… (`Some` a time) or take the selection's reminder off (`None`), keeping its
    /// dates and flagging it.
    pub fn follow_up_set_reminder(&mut self, reminder: Option<i64>) -> CmdResult {
        let sel = self.selected();
        if sel.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        let was = self.follow_up_days_of(&sel);
        let undo = self.set_flagged(&sel, true);
        self.set_follow_up_fields(&sel, None, None, Some(reminder), was, undo);
        Ok(serde_json::json!({ "flagged": true, "reminder": reminder, "count": sel.len() }))
    }

    /// Set the follow-up fields the caller named (outer `None`: leave it as it is; inner `None`:
    /// take it off), in the cache at once and where the account keeps them: Exchange's item, or
    /// the preferences (`Graph`'s reminder too, which Graph has nowhere for). `was` and `flags`
    /// are the Undo: the days kept here and the ops that put the flags back.
    fn set_follow_up_fields(
        &mut self,
        sel: &[Arc<Summary>],
        start: Option<Option<i64>>,
        due: Option<Option<i64>>,
        reminder: Option<Option<i64>>,
        was: FollowUpDays,
        undo: Vec<(String, Vec<Op>)>,
    ) {
        let mut undo: BTreeMap<String, Vec<Op>> = undo.into_iter().collect();
        // What each account keeps where: Exchange all three, Graph the dates, the others here.
        let (ews, graph) = self.mail_server_kind(sel);
        // The change to make, by account and its three values (a day as local midnight).
        let mut ops: BTreeMap<(String, i64, i64, i64), Vec<MessageKey>> = BTreeMap::new();
        let mut before: BTreeMap<(String, i64, i64, i64), Vec<MessageKey>> = BTreeMap::new();
        for m in sel {
            let id = follow_up_id(m);
            let old = (m.flags.follow_up.start.max(0), m.flags.follow_up.due.max(0), m.flags.follow_up.reminder.max(0));
            let now = (
                start.unwrap_or_else(|| self.follow_up_start(m)).map(|d| self.clock.midnight(d)).unwrap_or(0),
                due.unwrap_or_else(|| self.follow_up_due(m)).map(|d| self.clock.midnight(d)).unwrap_or(0),
                reminder.unwrap_or_else(|| self.follow_up_reminder(m)).unwrap_or(0),
            );
            let account = m.key.account.clone();
            // A reminder set anew rings again, though an earlier one was dismissed or snoozed.
            self.prefs.reminders.remove(&format!("{account}|mail|{}", m.key.to_id()));
            let exchange = ews.contains(account.as_str());
            let graph = graph.contains(account.as_str());
            if exchange || graph {
                ops.entry((account.clone(), now.0, now.1, now.2)).or_default().push(m.key.clone());
                before.entry((account, old.0, old.1, old.2)).or_default().push(m.key.clone());
            }
            // What stays here: everything for the other servers; Graph's reminder alone (its
            // dates are the server's, and an answer of none is none).
            if !exchange {
                let keep_dates = !graph;
                if keep_dates {
                    put_day(&mut self.prefs.follow_up_start, id.clone(), (now.0 > 0).then(|| self.clock.local_day(now.0)));
                    put_day(&mut self.prefs.follow_up, id.clone(), (now.1 > 0).then(|| self.clock.local_day(now.1)));
                }
                put_day(&mut self.prefs.follow_up_reminder, id.clone(), (now.2 > 0).then_some(now.2));
            } else {
                // Exchange's own: what was kept here would otherwise come back if it says none.
                self.prefs.follow_up_start.remove(&id);
                self.prefs.follow_up.remove(&id);
                self.prefs.follow_up_reminder.remove(&id);
            }
        }
        for ((account, s, d, r), keys) in ops {
            self.apply_op(&account, Op::SetFollowUp { keys, start: (s > 0).then_some(s), due: (d > 0).then_some(d), reminder: (r > 0).then_some(r) });
        }
        // Undo puts the server's own back too (the days here are put back by `was`).
        for ((account, s, d, r), keys) in before {
            undo.entry(account).or_default().push(Op::SetFollowUp {
                keys,
                start: (s > 0).then_some(s),
                due: (d > 0).then_some(d),
                reminder: (r > 0).then_some(r),
            });
        }
        self.remember_follow_up(undo.into_iter().collect(), was);
    }

    /// The accounts among `sel` whose server keeps a follow-up's dates: Exchange (EWS) all of
    /// them, Graph the dates alone.
    fn mail_server_kind<'a>(&self, sel: &'a [Arc<Summary>]) -> (BTreeSet<&'a str>, BTreeSet<&'a str>) {
        let mut ews = BTreeSet::new();
        let mut graph = BTreeSet::new();
        for m in sel {
            if let Some(a) = self.account(&m.key.account) {
                if a.config.ews.is_some() {
                    ews.insert(m.key.account.as_str());
                } else if a.config.graph.is_some() {
                    graph.insert(m.key.account.as_str());
                }
            }
        }
        (ews, graph)
    }

    /// Mark the selection's follow-up complete (Mark Complete): unflagged, with the
    /// [`FOLLOW_UP_COMPLETE`] keyword, and the day it was done kept (as Outlook keeps it). Its
    /// due day stays; Undo puts the flag and both days back as they were, in one step.
    pub fn complete_selection(&mut self) -> CmdResult {
        let sel = self.selected();
        if sel.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        let was = self.follow_up_days_of(&sel);
        let today = quatrelle_model::dates::today(&self.clock);
        let mut flagged: BTreeMap<String, Vec<MessageKey>> = BTreeMap::new();
        let mut marked: BTreeMap<String, Vec<MessageKey>> = BTreeMap::new();
        for m in &sel {
            if m.flags.flagged {
                flagged.entry(m.key.account.clone()).or_default().push(m.key.clone());
            }
            if !m.flags.has(FOLLOW_UP_COMPLETE) {
                marked.entry(m.key.account.clone()).or_default().push(m.key.clone());
            }
            // Done today, unless it was already done (marked complete again).
            let id = follow_up_id(m);
            if !(m.flags.follow_up_complete() && self.prefs.follow_up_done.contains_key(&id))
                && (self.prefs.follow_up_done.len() < MAX_FOLLOW_UP || self.prefs.follow_up_done.contains_key(&id))
            {
                self.prefs.follow_up_done.insert(id, today);
            }
        }
        // Undo: the keyword off before the flag goes back on (see `set_flagged`).
        let mut undo: BTreeMap<String, Vec<Op>> = BTreeMap::new();
        for (a, keys) in &marked {
            undo.entry(a.clone()).or_default().push(Op::SetKeyword { keys: keys.clone(), keyword: FOLLOW_UP_COMPLETE.into(), on: false, name: None });
        }
        for (a, keys) in &flagged {
            undo.entry(a.clone()).or_default().push(Op::SetFlag { keys: keys.clone(), flag: Flag::Flagged, on: true, all: false });
        }
        // The flag off before the keyword goes on: Exchange keeps both in one status.
        for (a, keys) in flagged {
            self.apply_op(&a, Op::SetFlag { keys, flag: Flag::Flagged, on: false, all: false });
        }
        for (a, keys) in marked {
            self.apply_op(&a, Op::SetKeyword { keys, keyword: FOLLOW_UP_COMPLETE.into(), on: true, name: None });
        }
        self.remember_follow_up(undo.into_iter().collect(), was);
        Ok(serde_json::json!({ "flagged": false, "complete": true, "count": sel.len() }))
    }

    /// When a flagged message is due, if a day was given: the server's own (Exchange, Graph),
    /// else the day kept here.
    pub fn follow_up_due(&self, m: &Summary) -> Option<i64> {
        if !m.flags.flagged {
            return None;
        }
        if m.flags.follow_up.due > 0 {
            return Some(self.clock.local_day(m.flags.follow_up.due));
        }
        self.prefs.follow_up.get(&follow_up_id(m)).copied()
    }

    /// When a flag's follow-up starts: the server's own, else the day kept here.
    pub fn follow_up_start(&self, m: &Summary) -> Option<i64> {
        if m.flags.follow_up.start > 0 {
            return Some(self.clock.local_day(m.flags.follow_up.start));
        }
        self.prefs.follow_up_start.get(&follow_up_id(m)).copied()
    }

    /// When a flag's reminder rings (Unix seconds): the server's own (Exchange), else the one
    /// kept here (everywhere else, Graph too, which has no reminder on a flag).
    pub fn follow_up_reminder(&self, m: &Summary) -> Option<i64> {
        if m.flags.follow_up.reminder > 0 {
            return Some(m.flags.follow_up.reminder);
        }
        self.prefs.follow_up_reminder.get(&follow_up_id(m)).copied()
    }

    /// What a flag's follow-up asks for: the title Outlook wrote (Exchange), else none.
    pub fn follow_up_title<'a>(&self, m: &'a Summary) -> Option<&'a str> {
        (!m.flags.follow_up.title.is_empty()).then_some(m.flags.follow_up.title.as_str())
    }

    /// When a message's follow-up was marked complete (a local day), if it was and the day is
    /// known: the server's (Exchange keeps it), else the one kept here.
    pub fn follow_up_done(&self, m: &Summary) -> Option<i64> {
        if !m.flags.follow_up_complete() {
            return None;
        }
        if m.flags.completed > 0 {
            return Some(self.clock.local_day(m.flags.completed));
        }
        self.prefs.follow_up_done.get(&follow_up_id(m)).copied()
    }
}

/// A day put back as it was (`None`: none), within [`MAX_FOLLOW_UP`].
fn put_day(days: &mut BTreeMap<String, i64>, id: String, day: Option<i64>) {
    match day {
        Some(d) if days.len() < MAX_FOLLOW_UP || days.contains_key(&id) => {
            days.insert(id, d);
        }
        Some(_) => {}
        None => {
            days.remove(&id);
        }
    }
}
