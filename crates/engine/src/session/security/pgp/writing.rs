// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The session: writing with OpenPGP (adapted from upstream f32b54b). Which protection a message
//! gets when the account has both a certificate and a key ([`Scheme`]), the job that signs and
//! encrypts it as PGP/MIME, the account's public key sent along (Autocrypt) or attached, and
//! whether the message being written can go encrypted to everyone in it ([`Reach`]), encryption
//! following that when the option asks. Nothing of it happens for an account without a key of its
//! own ([`Session::pgp_active`]).

use quatrelle_model::job::PgpWork;

use super::*;

/// What a refusal to encrypt says, before the recipients it names (`: a@x, b@y`): none of them
/// has a key Quatrelle can encrypt to (none known, run out, or withdrawn).
pub const NO_KEY_TO_ENCRYPT: &str = "there's no usable OpenPGP key to encrypt to";

/// What an attached public key is.
const PGP_KEYS: &str = "application/pgp-keys";

/// Which protection a message being written gets, when its account has both a certificate and
/// an OpenPGP key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Scheme {
    /// The one every recipient can receive (S/MIME when both or neither reach them all); a
    /// reply, the one the message it answers came with.
    #[default]
    Auto,
    Smime,
    Pgp,
}

impl Scheme {
    pub fn is_auto(&self) -> bool {
        *self == Scheme::Auto
    }
}

/// Whether the message being written can go encrypted to everyone in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reach {
    /// What it would be encrypted with ([`Scheme::Smime`] or [`Scheme::Pgp`]).
    pub scheme: Scheme,
    /// Its recipients who can't receive it so: no certificate or key known for them that can be
    /// encrypted to now (an OpenPGP key run out or withdrawn doesn't count).
    pub missing: Vec<String>,
    /// How many different recipients it has.
    pub recipients: usize,
}

impl Reach {
    /// It has recipients, and every one can receive it encrypted.
    pub fn everyone(&self) -> bool {
        self.recipients > 0 && self.missing.is_empty()
    }
}

/// A digest of what a [`Reach`] is worked out from, to work it out again only when that changes.
fn digest(parts: &[&[u8]]) -> u64 {
    parts.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, p| {
        let h = p.iter().fold(h, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3));
        (h ^ 0xff).wrapping_mul(0x0100_0000_01b3)
    })
}

