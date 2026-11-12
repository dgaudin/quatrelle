// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The Teams worker's sign-in: who signed in and of which organisation, the tokens and their
//! renewal, going on from a kept sign-in, and signing out.
//!
//! What is hostile here: the answers of the sign-in service and of Graph (ids, names, the
//! permissions said granted), the Options as read back from disk, and a chat's own fields, which
//! are whoever made the chat's to choose. What it protects: the refresh token (worth every
//! permission asked: in the keychain only, deleted first on signing out), and the copy, bound to
//! the one account Microsoft says signed in.

use quatrelle_model::AccountConfig;
use quatrelle_model::pim::teams::{TEAMS_ACCOUNT, is_bidi, one_line};
use quatrelle_store::{DATA_FILE_EXTENSION, StoreError};

use super::super::graph::{chat_of, member_of};
use super::super::store::{Identity, Opened};
use super::*;
use crate::oauth::TEAMS_SCOPE;

/// Where the keychain keeps the Teams sign-in: an entry of its own, no account's.
pub fn keyring() -> AccountConfig {
    AccountConfig { id: "teams".into(), email: "teams".into(), ..Default::default() }
}

/// What the keychain entry is called: the refresh token.
pub const REFRESH_KEY: &str = "refresh";

/// How long the sign-in in the browser is waited for.
const SIGN_IN_WAIT: Duration = Duration::from_secs(5 * 60);
/// The least between two renewals of a token that says it has run out.
const RENEWALS_APART: Duration = Duration::from_secs(30);
/// The first chats whose members are read to find the user's own entry, when the sign-in named
/// no organisation.
const OWN_ENTRY_CHATS: usize = 10;
/// The most characters shown of why a sign-in failed.
const MAX_WHY: usize = 300;
/// The kind of member an organisation's account is.
const ORGANISATION_MEMBER: &str = "aadUserConversationMember";

const NO_ORGANISATION: &str = "Teams couldn't tell which organisation this account belongs to";
const NO_ACCOUNT: &str = "Teams didn't say who signed in";
const NO_LASTING: &str = "Microsoft gave no lasting sign-in; try again";
const NO_COPY: &str = "The local copy of the conversations couldn't be opened";
const BAD_IDS: &str = "The application or directory ID in the Options can't be one";
const INTERRUPTED: &str = "The sign-in stopped unexpectedly; try again";

/// Who signs the account in and renews its tokens: Microsoft, or the tests' own.
pub(crate) trait Authority: Send + Sync {
    /// Sign in through the browser (`open` shows the page) to the application `client_id` at the
    /// directory `tenant`.
    fn authorize(&self, client_id: &str, tenant: &str, open: &mut dyn FnMut(&str) -> std::result::Result<(), String>) -> Result<Tokens>;
    /// A new access token from a refresh token.
    fn refresh(&self, client_id: &str, tenant: &str, refresh: &str) -> Result<Tokens>;
}

/// Microsoft's sign-in, with Teams' permissions.
pub(crate) struct Microsoft;

fn provider(client_id: &str, tenant: &str) -> Result<oauth::Provider> {
    oauth::teams_provider(client_id, tenant).ok_or_else(|| NetError::Refused(BAD_IDS.into()))
}

impl Authority for Microsoft {
    fn authorize(&self, client_id: &str, tenant: &str, open: &mut dyn FnMut(&str) -> std::result::Result<(), String>) -> Result<Tokens> {
        oauth::authorize(&provider(client_id, tenant)?, "", open, SIGN_IN_WAIT)
    }

    fn refresh(&self, client_id: &str, tenant: &str, refresh: &str) -> Result<Tokens> {
        oauth::refresh(&provider(client_id, tenant)?, refresh)
    }
}

/// A sign-in's answer, from its thread: which one (a later one, a sign-out or a stop makes an
/// earlier one's answer nobody's), what it asked for, and the tokens or why not.
pub(super) struct Authorized {
    sign_in: u64,
    client_id: String,
    tenant: String,
    tokens: Result<Tokens>,
}

/// A directory's id (a GUID), as the Options may name the organisation; anything else there is a
/// domain or `organizations`.
pub(crate) fn is_guid(s: &str) -> bool {
    s.len() == 36 && s.bytes().enumerate().all(|(i, b)| if matches!(i, 8 | 13 | 18 | 23) { b == b'-' } else { b.is_ascii_hexdigit() })
}

