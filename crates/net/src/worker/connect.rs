// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! An account's worker: signing in and connecting (IMAP, JMAP, Graph, EWS, POP), and what a failure means.

use super::*;
use quatrelle_model::EWS_FOUND_DOC;
use quatrelle_model::addr::trust::withdrawn;

/// An Exchange account's EWS address: the one set, or for `autodiscover` the one found the first
/// time, then kept ([`EWS_FOUND_DOC`]). A kept address is used as it is, asking nothing: it is in
/// service, and the credentials have gone there all along (an on-premises server is often on
/// another domain than the user's address). The one exception is a site the user withdrew: its address
/// counts for nothing, even saved after the removal by a connection already on its way, so
/// Autodiscover runs again, and it asks before it follows an answer to a site the account doesn't
/// trust (`Untrusted`).
pub(super) fn ews_url(acc: &AccountConfig, store: &Store, user: &str, secret: &str) -> Result<String> {
    let Some(url) = &acc.ews else { return Err(NetError::No("not an Exchange account".into())) };
    if url != crate::ews::AUTODISCOVER {
        return Ok(url.clone());
    }
    let kept: String = store.load_account_doc(&acc.id, EWS_FOUND_DOC).unwrap_or_default();
    if !kept.is_empty() && !withdrawn(acc, &kept) {
        return Ok(kept);
    }
    let found = crate::ews::autodiscover_as(&acc.email, user, secret, None, acc.user_agent.as_deref(), &Trust::of(acc, &[]))?;
    let _ = store.save_account_doc(&acc.id, EWS_FOUND_DOC, &found);
    Ok(found)
}

/// A wait for new mail that loses its connection after this long lost it on the way (a gateway
/// that changed over or forgot it), not to a server that won't wait.
#[cfg(not(test))]
const LOST_AFTER: Duration = Duration::from_secs(60);
#[cfg(test)]
const LOST_AFTER: Duration = Duration::from_millis(200);

impl Worker {
    /// The wait for new mail failed. A connection lost after a while of waiting (the keepalive, or
    /// IDLE's end, found it dead: the office's gateway changed over, 02/11) is made again at once,
    /// saying nothing: the account showed an error each time, with nothing for the user to do.
    /// Connecting again that fails is said as ever, and so is a connection lost straight away (a
    /// server that won't wait would be connected to over and over).
    pub(super) fn failed_waiting(&mut self, e: NetError, after: Duration) {
        if after < LOST_AFTER || !matches!(e, NetError::Io(_) | NetError::Closed(_)) {
            self.failed(e);
            return;
        }
        log::info!("{}: the connection waiting for new mail was lost after {} s ({e}); connecting again", self.acc.id, after.as_secs());
        self.conn = None;
        self.next_attempt = Instant::now();
    }

    /// What to sign in with: the password, or for OAuth accounts a current access token
    /// (refreshed from the stored refresh token when needed).
    pub(super) fn secret(&mut self) -> Result<String> {
        // Held while refreshing: the account's other connection waits for the new token rather
        // than refreshing with the same old one.
        let mut auth = self.auth.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(id) = self.acc.oauth.clone() else { return Ok(auth.secret.clone()) };
        if let Some(t) = &auth.access
            && !t.expired()
        {
            return Ok(t.access.clone());
        }
        let p = crate::oauth::provider_for_account(&self.acc)
            .ok_or_else(|| NetError::Auth(format!("signing in with {id} isn't set up in this copy of Quatrelle")))?;
        let t = crate::oauth::refresh(&p, &auth.secret).inspect_err(|e| {
            // The refresh token itself refused: only signing in again helps.
            if matches!(e, NetError::Auth(_)) {
                self.token_retried = true;
            }
        })?;
        // Some providers hand out a new refresh token each time: keep the newest.
        if let Some(r) = t.refresh.clone().filter(|r| *r != auth.secret) {
            auth.secret = r.clone();
            if let Some(c) = &self.creds
                && let Err(e) = c.set(&self.acc, &r)
            {
                self.sink
                    .emit(Event::Notice { account: self.acc.id.clone(), message: format!("the new sign-in couldn't be saved in the keychain: {e}") });
            }
        }
        let access = t.access.clone();
        auth.access = Some(t);
        Ok(access)
    }

