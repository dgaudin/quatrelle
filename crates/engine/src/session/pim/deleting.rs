// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Deleting appointments, in the calendar they're in, and the occurrences changed here whose
//! series is gone.

use quatrelle_model::pim::{HeldDeletion, HeldKind, MAX_HELD};

use super::*;

impl Session {
    /// Delete an event, or one occurrence of a recurring one (`occurrence`: its start), in the
    /// calendar it's in: the copies of a meeting in colleagues' calendars stay theirs (all
    /// those under its UID went, and were deleted from the server where the user may change
    /// them). An occurrence changed on its own whose series is gone goes alone.
    pub fn delete_event(&mut self, uid: &str, occurrence: Option<i64>) -> CmdResult {
        let found = self.find_event(uid).map(|(a, e)| (a.to_string(), e.calendar.clone(), e.event.uid.clone()));
        let (account, calendar, real) = match (found, occurrence) {
            (Some(f), _) => f,
            (None, Some(at)) => self.lone_occurrence(uid, at).ok_or_else(|| CmdError::Params(format!("no event `{uid}`")))?,
            (None, None) => return Err(CmdError::Params(format!("no event `{uid}`"))),
        };
        if self.pim.get(&account).is_some_and(|p| p.calendars.iter().any(|c| c.id == calendar && c.read_only)) {
            return Err(CmdError::Params("this calendar can't be changed".into()));
        }
        let others = self.find_event(&real).is_some_and(|(_, e)| self.organized_by_other(&e.event));
        let synced = self.account(&account).is_some_and(|a| !a.is_local());
        let p = self.pim_mut(&account);
        let here = |e: &StoredEvent| e.calendar == calendar && e.event.uid == real;
        let mut remote = false;
        match occurrence {
            Some(at) if p.events.iter().any(|e| here(e) && e.event.rrule.is_some()) => {
                // One occurrence: an exception on the rule, and any changed instance goes.
                p.events.retain(|e| !(here(e) && e.event.recurrence_id == Some(at)));
                if let Some(m) = p.events.iter_mut().find(|e| here(e) && e.event.recurrence_id.is_none()) {
                    if !m.event.exdates.contains(&at) {
                        m.event.exdates.push(at);
                    }
                    if !others {
                        m.event.sequence = m.event.sequence.saturating_add(1);
                    }
                    remote = m.calendar != LOCAL;
                    m.changed_here(remote);
                }
            }
            _ => {
                let gone: Vec<StoredEvent> = p.events.iter().filter(|e| here(e)).cloned().collect();
                for g in &gone {
                    if !g.href.is_empty() {
                        remote = true;
                        p.deleted.push(Tombstone { href: g.href.clone(), etag: g.etag.clone(), calendar: g.calendar.clone() });
                    } else if synced && g.calendar != LOCAL && !quatrelle_model::pim::is_web_calendar(&g.calendar) {
                        // Not on the server as far as this computer knows, but a sync may be
                        // making it there now (made, then deleted at once), and its result would
                        // bring it back: held until a result says where it was made.
                        hold(p, g);
                    }
                }
                p.events.retain(|e| !here(e));
            }
        }
        self.after_pim_change(&account, remote);
        Ok(serde_json::json!({"deleted": real}))
    }

    /// An occurrence changed on its own whose series is in no calendar of the account any more
    /// (`at`: its place in the series): its account, calendar and UID. Kept as Exchange
    /// read it then, the series is now its single appointments: the change could never be
    /// sent, nor the occurrence deleted.
    pub(super) fn lone_occurrence(&self, uid: &str, at: i64) -> Option<(String, String, String)> {
        self.pim.iter().find_map(|(account, p)| {
            let e = p.events.iter().find(|e| e.event.uid == uid && e.event.recurrence_id == Some(at) && own_calendar(&e.calendar))?;
            let series = p.events.iter().any(|m| m.calendar == e.calendar && m.event.uid == e.event.uid && m.event.recurrence_id.is_none());
            (!series).then(|| (account.clone(), e.calendar.clone(), e.event.uid.clone()))
        })
    }