impl Session {
    /// The addresses a message being written goes to (its To, Cc and Bcc, as far as they read),
    /// lower-case, each once.
    pub(crate) fn compose_recipients(c: &Compose) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for field in [&c.to, &c.cc, &c.bcc] {
            for a in addr::split_entries(field).into_iter().filter_map(addr::parse_one) {
                let email = a.email.trim().to_lowercase();
                if email.contains('@') && !out.contains(&email) && out.len() < 1000 {
                    out.push(email);
                }
            }
        }
        out
    }

    /// The key to encrypt to `email` with, when one can be encrypted to now: the account's own
    /// for its own address, else the one kept for them.
    fn pgp_key_for(&self, account: &str, email: &str) -> Option<(Vec<u8>, String)> {
        let email = email.trim().to_lowercase();
        let now = self.clock.now;
        if self.account(account).is_some_and(|a| a.config.email.trim().eq_ignore_ascii_case(&email)) {
            return self.pgp_key(account).filter(|k| k.info.encryptable(now).is_ok()).map(|k| (k.public.clone(), k.info.fingerprint.clone()));
        }
        let k = self.person_key(account, &email)?;
        k.info.encryptable(now).is_ok().then(|| (k.public.clone(), k.info.fingerprint.clone()))
    }

    /// The recipients of `recipients` that can't receive mail encrypted with `scheme`.
    fn unreachable(&self, account: &str, recipients: &[String], scheme: Scheme) -> Vec<String> {
        recipients
            .iter()
            .filter(|r| match scheme {
                Scheme::Smime => self.cert_for(account, r).is_none(),
                Scheme::Pgp => self.pgp_key_for(account, r).is_none(),
                Scheme::Auto => true,
            })
            .cloned()
            .collect()
    }

    /// Which protection `c` gets: `None` when its account has neither a certificate nor a key of
    /// its own; the one it has, when it has one; with both, the one chosen, else (Automatic) the
    /// one the message it answers came with, else the one every recipient can receive (S/MIME
    /// when both or neither reach them all).
    pub fn scheme_for(&self, c: &Compose) -> Option<Scheme> {
        let smime = self.identity(&c.account).is_some();
        let pgp = self.pgp_active(&c.account);
        Some(match (smime, pgp) {
            (false, false) => return None,
            (true, false) => Scheme::Smime,
            (false, true) => Scheme::Pgp,
            (true, true) => match (c.scheme, c.came_as) {
                (Scheme::Smime, _) | (Scheme::Auto, Some(Scheme::Smime)) => Scheme::Smime,
                (Scheme::Pgp, _) | (Scheme::Auto, Some(Scheme::Pgp)) => Scheme::Pgp,
                (Scheme::Auto, _) => {
                    let recipients = Self::compose_recipients(c);
                    let s = self.unreachable(&c.account, &recipients, Scheme::Smime).len();
                    let p = self.unreachable(&c.account, &recipients, Scheme::Pgp).len();
                    if p < s { Scheme::Pgp } else { Scheme::Smime }
                }
            },
        })
    }

    /// Before `c` goes with OpenPGP: the job to sign and/or encrypt `raw`, to every recipient's
    /// key and the account's own (the Sent copy reads too). A recipient without a key that can be
    /// encrypted to now is named, and nothing goes: never unencrypted unless encryption is off.
    pub(crate) fn pgp_protect_job(&mut self, c: &Compose, raw: &[u8], recipients: &[String], from: &str) -> Result<Job, CmdError> {
        let config = self.account(&c.account).map(|a| a.config.clone()).ok_or_else(|| CmdError::Failed("the sending account is gone".into()))?;
        let Some(own) = self.pgp_key(&c.account).cloned() else {
            return Err(CmdError::Disabled(
                "there's no OpenPGP key for this account to sign or encrypt with: make one in File › Info › Security".into(),
            ));
        };
        let now = self.clock.now;
        if let Some(p) = own.info.problems(now).into_iter().find(|p| !matches!(p, openpgp::Problem::NoEncryption)) {
            let why = match p {
                openpgp::Problem::Revoked(_) => "was withdrawn".to_string(),
                openpgp::Problem::Expired(at) => format!("ran out {}", quatrelle_model::dates::full_time(at, &self.clock)),
                openpgp::Problem::NotYetValid(at) => format!("isn't valid until {}", quatrelle_model::dates::full_time(at, &self.clock)),
                openpgp::Problem::NoEncryption => String::new(),
            };
            return Err(CmdError::Params(format!(
                "your OpenPGP key {why}: extend or replace it in File › Info › Security, or send without signing or encryption"
            )));
        }
        if c.sign && !own.info.signs {
            return Err(CmdError::Params("your OpenPGP key can't sign: send it without signing, or import a key that can".into()));
        }
        let mut encrypt_to: Vec<Vec<u8>> = Vec::new();
        if c.encrypt {
            let missing = self.unreachable(&c.account, recipients, Scheme::Pgp);
            if !missing.is_empty() {
                return Err(CmdError::Disabled(format!("{NO_KEY_TO_ENCRYPT}: {}", missing.join(", "))));
            }
            let mut used = Vec::new();
            for r in recipients {
                if let Some((k, fingerprint)) = self.pgp_key_for(&c.account, r) {
                    used.push((r.clone(), fingerprint));
                    if !encrypt_to.contains(&k) {
                        encrypt_to.push(k);
                    }
                }
            }
            // The Sent copy reads too.
            if own.info.encryptable(now).is_ok() && !encrypt_to.contains(&own.public) {
                encrypt_to.push(own.public.clone());
            }
            // Encrypting to someone is using their key: the least used goes first, not theirs.
            for (r, fingerprint) in used {
                self.key_used(&c.account, &r, &fingerprint);
            }
        }
        let hide_subject = !self.smime_settings(&c.account).keep_subject;
        let work = PgpWork::Protect { id: c.id, raw: Private(raw.to_vec()), sign: c.sign, encrypt_to, hide_subject };
        self.protecting.insert(c.id, Protecting { compose: c.clone(), from: from.to_string(), recipients: recipients.to_vec() });
        Ok(Job::Pgp { account: config, work })
    }

    /// Whether `c`'s draft is encrypted to the account's OpenPGP key before it's filed (it's to
    /// be encrypted with OpenPGP).
    pub(crate) fn seals_with_pgp(&self, c: &Compose) -> bool {
        c.encrypt && self.scheme_for(c) == Some(Scheme::Pgp)
    }

    /// `raw` (`c`, about to go) with the account's key in an `Autocrypt:` header (no
    /// `prefer-encrypt`), so whoever answers can encrypt to it: only when the account has a key
    /// that can be encrypted to, sends it along (`pgp.options`), and the message is from the
    /// account's own address (Autocrypt's key is for the From address).
    pub(crate) fn with_autocrypt(&self, c: &Compose, raw: Vec<u8>) -> Vec<u8> {
        if !self.pgp_active(&c.account) || !self.pgp_settings(&c.account).autocrypt {
            return raw;
        }
        let (Some(own), Some(a)) = (self.pgp_key(&c.account), self.account(&c.account)) else { return raw };
        let email = a.config.email.trim().to_string();
        let from = self.compose_from(c).email;
        if email.is_empty() || !from.trim().eq_ignore_ascii_case(&email) || own.info.encryptable(self.clock.now).is_err() {
            return raw;
        }
        let Ok(small) = openpgp::minimal(&own.public, &email) else { return raw };
        [openpgp::autocrypt::header(&email, &small, false).as_bytes(), &raw].concat()
    }

    /// The account's public key attached to the message being written, once (`<address>.asc`,
    /// `application/pgp-keys`), for its recipients to import.
    pub fn attach_public_key(&mut self) -> CmdResult {
        let c = self.compose.as_ref().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
        let account = c.account.clone();
        let own =
            self.pgp_key(&account).ok_or_else(|| CmdError::Disabled("this account has no OpenPGP key: make one in File › Info › Security".into()))?;
        let armored = openpgp::armored_public(&own.public).map_err(|e| CmdError::Failed(e.to_string()))?;
        let email = self.account(&account).map(|a| a.config.email.trim().to_string()).unwrap_or_default();
        let name = format!("{}.asc", if email.is_empty() { "key" } else { email.as_str() });
        let c = self.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
        let has = c.attachments.iter().any(|a| a.name == name && a.mime == PGP_KEYS);
        if !has {
            c.attachments.push(OutAttachment { name: name.clone(), mime: PGP_KEYS.into(), data: armored.into_bytes(), ..Default::default() });
        }
        Ok(serde_json::json!({ "attached": name, "already": has }))
    }

    /// A new message: encryption follows its recipients when the option asks it to (File ›
    /// Options › Safety) and the account can encrypt, unless it's on already.
    pub(in crate::session) fn encryption_follows(&self, c: &mut Compose) {
        c.encrypt_auto = self.prefs.encrypt_when_possible && !c.encrypt && (self.identity(&c.account).is_some() || self.pgp_active(&c.account));
    }

    /// Whether the message being written can go encrypted to everyone in it, and if not who
    /// can't; `None` when nothing is being written or its account can't encrypt at all. Worked
    /// out again only when its recipients, its protection or the keys known change (or the hour
    /// does: keys run out). While encryption follows the recipients, it's turned on and off to
    /// match.
    pub fn encryption_reach(&mut self) -> Option<Reach> {
        let c = self.compose.as_ref()?;
        let scheme = self.scheme_for(c)?;
        let count = |n: usize| (n as u64).to_le_bytes();
        let smime_known = self.smime.get(&c.account).map_or(0, |s| s.known.len() + s.chain.len());
        let pgp_known = self.pgp.people.get(&c.account).map_or(0, PeopleKeys::len) + self.pgp.accounts.get(&c.account).map_or(0, |p| p.keys.len());
        let d = digest(&[
            &c.id.to_le_bytes(),
            c.account.as_bytes(),
            c.to.as_bytes(),
            c.cc.as_bytes(),
            c.bcc.as_bytes(),
            format!("{scheme:?}").as_bytes(),
            &count(smime_known),
            &count(pgp_known),
            &self.clock.now.div_euclid(3600).to_le_bytes(),
        ]);
        let reach = match &self.pgp.reach {
            Some((was, r)) if *was == d => r.clone(),
            _ => {
                let recipients = Self::compose_recipients(c);
                let r = Reach { scheme, missing: self.unreachable(&c.account, &recipients, scheme), recipients: recipients.len() };
                self.pgp.reach = Some((d, r.clone()));
                r
            }
        };
        if let Some(c) = self.compose.as_mut()
            && c.encrypt_auto
        {
            c.encrypt = reach.everyone();
        }
        Some(reach)
    }
}
