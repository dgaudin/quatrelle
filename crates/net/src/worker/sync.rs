//! An account's worker: syncing folders, searching the server, calendars and contacts, and waiting for new mail.

use super::*;

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
            Ok(true) => self.sync_one(&inbox),
            Ok(false) => {}
            Err(e) => self.failed(e),
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
                // Folders gone from the server lose their cache.
                for old in &self.folders {
                    if !list.iter().any(|f| f.path == old.path) {
                        let _ = self.store.clear_folder(&id, &old.path);
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
        self.read_followed();
        self.let_go_of_online();
        let mut paths = sync_order(&self.folders, |f: &Folder| !self.read_online(&f.path));
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
            self.sync_one(&path);
        }
        // "Online" is said once the mail is current: the calendars and contacts can take far
        // longer, and shouldn't hold the status bar on the last folder's name.
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
            self.sync_dav();
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

    /// Search folders on the server (issue #4): IMAP's `SEARCH`, JMAP's `Email/query`, Graph's
    /// `$search` and EWS's `QueryString`, the messages found brought into their folders' lists
    /// and sent as they're found (an IMAP folder a range at a time, the others a folder at a
    /// time). Stops between two when a newer search is asked, or none (the search box cleared).
    pub(super) fn search_server(&mut self, folders: &[String], query: String, search: u64) {
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
        self.status(SyncStatus::Syncing { folder: path.to_string() });
        let (store, opts, id) = (self.store.clone(), self.options_for(path), self.id());
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
                let r = crate::ews::sync_folder(c, &mut self.jstate, &store, &id, path, &opts);
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
            Some(Conn::Pop) => Ok(crate::pop::local_counts(&store, &id, path)),
            None => Err(NetError::Closed("not connected".into())),
        };
        self.apply_report(path, result);
    }

    /// What a folder's sync changed: its counts, its list if any, and the account's state.
    pub(super) fn apply_report(&mut self, path: &str, result: std::result::Result<SyncReport, NetError>) {
        match result {
            Ok(mut r) => {
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
                if r.touched() || first {
                    self.sink.emit(Event::FolderChanged { account: self.acc.id.clone(), folder: path.to_string() });
                }
                if self.last_full.is_some() {
                    self.status(SyncStatus::Online { at: now() });
                }
            }
            Err(e) => self.failed(e),
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
                Ok(SyncDone::Folder(path, result, given)) => {
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
                            self.opts.parallel = 0;
                            let opts = self.options_for(&path);
                            if let Ok(mut q) = queue.lock() {
                                q.push_back((path, opts));
                            }
                        }
                        result => self.apply_report(&path, result),
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

    /// Sync calendars and address books with the account's CalDAV / CardDAV server (password
    /// accounts: OAuth tokens are for mail only).
    pub(super) fn sync_dav(&mut self) {
        self.pim_synced();
        // Exchange Online: its calendars and contacts through Graph.
        if let Some(Conn::Graph(c)) = &self.conn {
            let pim: quatrelle_model::pim::Pim = self.store.load_account_doc(&self.acc.id, quatrelle_model::pim::PIM_DOC).unwrap_or_default();
            match crate::graph_pim::sync(c, &pim, now()) {
                Ok(sync) => self.sink.emit(Event::DavSynced { account: self.id(), sync: Box::new(sync) }),
                Err(e) => log::info!("{}: calendars and contacts: {e}", self.acc.id),
            }
            return;
        }
        // On-premises Exchange: its calendar and contacts through EWS.
        if let Some(Conn::Ews(c)) = &self.conn {
            let pim: quatrelle_model::pim::Pim = self.store.load_account_doc(&self.acc.id, quatrelle_model::pim::PIM_DOC).unwrap_or_default();
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
        // OAuth tokens are for mail only.
        if self.acc.oauth.is_some() || self.acc.ews.is_some() {
            return;
        }
        let pim: quatrelle_model::pim::Pim = self.store.load_account_doc(&self.acc.id, quatrelle_model::pim::PIM_DOC).unwrap_or_default();
        let user = if self.acc.username.is_empty() { self.acc.email.clone() } else { self.acc.username.clone() };
        let password = self.auth.lock().unwrap_or_else(std::sync::PoisonError::into_inner).secret.clone();
        let dav = crate::dav::Dav::new(&user, &password);
        let host = self.acc.pop.as_ref().map_or(self.acc.imap.host.as_str(), |p| p.host.as_str());
        match crate::dav::sync(&dav, &self.acc.email, host, &pim, now(), system_zone()) {
            Ok(sync) => self.sink.emit(Event::DavSynced { account: self.id(), sync: Box::new(sync) }),
            Err(e) => {
                log::info!("{}: calendars and contacts: {e}", self.acc.id);
                if matches!(e, NetError::Auth(_)) {
                    self.notice(format!("calendars and contacts: {e}"));
                }
            }
        }
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

    /// Search with the server's own index (Exchange's): the local one can only find a message
    /// whose text is here, and a search over years of mail needs the rest.
    pub(super) fn search(&mut self, query: String, folder: Option<String>) {
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
                let mut fs: Vec<&quatrelle_model::Folder> = self.folders.iter().collect();
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

    pub(super) fn fetch(&mut self, key: MessageKey) {
        if let Some(raw) = self.store.read_body(&key) {
            self.sink.emit(Event::Body { key, raw });
            return;
        }
        let store = self.store.clone();
        let numbered = self.jstate.len();
        let r = (|| -> Result<Vec<u8>> {
            if self.conn.is_none() {
                self.connect()?;
            }
            match self.conn.as_mut() {
                Some(Conn::Jmap(c)) => jmap::fetch_body(c, &self.jstate, &store, &key),
                Some(Conn::Graph(c)) => crate::graph::fetch_body(c, &self.jstate, &store, &key),
                Some(Conn::Ews(c)) => crate::ews::fetch_body(c, &mut self.jstate, &store, &key),
                // POP mail is downloaded whole: what isn't here isn't anywhere.
                Some(Conn::Pop) => store.read_body(&key).ok_or_else(|| NetError::No("this message isn't on this computer".into())),
                _ => fetch_body(self.client()?, &store, &key),
            }
        })();
        if self.jstate.len() != numbered {
            // A message found again by its Message-ID: the numbering is worth keeping.
            self.save_jstate();
        }
        match r {
            Ok(raw) => {
                self.sink.emit(Event::Body { key: key.clone(), raw });
                self.sink.emit(Event::FolderChanged { account: key.account.clone(), folder: key.folder.clone() });
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

/// The system's time zone, for calendar times written without one (`UTC` when it can't say).
fn system_zone() -> quatrelle_model::zone::Zone {
    iana_time_zone::get_timezone().ok().and_then(|z| quatrelle_model::zone::Zone::named(&z)).unwrap_or_default()
}
