// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! An account's worker: the jobs it's handed, mailbox settings, and changes made offline replayed.

use super::*;

mod archive;
mod filing;
mod recovering;
mod rules;

/// How many times in all an IMAP change is tried when the server keeps refusing it for now
/// (Gmail's "System Error", RFC 5530's `UNAVAILABLE`): the third refusal drops it, as a final one.
const LATER_TRIES: u8 = 3;
/// How long before a change refused for now is tried again.
const LATER_EVERY: Duration = if cfg!(test) { Duration::from_millis(100) } else { Duration::from_secs(30) };

/// The IMAP change at the head of the journal that the server refused for now: how many times in
/// all, and when to try it again. In memory only: a restart counts afresh.
pub(super) struct Waiting {
    op: Op,
    tries: u8,
    pub(super) not_before: Instant,
}

pub(super) use archive::archive_here;
pub(super) use filing::{file_here, file_name};

impl Worker {
    pub(super) fn handle(&mut self, job: Job) {
        match job {
            Job::SyncAll { .. } => {
                if self.conn.is_none() {
                    self.auth_failed = false;
                    self.token_retried = false;
                    self.next_attempt = Instant::now();
                    self.connect_and_sync();
                }
                self.last_full = None;
            }
            Job::SyncFolder { folder, .. } => {
                self.read_followed();
                if self.conn.is_none() {
                    self.next_attempt = Instant::now();
                    self.connect_and_sync();
                }
                if self.conn.is_some() {
                    self.sync_one(&folder);
                }
            }
            Job::MoreMail { folder, more, .. } => {
                let kept = self.store.load_folder(&self.acc.id, &folder).map(|f| f.messages.len()).unwrap_or(0);
                self.read_followed();
                let now = self.options_for(&folder).window.max(kept);
                self.windows.insert(folder.clone(), now.saturating_add(more as usize).min(1_000_000));
                if let Err(e) = self.store.save_account_doc(&self.acc.id, WINDOWS, &self.windows) {
                    log::warn!("{}: {WINDOWS}: {e}", self.acc.id);
                }
                if self.conn.is_none() {
                    self.next_attempt = Instant::now();
                    self.connect_and_sync();
                }
                if self.conn.is_some() {
                    self.sync_one(&folder);
                }
            }
            Job::SearchServer { folders, query, search, .. } => self.search_server(&folders, query, search),
            Job::FetchBody { key } => self.fetch(key),
            Job::FetchTextFirst { key } => self.fetch_text_first(key),
            Job::Older { folder, offset, .. } => self.older(&folder, offset),
            Job::ReadOnline { key } => self.read_online_body(key),
            Job::Search { query, folder, .. } => self.search(query, folder),
            // Already in the cache and the journal (the pool wrote them): push the queue.
            Job::Apply { op, .. } if self.acc.pop.is_none() => {
                if op.is_folder_op() {
                    self.folders = self.store.load_folders(&self.acc.id).unwrap_or_default();
                }
                if self.conn.is_some() {
                    self.replay();
                }
            }
            Job::Vote { key, choice } => self.vote(key, choice),
            Job::ReadArchiveTag { key } => self.read_archive_tag(key),
            Job::Apply { op, .. } => {
                // POP: a move is carried out here (there's no server copy to come back); what's
                // deleted for good is noted, for removing the server's copy when the account says.
                {
                    let gone: Vec<quatrelle_model::MessageKey> = match &op {
                        Op::Expunge { keys, .. } => keys.clone(),
                        Op::EmptyFolder { path } => {
                            self.store.load_folder(&self.acc.id, path).map(|f| f.messages.into_iter().map(|m| m.key).collect()).unwrap_or_default()
                        }
                        _ => Vec::new(),
                    };
                    if let Err(e) = crate::pop::deleted_for_good(&self.store, &self.acc.id, &gone) {
                        self.notice(e.to_string());
                    }
                }
                let applied = match &op {
                    Op::Move { keys, to } if self.acc.pop.is_some() => crate::pop::move_locally(&self.store, &self.acc.id, keys, to),
                    _ => self.store.apply_op(&self.acc.id, &op).map_err(NetError::from),
                };
                match applied {
                    Ok(changed) => {
                        if op.is_folder_op() {
                            self.folders = self.store.load_folders(&self.acc.id).unwrap_or_default();
                            self.publish_folders();
                        }
                        for folder in changed {
                            self.sink.emit(Event::FolderChanged { account: self.id(), folder });
                        }
                    }
                    Err(e) => self.notice(e.to_string()),
                }
                if let Err(e) = self.store.add_op(&self.acc.id, op) {
                    self.notice(e.to_string());
                }
                if self.conn.is_some() {
                    self.replay();
                }
            }
            Job::Send { id, raw, from, recipients, sent_folder, .. } => self.send(id, &raw, &from, &recipients, sent_folder),
            // The pool keeps these itself; one reaching a worker is kept the same way.
            Job::KeepDraft { raw, folder, replaces, .. } => {
                if let Err(e) = keep_draft(&self.store, &self.acc.id, &folder, replaces, raw) {
                    log::warn!("{}: the draft couldn't be kept: {e}", self.acc.id);
                }
            }
            Job::SaveDraft { raw, folder, replaces, .. } => {
                let (raw_kept, replaces_kept) = (raw.clone(), replaces.clone());
                let mut filed = folder.clone();
                let r = (|| -> Result<()> {
                    if self.conn.is_none() {
                        self.connect()?;
                    }
                    if let Some(Conn::Pop) = self.conn.as_ref() {
                        let flags = quatrelle_model::Flags { draft: true, seen: true, ..Default::default() };
                        crate::pop::file_locally(&self.store, &self.acc.id, &folder, &raw, flags, now())?;
                        if let Some(old) = replaces {
                            self.store.apply_op(&self.acc.id, &Op::Expunge { keys: vec![old], tidy: true })?;
                        }
                        return Ok(());
                    }
                    if let Some(Conn::Jmap(c)) = self.conn.as_mut() {
                        jmap::append(c, &self.jstate, &folder, &raw, &["$draft", "$seen"])?;
                        if let Some(old) = replaces {
                            jmap::apply_remote(c, &self.jstate, &Op::Expunge { keys: vec![old], tidy: true })?;
                        }
                        return Ok(());
                    }
                    if let Some(Conn::Graph(c)) = self.conn.as_mut() {
                        crate::graph::append(c, &self.jstate, &folder, &raw)?;
                        if let Some(old) = replaces {
                            crate::graph::apply_remote(c, &self.jstate, &Op::Expunge { keys: vec![old], tidy: true })?;
                        }
                        return Ok(());
                    }
                    if let Some(Conn::Ews(c)) = self.conn.as_mut() {
                        crate::ews::append(c, &self.jstate, &folder, &raw)?;
                        if let Some(old) = replaces {
                            crate::ews::apply_remote(c, &self.jstate, &Op::Expunge { keys: vec![old], tidy: true })?;
                        }
                        return Ok(());
                    }
                    // A server without Drafts gets one, as it gets Sent Items (a draft was refused,
                    // and lost as Quatrelle closed): once its folders are known (those listed
                    // before, when this connection hasn't listed them yet).
                    if self.folders.is_empty()
                        && let Ok(listed) = self.store.load_folders(&self.acc.id)
                    {
                        self.folders = listed;
                    }
                    let missing = !self.folders.is_empty() && !self.folders.iter().any(|f| f.path == folder);
                    if missing && FolderRole::from_name(&folder, None) == FolderRole::Drafts {
                        filed = self.role_folder(FolderRole::Drafts, "Drafts")?.unwrap_or_else(|| folder.clone());
                    }
                    let trash = self.folders.iter().find(|f| f.role == FolderRole::Trash).map(|f| f.path.clone());
                    let c = self.client()?;
                    c.append(&filed, "\\Draft \\Seen", &raw)?;
                    if let Some(old) = replaces {
                        expunge_keys(c, &[old], trash.as_deref())?;
                    }
                    Ok(())
                })();
                match r {
                    Ok(()) => {
                        forget_draft(&self.store, &self.acc.id, &raw_kept);
                        self.sync_one(&filed)
                    }
                    Err(e) => {
                        // Not lost: kept on this computer and saved when the account connects.
                        match keep_draft(&self.store, &self.acc.id, &folder, replaces_kept, raw_kept.clone()) {
                            Ok(()) => self.notice(
                                "the draft couldn't be saved to the server; it's kept on this computer and saved there when the account connects",
                            ),
                            // Neither the server nor the disk: back to the screen, not lost.
                            Err(k) => self.sink.emit(Event::DraftReturned {
                                account: self.id(),
                                raw: raw_kept,
                                error: format!("{e}; and this computer's disk refused its copy: {k}"),
                            }),
                        }
                        self.failed(e);
                    }
                }
            }
            Job::FileMessages { folder, paths, .. } => self.file_messages(folder, paths),
            Job::FlushOutbox { .. } => self.flush_outbox(),
            Job::SyncDav { .. } => self.sync_dav(),
            Job::SyncCalendars { calendars, .. } => self.sync_calendars(&calendars),
            Job::GetVacation { .. } => {
                let r = self.vacation(None);
                self.report_vacation(r, None);
            }
            Job::SetVacation { vacation, .. } => {
                let r = self.vacation(Some(&vacation));
                match r {
                    Ok(v) => self.report_vacation(Ok(v), None),
                    // Show what the server still has, with why the change failed.
                    Err(e) => {
                        let back = self.vacation(None);
                        self.report_vacation(back, Some(e.to_string()));
                    }
                }
            }
            Job::SetServerRules { rules, delete, categories, .. } => self.set_server_rules(rules, delete, categories),
            Job::GetServerRules { ours, .. } => match self.read_server_rules(ours) {
                Ok((ids, others, cleared)) => {
                    log::info!("{}: the server keeps {} rules ({} of ours, {} others)", self.id(), ids.len() + others.len(), ids.len(), others.len());
                    self.sink.emit(Event::ServerRules { account: self.id(), ids, others, cleared, error: None });
                }
                // Read by itself on connecting: one that fails (the server busy, a connection
                // dropped) is only logged — the list kept is still right, and a red "the server's
                // rules weren't changed" for a read nobody asked for was one more false alarm.
                Err(e) => {
                    log::warn!("{}: the server's rules couldn't be read: {e}", self.id());
                    if e.is_fatal() || matches!(e, NetError::Auth(_)) {
                        self.failed(e);
                    }
                }
            },
            Job::SetServerRuleEnabled { id, enabled, ours, .. } => self.change_server_rule(&id, Some(enabled), false, ours),
            Job::SetServerRule { id, rule, ours, .. } => self.edit_server_rule(&id, &rule, ours),
            Job::DeleteServerRule { id, ours, .. } => self.change_server_rule(&id, None, true, ours),
            // The changes queued after it run on this connection with Outlook's copy removed.
            Job::ReplaceOutlookRules { .. } => {
                if self.conn.is_none()
                    && let Err(e) = self.connect()
                {
                    log::warn!("{}: Outlook's copy of the rules: {e}", self.id());
                }
                if let Some(Conn::Ews(c)) = self.conn.as_ref() {
                    c.replace_outlook_rules();
                }
            }
            Job::FetchPhoto { email, contact, .. } => self.fetch_photo(&email, contact.as_deref()),
            Job::FetchEventFile { reference, ask, .. } => self.fetch_event_file(&reference, ask),
            Job::GetRooms { .. } => self.rooms(),
            Job::GetAvailability { emails, from, to, .. } => self.availability(&emails, from, to),
            Job::ServerFolderSizes { .. } => self.folder_sizes(),
            Job::ListRecoverable { ask, offset, .. } => self.list_recoverable(ask, offset),
            Job::RestoreRecoverable { ask, ids, purge, .. } => self.restore_recoverable(ask, ids, purge),
            Job::FindCertificates { emails, .. } => self.directory_certificates(&emails),
            Job::SearchDirectory { query, .. } => self.search_directory(&query),
            Job::ExpandList { address, item_id, .. } => self.expand_list(address, item_id),
            Job::FetchUrl { .. }
            | Job::FetchCalendar { .. }
            | Job::Discover { .. }
            | Job::SignIn { .. }
            | Job::ImportPst { .. }
            | Job::StopImport { .. }
            | Job::ResumeImport { .. }
            | Job::ReadMessageFile { .. }
            | Job::CompactDataFile { .. }
            | Job::AskLlm { .. }
            | Job::ListLlmModels { .. }
            | Job::SetLlmKey { .. }
            | Job::WebexSignIn { .. }
            | Job::WebexSignOut
            | Job::SetWebexSecret { .. }
            | Job::WebexCreate { .. }
            | Job::WebexUpdate { .. }
            | Job::WebexDelete { .. }
            | Job::Teams { .. }
            | Job::LocalFolderSizes { .. }
            | Job::ExportPst { .. }
            | Job::ImportIdentity { .. }
            | Job::ForgetIdentity { .. }
            | Job::ForgetOlderIdentity { .. }
            | Job::UseOlderIdentity { .. }
            | Job::Protect { .. }
            | Job::Decrypt { .. }
            | Job::SealDraft { .. }
            | Job::CheckRevocation { .. }
            | Job::FindHeldKeys { .. }
            | Job::UseHeldKey { .. }
            | Job::Pgp { .. }
            | Job::CheckUpdates
            | Job::StopSearch => {}
            Job::Archive { keys, to_account, .. } => self.archive(keys, to_account),
            Job::PutBack { keys, to_folder, .. } => self.put_back(keys, to_folder),
            Job::Start { .. } | Job::SmtpPassword { .. } | Job::Remove { .. } | Job::WorkOffline { .. } | Job::SetSync { .. } => {}
        }
    }

