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

/// What an error says when a message was encrypted to none of the account's keys: the start of
/// the error, the key IDs it went to after it.
pub const NOT_FOR_YOU: &str = "it was encrypted to a key Quatrelle doesn't have";

/// Why the browser does no OpenPGP work with the account's own keys (yet): what it answers.
pub const NOT_IN_BROWSER: &str = "OpenPGP isn't available in the browser yet: your keys are used by the Quatrelle desktop app";

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
    /// Sign with the key in use (`sign`) and/or encrypt to `encrypt_to` (public keys; empty: not
    /// encrypted) message `id`, about to be sent, as PGP/MIME: signed, then encrypted, its header
    /// fields inside too, the Subject outside `[...]` with `hide_subject`. Answered by
    /// [`Event::Protected`](super::Event::Protected).
    Protect { id: u64, raw: Private, sign: bool, encrypt_to: Vec<Vec<u8>>, hide_subject: bool },
    /// A draft of a message to be encrypted with OpenPGP, encrypted to the account's own key
    /// first (the copy in Drafts is no more readable than what will be sent), then saved as
    /// [`Job::SaveDraft`](super::Job::SaveDraft) (or kept as
    /// [`Job::KeepDraft`](super::Job::KeepDraft), `keep`).
    SealDraft { raw: Private, folder: String, replaces: Option<MessageKey>, keep: bool },
}

/// What a place without OpenPGP of one's own (the browser, for now) does with [`PgpWork`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unavailable {
    /// Answered at once, with why.
    Answer(super::Event),
    /// A draft to seal, saved as it is instead (this job), the user told why (this notice).
    Saved(Box<super::Job>, super::Event),
}

impl PgpWork {
    /// What `account`'s work comes to where OpenPGP can't be done, `why` (in the user's words):
    /// every job is answered with it, as an error, so nothing waits for an answer that won't come
    /// and nothing goes unprotected; a draft is saved as it is, and the user told.
    pub fn unavailable(self, account: &str, why: &str) -> Unavailable {
        use super::Event;
        let (account, error) = (account.to_string(), Some(why.to_string()));
        Unavailable::Answer(match self {
            PgpWork::Import { .. } | PgpWork::Make { .. } | PgpWork::Extend { .. } | PgpWork::Use { .. } | PgpWork::Forget { .. } => {
                Event::PgpKeys { account, keys: Vec::new(), changed: None, error }
            }
            PgpWork::Backup { fingerprint, .. } => Event::PgpBackedUp { account, fingerprint, error },
            PgpWork::Decrypt { key, .. } => Event::PgpDecrypted { key, content: None, signature: None, keys: Vec::new(), error },
            PgpWork::Protect { id, .. } => Event::Protected { account, id, raw: None, error },
            PgpWork::SealDraft { raw, folder, replaces, keep } => {
                let notice =
                    Event::Notice { account: account.clone(), message: format!("the draft couldn't be encrypted, so it's saved as it is: {why}") };
                let raw = raw.0;
                let job = if keep {
                    super::Job::KeepDraft { account, raw, folder, replaces }
                } else {
                    super::Job::SaveDraft { account, raw, folder, replaces }
                };
                return Unavailable::Saved(Box::new(job), notice);
            }
        })
    }
}

/// What a signature inside an OpenPGP message says (checked with the keys the job gave).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "seal", rename_all = "camelCase")]
pub enum PgpSignature {
    /// It holds, by the `key`th of the keys given, made at `at`; `weak`: over SHA-1 or older;
    /// `until`: when the part of the key that made it (or the signature itself) runs out.
    Good {
        key: usize,
        at: Option<i64>,
        weak: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        until: Option<i64>,
    },
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

#[cfg(test)]
mod tests {
    use super::super::{Event, Job};
    use super::*;

    /// The browser answers every OpenPGP job at once, with why: a message to decrypt or to sign
    /// and encrypt is never left waiting, nor sent as it is; a draft is saved as it is, said.
    #[test]
    fn the_browser_answers_every_openpgp_job() {
        let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 7 };
        let decrypt = PgpWork::Decrypt { key: key.clone(), data: b"x".to_vec(), inline: false, charset: String::new(), verify_with: Vec::new() };
        let error = Some(NOT_IN_BROWSER.to_string());
        assert_eq!(
            decrypt.unavailable("a1", NOT_IN_BROWSER),
            Unavailable::Answer(Event::PgpDecrypted { key, content: None, signature: None, keys: Vec::new(), error: error.clone() })
        );
        let protect = PgpWork::Protect { id: 3, raw: Private(b"m".to_vec()), sign: true, encrypt_to: vec![b"k".to_vec()], hide_subject: true };
        assert_eq!(
            protect.unavailable("a1", NOT_IN_BROWSER),
            Unavailable::Answer(Event::Protected { account: "a1".into(), id: 3, raw: None, error: error.clone() })
        );
        let make = PgpWork::Make { name: "R".into(), email: "r@x".into(), now: 0 };
        assert!(
            matches!(make.unavailable("a1", NOT_IN_BROWSER), Unavailable::Answer(Event::PgpKeys { keys, error: Some(_), .. }) if keys.is_empty())
        );
        let backup = PgpWork::Backup { fingerprint: "AB".into(), passphrase: Secret("p".into()), path: "k.asc".into() };
        assert!(matches!(backup.unavailable("a1", NOT_IN_BROWSER), Unavailable::Answer(Event::PgpBackedUp { error: Some(_), .. })));
        let seal = PgpWork::SealDraft { raw: Private(b"draft".to_vec()), folder: "Drafts".into(), replaces: None, keep: false };
        match seal.unavailable("a1", NOT_IN_BROWSER) {
            Unavailable::Saved(job, Event::Notice { message, .. }) => {
                let Job::SaveDraft { raw, folder, .. } = *job else { panic!("{job:?}") };
                assert_eq!((raw.as_slice(), folder.as_str()), (b"draft".as_slice(), "Drafts"));
                assert!(message.contains(NOT_IN_BROWSER));
            }
            other => panic!("{other:?}"),
        }
        let kept = PgpWork::SealDraft { raw: Private(b"draft".to_vec()), folder: "Drafts".into(), replaces: None, keep: true };
        assert!(matches!(kept.unavailable("a1", NOT_IN_BROWSER), Unavailable::Saved(job, _) if matches!(*job, Job::KeepDraft { .. })));
    }
}
