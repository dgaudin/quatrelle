// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The session: correspondents' OpenPGP keys (adapted from upstream f32b54b), learnt on this
//! computer only: from their mail (its `Autocrypt:` header for the address it comes from, a key
//! attached for that address), or imported from a file. A newer message's key replaces an older
//! one's (Autocrypt Level 1 §2.3), never one imported by hand; a key that changes is said once,
//! on the message that brought it. At most [`PEOPLE_MAX`] per account, the one used least
//! recently let go first, never one imported by hand. Kept in the account's `pgp-people.json`,
//! read as any file is (a damaged one is set aside, never fatal).
//!
//! What a message brings is found where the message is read for showing already: on the reading
//! lane, away from the window's thread, as its body is got ready ([`offered`], in
//! `BodyWork::prepare`), and learnt from what that found. Where bodies aren't got ready elsewhere
//! (the CLI, the tests), it's found as the message is shown.

use super::*;

/// An account's correspondents' keys (an account document).
pub const PEOPLE_DOC: &str = "pgp-people.json";

/// The most correspondents' keys kept for an account.
pub const PEOPLE_MAX: usize = 10_000;

/// The largest key kept.
const KEY_MAX: usize = 1 << 20;

/// The most keys taken from one message.
pub(super) const OFFERED_MAX: usize = 8;

/// The largest message whose attached keys are looked for (as `mime::pgp::may_carry` does).
const KEYS_LOOKED_MAX: usize = 2 << 20;

/// How a correspondent's key came.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Learnt {
    /// From their mail: a newer message's key replaces it.
    #[default]
    FromMail,
    /// Imported by the user: only « Forget » lets it go.
    ByHand,
}

/// Someone's key.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PersonKey {
    /// Their address, lower-case.
    pub email: String,
    /// The key (binary).
    #[serde(with = "hex_bytes")]
    pub public: Vec<u8>,
    pub info: openpgp::KeyInfo,
    pub learnt: Learnt,
    /// When it was last learnt again or used (Unix seconds): the least recent goes first.
    pub last_used: i64,
    /// When the message that brought it was written (or, imported, when it was).
    pub from_date: i64,
    /// The fingerprint of the key it replaced, from mail.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
    /// That change was said (on the message that brought it).
    pub change_shown: bool,
}

/// What learning a key did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Learn {
    /// A first key for that address.
    New,
    /// The key already known (perhaps with newer self-signatures).
    Same,
    /// It replaced the key whose fingerprint this is.
    Replaced { previous: String },
    /// It wasn't taken: an older message's, a key imported by hand stays, or no room.
    Kept,
}

/// An account's correspondents' keys, by lower-case address.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PeopleKeys {
    pub keys: BTreeMap<String, PersonKey>,
}

mod hex_bytes {
    pub(super) fn serialize<S: serde::Serializer>(b: &[u8], to: S) -> Result<S::Ok, S::Error> {
        to.serialize_str(&b.iter().map(|x| format!("{x:02x}")).collect::<String>())
    }

    pub(super) fn deserialize<'de, D: serde::Deserializer<'de>>(from: D) -> Result<Vec<u8>, D::Error> {
        let s = <String as serde::Deserialize>::deserialize(from)?;
        let bad = || <D::Error as serde::de::Error>::custom("not hex");
        if !s.len().is_multiple_of(2) {
            return Err(bad());
        }
        (0..s.len()).step_by(2).map(|i| s.get(i..i + 2).and_then(|p| u8::from_str_radix(p, 16).ok()).ok_or_else(bad)).collect()
    }
}

fn address(email: &str) -> String {
    email.trim().to_lowercase()
}

