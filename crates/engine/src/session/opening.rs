//! The session: opening and selecting mail.

use super::*;

impl Session {
    pub fn open_view(&mut self, view: View) {
        // A reply written in the reading pane is set aside (in Drafts) for the folder.
        self.set_compose_aside(None);
        // Older mail read on the server belongs to the folder it was read for.
        if !matches!((&view, &self.older), (View::Folder { account, folder }, Some((a, f, ..))) if a == account && f == folder) {
            self.older = None;
        }
        // A folder not kept on this computer (a public folder, a shared mailbox's when they
        // aren't kept) is read from the server when it's opened, as in Outlook: its newest few
        // hundred come in, and the list fills as they do.
        if let View::Folder { account, folder } = &view
            && self.read_on_server(account, folder)
        {
            self.jobs.push(Job::SyncFolder { account: account.clone(), folder: folder.clone() });
        }
        self.view = view;
        self.filter_keep = None;
        self.search = None;
        self.stop_server_search();
        self.search_pending = false;
        self.index = None;
        self.selection.clear();
        self.focus = None;
        self.reading = None;
        self.leave_reading(None);
        self.refresh_list();
    }

    /// Mark a message read (when it's there and isn't yet).
    pub(super) fn mark_seen(&mut self, key: &MessageKey) {
        if key.folder != OUTBOX && self.summary(key).is_some_and(|m| !m.flags.seen) {
            self.apply_op(&key.account.clone(), Op::SetFlag { keys: vec![key.clone()], flag: Flag::Seen, on: true });
        }
    }

    /// The reading pane moves on from what it showed (to `next`, or to nothing): one waiting for
    /// that is marked read now, and one waiting for its time no longer is.
    pub(super) fn leave_reading(&mut self, next: Option<&MessageKey>) {
        if self.read_due.as_ref().is_some_and(|(k, _)| Some(k) != next) {
            self.read_due = None;
        }
        if let Some(prev) = self.read_on_leave.take() {
            if Some(&prev) == next {
                self.read_on_leave = Some(prev);
            } else {
                self.mark_seen(&prev);
            }
        }
    }

    /// A message waiting in the reading pane for its time to be marked read, once it's come.
    pub(crate) fn read_when_due(&mut self) {
        let Some((key, at)) = self.read_due.clone() else { return };
        if self.clock.now >= at {
            self.read_due = None;
            if self.reading.as_ref().is_some_and(|r| r.key == key) {
                self.mark_seen(&key);
            }
        }
    }

    /// Select one message and show it in the reading pane (marked read as the options say).
    pub fn open_message(&mut self, key: &MessageKey) {
        self.show_message(key, true);
    }

    /// Select one message and show it; `mark`: as the reading options say (else left as it is).
    pub(super) fn show_message(&mut self, key: &MessageKey, mark: bool) {
        // A reply written in the reading pane is set aside (in Drafts) for another message, and
        // one set aside for this message comes back.
        self.set_compose_aside(Some(key));
        self.take_up(key);
        let kept_before = self.filter_keep.clone().filter(|k| k != key);
        self.leave_reading(Some(key));
        self.filter_keep = Some(key.clone());
        self.selection = vec![key.clone()];
        self.focus = Some(key.clone());
        let thread = self.thread_of(key);
        self.reading = Some(Reading { key: key.clone(), parsed: None, error: None, thread, warning: None, security: None });
        let cached = if key.folder == OUTBOX {
            self.outbox_entry(key).and_then(|(a, i)| self.store.as_ref().and_then(|s| s.outbox_raw(&a, i.id)))
        } else {
            self.store.as_ref().and_then(|s| s.read_body(key))
        };
        match cached {
            Some(raw) => self.show_body(key, &raw),
            None => self.jobs.push(self.fetch_job(key)),
        }
        let unread = key.folder != OUTBOX && self.summary(key).is_some_and(|m| !m.flags.seen);
        if mark && unread && self.prefs.mark_read_on_open {
            match (self.prefs.read_in_pane, self.prefs.read_after_secs) {
                (ReadInPane::Shown, 0) => self.mark_seen(key),
                (ReadInPane::Shown, secs) => self.read_due = Some((key.clone(), self.clock.now.saturating_add(i64::from(secs.min(3600))))),
                (ReadInPane::Left, _) => self.read_on_leave = Some(key.clone()),
            }
        }
        // The one kept under the filter while it was read goes, now that another is: marked read
        // as it was left, it stayed under Unread (from upstream aaed8d6).
        if let Some(prev) = kept_before
            && self.list.iter().any(|m| m.key == prev && !self.filter_keeps(self.filter, m))
        {
            self.refresh_list();
        }
    }

