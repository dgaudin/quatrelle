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

/// One message's follow-up as it was, for Undo: start, due, reminder, title and done (local
/// days, the reminder's time, or none).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FollowUpWas {
    pub id: String,
    pub start: Option<i64>,
    pub due: Option<i64>,
    pub reminder: Option<i64>,
    pub title: Option<String>,
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
        let mut changed: BTreeMap<String, Vec<MessageKey>> = BTreeMap::new();
        // Carrying the keyword: complete, or flagged again elsewhere with it left behind.
        let mut completed: BTreeMap<String, Vec<MessageKey>> = BTreeMap::new();
        for m in sel {
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
        // Where the server keeps a follow-up's dates and reminder (Exchange, Graph): Clear Flag
        // takes them off there too, as Outlook's does (Mark Complete, which takes the flag off
        // the same way, keeps them: the flag-off write alone leaves them).
        let (ews, graph) = self.mail_server_kind(sel);
        let server_side = |a: &str| ews.contains(a) || graph.contains(a);
        let mut clear: BTreeMap<String, Vec<MessageKey>> = BTreeMap::new();
        let mut restore: BTreeMap<(String, i64, i64, i64, String), Vec<MessageKey>> = BTreeMap::new();
        if !on {
            for m in sel {
                // Only a message that has a flag or a date there is written to: Clear Flag on
                // one with neither would send an UpdateItem deleting eight properties it
                // doesn't have.
                let has = m.flags.flagged || m.flags.follow_up.start > 0 || m.flags.follow_up.due > 0 || m.flags.follow_up.reminder > 0;
                if server_side(&m.key.account) && has {
                    let old = (m.flags.follow_up.start.max(0), m.flags.follow_up.due.max(0), m.flags.follow_up.reminder.max(0));
                    let title = self.follow_up_title(m).unwrap_or_default().to_string();
                    clear.entry(m.key.account.clone()).or_default().push(m.key.clone());
                    restore.entry((m.key.account.clone(), old.0, old.1, old.2, title)).or_default().push(m.key.clone());
                }
            }
        }
        // The flag write goes only to the messages whose flag changes: one already flagged keeps
        // the text and icon Outlook gave it ([MS-OXOFLAG] 3.1.4.1.3: `PidLidFlagRequest` is set
        // only if it doesn't already exist — writing it again would replace a "Review" with
        // "Follow up").
        for (a, keys) in &changed {
            self.apply_op(a, Op::SetFlag { keys: keys.clone(), flag: Flag::Flagged, on, all: false });
        }
        // Flagged: the task's title goes with the flag on Exchange, as Outlook sets it
        // (`PidLidToDoTitle` with the message's subject), grouped by title.
        if on {
            let mut by_title: BTreeMap<(String, String), Vec<MessageKey>> = BTreeMap::new();
            for m in sel {
                if !m.flags.flagged && ews.contains(m.key.account.as_str()) {
                    by_title.entry((m.key.account.clone(), m.subject.clone())).or_default().push(m.key.clone());
                }
            }
            for ((a, title), keys) in by_title {
                self.apply_op(&a, Op::SetFollowUp { keys, start: None, due: None, reminder: None, title: Some(Some(title)) });
            }
        }
        for (a, keys) in &clear {
            self.apply_op(
                a,
                Op::SetFollowUp {
                    keys: keys.clone(),
                    start: Some(FieldChange::Clear),
                    due: Some(FieldChange::Clear),
                    reminder: Some(FieldChange::Clear),
                    // Clear Flag takes the title off too, as Outlook's does.
                    title: Some(None),
                },
            );
        }
        for m in sel {
            let id = follow_up_id(m);
            // Unflagged: no longer due. Flagged again once complete: a fresh flag, not the one
            // completed (one still flagged keeps its day).
            if !on || m.flags.follow_up_complete() {
                self.prefs.follow_up.remove(&id);
                self.prefs.follow_up_start.remove(&id);
                self.prefs.follow_up_reminder.remove(&id);
                self.prefs.follow_up_title.remove(&id);
                forget_mail_reminder_answers(&mut self.prefs, &m.key.account, &id);
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
        // Undo of Clear Flag puts the server's dates, reminder and title back as they were.
        for ((a, s, d, r, t), keys) in restore {
            let back = |old: i64| if old > 0 { FieldChange::Set(old) } else { FieldChange::Clear };
            undo.entry(a).or_default().push(Op::SetFollowUp {
                keys,
                start: Some(back(s)),
                due: Some(back(d)),
                reminder: Some(back(r)),
                title: if t.is_empty() { Some(None) } else { Some(Some(t)) },
            });
        }
        undo.into_iter().collect()
    }

    /// Each of `sel`'s follow-up days, start, reminder and title as they are, for Undo to put
    /// back.
    pub(crate) fn follow_up_days_of(&self, sel: &[Arc<Summary>]) -> FollowUpDays {
        sel.iter()
            .map(|m| {
                let id = follow_up_id(m);
                FollowUpWas {
                    start: self.prefs.follow_up_start.get(&id).copied(),
                    due: self.prefs.follow_up.get(&id).copied(),
                    reminder: self.prefs.follow_up_reminder.get(&id).copied(),
                    title: self.follow_up_title(m).filter(|t| !t.is_empty()).map(str::to_string),
                    done: self.prefs.follow_up_done.get(&id).copied(),
                    id,
                }
            })
            .collect()
    }

    /// Remember a change to follow-ups for Undo, when it changed anything: `ops` put the flags
    /// back, `was` the days and titles.
    pub(crate) fn remember_follow_up(&mut self, ops: Vec<(String, Vec<Op>)>, was: FollowUpDays) {
        let days_changed = was.iter().any(|w| {
            self.prefs.follow_up_start.get(&w.id).copied() != w.start
                || self.prefs.follow_up.get(&w.id).copied() != w.due
                || self.prefs.follow_up_reminder.get(&w.id).copied() != w.reminder
                || self.prefs.follow_up_title.get(&w.id).map(String::as_str) != w.title.as_deref()
                || self.prefs.follow_up_done.get(&w.id).copied() != w.done
        });
        if ops.iter().any(|(_, o)| !o.is_empty()) || days_changed {
            self.remember(Undo::FollowUp { ops, days: was });
        }
    }

    /// Undo of a follow-up change: the flags and keywords put back, then the days and titles.
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
            put_title(&mut self.prefs.follow_up_title, w.id.clone(), w.title);
            put_day(&mut self.prefs.follow_up_done, w.id, w.done);
        }
    }

    /// Flag the selection for follow-up by a day (`None`: no date), or (`complete`) mark it
    /// done ([`Self::complete_selection`]).
    pub fn follow_up_selection(&mut self, due: Option<i64>, complete: bool) -> CmdResult {
        if complete {
            return self.complete_selection();
        }
        self.follow_up_for(&self.selected(), due)
    }

    /// [`Self::follow_up_selection`] on these messages (the To-Do List's own rows, by key:
    /// `mail.flag` with `key`).
    pub fn follow_up_for(&mut self, sel: &[Arc<Summary>], due: Option<i64>) -> CmdResult {
        if sel.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        let was = self.follow_up_days_of(sel);
        let undo = self.set_flagged(sel, true);
        self.set_follow_up_fields(sel, None, Some(due), None, None, was, undo);
        Ok(serde_json::json!({ "flagged": true, "due": due.map(quatrelle_model::dates::day_string), "count": sel.len() }))
    }

    /// Follow Up › Custom…: the selection's start and due days, its reminder and its title as
    /// given (`None` takes each off), flagged for follow-up. Exchange keeps all three and the
    /// title; Graph keeps the dates and the reminder and title stay here; the other servers keep
    /// all of them here.
    pub fn follow_up_custom(&mut self, start: Option<i64>, due: Option<i64>, reminder: Option<i64>, title: Option<String>) -> CmdResult {
        self.follow_up_set(Some(start), Some(due), Some(reminder), Some(title))
    }

    /// The follow-up fields named (`None`: leave as it is; `Some(None)`: take it off), flagged.
    pub fn follow_up_set(
        &mut self,
        start: Option<Option<i64>>,
        due: Option<Option<i64>>,
        reminder: Option<Option<i64>>,
        title: Option<Option<String>>,
    ) -> CmdResult {
        self.follow_up_set_for(&self.selected(), start, due, reminder, title)
    }

    /// [`Self::follow_up_set`] on these messages (a row of the To-Do List, by key).
    pub fn follow_up_set_for(
        &mut self,
        sel: &[Arc<Summary>],
        start: Option<Option<i64>>,
        due: Option<Option<i64>>,
        reminder: Option<Option<i64>>,
        title: Option<Option<String>>,
    ) -> CmdResult {
        if sel.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        let was = self.follow_up_days_of(sel);
        let undo = self.set_flagged(sel, true);
        let named = title.clone();
        self.set_follow_up_fields(sel, start, due, reminder, title, was, undo);
        Ok(serde_json::json!({
            "flagged": true,
            "start": start.flatten().map(quatrelle_model::dates::day_string),
            "due": due.flatten().map(quatrelle_model::dates::day_string),
            "reminder": reminder.flatten(),
            "title": named.flatten(),
            "count": sel.len(),
        }))
    }

    /// Add Reminder… (`Some` a time) or take the selection's reminder off (`None`), keeping its
    /// dates, title and flagging it.
    pub fn follow_up_set_reminder(&mut self, reminder: Option<i64>) -> CmdResult {
        self.follow_up_set_for(&self.selected(), None, None, Some(reminder), None)
    }

    /// Set the follow-up fields the caller named (outer `None`: leave it as it is; inner `None`:
    /// take it off), in the cache at once and where the account keeps them: Exchange's item, or
    /// the preferences (`Graph`'s reminder and title too, which Graph has nowhere for). `was`
    /// and `flags` are the Undo: the days and titles kept here and the ops that put the flags
    /// back.
    fn set_follow_up_fields(
        &mut self,
        sel: &[Arc<Summary>],
        start: Option<Option<i64>>,
        due: Option<Option<i64>>,
        reminder: Option<Option<i64>>,
        title: Option<Option<String>>,
        was: FollowUpDays,
        undo: Vec<(String, Vec<Op>)>,
    ) {
        let mut undo: BTreeMap<String, Vec<Op>> = undo.into_iter().collect();
        // What each account keeps where: Exchange all three, Graph the dates, the others here.
        let (ews, graph) = self.mail_server_kind(sel);
        // The change each field names, as a day goes to the server: the day's floating midnight
        // (`day * 86_400`), the convention Exchange's `PidLidTask*Date` holds ([MS-OXOTASK]
        // 2.2.2.2.4). A field left unnamed (`None`) is left as the server has it, so a partial
        // change doesn't erase what the cache didn't know.
        let change = |v: Option<Option<i64>>| v.map(|d| d.map_or(FieldChange::Clear, |d| FieldChange::Set(d.saturating_mul(86_400))));
        let change_time = |v: Option<Option<i64>>| v.map(|t| t.map_or(FieldChange::Clear, FieldChange::Set));
        let (start, due, reminder) = (change(start), change(due), change_time(reminder));
        // The change to make, by account and its four fields as named.
        let mut ops: BTreeMap<(String, Option<FieldChange>, Option<FieldChange>, Option<FieldChange>, Option<Option<String>>), Vec<MessageKey>> =
            BTreeMap::new();
        let mut before: BTreeMap<(String, i64, i64, i64, String), Vec<MessageKey>> = BTreeMap::new();
        for m in sel {
            let id = follow_up_id(m);
            let old = (m.flags.follow_up.start.max(0), m.flags.follow_up.due.max(0), m.flags.follow_up.reminder.max(0));
            let old_title = self.follow_up_title(m).unwrap_or_default().to_string();
            let account = m.key.account.clone();
            // A reminder set anew or taken off rings again, though an earlier one was dismissed
            // or snoozed; one the change leaves as it was keeps the answer given to it.
            if let Some(c) = reminder {
                let new = c.value();
                let old = self.follow_up_reminder(m).filter(|t| *t > 0);
                if new != old {
                    forget_mail_reminder_answers(&mut self.prefs, &account, &id);
                }
            }
            let exchange = ews.contains(account.as_str());
            let graph = graph.contains(account.as_str());
            // A start set without a due takes the due with it only when the server has none
            // ([MS-OXOTASK] requires the due with the start); when it has one, the change names
            // the start alone and the server's due stays — same the other way round for Graph's
            // start. A field the change doesn't name is never rewritten from another one.
            let mut s = start;
            let mut d = due;
            if let Some(FieldChange::Set(v)) = d
                && !matches!(s, Some(FieldChange::Set(_)))
                && m.flags.follow_up.start <= 0
            {
                s = Some(FieldChange::Set(v));
            }
            if let Some(FieldChange::Set(v)) = s
                && d.is_none()
                && m.flags.follow_up.due <= 0
            {
                d = Some(FieldChange::Set(v));
            }
            if exchange || graph {
                ops.entry((account.clone(), s, d, reminder, title.clone())).or_default().push(m.key.clone());
                before.entry((account, old.0, old.1, old.2, old_title)).or_default().push(m.key.clone());
            }
            // What stays here: everything for the other servers; Graph's reminder and title
            // alone (its dates are the server's). Only the fields named are touched: an unnamed
            // one keeps what was kept here.
            if !exchange {
                let keep_dates = !graph;
                if keep_dates {
                    if let Some(c) = start {
                        put_day(&mut self.prefs.follow_up_start, id.clone(), c.value().map(quatrelle_model::dates::utc_day));
                    }
                    if let Some(c) = due {
                        put_day(&mut self.prefs.follow_up, id.clone(), c.value().map(quatrelle_model::dates::utc_day));
                    }
                }
                if let Some(c) = reminder {
                    put_day(&mut self.prefs.follow_up_reminder, id.clone(), c.value());
                }
                if let Some(t) = title.as_ref() {
                    put_title(&mut self.prefs.follow_up_title, id.clone(), t.clone());
                }
            } else {
                // Exchange's own: what was kept here would otherwise come back if it says none.
                self.prefs.follow_up_start.remove(&id);
                self.prefs.follow_up.remove(&id);
                self.prefs.follow_up_reminder.remove(&id);
                self.prefs.follow_up_title.remove(&id);
            }
        }
        for ((account, s, d, r, t), keys) in ops {
            if s.is_none() && d.is_none() && r.is_none() && t.is_none() {
                continue;
            }
            self.apply_op(&account, Op::SetFollowUp { keys, start: s, due: d, reminder: r, title: t });
        }
        // Undo puts the server's own back too, for the fields the change named (the days and
        // titles here are put back by `was`): what the cache had, set or taken off.
        for ((account, s, d, r, old_title), keys) in before {
            let back = |old: i64, named: Option<FieldChange>| named.map(|_| if old > 0 { FieldChange::Set(old) } else { FieldChange::Clear });
            let back_title = title.as_ref().map(|_| if old_title.is_empty() { None } else { Some(old_title) });
            undo.entry(account).or_default().push(Op::SetFollowUp {
                keys,
                start: back(s, start),
                due: back(d, due),
                reminder: back(r, reminder),
                title: back_title,
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
        self.complete_for(&self.selected())
    }

    /// [`Self::complete_selection`] on these messages (a row of the To-Do List, by key).
    pub fn complete_for(&mut self, sel: &[Arc<Summary>]) -> CmdResult {
        if sel.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        let (ews, _) = self.mail_server_kind(sel);
        let was = self.follow_up_days_of(sel);
        let today = quatrelle_model::dates::today(&self.clock);
        let mut flagged: BTreeMap<String, Vec<MessageKey>> = BTreeMap::new();
        let mut marked: BTreeMap<String, Vec<MessageKey>> = BTreeMap::new();
        for m in sel {
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
        // Undo on Exchange puts the task's title and the reminder back too (the mark complete
        // took the reminder off; the flag's text stayed).
        for m in sel {
            if ews.contains(m.key.account.as_str()) {
                let title = self.follow_up_title(m).map(str::to_string).filter(|t| !t.is_empty()).unwrap_or_else(|| m.subject.clone());
                let old_rem = m.flags.follow_up.reminder.max(0);
                let back_r = if old_rem > 0 { FieldChange::Set(old_rem) } else { FieldChange::Clear };
                undo.entry(m.key.account.clone()).or_default().push(Op::SetFollowUp {
                    keys: vec![m.key.clone()],
                    start: None,
                    due: None,
                    reminder: Some(back_r),
                    title: Some(Some(title)),
                });
            }
        }
        // The flag off before the keyword goes on — except on Exchange, where Mark Complete
        // keeps the flag's own text and title ([MS-OXOFLAG] 3.1.4.2): the keyword's status 1
        // alone takes the flag's place, and SetFlag off would delete them.
        for (a, keys) in &flagged {
            if !ews.contains(a.as_str()) {
                self.apply_op(a, Op::SetFlag { keys: keys.clone(), flag: Flag::Flagged, on: false, all: false });
            }
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
        self.follow_up_due_any(m)
    }

    /// When the follow-up is due whether or not it's still flagged: a completed one is still
    /// due, as the To-Do List shows it.
    pub fn follow_up_due_any(&self, m: &Summary) -> Option<i64> {
        if m.flags.follow_up.due > 0 {
            // A floating midnight, read as a UTC day, never in the local zone.
            return Some(quatrelle_model::dates::utc_day(m.flags.follow_up.due));
        }
        self.prefs.follow_up.get(&follow_up_id(m)).copied()
    }

    /// A kept message by its id ([`MessageKey::to_id`]), as the To-Do List's rows name one: the
    /// memory cache first, then the folders read from the disk cache (a folder never opened
    /// this session).
    pub fn message_by_id(&self, id: &str) -> Option<Arc<Summary>> {
        let key = MessageKey::from_id(id)?;
        if let Some(st) = self.cache.get(&(key.account.clone(), key.folder.clone()))
            && let Some(m) = st.messages.iter().find(|m| m.key == key)
        {
            return Some(m.clone());
        }
        let g = self.disk_lists.lock().unwrap_or_else(|e| e.into_inner());
        g.map.get(&(key.account.clone(), key.folder.clone()))?.iter().find(|m| m.key == key).map(|m| Arc::new(m.clone()))
    }

    /// When a flag's follow-up starts: the server's own, else the day kept here.
    pub fn follow_up_start(&self, m: &Summary) -> Option<i64> {
        if m.flags.follow_up.start > 0 {
            // A floating midnight, read as a UTC day, never in the local zone.
            return Some(quatrelle_model::dates::utc_day(m.flags.follow_up.start));
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

    /// What a flag's follow-up asks for: the title the server wrote (Exchange's
    /// `PidLidToDoTitle`), else the one kept here where the server has nowhere for it.
    pub fn follow_up_title<'a>(&'a self, m: &'a Summary) -> Option<&'a str> {
        if !m.flags.follow_up.title.is_empty() {
            return Some(m.flags.follow_up.title.as_str());
        }
        self.prefs.follow_up_title.get(&follow_up_id(m)).map(String::as_str)
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

/// A title put back as it was (`None` or empty: none), within [`MAX_FOLLOW_UP`].
fn put_title(titles: &mut BTreeMap<String, String>, id: String, title: Option<String>) {
    match title {
        Some(t) if !t.is_empty() && (titles.len() < MAX_FOLLOW_UP || titles.contains_key(&id)) => {
            titles.insert(id, t);
        }
        _ => {
            titles.remove(&id);
        }
    }
}
