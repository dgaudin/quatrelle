// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The pool: OpenPGP's work with an account's own keys (issue #36), each job on a thread of its
//! own. A key's secret part, its passphrase taken off as it's imported (or never given one, made
//! here), lives in the system's keychain only (`Credentials::set_secret`, one entry
//! `pgp.<fingerprint>` per key, listed in `pgp`, the one in use first), as S/MIME's key does
//! (`smime.rs`); the engine sees the public parts, never a secret one. As with S/MIME
//! certificates, older keys stay to read mail encrypted to them. A keychain that can't keep a
//! key leaves nothing behind, and says so ([`NO_KEYCHAIN`]).

use quatrelle_model::Private;
use quatrelle_model::job::{NO_KEYCHAIN, NOT_FOR_YOU, PgpChange, PgpSignature, PgpWork};
use quatrelle_pgp as pgp;

use super::*;

/// The list of the account's keys' fingerprints (JSON), the one in use first.
const LIST: &str = "pgp";
/// The most keys kept for an account.
pub(super) const KEYS_MAX: usize = 16;

/// Held while the entries change (each job runs on a thread of its own).
static CHANGING: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn changing() -> std::sync::MutexGuard<'static, ()> {
    CHANGING.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn entry(fingerprint: &str) -> String {
    format!("{LIST}.{}", fingerprint.trim().to_lowercase())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| s.get(i..i + 2).and_then(|p| u8::from_str_radix(p, 16).ok())).collect()
}

/// What the keychain said when it wouldn't keep something, in Quatrelle's words first.
fn no_keychain(said: String) -> String {
    format!("{NO_KEYCHAIN}: {said}")
}

/// The fingerprints of the account's keys, the one in use first.
pub(super) fn list(creds: &dyn Credentials, account: &AccountConfig) -> Vec<String> {
    let mut l: Vec<String> = creds.get_secret(account, LIST).and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default();
    l.truncate(KEYS_MAX);
    l
}

fn set_list(creds: &dyn Credentials, account: &AccountConfig, list: &[String]) -> std::result::Result<(), String> {
    if list.is_empty() {
        creds.delete_secret(account, LIST);
        return Ok(());
    }
    creds.set_secret(account, LIST, &serde_json::to_string(list).map_err(|e| e.to_string())?).map_err(no_keychain)
}

/// Key `fingerprint`'s secret part (binary, unlocked).
fn secret(creds: &dyn Credentials, account: &AccountConfig, fingerprint: &str) -> std::result::Result<Vec<u8>, String> {
    let gone = || "the key's secret part isn't in the keychain: import the key again (File › Info › Security)".to_string();
    creds.get_secret(account, &entry(fingerprint)).and_then(|h| unhex(&h)).ok_or_else(gone)
}

/// Key `fingerprint`, unlocked.
fn unlocked(creds: &dyn Credentials, account: &AccountConfig, fingerprint: &str) -> std::result::Result<pgp::Unlocked, String> {
    let s = secret(creds, account, fingerprint)?;
    pgp::read_secret_key(&s, "").map_err(|e| e.to_string())
}

/// Every key's secret part, the one in use first.
fn secrets(creds: &dyn Credentials, account: &AccountConfig) -> Vec<Vec<u8>> {
    list(creds, account).iter().filter_map(|fp| secret(creds, account, fp).ok()).collect()
}

/// The keys' public parts, the one in use first.
fn publics(creds: &dyn Credentials, account: &AccountConfig) -> Vec<Vec<u8>> {
    secrets(creds, account).iter().filter_map(|s| pgp::read_secret_key(s, "").ok()).map(|k| k.public).collect()
}

