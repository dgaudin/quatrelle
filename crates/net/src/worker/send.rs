// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! An account's worker: sending (on a second connection when it's free), the copy in Sent Items, and the Outbox;
//! and the second connection's other work (other people's calendars just ticked, read at once).

use super::*;

/// Calendars just ticked read at the same time, at most (each over `ews_pim`'s four
/// connections): the next waits for the first of them.
const TICKED_AT_ONCE: usize = 2;

/// The second connection's next job among those waiting: a message to send first (a question
/// asked as recipients are written, MailTips, never holds one back), then the others in the
/// order they came.
fn next_quick(waiting: &mut VecDeque<Job>) -> Option<Job> {
    let first = waiting.iter().position(|j| matches!(j, Job::Send { .. })).unwrap_or(0);
    waiting.remove(first)
}

/// Said when a sent message's copy has nowhere to go.
const NO_SENT: &str = "sent, but no copy was saved: the server has no Sent Items folder, and one couldn't be made";

/// Said, before what went wrong, when a message sent from the Outbox couldn't be taken out of it.
const SENT_NOT_TAKEN_OUT: &str = "a message was sent but couldn't be taken out of the Outbox; while Quatrelle is open it isn't sent again, and it's taken out as soon as it can be";

/// What the Outbox couldn't be told of one of its messages ([`Worker::outbox_done`]).
pub(super) enum Untold {
    /// It went: to take out of the Outbox.
    Sent,
    /// Refused for good, or maybe gone: its bookkeeping, saying so, to write.
    Marked(Box<OutboxItem>),
}

