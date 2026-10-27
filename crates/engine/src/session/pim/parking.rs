//! Other people's calendars set aside while the calendar isn't shown, as Outlook reads a shared
//! calendar from its offline file only to show it: their appointments (25,000 of them in the
//! owner's organisation, two thirds of the app's data) were held in memory all day, and written
//! again with every save of the calendar and contacts.
//!
//! Set aside, they're in a document of their own ([`PARKED_DOC`]) until the calendar is shown
//! again. One changed here and not yet on the server stays (its sync sends it), and so do the
//! account's own calendars (reminders, invitations and the day's agenda read them).

use std::collections::HashSet;

use super::*;

/// Away from the calendar this long, other people's appointments are set aside (coming back to
/// it a moment later doesn't read them again).
pub(super) const PARK_AFTER: i64 = 5 * 60;

/// The appointments set aside, an account's.
pub const PARKED_DOC: &str = "pim-parked.json";

/// Whether an appointment is set aside away from the calendar: another person's, not waiting to
/// go to the server.
fn parks(e: &StoredEvent) -> bool {
    !own_calendar(&e.calendar) && !e.dirty && !e.notify && e.respond.is_none()
}

impl Session {
    /// Set aside or brought back as the calendar is left or shown (looked at for every frame).
    pub(crate) fn park_calendars(&mut self) {
        let Some(store) = self.store.clone() else { return };
        if self.module == crate::Module::Calendar {
            self.away_from_calendar = None;
            let parked: Vec<String> = self.parked.iter().cloned().collect();
            for a in parked {
                self.unpark(&store, &a);
            }
            return;
        }
        let since = *self.away_from_calendar.get_or_insert(self.clock.now);
        if self.clock.now.saturating_sub(since) < PARK_AFTER {
            return;
        }
        let accounts: Vec<String> = self.pim.keys().filter(|a| !self.parked.contains(*a)).cloned().collect();
        for a in accounts {
            self.park(&store, &a);
        }
    }

    /// Other people's appointments out of memory, into their document (with those already set
    /// aside of calendars not among them), then the calendar saved without them: stopped
    /// between the two, both have them, and nothing is lost.
    fn park(&mut self, store: &Store, account: &str) {
        let Some(p) = self.pim.get_mut(account) else { return };
        if !p.events.iter().any(parks) {
            self.parked.insert(account.to_string());
            return;
        }
        let (aside, kept): (Vec<StoredEvent>, Vec<StoredEvent>) = std::mem::take(&mut p.events).into_iter().partition(parks);
        p.events = kept;
        let here: HashSet<String> = aside.iter().map(|e| e.calendar.clone()).collect();
        let mut doc: Vec<StoredEvent> = store.load_account_doc(account, PARKED_DOC).unwrap_or_default();
        doc.retain(|e| !here.contains(&e.calendar));
        doc.extend(aside);
        if let Err(e) = store.save_account_doc(account, PARKED_DOC, &doc) {
            // Kept in memory, as before.
            log::warn!("{account}: other people's appointments couldn't be set aside: {e}");
            if let Some(p) = self.pim.get_mut(account) {
                p.events.extend(doc.into_iter().filter(|e| here.contains(&e.calendar)));
            }
            return;
        }
        if let Some(p) = self.pim.get(account)
            && let Err(e) = store.save_account_doc(account, PIM_DOC, p)
        {
            log::warn!("{account}: the calendar couldn't be saved without other people's appointments: {e}");
        }
        log::info!("{account}: {} of other people's appointments set aside while the calendar isn't shown", doc.len());
        self.parked.insert(account.to_string());
        self.pim_revision = self.pim_revision.wrapping_add(1);
    }

    /// An account's appointments set aside now (none when they're in memory): read, not
    /// brought back.
    pub(crate) fn set_aside(&self, account: &str) -> Vec<StoredEvent> {
        match &self.store {
            Some(store) if self.parked.contains(account) => store.load_account_doc(account, PARKED_DOC).unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// The calendars a sync read again while theirs were set aside: what it read wins.
    pub(super) fn read_while_parked(&mut self, account: &str, sync: &quatrelle_model::pim::DavSync) {
        if self.parked.contains(account) {
            self.read_aside.entry(account.to_string()).or_default().extend(sync.synced_calendars.iter().filter(|c| !own_calendar(c)).cloned());
        }
    }

    /// Other people's appointments back in memory (a calendar read since keeps what was read; one
    /// kept in memory isn't doubled; one of a calendar no longer listed stays out), then nothing
    /// left aside. The calendar's document takes them at its next save: lost meanwhile, they're
    /// only a copy of the server's, read again when the calendar is shown.
    fn unpark(&mut self, store: &Store, account: &str) {
        self.parked.remove(account);
        let read = self.read_aside.remove(account).unwrap_or_default();
        let doc: Vec<StoredEvent> = store.load_account_doc(account, PARKED_DOC).unwrap_or_default();
        if doc.is_empty() {
            return;
        }
        let p = self.pim_mut(account);
        let kept: HashSet<(String, String, Option<i64>)> =
            p.events.iter().map(|e| (e.calendar.clone(), e.event.uid.clone(), e.event.recurrence_id)).collect();
        let listed: HashSet<String> = p.calendars.iter().map(|c| c.id.clone()).collect();
        let n = doc.len();
        p.events.extend(doc.into_iter().filter(|e| {
            listed.contains(&e.calendar)
                && !read.contains(&e.calendar)
                && !kept.contains(&(e.calendar.clone(), e.event.uid.clone(), e.event.recurrence_id))
        }));
        self.pim_revision = self.pim_revision.wrapping_add(1);
        if let Err(e) = store.save_account_doc(account, PARKED_DOC, &Vec::<StoredEvent>::new()) {
            log::warn!("{account}: the appointments set aside stay on disk too: {e}");
        }
        log::info!("{account}: {n} of other people's appointments read back for the calendar");
    }
}
