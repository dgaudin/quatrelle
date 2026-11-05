// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! An account's worker: opening a message, from here or from the server, and reading older mail
//! on the server.

use super::*;

impl Worker {
    pub(super) fn fetch(&mut self, key: MessageKey) {
        if let Some(raw) = self.store.read_body(&key) {
            self.sink.emit(Event::Body { key, raw });
            return;
        }
        let store = self.store.clone();
        let numbered = self.jstate.len();
        let r = (|| -> Result<(Vec<u8>, Option<quatrelle_model::Summary>)> {
            if self.conn.is_none() {
                self.connect()?;
            }
            match self.conn.as_mut() {
                Some(Conn::Jmap(c)) => jmap::fetch_body(c, &self.jstate, &store, &key),
                Some(Conn::Graph(c)) => crate::graph::fetch_body(c, &self.jstate, &store, &key),
                Some(Conn::Ews(c)) => crate::ews::fetch_body(c, &mut self.jstate, &store, &key),
                // POP mail is downloaded whole: what isn't here isn't anywhere.
                Some(Conn::Pop) => {
                    store.read_body(&key).map(|raw| (raw, None)).ok_or_else(|| NetError::No("this message isn't on this computer".into()))
                }
                _ => fetch_body(self.client()?, &store, &key),
            }
        })();
        if self.jstate.len() != numbered {
            // A message found again by its Message-ID: the numbering is worth keeping.
            self.save_jstate();
        }
        match r {
            // Its summary filled from it, said as it is now: the window takes it into the folder
            // it holds (it read all of the folder again for it, 196 ms on 50,000 messages).
            Ok((raw, filled)) => {
                self.sink.emit(Event::Body { key, raw });
                if let Some(summary) = filled {
                    self.sink.emit(Event::Filled { summary: Box::new(summary) });
                }
            }
            Err(e) => {
                self.sink.emit(Event::BodyFailed { key, error: e.to_string() });
                if e.is_fatal() || matches!(e, NetError::Auth(_)) {
                    self.failed(e);
                }
            }
        }
    }

    /// A page of a folder's mail older than what's kept, read on the server and held in memory
    /// by the session (Exchange, as Cached Exchange Mode's "more on the server"): numbered, and
    /// kept numbered as a search's finds are, but not kept.
    pub(super) fn older(&mut self, folder: &str, offset: u32) {
        let account = self.id();
        let before = self.opts.since(now()).unwrap_or(i64::MAX);
        let numbered = self.jstate.len();
        let r = (|| -> Result<(Vec<quatrelle_model::Summary>, bool)> {
            if self.conn.is_none() {
                self.connect()?;
            }
            match self.conn.as_ref() {
                Some(Conn::Ews(c)) => crate::ews::older(c, &mut self.jstate, &account, folder, before, offset as usize, OLDER_PAGE),
                _ => Err(NetError::No("this account reads older mail by keeping more of it".into())),
            }
        })();
        if self.jstate.len() != numbered {
            self.save_jstate();
        }
        match r {
            Ok((items, more)) => {
                for n in items.iter().map(|m| m.key.uid) {
                    if !self.search_numbers.contains(&n) {
                        self.search_numbers.push(n);
                    }
                }
                if self.search_numbers.len() > 4_000 {
                    self.search_numbers.drain(..2_000);
                }
                self.sink.emit(Event::Older { account, folder: folder.to_string(), items, more, error: None });
            }
            Err(e) => {
                self.sink.emit(Event::Older { account, folder: folder.to_string(), items: Vec::new(), more: false, error: Some(e.to_string()) })
            }
        }
    }

    /// A message's source read on the server for showing, not kept (mail older than what's
    /// kept, read online).
    pub(super) fn read_online_body(&mut self, key: MessageKey) {
        let store = self.store.clone();
        let r = (|| -> Result<Vec<u8>> {
            if self.conn.is_none() {
                self.connect()?;
            }
            match self.conn.as_ref() {
                Some(Conn::Ews(c)) => crate::ews::read_online(c, &mut self.jstate, &store, &key),
                _ => Err(NetError::No("this account reads its mail by keeping it".into())),
            }
        })();
        match r {
            Ok(raw) => self.sink.emit(Event::Body { key, raw }),
            Err(e) => self.sink.emit(Event::BodyFailed { key, error: e.to_string() }),
        }
    }
}
