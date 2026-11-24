// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! What was decrypted of messages, held in memory only, in the order it came; a message as it
//! reads with it; and whether a message is kept encrypted.

use std::collections::VecDeque;

use super::*;

/// Whether `raw` is encrypted whole (S/MIME, or OpenPGP in a part of its own), as Quatrelle
/// keeps a draft and sends a message: not text that has an encrypted block in it, which is what
/// its writer put there (a draft with one pasted in never opened again otherwise).
pub(crate) fn is_sealed(raw: &[u8]) -> bool {
    matches!(mime::smime::protection(raw), mime::smime::Protection::Encrypted(_))
        || matches!(mime::pgp::protection(raw), mime::pgp::Protection::Encrypted(_))
}

/// Whether `raw` is encrypted as it stands (S/MIME, or OpenPGP in its own part or in its text).
fn is_encrypted(raw: &[u8]) -> bool {
    if matches!(mime::smime::protection(raw), mime::smime::Protection::Encrypted(_)) {
        return true;
    }
    match mime::pgp::protection(raw) {
        mime::pgp::Protection::Encrypted(_) => true,
        mime::pgp::Protection::Inline { body, .. } => {
            matches!(quatrelle_pgp::inline(&body), Some(quatrelle_pgp::Inline::Message(block)) if quatrelle_pgp::is_encrypted(block))
        }
        _ => false,
    }
}

/// The most answers let pass ([`Opened::stale`]) kept count of: past it, the count starts again.
const STALE_MAX: usize = 256;

/// The most decrypted messages held in memory at once.
pub(super) const OPENED_MAX: usize = 64;

/// Decrypted messages: for each, the MIME entity inside and whether it was authenticated, and
/// the order they were decrypted in (oldest first), for the oldest to be let go first. No
/// `Debug`: what was decrypted is never written out.
#[derive(Default)]
pub(crate) struct Opened {
    held: HashMap<MessageKey, (Vec<u8>, bool)>,
    order: VecDeque<MessageKey>,
    /// Answers still to come for messages asked to be decrypted before their keys came to name
    /// others (the Outbox's rows moved): how many, by key. Each is let pass.
    stale: HashMap<MessageKey, u32>,
}

impl Opened {
    /// What was decrypted of `key`, and whether it was authenticated.
    pub(crate) fn get(&self, key: &MessageKey) -> Option<&(Vec<u8>, bool)> {
        self.held.get(key)
    }

    /// Hold what was decrypted of `key`, as the newest. Past [`OPENED_MAX`] the oldest one that
    /// isn't `shown` goes first (none goes while every one is), and is given back.
    pub(super) fn keep(&mut self, key: &MessageKey, entity: Vec<u8>, authenticated: bool, shown: impl Fn(&MessageKey) -> bool) -> Option<MessageKey> {
        let mut gone = None;
        if self.held.contains_key(key) {
            self.order.retain(|k| k != key);
        } else if self.held.len() >= OPENED_MAX
            && let Some(at) = self.order.iter().position(|k| !shown(k))
            && let Some(k) = self.order.remove(at)
        {
            self.held.remove(&k);
            gone = Some(k);
        }
        self.held.insert(key.clone(), (entity, authenticated));
        self.order.push_back(key.clone());
        gone
    }

    /// Those `keep` says no to let go of.
    fn retain(&mut self, keep: impl Fn(&MessageKey) -> bool) {
        self.held.retain(|k, _| keep(k));
        self.order.retain(|k| keep(k));
    }

    /// `asked` were asked to be decrypted before their keys came to name other messages: their
    /// answers, when they come, are let pass.
    fn asked_before(&mut self, asked: impl Iterator<Item = MessageKey>) {
        if self.stale.len() > STALE_MAX {
            self.stale.clear();
        }
        for k in asked {
            *self.stale.entry(k).or_default() += 1;
        }
    }

