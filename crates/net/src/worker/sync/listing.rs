// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! An account's worker: the folder list read from the server and kept, and what of each folder
//! is kept here (the public folders followed, what is read on the server, what is let go of).

use super::*;

impl Worker {
    /// The folder list, then every folder (Inbox first), staying responsive to opened messages.
    /// Refresh the folder list from the server; `false` when that failed.
    pub(in crate::worker) fn relist(&mut self) -> bool {
        let _at_work = self.at_work();
        let id = self.id();
        let listed = match self.conn.as_mut() {
            Some(Conn::Imap(c)) => c.list(&id),
            Some(Conn::Jmap(c)) => jmap::list_folders(c, &mut self.jstate, &id),
            Some(Conn::Graph(c)) => crate::graph::list_folders(c, &mut self.jstate, &id, &self.acc.shared_mailboxes),
            Some(Conn::Ews(c)) => crate::ews::list_folders(c, &mut self.jstate, &id),
            Some(Conn::Pop) => crate::pop::folders(&self.store, &id),
            None => Err(NetError::Closed("not connected".into())),
        };
        let jmap = matches!(self.conn, Some(Conn::Jmap(_) | Conn::Graph(_) | Conn::Ews(_)));
        if jmap {
            self.save_jstate();
        }
        match listed {
            Ok(mut list) => {
                // JMAP lists the counts; IMAP's come from each folder's sync.
                for f in list.iter_mut().filter(|_| !jmap) {
                    if let Some(old) = self.folders.iter().find(|o| o.path == f.path) {
                        f.unread = old.unread;
                        f.total = old.total;
                    }
                }
                sort_folders(&mut list);
                // Folders gone from the server lose their cache; not one the list on disk still
                // has while changes to the folders are still to be sent, which may make it (a
                // folder renamed twice offline, listed after the first rename, lost its cache,
                // and its mail was downloaded again). That one's looked at again at the next
                // listing, once they're sent (one deleted on the server meanwhile would keep its
                // cache for good otherwise).
                let waiting = self.store.load_ops(&id).unwrap_or_default().iter().any(Op::is_folder_op);
                let here = if waiting { self.store.load_folders(&id).unwrap_or_default() } else { Vec::new() };
                let mut gone: Vec<String> = self.folders.iter().map(|f| f.path.clone()).chain(std::mem::take(&mut self.gone_meanwhile)).collect();
                gone.retain(|p| !list.iter().any(|f| f.path == *p));
                gone.sort_unstable();
                gone.dedup();
                for path in gone {
                    if here.iter().any(|f| f.path == path) {
                        self.gone_meanwhile.push(path);
                    } else {
                        let _ = self.store.clear_folder(&id, &path);
                    }
                }
                self.folders = list;
                self.publish_folders();
                // The archive and retention policy tags the server offers (asked once a session).
                if let Some(Conn::Ews(c)) = &self.conn
                    && let Ok(tags) = c.archive_tags()
                {
                    self.sink.emit(Event::ArchiveTags { account: self.id(), tags });
                }
                true
            }
            Err(e) => {
                self.failed(e);
                false
            }
        }
    }

    pub(in crate::worker) fn sync_all(&mut self, rx: &Receiver<Job>) {
        let began = Instant::now();
        if !self.relist() {
            return;
        }
        if began.elapsed() > crate::stream::SLOW_CONNECT {
            log::info!("{}: the folders listed in {:.1} s", self.acc.id, began.elapsed().as_secs_f32());
        }
        // A change the server refused for now gets its try at every full sync at the latest.
        if self.waiting.is_some() {
            self.replay();
        }
        self.read_followed();
        self.let_go_of_online();
        let mut paths = sync_order(&self.folders, |f: &Folder| !self.read_online(&f.path));
        let followed = paths.clone();
        if self.opts.parallel > 0 && paths.len() > 2 {
            paths = self.sync_parallel(paths, rx);
            if self.closed {
                return;
            }
        }
        for path in paths {
            self.drain(rx);
            // What someone is waiting on jumps the queue: an opened message, an edit just made.
            let (urgent, rest): (VecDeque<Job>, VecDeque<Job>) = std::mem::take(&mut self.queue).into_iter().partition(between_folders);
            self.queue = rest;
            for j in urgent {
                self.handle(j);
            }
            if self.closed || self.conn.is_none() {
                return;
            }
            // Told of changes, new mail is read between two folders rather than once the pass
            // is done (seconds over a hundred folders, every quarter of an hour).
            if self.listen.on() {
                self.hear(Duration::ZERO);
                if self.conn.is_none() {
                    return;
                }
            }
            // A folder that went from the server meanwhile (told while listening) is passed over.
            if !self.folders.iter().any(|f| f.path == path) {
                continue;
            }
            self.sync_one(&path);
        }
        // "Online" is said once the mail is current: the calendars and contacts can take far
        // longer, and shouldn't hold the status bar on the last folder's name.
        // Every folder followed asked once for its follow-ups marked complete, and its messages
        // replied to or forwarded, in Outlook before they were read here: each pass is done (one
        // cut short goes on at the next sync).
        if matches!(self.conn, Some(Conn::Ews(_))) {
            let (ticks, replies) = (self.jstate.finish_ticks(&followed), self.jstate.finish_replies(&followed));
            if ticks || replies {
                log::info!("{}: in all {} folders, read: completed follow-ups {ticks}, replies {replies}", self.acc.id, followed.len());
                self.save_jstate();
            }
        }
        if self.conn.is_some() {
            self.report_quota();
            self.prune_jmap_numbers();
            self.last_full = Some(Instant::now());
            self.next_poll = Instant::now() + self.poll_every();
            log::info!("{}: full sync done in {:.0} s", self.acc.id, began.elapsed().as_secs_f32());
            // Anything left from a failed attempt gets another try every full sync.
            self.flush_outbox();
            // Then the messages kept are downloaded whole, folder after folder, while nothing
            // else is asked.
            if self.opts.full {
                self.ahead = sync_order(&self.folders, |f: &Folder| !self.read_online(&f.path)).into_iter().collect();
                self.ahead_left = None;
                // Worked out again: what the server wouldn't give gets another try.
                self.ahead_todo = crate::ews::AheadTodo::default();
            }
            log::info!("{}: synced {} folders in {:.1} s", self.acc.id, self.folders.len(), began.elapsed().as_secs_f32());
            self.status(SyncStatus::Online { at: now() });
        }
        if self.conn.is_some() {
            let began = Instant::now();
            self.pim_pass(rx);
            log::info!("{}: PIM pass took {:.1} s", self.acc.id, began.elapsed().as_secs_f32());
        }
    }

