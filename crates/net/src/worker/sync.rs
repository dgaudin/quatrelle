// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! An account's worker: syncing folders, and waiting for new mail; searching the server is in
//! `sync/searching.rs`.

use super::*;

mod searching;

impl Worker {
    /// How often the Inbox is looked at, on a server that can't say when mail comes.
    pub(super) fn poll_every(&self) -> Duration {
        if self.listen.on() {
            listening::LISTEN_POLL_EVERY
        } else if self.acc.ews.is_some() {
            EWS_POLL_EVERY
        } else {
            POLL_EVERY
        }
    }

    /// Wait for new mail: IDLE on the Inbox when the server supports it, else poll.
    pub(super) fn wait_for_mail(&mut self, rx: &Receiver<Job>) {
        if self.listening() {
            self.wait_for_changes(rx);
            return;
        }
        let inbox = self.inbox();
        if matches!(self.conn, Some(Conn::Jmap(_))) && !self.no_push && inbox.is_some() && self.wait_for_push(rx) {
            return;
        }
        let idle = matches!(&self.conn, Some(Conn::Imap(c)) if c.has("IDLE"));
        if !idle || inbox.is_none() {
            let next_soon = self.soon.iter().map(|(at, _)| *at).min();
            let poll_in = self.next_poll.saturating_duration_since(Instant::now());
            let mut wait = next_soon.map_or(poll_in, |at| at.saturating_duration_since(Instant::now()).min(poll_in));
            // Listening that stopped starts again at its time, not at the next look.
            if let Some(retry) = self.listen.retry_in().filter(|_| self.main.is_none() && matches!(self.conn, Some(Conn::Ews(_)))) {
                wait = wait.min(retry.max(Duration::from_millis(10)));
            }
            match rx.recv_timeout(wait) {
                Ok(j) => push_job(&mut self.queue, j),
                Err(RecvTimeoutError::Disconnected) => self.closed = true,
                Err(RecvTimeoutError::Timeout) if next_soon.is_some_and(|at| at <= Instant::now()) => self.sync_soon_due(),
                // Listening's time to start again (it looks at the Inboxes itself).
                Err(RecvTimeoutError::Timeout) if Instant::now() < self.next_poll => {}
                Err(RecvTimeoutError::Timeout) => {
                    self.next_poll = Instant::now() + self.poll_every();
                    if let Some(f) = inbox {
                        self.sync_one(&f);
                    }
                }
            }
            return;
        }
        let Some(inbox) = inbox else { return };
        let waiting_since = Instant::now();
        let result = (|| -> Result<bool> {
            let Some(Conn::Imap(c)) = self.conn.as_mut() else { return Err(NetError::Closed("not connected".into())) };
            // Mail that came after the Inbox was synced and before IDLE starts is never pushed
            // during IDLE: the server already said it (or will only say it to a command). Look
            // now: the Inbox grew since the sync's SELECT, or a command's answer said it did.
            if c.selected() != Some(inbox.as_str()) {
                let synced = c.seen(&inbox).cloned();
                let now = c.select(&inbox, false)?;
                if synced.is_some_and(|s| (s.validity, s.uid_next, s.exists) != (now.validity, now.uid_next, now.exists)) {
                    return Ok(true);
                }
            } else if c.take_news() {
                return Ok(true);
            }
            let queue = &mut self.queue;
            let closed = &mut self.closed;
            c.idle(IDLE_FOR, || {
                loop {
                    match rx.try_recv() {
                        Ok(j) => push_job(queue, j),
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            *closed = true;
                            break;
                        }
                    }
                }
                !queue.is_empty() || *closed
            })
        })();
        match result {
            // How soon new mail was told of, and read, as Exchange's is (the log had nothing
            // for an IMAP account: a Gmail message that seemed not to come couldn't be followed).
            Ok(true) => {
                let (began, heard_at) = (Instant::now(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok());
                let newest = self.sync_one_newest(&inbox);
                let late = match (newest, heard_at) {
                    (Some(came), Some(at)) => format!(
                        "; the newest the server had at {}, told of {:.1} s later (by this computer's clock, to the second)",
                        crate::ews_pim::utc(came).get(11..19).unwrap_or(""),
                        listening::told_after(came, 0, at.as_millis())
                    ),
                    _ => String::new(),
                };
                log::info!(
                    "{}: new mail in {}, read in {} ms{late}",
                    self.acc.id,
                    quatrelle_model::utf7::decode(&inbox),
                    began.elapsed().as_millis()
                );
            }
            Ok(false) => {}
            Err(e) => self.failed_waiting(e, waiting_since.elapsed()),
        }
    }

