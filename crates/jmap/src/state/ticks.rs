// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Exchange: the passes, made once, that read marks Outlook set before Quatrelle knew them.
//!
//! Until then a message marked complete in Outlook (`PidTagFlagStatus` 1) came in with no mark,
//! as did one replied to or forwarded (`PidTagLastVerbExecuted`), and a sync that asks only what
//! changed never brings them again: each folder is asked once for those messages
//! ([`State::ticks_wanted`], [`State::replies_wanted`]), the folders done kept here, so a pass
//! cut short (offline, the app closed) goes on at the next sync where it stopped; once every
//! folder followed has had it, the account's pass is done ([`State::finish_ticks`],
//! [`State::finish_replies`]).

use std::collections::BTreeSet;

use super::*;

/// Where the pass stands for an account.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Ticks {
    /// Every folder followed had it: nothing more is asked.
    #[serde(default)]
    pub done: bool,
    /// The folders that had it, while it isn't done.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub folders: BTreeSet<String>,
}

impl Ticks {
    pub(super) fn is_new(&self) -> bool {
        !self.done && self.folders.is_empty()
    }
}

/// The pass as written, or none when it can't be read (a damaged file): it's done again rather
/// than the whole state, its numbering with it, refused.
pub(super) fn lenient<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Ticks, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(Ticks::deserialize(v).unwrap_or_default())
}

/// Folders kept in the list of those done, at most (a mailbox has a few hundred; a state file
/// made to hold more is taken as the pass done rather than grown without end).
const TICKS_FOLDERS_MAX: usize = 10_000;

impl Ticks {
    /// Whether this folder is still to have the pass.
    pub fn wanted(&self, folder: &str) -> bool {
        !self.done && !self.folders.contains(folder)
    }

    /// This folder had the pass (or had nothing to read: nothing kept yet).
    pub fn read(&mut self, folder: &str) {
        if self.done {
            return;
        }
        if self.folders.len() >= TICKS_FOLDERS_MAX {
            *self = Ticks { done: true, folders: BTreeSet::new() };
            return;
        }
        self.folders.insert(folder.to_string());
    }

    /// Once every folder `followed` had the pass, it's done (and its list of folders let go).
    /// Whether it was just finished.
    pub fn finish(&mut self, followed: &[String]) -> bool {
        if self.done || !followed.iter().all(|f| self.folders.contains(f)) {
            return false;
        }
        *self = Ticks { done: true, folders: BTreeSet::new() };
        true
    }

    /// The folders a reader of its own did, taken back.
    pub(super) fn absorb(&mut self, other: Ticks) {
        for f in other.folders {
            self.read(&f);
        }
    }
}

impl State {
    /// Whether this folder is still to be asked for its completed follow-ups.
    pub fn ticks_wanted(&self, folder: &str) -> bool {
        self.ticks.wanted(folder)
    }

    /// This folder had the pass for completed follow-ups.
    pub fn ticks_read(&mut self, folder: &str) {
        self.ticks.read(folder);
    }

    /// The pass for completed follow-ups done once every folder `followed` had it.
    pub fn finish_ticks(&mut self, followed: &[String]) -> bool {
        self.ticks.finish(followed)
    }

    /// Whether this folder is still to be asked for its messages replied to or forwarded.
    pub fn replies_wanted(&self, folder: &str) -> bool {
        self.replies.wanted(folder)
    }

    /// This folder had the pass for messages replied to or forwarded.
    pub fn replies_read(&mut self, folder: &str) {
        self.replies.read(folder);
    }

    /// The pass for messages replied to or forwarded done once every folder `followed` had it.
    pub fn finish_replies(&mut self, followed: &[String]) -> bool {
        self.replies.finish(followed)
    }
}
