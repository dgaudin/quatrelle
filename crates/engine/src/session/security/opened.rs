// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! What was decrypted of messages, held in memory only, in the order it came.

use std::collections::VecDeque;

use super::*;

/// The most decrypted messages held in memory at once.
pub(super) const OPENED_MAX: usize = 64;

/// Decrypted messages: for each, the MIME entity inside and whether it was authenticated, and
/// the order they were decrypted in (oldest first), for the oldest to be let go first. No
/// `Debug`: what was decrypted is never written out.
#[derive(Default)]
pub(crate) struct Opened {
    held: HashMap<MessageKey, (Vec<u8>, bool)>,
    order: VecDeque<MessageKey>,
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
}