    /// The picture kept for a person: the one their contact item carries (`contact`), or else
    /// the one the directory keeps for their address. None is an answer too (the person simply
    /// has no picture, or the server doesn't keep them).
    pub(super) fn fetch_photo(&mut self, email: &str, contact: Option<&str>) {
        let r = (|| -> Result<(String, Vec<u8>)> {
            if self.conn.is_none() {
                self.connect()?;
            }
            if let Some(id) = contact {
                let own = match self.conn.as_mut() {
                    Some(Conn::Ews(c)) => c.contact_photo(id),
                    Some(Conn::Graph(c)) => c.contact_photo(id),
                    _ => Err(NetError::No("this server keeps no contact pictures".into())),
                };
                // A contact without a picture of its own may still have one in the directory.
                match own {
                    Ok(p) => return Ok(p),
                    Err(NetError::No(_)) => {}
                    Err(e) => return Err(e),
                }
            }
            match self.conn.as_mut() {
                Some(Conn::Ews(c)) => c.user_photo(email),
                Some(Conn::Graph(c)) => c.user_photo(email),
                _ => Err(NetError::No("this server keeps no pictures".into())),
            }
        })();
        let (media, data, error) = match r {
            Ok((media, data)) => (media, data, None),
            Err(e) => {
                let said = e.to_string();
                // The connection lost or the sign-in refused is the account's, as when a message
                // is opened: left unsaid, "Connecting…" stayed up for good.
                if e.is_fatal() || matches!(e, NetError::Auth(_)) {
                    self.failed(e);
                }
                (String::new(), Vec::new(), Some(said))
            }
        };
        // Said in the log either way: "why is there no picture?" is answered without a debugger.
        match &error {
            Some(e) => log::info!("picture of {email}: none ({e})"),
            None => log::info!("picture of {email}: {} bytes, {media}", data.len()),
        }
        self.sink.emit(Event::Photo { email: email.to_string(), media, data, error });
    }