impl Worker {
    /// The account's second connection: sends mail, reads or changes the automatic replies, and
    /// answers for rooms, free/busy and MailTips, whatever the main connection is doing. It connects when
    /// there's something to do, and lets go of an idle connection after a while.
    pub(super) fn run_quick(mut self, rx: Receiver<Job>) {
        let mut reading = Vec::new();
        // What came while the last job ran, taken in [`next_quick`]'s order.
        let mut waiting: VecDeque<Job> = VecDeque::new();
        loop {
            while let Ok(j) = rx.try_recv() {
                waiting.push_back(j);
            }
            if waiting.is_empty() {
                match rx.recv_timeout(QUICK_IDLE) {
                    Ok(j) => waiting.push_back(j),
                    Err(RecvTimeoutError::Timeout) => {
                        self.let_go();
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
                while let Ok(j) = rx.try_recv() {
                    waiting.push_back(j);
                }
            }
            let Some(job) = next_quick(&mut waiting) else { continue };
            // Behind the same fence as the main connection's turns (`fenced`).
            let done = fenced(|| {
                self.fresh_token();
                match job {
                    Job::Send { account, id, raw, from, recipients, sent_folder } => {
                        self.send_quick(Job::Send { account, id, raw, from, recipients, sent_folder });
                    }
                    Job::SyncCalendars { calendars, .. } if !calendars.is_empty() => self.read_ticked(calendars, &mut reading),
                    j @ (Job::GetVacation { .. }
                    | Job::SetVacation { .. }
                    | Job::GetRooms { .. }
                    | Job::GetAvailability { .. }
                    | Job::GetMailTips { .. }
                    | Job::ServerFolderSizes { .. }
                    | Job::ListRecoverable { .. }
                    | Job::RestoreRecoverable { .. }
                    | Job::FindCertificates { .. }
                    | Job::SearchDirectory { .. }
                    | Job::ExpandList { .. }
                    | Job::SyncCalendars { .. }) => self.handle(j),
                    // Anything else belongs to the main connection.
                    other => {
                        if let Some(m) = &self.main {
                            let _ = m.send(other);
                        }
                    }
                }
            });
            if done.is_none() {
                self.after_quick_panic();
            }
        }
        for r in reading {
            let _ = r.join();
        }
        self.let_go();
    }

    /// Other people's calendars just ticked (on-premises Exchange), each read on a connection of
    /// its own, while the next is ticked: one ticked as the one before was being read waited for
    /// it, and showed 5.7 s later where alone it took 3 (29/10). [`TICKED_AT_ONCE`] at most;
    /// `reading`, those being read.
    pub(super) fn read_ticked(&mut self, calendars: Vec<String>, reading: &mut Vec<JoinHandle<()>>) {
        if self.conn.is_none()
            && let Err(e) = self.connect()
        {
            log::info!("{}: calendars: {e}", self.acc.id);
            return;
        }
        let Some(Conn::Ews(c)) = &self.conn else { return self.sync_calendars(&calendars) };
        let pim = Arc::new(self.load_pim());
        for id in calendars {
            reading.retain(|r| !r.is_finished());
            while reading.len() >= TICKED_AT_ONCE {
                let _ = reading.remove(0).join();
            }
            let (sibling, pim_there, sink, account, there) = (c.sibling(), Arc::clone(&pim), self.sink.clone(), self.id(), id.clone());
            let read = move || read_calendar(&sibling, &pim_there, &there, &sink, account);
            match std::thread::Builder::new().name(format!("calendar-{}", self.acc.id)).spawn(read) {
                Ok(r) => reading.push(r),
                // Read here instead, the next waiting for it.
                Err(e) => {
                    log::info!("{}: a calendar ticked read on this connection ({e})", self.acc.id);
                    read_calendar(c, &pim, &id, &self.sink, self.id());
                }
            }
        }
    }

    /// Close an idle connection (servers drop them anyway).
    pub(super) fn let_go(&mut self) {
        if let Some(Conn::Imap(c)) = self.conn.as_mut() {
            c.logout();
        }
        self.conn = None;
    }

    /// Send on the second connection. Sent: a copy filed in Sent Items (IMAP), and the main
    /// connection told to look there. Not sent for now (offline, asked to wait): handed to the
    /// main connection, which keeps it in the Outbox; refused, or maybe gone (the server never
    /// answered once it had the whole of it, `Unconfirmed`): said, and never handed on, which
    /// would send it again.
    pub(super) fn send_quick(&mut self, job: Job) {
        let Job::Send { id, raw, from, recipients, sent_folder, .. } = &job else { return };
        match self.deliver(raw, from, recipients, sent_folder.as_deref()) {
            Ok(()) => {
                log::info!("{}: message {id} sent to {} recipient(s)", self.acc.id, recipients.len());
                self.sink.emit(Event::Sent { account: self.id(), id: *id });
                self.file_sent_quick(raw, sent_folder.clone());
            }
            Err(e) if e.is_retryable() => {
                log::info!("{}: sending on the second connection: {e}; left to the main one", self.acc.id);
                self.conn = None;
                if let Some(m) = &self.main
                    && m.send(job.clone()).is_err()
                {
                    self.sink.emit(Event::SendFailed { account: self.id(), id: *id, error: e.to_string(), unconfirmed: false });
                }
            }
            Err(e) => {
                if matches!(e, NetError::Closed(_) | NetError::Io(_)) {
                    self.conn = None;
                }
                let unconfirmed = matches!(e, NetError::Unconfirmed(_));
                self.sink.emit(Event::SendFailed { account: self.id(), id: *id, error: e.to_string(), unconfirmed });
            }
        }
    }

    /// Gmail files sent mail itself.
    fn files_itself(&self) -> bool {
        self.acc.smtp.host.ends_with("gmail.com") || self.acc.smtp.host.ends_with("googlemail.com")
    }

    /// The folder a sent message's copy goes in: the one it was sent with, else the account's
    /// Sent Items. An IMAP account with none gets one, as Thunderbird, Evolution and Outlook make
    /// one (issue #19: sent mail was kept nowhere, and nothing said so): `Sent`, or `INBOX.Sent`
    /// on a server that keeps every folder in the Inbox. Connects to make it.
    pub(super) fn sent_folder(&mut self, given: Option<String>) -> Result<Option<String>> {
        if let Some(f) = given.or_else(|| self.folders.iter().find(|f| f.role == FolderRole::Sent).map(|f| f.path.clone())) {
            return Ok(Some(f));
        }
        if self.acc.is_api() || self.files_itself() {
            return Ok(None);
        }
        self.role_folder(FolderRole::Sent, "Sent")
    }

    /// The account's folder for `role` (Sent Items, Drafts) on an IMAP server that lists none:
    /// one there by its name without the role (another client's), else one made, `name` or
    /// `INBOX.name` on a server that keeps every folder in the Inbox. Connects to make it.
    pub(super) fn role_folder(&mut self, role: FolderRole, name: &str) -> Result<Option<String>> {
        if let Some(f) = self.folders.iter().find(|f| f.role == role) {
            return Ok(Some(f.path.clone()));
        }
        if self.conn.is_none() {
            self.connect()?;
        }
        if !matches!(self.conn, Some(Conn::Imap(_))) {
            return Ok(None);
        }
        // A folder there by the name, without the role (another client's, unflagged): that one.
        if let Some(f) = self.folders.iter_mut().find(|f| f.role == FolderRole::Other && FolderRole::from_name(&f.path, f.delimiter) == role) {
            f.role = role;
            let path = f.path.clone();
            self.publish_folders();
            return Ok(Some(path));
        }
        let delimiter = self
            .folders
            .iter()
            .find(|f| f.role == FolderRole::Inbox)
            .and_then(|f| f.delimiter)
            .or_else(|| self.folders.iter().find_map(|f| f.delimiter));
        let others: Vec<&Folder> = self.folders.iter().filter(|f| !f.path.eq_ignore_ascii_case("INBOX")).collect();
        let in_inbox = delimiter
            .map(|d| format!("INBOX{d}"))
            .filter(|p| !others.is_empty() && others.iter().all(|f| f.path.to_ascii_uppercase().starts_with(p.as_str())));
        let mut names = Vec::new();
        if let Some(p) = &in_inbox {
            names.push(format!("{p}{name}"));
        }
        names.push(name.to_string());
        if let (None, Some(d)) = (&in_inbox, delimiter) {
            names.push(format!("INBOX{d}{name}"));
        }
        for name in names {
            match self.client()?.create(&name) {
                Ok(()) => {
                    log::info!("{}: no {role:?} folder on the server; made {name}", self.acc.id);
                    self.folders.retain(|f| f.path != name);
                    self.folders.push(Folder { account: self.id(), path: name.clone(), delimiter, role, selectable: true, ..Default::default() });
                    sort_folders(&mut self.folders);
                    self.publish_folders();
                    return Ok(Some(name));
                }
                // Not allowed there (a server that keeps folders in the Inbox): the next name.
                Err(NetError::No(m)) => log::info!("{}: making {name}: {m}", self.acc.id),
                Err(e) => return Err(e),
            }
        }
        Ok(None)
    }

    /// A sent message's copy: Exchange, Graph and JMAP file it themselves; IMAP servers get one
    /// (not Gmail, which files its own). Then the main connection reads Sent Items, now and, where
    /// the server files it a moment later, again then.
    pub(super) fn file_sent_quick(&mut self, raw: &[u8], sent_folder: Option<String>) {
        // What the main connection has listed since this one started.
        if let Ok(listed) = self.store.load_folders(&self.acc.id)
            && !listed.is_empty()
        {
            self.folders = listed;
        }
        let folder = if self.acc.is_api() || self.files_itself() {
            sent_folder.or_else(|| self.folders.iter().find(|f| f.role == FolderRole::Sent).map(|f| f.path.clone()))
        } else {
            let r = (|| -> Result<Option<String>> {
                let Some(folder) = self.sent_folder(sent_folder)? else { return Ok(None) };
                if self.conn.is_none() {
                    self.connect()?;
                }
                self.client()?.append(&folder, "\\Seen", raw)?;
                Ok(Some(folder))
            })();
            match r {
                Ok(Some(f)) => {
                    log::info!("{}: sent copy saved in {f}", self.acc.id);
                    Some(f)
                }
                Ok(None) => {
                    self.notice(NO_SENT.to_string());
                    None
                }
                Err(e) => {
                    self.conn = None;
                    self.notice(format!("sent, but no copy was saved in Sent Items: {e}"));
                    return;
                }
            }
        };
        let Some(folder) = folder else { return };
        let Some(main) = self.main.clone() else { return };
        let job = Job::SyncFolder { account: self.id(), folder: folder.clone() };
        let _ = main.send(job.clone());
        if self.acc.is_api() {
            // Graph and Exchange file the copy a moment after taking the message. Not past this
            // connection's end: it would keep a stopped account's main thread waiting for it.
            let alive = Arc::downgrade(&self.alive);
            let _ = std::thread::Builder::new().name("sent-later".into()).spawn(move || {
                for wait in [SENT_SOON, SENT_LATER.saturating_sub(SENT_SOON)] {
                    let until = Instant::now() + wait;
                    while let Some(left) = until.checked_duration_since(Instant::now()).filter(|l| !l.is_zero()) {
                        if alive.upgrade().is_none() {
                            return;
                        }
                        std::thread::sleep(left.min(SENT_LATER_SLICE));
                    }
                    if main.send(job.clone()).is_err() {
                        return;
                    }
                }
            });
        }
    }

    /// Hand a message to the server: SMTP, or for JMAP accounts a submission (which also files
    /// the copy in Sent).
    /// An OAuth token the server refuses (SMTP's XOAUTH2, Graph's 401) is renewed from the
    /// refresh token and the message tried once more.
    pub(super) fn deliver(&mut self, raw: &[u8], from: &str, recipients: &[String], sent_folder: Option<&str>) -> Result<()> {
        match self.deliver_once(raw, from, recipients, sent_folder) {
            Err(NetError::Auth(m)) if self.acc.oauth.is_some() => {
                log::info!("{}: sending: token refused ({m}); refreshing it", self.acc.id);
                self.renew_token()?;
                self.deliver_once(raw, from, recipients, sent_folder)
            }
            r => r,
        }
    }

    fn deliver_once(&mut self, raw: &[u8], from: &str, recipients: &[String], sent_folder: Option<&str>) -> Result<()> {
        if self.acc.graph.is_some() {
            if self.conn.is_none() {
                self.connect()?;
            }
            let Some(Conn::Graph(c)) = &self.conn else { return Err(NetError::Closed("not connected".into())) };
            return crate::graph::send(c, raw, recipients);
        }
        if self.acc.ews.is_some() {
            if self.conn.is_none() {
                self.connect()?;
            }
            let Some(Conn::Ews(c)) = &self.conn else { return Err(NetError::Closed("not connected".into())) };
            return crate::ews::send(c, raw, recipients);
        }
        if self.acc.jmap.is_some() {
            // Its copy goes in the Sent folder the numbering knows: while that can't be read, the
            // message waits in the Outbox (nothing is asked of the server).
            self.reread_numbering().map_err(|e| NetError::Later(e.to_string()))?;
            if self.conn.is_none() {
                self.connect()?;
            }
            let sent = sent_folder.map(str::to_string).or_else(|| self.folders.iter().find(|f| f.role == FolderRole::Sent).map(|f| f.path.clone()));
            let Some(Conn::Jmap(c)) = &self.conn else { return Err(NetError::Closed("not connected".into())) };
            return jmap::send(c, &self.jstate, sent.as_deref(), raw, from, recipients);
        }
        let (user, secret) = match self.acc.smtp_username.as_deref().map(str::trim).filter(|u| !u.is_empty() && self.acc.oauth.is_none()) {
            // The outgoing server's own sign-in (issue #3: its user name differs from the
            // incoming server's), with its own password.
            Some(u) => {
                let pw = self.creds.as_ref().and_then(|c| c.get_smtp(&self.acc)).ok_or_else(|| {
                    NetError::Auth(format!(
                        "the outgoing server's password for {u} isn't on this computer: enter it in the account's Server settings"
                    ))
                })?;
                (u.to_string(), pw)
            }
            None => (if self.acc.username.is_empty() { self.acc.email.clone() } else { self.acc.username.clone() }, self.secret()?),
        };
        smtp::send_with(&self.acc.smtp, &user, &secret, self.acc.oauth.is_some(), from, recipients, raw).map_err(|e| match e {
            // Say whether the sign-in covered sending: Microsoft lists what it granted.
            NetError::Auth(m) if self.acc.oauth.as_deref() == Some("microsoft") => {
                let granted = self.granted();
                let why = if granted.is_empty() {
                    String::new()
                } else if granted.to_ascii_lowercase().contains("smtp.send") {
                    " (the sign-in includes sending: Microsoft's server refused it)".to_string()
                } else {
                    format!(" (Microsoft's sign-in didn't include sending; it granted: {granted})")
                };
                NetError::Auth(format!("{m}{why}"))
            }
            e => e,
        })
    }

    /// Send on the main connection. Not sent for now: kept in the Outbox; refused, or maybe gone
    /// (`Unconfirmed`, which the Outbox would send again): said.
    pub(super) fn send(&mut self, id: u64, raw: &[u8], from: &str, recipients: &[String], sent_folder: Option<String>) {
        let sent = self.deliver(raw, from, recipients, sent_folder.as_deref());
        if let Err(e) = sent {
            if e.is_retryable() {
                // Offline or asked to wait: keep it in the Outbox and send it on reconnect.
                let item = OutboxItem {
                    id,
                    from: from.to_string(),
                    recipients: recipients.to_vec(),
                    sent_folder,
                    queued: now(),
                    attempts: 1,
                    error: Some(e.to_string()),
                    not_before: 0,
                };
                match self.store.outbox_add(&self.acc.id, &item, raw) {
                    Ok(()) => self.sink.emit(Event::Queued { account: self.id(), id, error: e.to_string() }),
                    Err(se) => self.sink.emit(Event::SendFailed {
                        account: self.id(),
                        id,
                        error: format!("{e} (and the Outbox couldn't keep it: {se})"),
                        unconfirmed: false,
                    }),
                }
            } else {
                let unconfirmed = matches!(e, NetError::Unconfirmed(_));
                self.sink.emit(Event::SendFailed { account: self.id(), id, error: e.to_string(), unconfirmed });
            }
            return;
        }
        self.after_sent(id, raw, sent_folder);
    }

    /// Send what waits in the Outbox, oldest first. Stops at the first failure that may clear
    /// up later (still offline); a message refused for good stays, marked `refused:` with the
    /// error, and one that may have gone (`Unconfirmed`) stays marked `unconfirmed:`: neither is
    /// sent again by itself.
    ///
    /// What was taken out of it before and left files is cleared away first
    /// ([`Store::outbox_sweep`]); a message the Outbox couldn't be told of
    /// ([`Worker::outbox_done`]) is told again, and doesn't go in this round either way.
    pub(super) fn flush_outbox(&mut self) {
        self.store.outbox_sweep(&self.acc.id);
        let untold: HashSet<u64> = self.outbox_done.keys().copied().collect();
        self.tell_outbox_again();
        for mut item in self.store.outbox(&self.acc.id) {
            // Sent, refused or maybe gone already: the Outbox couldn't be told.
            if untold.contains(&item.id) {
                continue;
            }
            // Held back (Delay Delivery) until its time; then dated when it goes.
            if item.not_before > now() {
                continue;
            }
            let Some(raw) = self.store.outbox_raw(&self.acc.id, item.id) else { continue };
            let raw = if item.not_before > 0 { quatrelle_mime::with_date(&raw, now()) } else { raw };
            if item.error.as_deref().is_some_and(|e| e.starts_with("refused:") || e.starts_with("unconfirmed:")) {
                // Refused for good, or maybe sent already: waits for the user's word.
                continue;
            }
            let sent = self.deliver(&raw, &item.from, &item.recipients, item.sent_folder.as_deref());
            match sent {
                Ok(()) => {
                    // Left in the Outbox, it would go again.
                    let kept = self.store.outbox_remove(&self.acc.id, item.id).err();
                    if kept.is_some() {
                        self.outbox_done.insert(item.id, Untold::Sent);
                    }
                    let folder = item.sent_folder.take();
                    self.after_sent(item.id, &raw, folder);
                    if let Some(e) = kept {
                        log::error!("{}: message {} sent, but the Outbox couldn't be told", self.acc.id, item.id);
                        self.notice(format!("{SENT_NOT_TAKEN_OUT}: {e}"));
                    }
                }
                Err(e) => {
                    let retry = e.is_retryable();
                    item.attempts = item.attempts.saturating_add(1);
                    item.error = Some(match &e {
                        _ if retry => e.to_string(),
                        NetError::Unconfirmed(_) => format!("unconfirmed: {e}"),
                        _ => format!("refused: {e}"),
                    });
                    if self.store.outbox_update(&self.acc.id, &item).is_err() {
                        log::warn!("{}: message {}: why it wasn't sent couldn't be kept in the Outbox", self.acc.id, item.id);
                        // Refused for good or maybe gone: never tried again for want of a mark.
                        if !retry {
                            self.outbox_done.insert(item.id, Untold::Marked(Box::new(item.clone())));
                        }
                    }
                    if retry {
                        self.sink.emit(Event::Queued { account: self.id(), id: item.id, error: e.to_string() });
                        return;
                    }
                    let unconfirmed = matches!(e, NetError::Unconfirmed(_));
                    self.sink.emit(Event::SendFailed { account: self.id(), id: item.id, error: e.to_string(), unconfirmed });
                }
            }
        }
    }

    /// Tells the Outbox again what it couldn't be told ([`Worker::outbox_done`]): a message that
    /// went is taken out, a refused one marked so, each forgotten once that's done. A refused one
    /// no longer there (deleted, or taken back to be changed) is forgotten, nothing written.
    fn tell_outbox_again(&mut self) {
        if self.outbox_done.is_empty() {
            return;
        }
        let (store, account) = (&self.store, &self.acc.id);
        let there: HashSet<u64> = store.outbox(account).iter().map(|i| i.id).collect();
        self.outbox_done.retain(|id, untold| {
            let told = match untold {
                Untold::Sent => store.outbox_remove(account, *id).is_ok(),
                Untold::Marked(item) => !there.contains(id) || store.outbox_update(account, item).is_ok(),
            };
            if told {
                log::info!("{account}: message {id}: the Outbox is told of it now");
            }
            !told
        });
    }

    /// A message went out: tell the engine and file a copy in Sent.
    pub(super) fn after_sent(&mut self, id: u64, raw: &[u8], sent_folder: Option<String>) {
        self.sink.emit(Event::Sent { account: self.id(), id });
        if self.acc.is_api() {
            // Filed when it was submitted.
            let sent = sent_folder.or_else(|| self.folders.iter().find(|f| f.role == FolderRole::Sent).map(|f| f.path.clone()));
            if let Some(f) = sent.filter(|_| self.conn.is_some()) {
                self.sync_one(&f);
                // Graph and Exchange file the copy a moment later: look again then.
                if matches!(self.conn, Some(Conn::Graph(_) | Conn::Ews(_))) {
                    let now = Instant::now();
                    self.soon.retain(|(_, g)| *g != f);
                    self.soon.push((now + SENT_SOON, f.clone()));
                    self.soon.push((now + SENT_LATER, f));
                }
            }
            return;
        }
        // Gmail files sent mail itself; elsewhere keep a copy in Sent.
        if self.files_itself() {
            return;
        }
        let r = (|| -> Result<Option<String>> {
            if self.conn.is_none() {
                self.connect()?;
            }
            // POP: Sent Items is on this computer.
            if let Some(Conn::Pop) = self.conn.as_ref() {
                let folder = sent_folder.clone().unwrap_or_else(|| "Sent Items".into());
                let flags = quatrelle_model::Flags { seen: true, ..Default::default() };
                return crate::pop::file_locally(&self.store, &self.acc.id, &folder, raw, flags, now()).map(|_| Some(folder));
            }
            let Some(folder) = self.sent_folder(sent_folder.clone())? else { return Ok(None) };
            self.client()?.append(&folder, "\\Seen", raw)?;
            Ok(Some(folder))
        })();
        match r {
            Ok(Some(folder)) => {
                log::info!("{}: message {id}'s sent copy saved in {folder}", self.acc.id);
                self.sync_one(&folder);
            }
            Ok(None) => self.notice(NO_SENT.to_string()),
            Err(e) => self.notice(format!("sent, but no copy was saved in Sent Items: {e}")),
        }
    }
}

/// Someone else's calendar, read, and told.
fn read_calendar(c: &crate::ews::Client, pim: &quatrelle_model::pim::Pim, id: &str, sink: &Sink, account: String) {
    match crate::ews_pim::sync_calendars(c, pim, &[id.to_string()], now()) {
        Ok(sync) => sink.emit(Event::DavSynced { account, sync: Box::new(sync) }),
        Err(e) => log::info!("{account}: calendars: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A message waiting to go is sent before the questions that came before it (MailTips asked
    /// as recipients are written never hold a send back); the others keep their order.
    #[test]
    fn a_send_goes_before_the_questions_waiting() {
        let tips = |ask| Job::GetMailTips { account: "a1".into(), from: String::new(), recipients: vec!["ana@contoso.example".into()], ask };
        let send = Job::Send { account: "a1".into(), id: 3, raw: Vec::new(), from: String::new(), recipients: Vec::new(), sent_folder: None };
        let mut waiting: VecDeque<Job> = [tips(1), send, tips(2)].into_iter().collect();
        assert!(matches!(next_quick(&mut waiting), Some(Job::Send { id: 3, .. })), "the send first");
        assert!(matches!(next_quick(&mut waiting), Some(Job::GetMailTips { ask: 1, .. })));
        assert!(matches!(next_quick(&mut waiting), Some(Job::GetMailTips { ask: 2, .. })));
        assert!(next_quick(&mut waiting).is_none());
    }

    /// A message to send.
    const RAW: &[u8] = b"From: me@studio.example\r\nTo: zoe@studio.example\r\nSubject: once\r\n\r\nOnly once.\r\n";

    /// A worker of an IMAP account sending through the scripted server at `port`, its events in
    /// `pool`.
    fn sender(port: u16, store: &Store, pool: &Pool) -> Worker {
        let acc = AccountConfig {
            id: "a1".into(),
            email: "me@studio.example".into(),
            username: "me@studio.example".into(),
            smtp: quatrelle_model::ServerConfig { host: "127.0.0.1".into(), port, security: quatrelle_model::Security::None },
            ..Default::default()
        };
        Worker::new(acc, Auth::shared("pw"), store.clone(), pool.sink.clone(), SyncOptions::default())
    }

    fn send_job(id: u64) -> Job {
        Job::Send {
            account: "a1".into(),
            id,
            raw: RAW.to_vec(),
            from: "me@studio.example".into(),
            recipients: vec!["zoe@studio.example".into()],
            sent_folder: None,
        }
    }

    /// Whether `seen` says message `id` may have gone, for the engine to say it so.
    fn said_maybe_gone(seen: &[Event], id: u64) -> bool {
        seen.iter()
            .any(|e| matches!(e, Event::SendFailed { id: i, error, unconfirmed: true, .. } if *i == id && error.contains("didn't say whether")))
    }

    /// The second connection sends; the server takes the whole message and drops the connection
    /// without a word. Taken for "offline", the message was handed to the main connection, which
    /// sent it again: the recipient had it twice. It's said, and goes nowhere by itself.
    #[test]
    fn the_second_connection_never_hands_an_unconfirmed_send_on() {
        let (port, heard) = crate::smtp::tests::scripted_heard(crate::smtp::tests::cut_after_the_message);
        let store = Store::memory();
        let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
        let (mut quick, mut main) = (sender(port, &store, &pool), sender(port, &store, &pool));
        let (to_main, handed) = mpsc::channel();
        quick.main = Some(to_main);
        quick.send_quick(send_job(5));
        // The main connection's part: what was handed to it, sent as it sends.
        for job in handed.try_iter() {
            main.handle(job);
        }
        assert_eq!(heard.messages(), 1, "the message went twice");
        assert!(said_maybe_gone(&pool.poll(), 5));
        assert!(store.outbox("a1").is_empty(), "{:?}", store.outbox("a1"));
    }

    /// The main connection sends with no answer after the whole message: said, and not kept in
    /// the Outbox, which would send it again.
    #[test]
    fn a_send_that_may_have_gone_isnt_kept_in_the_outbox() {
        let (port, heard) = crate::smtp::tests::scripted_heard(crate::smtp::tests::cut_after_the_message);
        let store = Store::memory();
        let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
        let mut w = sender(port, &store, &pool);
        w.send(6, RAW, "me@studio.example", &["zoe@studio.example".to_string()], None);
        assert!(store.outbox("a1").is_empty(), "{:?}", store.outbox("a1"));
        assert!(said_maybe_gone(&pool.poll(), 6));
        assert_eq!(heard.messages(), 1);
    }

    /// One from the Outbox with no answer after the whole of it: marked as maybe gone (never as
    /// refused), said, and not sent again by the next round. One the server asks to wait for still
    /// goes again.
    #[test]
    fn an_outbox_message_that_may_have_gone_isnt_sent_again() {
        let (port, heard) = crate::smtp::tests::scripted_heard(crate::smtp::tests::cut_after_the_message);
        let store = Store::memory();
        let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
        let mut w = sender(port, &store, &pool);
        let item = OutboxItem { id: 7, from: "me@studio.example".into(), recipients: vec!["zoe@studio.example".into()], ..Default::default() };
        store.outbox_add("a1", &item, RAW).unwrap();
        w.flush_outbox();
        w.flush_outbox();
        assert_eq!(heard.messages(), 1, "sent again");
        let kept = store.outbox("a1");
        let marked = kept.first().and_then(|i| i.error.clone()).unwrap_or_default();
        assert!(marked.starts_with("unconfirmed: the server didn't say whether"), "{kept:?}");
        let seen = pool.poll();
        assert!(said_maybe_gone(&seen, 7), "{seen:?}");
        assert!(!seen.iter().any(|e| matches!(e, Event::Queued { id: 7, .. })), "{seen:?}");
        // Asked to wait: kept for the next round, as ever.
        let (port, heard) = crate::smtp::tests::scripted_heard(|command| match command.split(' ').next().unwrap_or_default() {
            "EHLO" => "250-scripted.example\r\n250 AUTH PLAIN\r\n",
            "AUTH" => "235 2.7.0 welcome\r\n",
            "MAIL" => "451 4.3.0 try again in a moment\r\n",
            "QUIT" => "221 2.0.0 closing\r\n",
            _ => "250 2.0.0 fine\r\n",
        });
        let store = Store::memory();
        let mut w = sender(port, &store, &pool);
        store.outbox_add("a1", &OutboxItem { id: 8, ..item }, RAW).unwrap();
        w.flush_outbox();
        w.flush_outbox();
        assert_eq!(heard.said().iter().filter(|l| l.starts_with("MAIL FROM")).count(), 2, "not tried again");
        assert!(store.outbox("a1").first().and_then(|i| i.error.as_deref()).is_some_and(|e| e.contains("try again later")));
    }
}