/// Keep `key`, which signs and decrypts from now on. Either both its entry and the list are
/// written, or neither changes.
fn take(creds: &dyn Credentials, account: &AccountConfig, key: &pgp::Unlocked) -> std::result::Result<(), String> {
    let _changing = changing();
    let fp = key.info.fingerprint.clone();
    let mut l = list(creds, account);
    let had = l.iter().any(|f| f.eq_ignore_ascii_case(&fp));
    l.retain(|f| !f.eq_ignore_ascii_case(&fp));
    if l.len() >= KEYS_MAX {
        return Err(format!("Quatrelle keeps at most {KEYS_MAX} OpenPGP keys for an account: remove one first (File › Info › Security)"));
    }
    creds.set_secret(account, &entry(&fp), &hex(&key.secret)).map_err(no_keychain)?;
    l.insert(0, fp.clone());
    let listed = set_list(creds, account, &l);
    if listed.is_err() && !had {
        creds.delete_secret(account, &entry(&fp));
    }
    listed
}

/// Key `fingerprint` signs and decrypts again.
fn use_key(creds: &dyn Credentials, account: &AccountConfig, fingerprint: &str) -> std::result::Result<(), String> {
    let _changing = changing();
    let mut l = list(creds, account);
    let Some(at) = l.iter().position(|f| f.eq_ignore_ascii_case(fingerprint.trim())) else { return Err("that key isn't kept".into()) };
    let fp = l.remove(at);
    l.insert(0, fp);
    set_list(creds, account, &l)
}

/// Key `fingerprint` signed again to run three years from `now`, kept in place of the old one.
fn extend(creds: &dyn Credentials, account: &AccountConfig, fingerprint: &str, now: i64) -> std::result::Result<String, String> {
    let _changing = changing();
    let key = unlocked(creds, account, fingerprint)?;
    let longer = pgp::extend(&key, now, &mut rand_core::OsRng).map_err(|e| e.to_string())?;
    creds.set_secret(account, &entry(&key.info.fingerprint), &hex(&longer.secret)).map_err(no_keychain)?;
    Ok(longer.info.fingerprint)
}

/// Key `fingerprint`'s backup written to `path`, behind `passphrase`.
fn backup(creds: &dyn Credentials, account: &AccountConfig, fingerprint: &str, passphrase: &str, path: &str) -> std::result::Result<(), String> {
    if path.trim().is_empty() {
        return Err("there's nowhere to save the backup".into());
    }
    let key = unlocked(creds, account, fingerprint)?;
    let armored = pgp::backup(&key, passphrase, &mut rand_core::OsRng).map_err(|e| e.to_string())?;
    quatrelle_store::write_atomic(std::path::Path::new(path), armored.as_bytes()).map_err(|e| e.to_string())
}

fn forget(creds: &dyn Credentials, account: &AccountConfig, fingerprint: &str) {
    let _changing = changing();
    creds.delete_secret(account, &entry(fingerprint));
    let mut l = list(creds, account);
    l.retain(|f| !f.eq_ignore_ascii_case(fingerprint.trim()));
    let _ = set_list(creds, account, &l);
}

/// Every key of the account's, as the account goes.
pub(crate) fn forget_all(creds: &dyn Credentials, account: &AccountConfig) {
    let _changing = changing();
    for fp in list(creds, account) {
        creds.delete_secret(account, &entry(&fp));
    }
    creds.delete_secret(account, LIST);
}

/// The most keys attached inside a decrypted message handed back, and the largest such message
/// looked through for them (as the engine looks through one that isn't encrypted).
const INSIDE_MAX: usize = 4;
const INSIDE_LOOKED_MAX: usize = 2 << 20;

/// What `data` decrypted with the account's keys gives: the MIME entity inside (a text made one,
/// for inline PGP), what a signature in it says, and the public keys attached inside it.
type Opened = (Vec<u8>, Option<PgpSignature>, Vec<Vec<u8>>);

