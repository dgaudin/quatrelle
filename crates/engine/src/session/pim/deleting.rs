//! Deleting appointments, in the calendar they're in, and the occurrences changed here whose
//! series is gone.

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
                    m.event.sequence = m.event.sequence.saturating_add(1);
                    remote = m.calendar != LOCAL;
                    m.dirty |= remote;
                }
            }
            _ => {
                let gone: Vec<StoredEvent> = p.events.iter().filter(|e| here(e)).cloned().collect();
                for g in gone.iter().filter(|g| !g.href.is_empty()) {
                    remote = true;
                    p.deleted.push(Tombstone { href: g.href.clone(), etag: g.etag.clone(), calendar: g.calendar.clone() });
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
            self.notice(format!("{what}: a change to one occurrence couldn't be sent: its series is no longer on the server"), true);
        }
    }
}