    /// Whether an answer for `key` is one asked for before its key named another message (it's
    /// counted off).
    fn stale_answer(&mut self, key: &MessageKey) -> bool {
        let Some(n) = self.stale.get_mut(key) else { return false };
        *n = n.saturating_sub(1);
        if *n == 0 {
            self.stale.remove(key);
        }
        true
    }
}

impl Session {
    /// Keep what was decrypted of `key` (the MIME entity inside, and whether it was
    /// authenticated) in memory. Past [`OPENED_MAX`] the oldest goes, with what its OpenPGP
    /// signature said: never one shown, which would ask for its key (a keychain or card prompt)
    /// again.
    pub(super) fn keep_opened(&mut self, key: &MessageKey, entity: Vec<u8>, authenticated: bool) {
        let (reading, windows) = (&self.reading, &self.windows);
        let shown = |k: &MessageKey| reading.as_ref().is_some_and(|r| r.key == *k) || windows.iter().any(|w| w.key == *k);
        if let Some(gone) = self.opened.keep(key, entity, authenticated, shown) {
            self.pgp.forget_message(&gone);
        }
    }

    /// A message as it reads: what [`Session::unwrap`] shows, without checking anything again.
    pub fn readable_raw(&self, key: &MessageKey) -> Option<Vec<u8>> {
        Some(self.as_written(key, self.kept_or_held(key)?).0)
    }

    /// `raw` (message `key`) with its protection taken off as far as it can be (an encryption
    /// not decrypted yet stays), and whether a signature and an encryption were.
    pub(crate) fn as_written(&self, key: &MessageKey, raw: Vec<u8>) -> (Vec<u8>, bool, bool) {
        let mut cur = raw;
        let theirs = self.shows_sender_inside(key);
        let (mut signed, mut encrypted) = (false, false);
        for _ in 0..4 {
            cur = match mime::smime::protection(&cur) {
                mime::smime::Protection::None => {
                    let pgp = !matches!(mime::pgp::protection(&cur), mime::pgp::Protection::None);
                    let sealed = is_encrypted(&cur);
                    match self.readable_pgp(key, &cur, theirs) {
                        Some(next) => {
                            encrypted |= sealed;
                            signed |= pgp && !sealed;
                            next
                        }
                        None => break,
                    }
                }
                mime::smime::Protection::Signed { content, .. } => {
                    signed = true;
                    put_back(&cur, &content, theirs).0
                }
                mime::smime::Protection::Opaque(cms) => match smime::carried(&cms) {
                    Ok(c) => {
                        signed = true;
                        put_back(&cur, &c, theirs).0
                    }
                    Err(_) => break,
                },
                mime::smime::Protection::Encrypted(_) => match self.opened.get(key) {
                    Some((entity, _)) => {
                        encrypted = true;
                        mime::smime::with_entity(&cur, entity)
                    }
                    None => break,
                },
            };
        }
        (cur, signed, encrypted)
    }

    /// Whether what was decrypted for `key` is to be taken: asked for, and not an answer to an
    /// asking from before its folder's keys moved on (those come first, in order).
    pub(super) fn decrypt_awaited(&mut self, key: &MessageKey) -> bool {
        if self.opened.stale_answer(key) {
            return false;
        }
        self.opening.remove(key) || key.folder != OUTBOX
    }

    /// What was decrypted, and what's being, of the messages in `folder` is forgotten: its keys
    /// name other messages now (the Outbox's count its rows). An answer still to come for one
    /// asked before is that message's, not the one its key names now: it's let pass.
    pub(crate) fn forget_opened_in(&mut self, folder: &str) {
        self.opened.asked_before(self.opening.iter().filter(|k| k.folder == folder).cloned());
        self.opened.retain(|k| k.folder != folder);
        self.opening.retain(|k| k.folder != folder);
        self.decrypt_errors.retain(|k, _| k.folder != folder);
        self.pgp.forget_folder(folder);
    }
}
