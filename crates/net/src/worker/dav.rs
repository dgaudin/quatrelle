// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! An account's worker: calendars and address books over CalDAV and CardDAV, and the sites they ask about.

use quatrelle_model::pim::{DavSync, Pim, StoredCard, StoredEvent, StoredNote, StoredTask};

use super::*;

/// The most appointments (and as many contacts, notes, tasks) an account's syncs made that are
/// remembered (the oldest go first).
const STORED_MAX: usize = 500;

/// What an account's syncs put on the server, each with its item there, until the calendars,
/// contacts, notes and tasks as saved show it. The window takes a sync's result in a moment
/// after it's sent, and the next sync may start before, from what was saved: an appointment
/// made by the sync before, then moved, was still new there, and was made a second time
/// (02/11: Outlook showed it on Monday and on Tuesday). So with a new contact, note or task, and
/// with a changed contact or note, which Exchange takes as a new item each time (the old one
/// deleted): the next sync deleted nothing and made it again. Its item is put back on it
/// instead, so the change goes to the item made.
#[derive(Default)]
pub(super) struct Stored {
    events: VecDeque<StoredEvent>,
    cards: VecDeque<StoredCard>,
    notes: VecDeque<StoredNote>,
    tasks: VecDeque<StoredTask>,
}

/// Something a sync puts on the server and [`Stored`] remembers.
trait Pushed: Clone {
    /// The same appointment, contact, note or task.
    fn same(&self, other: &Self) -> bool;
    fn href(&self) -> &str;
    /// As saved, still to send through another item than the one `made`: none yet, or (contacts
    /// and notes, made again at each change) the one the sync before replaced.
    fn behind(&self, made: &Self) -> bool {
        self.href().is_empty() || (self.dirty() && self.made_again() && self.href() != made.href())
    }
    fn dirty(&self) -> bool;
    /// Sent as a new item at each change.
    fn made_again(&self) -> bool {
        false
    }
    /// Given the item made, and where it went.
    fn take_item(&mut self, made: &Self);
    /// How the log names it (no one's name or text).
    fn id(&self) -> &str;
}

impl Pushed for StoredEvent {
    fn same(&self, other: &Self) -> bool {
        self.event.uid == other.event.uid && self.event.recurrence_id == other.event.recurrence_id
    }
    fn href(&self) -> &str {
        &self.href
    }
    fn dirty(&self) -> bool {
        self.dirty
    }
    fn take_item(&mut self, made: &Self) {
        self.href.clone_from(&made.href);
        self.etag.clone_from(&made.etag);
        self.calendar.clone_from(&made.calendar);
    }
    fn id(&self) -> &str {
        &self.event.uid
    }
}

impl Pushed for StoredCard {
    fn same(&self, other: &Self) -> bool {
        self.card.uid == other.card.uid
    }
    fn href(&self) -> &str {
        &self.href
    }
    fn dirty(&self) -> bool {
        self.dirty
    }
    fn made_again(&self) -> bool {
        self.href.starts_with("contact:")
    }
    fn take_item(&mut self, made: &Self) {
        self.href.clone_from(&made.href);
        self.etag.clone_from(&made.etag);
        self.book.clone_from(&made.book);
    }
    fn id(&self) -> &str {
        &self.card.uid
    }
}

impl Pushed for StoredNote {
    fn same(&self, other: &Self) -> bool {
        self.note.uid == other.note.uid
    }
    fn href(&self) -> &str {
        &self.href
    }
    fn dirty(&self) -> bool {
        self.dirty
    }
    fn made_again(&self) -> bool {
        self.href.starts_with("note:")
    }
    fn take_item(&mut self, made: &Self) {
        self.href.clone_from(&made.href);
        self.etag.clone_from(&made.etag);
    }
    fn id(&self) -> &str {
        &self.note.uid
    }
}

impl Pushed for StoredTask {
    fn same(&self, other: &Self) -> bool {
        self.task.uid == other.task.uid
    }
    fn href(&self) -> &str {
        &self.href
    }
    fn dirty(&self) -> bool {
        self.dirty
    }
    fn take_item(&mut self, made: &Self) {
        self.href.clone_from(&made.href);
        self.etag.clone_from(&made.etag);
    }
    fn id(&self) -> &str {
        &self.task.uid
    }
}

/// What a sync put on the server, each in place of what an earlier sync put there.
fn record<T: Pushed>(list: &mut VecDeque<T>, pushed: &[T]) {
    for x in pushed.iter().filter(|x| !x.href().is_empty()) {
        list.retain(|e| !e.same(x));
        list.push_back(x.clone());
    }
    while list.len() > STORED_MAX {
        list.pop_front();
    }
}

