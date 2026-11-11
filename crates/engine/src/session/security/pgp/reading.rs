// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The session: reading OpenPGP mail (adapted from upstream f32b54b), a step of
//! [`Session::unwrap`] for PGP/MIME and PGP blocks in a text. A signature is checked with the key
//! kept for its sender (and the account's own, for its own mail); one made by a key kept for
//! someone else says it's theirs, not the sender's. Encrypted mail is decrypted by the network
//! layer with the account's keys, asked only when the account has one, and what it held is kept
//! in memory only (as S/MIME's), never in the cache or the search index; mail encrypted to none
//! of the account's keys says so ([`Lock::NoKey`]).

use quatrelle_model::job::{NOT_FOR_YOU, PgpSignature};

use super::*;

/// What a step of taking a message's protection off came to.
pub(crate) enum Step {
    /// It isn't OpenPGP.
    Not,
    /// What it shows with this layer taken off.
    Next(Vec<u8>),
    /// It's OpenPGP, and nothing more can be taken off (yet).
    Stop,
}

/// A day: a key's use is written down at most this often (the file of people's keys is
/// written whole).
const USE_KEPT_EVERY: i64 = 86_400;

/// The most a PGP block in a text may hold: it's opened where the message is shown, and a few
/// kilobytes can decompress to a thousand times as much. As much as a text is shown up to.
const INLINE_MAX: usize = 4 << 20;

/// What a check says, as the network layer passes it.
fn as_signature(c: openpgp::Check) -> PgpSignature {
    match c {
        openpgp::Check::Good { key, at, weak, until } => PgpSignature::Good { key, at, weak, until },
        openpgp::Check::Bad => PgpSignature::Bad,
        openpgp::Check::Unknown(key_id) => PgpSignature::Unknown { key_id },
    }
}

fn broken(why: String) -> SignedBy {
    SignedBy { seal: Seal::Broken(why), cert: None, key: None, learnt: None, at: None, weak: false, revocation: None }
}

/// What a check made here gives back: what it read (the text, for a signed text), and what its
/// signature says (`None`: not signed).
type Checked<T> = Result<(T, Option<openpgp::Check>), openpgp::Error>;

impl Session {
    /// The keys a signature from `from` may be checked with: the key kept for them, and the
    /// account's own when it's the account's own mail.
    fn keys_for(&self, account: &str, from: &str) -> Vec<Vec<u8>> {
        let mut out: Vec<Vec<u8>> = self.person_key(account, from).map(|k| k.public.clone()).into_iter().collect();
        if self.account(account).is_some_and(|a| a.config.email.trim().eq_ignore_ascii_case(from.trim())) {
            out.extend(self.pgp.accounts.get(account).into_iter().flat_map(|p| p.keys.iter().map(|k| k.public.clone())));
        }
        out
    }

    /// The key kept for someone whose key ID is `key_id` (what a signature names): theirs and
    /// their address.
    fn kept_key_by_id(&self, account: &str, key_id: &str) -> Option<(String, Vec<u8>)> {
        let id = key_id.trim();
        if id.is_empty() {
            return None;
        }
        self.pgp.people.get(account)?.keys.values().find(|k| k.info.key_id.eq_ignore_ascii_case(id)).map(|k| (k.email.clone(), k.public.clone()))
    }

