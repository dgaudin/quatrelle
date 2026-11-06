// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! OpenPGP's work with an account's own keys ([`Job::Pgp`], issue #36), whose secret parts the
//! network layer keeps in the system's keychain, and what it answers.

use serde::{Deserialize, Serialize};

use super::{Private, Secret};
use crate::MessageKey;

/// What an error says when the system's keychain refused or couldn't be reached (GNOME's keyring
/// not unlocked, none at all): the start of the error, what the system said after it.
pub const NO_KEYCHAIN: &str = "the system keychain isn't available";

/// What [`Job::Pgp`](super::Job::Pgp) does.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "work", rename_all = "camelCase")]
pub enum PgpWork {
    /// Keep the secret key in `file` (a key file as it is, opened with `passphrase`), which then
    /// signs and decrypts; the one in use until now is kept to decrypt. Answered by
    /// [`Event::PgpKeys`](super::Event::PgpKeys).
    Import { file: Private, passphrase: Secret },
    /// Make a key for `name <email>`, valid from `now` for three years, which then signs and
    /// decrypts. Answered by [`Event::PgpKeys`](super::Event::PgpKeys).
    Make { name: String, email: String, now: i64 },
    /// Key `fingerprint` signed again to run three years from `now` (same key, same fingerprint).
    /// Answered by [`Event::PgpKeys`](super::Event::PgpKeys).
    Extend { fingerprint: String, now: i64 },
    /// Key `fingerprint`'s secret part written to `path`, armored, behind `passphrase`. Answered
    /// by [`Event::PgpBackedUp`](super::Event::PgpBackedUp).
    Backup { fingerprint: String, passphrase: Secret, path: String },
    /// Sign and decrypt with key `fingerprint` again; the one in use is kept to decrypt. Answered
    /// by [`Event::PgpKeys`](super::Event::PgpKeys).
    Use { fingerprint: String },
    /// Forget key `fingerprint` and its secret part. Answered by
    /// [`Event::PgpKeys`](super::Event::PgpKeys).
    Forget { fingerprint: String },
    /// Decrypt `data` (an OpenPGP message) with the account's keys, a signature inside checked
    /// with `verify_with` (public keys); `inline`: it was a PGP block in a text (what it holds is
    /// text in `charset`, not a MIME entity). Answered by
    /// [`Event::PgpDecrypted`](super::Event::PgpDecrypted).
    Decrypt { key: MessageKey, data: Vec<u8>, inline: bool, charset: String, verify_with: Vec<Vec<u8>> },
}

/// What a signature inside an OpenPGP message says (checked with the keys the job gave).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "seal", rename_all = "camelCase")]
pub enum PgpSignature {
    /// It holds, by the `key`th of the keys given, made at `at`; `weak`: over SHA-1 or older.
    Good { key: usize, at: Option<i64>, weak: bool },
    /// It doesn't hold.
    Bad,
    /// It was made by a key not given (its key ID).
    Unknown { key_id: String },
}

/// What [`PgpWork`] just did to the account's keys (its key's fingerprint).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "change", rename_all = "camelCase")]
pub enum PgpChange {
    /// A key imported from a file: it was backed up already, being a file.
    Imported { fingerprint: String },
    /// A key made here: it has no backup until one is saved.
    Made { fingerprint: String },
    /// A key signed again to run longer.
    Extended { fingerprint: String },
}