/// What was saved, each thing made since and still behind there given its item; one saved with
/// its item is no longer remembered.
fn carry<T: Pushed>(list: &mut VecDeque<T>, saved: &mut [T]) {
    list.retain(|x| {
        if saved.iter().any(|e| e.same(x) && e.href() == x.href()) {
            return false;
        }
        for e in saved.iter_mut().filter(|e| e.same(x) && e.behind(x)) {
            log::info!("{}: made on the server by the sync before, not taken in yet: changed there, not made again", e.id());
            e.take_item(x);
        }
        true
    });
}

impl Stored {
    /// What a sync put on the server, each in place of what an earlier sync put there.
    fn record(&mut self, sync: &DavSync) {
        record(&mut self.events, &sync.pushed_events);
        record(&mut self.cards, &sync.pushed_cards);
        record(&mut self.notes, &sync.pushed_notes);
        record(&mut self.tasks, &sync.pushed_tasks);
    }

    /// What was saved, with what the syncs made since and the window hasn't taken in yet.
    fn carry(&mut self, pim: &mut Pim) {
        carry(&mut self.events, &mut pim.events);
        carry(&mut self.cards, &mut pim.cards);
        carry(&mut self.notes, &mut pim.notes);
        carry(&mut self.tasks, &mut pim.tasks);
    }
}

impl Sink {
    /// A sync's result on its way to the window: what it put on the server is remembered.
    pub(super) fn note_stored(&self, e: &Event) {
        if let Event::DavSynced { account, sync } = e
            && !(sync.pushed_events.is_empty() && sync.pushed_cards.is_empty() && sync.pushed_notes.is_empty() && sync.pushed_tasks.is_empty())
            && let Ok(mut stored) = self.stored.lock()
        {
            stored.entry(account.clone()).or_default().record(sync);
        }
    }

    /// An account's calendars as saved, with what its syncs put on the server since.
    fn carry_stored(&self, account: &str, pim: &mut Pim) {
        if let Ok(mut stored) = self.stored.lock()
            && let Some(s) = stored.get_mut(account)
        {
            s.carry(pim);
        }
    }
}

impl Worker {
    /// The account's calendars and address books as the window last kept them, reading what it
    /// last asked for ([`quatrelle_model::pim::PIM_READING_DOC`]), with the appointments made on
    /// the server since that it hasn't taken in yet ([`Stored`]).
    pub(super) fn load_pim(&self) -> quatrelle_model::pim::Pim {
        use quatrelle_model::pim::{PIM_DOC, PIM_READING_DOC, PimReading};
        let mut pim: Pim = self.store.load_account_doc(&self.acc.id, PIM_DOC).unwrap_or_default();
        pim.read_as(self.store.load_account_doc::<Option<PimReading>>(&self.acc.id, PIM_READING_DOC).unwrap_or_default());
        self.sink.carry_stored(&self.acc.id, &mut pim);
        pim
    }

    /// Sync calendars and address books with the account's CalDAV / CardDAV server (password
    /// accounts: OAuth tokens are for mail only).
    pub(super) fn sync_dav(&mut self) {
        let _at_work = self.at_work();
        self.pim_synced();
        // Exchange Online: its calendars and contacts through Graph.
        if let Some(Conn::Graph(c)) = &self.conn {
            let pim = self.load_pim();
            match crate::graph_pim::sync(c, &pim, now()) {
                Ok(sync) => self.sink.emit(Event::DavSynced { account: self.id(), sync: Box::new(sync) }),
                Err(e) => log::info!("{}: calendars and contacts: {e}", self.acc.id),
            }
            return;
        }
        // On-premises Exchange: its calendar and contacts through EWS.
        if let Some(Conn::Ews(c)) = &self.conn {
            let pim = self.load_pim();
            match crate::ews_pim::sync(c, &pim, now()) {
                Ok(sync) => self.sink.emit(Event::DavSynced { account: self.id(), sync: Box::new(sync) }),
                Err(e) => log::info!("{}: calendars and contacts: {e}", self.acc.id),
            }
            // The categories Outlook offers, with its colours.
            match crate::ews_pim::master_categories(c) {
                Ok(categories) if !categories.is_empty() => self.sink.emit(Event::Categories { account: self.id(), categories }),
                Ok(_) => {}
                Err(e) => log::info!("{}: the category list: {e}", self.acc.id),
            }
            return;
        }
        // A site to allow first: nothing until the next start.
        if !looks_for_calendars(&self.acc) || self.dav_waits {
            return;
        }
        // The homes saved in the account's PIM doc are trusted: the password has gone there.
        let pim = self.load_pim();
        let user = if self.acc.username.is_empty() { self.acc.email.clone() } else { self.acc.username.clone() };
        let password = self.auth.lock().unwrap_or_else(std::sync::PoisonError::into_inner).secret.clone();
        let dav = crate::dav::Dav::new(&user, &password, crate::dav::trust_of(&self.acc, &pim));
        let host = self.acc.pop.as_ref().map_or(self.acc.imap.host.as_str(), |p| p.host.as_str());
        let done = crate::dav::sync(&dav, &self.acc.email, host, &pim, now(), system_zone());
        self.dav_done(done);
    }

