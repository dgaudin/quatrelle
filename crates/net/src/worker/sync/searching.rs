// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! An account's worker: searching the server, the folders' own search and the mailbox's index
//! (Exchange's).

use super::*;

impl Worker {
    /// Search folders on the server (issue #4): IMAP's `SEARCH`, JMAP's `Email/query`, Graph's
    /// `$search` and EWS's `QueryString`, the messages found brought into their folders' lists
    /// and sent as they're found (an IMAP folder a range at a time, the others a folder at a
    /// time). Stops between two when a newer search is asked, or none (the search box cleared).
    pub(crate) fn search_server(&mut self, folders: &[String], query: String, search: u64) {
        let id = self.id();
        let (sink, wanted) = (self.sink.clone(), self.search.clone());
        let still = move || wanted.load(Ordering::Relaxed) == search;
        let answer = |keys, error, done| Event::ServerSearched { account: id.clone(), query: query.clone(), keys, error, done };
        if !still() {
            self.sink.emit(answer(Vec::new(), None, true));
            return;
        }
        if self.conn.is_none() {
            self.next_attempt = Instant::now();
            self.connect_and_sync();
        }
        let store = self.store.clone();
        let (imap, filter, words) = (crate::sync::imap_criteria(&query), quatrelle_jmap::search_filter(&query), crate::sync::word_query(&query));
        let asks = match self.conn.as_ref() {
            None => {
                self.sink.emit(answer(Vec::new(), Some("the account isn't connected".into()), true));
                return;
            }
            Some(Conn::Pop) => {
                self.sink.emit(answer(Vec::new(), Some("a POP account keeps all its mail here".into()), true));
                return;
            }
            Some(Conn::Imap(_)) => imap.is_some(),
            Some(Conn::Jmap(_)) => filter.is_some(),
            Some(Conn::Graph(_) | Conn::Ews(_)) => words.is_some(),
        };
        if !asks {
            self.sink.emit(answer(Vec::new(), None, true));
            return;
        }
        let (mut any, mut failed) = (false, None);
        for f in folders.iter().take(200) {
            if !still() {
                break;
            }
            // What a folder finds goes out as it's found.
            let mut sent = |keys: Vec<MessageKey>, new: usize| {
                if new > 0 {
                    sink.emit(Event::FolderChanged { account: id.clone(), folder: f.clone() });
                }
                if !keys.is_empty() {
                    any = true;
                    sink.emit(answer(keys, None, false));
                }
                still()
            };
            let found = match (self.conn.as_mut(), &imap, &filter, &words) {
                (Some(Conn::Imap(c)), Some(q), _, _) => crate::sync::find_on_server(c, &store, &id, f, q, FOUND_MAX, &mut sent).map(|_| ()),
                (Some(Conn::Jmap(c)), _, Some(q), _) => jmap::find_on_server(c, &mut self.jstate, &store, &id, f, q, FOUND_MAX).map(|(keys, new)| {
                    sent(keys, new);
                }),
                (Some(Conn::Graph(c)), _, _, Some(q)) => {
                    crate::graph::find_on_server(c, &mut self.jstate, &store, &id, f, q, FOUND_MAX).map(|(keys, new)| {
                        sent(keys, new);
                    })
                }
                (Some(Conn::Ews(c)), _, _, Some(q)) => {
                    crate::ews::find_on_server(c, &mut self.jstate, &store, &id, f, q, FOUND_MAX).map(|(keys, new)| {
                        sent(keys, new);
                    })
                }
                _ => break,
            };
            if let Err(e) = found {
                log::info!("{id}/{f}: searching the server: {e}");
                failed = Some(e.to_string());
                if matches!(e, NetError::Io(_) | NetError::Closed(_)) {
                    self.conn = None;
                    break;
                }
            }
        }
        if !matches!(self.conn, Some(Conn::Imap(_))) {
            self.save_jstate();
        }
        let error = if any { None } else { failed };
        self.sink.emit(answer(Vec::new(), error, true));
    }

    /// Search with the server's own index (Exchange's): the local one can only find a message
    /// whose text is here, and a search over years of mail needs the rest.
    pub(crate) fn search(&mut self, query: String, folder: Option<String>) {
        let account = self.acc.id.clone();
        if self.conn.is_none()
            && let Err(e) = self.connect()
        {
            self.sink.emit(Event::Found { account, query, items: Vec::new(), error: Some(e.to_string()) });
            return;
        }
        // Which folders to ask: the one on view, or every folder of the account, the biggest
        // first (Gmail's All Mail holds everything; the walk stops when its time is up).
        let paths: Vec<String> = match &folder {
            Some(f) => vec![f.clone()],
            None => {
                // Not the online archive: a search of the whole mailbox doesn't look there, as in
                // Outlook (one of its folders is searched when it's the one on view).
                let mut fs: Vec<&quatrelle_model::Folder> = self.folders.iter().filter(|f| !f.archive).collect();
                fs.sort_by_key(|f| std::cmp::Reverse(f.total));
                fs.into_iter().map(|f| f.path.clone()).collect()
            }
        };
        // The numbering an id gets is kept, so a message found by a search can be read afterwards
        // (and after a restart). The state is taken out of `self` — it can't be borrowed twice for
        // the call — and goes back whatever the call answered: one left empty would key every
        // cached message to nothing.
        let mut state = std::mem::take(&mut self.jstate);
        let store = self.store.clone();
        let r = match self.conn.as_mut() {
            Some(Conn::Ews(c)) => crate::ews::search(c, &mut state, &account, &paths, &query, SEARCH_LIMIT),
            // IMAP's own SEARCH: the words may be in a message older than the window kept here,
            // or in a body never downloaded.
            Some(Conn::Imap(c)) => crate::sync::search_folders(
                c,
                &store,
                &account,
                &paths,
                &query,
                SEARCH_LIMIT,
                std::time::Duration::from_secs(crate::sync::SEARCH_SECONDS),
                crate::sync::SEARCH_FOLDERS,
            ),
            // The other transports answer from this computer alone.
            _ => Ok(Vec::new()),
        };
        let numbers: Vec<u32> = r.as_ref().map(|items| items.iter().map(|m| m.key.uid).collect()).unwrap_or_default();
        let _ = state.save(&self.store, &account);
        self.jstate = state;
        // What only a search found isn't in any folder's cache, so the numbers it may not be
        // kept by must survive the next cleanup: the message is still one to open.
        // Only the API transports number a message themselves; an IMAP key is the server's UID,
        // kept in its folder's cache already.
        if self.acc.is_api() {
            for n in numbers {
                if !self.search_numbers.contains(&n) {
                    self.search_numbers.push(n);
                }
            }
            if self.search_numbers.len() > 4_000 {
                self.search_numbers.drain(..2_000);
            }
        }
        match r {
            Ok(items) => self.sink.emit(Event::Found { account, query, items, error: None }),
            Err(e) => {
                self.sink.emit(Event::Found { account, query, items: Vec::new(), error: Some(e.to_string()) });
                if e.is_fatal() {
                    self.failed(e);
                }
            }
        }
    }
}
