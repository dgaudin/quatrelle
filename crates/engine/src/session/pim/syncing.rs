//! What a CalDAV / CardDAV (or Exchange) sync found, merged into an account's calendars and
//! address books: changes made here since stay (and go to the server next time); everything else
//! follows the server.
//!
//! Saved, and the address book worked out again, only when something changed. While the
//! calendar is shown, other people's appointments are in memory (25,000 in the owner's
//! organisation), and a sync that found nothing new, every minute or two, wrote them all again,
//! compressed, on the window's thread; each appointment it read was looked for through all of
//! them, twice. The window answered late (27/10).

use std::collections::{HashMap, HashSet};

use quatrelle_model::pim::is_web_calendar;

use super::*;

impl Session {
    /// Merge what a CalDAV / CardDAV sync found. Changes made here since stay (and go to the
    /// server next time); everything else follows the server.
    pub fn apply_dav(&mut self, account: &str, sync: &quatrelle_model::pim::DavSync) {
        self.read_while_parked(account, sync);
        let p = self.pim_mut(account);
        // As it was, to tell whether the sync changed anything: the appointments by their digest,
        // the rest as it is.
        let events_were = events_digest(&p.events);
        let events = std::mem::take(&mut p.events);
        let was = p.clone();
        p.events = events;
        p.caldav_home = sync.caldav_home.clone().or_else(|| Some(String::new()));
        p.carddav_home = sync.carddav_home.clone().or_else(|| Some(String::new()));
        p.dav_checked = Some(sync.checked);
        p.deleted.retain(|t| !sync.deleted.contains(&t.href));
        // What was stored: its address and version, and no longer to send (unless it changed
        // again meanwhile).
        for x in &sync.pushed_events {
            for e in p.events.iter_mut().filter(|e| e.event.uid == x.event.uid && e.event.recurrence_id == x.event.recurrence_id) {
                e.href = x.href.clone();
                e.etag = x.etag.clone();
                // Stored in another calendar (a new event goes to the server's default one).
                e.calendar = x.calendar.clone();
                e.dirty &= e.event != x.event;
                // The server told the guests with that change; a later change asks again.
                e.notify &= e.dirty;
            }
        }
        // Notes stored on the server: their item there, and no longer to send.
        for x in &sync.pushed_notes {
            for n in p.notes.iter_mut().filter(|n| n.note.uid == x.note.uid) {
                n.href = x.href.clone();
                n.etag = x.etag.clone();
                n.dirty &= n.note != x.note;
            }
        }
        // The server's notes, by their item there; those changed here (and those never sent)
        // stay.
        if let Some(server) = &sync.notes {
            p.notes.retain(|n| n.dirty || n.href.is_empty());
            for x in server {
                // Deleted here, the server not told yet (offline): it stays deleted.
                let deleting = p.deleted.iter().any(|d| d.href == x.href);
                if !deleting && !p.notes.iter().any(|n| !n.href.is_empty() && n.href == x.href) {
                    p.notes.push(x.clone());
                }
            }
        }
        tasks::merge_tasks(p, sync);
        for x in &sync.pushed_cards {
            for c in p.cards.iter_mut().filter(|c| c.card.uid == x.card.uid) {
                c.href = x.href.clone();
                c.etag = x.etag.clone();
                c.book = x.book.clone();
                c.dirty &= c.card != x.card;
            }
        }
        if let Some(server) = &sync.calendars {
            // The calendar on this computer goes once the server has calendars, unless it holds
            // events (they stay there).
            let keep_local = server.is_empty() || p.events.iter().any(|e| e.calendar == LOCAL);
            let mut list: Vec<quatrelle_model::pim::CalendarInfo> = server.clone();
            if keep_local && let Some(l) = p.calendars.iter().find(|c| c.id == LOCAL) {
                list.push(l.clone());
            }
            // Only some looked at (this account's own, or a calendar just ticked): the others
            // stay, with their events, where the side pane lists them.
            if sync.partial {
                let mut merged: Vec<quatrelle_model::pim::CalendarInfo> = p
                    .calendars
                    .iter()
                    .filter(|c| c.id != LOCAL || list.iter().any(|x| x.id == LOCAL))
                    .map(|c| list.iter().find(|x| x.id == c.id).cloned().unwrap_or_else(|| c.clone()))
                    .collect();
                for x in list {
                    if !merged.iter().any(|m| m.id == x.id) {
                        merged.push(x);
                    }
                }
                list = merged;
            }
            // Calendars subscribed to by their links aren't the server's: they stay (issue #7).
            let web: Vec<_> = p.calendars.iter().filter(|c| is_web_calendar(&c.id) && !list.iter().any(|x| x.id == c.id)).cloned().collect();
            list.extend(web);
            let listed: HashSet<&str> = list.iter().map(|c| c.id.as_str()).collect();
            p.events.retain(|e| e.calendar == LOCAL || listed.contains(e.calendar.as_str()) || e.dirty);
            merge_events(p, sync);
            p.calendars = list;
        }
        self.note_new_calendars();
        let p = self.pim_mut(account);
        if let Some(server) = &sync.books {
            let keep_local = server.is_empty() || p.cards.iter().any(|c| c.book == LOCAL);
            let mut list: Vec<quatrelle_model::pim::BookInfo> = server.clone();
            if keep_local && let Some(l) = p.books.iter().find(|b| b.id == LOCAL) {
                list.push(l.clone());
            }
            if sync.partial {
                let kept: Vec<_> = p.books.iter().filter(|b| b.id != LOCAL && !list.iter().any(|x| x.id == b.id)).cloned().collect();
                list.extend(kept);
            }
            let listed: HashSet<&str> = list.iter().map(|b| b.id.as_str()).collect();
            p.cards.retain(|c| c.book == LOCAL || listed.contains(c.book.as_str()) || c.dirty);
            merge_cards(p, sync);
            p.books = list;
        }
        p.ensure_defaults();
        self.drop_lone_occurrences(account);
        for e in sync.errors.iter().take(3) {
            self.notice(format!("calendars and contacts: {e}"), true);
        }
        // Invitations the server answered, and copies of invitations to answer instead.
        if self.settle_answers(account, sync) {
            log::info!("{account}: invitations' answers to send");
            self.jobs.push(Job::SyncDav { account: account.to_string() });
        }
        // Saved only when something changed (when the server was last asked isn't a change), and
        // the address book worked out again only when the contacts did.
        let p = self.pim_mut(account);
        let events_changed = events_digest(&p.events) != events_were;
        let events = std::mem::take(&mut p.events);
        let checked = p.dav_checked;
        let contacts_changed = p.cards != was.cards || p.books != was.books;
        let rest_changed = *p != Pim { dav_checked: checked, ..was };
        p.events = events;
        if contacts_changed {
            self.save_pim(account);
        } else if events_changed || rest_changed {
            self.write_pim(account);
        }
    }
}