    /// Read (and first change, when `set`) the automatic replies: JMAP's own, or a Sieve script
    /// through ManageSieve on the IMAP host.
    pub(super) fn vacation(&mut self, set: Option<&quatrelle_model::job::Vacation>) -> Result<quatrelle_model::job::Vacation> {
        // Not connected: one try, and a failure shows as the account's status (it said
        // "Connecting…" for good when the vacation page was opened on an account that can't).
        // (The second connection asks ManageSieve straight away: it needs no IMAP session.)
        if self.conn.is_none() && (self.main.is_none() || self.acc.is_api()) {
            if self.auth_failed {
                return Err(NetError::Refused("it shows once the account is connected".into()));
            }
            if let Err(e) = self.connect() {
                if self.main.is_some() {
                    return Err(e);
                }
                self.connect_failed(e);
                return Err(NetError::Refused("it shows once the account is connected".into()));
            }
        }
        if let Some(Conn::Pop) = &self.conn {
            return Err(NetError::No("POP accounts have no automatic replies on the server".into()));
        }
        if let Some(Conn::Jmap(c)) = &self.conn {
            if let Some(v) = set {
                c.set_vacation(v)?;
            }
            return c.vacation();
        }
        if let Some(Conn::Graph(c)) = &self.conn {
            if let Some(v) = set {
                c.set_vacation(v)?;
            }
            return c.vacation();
        }
        if let Some(Conn::Ews(c)) = &self.conn {
            if let Some(v) = set {
                c.set_vacation(v)?;
            }
            return c.vacation();
        }
        let secret = self.secret()?;
        let user = if self.acc.username.is_empty() { self.acc.email.clone() } else { self.acc.username.clone() };
        let mut s = crate::sieve::Client::connect(&self.acc.imap, self.sieve_port).map_err(|e| match e {
            NetError::Io(_) => NetError::No("this server doesn't offer automatic replies (no ManageSieve)".into()),
            e => e,
        })?;
        if self.acc.oauth.is_some() {
            s.login_xoauth2(&user, &secret)?;
        } else {
            s.login(&user, &secret)?;
        }
        if let Some(v) = set {
            s.set_vacation(v)?;
        }
        let v = s.vacation();
        s.logout();
        v
    }