    /// What a signature says, as the reading pane shows it: it holds, by a key for the sender
    /// valid now, or with what's wrong with its key; or it doesn't hold, or can't be checked.
    /// `keys` are those it was checked with, `owner` the address they were kept for: how the key
    /// came is said, and its use written down.
    pub(super) fn pgp_signed_by(&mut self, account: &str, sig: &PgpSignature, keys: &[Vec<u8>], from: &str, owner: &str) -> SignedBy {
        match sig {
            PgpSignature::Good { key, at, weak, until } => {
                let info = keys.get(*key).and_then(|k| openpgp::info(k).ok());
                let mut problems = Vec::new();
                let mut learnt = None;
                if let Some(i) = &info {
                    for p in i.problems(self.clock.now) {
                        problems.push(match p {
                            openpgp::Problem::Revoked(t) => smime::Problem::Revoked(t),
                            openpgp::Problem::Expired(t) => smime::Problem::Expired(t),
                            openpgp::Problem::NotYetValid(t) => smime::Problem::NotYetValid(t),
                            openpgp::Problem::NoEncryption => continue,
                        });
                    }
                    // The part of the key that signed (a subkey of its own end), or the
                    // signature itself, ran out: before now, or before it was made.
                    if let Some(end) = until.filter(|end| *end <= self.clock.now || at.is_some_and(|at| at > *end))
                        && !problems.iter().any(|p| matches!(p, smime::Problem::Expired(_)))
                    {
                        problems.push(smime::Problem::Expired(end));
                    }
                    if !from.trim().is_empty() && !i.is_for(from) {
                        problems.push(smime::Problem::OtherAddress(i.emails.clone()));
                    }
                    learnt = self.key_used(account, owner, &i.fingerprint);
                }
                SignedBy {
                    seal: if problems.is_empty() { Seal::Trusted } else { Seal::Untrusted(problems) },
                    cert: None,
                    key: info,
                    learnt,
                    at: *at,
                    weak: *weak,
                    revocation: None,
                }
            }
            PgpSignature::Bad => broken("the message was changed after it was signed, or the signature isn't the sender's".into()),
            PgpSignature::Unknown { key_id } => {
                SignedBy { seal: Seal::UnknownKey(key_id.clone()), cert: None, key: None, learnt: None, at: None, weak: false, revocation: None }
            }
        }
    }

    /// `owner`'s key (`fingerprint`) checked a signature (or was encrypted to): how it came, and its use written down
    /// (a day at most), the least used going first. `None`: not a correspondent's (the account's
    /// own).
    pub(super) fn key_used(&mut self, account: &str, owner: &str, fingerprint: &str) -> Option<Learnt> {
        let now = self.clock.now;
        let people = self.pgp.people.get_mut(account)?;
        let k = people.get(owner).filter(|k| k.info.fingerprint == fingerprint)?;
        let (learnt, stale) = (k.learnt, k.last_used.saturating_add(USE_KEPT_EVERY) <= now);
        people.used(owner, now);
        if stale {
            self.save_people(account);
        }
        Some(learnt)
    }

    /// A signature checked here with `check` (given the keys to check it with): with the
    /// sender's key, and, when it was made by a key kept for someone else, with that one (it's
    /// then said to be theirs, not the sender's).
    fn check_pgp<T>(&mut self, account: &str, from: &str, check: impl Fn(&[&[u8]]) -> Checked<T>) -> Result<(T, Option<SignedBy>), openpgp::Error> {
        let keys = self.keys_for(account, from);
        let refs: Vec<&[u8]> = keys.iter().map(Vec::as_slice).collect();
        let (read, said) = check(&refs)?;
        if let Some(openpgp::Check::Unknown(id)) = &said
            && let Some((owner, theirs)) = self.kept_key_by_id(account, id)
            && let Ok((again, Some(c @ (openpgp::Check::Good { .. } | openpgp::Check::Bad)))) = check(&[theirs.as_slice()])
        {
            let by = self.pgp_signed_by(account, &as_signature(c), std::slice::from_ref(&theirs), from, &owner);
            return Ok((again, Some(by)));
        }
        let by = said.map(|c| self.pgp_signed_by(account, &as_signature(c), &keys, from, from));
        Ok((read, by))
    }

