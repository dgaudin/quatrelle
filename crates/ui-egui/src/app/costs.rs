//! What a calendars' read costs on the window's thread, said in the log: merged into the
//! calendar, then saved. With what the read cost on the server (said by the network side), it
//! tells where a calendar just ticked spends its seconds (29/10: a quarter of a minute).

use quatrelle_engine::model::Event;

/// A calendars' read about to be merged: whose, how much, and since when.
pub(crate) struct Merging {
    account: String,
    calendars: usize,
    events: usize,
    began: web_time::Instant,
}

impl Merging {
    /// For a calendars' read; `None` for any other event.
    pub(crate) fn of(e: &Event) -> Option<Merging> {
        match e {
            Event::DavSynced { account, sync } => Some(Merging {
                account: account.clone(),
                calendars: sync.synced_calendars.len(),
                events: sync.events.len(),
                began: web_time::Instant::now(),
            }),
            _ => None,
        }
    }

    /// Merged: said in the log.
    pub(crate) fn done(self) {
        let Merging { account, calendars, events, began } = self;
        log::info!("{account}: {events} appointments of {calendars} calendars merged and saved in {} ms", began.elapsed().as_millis());
    }
}