    /// JMAP push: hold the server's event stream open (as IMAP holds IDLE), checking for jobs
    /// between short reads; a `state` event means something changed. `false` when the server
    /// can't push (the caller polls instead).
    pub(super) fn wait_for_push(&mut self, rx: &Receiver<Job>) -> bool {
        let Some(Conn::Jmap(c)) = &self.conn else { return false };
        let mut es = match c.events() {
            Ok(Some(es)) => es,
            Ok(None) => {
                self.no_push = true;
                return false;
            }
            Err(e @ NetError::Auth(_)) => {
                self.failed(e);
                return true;
            }
            Err(e) => {
                // Push not working (a proxy, an old server): poll from now on.
                log::info!("{}: no JMAP push: {e}", self.acc.id);
                self.no_push = true;
                return false;
            }
        };
        // Mail that came between the last sync and this stream opening sends no event (unless
        // the server replays it, RFC 8620 §7.3): check once now that nothing will be missed.
        if !std::mem::take(&mut self.pushed)
            && let Some(f) = self.inbox()
        {
            self.sync_one(&f);
        }
        let end = Instant::now() + IDLE_FOR;
        let mut changed = false;
        while Instant::now() < end && !changed {
            self.drain(rx);
            if self.closed || !self.queue.is_empty() {
                return true;
            }
            match es.next(Duration::from_millis(500)) {
                Ok(Some(e)) => changed = e.event == "state",
                Ok(None) => {}
                Err(e) => {
                    // Dropped: reopened on the next wait.
                    log::info!("{}: JMAP push ended: {e}", self.acc.id);
                    break;
                }
            }
        }
        if changed && let Some(f) = self.inbox() {
            self.sync_one(&f);
            self.pushed = true;
        }
        true
    }

    pub(super) fn inbox(&self) -> Option<String> {
        self.folders.iter().find(|f| f.role == FolderRole::Inbox && self.acc.folder_owner(&f.path).is_none()).map(|f| f.path.clone())
    }

