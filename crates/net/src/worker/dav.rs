//! An account's worker: calendars and address books over CalDAV and CardDAV, and the sites they ask about.

use quatrelle_model::pim::{DavSync, Pim, StoredEvent};

use super::*;

/// The most appointments an account's syncs made that are remembered (the oldest go first).
const STORED_MAX: usize = 500;

/// The appointments an account's syncs put on the server, each with its item there, until the
/// calendars as saved show it. The window takes a sync's result in a moment after it's sent,
/// and the next sync may start before, from the calendars as they were saved: an appointment
/// made by the sync before, then moved, was still new there, and was made a second time
/// (02/11: Outlook showed it on Monday and on Tuesday). Its item is put back on it instead, so
/// the change goes to the item made.
#[derive(Default)]
pub(super) struct Stored {
    events: VecDeque<StoredEvent>,
}

impl Stored {
    /// What a sync put on the server, each in place of what an earlier sync put there.
    fn record(&mut self, sync: &DavSync) {
        for x in sync.pushed_events.iter().filter(|x| !x.href.is_empty()) {
            self.events.retain(|e| !(e.event.uid == x.event.uid && e.event.recurrence_id == x.event.recurrence_id));
            self.events.push_back(x.clone());
        }
        while self.events.len() > STORED_MAX {
            self.events.pop_front();
        }
    }

    /// The calendars as saved, each appointment made since and still new there given its item;
    /// one the calendars show with its item is no longer remembered.
    fn carry(&mut self, pim: &mut Pim) {
        self.events.retain(|x| {
            let same = |e: &StoredEvent| e.event.uid == x.event.uid && e.event.recurrence_id == x.event.recurrence_id;
            if pim.events.iter().any(|e| same(e) && e.href == x.href) {
                return false;
            }
            for e in pim.events.iter_mut().filter(|e| same(e) && e.href.is_empty()) {
                log::info!("{}: made on the server by the sync before, not taken in yet: changed there, not made again", e.event.uid);
                e.href.clone_from(&x.href);
                e.etag.clone_from(&x.etag);
                e.calendar.clone_from(&x.calendar);
            }
            true
        });
    }
}

impl Sink {
    /// A sync's result on its way to the window: what it put on the server is remembered.
    pub(super) fn note_stored(&self, e: &Event) {
        if let Event::DavSynced { account, sync } = e
            && !sync.pushed_events.is_empty()
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
