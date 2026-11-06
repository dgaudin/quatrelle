// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The session: OpenPGP (issue #36), beside S/MIME. An account's own keys (made here or imported
//! from a key file; their secret parts kept by the network layer in the system's keychain, only
//! the public parts here; the one in use first, older ones kept to decrypt). Nothing is active
//! until the account has one ([`Session::pgp_active`]): no Autocrypt header, no signing, no
//! encryption. A key made here asks for its backup until one is saved, and says a month ahead
//! that it runs out. Correspondents' keys are [`people`]'s; reading OpenPGP mail is [`reading`]'s,
//! sending it [`writing`]'s.

use quatrelle_model::job::{PgpChange, PgpWork};
use quatrelle_pgp as openpgp;

use super::*;

mod people;
mod reading;
mod writing;
pub use people::{Learn, Learnt, PEOPLE_DOC, PEOPLE_MAX, PeopleKeys, PersonKey};
pub(crate) use people::{OfferedKey, offered};
pub(crate) use reading::Step;
pub use writing::{NO_KEY_TO_ENCRYPT, Reach, Scheme};

/// An account's own OpenPGP keys and settings (the account document `pgp.json`).
pub const PGP_DOC: &str = "pgp.json";

/// An account's OpenPGP: its own keys, and whether its mail sends the public key along.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AccountPgp {
    /// The one that signs and decrypts first, then older ones kept to decrypt.
    pub keys: Vec<OwnKey>,
    /// Its mail carries the public key in an `Autocrypt:` header (on once a key is made).
    pub autocrypt: bool,
}

impl Default for AccountPgp {
    fn default() -> AccountPgp {
        AccountPgp { keys: Vec::new(), autocrypt: true }
    }
}

/// One of the account's own keys.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OwnKey {
    /// The public part (binary); the secret part is in the keychain.
    pub public: Vec<u8>,
    /// What it is, read from `public`.
    #[serde(skip)]
    pub info: openpgp::KeyInfo,
    /// A backup of it was saved (a key imported came from one).
    pub backed_up: bool,
    /// It signs and decrypts (the first); the others only decrypt.
    #[serde(skip)]
    pub in_use: bool,
}

/// What File › Info › Security's OpenPGP card shows of an account.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PgpCard {
    /// The one in use first.
    pub keys: Vec<OwnKey>,
    /// A key made here has no backup yet.
    pub needs_backup: bool,
    /// When the key in use runs out (or ran out), within [`openpgp::EXPIRY_WARNING_SECS`].
    pub expires_soon: Option<i64>,
    /// The public key goes with the account's mail.
    pub autocrypt: bool,
    /// Correspondents' keys, by address.
    pub people: Vec<PersonKey>,
}

/// The session's OpenPGP state.
#[derive(Debug, Default)]
pub(crate) struct PgpState {
    accounts: HashMap<String, AccountPgp>,
    people: HashMap<String, PeopleKeys>,
    /// Messages being decrypted: the keys their signature is checked with, and their sender.
    asked: HashMap<MessageKey, (Vec<Vec<u8>>, String)>,
    /// What signatures inside decrypted messages say (in memory, as what they held is).
    signed: HashMap<MessageKey, SignedBy>,
    /// Whether the message being written can go encrypted to everyone, and what that was
    /// worked out from (a digest).
    reach: Option<(u64, Reach)>,
}

/// `p` as it's kept, its keys read again from their public parts (one that can't be read is let
/// go: the file is Quatrelle's, but it's read as any file is), the first in use.
fn read_keys(mut p: AccountPgp) -> AccountPgp {
    p.keys.retain_mut(|k| match openpgp::info(&k.public) {
        Ok(info) => {
            k.info = info;
            true
        }
        Err(_) => false,
    });
    for (i, k) in p.keys.iter_mut().enumerate() {
        k.in_use = i == 0;
    }
    p
}

/// A fingerprint as written (spaces, any case) as the keys have it.
fn bare(fingerprint: &str) -> String {
    fingerprint.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_uppercase()
}

impl Session {
    pub(crate) fn load_pgp(&mut self) {
        let Some(store) = self.store.clone() else { return };
        for a in &self.accounts {
            let p: AccountPgp = store.load_account_doc(&a.config.id, PGP_DOC).unwrap_or_default();
            self.pgp.accounts.insert(a.config.id.clone(), read_keys(p));
        }
        let ids: Vec<String> = self.accounts.iter().map(|a| a.config.id.clone()).collect();
        for id in ids {
            self.load_people(&id);
        }
    }