    /// Before each job: an OAuth account's Graph or JMAP connection carries the current access
    /// token, refreshed when it ran out by the wall clock (or by the account's other connection).
    pub(super) fn fresh_token(&mut self) {
        if self.acc.oauth.is_none() || !matches!(self.conn, Some(Conn::Graph(_) | Conn::Jmap(_))) {
            return;
        }
        match self.secret() {
            Ok(t) => self.use_token(&t),
            Err(e) => self.failed(e),
        }
    }

    /// Drop the access token the server refused and fetch a new one with the refresh token.
    pub(super) fn renew_token(&mut self) -> Result<String> {
        self.auth.lock().unwrap_or_else(std::sync::PoisonError::into_inner).access = None;
        let t = self.secret()?;
        self.use_token(&t);
        Ok(t)
    }

    fn use_token(&mut self, token: &str) {
        match self.conn.as_mut() {
            Some(Conn::Graph(c)) => c.set_token(token),
            Some(Conn::Jmap(c)) => c.set_bearer(token),
            _ => {}
        }
    }

    /// What Microsoft's sign-in granted (its scopes), when known.
    pub(super) fn granted(&self) -> String {
        self.auth.lock().unwrap_or_else(std::sync::PoisonError::into_inner).access.as_ref().map(|t| t.granted.clone()).unwrap_or_default()
    }

    /// Note a failure; drop the connection when it is unusable.
    pub(super) fn failed(&mut self, e: NetError) {
        log::warn!("{}: {e}", self.acc.id);
        match e {
            // Microsoft took the token but won't open the mailbox: nothing to retry until changed.
            NetError::Auth(m) | NetError::Bad(m) | NetError::No(m)
                if self.acc.oauth.as_deref() == Some("microsoft") && m.to_ascii_lowercase().contains("authenticated but not connected") =>
            {
                self.conn = None;
                self.auth_failed = true;
                self.status(SyncStatus::Error { error: microsoft_imap_closed(&self.acc.email, self.acc.oauth_user.as_deref()) });
            }
            // Microsoft's servers refuse passwords from mail apps: asking for it again only loops.
            NetError::Auth(m) if quatrelle_model::microsoft_password_account(&self.acc) => {
                self.conn = None;
                self.auth_failed = true;
                self.status(SyncStatus::Error { error: format!("{MICROSOFT_NO_PASSWORDS} ({m})") });
            }
            // An OAuth token refused (expired early, revoked by a password change elsewhere, or
            // taken before the computer slept): a fresh one from the refresh token, then the
            // connection again, once. Only a refused refresh, or a fresh token refused too,
            // means signing in again.
            NetError::Auth(m) if self.acc.oauth.is_some() && !self.token_retried => {
                log::info!("{}: token refused ({m}); refreshing it", self.acc.id);
                self.token_retried = true;
                match self.renew_token() {
                    Ok(_) => {
                        self.conn = None;
                        self.next_attempt = Instant::now();
                        self.status(SyncStatus::Connecting);
                    }
                    Err(e) => self.failed(e),
                }
            }
            NetError::Auth(m) => {
                self.conn = None;
                self.auth_failed = true;
                self.status(SyncStatus::NeedsPassword { error: m });
            }
            e if e.is_fatal() => {
                self.conn = None;
                self.next_attempt = Instant::now() + self.backoff;
                self.backoff = (self.backoff * 2).min(Duration::from_secs(300));
                self.status(SyncStatus::Error { error: e.to_string() });
            }
            // The server can't now (busy, a mailbox being moved): the next sync tries again;
            // the connection is fine and nothing is the user's to do.
            NetError::Later(_) => {}
            e => self.notice(e.to_string()),
        }
    }

