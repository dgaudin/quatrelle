// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! An account's own key, made here: the form GnuPG 2.2+, Thunderbird and Proton read, for three
//! years; extended for three more; backed up behind a passphrase as GnuPG reads it.
//!
//! The key is OpenPGP version 4: an EdDSA-legacy Ed25519 primary key (algorithm 22) that certifies
//! and signs, and an ECDH Curve25519-legacy subkey (algorithm 18) that encrypts. Never version 6
//! nor RFC 9580's new algorithms (25, 27): GnuPG 2.4 can't read them.

use pgp::composed::{EncryptionCaps, KeyType, SecretKeyParamsBuilder, SignedSecretKey, SubkeyParamsBuilder};
use pgp::crypto::ecc_curve::ECCCurve;
use pgp::crypto::hash::HashAlgorithm;
use pgp::crypto::sym::SymmetricKeyAlgorithm;
use pgp::packet::{PublicKey, PublicSubkey, SecretKey, Signature, SignatureConfig, SignatureType, Subpacket, SubpacketData};
use pgp::ser::Serialize as _;
use pgp::types::{CompressionAlgorithm, Duration, KeyDetails as _, KeyVersion, Password, Tag};
use rand_core::{CryptoRng, RngCore};

use crate::keys::{Unlocked, analyse, email_of};
use crate::message::secret;
use crate::{Error, guard, stamp};

/// How long a key made or extended here is valid: three years (1,095 days), on the primary key
/// and its subkey.
pub const VALID_FOR_SECS: u64 = 94_608_000;

/// How long before its end a key is said to be running out: 30 days.
pub const EXPIRY_WARNING_SECS: i64 = 2_592_000;

/// The most user IDs and subkeys re-signed in one key.
const PARTS_MAX: usize = 64;

/// The user ID for `name` and `email`: `Name <address>`, or `<address>` without a name. The name
/// loses what could end it early (angle brackets, line breaks and other controls).
fn user_id(name: &str, email: &str) -> String {
    let clean: String = name.chars().map(|c| if c.is_control() || c == '<' || c == '>' { ' ' } else { c }).collect();
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.is_empty() { format!("<{email}>") } else { format!("{clean} <{email}>") }
}

/// `key` as an [`Unlocked`] key: binary, unprotected, and what it is.
fn unlocked(key: &SignedSecretKey) -> Result<Unlocked, Error> {
    let fail = |_| Error::Failed("the key can't be written".into());
    let public = key.to_public_key();
    let info = analyse(&public).info;
    Ok(Unlocked { secret: key.to_bytes().map_err(fail)?, public: public.to_bytes().map_err(fail)?, info })
}

/// A new key pair for `name` and `email`, made at `now` and valid for [`VALID_FOR_SECS`], its
/// secret part unprotected (for the system keychain).
pub fn generate(name: &str, email: &str, now: i64, rng: &mut (impl RngCore + CryptoRng)) -> Result<Unlocked, Error> {
    let email = email.trim();
    if email_of(email).is_none_or(|e| !e.eq_ignore_ascii_case(email)) || email.contains(['<', '>', '"']) {
        return Err(Error::Failed("the address isn't one a key can be made for".into()));
    }
    let uid = user_id(name, email);
    let fail = |_| Error::Failed("the key can't be made".into());
    guard(Error::Failed("the key can't be made".into()), || {
        let sub = SubkeyParamsBuilder::default()
            .version(KeyVersion::V4)
            .key_type(KeyType::ECDH(ECCCurve::Curve25519Legacy))
            .can_encrypt(EncryptionCaps::All)
            .created_at(stamp(now))
            .build()
            .map_err(|_| Error::Failed("the key can't be made".into()))?;
        let params = SecretKeyParamsBuilder::default()
            .version(KeyVersion::V4)
            .key_type(KeyType::Ed25519Legacy)
            .can_certify(true)
            .can_sign(true)
            .created_at(stamp(now))
            .primary_user_id(uid)
            .preferred_symmetric_algorithms([SymmetricKeyAlgorithm::AES256].into_iter().collect())
            .preferred_hash_algorithms([HashAlgorithm::Sha256].into_iter().collect())
            .preferred_compression_algorithms([CompressionAlgorithm::Uncompressed, CompressionAlgorithm::ZLIB].into_iter().collect())
            .subkeys(vec![sub])
            .build()
            .map_err(|_| Error::Failed("the key can't be made".into()))?;
        // rPGP signs what it makes with the system's time and no end: those self-signatures are
        // replaced by ours, stamped `now`.
        let key = params.generate(&mut *rng).map_err(fail)?;
        let key = resign(key, now, rng)?;
        unlocked(&key)
    })
}

/// `key` (unlocked) with new self-signatures, made at `now` and valid for [`VALID_FOR_SECS`]
/// from then: the same key (fingerprint and key material), its end moved.
pub fn extend(key: &Unlocked, now: i64, rng: &mut (impl RngCore + CryptoRng)) -> Result<Unlocked, Error> {
    let k = secret(&key.secret)?;
    guard(Error::Failed("the key can't be extended".into()), || unlocked(&resign(k, now, rng)?))
}

/// When signatures were made.
fn made_at(s: &Signature) -> i64 {
    s.created().map(|t| i64::from(t.as_secs())).unwrap_or(0)
}