    fn save_pgp(&mut self, account: &str) {
        let (Some(store), Some(p)) = (&self.store, self.pgp.accounts.get(account)) else { return };
        if let Err(e) = store.save_account_doc(account, PGP_DOC, p) {
            self.notice_with("the OpenPGP settings could not be saved: {error}", &[("error", &e.to_string())], true);
        }
    }

    /// An account's own OpenPGP keys and settings.
    pub fn pgp_settings(&self, account: &str) -> AccountPgp {
        self.pgp.accounts.get(account).cloned().unwrap_or_default()
    }

    /// Whether OpenPGP is on for `account`: it has a key of its own in use. Every sending path
    /// asks this: without one, nothing is signed, encrypted or sent along.
    pub fn pgp_active(&self, account: &str) -> bool {
        self.pgp.accounts.get(account).is_some_and(|p| p.keys.iter().any(|k| k.in_use))
    }

    /// The account's key in use, when it has one.
    pub fn pgp_key(&self, account: &str) -> Option<&OwnKey> {
        self.pgp.accounts.get(account).and_then(|p| p.keys.iter().find(|k| k.in_use))
    }

    /// What the OpenPGP card shows of `account`.
    pub fn pgp_card(&self, account: &str) -> PgpCard {
        let p = self.pgp_settings(account);
        let now = self.clock.now;
        let expires_soon =
            p.keys.iter().find(|k| k.in_use).and_then(|k| k.info.expires).filter(|end| end.saturating_sub(now) <= openpgp::EXPIRY_WARNING_SECS);
        PgpCard {
            needs_backup: p.keys.iter().any(|k| !k.backed_up),
            expires_soon,
            autocrypt: p.autocrypt,
            keys: p.keys,
            people: self.pgp_people(account),
        }
    }

    fn pgp_config(&self, account: &str) -> Result<AccountConfig, CmdError> {
        self.account(account).map(|a| a.config.clone()).ok_or_else(|| CmdError::Params(format!("no account `{account}`")))
    }

    /// One of the account's own keys by fingerprint (spaces and case don't matter; its key ID
    /// will do): its fingerprint as kept.
    fn own_fingerprint(&self, account: &str, fingerprint: &str) -> Result<String, CmdError> {
        let wanted = bare(fingerprint);
        self.pgp
            .accounts
            .get(account)
            .into_iter()
            .flat_map(|p| p.keys.iter())
            .find(|k| !wanted.is_empty() && (k.info.fingerprint == wanted || k.info.key_id == wanted))
            .map(|k| k.info.fingerprint.clone())
            .ok_or_else(|| CmdError::Params(format!("no OpenPGP key `{fingerprint}` for this account")))
    }

    /// Make a key for `account` (its name and address), which signs and decrypts once made.
    pub fn create_pgp_key(&mut self, account: &str) -> CmdResult {
        let config = self.pgp_config(account)?;
        let work = PgpWork::Make { name: config.name.clone(), email: config.email.trim().to_string(), now: self.clock.now };
        self.jobs.push(Job::Pgp { account: config, work });
        Ok(serde_json::json!({ "making": account }))
    }

    /// Sign one of the account's keys again to run three years from today.
    pub fn extend_pgp_key(&mut self, account: &str, fingerprint: &str) -> CmdResult {
        let config = self.pgp_config(account)?;
        let fingerprint = self.own_fingerprint(account, fingerprint)?;
        self.jobs.push(Job::Pgp { account: config, work: PgpWork::Extend { fingerprint: fingerprint.clone(), now: self.clock.now } });
        Ok(serde_json::json!({ "extending": fingerprint }))
    }

    /// Save a backup of one of the account's keys to `path`, behind `passphrase`.
    pub fn backup_pgp_key(&mut self, account: &str, fingerprint: &str, passphrase: &str, path: &str) -> CmdResult {
        let config = self.pgp_config(account)?;
        let fingerprint = self.own_fingerprint(account, fingerprint)?;
        if passphrase.is_empty() {
            return Err(CmdError::Params("a backup needs a passphrase".into()));
        }
        if path.trim().is_empty() {
            return Err(CmdError::Params("`path`: where to save the backup".into()));
        }
        let work =
            PgpWork::Backup { fingerprint: fingerprint.clone(), passphrase: quatrelle_model::Secret(passphrase.to_string()), path: path.to_string() };
        self.jobs.push(Job::Pgp { account: config, work });
        Ok(serde_json::json!({ "saving": fingerprint }))
    }