    pub(super) fn report_vacation(&self, r: Result<quatrelle_model::job::Vacation>, failed: Option<String>) {
        let (vacation, error) = match r {
            Ok(v) => (Some(v), failed),
            Err(e) => (None, Some(failed.unwrap_or_else(|| e.to_string()))),
        };
        self.sink.emit(Event::Vacation { account: self.id(), vacation, error });
    }

    /// The edits still waiting that touch `path`, on its freshly saved cache; `true` when there are
    /// any (the list shows them). A sync's save made them again on the list already
    /// (`Store::save_folder`, one pass), all but emptying the folder, made here. Each was made
    /// again here too, the list read and written whole for each move: five minutes after a
    /// burst of 1,000 changes on a folder of 50,000.
    pub(super) fn reapply_waiting(&mut self, path: &str) -> bool {
        let mut any = false;
        for op in self.store.load_ops(&self.acc.id).unwrap_or_default() {
            if !touches(&op, path) {
                continue;
            }
            any = true;
            if matches!(op, Op::EmptyFolder { .. })
                && let Err(e) = self.store.apply_op(&self.acc.id, &op)
            {
                log::warn!("{}: a folder still to empty wasn't: {e}", self.acc.id);
            }
        }
        any
    }

    /// Whether an edit still waiting to go out touches the folder `path`.
    pub(super) fn waiting_in(&self, path: &str) -> bool {
        self.store.load_ops(&self.acc.id).unwrap_or_default().iter().any(|op| touches(op, path))
    }

