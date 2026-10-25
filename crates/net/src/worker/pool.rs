//! The pool: a worker started for each account, the jobs handed to them, and the events they report back.

use super::*;

impl Pool {
    pub fn new(store: Store, creds: Arc<dyn Credentials>) -> Pool {
        let (tx, rx) = mpsc::channel();
        let sink = Sink { tx, wake: Arc::new(Mutex::new(None)) };
        let (etx, erx) = mpsc::channel();
        let (st, sk) = (store.clone(), sink.clone());
        let edits_thread = std::thread::Builder::new().name("mail-edits".into()).spawn(move || edit_writer(st, sk, erx)).ok();
        Pool {
            edits: edits_thread.is_some().then_some(etx),
            edits_thread,
            store,
            creds,
            sink,
            rx,
            workers: HashMap::new(),
            configs: HashMap::new(),
            opts: SyncOptions::default(),
            sieve_port: crate::sieve::PORT,
            fetcher: None,
            offline: false,
            search: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Browser sign-in for an OAuth account, on its own thread: the page opens through the UI
    /// (`Event::OpenUrl`), the refresh token goes to the keychain, then the account starts.
    fn sign_in(&mut self, account: AccountConfig) {
        let Some(p) = crate::oauth::provider_for_account(&account) else {
            let error = format!("signing in with {} isn't set up in this copy of Quatrelle", account.oauth.as_deref().unwrap_or("that provider"));
            return self.sink.emit(Event::Status { account: account.id, status: SyncStatus::NeedsPassword { error } });
        };
        let (sink, creds) = (self.sink.clone(), self.creds.clone());
        let spawned = std::thread::Builder::new().name("mail-sign-in".into()).spawn(move || {
            let mut open = |url: &str| -> std::result::Result<(), String> {
                sink.emit(Event::OpenUrl { url: url.to_string() });
                Ok(())
            };
            match crate::oauth::authorize(&p, &account.email, &mut open, Duration::from_secs(5 * 60)) {
                Ok(t) => match t.refresh {
                    Some(r) => match creds.set(&account, &r) {
                        Ok(()) => sink.emit(Event::SignedIn { account: account.id, user: t.user }),
                        Err(e) => sink.emit(Event::Status {
                            account: account.id,
                            status: SyncStatus::NeedsPassword { error: format!("the sign-in couldn't be kept in the keychain: {e}") },
                        }),
                    },
                    None => sink.emit(Event::Status {
                        account: account.id,
                        status: SyncStatus::NeedsPassword { error: format!("{} gave no lasting sign-in; try again", p.name) },
                    }),
                },
                Err(e) => sink.emit(Event::Status { account: account.id, status: SyncStatus::NeedsPassword { error: e.to_string() } }),
            }
        });
        if let Err(e) = spawned {
            log::warn!("sign-in: {e}");
        }
    }

    fn fetch_url(&mut self, url: String) {
        if self.fetcher.is_none() {
            let (tx, rx) = mpsc::channel::<String>();
            let sink = self.sink.clone();
            let spawned = std::thread::Builder::new().name("mail-pictures".into()).spawn(move || {
                for url in rx {
                    // A message's picture: from the internet only, never this computer's network.
                    let r = crate::http::get_public(&url, MAX_PICTURE);
                    let e = match r {
                        Ok(resp) if resp.ok() => Event::Fetched { url, data: Some(resp.body), error: None },
                        Ok(resp) => Event::Fetched { url, data: None, error: Some(format!("HTTP {}", resp.status)) },
                        Err(e) => Event::Fetched { url, data: None, error: Some(e.to_string()) },
                    };
                    sink.emit(e);
                }
            });
            match spawned {
                Ok(_) => self.fetcher = Some(tx),
                Err(e) => return self.sink.emit(Event::Fetched { url, data: None, error: Some(e.to_string()) }),
            }
        }
        if let Some(tx) = &self.fetcher {
            let _ = tx.send(url);
        }
    }

    /// Export to a data file on a thread of its own.
    fn export_pst(&mut self, account: String, path: String, name: String) {
        let sink = self.sink.clone();
        let store = self.store.clone();
        let spawned = std::thread::Builder::new().name("mail-export".into()).spawn(move || {
            let mut progress = |messages| sink.emit(Event::Exported { account: account.clone(), messages, skipped: 0, done: false, error: None });
            let event = match crate::pst_export::export(&store, &account, &name, std::path::Path::new(&path), &mut progress) {
                Ok(done) => Event::Exported { account, messages: done.messages, skipped: done.skipped, done: true, error: None },
                Err(e) => Event::Exported { account, messages: 0, skipped: 0, done: true, error: Some(e) },
            };
            sink.emit(event);
        });
        if let Err(e) = spawned {
            self.sink.emit(Event::Notice { account: String::new(), message: format!("export: {e}") });
        }
    }

    /// Import a data file on a thread of its own: an Outlook one (`.pst`), or Quatrelle's own
    /// (`.quatrelle`, copied in as it is).
    /// Compact a data file, on a thread of its own (a minute for 40 GB): whatever else wants
    /// the account's file waits for it.
    fn compact(&mut self, account: String) {
        let sink = self.sink.clone();
        let store = self.store.clone();
        let spawned = std::thread::Builder::new().name("compact".into()).spawn(move || {
            let done = match store.compact_data_file(&account) {
                Ok(Some((before, after))) => Event::Compacted { account, before, after, error: None },
                Ok(None) => Event::Compacted { account, before: 0, after: 0, error: Some("this account has no data file".into()) },
                Err(e) => Event::Compacted { account, before: 0, after: 0, error: Some(e.to_string()) },
            };
            sink.emit(done);
        });
        if let Err(e) = spawned {
            log::warn!("compact: {e}");
        }
    }

    /// Each folder's size on this computer, worked out on a thread of its own (a big data file
    /// has hundreds of thousands of entries to add up).
    fn local_sizes(&mut self, account: String) {
        let sink = self.sink.clone();
        let store = self.store.clone();
        let spawned = std::thread::Builder::new().name("folder-sizes".into()).spawn(move || {
            let done = match store.folder_sizes(&account) {
                Ok(folders) => Event::FolderSizes { account, server: false, folders, quota: None, error: None },
                Err(e) => Event::FolderSizes { account, server: false, folders: Vec::new(), quota: None, error: Some(e.to_string()) },
            };
            sink.emit(done);
        });
        if let Err(e) = spawned {
            log::warn!("folder sizes: {e}");
        }
    }

    fn import_pst(&mut self, account: String, path: String) {
        let sink = self.sink.clone();
        let store = self.store.clone();
        let spawned = std::thread::Builder::new().name("mail-import".into()).spawn(move || {
            if path.to_ascii_lowercase().ends_with(".quatrelle") {
                match store.import_data_file(&account, std::path::Path::new(&path)) {
                    Ok(messages) => {
                        let folders = store.load_folders(&account).unwrap_or_default();
                        let paths: Vec<String> = folders.iter().map(|f| f.path.clone()).collect();
                        sink.emit(Event::Folders { account: account.clone(), folders });
                        for folder in paths {
                            sink.emit(Event::FolderChanged { account: account.clone(), folder });
                        }
                        sink.emit(Event::Imported { account, messages, skipped: 0, done: true, error: None });
                    }
                    Err(e) => sink.emit(Event::Imported { account, messages: 0, skipped: 0, done: true, error: Some(e.to_string()) }),
                }
                return;
            }
            let mut progress = |messages| sink.emit(Event::Imported { account: account.clone(), messages, skipped: 0, done: false, error: None });
            let result = crate::pst::import(std::path::Path::new(&path), &store, &account, &mut progress);
            match result {
                Ok(done) => {
                    let paths: Vec<String> = done.folders.iter().map(|f| f.path.clone()).collect();
                    sink.emit(Event::Folders { account: account.clone(), folders: done.folders });
                    for folder in paths {
                        sink.emit(Event::FolderChanged { account: account.clone(), folder });
                    }
                    sink.emit(Event::Imported { account, messages: done.messages, skipped: done.skipped, done: true, error: None });
                }
                Err(e) => sink.emit(Event::Imported { account, messages: 0, skipped: 0, done: true, error: Some(e) }),
            }
        });
        if let Err(e) = spawned {
            self.sink.emit(Event::Notice { account: String::new(), message: format!("import: {e}") });
        }
    }

    /// Called (from worker threads) after every event, e.g. to request a UI repaint.
    pub fn set_waker(&self, f: Arc<dyn Fn() + Send + Sync>) {
        if let Ok(mut w) = self.sink.wake.lock() {
            *w = Some(f);
        }
    }

    pub fn submit(&mut self, job: Job) {
        if let Job::SearchServer { search, .. } = &job {
            self.search.store(*search, Ordering::Relaxed);
        }
        match job {
            Job::FetchUrl { url } => self.fetch_url(url),
            Job::StopSearch => self.search.store(0, Ordering::Relaxed),
            Job::CheckUpdates => {
                let sink = self.sink.clone();
                let spawned = std::thread::Builder::new().name("mail-updates".into()).spawn(move || {
                    let (releases, error) = match crate::updates::releases() {
                        Ok(r) => (r, None),
                        Err(e) => (Vec::new(), Some(e.to_string())),
                    };
                    sink.emit(Event::Releases { releases, error });
                });
                if let Err(e) = spawned {
                    self.sink.emit(Event::Releases { releases: Vec::new(), error: Some(e.to_string()) });
                }
            }
            Job::SignIn { account } => self.sign_in(account),
            Job::Discover { email } => {
                let sink = self.sink.clone();
                let spawned = std::thread::Builder::new().name("mail-discover".into()).spawn(move || {
                    let found = crate::autoconfig::discover(&email);
                    let (config, source) = match found {
                        Some(f) => (Some(Box::new(f.config)), f.source),
                        None => (None, String::new()),
                    };
                    sink.emit(Event::Discovered { email, config, source });
                });
                if let Err(e) = spawned {
                    log::warn!("autoconfiguration: {e}");
                }
            }
            Job::ImportPst { account, path } => self.import_pst(account, path),
            Job::CompactDataFile { account } => self.compact(account),
            Job::LocalFolderSizes { account } => self.local_sizes(account),
            j @ (Job::ImportIdentity { .. }
            | Job::ForgetIdentity { .. }
            | Job::Protect { .. }
            | Job::Decrypt { .. }
            | Job::CheckRevocation { .. }
            | Job::FindHeldKeys { .. }
            | Job::UseHeldKey { .. }) => self.smime(j),
            Job::SealDraft { account, raw, folder, replaces, keep } => {
                let raw = self.seal(&account, raw.0);
                let account = account.id;
                self.submit(if keep { Job::KeepDraft { account, raw, folder, replaces } } else { Job::SaveDraft { account, raw, folder, replaces } });
            }
            Job::ExportPst { account, path, name } => self.export_pst(account, path, name),
            Job::SetSync { days, messages, full } => {
                let window = if messages == 0 { usize::MAX } else { messages as usize };
                let opts = SyncOptions { window, since_days: days, full, ..self.opts };
                if opts.window != self.opts.window || opts.since_days != self.opts.since_days || opts.full != self.opts.full {
                    self.opts = opts;
                    // Connected accounts start again with it (a rare change, from Options).
                    let running: Vec<AccountConfig> = self.workers.keys().filter_map(|id| self.configs.get(id).cloned()).collect();
                    for c in running {
                        self.start(c, None);
                    }
                }
            }
            Job::WorkOffline { on } => {
                self.offline = on;
                if on {
                    let ids: Vec<String> = self.workers.keys().cloned().collect();
                    for id in ids {
                        self.stop(&id);
                        self.sink.emit(Event::Status { account: id, status: SyncStatus::Offline });
                    }
                } else {
                    let configs: Vec<AccountConfig> = self.configs.values().cloned().collect();
                    for c in configs {
                        self.start(c, None);
                    }
                }
            }
            Job::Start { account, password } if self.offline => {
                // Working offline: remembered (and its password kept) for when that's turned off.
                if let Some(p) = &password
                    && let Err(e) = self.creds.set(&account, &p.0)
                {
                    self.sink.emit(Event::Notice {
                        account: account.id.clone(),
                        message: format!("the password could not be saved in the keychain: {e}"),
                    });
                }
                self.sink.emit(Event::Status { account: account.id.clone(), status: SyncStatus::Offline });
                self.configs.insert(account.id.clone(), account);
            }
            Job::Start { account, password } => self.start(account, password),
            Job::SmtpPassword { account, password } => {
                if let Err(e) = self.creds.set_smtp(&account, &password.0) {
                    self.sink.emit(Event::Notice {
                        account: account.id.clone(),
                        message: format!("the outgoing server's password could not be saved in the keychain: {e}"),
                    });
                }
            }
            Job::Remove { account } => {
                self.stop(&account);
                if let Some(c) = self.configs.remove(&account) {
                    self.creds.delete(&c);
                    self.creds.delete_smtp(&c);
                }
            }
            Job::Search { account, query, .. } if !self.workers.contains_key(&account) => {
                // No connection: the local index's answer stands.
                self.sink.emit(Event::Found { account, query, items: Vec::new(), error: Some("the account is offline".into()) });
            }
            Job::FetchBody { key } if !self.workers.contains_key(&key.account) => {
                // Offline or not started: the cache may still have it.
                match self.store.read_body(&key) {
                    Some(raw) => self.sink.emit(Event::Body { key, raw }),
                    None => self.sink.emit(Event::BodyFailed { key, error: "not downloaded yet, and the account is offline".into() }),
                }
            }
            Job::Send { account, id, raw, from, recipients, sent_folder } if !self.workers.contains_key(&account) => {
                // Not connected: straight into the Outbox.
                let item = OutboxItem { id, from, recipients, sent_folder, queued: now(), attempts: 0, error: Some("offline".into()), not_before: 0 };
                match self.store.outbox_add(&account, &item, &raw) {
                    Ok(()) => self.sink.emit(Event::Queued { account, id, error: "offline".into() }),
                    Err(e) => self.sink.emit(Event::SendFailed { account, id, error: e.to_string() }),
                }
            }
            // Offline: changes wait in the cache for the next sync.
            Job::SyncDav { account } if !self.workers.contains_key(&account) => {}
            Job::SearchServer { account, query, .. } if !self.workers.contains_key(&account) => {
                self.sink.emit(Event::ServerSearched { account, query, keys: Vec::new(), error: Some("the account is offline".into()), done: true });
            }
            // Working offline: nothing connects, by choice.
            Job::SyncAll { account } | Job::SyncFolder { account, .. } | Job::FlushOutbox { account }
                if self.offline && !self.workers.contains_key(&account) => {}
            // Closing with a message written: on disk before the app goes (the worker may be busy
            // or about to stop); filed on the server at the next connection, as offline drafts.
            Job::KeepDraft { account, raw, folder, replaces } => {
                if let Err(e) = keep_draft(&self.store, &account, &folder, replaces, raw) {
                    log::warn!("{account}: the draft couldn't be kept: {e}");
                }
            }
            Job::SaveDraft { account, raw, folder, replaces } if !self.workers.contains_key(&account) => {
                match keep_draft(&self.store, &account, &folder, replaces, raw) {
                    Ok(()) => self.sink.emit(Event::Notice {
                        account,
                        message: "the draft is kept on this computer; it goes to Drafts when you're back online".into(),
                    }),
                    Err(e) => self.sink.emit(Event::Notice { account, message: format!("the draft couldn't be kept: {e}") }),
                }
            }
            Job::Vote { key, choice } if !self.workers.contains_key(&key.account) => {
                self.sink.emit(Event::Voted { key, choice, error: Some("the account is offline".into()) });
            }
            Job::GetVacation { account } | Job::SetVacation { account, .. } if !self.workers.contains_key(&account) => {
                self.sink.emit(Event::Vacation { account, vacation: None, error: Some("the account is offline".into()) });
            }
            Job::ServerFolderSizes { account } if !self.workers.contains_key(&account) => {
                let error = Some("the account is offline".to_string());
                self.sink.emit(Event::FolderSizes { account, server: true, folders: Vec::new(), quota: None, error });
            }
            Job::FindCertificates { account, .. } if !self.workers.contains_key(&account) => {
                self.sink.emit(Event::DirectoryCertificates { account, found: Vec::new(), error: Some("the account is offline".into()) });
            }
            Job::GetRooms { account } if !self.workers.contains_key(&account) => {
                self.sink.emit(Event::Rooms { account, lists: Vec::new(), rooms: Vec::new(), error: Some("the account is offline".into()) });
            }
            Job::GetAvailability { account, emails, from, to } if !self.workers.contains_key(&account) => {
                let error = "the account is offline".to_string();
                let people = emails
                    .into_iter()
                    .map(|email| quatrelle_model::scheduling::Availability { email, slots: Vec::new(), error: Some(error.clone()) })
                    .collect();
                self.sink.emit(Event::Availability { account, from, to, people, error: Some(error) });
            }
            // The change goes into the cache and ops.json here, on the caller's thread, before
            // the account thread is free: it may be minutes into a full sync. Waiting meant a
            // folder refresh could paint the server's old state over the edit, and closing the
            // app dropped it. A POP account's worker keeps its own bookkeeping, so it still goes
            // that way.
            Job::Apply { account, op } if self.records_edits(&account) => {
                // Into the cache and ops.json on the edit writer's thread (in order, before the
                // account's thread hears of it): not on the window's.
                let worker = self.workers.get(&account).map(|h| h.tx.clone());
                let unsent = match &self.edits {
                    Some(tx) => tx.send((account, op, worker)).err().map(|mpsc::SendError(e)| e),
                    None => Some((account, op, worker)),
                };
                if let Some((account, op, worker)) = unsent {
                    write_edit(&self.store, &self.sink, account, op, worker);
                }
            }
            other => {
                let id = other.account().to_string();
                let quick = self.configs.get(&id).is_some_and(|c| quick_job(&other, c));
                match self.workers.get(&id) {
                    Some(h) => {
                        // Sending and automatic replies go on the second connection; if it has
                        // stopped, the main one does it (after its sync).
                        let other = match h.quick.as_ref().filter(|_| quick) {
                            Some(q) => match q.send(other) {
                                Ok(()) => return,
                                Err(mpsc::SendError(j)) => j,
                            },
                            None => other,
                        };
                        if h.tx.send(other).is_err() {
                            self.sink.emit(Event::Notice {
                                account: id,
                                message: "the account's connection stopped; restart it from the account menu".into(),
                            });
                        }
                    }
                    None => self.sink.emit(Event::Notice { account: id, message: "the account is not connected".into() }),
                }
            }
        }
    }

    /// Whether this side writes an account's edit to the cache and ops.json: always when no
    /// thread runs for it, and for a running one unless it is POP (whose thread does it, with
    /// bookkeeping of its own).
    fn records_edits(&self, account: &str) -> bool {
        match self.workers.get(account) {
            None => true,
            Some(_) => matches!(self.configs.get(account), Some(c) if c.pop.is_none()),
        }
    }

    /// Events reported since the last call.
    pub fn poll(&self) -> Vec<Event> {
        self.rx.try_iter().collect()
    }

    /// Block until an event arrives or `timeout` passes (for the CLI).
    pub fn wait(&self, timeout: Duration) -> Option<Event> {
        self.rx.recv_timeout(timeout).ok()
    }

    fn start(&mut self, account: AccountConfig, password: Option<Secret>) {
        // The account's threads from before end first: one still saving its numbering after
        // the new one read it took the numbers it gave last with it (messages not found again).
        let before = self.stopping(&account.id);
        if let Some(p) = &password
            && let Err(e) = self.creds.set(&account, &p.0)
        {
            self.sink.emit(Event::Notice { account: account.id.clone(), message: format!("the password could not be saved in the keychain: {e}") });
        }
        let pw = password.map(|p| p.0).or_else(|| self.creds.get(&account));
        let Some(pw) = pw else {
            self.sink.emit(Event::Status { account: account.id.clone(), status: SyncStatus::NeedsPassword { error: String::new() } });
            return;
        };
        // The workers are set up on the account's own thread: each reads the account's numbering
        // (tens of megabytes for a big Exchange mailbox), which froze the window at every start,
        // password typed, Work offline and settings change. Jobs sent meanwhile wait in the
        // channels.
        let (tx, rx) = mpsc::channel();
        let (qtx, qrx) = mpsc::channel();
        let (creds, store, sink, opts, sieve_port, search) =
            (self.creds.clone(), self.store.clone(), self.sink.clone(), self.opts, self.sieve_port, self.search.clone());
        let (acc, main_tx) = (account.clone(), tx.clone());
        let spawned = std::thread::Builder::new().name(format!("mail-{}", account.id)).spawn(move || {
            for t in before {
                let _ = t.join();
            }
            let auth = Auth::shared(pw);
            let mut w = Worker::new(acc.clone(), auth.clone(), store.clone(), sink.clone(), opts);
            w.creds = Some(creds.clone());
            w.sieve_port = sieve_port;
            w.search = search;
            // The second connection: quiet (the account's status is the main one's), connecting
            // only when there's something to do. Should it not start, its jobs go to the main one.
            let mut q = Worker::with_numbering(acc.clone(), auth, store, sink, opts, acc.ews.is_none());
            q.creds = Some(creds);
            q.sieve_port = sieve_port;
            q.main = Some(main_tx);
            let quick = std::thread::Builder::new().name(format!("mail-{}-quick", acc.id)).spawn(move || q.run_quick(qrx)).ok();
            w.run(rx);
            if let Some(t) = quick {
                let _ = t.join();
            }
        });
        match spawned {
            Ok(thread) => {
                self.workers.insert(account.id.clone(), Handle { tx, thread: Some(thread), quick: Some(qtx), quick_thread: None });
            }
            Err(e) => self.sink.emit(Event::Status { account: account.id.clone(), status: SyncStatus::Error { error: e.to_string() } }),
        }
        self.configs.insert(account.id.clone(), account);
    }

    fn stop(&mut self, account: &str) {
        for t in self.stopping(account) {
            std::thread::spawn(move || {
                let _ = t.join();
            });
        }
    }

    /// Tell an account's threads to stop; the threads, to wait for. They notice the closed
    /// channel within half a second (IDLE slices), or once a slow network call ends: never
    /// waited for on the window's thread.
    fn stopping(&mut self, account: &str) -> Vec<JoinHandle<()>> {
        let Some(mut h) = self.workers.remove(account) else { return Vec::new() };
        drop(h.tx);
        drop(h.quick.take());
        [h.thread.take(), h.quick_thread.take()].into_iter().flatten().collect()
    }

    pub fn shutdown(&mut self) {
        // Every edit made is written before the app goes (closing must not drop one).
        drop(self.edits.take());
        if let Some(t) = self.edits_thread.take() {
            let _ = t.join();
        }
        let ids: Vec<String> = self.workers.keys().cloned().collect();
        for id in ids {
            self.stop(&id);
        }
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.shutdown();
    }
}