    /// After a sync: occurrences changed here whose series the server no longer has (deleted,
    /// or read since as single appointments) are let go, and the user told once — sent again
    /// at every sync, each was refused with "the series isn't on the server yet", forever.
    /// (A calendar subscribed to by its link never goes to the server: an occurrence it
    /// publishes without its series stays.)
    pub(super) fn drop_lone_occurrences(&mut self, account: &str) {
        let Some(p) = self.pim.get_mut(account) else { return };
        let lone = |e: &StoredEvent, all: &[StoredEvent]| {
            e.event.recurrence_id.is_some()
                && e.href.is_empty()
                && e.calendar != LOCAL
                && !quatrelle_model::pim::is_web_calendar(&e.calendar)
                && !all.iter().any(|m| m.calendar == e.calendar && m.event.uid == e.event.uid && m.event.recurrence_id.is_none())
        };
        let gone: Vec<String> = p.events.iter().filter(|e| lone(e, &p.events)).map(|e| e.event.summary.clone()).collect();
        if gone.is_empty() {
            return;
        }
        let all = p.events.clone();
        p.events.retain(|e| !lone(e, &all));
        for what in gone {
            self.notice_with(
                "{event}: a change to one occurrence couldn't be sent: its series is no longer on the server",
                &[("event", &what)],
                true,
            );
        }
    }
}

/// An appointment deleted here with no item on the server, held ([`Pim::held`]): once, at most
/// [`MAX_HELD`] of them (the oldest go first).
fn hold(p: &mut Pim, e: &StoredEvent) {
    let (uid, recurrence_id) = (&e.event.uid, e.event.recurrence_id);
    hold_item(&mut p.held, HeldDeletion { uid: uid.clone(), recurrence_id, kind: HeldKind::Event });
}

/// A task, note or contact deleted here with no item on the server ([`Pim::held`]).
pub(super) fn hold_other(held: &mut Vec<HeldDeletion>, kind: HeldKind, uid: &str) {
    hold_item(held, HeldDeletion { uid: uid.to_string(), recurrence_id: None, kind });
}

fn hold_item(held: &mut Vec<HeldDeletion>, h: HeldDeletion) {
    if !held.contains(&h) {
        held.push(h);
    }
    let over = held.len().saturating_sub(MAX_HELD);
    held.drain(..over);
}

/// What a sync made on the server that was deleted here meanwhile ([`Pim::held`]): deleted
/// there as anything deleted here, now that the server says where it is, and that sync's
/// copy of it not taken in. One made here again since under the same UID (no item yet, or
/// that one) is what the server made: it stays, and takes its item as usual. The places in
/// `made` of those deleted.
pub(super) fn settle_held(p: &mut Pim, made: &[StoredEvent]) -> Vec<usize> {
    let mut deleted = Vec::new();
    for (at, x) in made.iter().enumerate() {
        let same = |uid: &str, rid: Option<i64>| uid == x.event.uid && rid == x.event.recurrence_id;
        // (Made where the server didn't say: still held.)
        let Some(h) = p.held.iter().position(|h| h.kind.is_event() && same(&h.uid, h.recurrence_id)).filter(|_| !x.href.is_empty()) else { continue };
        p.held.remove(h);
        if p.events.iter().any(|e| same(&e.event.uid, e.event.recurrence_id) && (e.href.is_empty() || e.href == x.href)) {
            continue;
        }
        if !p.deleted.iter().any(|t| t.href == x.href) {
            p.deleted.push(Tombstone { href: x.href.clone(), etag: x.etag.clone(), calendar: x.calendar.clone() });
        }
        deleted.push(at);
    }
    deleted
}

/// What a sync made on the server that was a task, note or contact deleted here meanwhile
/// ([`Pim::held`]): deleted there as anything deleted here, now that the server says where it is
/// (`made`: each one's name here, its item and its version there). Made here again since under
/// the same name (no item yet, or that one), it stays. The places in `made` of those deleted.
pub(super) fn settle_held_other<'a>(p: &mut Pim, kind: HeldKind, made: impl Iterator<Item = (&'a str, &'a str, &'a str)>) -> Vec<usize> {
    let mut deleted = Vec::new();
    for (at, (uid, href, etag)) in made.enumerate() {
        // (Made where the server didn't say: still held.)
        let Some(h) = p.held.iter().position(|h| h.kind == kind && h.uid == uid).filter(|_| !href.is_empty()) else { continue };
        p.held.remove(h);
        let again = match kind {
            HeldKind::Task => p.tasks.iter().any(|t| t.task.uid == uid && (t.href.is_empty() || t.href == href)),
            HeldKind::Note => p.notes.iter().any(|n| n.note.uid == uid && (n.href.is_empty() || n.href == href)),
            HeldKind::Card => p.cards.iter().any(|c| c.card.uid == uid && (c.href.is_empty() || c.href == href)),
            HeldKind::Event => false,
        };
        if again {
            continue;
        }
        if !p.deleted.iter().any(|t| t.href == href) {
            p.deleted.push(Tombstone { href: href.to_string(), etag: etag.to_string(), ..Default::default() });
        }
        deleted.push(at);
    }
    deleted
}