    /// Open a message in its own window (it stays open while the list changes). Already open:
    /// nothing changes. At most [`MAX_WINDOWS`]; the oldest closes first.
    pub fn pop_out(&mut self, key: &MessageKey) -> CmdResult {
        if !self.windows.iter().any(|w| &w.key == key) {
            if self.windows.len() >= MAX_WINDOWS {
                self.windows.remove(0);
            }
            let cached = if key.folder == OUTBOX {
                self.outbox_entry(key).and_then(|(a, i)| self.store.as_ref().and_then(|s| s.outbox_raw(&a, i.id)))
            } else {
                self.store.as_ref().and_then(|s| s.read_body(key))
            };
            let mut w = Reading { key: key.clone(), parsed: None, error: None, thread: Vec::new(), warning: None, security: None };
            match cached {
                Some(raw) => {
                    let (shown, security) = self.unwrap(key, &raw);
                    w.parsed = mime::parse(&shown);
                    w.error = w.parsed.is_none().then(|| "this message could not be read".to_string());
                    w.warning = w.parsed.as_ref().and_then(|p| self.sender_warning(&p.from));
                    w.security = security;
                }
                None => self.jobs.push(self.fetch_job(key)),
            }
            self.windows.push(w);
            // Opened on its own (a double-click): read, whatever the reading pane does.
            self.mark_seen(key);
        }
        Ok(serde_json::json!({"key": key.to_id(), "windows": self.windows.len()}))
    }

    /// How a message not on this computer is brought: kept as it comes, or, older than what's
    /// kept, read on the server for showing only.
    pub(super) fn fetch_job(&self, key: &MessageKey) -> Job {
        if self.is_older(key) { Job::ReadOnline { key: key.clone() } } else { Job::FetchBody { key: key.clone() } }
    }

    /// Whether a message is one of the older mail read on the server, not kept.
    pub(super) fn is_older(&self, key: &MessageKey) -> bool {
        self.older.as_ref().is_some_and(|(_, _, items, _)| items.iter().any(|m| &m.key == key))
    }

    pub fn close_window(&mut self, key: &MessageKey) {
        self.windows.retain(|w| &w.key != key);
    }

    pub(super) fn show_body(&mut self, key: &MessageKey, raw: &[u8]) {
        // Signed or encrypted: what's inside is what's shown, the protection said beside it.
        let shown = self.reading.as_ref().is_some_and(|r| &r.key == key) || self.windows.iter().any(|w| &w.key == key && w.parsed.is_none());
        let (raw, security) = if shown { self.unwrap(key, raw) } else { (raw.to_vec(), None) };
        let raw = raw.as_slice();
        let mut windows = std::mem::take(&mut self.windows);
        for w in windows.iter_mut().filter(|w| &w.key == key && w.parsed.is_none()) {
            w.parsed = mime::parse(raw);
            w.error = w.parsed.is_none().then(|| "this message could not be read".to_string());
            w.warning = w.parsed.as_ref().and_then(|p| self.sender_warning(&p.from));
            w.security = security.clone();
        }
        self.windows = windows;
        // Only the open message is laid out (bodies downloaded ahead aren't, each costing time).
        if let Some(r) = self.reading.as_mut()
            && &r.key == key
        {
            let parsed = mime::parse(raw);
            r.error = if parsed.is_none() { Some("this message could not be read".into()) } else { None };
            r.parsed = parsed;
            r.security = security;
        }
        let from = self.reading.as_ref().filter(|r| &r.key == key).and_then(|r| r.parsed.as_ref()).map(|p| p.from.clone());
        if let Some(from) = from {
            let warning = self.sender_warning(&from);
            if let Some(r) = self.reading.as_mut() {
                r.warning = warning;
            }
        }
        self.request_pictures();
    }

    /// The domains the user deals with: their own accounts', and those of the people they write
    /// to or keep in an address book.
    pub fn known_domains(&mut self) -> BTreeSet<String> {
        let revision = self.contacts_revision();
        if let Some((at, known)) = &self.known_domains
            && *at == revision
        {
            return known.clone();
        }
        let mut known: BTreeSet<String> = self.accounts.iter().map(|a| quatrelle_model::lookalike::domain_of(&a.config.email)).collect();
        known.extend(self.written_domains());
        known.remove("");
        // As the book is now (reading it may have changed it).
        self.known_domains = Some((self.contacts_revision(), known.clone()));
        known
    }

    /// What doesn't add up about a sender (nothing for the user's own messages).
    pub fn sender_warning(&mut self, from: &EmailAddress) -> Option<quatrelle_model::lookalike::SenderWarning> {
        if self.accounts.iter().any(|a| a.config.email.eq_ignore_ascii_case(from.email.trim())) {
            return None;
        }
        let known = self.known_domains();
        quatrelle_model::lookalike::check_sender(from, &known)
    }

    /// A body that came in joins the search index there is (rather than throwing it away).
    pub(super) fn index_body(&mut self, key: &MessageKey, raw: &[u8]) {
        let words: Arc<str> = Arc::from(mime::search_text(raw));
        // Kept even without an index: a search made later needs no reading again.
        self.keep_words(key, words.clone());
        let Some(ix) = self.index.as_mut() else { return };
        // The folder being read numbers its messages from zero, the whole index after it.
        if let Some(b) = ix.building.as_mut()
            && let Some(doc) = b.keys.iter().position(|k| k == key)
        {
            if b.has_body.get(doc) != Some(&true) {
                b.index.add(u32::try_from(doc).unwrap_or(u32::MAX), &words);
                if let Some(seen) = b.has_body.get_mut(doc) {
                    *seen = true;
                }
            }
            return;
        }
        if let Some(doc) = ix.keys.iter().position(|k| k == key)
            && ix.has_body.get(doc) == Some(&false)
        {
            ix.index.add(u32::try_from(doc).unwrap_or(u32::MAX), &words);
            if let Some(seen) = ix.has_body.get_mut(doc) {
                *seen = true;
            }
        }
    }
}