/// `data` decrypted with the account's keys: the MIME entity inside (a text made one, for inline
/// PGP), what a signature in it says, and the keys attached inside it (found here, off the
/// window's thread, for the engine to learn as those of a message that isn't encrypted). Kept in
/// memory only.
pub(crate) fn decrypt(
    creds: &dyn Credentials,
    account: &AccountConfig,
    data: &[u8],
    inline: bool,
    charset: &str,
    verify_with: &[Vec<u8>],
) -> std::result::Result<Opened, String> {
    let mine = secrets(creds, account);
    // The engine asks only for an account with a key: none given back is the keychain's doing.
    if mine.is_empty() {
        return Err(no_keychain("it gave no OpenPGP key for this account".into()));
    }
    let secrets: Vec<&[u8]> = mine.iter().map(Vec::as_slice).collect();
    let keys: Vec<&[u8]> = verify_with.iter().map(Vec::as_slice).collect();
    let d = pgp::decrypt(data, &secrets, &keys).map_err(|e| match e {
        pgp::Error::Decrypt => {
            let to = pgp::recipients(data);
            if to.is_empty() {
                format!("{NOT_FOR_YOU}: import that key too (File › Info › Security)")
            } else {
                format!("{NOT_FOR_YOU} ({}): import that key too (File › Info › Security)", to.join(", "))
            }
        }
        other => other.to_string(),
    })?;
    let signature = d.check.map(|c| match c {
        pgp::Check::Good { key, at, weak } => PgpSignature::Good { key, at, weak },
        pgp::Check::Bad => PgpSignature::Bad,
        pgp::Check::Unknown(key_id) => PgpSignature::Unknown { key_id },
    });
    let entity = if inline { quatrelle_mime::pgp::text_entity(&d.data, charset) } else { d.data };
    let keys = attached_keys(&entity);
    Ok((entity, signature, keys))
}

/// The public keys attached to `entity` (`application/pgp-keys` parts), a few at most.
fn attached_keys(entity: &[u8]) -> Vec<Vec<u8>> {
    let names = entity.len() <= INSIDE_LOOKED_MAX && entity.windows(20).any(|w| w.eq_ignore_ascii_case(b"application/pgp-keys"));
    if !names {
        return Vec::new();
    }
    quatrelle_mime::pgp::key_parts(entity).iter().flat_map(|part| pgp::public_keys(part).unwrap_or_default()).take(INSIDE_MAX).collect()
}

/// `raw` signed with `secret` (`sign`) and/or encrypted to `encrypt_to` (empty: not encrypted),
/// as PGP/MIME, as it will be sent (adapted from upstream f32b54b).
pub(crate) fn protect(raw: &[u8], secret: &[u8], sign: bool, encrypt_to: &[Vec<u8>], hide_subject: bool) -> std::result::Result<Vec<u8>, String> {
    use rand_core::RngCore as _;
    let boundary = format!("=_mcp{:016x}", rand_core::OsRng.next_u64());
    let at = now();
    let keys: Vec<&[u8]> = encrypt_to.iter().map(Vec::as_slice).collect();
    let mut sign_with = |entity: &[u8]| pgp::sign(entity, secret, at, &mut rand_core::OsRng).map(String::into_bytes).map_err(|e| e.to_string());
    let mut encrypt_with =
        |entity: &[u8]| pgp::encrypt(entity, &keys, None, at, &mut rand_core::OsRng).map(String::into_bytes).map_err(|e| e.to_string());
    let signer: Option<quatrelle_mime::smime::Crypto<'_>> = if sign { Some(&mut sign_with) } else { None };
    let encrypter: Option<quatrelle_mime::smime::Crypto<'_>> = if keys.is_empty() { None } else { Some(&mut encrypt_with) };
    quatrelle_mime::pgp::protect(raw, signer, encrypter, hide_subject, &boundary)
}

/// The secret part of the key in use.
fn in_use(creds: &dyn Credentials, account: &AccountConfig) -> std::result::Result<Vec<u8>, String> {
    let none = || no_keychain("it gave no OpenPGP key for this account".into());
    list(creds, account).first().ok_or_else(none).and_then(|fp| secret(creds, account, fp))
}

/// A draft encrypted to the key in use, as what's sent would be.
fn seal(creds: &dyn Credentials, account: &AccountConfig, raw: &[u8]) -> std::result::Result<Vec<u8>, String> {
    let s = in_use(creds, account)?;
    let me = pgp::read_secret_key(&s, "").map_err(|e| e.to_string())?;
    protect(raw, &s, false, &[me.public], false)
}