    /// The folder list, then every folder (Inbox first), staying responsive to opened messages.
    /// Refresh the folder list from the server; `false` when that failed.
    pub(super) fn relist(&mut self) -> bool {
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
                true
            }
            Err(e) => {
                self.failed(e);
                false
            }
        }
    }

    pub(super) fn sync_all(&mut self, rx: &Receiver<Job>) {
        let began = Instant::now();
        if !self.relist() {
            return;
        }
        if began.elapsed() > crate::stream::SLOW_CONNECT {
            log::info!("{}: the folders listed in {:.1} s", self.acc.id, began.elapsed().as_secs_f32());
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
    pub(super) fn prune_jmap_numbers(&mut self) {
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

    pub(super) fn publish_folders(&self) {
        let _ = self.store.save_folders(&self.acc.id, &self.folders);
        self.sink.emit(Event::Folders { account: self.id(), folders: self.folders.clone() });
    }

    /// The public folders the session asks to follow, as it last wrote them.
    pub(super) fn read_followed(&mut self) {
        self.followed = self.store.load_account_doc(&self.acc.id, quatrelle_model::FOLLOWED_PUBLIC_DOC).unwrap_or_default();
    }

    /// Whether a folder is read on the server when it's opened rather than kept here, as
    /// Outlook's Cached Exchange Mode does: a public folder not followed (a Favorite, when public
    /// Favorites are kept), and a shared mailbox's folder when shared mailboxes aren't kept.
    pub(super) fn read_online(&self, path: &str) -> bool {
        if quatrelle_model::is_public_folder(path) {
            return !self.followed.contains(path);
        }
        self.acc.shared_online && self.folders.iter().any(|f| f.path == path && f.mailbox.is_some())
    }

    /// How a folder is read: how many of its newest, and whether it's only listed. A folder
    /// asked to keep more keeps them (the API servers keep only the newest window); a folder
    /// read on the server is listed when opened, its newest few hundred, nothing downloaded.
    pub(super) fn options_for(&self, path: &str) -> SyncOptions {
        let more = self.windows.get(path).copied().unwrap_or(0);
        if self.read_online(path) {
            return SyncOptions { window: more.max(ONLINE_WINDOW), prefetch: 0, list_only: true, full: false, since_days: 0, ..self.opts };
        }
        SyncOptions { window: more.max(self.opts.window), ..self.opts }
    }

    /// What earlier passes kept of folders now read on the server (a public folder held up to a
    /// hundred thousand messages, most of a big mailbox's cache) is let go: they keep what they
    /// show when opened. Their message numbers go at the end of the pass.
    pub(super) fn let_go_of_online(&mut self) {
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

    pub(super) fn sync_one(&mut self, path: &str) {
        self.sync_one_newest(path);
    }

    /// [`Self::sync_one`], and when the server received the newest message that came in, if one
    /// did (for the log).
    pub(super) fn sync_one_newest(&mut self, path: &str) -> Option<i64> {
        let _at_work = self.at_work();
        self.status(SyncStatus::Syncing { folder: path.to_string() });
        let (store, opts, id) = (self.store.clone(), self.options_for(path), self.id());
        // What its saves change in the list, told the window as it is (see `tell_noted`).
        let noting = store.note_changes(&id, path);
        // An edit still waiting is put back on the list once it's read (`apply_report`): the
        // list isn't shown before then, or it would show the server's old state a moment.
        let early = matches!(self.conn, Some(Conn::Ews(_))) && !self.waiting_in(path);
        let result = match self.conn.as_mut() {
            Some(Conn::Imap(c)) => sync_folder(c, &store, &id, path, &opts),
            Some(Conn::Jmap(c)) => {
                let r = jmap::sync_folder(c, &mut self.jstate, &store, &id, path, &opts);
                self.save_jstate();
                r
            }
            Some(Conn::Graph(c)) => {
                let r = crate::graph::sync_folder(c, &mut self.jstate, &store, &id, path, &opts);
                self.save_jstate();
                r
            }
            Some(Conn::Ews(c)) => {
                // New mail shows once its header is in, while it's downloaded whole.
                let sink = &self.sink;
                let mut listed = || {
                    if early {
                        tell_noted(sink, &id, path, noting.so_far());
                    }
                };
                let r = crate::ews::sync_folder_then(c, &mut self.jstate, &store, &id, path, &opts, &mut listed);
                self.save_jstate();
                r
            }
            // POP: the Inbox is downloaded; every other folder is here only.
            Some(Conn::Pop) if self.folders.iter().any(|f| f.path == path && f.role == FolderRole::Inbox) => {
                let secret = self.secret();
                (|| {
                    let mut c = crate::pop::open(&self.acc, &secret?)?;
                    let r = crate::pop::fetch_new(&mut c, &store, &id, path, &crate::pop::Keep::of(&self.acc), now())?;
                    let _ = c.quit();
                    Ok(r)
                })()
            }
            Some(Conn::Pop) => crate::pop::local_counts(&store, &id, path),
            None => Err(NetError::Closed("not connected".into())),
        };
        let newest = result.as_ref().ok().and_then(|r| r.newest);
        self.apply_report(path, result, noting);
        newest
    }

    /// What a folder's sync changed: its counts, its list if any, and the account's state.
    /// `noting`: what its saves changed in the list, told once an edit still waiting is made
    /// again on it.
    pub(super) fn apply_report(&mut self, path: &str, result: std::result::Result<SyncReport, NetError>, noting: NoteGuard) {
        match result {
            Ok(mut r) => {
                // The token worked: a later refusal gets a fresh one again.
                self.token_retried = false;
                // The listing predates an edit still waiting: re-apply it to what was just saved,
                // or the folder shows the server's old state until the edit goes out.
                if self.reapply_waiting(path) {
                    r.changed += 1;
                }
                let first = self.folders.iter().find(|f| f.path == path).is_some_and(|f| f.total == 0 && r.total > 0);
                if let Some(f) = self.folders.iter_mut().find(|f| f.path == path) {
                    let counts_changed = f.unread != r.unread || f.total != r.total;
                    f.unread = r.unread;
                    f.total = r.total;
                    if counts_changed {
                        self.publish_folders();
                    }
                }
                // A first list is read whole; otherwise what changed, as it is.
                if first {
                    drop(noting);
                    self.sink.emit(Event::FolderChanged { account: self.acc.id.clone(), folder: path.to_string() });
                } else {
                    tell_noted(&self.sink, &self.acc.id, path, noting.take());
                }
                if self.last_full.is_some() {
                    self.status(SyncStatus::Online { at: now() });
                }
            }
            Err(e) => {
                // What was saved before it failed is on the disk all the same.
                tell_noted(&self.sink, &self.acc.id, path, noting.take());
                self.failed(e);
            }
        }
    }

    /// Read folders on connections of their own (up to `opts.parallel`), each taking folders
    /// from a shared list, while this connection stays free for what someone is waiting on.
    /// Answers the folders no connection read (none unless one was refused): the caller reads
    /// them here.
    pub(super) fn sync_parallel(&mut self, paths: Vec<String>, rx: &Receiver<Job>) -> Vec<String> {
        let n = self.opts.parallel.min(3).min(paths.len().saturating_sub(1));
        if n == 0 || matches!(self.conn, Some(Conn::Pop) | None) {
            return paths;
        }
        let Ok(secret) = self.secret() else {
            return paths;
        };
        // Each folder with how it's read: one asked to keep more keeps them on a reader too
        // (they read every folder with the account's window, and cut such a folder back to it).
        let queue = Arc::new(Mutex::new(
            paths
                .into_iter()
                .map(|p| {
                    let o = self.options_for(&p);
                    (p, o)
                })
                .collect::<VecDeque<(String, SyncOptions)>>(),
        ));
        let (tx, done) = mpsc::channel::<SyncDone>();
        let mut running = 0usize;
        // The numbers given so far, one copy for every reader: they must know the messages
        // they find (see `State::reserve`).
        let numbering = self.jstate.numbering();
        for _ in 0..n {
            // The numbers it may give, kept apart from this connection's (see `State::reserve`).
            let state = self.jstate.reserve(1_000_000, &numbering);
            let (acc, store) = (self.acc.clone(), self.store.clone());
            let (secret, q, tx) = (secret.clone(), queue.clone(), tx.clone());
            if std::thread::Builder::new().name(format!("{} sync", acc.id)).spawn(move || sync_folders(acc, store, secret, state, q, tx)).is_err() {
                break;
            }
            running += 1;
        }
        drop(tx);
        drop(numbering);
        // This connection answers what someone waits on meanwhile, then takes a folder of its
        // own if the others are still at it.
        let mut refused = false;
        while running > 0 {
            self.drain(rx);
            // What someone waits on, and the edits just made: a read mark went to the server
            // only once every reader was done, a whole first sync later (the server kept the
            // messages unread meanwhile, and its counts said so).
            let now = |j: &Job| jumps_queue(j) || between_folders(j);
            let urgent: Vec<Job> = self.queue.iter().filter(|j| now(j)).cloned().collect();
            self.queue.retain(|j| !now(j));
            for j in urgent {
                self.handle(j);
            }
            if self.closed {
                break;
            }
            match done.recv_timeout(std::time::Duration::from_millis(25)) {
                Ok(SyncDone::Folder(path, result, given, noted)) => {
                    // Known here before the folder is shown: a message in it may be read (and
                    // the mark sent) at once.
                    self.jstate.learn(given);
                    self.status(SyncStatus::Syncing { folder: path.clone() });
                    match result {
                        // A reader that timed out, lost its connection or failed to sign in has
                        // stopped (the server doesn't want several of us right now: Exchange
                        // throttles a first full sync). Its folder is read again on this
                        // connection, after the others, and the rest goes on this connection
                        // too. It says nothing about this connection: taken as this
                        // connection's failure, a reader's dropped connection asked for the
                        // password while the password was fine.
                        Err(e) if reader_stops(&e) => {
                            log::warn!("{}: reading {path} on a connection of its own: {e}; read on the main one", self.acc.id);
                            tell_noted(&self.sink, &self.acc.id, &path, noted);
                            self.opts.parallel = 0;
                            let opts = self.options_for(&path);
                            if let Ok(mut q) = queue.lock() {
                                q.push_back((path, opts));
                            }
                        }
                        result => {
                            // Told as the reader noted it (and what's saved meanwhile here).
                            let noting = self.store.note_changes(&self.acc.id, &path);
                            tell_noted(&self.sink, &self.acc.id, &path, noted);
                            self.apply_report(&path, result, noting);
                        }
                    }
                }
                Ok(SyncDone::State(state)) => {
                    self.jstate.absorb(*state);
                    running -= 1;
                }
                Ok(SyncDone::Refused) => {
                    // A server that takes only one connection (or says it's busy): the rest waits
                    // for this one, and this account syncs on one connection from now on.
                    refused = true;
                    self.opts.parallel = 0;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        self.jstate.compact();
        self.save_jstate();
        // Whatever no connection took (a refusal, or a thread that couldn't start).
        let _ = refused;
        queue.lock().map(|q| q.iter().map(|(p, _)| p.clone()).collect()).unwrap_or_default()
    }

    /// Other people's calendars just ticked, read alone, or (none named) every calendar ticked
    /// as the calendar opens, without the address books (on-premises Exchange): seconds, where
    /// the calendars' and contacts' sync, which reads them elsewhere, takes half a minute. Each
    /// calendar ticked is shown as soon as it's read, not with the last of those ticked
    /// together (two ticked in a row showed after 15 s, 29/10).
    pub(super) fn sync_calendars(&mut self, calendars: &[String]) {
        let _at_work = self.at_work();
        // On the second connection (`main` names the first), connected as it's needed.
        if self.conn.is_none()
            && self.main.is_some()
            && let Err(e) = self.connect()
        {
            log::info!("{}: calendars: {e}", self.acc.id);
            return;
        }
        let Some(Conn::Ews(c)) = &self.conn else { return self.sync_dav() };
        let pim = self.load_pim();
        let each: Vec<&[String]> = if calendars.is_empty() { vec![calendars] } else { calendars.chunks(1).collect() };
        for ids in each {
            match crate::ews_pim::sync_calendars(c, &pim, ids, now()) {
                Ok(sync) => self.sink.emit(Event::DavSynced { account: self.id(), sync: Box::new(sync) }),
                Err(e) => log::info!("{}: calendars: {e}", self.acc.id),
            }
        }
    }

    /// The Tasks folder alone (on-premises Exchange), read as the server tells of a change in it:
    /// a tenth of a second, where the calendars' and contacts' sync takes seconds.
    pub(super) fn sync_tasks(&mut self) {
        self.tasks_synced();
        let Some(Conn::Ews(c)) = &self.conn else { return };
        let pim = self.load_pim();
        let sync = crate::ews_pim::sync_tasks(c, &pim, now());
        self.sink.emit(Event::DavSynced { account: self.id(), sync: Box::new(sync) });
    }

    /// Report the space used on the server (when it publishes a quota).
    pub(super) fn report_quota(&mut self) {
        let q = match self.conn.as_mut() {
            Some(Conn::Imap(c)) => c.quota(),
            Some(Conn::Jmap(c)) => c.quota(),
            Some(Conn::Ews(c)) => sizes::ews_quota(c),
            // Graph doesn't tell a mailbox's size to the mailbox's owner.
            Some(Conn::Graph(_) | Conn::Pop) | None => return,
        };
        match q {
            Ok(Some((used, limit))) => self.sink.emit(Event::Quota { account: self.id(), used, limit }),
            Ok(None) => {}
            Err(e) if e.is_fatal() => self.failed(e),
            Err(e) => log::info!("{}: quota: {e}", self.acc.id),
        }
    }

    /// Answer a message's voting buttons; the choice is kept with the message once it's gone.
    pub(super) fn vote(&mut self, key: MessageKey, choice: String) {
        let r = (|| -> Result<()> {
            if self.conn.is_none() {
                self.connect()?;
            }
            let subject =
                self.store.load_folder(&key.account, &key.folder).ok().and_then(|f| f.find(key.uid).map(|m| m.subject.clone())).unwrap_or_default();
            match self.conn.as_ref() {
                Some(Conn::Graph(c)) => crate::graph::vote(c, &self.jstate, &key, &choice, &subject),
                Some(Conn::Ews(c)) => crate::ews::vote(c, &self.jstate, &key, &choice, &subject),
                _ => Err(NetError::No("voting buttons can only be answered on Exchange and Microsoft 365 accounts".into())),
            }
        })();
        match r {
            Ok(()) => {
                if let Ok(mut fs) = self.store.load_folder(&key.account, &key.folder)
                    && let Some(m) = fs.find_mut(key.uid)
                {
                    m.voted = choice.clone();
                    if let Err(e) = self.store.save_folder(&key.account, &key.folder, &fs) {
                        self.notice(e.to_string());
                    }
                }
                self.sink.emit(Event::Voted { key: key.clone(), choice, error: None });
                self.sink.emit(Event::FolderChanged { account: key.account, folder: key.folder });
            }
            Err(e) => {
                self.sink.emit(Event::Voted { key, choice, error: Some(e.to_string()) });
                if e.is_fatal() || matches!(e, NetError::Auth(_)) {
                    self.failed(e);
                }
            }
        }
    }
}

/// What a sync's saves changed in a folder's list, told the window: as it is, so the folder it
/// holds and its list take it (`FolderSynced`); as a change to read again when it can't be told
/// so (`FolderChanged`); nothing when nothing changed.
pub(super) fn tell_noted(sink: &Sink, account: &str, folder: &str, noted: Noted) {
    match noted {
        Noted::Nothing => {}
        Noted::Changed(change) => sink.emit(Event::FolderSynced { account: account.to_string(), folder: folder.to_string(), change }),
        Noted::Unknown => sink.emit(Event::FolderChanged { account: account.to_string(), folder: folder.to_string() }),
    }
}