    /// Forget the numbers of JMAP messages no longer cached (nor waiting in a change, nor found
    /// by a search).
    pub(in crate::worker) fn prune_jmap_numbers(&mut self) {
        if !matches!(self.conn, Some(Conn::Jmap(_) | Conn::Graph(_) | Conn::Ews(_))) {
            return;
        }
        let ops = self.store.load_ops(&self.acc.id).unwrap_or_default();
        let mut keep: std::collections::HashSet<u32> = ops.iter().flat_map(|o| o.keys().iter().map(|k| k.uid).collect::<Vec<_>>()).collect();
        for f in &self.folders {
            if let Ok(st) = self.store.load_folder(&self.acc.id, &f.path) {
                keep.extend(st.messages.iter().map(|m| m.key.uid));
            }
        }
        keep.extend(self.search_numbers.iter().copied());
        self.jstate.prune(&keep);
        self.save_jstate();
    }

    pub(in crate::worker) fn publish_folders(&self) {
        let _ = self.store.save_folders(&self.acc.id, &self.folders);
        self.sink.emit(Event::Folders { account: self.id(), folders: self.folders.clone() });
    }

    /// The public folders the session asks to follow, as it last wrote them.
    pub(in crate::worker) fn read_followed(&mut self) {
        self.followed = self.store.load_account_doc(&self.acc.id, quatrelle_model::FOLLOWED_PUBLIC_DOC).unwrap_or_default();
    }

    /// Whether a folder is read on the server when it's opened rather than kept here, as
    /// Outlook's Cached Exchange Mode does: a public folder not followed (a Favorite, when public
    /// Favorites are kept), and a shared mailbox's folder when shared mailboxes aren't kept.
    pub(in crate::worker) fn read_online(&self, path: &str) -> bool {
        // The online archive, always: nothing of it is synced nor kept, whatever the settings.
        if self.in_archive(path) {
            return true;
        }
        if quatrelle_model::is_public_folder(path) {
            return !self.followed.contains(path);
        }
        self.acc.shared_online && self.folders.iter().any(|f| f.path == path && f.mailbox.is_some())
    }

    /// Whether a folder is one of the account's online archive.
    pub(in crate::worker) fn in_archive(&self, path: &str) -> bool {
        self.folders.iter().any(|f| f.path == path && f.archive)
    }

    /// How a folder is read: how many of its newest, and whether it's only listed. A folder
    /// asked to keep more keeps them (the API servers keep only the newest window); a folder
    /// read on the server is listed when opened, its newest few hundred, nothing downloaded.
    pub(in crate::worker) fn options_for(&self, path: &str) -> SyncOptions {
        let more = self.windows.get(path).copied().unwrap_or(0);
        // The online archive's folders are years of mail: never more than their newest few
        // hundred listed, whatever was asked to be kept (what's older comes a page at a time).
        if self.in_archive(path) {
            return SyncOptions { window: ONLINE_WINDOW, prefetch: 0, list_only: true, full: false, since_days: 0, ..self.opts };
        }
        if self.read_online(path) {
            return SyncOptions { window: more.max(ONLINE_WINDOW), prefetch: 0, list_only: true, full: false, since_days: 0, ..self.opts };
        }
        SyncOptions { window: more.max(self.opts.window), ..self.opts }
    }

    /// What earlier passes kept of folders now read on the server (a public folder held up to a
    /// hundred thousand messages, most of a big mailbox's cache) is let go: they keep what they
    /// show when opened. Their message numbers go at the end of the pass.
    pub(in crate::worker) fn let_go_of_online(&mut self) {
        let paths: Vec<String> = self.folders.iter().filter(|f| self.read_online(&f.path)).map(|f| f.path.clone()).collect();
        for path in paths {
            let window = self.options_for(&path).window;
            let kept = self.store.load_folder(&self.acc.id, &path).map(|f| f.messages.len()).unwrap_or(0);
            if kept <= window {
                continue;
            }
            match self.store.clear_folder(&self.acc.id, &path) {
                Ok(()) => {
                    log::info!("{}: {}: not followed, {kept} messages let go", self.acc.id, quatrelle_model::utf7::decode(&path));
                    self.jstate.marks.remove(&path);
                    self.sink.emit(Event::FolderChanged { account: self.acc.id.clone(), folder: path });
                }
                Err(e) => log::warn!("{}: {}: {e}", self.acc.id, quatrelle_model::utf7::decode(&path)),
            }
        }
    }
}