/// A digest of an account's appointments as kept, whatever their order: what a sync changes in
/// one (its item on the server, its version, its calendar) or what changed here (sent or not,
/// answered or not, its sequence).
fn events_digest(events: &[StoredEvent]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut sum: u64 = events.len() as u64;
    for e in events {
        let mut h = std::hash::DefaultHasher::new();
        (&e.calendar, &e.href, &e.etag, &e.event.uid, e.event.recurrence_id, e.event.sequence).hash(&mut h);
        (e.dirty, e.notify, &e.respond).hash(&mut h);
        sum = sum.wrapping_add(h.finish());
    }
    sum
}

/// The server's appointments of the calendars it read in place of those kept here: one gone
/// from the server goes, one changed here stays (by its address, or its UID for one never
/// sent), one the server has as it's kept here stays where it is, and the others come in. The
/// same item there (its address) and an appointment kept here that never went (no address) are
/// replaced. Two items may share a UID — some Exchange series arrive as separate items keeping
/// the series' UID — and each is kept, since the server's address tells them apart. One CalDAV
/// item holds a series and its moved occurrences: each is kept too. Each is found by an index,
/// not by a pass through them all.
fn merge_events(p: &mut Pim, sync: &quatrelle_model::pim::DavSync) {
    type Key<'a> = (&'a str, Option<i64>);
    let synced: HashSet<&str> = sync.synced_calendars.iter().map(String::as_str).collect();
    let kept: HashSet<&str> = sync.kept.iter().map(String::as_str).collect();
    // Read again (a whole calendar read): weighed against what came below, rather than taken out
    // and put back.
    let read: HashSet<Key<'_>> = sync.events.iter().filter(|x| !x.href.is_empty()).map(|x| (x.href.as_str(), x.event.recurrence_id)).collect();
    p.events.retain(|e| {
        !synced.contains(e.calendar.as_str())
            || e.dirty
            || e.href.is_empty()
            || kept.contains(e.href.as_str())
            || read.contains(&(e.href.as_str(), e.event.recurrence_id))
    });
    let (gone, add) = {
        let mut edited_href: HashSet<&str> = HashSet::new();
        let mut edited_uid: HashSet<&str> = HashSet::new();
        let mut by_href: HashMap<Key<'_>, Vec<usize>> = HashMap::new();
        let mut unsent: HashMap<Key<'_>, Vec<usize>> = HashMap::new();
        for (i, e) in p.events.iter().enumerate() {
            if e.dirty {
                edited_href.insert(&e.href);
                if e.href.is_empty() {
                    edited_uid.insert(&e.event.uid);
                }
            } else if e.href.is_empty() && !is_web_calendar(&e.calendar) {
                // (One of a calendar subscribed to by its link never goes to the server: it
                // isn't replaced by the server's with the same UID.)
                unsent.entry((&e.event.uid, e.event.recurrence_id)).or_default().push(i);
            }
            by_href.entry((&e.href, e.event.recurrence_id)).or_default().push(i);
        }
        let mut gone: HashSet<usize> = HashSet::new();
        // What comes in, a later one with the same address (or UID, without one) in place of an
        // earlier one, as each replaced what came before it.
        let mut add: Vec<Option<&StoredEvent>> = Vec::new();
        let mut added_href: HashMap<Key<'_>, usize> = HashMap::new();
        let mut added_unsent: HashMap<Key<'_>, usize> = HashMap::new();
        for x in sync.events.iter().filter(|x| synced.contains(x.calendar.as_str())) {
            if edited_href.contains(x.href.as_str()) || edited_uid.contains(x.event.uid.as_str()) {
                continue;
            }
            let rid = x.event.recurrence_id;
            let mut same: Vec<usize> = Vec::new();
            if !x.href.is_empty() {
                same.extend(by_href.get(&(x.href.as_str(), rid)).into_iter().flatten().copied());
                if let Some(slot) = added_href.remove(&(x.href.as_str(), rid)).and_then(|a| add.get_mut(a)) {
                    *slot = None;
                }
            }
            same.extend(unsent.get(&(x.event.uid.as_str(), rid)).into_iter().flatten().copied());
            same.retain(|i| !gone.contains(i));
            if let [one] = same.as_slice()
                && p.events.get(*one) == Some(x)
            {
                continue;
            }
            gone.extend(same);
            if x.href.is_empty() {
                if let Some(slot) = added_unsent.insert((x.event.uid.as_str(), rid), add.len()).and_then(|a| add.get_mut(a)) {
                    *slot = None;
                }
            } else {
                added_href.insert((x.href.as_str(), rid), add.len());
            }
            add.push(Some(x));
        }
        (gone, add.into_iter().flatten().cloned().collect::<Vec<StoredEvent>>())
    };
    if gone.is_empty() && add.is_empty() {
        return;
    }
    let mut i = 0usize;
    p.events.retain(|_| {
        i += 1;
        !gone.contains(&(i - 1))
    });
    p.events.extend(add);
}