    /// One layer of OpenPGP taken off `cur` (message `key`), what it was said in `sec`.
    pub(crate) fn unwrap_pgp(&mut self, key: &MessageKey, cur: &[u8], sec: &mut Security) -> Step {
        let from = sender(cur);
        let account = key.account.clone();
        match mime::pgp::protection(cur) {
            mime::pgp::Protection::None => Step::Not,
            mime::pgp::Protection::Signed { content, signature } => {
                sec.pgp = true;
                if sec.signed.is_none() {
                    let done = self.check_pgp(&account, &from, |keys| openpgp::verify(&content, &signature, keys).map(|c| ((), Some(c))));
                    sec.signed = Some(match done {
                        Ok((_, by)) => by.unwrap_or_else(|| broken("the signature can't be read".into())),
                        Err(e) => broken(e.to_string()),
                    });
                }
                let (next, headers) = mime::smime::with_entity_protected(cur, &content);
                sec.headers |= headers;
                Step::Next(next)
            }
            mime::pgp::Protection::Encrypted(data) => {
                sec.pgp = true;
                self.open_encrypted(key, cur, data, ("", false), &from, sec)
            }
            mime::pgp::Protection::Inline { body, charset } => match openpgp::inline(&body) {
                None => Step::Not,
                Some(openpgp::Inline::Signed(block)) => {
                    sec.pgp = true;
                    match self.check_pgp(&account, &from, |keys| openpgp::verify_cleartext(block, keys).map(|(t, c)| (t, Some(c)))) {
                        Ok((text, by)) => {
                            if sec.signed.is_none() {
                                sec.signed = by;
                            }
                            Step::Next(mime::smime::with_entity(cur, &mime::pgp::text_entity(text.as_bytes(), &charset)))
                        }
                        Err(e) => {
                            sec.signed.get_or_insert_with(|| broken(e.to_string()));
                            Step::Stop
                        }
                    }
                }
                Some(openpgp::Inline::Message(block)) if openpgp::is_encrypted(block) => {
                    sec.pgp = true;
                    self.open_encrypted(key, cur, block.to_vec(), (&charset, true), &from, sec)
                }
                Some(openpgp::Inline::Message(block)) => {
                    sec.pgp = true;
                    match self.check_pgp(&account, &from, |keys| openpgp::open_within(block, keys, INLINE_MAX).map(|d| (d.data, d.check))) {
                        Ok((data, by)) => {
                            if sec.signed.is_none() {
                                sec.signed = by;
                            }
                            Step::Next(mime::smime::with_entity(cur, &mime::pgp::text_entity(&data, &charset)))
                        }
                        Err(e) => {
                            sec.signed.get_or_insert_with(|| broken(e.to_string()));
                            Step::Stop
                        }
                    }
                }
            },
        }
    }

    /// An encrypted message (`data`; `inline`: a PGP block in a text in `charset`): what it held
    /// when it's been decrypted, else asked for.
    fn open_encrypted(
        &mut self,
        key: &MessageKey,
        cur: &[u8],
        data: Vec<u8>,
        (charset, inline): (&str, bool),
        from: &str,
        sec: &mut Security,
    ) -> Step {
        // One encryption is decrypted; another inside it stays as it is.
        if sec.lock.is_some() {
            return Step::Stop;
        }
        if let Some((entity, _)) = self.opened.get(key) {
            // OpenPGP's encryption (with its modification detection) tells whether it was changed.
            sec.lock = Some(Lock::Open { authenticated: true });
            if sec.signed.is_none() {
                sec.signed = self.pgp.signed.get(key).cloned();
            }
            let (next, headers) = mime::smime::with_entity_protected(cur, entity);
            sec.headers |= headers;
            return Step::Next(next);
        }
        sec.lock = Some(match self.decrypt_errors.get(key) {
            Some(e) if e.starts_with(NOT_FOR_YOU) => Lock::NoKey(openpgp::recipients(&data)),
            Some(e) => Lock::Failed(e.clone()),
            None => self.ask_pgp_decrypt(key, data, inline, charset, from),
        });
        Step::Stop
    }

    /// Asked of the network layer, once, for an account that has a key: else it's encrypted to
    /// a key Quatrelle doesn't have.
    fn ask_pgp_decrypt(&mut self, key: &MessageKey, data: Vec<u8>, inline: bool, charset: &str, from: &str) -> Lock {
        let Some(config) = self.account(&key.account).map(|a| a.config.clone()) else { return Lock::Failed("its account is gone".into()) };
        if self.pgp.accounts.get(&key.account).is_none_or(|p| p.keys.is_empty()) {
            return Lock::NoKey(openpgp::recipients(&data));
        }
        if self.opening.insert(key.clone()) {
            let verify_with = self.keys_for(&key.account, from);
            self.pgp.asked.insert(key.clone(), (verify_with.clone(), from.to_string()));
            let work = PgpWork::Decrypt { key: key.clone(), data, inline, charset: charset.to_string(), verify_with };
            self.jobs.push(Job::Pgp { account: config, work });
        }
        Lock::Opening
    }

