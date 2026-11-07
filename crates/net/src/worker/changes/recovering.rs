// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! An account's worker: the items an Exchange server keeps of what was deleted for good
//! (Recoverable Items), listed, brought back or purged, on the second connection so none waits
//! behind a sync. Read on the server each time; nothing of it is kept.

use super::*;

impl Worker {
    /// A page of Recoverable Items, answered with [`Event::Recoverable`].
    pub(super) fn list_recoverable(&mut self, ask: u64, offset: u32) {
        let r = (|| -> Result<(Vec<quatrelle_model::Recoverable>, bool)> {
            if self.conn.is_none() {
                self.connect()?;
            }
            match self.conn.as_ref() {
                Some(Conn::Ews(c)) => c.recoverable_page(offset as usize, quatrelle_model::RECOVERABLE_PAGE),
                Some(Conn::Graph(c)) => c.recoverable_page(offset as usize, quatrelle_model::RECOVERABLE_PAGE),
                _ => Err(NetError::No("this server does not keep deleted items".into())),
            }
        })();
        let account = self.id();
        match r {
            Ok((items, more)) => {
                log::info!("{account}: {} recoverable items from {offset}, more: {more}", items.len());
                self.sink.emit(Event::Recoverable { account, ask, offset, items, more, error: None });
            }
            Err(e) => {
                let error = Some(self.broken(e));
                self.sink.emit(Event::Recoverable { account, ask, offset, items: Vec::new(), more: false, error });
            }
        }
    }

    /// Bring items back into Deleted Items, or purge them, answered with [`Event::RecoverableDone`].
    pub(super) fn restore_recoverable(&mut self, ask: u64, ids: Vec<String>, purge: bool) {
        let connected = if self.conn.is_none() { self.connect().err() } else { None };
        let (done, failed) = match (connected, self.conn.as_ref()) {
            (Some(e), _) => (Vec::new(), Some(e)),
            (None, Some(Conn::Ews(c))) => c.restore_recoverable(&ids, purge),
            (None, Some(Conn::Graph(c))) => c.restore_recoverable(&ids, purge),
            _ => (Vec::new(), Some(NetError::No("this server does not keep deleted items".into()))),
        };
        let account = self.id();
        let error = failed.map(|e| self.broken(e));
        log::info!("{account}: {} of {} recoverable items {}", done.len(), ids.len(), if purge { "purged" } else { "restored" });
        self.sink.emit(Event::RecoverableDone { account, ask, done, purge, error });
    }

    /// What an error says, dropping the connection when it can't be used again.
    fn broken(&mut self, e: NetError) -> String {
        let said = e.to_string();
        if e.is_fatal() || matches!(e, NetError::Auth(_)) {
            self.conn = None;
        }
        said
    }
}