/// The server's cards of the address books it read in place of those kept here: one gone from
/// the server goes, one changed here stays, one the server has as it's kept here stays where it
/// is, and the others take the place of any card with their UID. Each found by an index.
fn merge_cards(p: &mut Pim, sync: &quatrelle_model::pim::DavSync) {
    let synced: HashSet<&str> = sync.synced_books.iter().map(String::as_str).collect();
    let kept: HashSet<&str> = sync.kept.iter().map(String::as_str).collect();
    let read: HashSet<&str> = sync.cards.iter().map(|x| x.card.uid.as_str()).collect();
    p.cards.retain(|c| {
        !synced.contains(c.book.as_str()) || c.dirty || c.href.is_empty() || kept.contains(c.href.as_str()) || read.contains(c.card.uid.as_str())
    });
    let (gone, add) = {
        let edited: HashSet<&str> = p.cards.iter().filter(|c| c.dirty).map(|c| c.card.uid.as_str()).collect();
        let mut by_uid: HashMap<&str, Vec<usize>> = HashMap::new();
        for (i, c) in p.cards.iter().enumerate() {
            by_uid.entry(c.card.uid.as_str()).or_default().push(i);
        }
        let mut gone: HashSet<usize> = HashSet::new();
        let mut add: Vec<Option<&StoredCard>> = Vec::new();
        let mut added: HashMap<&str, usize> = HashMap::new();
        for book in &sync.synced_books {
            for x in sync.cards.iter().filter(|x| &x.book == book) {
                let uid = x.card.uid.as_str();
                if edited.contains(uid) {
                    continue;
                }
                if let Some(slot) = added.remove(uid).and_then(|a| add.get_mut(a)) {
                    *slot = None;
                }
                let same: Vec<usize> = by_uid.get(uid).into_iter().flatten().copied().filter(|i| !gone.contains(i)).collect();
                if let [one] = same.as_slice()
                    && p.cards.get(*one) == Some(x)
                {
                    continue;
                }
                gone.extend(same);
                added.insert(uid, add.len());
                add.push(Some(x));
            }
        }
        (gone, add.into_iter().flatten().cloned().collect::<Vec<StoredCard>>())
    };
    if gone.is_empty() && add.is_empty() {
        return;
    }
    let mut i = 0usize;
    p.cards.retain(|_| {
        i += 1;
        !gone.contains(&(i - 1))
    });
    p.cards.extend(add);
}