    /// Whether a change refused for now is due to be tried again.
    pub(super) fn retry_due(&self) -> bool {
        self.conn.is_some() && self.waiting.as_ref().is_some_and(|w| w.not_before <= Instant::now())
    }

    /// The server refused `op` for now. On IMAP it is tried again later (the refusal is returned
    /// as it came); the third time it is the server's final word.
    fn refused_for_now(&mut self, op: &Op, e: NetError) -> (Result<()>, bool) {
        let NetError::Later(said) = e else { return (Err(e), false) };
        if !matches!(self.conn, Some(Conn::Imap(_))) {
            return (Err(NetError::Later(said)), false);
        }
        let tries = self.waiting.as_ref().filter(|w| w.op == *op).map_or(0, |w| w.tries).saturating_add(1);
        if tries >= LATER_TRIES {
            self.waiting = None;
            return (Err(NetError::No(said)), true);
        }
        self.waiting = Some(Waiting { op: op.clone(), tries, not_before: Instant::now() + LATER_EVERY });
        (Err(NetError::Later(said)), false)
    }

    /// Replay offline changes on the server, oldest first.
    pub(super) fn replay(&mut self) {
        let mut destinations: Vec<String> = Vec::new();
        let mut relist = false;
        let trash = self.folders.iter().find(|f| f.role == FolderRole::Trash).map(|f| f.path.clone());
        // A change refused once is tried again after the folders are listed anew (see below).
        let mut retried = false;
        // The queue is the journal's: bounded, in case taking an entry out keeps failing.
        for _ in 0..100_000 {
            let Some(op) = self.store.first_op(&self.acc.id).unwrap_or_default() else {
                self.waiting = None;
                break;
            };
            // The change refused for now waits for its time; the ones after it wait with it.
            match &self.waiting {
                Some(w) if w.op == op && w.not_before > Instant::now() => break,
                Some(w) if w.op != op => self.waiting = None,
                _ => {}
            }
            let r = match self.conn.as_mut() {
                Some(Conn::Imap(c)) => apply_remote_with(c, &op, trash.as_deref()),
                Some(Conn::Jmap(c)) => jmap::apply_remote(c, &self.jstate, &op),
                Some(Conn::Graph(c)) => crate::graph::apply_remote(c, &self.jstate, &op),
                // Messages the numbering lost are looked for first: left out, the change was
                // made here only.
                Some(Conn::Ews(c)) => {
                    crate::ews::find_lost(c, &mut self.jstate, &self.store, op.keys()).and_then(|()| crate::ews::apply_remote(c, &self.jstate, &op))
                }
                // POP: changes are made here only (the cache has them already).
                Some(Conn::Pop) => Ok(()),
                None => Err(NetError::Closed("not connected".into())),
            };
            let (r, gave_up) = match r {
                Err(e) => self.refused_for_now(&op, e),
                ok => (ok, false),
            };
            // A change given up on isn't tried once more after the folders are listed anew.
            retried |= gave_up;
            match r {
                Ok(()) => {
                    self.waiting = None;
                    retried = false;
                    match &op {
                        Op::Move { to, .. } => destinations.push(to.clone()),
                        Op::RenameFolder { to, .. } => {
                            relist = true;
                            destinations.push(to.clone());
                        }
                        Op::CreateFolder { .. } | Op::DeleteFolder { .. } => relist = true,
                        Op::EmptyFolder { path } => destinations.push(path.clone()),
                        Op::SetFlag { .. } | Op::SetKeyword { .. } | Op::Expunge { .. } | Op::SetArchiveTag { .. } => {}
                    }
                    if let Err(e) = self.store.remove_op(&self.acc.id, &op) {
                        self.notice(e.to_string());
                        break;
                    }
                    // The next edit may name this folder (or its new name): refresh the ids now
                    // rather than after the pass.
                    if op.is_folder_op() && relist {
                        relist = !self.relist();
                    }
                }
                Err(e) if e.is_fatal() => {
                    self.failed(e);
                    return;
                }
                // Busy: the change stays queued for the next pass.
                Err(e @ NetError::Later(_)) => {
                    log::warn!("{}: a change waits ({}): {e}", self.acc.id, op.summary());
                    // On IMAP it is tried again soon; the folders it moved to are still read.
                    if self.waiting.is_some() {
                        break;
                    }
                    return;
                }
                // A folder the change names may be one the last listing missed (one just renamed
                // or made, after a listing that failed): listed again, the change is tried once
                // more before it counts as refused, rather than dropped for want of its id.
                Err(_) if !retried && op_names_folder(&op) && self.relist() => retried = true,
                Err(e) => {
                    retried = false;
                    self.waiting = None;
                    // The server refused: drop the change and resync so the cache matches the
                    // server again (forcing a full flag check).
                    if let Err(e) = self.store.remove_op(&self.acc.id, &op) {
                        self.notice(e.to_string());
                        break;
                    }
                    let mut folders: Vec<String> = op.keys().iter().map(|k| k.folder.clone()).collect();
                    folders.dedup();
                    for f in &folders {
                        if let Ok(mut st) = self.store.load_folder(&self.acc.id, f) {
                            st.highest_modseq = 0;
                            st.uid_next = 0;
                            let _ = self.store.save_folder(&self.acc.id, f, &st);
                        }
                    }
                    relist |= op.is_folder_op();
                    self.sink.emit(Event::OpFailed { account: self.id(), op, error: e.to_string() });
                    destinations.extend(folders);
                }
            }
        }
        if relist {
            self.relist();
        }
        destinations.sort();
        destinations.dedup();
        // Only the folders the server has: one renamed again after (the folder in between,
        // gone, said "no such mailbox").
        for d in destinations {
            if self.conn.is_some() && self.folders.iter().any(|f| f.path == d) {
                self.sync_one(&d);
            }
        }
    }
}

/// Whether a change names a folder by its path (one a listing may not know yet).
fn op_names_folder(op: &Op) -> bool {
    op.is_folder_op() || matches!(op, Op::Move { .. } | Op::EmptyFolder { .. })
}

/// Whether `op` changes something in the folder `path`.
fn touches(op: &Op, path: &str) -> bool {
    match op {
        Op::EmptyFolder { path: p } => p == path,
        o if o.is_folder_op() => false,
        o => o.keys().iter().any(|k| k.folder == path),
    }
}
