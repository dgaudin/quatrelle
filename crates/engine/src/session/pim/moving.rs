// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Moving appointments (the calendar's drag and drop): the whole event, or one occurrence of a
//! recurring one, which becomes an instance of its own (`RECURRENCE-ID`, RFC 5545 §3.8.4.4);
//! and how the guests of a meeting the user organizes hear of a change.
//!
//! Who tells the guests depends on where the calendar lives. Exchange sends meeting updates
//! itself when asked to (EWS `SendToAllAndSaveCopy`, as Outlook's "Send update"; Microsoft 365
//! always does on a change), so for those accounts the change is marked `notify` and the next
//! sync carries it — mailing a `REQUEST` from here as well would invite everyone twice. Anywhere
//! else (CalDAV, a calendar kept on this computer) the `REQUEST` goes by mail, with its
//! `SEQUENCE` raised, as it always has.

use super::*;

impl Session {
    /// Whether the user organizes this event and it has guests to tell.
    pub fn organizes(&self, e: &CalEvent) -> bool {
        self.has_guests(e) && e.organizer.as_ref().is_some_and(|o| self.is_my_address(&o.email))
    }

    /// Whether this is a copy of a meeting someone else organizes. `SEQUENCE` is the organizer's
    /// revision number (RFC 5545 §3.8.7.4): the user's changes to such a copy leave it as sent,
    /// else the organizer's next update would be judged older and ignored.
    pub(crate) fn organized_by_other(&self, e: &CalEvent) -> bool {
        e.organizer.as_ref().is_some_and(|o| !o.email.trim().is_empty() && !self.is_my_address(super::super::invite::without_mailto(&o.email)))
    }

    /// Whether an event has someone to tell: an attendee with an address that isn't one of
    /// the user's. Exchange lists the organizer among the attendees, so an appointment the user
    /// made alone can come back with the user as its one attendee: nobody's meeting to announce
    /// (asked to send it, the server refuses for want of recipients).
    pub fn has_guests(&self, e: &CalEvent) -> bool {
        e.attendees.iter().any(|a| {
            let email = super::super::invite::without_mailto(&a.email);
            email.contains('@') && !self.is_my_address(email)
        })
    }

    /// Whether this address is one of the accounts'.
    pub(super) fn is_my_address(&self, email: &str) -> bool {
        !email.trim().is_empty()
            && self.accounts.iter().any(|a| a.config.email.eq_ignore_ascii_case(email) || a.config.username.eq_ignore_ascii_case(email))
    }

    /// Whether an account's server tells the guests of a change itself (Exchange, on-premises
    /// or Microsoft 365), so that nothing is mailed from here.
    pub fn server_tells_guests(&self, account: &str) -> bool {
        self.account(account).is_some_and(|a| a.config.ews.is_some() || a.config.graph.is_some())
    }

    /// Whether one occurrence of a recurring event in this account can be changed on its own:
    /// every calendar can keep an instance of its own, but Microsoft 365 (Graph) reads only
    /// whole series back, so a moved occurrence would come back on its old day at the next
    /// sync; there, only the whole series moves.
    pub fn occurrence_moves(&self, account: &str) -> bool {
        self.account(account).is_none_or(|a| a.config.graph.is_none())
    }

    /// Move an event to `start`..`end` (in the event's own times: an all-day event's are UTC
    /// midnights), or one occurrence of a recurring one (`occurrence`: the start its rule gives
    /// that occurrence). `send`: the guests of a meeting the user organizes hear of it. Undo puts
    /// it back.
    pub fn move_event(&mut self, uid: &str, occurrence: Option<i64>, start: i64, end: i64, send: bool) -> CmdResult {
        self.move_event_to(uid, occurrence, start, end, send, true)
    }

    /// Undo's half of [`Session::move_event`]: back where it was, without remembering again.
    pub(crate) fn move_event_back(&mut self, uid: &str, occurrence: Option<i64>, start: i64, end: i64, send: bool) -> CmdResult {
        self.move_event_to(uid, occurrence, start, end, send, false)
    }

    fn move_event_to(&mut self, uid: &str, occurrence: Option<i64>, start: i64, end: i64, send: bool, remember: bool) -> CmdResult {
        let (account, before) =
            self.find_event(uid).map(|(a, e)| (a.to_string(), e.event.clone())).ok_or_else(|| CmdError::Params(format!("no event `{uid}`")))?;
        let real = before.uid.clone();
        // One occurrence of a series: an instance of its own. (An occurrence asked of an event
        // that doesn't recur is the event itself.)
        let was = match occurrence.filter(|_| before.rrule.is_some()) {
            Some(at) => {
                let instance =
                    self.pim.get(&account).and_then(|p| p.events.iter().find(|e| e.event.uid == real && e.event.recurrence_id == Some(at)));
                let (old_start, old_end) = instance.map_or((at, at + (before.end - before.start)), |e| (e.event.start, e.event.end));
                self.update_occurrence(&real, at, |e| {
                    e.start = start;
                    e.end = end;
                })?;
                (Some(at), old_start, old_end)
            }
            None => {
                let delta = start - before.start;
                self.update_event(&real, |e| {
                    e.start = start;
                    e.end = end;
                    // The days taken out of the series, and those added to it, move with it.
                    for x in e.exdates.iter_mut().chain(e.rdates.iter_mut()) {
                        *x = x.saturating_add(delta);
                    }
                })?;
                // So do the occurrences changed on their own: each keeps its place in the series.
                if before.rrule.is_some() && delta != 0 {
                    let p = self.pim_mut(&account);
                    let mut remote = false;
                    for e in p.events.iter_mut().filter(|e| e.event.uid == real && e.event.recurrence_id.is_some()) {
                        e.event.recurrence_id = e.event.recurrence_id.map(|r| r.saturating_add(delta));
                        e.event.start = e.event.start.saturating_add(delta);
                        e.event.end = e.event.end.saturating_add(delta);
                        e.dirty |= e.calendar != LOCAL;
                        remote |= e.calendar != LOCAL;
                    }
                    self.after_pim_change(&account, remote);
                }
                // Undo comes through here as well, and Webex's meeting follows it back.
                if let Some(after) = self.find_event(&real).map(|(_, e)| e.event.clone()) {
                    self.webex_follow(Some(&before), &after);
                }
                (None, before.start, before.end)
            }
        };
        let told = self.tell_guests(&account, &real, was.0, send)?;
        if remember {
            self.remember(Undo::EventMove { uid: real.clone(), occurrence: was.0, start: was.1, end: was.2, send });
        }
        Ok(serde_json::json!({"uid": real, "occurrence": was.0, "start": start, "end": end, "allDay": before.all_day, "told": told}))
    }

