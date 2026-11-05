// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Exchange: the one pass that reads the follow-ups marked complete before Quatrelle knew them.
//!
//! Until then a message marked complete in Outlook (`PidTagFlagStatus` 1) came in with no mark,
//! and a sync that asks only what changed never brings it again: each folder is asked once for
//! its completed messages ([`State::ticks_wanted`]), the folders done kept here, so a pass cut
//! short (offline, the app closed) goes on at the next sync where it stopped; once every folder
//! followed has had it, the account's pass is done ([`State::finish_ticks`]).

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

impl State {
    /// Whether this folder is still to be asked for its completed follow-ups.
    pub fn ticks_wanted(&self, folder: &str) -> bool {
        !self.ticks.done && !self.ticks.folders.contains(folder)
    }

    /// This folder had the pass (or had nothing to tick: nothing kept yet).
    pub fn ticks_read(&mut self, folder: &str) {
        if self.ticks.done {
            return;
        }
        if self.ticks.folders.len() >= TICKS_FOLDERS_MAX {
            self.ticks = Ticks { done: true, folders: BTreeSet::new() };
            return;
        }
        self.ticks.folders.insert(folder.to_string());
    }

    /// Once every folder `followed` had the pass, the account's is done (and its list of folders
    /// let go). Whether it was just finished.
    pub fn finish_ticks(&mut self, followed: &[String]) -> bool {
        if self.ticks.done || !followed.iter().all(|f| self.ticks.folders.contains(f)) {
            return false;
        }
        self.ticks = Ticks { done: true, folders: BTreeSet::new() };
        true
    }
}