    pub(super) fn connect(&mut self) -> Result<()> {
        if self.conn.is_some() {
            return Ok(());
        }
        self.status(SyncStatus::Connecting);
        let r = self.open_connection();
        // Connected for something asked (a message, a picture, the rules) rather than by the
        // sync: "Connecting…" said so far must not stay up — the sync, if it's due, says
        // "Syncing" next; otherwise the account is as current as before.
        match &r {
            Ok(()) => {
                self.auth_failed = false;
                if self.last_full.is_some() {
                    self.status(SyncStatus::Online { at: now() });
                }
            }
            // Said here too: not every caller hands the failure on (a picture, a search), and
            // the caller that does says it again, more precisely (a password to ask for).
            // Another server to allow is said as such whoever asked (a message to send).
            Err(NetError::Untrusted { site }) => self.status(SyncStatus::NeedsTrust { site: site.clone() }),
            Err(e) => self.status(SyncStatus::Error { error: e.to_string() }),
        }
        r
    }

    pub(super) fn open_connection(&mut self) -> Result<()> {
        let secret = self.secret()?;
        let user = if self.acc.username.is_empty() { self.acc.email.clone() } else { self.acc.username.clone() };
        if let Some(url) = &self.acc.graph {
            self.conn = Some(Conn::Graph(crate::graph::Client::connect(url, &secret)?));
            self.backoff = Duration::from_secs(5);
            return Ok(());
        }
        if self.acc.pop.is_some() {
            crate::pop::open(&self.acc, &secret)?.quit()?;
            self.conn = Some(Conn::Pop);
            self.backoff = Duration::from_secs(5);
            return Ok(());
        }
        if self.acc.ews.is_some() {
            let url = ews_url(&self.acc, &self.store, &user, &secret)?;
            let c = crate::ews::Client::connect_as(&url, &user, &secret, &self.acc.email, self.acc.user_agent.as_deref())?;
            c.set_shared(&self.acc.shared_mailboxes);
            self.conn = Some(Conn::Ews(Box::new(c)));
            self.backoff = Duration::from_secs(5);
            return Ok(());
        }
        if let Some(url) = &self.acc.jmap {
            self.conn = Some(Conn::Jmap(jmap::Client::connect(url, &user, &secret, self.acc.oauth.is_some(), &Trust::of(&self.acc, &[]).sites())?));
            self.backoff = Duration::from_secs(5);
            return Ok(());
        }
        // Where a slow start went (a Gmail account took 18 s before its first folder, 29/10).
        let began = Instant::now();
        let (c, connected) = if self.acc.oauth.is_some() {
            (self.imap_with_token(&user, &secret)?, None)
        } else {
            let mut c = Client::connect(&self.acc.imap)?;
            let connected = began.elapsed();
            c.login(&user, &secret)?;
            (c, Some(connected))
        };
        if began.elapsed() > crate::stream::SLOW_CONNECT {
            let of_it = connected.map(|d| format!(" ({:.1} s of it connecting)", d.as_secs_f32())).unwrap_or_default();
            log::info!("{}: signed in after {:.1} s{of_it}", self.acc.id, began.elapsed().as_secs_f32());
        }
        self.conn = Some(Conn::Imap(c));
        self.backoff = Duration::from_secs(5);
        Ok(())
    }

