// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! An account's worker: the folder list read from the server and kept, and what of each folder
//! is kept here (the public folders followed, what is read on the server, what is let go of).

use quatrelle_model::utf7;

use super::*;
use crate::exchange::missed::{Listed, Missed};

impl Worker {
    /// The folder list, then every folder (Inbox first), staying responsive to opened messages.
    /// Refresh the folder list from the server; `false` when that failed.
    pub(in crate::worker) fn relist(&mut self) -> bool {
        // Nothing is listed while the numbering couldn't be read (the account is set aside).
        if self.numbering_unread.is_some() {
            return false;
        }
        let _at_work = self.at_work();
        let id = self.id();
        // What the numbering knew before: the folders of a zone the listing misses keep their ids.
        let (ids_before, owners_before) = (self.jstate.mailboxes.clone(), self.jstate.owners.clone());
        if let Some(Conn::Ews(c)) = &self.conn {
            // A mailbox added by hand keeps the name it's listed under while the directory is silent.
            c.set_known_shared(
                self.folders
                    .iter()
                    .filter(|f| f.depth() == 0)
                    .filter_map(|f| Some((f.shared_mailbox()?.to_string(), utf7::decode(&f.path))))
                    .collect(),
            );
        }
        let whole = |folders| Listed { folders, missed: Missed::default() };
        let listed = match self.conn.as_mut() {
            Some(Conn::Imap(c)) => c.list(&id).map(whole),
            Some(Conn::Jmap(c)) => jmap::list_folders(c, &mut self.jstate, &id).map(whole),
            Some(Conn::Graph(c)) => crate::graph::list_folders(c, &mut self.jstate, &id, &self.acc.shared_mailboxes),
            Some(Conn::Ews(c)) => crate::ews::list_folders(c, &mut self.jstate, &id),
            Some(Conn::Pop) => crate::pop::folders(&self.store, &id).map(whole),
            None => Err(NetError::Closed("not connected".into())),
        };
        // A zone that didn't list keeps its folders as they were, with their ids: the numbering
        // saved next must have them (taken for gone, the zone's numbers went, and its mail was
        // downloaded again once it listed).
        let listed = listed.map(|Listed { folders: mut list, missed }| {
            let kept = zones::keep_missed(
                &self.folders,
                &ids_before,
                &owners_before,
                &mut list,
                &mut self.jstate.mailboxes,
                &mut self.jstate.owners,
                &missed,
            );
            let unread = if self.folders_unread {
                zones::keep_unread_ids(&ids_before, &owners_before, &mut self.jstate.mailboxes, &mut self.jstate.owners, &missed)
            } else {
                HashSet::new()
            };
            zones::log_missed(&id, kept.len(), &missed);
            (list, missed, kept, unread)
        });
        let jmap = matches!(self.conn, Some(Conn::Jmap(_) | Conn::Graph(_) | Conn::Ews(_)));
        if jmap {
            self.save_jstate();
        }
        match listed {
            Ok((mut list, missed, kept, unread)) => {
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
                // cache for good otherwise). Changes or a list that can't be read (the disk
                // failing) can't say which those are: every one keeps its cache till the next
                // listing, as does one whose cache couldn't be cleared (taken for nothing
                // waiting, they lost it all).
                // One in a zone the listing missed can't be told gone either: looked at again once
                // it lists.
                let here = self.kept_while_waiting(&id);
                let mut gone: Vec<String> = self.folders.iter().map(|f| f.path.clone()).chain(std::mem::take(&mut self.gone_meanwhile)).collect();
                gone.retain(|p| !list.iter().any(|f| f.path == *p));
                gone.sort_unstable();
                gone.dedup();
                for path in gone {
                    if zones::in_missed_zone(&path, &kept, &self.jstate.mailboxes, &missed)
                        || here.as_ref().is_none_or(|here| here.iter().any(|f| f.path == path))
                    {
                        self.gone_meanwhile.push(path);
                    } else if let Err(e) = self.store.clear_folder(&id, &path) {
                        log::warn!("{id}: a folder gone from the server keeps its cache for now: {e}");
                        self.gone_meanwhile.push(path);
                    }
                }
                // Nothing of a zone kept is read, written nor let go of till it lists again.
                self.ahead.retain(|p| !kept.contains(p));
                self.unlisted_kept = kept;
                // The list before is known again once a listing misses no zone with something
                // here: one with nothing here (a server without public folders misses them at
                // every listing) has nothing to lose.
                if self.folders_unread && self.unlisted_kept.is_empty() && unread.is_empty() {
                    log::info!("{id}: the folder list is whole again: kept, and numbers pruned, from now on");
                    self.folders_unread = false;
                } else if self.folders_unread {
                    log::info!("{id}: the folder list before couldn't be read: {} folders may be in a zone that didn't list", unread.len());
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

    /// The folders the list on disk has while folder changes wait (none when none wait); `None`,
    /// logged, when either can't be read.
    fn kept_while_waiting(&self, id: &str) -> Option<Vec<Folder>> {
        let read = || -> std::result::Result<Vec<Folder>, quatrelle_store::StoreError> {
            if self.store.load_ops(id)?.iter().any(Op::is_folder_op) { self.store.load_folders(id) } else { Ok(Vec::new()) }
        };
        read().inspect_err(|e| log::warn!("{id}: the folders gone from the server keep their cache for now: {e}")).ok()
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
        let mut paths = sync_order(&self.folders, |f: &Folder| !self.read_online(&f.path) && !self.kept_unlisted(&f.path));
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
                self.ahead = sync_order(&self.folders, |f: &Folder| !self.read_online(&f.path) && !self.kept_unlisted(&f.path)).into_iter().collect();
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
    /// by a search). Not when the changes or a folder can't be read (the disk failing): their
    /// numbers would be lost, and the changes never sent. The next full sync tries again. Nor
    /// while the folder list before couldn't be read (`folders_unread`): a zone the listings miss
    /// may have folders here the worker doesn't know of.
    pub(in crate::worker) fn prune_jmap_numbers(&mut self) {
        if self.numbering_unread.is_some() {
            return;
        }
        if !matches!(self.conn, Some(Conn::Jmap(_) | Conn::Graph(_) | Conn::Ews(_))) {
            return;
        }
        if self.folders_unread {
            log::info!("{}: the server's message numbers are all kept till the folder list is whole", self.acc.id);
            return;
        }
        let read = || -> std::result::Result<std::collections::HashSet<u32>, quatrelle_store::StoreError> {
            let ops = self.store.load_ops(&self.acc.id)?;
            let mut keep: std::collections::HashSet<u32> = ops.iter().flat_map(|o| o.keys().iter().map(|k| k.uid).collect::<Vec<_>>()).collect();
            for f in &self.folders {
                keep.extend(self.store.load_folder(&self.acc.id, &f.path)?.messages.iter().map(|m| m.key.uid));
            }
            Ok(keep)
        };
        match read() {
            Ok(mut keep) => {
                keep.extend(self.search_numbers.iter().copied());
                self.jstate.prune(&keep);
                self.save_jstate();
            }
            Err(e) => log::warn!("{}: the server's message numbers are all kept for now: {e}", self.acc.id),
        }
    }

    /// The worker's folder list kept, then told; told only once kept: the list told and not
    /// kept (the disk failing) was the window's till the next start, which found the one before.
    /// While the list before couldn't be read (`folders_unread`), the worker's may lack a zone a
    /// listing missed: told for the folder pane, not kept over the one on disk (the archive's
    /// folders held in memory all the same).
    pub(in crate::worker) fn publish_folders(&self) {
        if self.folders_unread {
            let archive: Vec<&str> = self.folders.iter().filter(|f| f.archive).map(|f| f.path.as_str()).collect();
            self.store.hold_folders(&self.acc.id, &archive);
            self.sink.emit(Event::Folders { account: self.id(), folders: self.folders.clone() });
            return;
        }
        match self.store.save_folders(&self.acc.id, &self.folders) {
            Ok(()) => self.sink.emit(Event::Folders { account: self.id(), folders: self.folders.clone() }),
            Err(e) => log::warn!("{}: the folder list wasn't kept: {e}", self.acc.id),
        }
    }

    /// The public folders the session asks to follow, as it last wrote them. A list that can't be
    /// read or is damaged leaves the one before, and no public folder is let go of till it's read
    /// (`followed_unread`): taken for none followed, every one kept here lost its cache, and its
    /// mail was downloaded again once the list was read.
    pub(in crate::worker) fn read_followed(&mut self) {
        match Worker::followed_doc(&self.store, &self.acc.id) {
            Some(followed) => {
                self.followed = followed;
                self.followed_unread = false;
            }
            None => self.followed_unread = true,
        }
    }

    /// The public folders followed as the session wrote them ([`FOLLOWED_PUBLIC_DOC`]); `None`,
    /// logged, when the list can't be read or is damaged (it's left as it is).
    pub(in crate::worker) fn followed_doc(store: &Store, account: &str) -> Option<BTreeSet<String>> {
        store
            .load_account_doc_strict(account, FOLLOWED_PUBLIC_DOC)
            .inspect_err(|e| log::warn!("{account}: the public folders followed couldn't be read, none is let go of: {e}"))
            .ok()
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
            // While the list of public folders followed can't be read, this one may be one: what
            // it keeps isn't cut to an opened folder's newest few, with their sources (downloaded
            // again once the list is read), and a few more are listed, no more (the app keeps every
            // message: a window of a followed folder listed up to 100,000 of one not followed at
            // each opening). Nor is it read as followed (nothing downloaded ahead, no state
            // followed). What it keeps that can't be read counts none: no more than a few are
            // listed, and the sync, reading it next, stops on it with nothing let go of.
            let window = if self.followed_unread && is_public_folder(path) {
                let held = self
                    .store
                    .load_folder(&self.acc.id, path)
                    .map(|f| f.messages.len())
                    .inspect_err(|e| log::warn!("{}: a public folder's list couldn't be read to list it: {e}", self.acc.id))
                    .unwrap_or(0);
                more.max(ONLINE_WINDOW).max(held.saturating_add(ONLINE_WINDOW)).min(self.opts.window.max(ONLINE_WINDOW))
            } else {
                more.max(ONLINE_WINDOW)
            };
            return SyncOptions { window, prefetch: 0, list_only: true, full: false, since_days: 0, ..self.opts };
        }
        SyncOptions { window: more.max(self.opts.window), ..self.opts }
    }

    /// What earlier passes kept of folders now read on the server (a public folder held up to a
    /// hundred thousand messages, most of a big mailbox's cache) is let go: they keep what they
    /// show when opened. Their message numbers go at the end of the pass. No public folder is
    /// while the list of those followed can't be read (`followed_unread`): it may be one. Nor a
    /// folder kept as it was while its zone doesn't list ([`Worker::kept_unlisted`]).
    pub(in crate::worker) fn let_go_of_online(&mut self) {
        if self.numbering_unread.is_some() {
            return;
        }
        let unread = self.followed_unread;
        let paths: Vec<String> = self
            .folders
            .iter()
            .filter(|f| self.read_online(&f.path) && !(unread && is_public_folder(&f.path)) && !self.kept_unlisted(&f.path))
            .map(|f| f.path.clone())
            .collect();
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