/// What the account has now, or why it couldn't be done.
fn keys_now(creds: &dyn Credentials, account: &AccountConfig, done: std::result::Result<Option<PgpChange>, String>) -> Event {
    let keys = publics(creds, account);
    match done {
        Ok(changed) => Event::PgpKeys { account: account.id.clone(), keys, changed, error: None },
        Err(e) => Event::PgpKeys { account: account.id.clone(), keys, changed: None, error: Some(e) },
    }
}

/// One job's work, on its own thread (a draft to seal is the pool's own, at once: `None`).
fn work(creds: &dyn Credentials, account: &AccountConfig, work: PgpWork) -> Option<Event> {
    Some(match work {
        PgpWork::Import { file, passphrase } => {
            let done = pgp::read_secret_key(&file.0, &passphrase.0)
                .map_err(|e| e.to_string())
                .and_then(|key| take(creds, account, &key).map(|()| Some(PgpChange::Imported { fingerprint: key.info.fingerprint })));
            keys_now(creds, account, done)
        }
        PgpWork::Make { name, email, now } => {
            let done = pgp::generate(&name, &email, now, &mut rand_core::OsRng)
                .map_err(|e| e.to_string())
                .and_then(|key| take(creds, account, &key).map(|()| Some(PgpChange::Made { fingerprint: key.info.fingerprint })));
            keys_now(creds, account, done)
        }
        PgpWork::Extend { fingerprint, now } => {
            let done = extend(creds, account, &fingerprint, now).map(|fingerprint| Some(PgpChange::Extended { fingerprint }));
            keys_now(creds, account, done)
        }
        PgpWork::Backup { fingerprint, passphrase, path } => {
            let error = backup(creds, account, &fingerprint, &passphrase.0, &path).err();
            Event::PgpBackedUp { account: account.id.clone(), fingerprint, error }
        }
        PgpWork::Use { fingerprint } => keys_now(creds, account, use_key(creds, account, &fingerprint).map(|()| None)),
        PgpWork::Forget { fingerprint } => {
            forget(creds, account, &fingerprint);
            keys_now(creds, account, Ok(None))
        }
        PgpWork::Decrypt { key, data, inline, charset, verify_with } => match decrypt(creds, account, &data, inline, &charset, &verify_with) {
            Ok((entity, signature, keys)) => Event::PgpDecrypted { key, content: Some(Private(entity)), signature, keys, error: None },
            Err(e) => Event::PgpDecrypted { key, content: None, signature: None, keys: Vec::new(), error: Some(e) },
        },
        PgpWork::Protect { id, raw, sign, encrypt_to, hide_subject } => {
            match in_use(creds, account).and_then(|s| protect(&raw.0, &s, sign, &encrypt_to, hide_subject)) {
                Ok(r) => Event::Protected { account: account.id.clone(), id, raw: Some(Private(r)), error: None },
                Err(e) => Event::Protected { account: account.id.clone(), id, raw: None, error: Some(e) },
            }
        }
        PgpWork::SealDraft { .. } => return None,
    })
}

impl Pool {
    pub(super) fn pgp(&mut self, account: AccountConfig, job: PgpWork) {
        // A draft is sealed at once (it may be the last thing before Quatrelle closes), then
        // saved; without the key it's saved as it is, and the user is told.
        if let PgpWork::SealDraft { raw, folder, replaces, keep } = job {
            let raw = match seal(self.creds.as_ref(), &account, &raw.0) {
                Ok(sealed) => sealed,
                Err(e) => {
                    self.sink.emit(Event::Notice {
                        account: account.id.clone(),
                        message: format!("the draft couldn't be encrypted, so it's saved as it is: {e}"),
                    });
                    raw.0
                }
            };
            let account = account.id;
            self.submit(if keep { Job::KeepDraft { account, raw, folder, replaces } } else { Job::SaveDraft { account, raw, folder, replaces } });
            return;
        }
        let creds = self.creds.clone();
        let sink = self.sink.clone();
        let spawned = std::thread::Builder::new().name("mail-pgp".into()).spawn(move || {
            if let Some(e) = work(creds.as_ref(), &account, job) {
                sink.emit(e);
            }
        });
        if let Err(e) = spawned {
            log::warn!("OpenPGP: {e}");
        }
    }
}

#[cfg(test)]
mod tests;