    /// An IMAP connection signed in with an OAuth token. Microsoft can take the token and then
    /// refuse the mailbox ("User is authenticated but not connected"): IMAP is switched off for
    /// it (Outlook.com's own setting, or an organisation's), or the browser signed in as another
    /// address. The address it signed in as is tried once; otherwise it's a refusal that says
    /// what to do, not a sign-in to ask for again (that only loops).
    pub(super) fn imap_with_token(&self, user: &str, token: &str) -> Result<Client<Stream>> {
        let mut c = Client::connect(&self.acc.imap)?;
        let m = match c.login_xoauth2(user, token) {
            Ok(()) => return Ok(c),
            Err(NetError::Auth(m) | NetError::Bad(m) | NetError::No(m))
                if self.acc.oauth.as_deref() == Some("microsoft") && m.to_ascii_lowercase().contains("not connected") =>
            {
                m
            }
            Err(e) => return Err(e),
        };
        log::warn!("{}: {m}", self.acc.id);
        let other = self.acc.oauth_user.as_deref().filter(|o| !o.eq_ignore_ascii_case(user));
        if let Some(other) = other {
            let mut c = Client::connect(&self.acc.imap)?;
            if c.login_xoauth2(other, token).is_ok() {
                return Ok(c);
            }
        }
        Err(NetError::Refused(microsoft_imap_closed(&self.acc.email, other)))
    }

    /// A connection that couldn't be made is never tried again at once (that looped, showing
    /// "Connecting…" for good): a refusal (Graph's "no mailbox", a setting the server won't take)
    /// waits until asked, anything else for the backoff.
    pub(super) fn connect_failed(&mut self, e: NetError) {
        match e {
            e @ NetError::Auth(_) => self.failed(e),
            // A server on a site the account doesn't trust: nothing went there. The user is
            // asked; the account waits for the answer (it starts again once allowed), with no
            // back-off: it isn't a failure to sign in, nor the network's.
            NetError::Untrusted { site } => {
                log::warn!("{}: {site} isn't allowed for this account: waiting for the answer", self.acc.id);
                self.conn = None;
                self.auth_failed = true;
                self.status(SyncStatus::NeedsTrust { site });
            }
            e if e.is_fatal() => self.failed(e),
            // Busy: tried again after the backoff, as when the network fails.
            e @ NetError::Later(_) => {
                log::warn!("{}: {e}", self.acc.id);
                self.conn = None;
                self.next_attempt = Instant::now() + self.backoff;
                self.backoff = (self.backoff * 2).min(Duration::from_secs(300));
                self.status(SyncStatus::Error { error: e.to_string() });
            }
            e => {
                log::warn!("{}: {e}", self.acc.id);
                self.conn = None;
                self.auth_failed = true;
                self.status(SyncStatus::Error { error: e.to_string() });
            }
        }
    }

    pub(super) fn connect_and_sync(&mut self) {
        self.listen.reset();
        if let Err(e) = self.connect() {
            self.connect_failed(e);
            return;
        }
        self.replay();
        self.flush_outbox();
        // Drafts kept while offline go to the server first. They stay on disk until each one is
        // filed ([`forget_draft`]): a worker given up or a connection lost before then loses none.
        let kept: Vec<PendingDraft> = self.store.load_account_doc(&self.acc.id, PENDING_DRAFTS).unwrap_or_default();
        for d in kept.into_iter().rev() {
            // Not queued twice by a connection made again before the queue was done.
            if self.queue.iter().any(|j| matches!(j, Job::SaveDraft { raw, .. } if *raw == d.raw)) {
                continue;
            }
            self.queue.push_front(Job::SaveDraft { account: self.acc.id.clone(), raw: d.raw, folder: d.folder, replaces: d.replaces });
        }
        self.last_full = None;
    }

    pub(super) fn client(&mut self) -> Result<&mut Client<Stream>> {
        match self.conn.as_mut() {
            Some(Conn::Imap(c)) => Ok(c),
            _ => Err(NetError::Closed("not connected".into())),
        }
    }

    /// Save the JMAP / Graph numbering after it changed.
    pub(super) fn save_jstate(&self) {
        if self.acc.is_api()
            && let Err(e) = self.jstate.save(&self.store, &self.acc.id)
        {
            log::warn!("{}: {e}", self.acc.id);
        }
    }
}