    /// Write one of the account's public keys (`fingerprint`, else the one in use) to `path`, as
    /// a key file others import.
    pub fn export_public_key(&mut self, account: &str, fingerprint: Option<&str>, path: &str) -> CmdResult {
        self.pgp_config(account)?;
        let public = match fingerprint {
            Some(f) => {
                let fp = self.own_fingerprint(account, f)?;
                self.pgp.accounts.get(account).and_then(|p| p.keys.iter().find(|k| k.info.fingerprint == fp)).map(|k| k.public.clone())
            }
            None => self.pgp_key(account).map(|k| k.public.clone()),
        };
        let public = public.ok_or_else(|| CmdError::Disabled("this account has no OpenPGP key".into()))?;
        if path.trim().is_empty() {
            return Err(CmdError::Params("`path`: where to save the key".into()));
        }
        let armored = openpgp::armored_public(&public).map_err(|e| CmdError::Failed(e.to_string()))?;
        std::fs::write(path, armored).map_err(|e| CmdError::Failed(format!("{path}: {e}")))?;
        Ok(serde_json::json!({ "saved": path }))
    }

    /// Keep the secret key in the file at `path` (opened with `passphrase`) for `account`.
    pub fn import_pgp_key(&mut self, account: &str, path: &str, passphrase: &str) -> CmdResult {
        let data = crate::cmd::read_capped(path, 1 << 20)?;
        self.import_pgp_key_data(account, data, passphrase)
    }

    /// [`Session::import_pgp_key`] from the file's bytes.
    pub fn import_pgp_key_data(&mut self, account: &str, data: Vec<u8>, passphrase: &str) -> CmdResult {
        let config = self.pgp_config(account)?;
        if data.len() > 1 << 20 {
            return Err(CmdError::Params("that file is too big for an OpenPGP key".into()));
        }
        let work = PgpWork::Import { file: Private(data), passphrase: quatrelle_model::Secret(passphrase.to_string()) };
        self.jobs.push(Job::Pgp { account: config, work });
        Ok(serde_json::json!({ "importing": account }))
    }

    /// Forget one of the account's keys (its secret part goes from the keychain too): mail
    /// encrypted to it can't be read any more.
    pub fn forget_pgp_key(&mut self, account: &str, fingerprint: &str) -> CmdResult {
        let config = self.pgp_config(account)?;
        let fp = self.own_fingerprint(account, fingerprint)?;
        if let Some(p) = self.pgp.accounts.get_mut(account) {
            p.keys.retain(|k| k.info.fingerprint != fp);
            *p = read_keys(std::mem::take(p));
        }
        self.jobs.push(Job::Pgp { account: config, work: PgpWork::Forget { fingerprint: fp.clone() } });
        self.save_pgp(account);
        Ok(serde_json::json!({ "forgotten": fp }))
    }

    /// Sign and decrypt with one of the account's older keys again.
    pub fn use_pgp_key(&mut self, account: &str, fingerprint: &str) -> CmdResult {
        let config = self.pgp_config(account)?;
        let fp = self.own_fingerprint(account, fingerprint)?;
        self.jobs.push(Job::Pgp { account: config, work: PgpWork::Use { fingerprint: fp.clone() } });
        Ok(serde_json::json!({ "using": fp }))
    }

    /// Whether `account`'s mail sends its public key along (`autocrypt`).
    pub fn set_pgp_options(&mut self, account: &str, autocrypt: Option<bool>) -> CmdResult {
        self.pgp_config(account)?;
        let p = self.pgp.accounts.entry(account.to_string()).or_default();
        if let Some(on) = autocrypt {
            p.autocrypt = on;
        }
        let out = serde_json::json!({ "autocrypt": p.autocrypt });
        self.save_pgp(account);
        Ok(out)
    }

    /// An account going: each of its keys' secret parts asked to go from the keychain, and its
    /// OpenPGP documents (its keys', its correspondents') from its data file, which stays.
    pub(in crate::session) fn forget_pgp_account(&mut self, account: &str) {
        let config = self.account(account).map(|a| a.config.clone());
        if let (Some(config), Some(p)) = (config, self.pgp.accounts.remove(account)) {
            for k in p.keys {
                self.jobs.push(Job::Pgp { account: config.clone(), work: PgpWork::Forget { fingerprint: k.info.fingerprint } });
            }
        }
        self.pgp.people.remove(account);
        for doc in [PGP_DOC, PEOPLE_DOC] {
            if let Some(store) = &self.store
                && let Err(e) = store.remove_account_doc(account, doc)
            {
                log::warn!("{account}: {doc}: {e}");
            }
        }
    }