    /// The network layer's answer: what message `key` held, kept in memory with what its
    /// signature says; the keys attached inside learnt as those outside are. Shown again.
    pub(super) fn heard_pgp_decrypted(
        &mut self,
        key: MessageKey,
        content: Option<Private>,
        signature: Option<PgpSignature>,
        keys: Vec<Vec<u8>>,
        error: Option<String>,
    ) {
        self.opening.remove(&key);
        let (verify_with, from) = self.pgp.asked.remove(&key).unwrap_or_default();
        match (content, error) {
            (Some(Private(entity)), None) => {
                if self.opened.len() >= OPENED_MAX
                    && let Some(k) = self.opened.keys().next().cloned()
                {
                    self.opened.remove(&k);
                    self.pgp.signed.remove(&k);
                }
                self.opened.insert(key.clone(), (entity, true));
                match signature {
                    Some(sig) => {
                        let by = self.pgp_signed_by(&key.account, &sig, &verify_with, &from, &from);
                        self.pgp.signed.insert(key.clone(), by);
                    }
                    None => {
                        self.pgp.signed.remove(&key);
                    }
                }
                let offered: Vec<OfferedKey> = keys
                    .into_iter()
                    .take(people::OFFERED_MAX)
                    .filter_map(|public| openpgp::info(&public).ok().map(|info| OfferedKey { addr: None, public, info }))
                    .collect();
                self.learn_offered(&key, offered);
            }
            (_, e) => {
                self.decrypt_errors.insert(key.clone(), e.unwrap_or_else(|| "it can't be decrypted".into()));
            }
        }
        self.show_again(&key);
    }

    /// A layer of OpenPGP taken off as [`Session::readable_raw`] reads: nothing checked again.
    pub(crate) fn readable_pgp(&self, key: &MessageKey, cur: &[u8]) -> Option<Vec<u8>> {
        match mime::pgp::protection(cur) {
            mime::pgp::Protection::None => None,
            mime::pgp::Protection::Signed { content, .. } => Some(mime::smime::with_entity(cur, &content)),
            mime::pgp::Protection::Encrypted(_) => self.opened.get(key).map(|(entity, _)| mime::smime::with_entity(cur, entity)),
            mime::pgp::Protection::Inline { body, charset } => match openpgp::inline(&body)? {
                openpgp::Inline::Signed(block) => {
                    let (text, _) = openpgp::verify_cleartext(block, &[]).ok()?;
                    Some(mime::smime::with_entity(cur, &mime::pgp::text_entity(text.as_bytes(), &charset)))
                }
                openpgp::Inline::Message(block) if openpgp::is_encrypted(block) => {
                    self.opened.get(key).map(|(entity, _)| mime::smime::with_entity(cur, entity))
                }
                openpgp::Inline::Message(block) => {
                    let d = openpgp::open_within(block, &[], INLINE_MAX).ok()?;
                    Some(mime::smime::with_entity(cur, &mime::pgp::text_entity(&d.data, &charset)))
                }
            },
        }
    }

    /// Messages shown that couldn't be decrypted for want of a key of the account's, shown
    /// again now that it has one.
    pub(super) fn show_again_without_key(&mut self, account: &str) {
        let locked = |r: &Reading| r.key.account == account && matches!(r.security.as_ref().and_then(|s| s.lock.as_ref()), Some(Lock::NoKey(_)));
        let keys: Vec<MessageKey> = self.reading.iter().chain(&self.windows).filter(|r| locked(r)).map(|r| r.key.clone()).collect();
        for k in keys {
            self.show_again(&k);
        }
    }
}