    /// Change one occurrence of a recurring event (`at`: the start its rule gives it): an
    /// instance of its own, cut from the series the first time, changed in place after. Goes to
    /// the server with the series (CalDAV writes them together; Exchange is asked for the
    /// occurrence's own item).
    pub fn update_occurrence(&mut self, uid: &str, at: i64, change: impl FnOnce(&mut CalEvent)) -> CmdResult {
        let (account, master) =
            self.find_event(uid).map(|(a, e)| (a.to_string(), e.clone())).ok_or_else(|| CmdError::Params(format!("no event `{uid}`")))?;
        if master.event.rrule.is_none() {
            return self.update_event(uid, change);
        }
        if self.pim.get(&account).is_some_and(|p| p.calendars.iter().any(|c| c.id == master.calendar && c.read_only)) {
            return Err(CmdError::Params("this calendar can't be changed".into()));
        }
        // The occurrence must be one the rule gives (and not taken out already).
        if master.event.exdates.contains(&at) || !ical::starts(&master.event, at.saturating_add(1)).contains(&at) {
            return Err(CmdError::Params(format!("the series has no occurrence at {at}")));
        }
        let real = master.event.uid.clone();
        let others = self.organized_by_other(&master.event);
        let p = self.pim_mut(&account);
        let existing = p.events.iter().position(|e| e.event.uid == real && e.event.recurrence_id == Some(at));
        let mut e = match existing.and_then(|i| p.events.get(i)) {
            Some(x) => x.event.clone(),
            None => {
                let len = master.event.end - master.event.start;
                CalEvent {
                    recurrence_id: Some(at),
                    rrule: None,
                    exdates: Vec::new(),
                    rdates: Vec::new(),
                    start: at,
                    end: at + len,
                    ..master.event.clone()
                }
            }
        };
        change(&mut e);
        check_event(&e)?;
        e.uid = real.clone();
        e.recurrence_id = Some(at);
        e.rrule = None;
        e.exdates.clear();
        e.rdates.clear();
        e.sequence = e.sequence.max(master.event.sequence);
        if !others {
            e.sequence = e.sequence.saturating_add(1);
        }
        let remote = master.calendar != LOCAL;
        match existing.and_then(|i| p.events.get_mut(i)) {
            Some(stored) => {
                stored.event = e;
                stored.changed_here(remote);
            }
            None => p.events.push(StoredEvent {
                calendar: master.calendar.clone(),
                href: String::new(),
                etag: String::new(),
                event: e,
                dirty: remote,
                notify: false,
                respond: None,
                changed_too: false,
            }),
        }
        self.after_pim_change(&account, remote);
        Ok(serde_json::json!({"uid": real, "occurrence": at}))
    }

    /// Tell the guests of a meeting the user organizes of a change to it (`occurrence`: to that
    /// instance alone), the way its account does it; nothing when `send` is off, or when it's
    /// nobody's meeting. Says who told them: `server`, `mail` or `none`.
    pub(crate) fn tell_guests(&mut self, account: &str, uid: &str, occurrence: Option<i64>, send: bool) -> Result<&'static str, CmdError> {
        let Some(stored) =
            self.pim.get(account).and_then(|p| p.events.iter().find(|e| e.event.uid == uid && e.event.recurrence_id == occurrence)).cloned()
        else {
            return Ok("none");
        };
        if !self.organizes(&stored.event) {
            return Ok("none");
        }
        if self.server_tells_guests(account) {
            // The next sync asks the server to send the update. Not sending this one never takes
            // back what's still to go: a meeting made private before its invitation went out is
            // still sent to its guests.
            let p = self.pim_mut(account);
            let mut changed = false;
            for e in p.events.iter_mut().filter(|e| e.event.uid == uid && e.event.recurrence_id == occurrence) {
                changed |= send && !e.notify;
                e.notify |= send;
            }
            if changed {
                self.save_pim(account);
            }
            return Ok(if send { "server" } else { "none" });
        }
        if !send {
            return Ok("none");
        }
        self.send_invitations(&stored.event, account, "REQUEST")?;
        Ok("mail")
    }
}