    /// What the server told of in the account's own calendar or contacts (on-premises Exchange):
    /// those two read by their changes alone, a few requests on the connection new mail comes on,
    /// not the whole sync (a real calendar's few hundred appointments, 3 to 6 s, at each change, 06/11). The
    /// tasks told of meanwhile wait for their own read.
    pub(super) fn sync_pim_changes(&mut self) {
        let _at_work = self.at_work();
        self.pim_changes_synced();
        let Some(Conn::Ews(c)) = &self.conn else { return self.sync_dav() };
        let pim = self.load_pim();
        match crate::ews_pim::sync_changes(c, &pim, now()) {
            Ok(sync) => self.sink.emit(Event::DavSynced { account: self.id(), sync: Box::new(sync) }),
            Err(e) => log::info!("{}: the calendar's and contacts' changes: {e}", self.acc.id),
        }
    }

    /// The calendars' and contacts' pass after a full sync, unless the worker was told to stop
    /// meanwhile: it would run under the settings it started with, a site just removed among them.
    pub(super) fn pim_pass(&mut self, rx: &Receiver<Job>) {
        self.drain(rx);
        if !self.closed {
            self.sync_dav();
        }
    }

    /// A CalDAV / CardDAV sync's result: merged, and a site it stopped at asked about (once).
    pub(super) fn dav_done(&mut self, done: Result<quatrelle_model::pim::DavSync>) {
        match done {
            Ok(sync) => {
                let untrusted = sync.untrusted.clone();
                self.sink.emit(Event::DavSynced { account: self.id(), sync: Box::new(sync) });
                if let Some(site) = untrusted {
                    log::info!("{}: a calendar or address book waits for {site} to be allowed", self.acc.id);
                    self.ask_trust(site);
                }
            }
            Err(NetError::Untrusted { site }) => {
                log::info!("{}: calendars and contacts wait for {site} to be allowed", self.acc.id);
                self.dav_waits = true;
                self.ask_trust(site);
            }
            Err(e) => {
                log::info!("{}: calendars and contacts: {e}", self.acc.id);
                if matches!(e, NetError::Auth(_)) {
                    self.notice(format!("calendars and contacts: {e}"));
                }
            }
        }
    }

    fn ask_trust(&mut self, site: String) {
        if self.dav_asked.insert(site.clone()) {
            self.sink.emit(Event::TrustAsked { account: self.id(), site });
        }
    }
}

/// Whether an account's calendars are looked for over CalDAV and CardDAV: OAuth tokens are for
/// mail only, Exchange has its own way, and Google's take no password.
fn looks_for_calendars(acc: &AccountConfig) -> bool {
    acc.oauth.is_none() && acc.ews.is_none() && !quatrelle_model::google_mailbox(&acc.imap.host, &acc.smtp.host)
}

/// The system's time zone, for calendar times written without one (`UTC` when it can't say).
fn system_zone() -> quatrelle_model::zone::Zone {
    iana_time_zone::get_timezone().ok().and_then(|z| quatrelle_model::zone::Zone::named(&z)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendars_are_looked_for_only_where_a_password_can_find_them() {
        let at = |host: &str| {
            let mut a = AccountConfig::default();
            a.imap.host = host.into();
            a
        };
        assert!(looks_for_calendars(&at("imap.fastmail.com")));
        assert!(!looks_for_calendars(&at("imap.gmail.com")));
        assert!(!looks_for_calendars(&AccountConfig { oauth: Some("microsoft".into()), ..at("imap.fastmail.com") }));
        assert!(!looks_for_calendars(&AccountConfig { ews: Some("https://mail.example/EWS/Exchange.asmx".into()), ..at("mail.example") }));
    }
}
