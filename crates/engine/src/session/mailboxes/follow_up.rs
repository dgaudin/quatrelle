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

/// The days a message was due and done, as they were, by its follow-up id (for Undo).
type FollowUpDays = Vec<(String, Option<i64>, Option<i64>)>;

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
            self.apply_op(&a, Op::SetFlag { keys, flag: Flag::Flagged, on });
        }
        for m in sel {
            let id = follow_up_id(m);
            // Unflagged: no longer due. Flagged again once complete: a fresh flag, not the one
            // completed (one still flagged keeps its day).
            if !on || m.flags.follow_up_complete() {
                self.prefs.follow_up.remove(&id);
            }
            self.prefs.follow_up_done.remove(&id);
        }
        let mut undo: BTreeMap<String, Vec<Op>> = BTreeMap::new();
        for (a, keys) in changed {
            undo.entry(a).or_default().push(Op::SetFlag { keys, flag: Flag::Flagged, on: !on });
        }
        for (a, keys) in completed {
            undo.entry(a).or_default().push(Op::SetKeyword { keys, keyword: FOLLOW_UP_COMPLETE.into(), on: true, name: None });
        }
        undo.into_iter().collect()
    }

    /// Each of `sel`'s due and done days as they are, for Undo to put back.
    pub(crate) fn follow_up_days_of(&self, sel: &[Arc<Summary>]) -> FollowUpDays {
        sel.iter()
            .map(|m| {
                let id = follow_up_id(m);
                let (due, done) = (self.prefs.follow_up.get(&id).copied(), self.prefs.follow_up_done.get(&id).copied());
                (id, due, done)
            })
            .collect()
    }

    /// Remember a change to follow-ups for Undo, when it changed anything: `ops` put the flags
    /// back, `was` the days.
    pub(crate) fn remember_follow_up(&mut self, ops: Vec<(String, Vec<Op>)>, was: FollowUpDays) {
        let days_changed =
            was.iter().any(|(id, due, done)| self.prefs.follow_up.get(id).copied() != *due || self.prefs.follow_up_done.get(id).copied() != *done);
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
        for (id, due, done) in days {
            put_day(&mut self.prefs.follow_up, id.clone(), due);
            put_day(&mut self.prefs.follow_up_done, id, done);
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
        for m in &sel {
            let id = follow_up_id(m);
            match due {
                Some(d) if self.prefs.follow_up.len() < MAX_FOLLOW_UP || self.prefs.follow_up.contains_key(&id) => {
                    self.prefs.follow_up.insert(id, d);
                }
                _ => {
                    self.prefs.follow_up.remove(&id);
                }
            }
        }
        self.remember_follow_up(undo, was);
        Ok(serde_json::json!({ "flagged": true, "due": due.map(quatrelle_model::dates::day_string), "count": sel.len() }))
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
            undo.entry(a.clone()).or_default().push(Op::SetFlag { keys: keys.clone(), flag: Flag::Flagged, on: true });
        }
        // The flag off before the keyword goes on: Exchange keeps both in one status.
        for (a, keys) in flagged {
            self.apply_op(&a, Op::SetFlag { keys, flag: Flag::Flagged, on: false });
        }
        for (a, keys) in marked {
            self.apply_op(&a, Op::SetKeyword { keys, keyword: FOLLOW_UP_COMPLETE.into(), on: true, name: None });
        }
        self.remember_follow_up(undo.into_iter().collect(), was);
        Ok(serde_json::json!({ "flagged": false, "complete": true, "count": sel.len() }))
    }

    /// When a flagged message is due, if a day was given.
    pub fn follow_up_due(&self, m: &Summary) -> Option<i64> {
        if !m.flags.flagged {
            return None;
        }
        self.prefs.follow_up.get(&follow_up_id(m)).copied()
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