/// What a new self-signature made at `now` says: its time, maker and the key's end (`end`,
/// counted from `made`, when the key or subkey was made), then what `old` said besides.
fn config(
    old: &Signature,
    primary: &SecretKey,
    now: i64,
    made: i64,
    end: i64,
    rng: &mut (impl RngCore + CryptoRng),
) -> Result<SignatureConfig, Error> {
    let fail = |_| Error::Failed("the key can't be signed".into());
    let typ = old.typ().ok_or(Error::Failed("the key can't be signed".into()))?;
    let mut c = SignatureConfig::from_key(&mut *rng, primary, typ).map_err(fail)?;
    let lasts = u32::try_from(end.saturating_sub(made).max(1)).unwrap_or(u32::MAX);
    c.hashed_subpackets = vec![
        Subpacket::regular(SubpacketData::SignatureCreationTime(stamp(now))).map_err(fail)?,
        Subpacket::regular(SubpacketData::IssuerFingerprint(primary.fingerprint())).map_err(fail)?,
        Subpacket::regular(SubpacketData::KeyExpirationTime(Duration::from_secs(lasts))).map_err(fail)?,
    ];
    let renewed = |d: &SubpacketData| {
        matches!(
            d,
            SubpacketData::SignatureCreationTime(_)
                | SubpacketData::SignatureExpirationTime(_)
                | SubpacketData::KeyExpirationTime(_)
                | SubpacketData::IssuerFingerprint(_)
                | SubpacketData::IssuerKeyId(_)
        )
    };
    if let Some(was) = old.config() {
        c.hashed_subpackets.extend(was.hashed_subpackets.iter().filter(|p| !renewed(&p.data)).cloned());
        c.unhashed_subpackets = was.unhashed_subpackets.iter().filter(|p| matches!(p.data, SubpacketData::EmbeddedSignature(_))).cloned().collect();
    }
    if primary.version() <= KeyVersion::V4 {
        c.unhashed_subpackets.insert(0, Subpacket::regular(SubpacketData::IssuerKeyId(primary.legacy_key_id())).map_err(fail)?);
    }
    Ok(c)
}

/// `key`'s self-signatures (each user ID's, each subkey's binding) made again at `now`, ending
/// [`VALID_FOR_SECS`] later. A user ID or subkey its owner withdrew stays as it is; signatures by
/// other keys stay beside the new ones.
fn resign(mut key: SignedSecretKey, now: i64, rng: &mut (impl RngCore + CryptoRng)) -> Result<SignedSecretKey, Error> {
    let fail = |_| Error::Failed("the key can't be signed".into());
    let end = now.saturating_add(i64::try_from(VALID_FOR_SECS).unwrap_or(i64::MAX));
    let primary = key.primary_key.clone();
    let primary_pub: PublicKey = primary.public_key().clone();
    let made = i64::from(primary_pub.created_at().as_secs());
    let none = Password::empty();
    let mut signed = 0;
    for user in key.details.users.iter_mut().take(PARTS_MAX) {
        let mine = |s: &Signature| s.is_certification() && s.verify_certification(&primary_pub, Tag::UserId, &user.id).is_ok();
        let Some(latest) = user.signatures.iter().filter(|s| mine(s)).max_by_key(|s| made_at(s)) else { continue };
        if latest.typ() == Some(SignatureType::CertRevocation) {
            continue;
        }
        let sig =
            config(latest, &primary, now, made, end, rng)?.sign_certification(&primary, &primary_pub, &none, Tag::UserId, &user.id).map_err(fail)?;
        let id = user.id.clone();
        user.signatures.retain(|s| !(s.is_certification() && s.verify_certification(&primary_pub, Tag::UserId, &id).is_ok()));
        user.signatures.push(sig);
        signed += 1;
    }
    if signed == 0 {
        return Err(Error::Failed("the key has no user ID of its own".into()));
    }
    let bind = |signatures: &mut Vec<Signature>, sub: &PublicSubkey, rng: &mut _| -> Result<(), Error> {
        let mine = |s: &Signature| s.verify_subkey_binding(&primary_pub, sub).is_ok();
        if signatures.iter().any(|s| s.typ() == Some(SignatureType::SubkeyRevocation) && mine(s)) {
            return Ok(());
        }
        let Some(latest) = signatures.iter().filter(|s| s.typ() == Some(SignatureType::SubkeyBinding) && mine(s)).max_by_key(|s| made_at(s)) else {
            return Ok(());
        };
        let sub_made = i64::from(sub.created_at().as_secs());
        let sig = config(latest, &primary, now, sub_made, end, rng)?.sign_subkey_binding(&primary, &primary_pub, &none, sub).map_err(fail)?;
        signatures.retain(|s| !(s.typ() == Some(SignatureType::SubkeyBinding) && mine(s)));
        signatures.push(sig);
        Ok(())
    };
    for sub in key.secret_subkeys.iter_mut().take(PARTS_MAX) {
        let public = sub.key.public_key().clone();
        bind(&mut sub.signatures, &public, &mut *rng)?;
    }
    for sub in key.public_subkeys.iter_mut().take(PARTS_MAX) {
        let public = sub.key.clone();
        bind(&mut sub.signatures, &public, &mut *rng)?;
    }
    Ok(key)
}