impl PeopleKeys {
    /// What a file gave back, checked: each key under its own address, none too big, at most
    /// [`PEOPLE_MAX`] (the least used from mail let go).
    pub fn checked(mut self) -> PeopleKeys {
        self.keys.retain(|email, k| *email == address(&k.email) && !email.is_empty() && !k.public.is_empty() && k.public.len() <= KEY_MAX);
        while self.keys.len() > PEOPLE_MAX && self.let_one_go() {}
        self
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// The key kept for `email`.
    pub fn get(&self, email: &str) -> Option<&PersonKey> {
        self.keys.get(&address(email))
    }

    /// `email`'s key was used (a signature checked, mail encrypted to it).
    pub fn used(&mut self, email: &str, now: i64) {
        if let Some(k) = self.keys.get_mut(&address(email)) {
            k.last_used = k.last_used.max(now);
        }
    }

    /// The least recently used key from mail let go; false when there's none.
    fn let_one_go(&mut self) -> bool {
        let least = self.keys.iter().filter(|(_, k)| k.learnt == Learnt::FromMail).min_by_key(|(_, k)| k.last_used).map(|(e, _)| e.clone());
        least.is_some_and(|e| self.keys.remove(&e).is_some())
    }

    /// The key `public` (`info`, what it is) from a message from `email` written at
    /// `message_date`, seen at `now`.
    pub fn learn_from_mail(&mut self, email: &str, public: &[u8], info: &openpgp::KeyInfo, message_date: i64, now: i64) -> Learn {
        let email = address(email);
        if email.is_empty() || public.is_empty() || public.len() > KEY_MAX {
            return Learn::Kept;
        }
        if let Some(k) = self.keys.get_mut(&email) {
            if k.info.fingerprint == info.fingerprint {
                k.last_used = k.last_used.max(now);
                // Newer self-signatures (a later expiry) from a newer message; never over a key
                // imported by hand.
                if k.learnt == Learnt::FromMail && message_date >= k.from_date {
                    k.public = public.to_vec();
                    k.info = info.clone();
                    k.from_date = message_date;
                }
                return Learn::Same;
            }
            if k.learnt == Learnt::ByHand || message_date < k.from_date {
                return Learn::Kept;
            }
            let previous = k.info.fingerprint.clone();
            *k = PersonKey {
                email,
                public: public.to_vec(),
                info: info.clone(),
                learnt: Learnt::FromMail,
                last_used: now,
                from_date: message_date,
                previous: Some(previous.clone()),
                change_shown: false,
            };
            return Learn::Replaced { previous };
        }
        if self.keys.len() >= PEOPLE_MAX && !self.let_one_go() {
            return Learn::Kept;
        }
        let k = PersonKey {
            email: email.clone(),
            public: public.to_vec(),
            info: info.clone(),
            last_used: now,
            from_date: message_date,
            ..Default::default()
        };
        self.keys.insert(email, k);
        Learn::New
    }

    /// The key `public` imported by the user for `email`: theirs from now on, whatever mail says.
    pub fn import_by_hand(&mut self, email: &str, public: &[u8], info: &openpgp::KeyInfo, now: i64) -> Learn {
        let email = address(email);
        if email.is_empty() || public.is_empty() || public.len() > KEY_MAX {
            return Learn::Kept;
        }
        let learn = match self.keys.get(&email) {
            Some(k) if k.info.fingerprint == info.fingerprint => Learn::Same,
            Some(k) => Learn::Replaced { previous: k.info.fingerprint.clone() },
            None => {
                if self.keys.len() >= PEOPLE_MAX {
                    self.let_one_go();
                }
                Learn::New
            }
        };
        let k = PersonKey {
            email: email.clone(),
            public: public.to_vec(),
            info: info.clone(),
            learnt: Learnt::ByHand,
            last_used: now,
            from_date: now,
            previous: None,
            change_shown: true,
        };
        self.keys.insert(email, k);
        learn
    }

    /// `email`'s key forgotten; whether there was one.
    pub fn forget(&mut self, email: &str) -> bool {
        self.keys.remove(&address(email)).is_some()
    }

    /// The fingerprint of the key `email`'s replaced, the first time it's asked (the change is
    /// said once).
    pub fn take_change(&mut self, email: &str) -> Option<String> {
        let k = self.keys.get_mut(&address(email))?;
        if k.change_shown {
            return None;
        }
        k.change_shown = true;
        k.previous.clone()
    }
}

/// A key a message brings: from its `Autocrypt:` header (`addr`, the address it says it's for)
/// or attached (`addr` none: the key's user IDs say).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OfferedKey {
    pub(crate) addr: Option<String>,
    pub(crate) public: Vec<u8>,
    pub(crate) info: openpgp::KeyInfo,
}

/// Whether `needle` (lower-case) is in `hay`, in any case.
fn holds(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w.eq_ignore_ascii_case(needle))
}

/// The keys `raw` brings, read (runs on any thread: nothing of the session's). The header is
/// looked at for `Autocrypt:`; the message is taken apart only when it names an attached key.
pub(crate) fn offered(raw: &[u8]) -> Vec<OfferedKey> {
    let mut out = Vec::new();
    for h in mime::pgp::autocrypt(raw) {
        if let Some(a) = openpgp::autocrypt::parse(&h)
            && let Ok(info) = openpgp::info(&a.keydata)
        {
            out.push(OfferedKey { addr: Some(a.addr), public: a.keydata, info });
        }
    }
    if raw.len() <= KEYS_LOOKED_MAX && holds(raw, b"application/pgp-keys") {
        for part in mime::pgp::key_parts(raw) {
            for public in openpgp::public_keys(&part).unwrap_or_default().into_iter().take(4) {
                if let Ok(info) = openpgp::info(&public) {
                    out.push(OfferedKey { addr: None, public, info });
                }
            }
        }
    }
    out.truncate(OFFERED_MAX);
    out
}