/// An account's or an organisation's id as Microsoft writes them: letters, digits and hyphens,
/// a few of them. Anything else isn't taken for one, wherever it came from.
fn plain_id(s: &str) -> bool {
    (1..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// The first of Teams' permissions that `granted` (what the sign-in says it granted) lacks; none
/// when it says nothing. Microsoft lists a permission by its address or by its short name, and
/// never `offline_access`.
pub(crate) fn missing_scope(granted: &str) -> Option<String> {
    let granted: Vec<&str> = granted.split_whitespace().collect();
    if granted.is_empty() {
        return None;
    }
    let has = |scope: &str| {
        let short = scope.rsplit('/').next().unwrap_or(scope);
        granted.iter().any(|g| g.eq_ignore_ascii_case(scope) || g.eq_ignore_ascii_case(short))
    };
    TEAMS_SCOPE.split(' ').filter(|scope| *scope != "offline_access").find(|scope| !has(scope)).map(str::to_string)
}

/// A text to show, from an error: on one line, without control or bidi characters, cut.
fn shown(text: &str, max: usize) -> String {
    text.chars().map(|c| if c.is_whitespace() { ' ' } else { c }).filter(|c| !c.is_control() && !is_bidi(*c)).take(max).collect()
}

/// A failure of the local copy as the log may say it (not its path: it names a chat's hash).
fn store_kind(e: &StoreError) -> String {
    match e {
        StoreError::Io { source, .. } => format!("{:?}", source.kind()),
        StoreError::Json { .. } => "damaged".into(),
    }
}

impl Worker {
    /// [`Job::SignIn`]: the browser's page, on a thread of its own (it can wait five minutes for
    /// the person; the worker goes on hearing its jobs). Whatever was signed in stops being
    /// polled; its copy stays open to read. The answer is taken by [`Worker::authorized`].
    pub(super) fn sign_in(&mut self, client_id: &str, tenant: &str) {
        let (client_id, tenant) = (client_id.trim().to_string(), tenant.trim().to_string());
        if !quatrelle_model::valid_oauth_client_id(&client_id) || !quatrelle_model::valid_oauth_tenant(&tenant) {
            return self.refuse(BAD_IDS);
        }
        self.halt();
        self.sign_ins = self.sign_ins.saturating_add(1);
        let sign_in = self.sign_ins;
        self.status(Status::SigningIn);
        let (authority, emit, answer) = (self.authority.clone(), self.emit.clone(), self.answers.0.clone());
        let spawned = std::thread::Builder::new().name("teams-sign-in".into()).spawn(move || {
            let mut open = |url: &str| -> std::result::Result<(), String> {
                emit(Event::OpenUrl { url: url.to_string() });
                Ok(())
            };
            let tokens = fenced(|| authority.authorize(&client_id, &tenant, &mut open)).unwrap_or_else(|| Err(NetError::Refused(INTERRUPTED.into())));
            let _ = answer.send(Authorized { sign_in, client_id, tenant, tokens });
        });
        match spawned {
            Ok(_) => self.signing = Some(sign_in),
            Err(_) => self.refuse("The sign-in couldn't be started; try again"),
        }
    }

    /// A sign-in said why it didn't happen. Nothing of it is kept.
    fn refuse(&mut self, why: &str) {
        self.status(Status::Failed { why: why.to_string() });
    }

    /// The answers of the sign-in threads: the one awaited is gone on with, any other dropped
    /// with its tokens.
    pub(super) fn authorized(&mut self, now: Instant) {
        while let Ok(answer) = self.answers.1.try_recv() {
            if self.signing == Some(answer.sign_in) {
                self.signing = None;
                // A bug met on the way (a panic) ends this sign-in, said as failed: never "signing
                // in" for good, and nothing of it kept.
                if fenced(|| self.signed(answer, now)).is_none() {
                    log::error!("teams: internal error: the sign-in was given up");
                    self.creds.delete_secret(&keyring(), REFRESH_KEY);
                    self.halt();
                    self.copy = None;
                    self.refuse(INTERRUPTED);
                }
            } else {
                log::info!("teams: a sign-in nobody waits for any more was dropped");
            }
        }
    }

    /// The browser's sign-in ended: what it granted is checked, Graph says who it is, the
    /// organisation is told, the copy is opened for that identity; only then is the refresh token
    /// kept. A refusal at any step keeps nothing.
    fn signed(&mut self, answer: Authorized, now: Instant) {
        #[cfg(test)]
        Hooks::trip_on(&self.hooks.trip_sign_in);
        let Authorized { client_id, tenant, tokens, .. } = answer;
        let tokens = match tokens {
            Ok(tokens) => tokens,
            Err(e) => {
                log::warn!("teams: the sign-in didn't end: {}", kind(&e));
                return self.refuse(&shown(&e.to_string(), MAX_WHY));
            }
        };
        let Some(refresh) = tokens.refresh.clone().filter(|r| !r.is_empty()) else { return self.refuse(NO_LASTING) };
        if let Some(scope) = missing_scope(&tokens.granted) {
            log::warn!("teams: the sign-in lacks a permission");
            return self.status(Status::PermissionMissing { scope });
        }
        // The directory the Options name by its id: whatever says the account's organisation
        // has to say that one.
        let elsewhere = |said: &str| is_guid(&tenant) && !said.eq_ignore_ascii_case(&tenant);
        let tid = tokens.tenant.as_deref().map(str::trim).filter(|t| plain_id(t)).map(str::to_string);
        if tid.as_deref().is_some_and(elsewhere) {
            log::warn!("teams: the sign-in is of another directory than the Options'");
            return self.refuse(NO_ORGANISATION);
        }
        self.limiter = Limiter::new(now);
        self.limiter.took(Route::Me, now);
        // Never waited out in a request: the limiter is what waits, at the loop's deadlines.
        let (client, me) = match TeamsClient::connect_waiting(&self.graph, &tokens.access, false) {
            Ok(connected) => connected,
            Err(e) => {
                log::warn!("teams: Graph didn't say who signed in: {}", kind(&e));
                return self.refuse(&format!("{NO_ACCOUNT} ({})", kind(&e)));
            }
        };
        if !plain_id(&me.id) {
            log::warn!("teams: the account's id can't be one");
            return self.refuse(NO_ACCOUNT);
        }
        let by_sign_in = tid.is_some();
        let Some(tenant_id) = tid.or_else(|| self.own_tenant(&client, &me.id, now)).filter(|t| !elsewhere(t)) else {
            log::warn!("teams: nothing the account can be bound to says its organisation");
            return self.refuse(NO_ORGANISATION);
        };
        self.limiter.took(Route::Drive, now);
        let drive_host = client.drive_host().unwrap_or_else(|e| {
            log::info!("teams: the OneDrive's host wasn't read: {}", kind(&e));
            None
        });
        let who = Identity { tenant_id: tenant_id.clone(), object_id: me.id.clone() };
        let (copy, opened) = match self.open_copy(&who) {
            Ok(open) => open,
            Err(e) => {
                log::error!("teams: the local copy couldn't be opened: {}", store_kind(&e));
                return self.refuse(NO_COPY);
            }
        };
        // Kept only now: in the keychain, or for want of one in memory until Quatrelle closes
        // (credentials that take it but keep nothing past this run are no keychain either).
        let taken = self.creds.set_secret(&keyring(), REFRESH_KEY, &refresh).is_ok();
        self.no_keychain = !(taken && self.creds.persists());
        if !taken {
            // One kept before isn't this sign-in's.
            self.creds.delete_secret(&keyring(), REFRESH_KEY);
        }
        if self.no_keychain {
            log::warn!("teams: no keychain keeps the sign-in: it lasts until Quatrelle closes");
        }
        let address = if me.mail.trim().is_empty() { me.upn.trim() } else { me.mail.trim() };
        self.settings = TeamsSettings {
            on: true,
            client_id,
            tenant,
            signed_in: true,
            name: one_line(&me.display_name),
            address: shown(address, 320),
            object_id: who.object_id,
            tenant_id: who.tenant_id,
            drive_host,
            notify: self.settings.notify,
            notify_only_direct: self.settings.notify_only_direct,
        };
        let s = &self.settings;
        self.say(teams::Event::Account {
            name: s.name.clone(),
            address: s.address.clone(),
            object_id: s.object_id.clone(),
            tenant_id: s.tenant_id.clone(),
            drive_host: s.drive_host.clone(),
        });
        self.client = Some(client);
        self.copy = Some(copy);
        self.tokens = Some(tokens);
        self.refresh = Some(Secret(refresh));
        self.set_aside(opened);
        if self.no_keychain {
            self.status(Status::NoKeychain);
        }
        self.polling = true;
        log::info!("teams: signed in, the organisation told by {}", if by_sign_in { "the sign-in" } else { "the account's own membership" });
    }

    /// The organisation of the account `me`, as its own entry among the members of one of its
    /// first chats says it: an organisation's member, with the account's own id. Never a chat's
    /// own `tenantId`: that one is the organisation of whoever made the chat.
    fn own_tenant(&mut self, client: &TeamsClient, me: &str, now: Instant) -> Option<String> {
        self.limiter.took(Route::ChatList, now);
        let chats = match client.chats(None) {
            Ok(page) => page,
            Err(e) => {
                log::warn!("teams: the conversations weren't listed to find the organisation: {}", kind(&e));
                return None;
            }
        };
        let mut looked = 0u32;
        for chat in chats.items.iter().filter_map(chat_of).take(OWN_ENTRY_CHATS) {
            looked = looked.saturating_add(1);
            self.limiter.took(Route::Members, now);
            let Ok(members) = client.members(&chat.id) else { continue };
            let own = members.iter().filter_map(member_of).find(|m| m.kind == ORGANISATION_MEMBER && m.user_id.eq_ignore_ascii_case(me));
            if let Some(tenant) = own.and_then(|m| m.tenant_id).filter(|t| plain_id(t)) {
                log::info!("teams: the account's own entry found in conversation {} ({looked} looked at)", self.hashed(&chat.id));
                return Some(tenant);
            }
        }
        log::warn!("teams: the account's own entry wasn't found ({looked} conversations looked at)");
        None
    }

    /// The copy for `who`, opened behind a guard: reading a file can panic (a parser's bug, damage
    /// nothing checked for). One that does is set aside as damaged — deleted, in a cache that
    /// keeps no data files — and an empty one begun, rebuilt from the server: never the same file
    /// opened again, nor the worker stopped over it.
    fn open_copy(&self, who: &Identity) -> std::result::Result<(Copy, Opened), StoreError> {
        let open = || {
            #[cfg(test)]
            Hooks::trip_on(&self.hooks.trip_open);
            Copy::open(self.store.clone(), who)
        };
        if let Some(opened) = fenced(open) {
            return opened;
        }
        log::error!("teams: internal error reading the local copy; it is set aside as damaged and begun again");
        let damaged = format!("{TEAMS_ACCOUNT}.{DATA_FILE_EXTENSION}.damaged");
        if self.store.set_aside_data_file(TEAMS_ACCOUNT, &damaged)?.is_none() {
            self.store.delete_account(TEAMS_ACCOUNT)?;
        }
        fenced(|| Copy::open(self.store.clone(), who))
            .unwrap_or_else(|| Err(StoreError::Io { path: TEAMS_ACCOUNT.into(), source: std::io::Error::other("the local copy can't be read") }))
    }

    /// A copy that was in the way was set aside: said, for the Options to offer its deletion.
    fn set_aside(&mut self, opened: Opened) {
        if let Opened::SetAside(path) = opened {
            log::warn!("teams: a copy made for another account was set aside, unread");
            self.say(teams::Event::SetAside { path: path.display().to_string() });
        }
    }

    /// [`Job::DeleteSetAside`]: a copy set aside deleted, as the user chose; a failure is said.
    pub(super) fn delete_set_aside(&mut self, path: &str) {
        match Copy::delete_set_aside(&self.store, path) {
            Ok(true) => log::info!("teams: a copy set aside was deleted"),
            Ok(false) => log::warn!("teams: no copy set aside there to delete"),
            Err(e) => {
                log::error!("teams: a copy set aside couldn't be deleted: {}", store_kind(&e));
                (self.emit)(Event::Notice { account: String::new(), message: "Teams: the copy set aside couldn't be deleted".into() });
            }
        }
    }

    /// [`Job::Start`]: go on from the sign-in kept (Quatrelle starting, signed in before). The
    /// copy is opened at once, to be read with or without a network; Graph is asked at the first
    /// turn ([`Worker::ready`]). No sign-in kept: "sign in again", with the copy open to read;
    /// Options that say nobody: the same, without it. Signing in is the user's own click (the
    /// browser never opens by itself).
    pub(super) fn start(&mut self, settings: TeamsSettings) {
        if !settings.on || !settings.signed_in {
            return log::info!("teams: not started: switched off, or nobody signed in");
        }
        let s = &self.settings;
        let same = [
            (&s.object_id, &settings.object_id),
            (&s.tenant_id, &settings.tenant_id),
            (&s.client_id, &settings.client_id),
            (&s.tenant, &settings.tenant),
        ];
        // Going already for this account, or a sign-in on its way (which says who it is when it
        // ends): only what the Options changed besides.
        if self.signing.is_some() || (self.polling && same.iter().all(|(a, b)| a.eq_ignore_ascii_case(b.trim()))) {
            self.settings.notify = settings.notify;
            self.settings.notify_only_direct = settings.notify_only_direct;
            return;
        }
        self.halt();
        self.copy = None;
        let who = Identity { tenant_id: settings.tenant_id.trim().to_string(), object_id: settings.object_id.trim().to_string() };
        let (client_id, tenant) = (settings.client_id.trim().to_string(), settings.tenant.trim().to_string());
        // The Options come from a file: ids that could be ones, and, as at the sign-in, a
        // directory named by its id is the account's organisation.
        let sound = quatrelle_model::valid_oauth_client_id(&client_id)
            && quatrelle_model::valid_oauth_tenant(&tenant)
            && plain_id(&who.tenant_id)
            && plain_id(&who.object_id)
            && (!is_guid(&tenant) || tenant.eq_ignore_ascii_case(&who.tenant_id));
        if !sound {
            log::warn!("teams: the Options don't say who signed in");
            return self.status(Status::SignInAgain);
        }
        match self.open_copy(&who) {
            Ok((copy, opened)) => {
                self.copy = Some(copy);
                self.set_aside(opened);
            }
            Err(e) => {
                log::error!("teams: the local copy couldn't be opened: {}", store_kind(&e));
                return self.refuse(NO_COPY);
            }
        }
        self.settings = TeamsSettings { client_id, tenant, object_id: who.object_id, tenant_id: who.tenant_id, ..settings };
        // The list as the copy has it, at once, online or not.
        crate::teams::backfill::from_copy(self);
        // No sign-in kept (no keychain last time, or its entry gone): the copy is there to read.
        let Some(refresh) = self.creds.get_secret(&keyring(), REFRESH_KEY).filter(|r| !r.is_empty()) else {
            log::info!("teams: no sign-in is kept to go on from");
            return self.status(Status::SignInAgain);
        };
        self.refresh = Some(Secret(refresh));
        self.no_keychain = !self.creds.persists();
        // Said once, as at a sign-in; the status goes on from there.
        if self.no_keychain {
            self.status(Status::NoKeychain);
        }
        self.limiter = Limiter::new(Instant::now());
        self.polling = true;
        log::info!("teams: going on from the sign-in kept");
    }

    /// Whether Graph can be asked at `now`: the client made (after a start, at the first turn
    /// that finds the network) and its token not about to run out. A failure is said and tried
    /// again in a while.
    pub(super) fn ready(&mut self, now: Instant) -> bool {
        let made = if self.client.is_none() {
            self.connect(now)
        } else if self.stale(now) {
            self.renew(now)
        } else {
            return true;
        };
        if let Err(e) = &made
            && self.polling
        {
            log::warn!("teams: Graph can't be asked yet: {}", kind(e));
            self.pause(e, now);
        }
        made.is_ok() && self.client.is_some()
    }

    /// Whether the access token is to be renewed: there is none, or it has run out or will within
    /// a minute by the wall clock (which goes on while the computer sleeps; an `Instant`
    /// doesn't). A token just renewed isn't renewed again for [`RENEWALS_APART`], whatever it
    /// says of itself: a sign-in service whose tokens never last isn't asked at every turn (a
    /// token refused is still renewed, once, where it's refused).
    fn stale(&self, now: Instant) -> bool {
        let just_renewed = self.renewed.is_some_and(|at| now.saturating_duration_since(at) < RENEWALS_APART);
        match &self.tokens {
            Some(t) => !just_renewed && oauth::now_unix() >= t.expires_at.saturating_sub(60),
            None => true,
        }
    }

    /// The client of a kept sign-in: a renewal, then Graph asked whose token it is. Another
    /// account's than the one the Options name (and the copy was opened for) is never gone on
    /// with: nothing of its chats is read into this copy.
    fn connect(&mut self, now: Instant) -> Result<()> {
        if self.stale(now) {
            self.renew(now)?;
        }
        let Some(access) = self.tokens.as_ref().map(|t| t.access.clone()) else { return Ok(()) };
        self.limiter.took(Route::Me, now);
        let (client, me) = match TeamsClient::connect_waiting(&self.graph, &access, false) {
            Ok(connected) => connected,
            Err(NetError::Auth(_)) => {
                log::warn!("teams: Graph refused the renewed token");
                self.sign_in_again();
                return Ok(());
            }
            Err(e) => return Err(e),
        };
        if !me.id.eq_ignore_ascii_case(&self.settings.object_id) {
            log::warn!("teams: the sign-in kept is another account's than the copy's");
            self.sign_in_again();
            return Ok(());
        }
        self.client = Some(client);
        Ok(())
    }

    /// A new access token from the refresh token: the one renewal there is, on this thread. A
    /// newer refresh token handed with it replaces the one kept. Refused, it is "sign in again"
    /// (and an error, so that what asked stops); anything else is the network's and tried again.
    pub(super) fn renew(&mut self, now: Instant) -> Result<()> {
        let refused = || NetError::Auth("the Teams sign-in is no longer good; sign in again".into());
        let Some(refresh) = self.refresh.clone() else {
            self.sign_in_again();
            return Err(refused());
        };
        let tokens = match self.authority.refresh(&self.settings.client_id, &self.settings.tenant, &refresh.0) {
            Ok(tokens) => tokens,
            Err(NetError::Auth(_) | NetError::Refused(_)) => {
                log::warn!("teams: the renewal was refused");
                self.sign_in_again();
                return Err(refused());
            }
            Err(e) => return Err(e),
        };
        if let Some(newer) = tokens.refresh.as_deref().filter(|r| !r.is_empty() && *r != refresh.0) {
            // Where the one before is held: a keychain that refuses now still holds that one.
            let held = self.creds.get_secret(&keyring(), REFRESH_KEY).is_some();
            if held && self.creds.set_secret(&keyring(), REFRESH_KEY, newer).is_err() {
                log::warn!("teams: the keychain didn't take the newer sign-in");
            }
            self.refresh = Some(Secret(newer.to_string()));
        }
        if let Some(client) = &self.client {
            client.set_token(&tokens.access);
        }
        self.tokens = Some(tokens);
        self.renewed = Some(now);
        log::info!("teams: the access token was renewed");
        Ok(())
    }

    /// The sign-in no longer holds (revoked, run out, or not this account's): nothing more is
    /// asked of Teams until the user signs in again. The copy stays open to read.
    pub(super) fn sign_in_again(&mut self) {
        self.halt();
        self.status(Status::SignInAgain);
    }

    /// [`Job::SignOut`]: the refresh token is deleted first (whatever fails after, the sign-in is
    /// gone), then the copy is closed and, when asked, deleted with every copy of it set aside as
    /// damaged.
    pub(super) fn sign_out(&mut self, delete_copy: bool) {
        self.creds.delete_secret(&keyring(), REFRESH_KEY);
        self.halt();
        self.copy = None;
        self.open = None;
        self.settings = TeamsSettings::default();
        if delete_copy && let Err(e) = Copy::delete_in(&self.store) {
            log::error!("teams: the local copy couldn't be deleted: {}", store_kind(&e));
            (self.emit)(Event::Notice { account: String::new(), message: "Teams: the local copy of the conversations couldn't be deleted".into() });
        }
        log::info!("teams: signed out{}", if delete_copy { ", the local copy deleted" } else { "" });
        // Said each time it's asked, signed in or not.
        self.said = None;
        self.status(Status::SignedOut);
    }
}