    /// The account's keys as the network layer has them now (the one in use first). An empty
    /// answer never takes them away (a keychain that gave nothing back): forgetting one is done
    /// here as it's asked.
    fn heard_pgp_keys(&mut self, account: &str, keys: Vec<Vec<u8>>, changed: Option<PgpChange>, error: Option<String>) {
        let had = self.pgp.accounts.get(account).map(|p| p.keys.clone()).unwrap_or_default();
        if !keys.is_empty() {
            let made = match &changed {
                Some(PgpChange::Made { fingerprint }) => Some(fingerprint.clone()),
                _ => None,
            };
            let p = self.pgp.accounts.entry(account.to_string()).or_default();
            p.keys = keys.into_iter().map(|public| OwnKey { public, ..Default::default() }).collect();
            *p = read_keys(std::mem::take(p));
            for k in &mut p.keys {
                k.backed_up = match had.iter().find(|h| h.info.fingerprint == k.info.fingerprint) {
                    Some(h) => h.backed_up,
                    // Made here: no backup yet. Imported: it came from one.
                    None => made.as_deref() != Some(k.info.fingerprint.as_str()),
                };
            }
            self.save_pgp(account);
        }
        if let Some(e) = error {
            self.notice_with("The OpenPGP key couldn't be kept: {error}", &[("error", &e)], true);
            return;
        }
        let who = |s: &Session, fp: &str| {
            let keys = s.pgp.accounts.get(account).map(|p| p.keys.as_slice()).unwrap_or_default();
            keys.iter().find(|k| k.info.fingerprint == fp).map(|k| k.info.emails.first().cloned().unwrap_or(k.info.name.clone()))
        };
        match changed {
            Some(PgpChange::Made { fingerprint }) => {
                let who = who(self, &fingerprint).unwrap_or_default();
                self.notice_with("Your OpenPGP key for {who} is made: save a backup of it", &[("who", &who)], false);
            }
            Some(PgpChange::Imported { fingerprint }) => {
                let who = who(self, &fingerprint).unwrap_or_default();
                self.notice_with("Your OpenPGP key for {who} is ready", &[("who", &who)], false);
            }
            Some(PgpChange::Extended { fingerprint }) => {
                let end = self
                    .pgp
                    .accounts
                    .get(account)
                    .and_then(|p| p.keys.iter().find(|k| k.info.fingerprint == fingerprint))
                    .and_then(|k| k.info.expires);
                if let Some(end) = end {
                    let until = quatrelle_model::dates::full_time(end, &self.clock);
                    self.notice_with("Your OpenPGP key now runs until {until}", &[("until", &until)], false);
                }
            }
            None => {}
        }
        // Mail that couldn't be read for want of a key is tried again.
        let failed: Vec<MessageKey> = self.decrypt_errors.drain().map(|(k, _)| k).collect();
        for k in failed {
            self.show_again(&k);
        }
        self.show_again_without_key(account);
    }

    fn heard_pgp_backup(&mut self, account: &str, fingerprint: &str, error: Option<String>) {
        if let Some(e) = error {
            self.notice_with("The OpenPGP key's backup couldn't be saved: {error}", &[("error", &e)], true);
            return;
        }
        if let Some(k) = self.pgp.accounts.get_mut(account).and_then(|p| p.keys.iter_mut().find(|k| k.info.fingerprint == fingerprint)) {
            k.backed_up = true;
        }
        self.save_pgp(account);
        self.notice("The OpenPGP key's backup is saved: keep it, and its passphrase, somewhere safe", false);
    }

    pub(super) fn apply_pgp(&mut self, e: Event) {
        match e {
            Event::PgpKeys { account, keys, changed, error } => self.heard_pgp_keys(&account, keys, changed, error),
            Event::PgpBackedUp { account, fingerprint, error } => self.heard_pgp_backup(&account, &fingerprint, error),
            Event::PgpDecrypted { key, content, signature, keys, error } => self.heard_pgp_decrypted(key, content, signature, keys, error),
            _ => {}
        }
    }
}