impl Session {
    pub(super) fn load_people(&mut self, account: &str) {
        let Some(store) = self.store.clone() else { return };
        let p: PeopleKeys = store.load_account_doc(account, PEOPLE_DOC).unwrap_or_default();
        self.pgp.people.insert(account.to_string(), p.checked());
    }

    pub(super) fn save_people(&mut self, account: &str) {
        let (Some(store), Some(p)) = (&self.store, self.pgp.people.get(account)) else { return };
        if let Err(e) = store.save_account_doc(account, PEOPLE_DOC, p) {
            self.notice_with("the people's OpenPGP keys could not be saved: {error}", &[("error", &e.to_string())], true);
        }
    }

    /// The account's correspondents' keys, by address.
    pub fn pgp_people(&self, account: &str) -> Vec<PersonKey> {
        self.pgp.people.get(account).map(|p| p.keys.values().cloned().collect()).unwrap_or_default()
    }

    /// The key kept for `email`, for `account`.
    pub fn person_key(&self, account: &str, email: &str) -> Option<&PersonKey> {
        self.pgp.people.get(account).and_then(|p| p.get(email))
    }

    /// Whether `email` is one of the accounts' own addresses (their mail brings no one's key).
    fn own_address(&self, email: &str) -> bool {
        self.accounts.iter().any(|a| a.config.email.trim().eq_ignore_ascii_case(email.trim()))
    }

    /// Learn the keys message `key` brings (found already) for its sender: its `Autocrypt:`
    /// header's for the address it comes from, attached keys made out to that address. Its
    /// sender and date are the list's (nothing is read again here).
    pub(crate) fn learn_offered(&mut self, key: &MessageKey, offered: Vec<OfferedKey>) {
        if offered.is_empty() {
            return;
        }
        let Some((from, date)) = self.summary(key).map(|m| (address(&m.from.email), m.date)) else { return };
        if from.is_empty() || self.own_address(&from) {
            return;
        }
        let now = self.clock.now;
        let people = self.pgp.people.entry(key.account.clone()).or_default();
        let mut changed = false;
        for o in offered {
            let for_sender = match &o.addr {
                Some(addr) => address(addr) == from,
                None => o.info.is_for(&from),
            };
            if for_sender {
                changed |= people.learn_from_mail(&from, &o.public, &o.info, date, now) != Learn::Kept;
            }
        }
        if changed {
            self.save_people(&key.account);
        }
    }

    /// Whether the key learnt for message `key`'s sender changed and it isn't said yet: said now
    /// (true once).
    pub(in crate::session) fn key_change_shown(&mut self, key: &MessageKey) -> bool {
        let Some(from) = self.summary(key).map(|m| address(&m.from.email)) else { return false };
        let changed = self.pgp.people.get_mut(&key.account).and_then(|p| p.take_change(&from)).is_some();
        if changed {
            self.save_people(&key.account);
        }
        changed
    }

    /// Keep the correspondents' keys in the file at `path`.
    pub fn import_public_keys(&mut self, account: &str, path: &str) -> CmdResult {
        let data = crate::cmd::read_capped(path, 8 << 20)?;
        self.import_public_keys_data(account, &data)
    }

    /// Keep the correspondents' keys in `data` (a key file or an attached key, armored or
    /// binary), each for the addresses its user IDs name. Importing is choosing: a key imported
    /// isn't replaced by one that comes in mail.
    pub fn import_public_keys_data(&mut self, account: &str, data: &[u8]) -> CmdResult {
        self.pgp_config(account)?;
        let keys = openpgp::public_keys(data).map_err(|e| CmdError::Params(e.to_string()))?;
        let now = self.clock.now;
        let mut added: Vec<String> = Vec::new();
        for public in keys {
            let Ok(info) = openpgp::info(&public) else { continue };
            for email in info.emails.iter().take(16) {
                if self.own_address(email) {
                    continue;
                }
                let people = self.pgp.people.entry(account.to_string()).or_default();
                if people.import_by_hand(email, &public, &info, now) != Learn::Kept && !added.contains(email) {
                    added.push(email.clone());
                }
            }
        }
        if added.is_empty() {
            return Err(CmdError::Params("there's no key in it for someone's address".into()));
        }
        self.save_people(account);
        self.notice_with("OpenPGP keys kept for {who}", &[("who", &added.join(", "))], false);
        Ok(serde_json::json!({ "added": added }))
    }

    /// Forget the key Quatrelle has for `email`.
    pub fn forget_public_key(&mut self, account: &str, email: &str) -> CmdResult {
        self.pgp_config(account)?;
        let gone = self.pgp.people.get_mut(account).is_some_and(|p| p.forget(email));
        if !gone {
            return Err(CmdError::Params(format!("there's no OpenPGP key for `{}`", address(email))));
        }
        self.save_people(account);
        Ok(serde_json::json!({ "forgotten": address(email) }))
    }
}

#[cfg(test)]
mod tests;