/// What a connection of its own sends back while it reads folders.
pub(super) enum SyncDone {
    /// A folder read, and what changed there (or why not, as it was: a connection that timed
    /// out must stop the sync, not pass for "no such folder"); with the numbers given meanwhile,
    /// and what its saves changed in its list.
    Folder(String, std::result::Result<SyncReport, NetError>, Vec<(String, u32)>, quatrelle_store::Noted),
    /// The numbers it gave while reading, when it's done.
    State(Box<jmap::State>),
    /// It couldn't connect: this account syncs on one connection.
    Refused,
}

/// Open a connection like the account's main one, for a folder reader of its own.
pub(super) fn connect_for_sync(acc: &AccountConfig, store: &Store, secret: &str) -> Result<Conn> {
    let user = if acc.username.is_empty() { acc.email.clone() } else { acc.username.clone() };
    if let Some(url) = &acc.graph {
        return Ok(Conn::Graph(crate::graph::Client::connect(url, secret)?));
    }
    if acc.pop.is_some() {
        return Err(NetError::No("POP accounts read mail on one connection".into()));
    }
    if acc.ews.is_some() {
        let url = connect::ews_url(acc, store, &user, secret)?;
        let c = crate::ews::Client::connect(&url, &user, secret, &acc.email)?;
        c.set_shared(&acc.shared_mailboxes);
        return Ok(Conn::Ews(Box::new(c)));
    }
    if let Some(url) = &acc.jmap {
        return Ok(Conn::Jmap(jmap::Client::connect(url, &user, secret, acc.oauth.is_some(), &Trust::of(acc, &[]).sites())?));
    }
    let mut c = Client::connect(&acc.imap)?;
    if acc.oauth.is_some() {
        c.login_xoauth2(&user, secret)?;
    } else {
        c.login(&user, secret)?;
    }
    Ok(Conn::Imap(c))
}

/// Read folders from a shared list on a connection of its own, reporting each as it's read.
pub(super) fn sync_folders(
    acc: AccountConfig,
    store: Store,
    secret: String,
    mut state: jmap::State,
    queue: Arc<Mutex<VecDeque<(String, SyncOptions)>>>,
    tx: Sender<SyncDone>,
) {
    let mut conn = match connect_for_sync(&acc, &store, &secret) {
        Ok(c) => c,
        Err(e) => {
            log::info!("{}: a second connection wasn't taken ({e}): the rest syncs on the first", acc.id);
            let _ = tx.send(SyncDone::Refused);
            let _ = tx.send(SyncDone::State(Box::new(state)));
            return;
        }
    };
    while let Some((path, opts)) = queue.lock().ok().and_then(|mut q| q.pop_front()) {
        let noting = store.note_changes(&acc.id, &path);
        let r = match &mut conn {
            Conn::Imap(c) => sync_folder(c, &store, &acc.id, &path, &opts),
            Conn::Jmap(c) => jmap::sync_folder(c, &mut state, &store, &acc.id, &path, &opts),
            Conn::Graph(c) => crate::graph::sync_folder(c, &mut state, &store, &acc.id, &path, &opts),
            Conn::Ews(c) => crate::ews::sync_folder(c, &mut state, &store, &acc.id, &path, &opts),
            Conn::Pop => Err(NetError::No("POP accounts read mail on one connection".into())),
        };
        let stop = r.as_ref().is_err_and(reader_stops);
        if tx.send(SyncDone::Folder(path, r, state.take_given(), noting.take())).is_err() {
            break;
        }
        if stop {
            break;
        }
    }
    if let Conn::Imap(mut c) = conn {
        c.logout();
    }
    let _ = tx.send(SyncDone::State(Box::new(state)));
}

/// A reader connection's error that ends it (the main connection reads its folder instead): its
/// connection is gone or its sign-in refused.
pub(super) fn reader_stops(e: &NetError) -> bool {
    e.is_fatal() || matches!(e, NetError::Auth(_))
}
