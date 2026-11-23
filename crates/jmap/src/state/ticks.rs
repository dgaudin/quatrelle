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
//! [`State::finish_replies`]). What mail kept before is (a meeting message, a report…) is asked
//! for the same way ([`State::kinds_wanted`]), except that a folder it failed in waits for the
//! next start ([`State::kinds_due`]): a big folder that times out isn't searched at every sync,
//! and one it failed in at [`TICKS_TRIES`] starts has it given up ([`State::kinds_failed`]).

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
    /// The folders it failed in, at how many of Quatrelle's starts, while still to have it
    /// ([`Ticks::failed_at_start`]). Written by no version before; read leniently.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty", deserialize_with = "lenient_failed")]
    pub failed: BTreeMap<String, u8>,
}

/// Starts of Quatrelle a folder's pass may fail at before it's given up there (taken as had): a
/// folder whose search always times out (a big public folder) isn't searched at every start for
/// ever.
pub const TICKS_TRIES: u8 = 3;

impl Ticks {
    pub(super) fn is_new(&self) -> bool {
        !self.done && self.folders.is_empty() && self.failed.is_empty()
    }
}

/// The pass as written, or none when it can't be read (a damaged file): it's done again rather
/// than the whole state, its numbering with it, refused.
pub(super) fn lenient<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Ticks, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(Ticks::deserialize(v).unwrap_or_default())
}

/// The starts each folder's pass failed at, as written: what isn't such a count is left out, and
/// what isn't a list of them is none (the pass tried again rather than all it did forgotten).
fn lenient_failed<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<BTreeMap<String, u8>, D::Error> {
    let Value::Object(counts) = Value::deserialize(d)? else { return Ok(BTreeMap::new()) };
    let count = |n: &Value| n.as_u64().and_then(|n| u8::try_from(n).ok());
    Ok(counts.into_iter().filter_map(|(f, n)| Some((f, count(&n)?))).take(TICKS_FOLDERS_MAX).collect())
}

/// Folders kept in the list of those done, at most (a mailbox has a few hundred; a state file
/// made to hold more is taken as the pass done rather than grown without end).
const TICKS_FOLDERS_MAX: usize = 10_000;

impl Ticks {
    /// Whether this folder is still to have the pass.
    pub fn wanted(&self, folder: &str) -> bool {
        !self.done && !self.folders.contains(folder)
    }

    /// This folder had the pass (or had nothing to read: nothing kept yet), its failures forgotten.
    pub fn read(&mut self, folder: &str) {
        if self.done {
            return;
        }
        self.failed.remove(folder);
        if self.folders.len() >= TICKS_FOLDERS_MAX {
            *self = Ticks { done: true, ..Ticks::default() };
            return;
        }
        self.folders.insert(folder.to_string());
    }

    /// The pass failed in this folder at one of Quatrelle's starts (once a start: the caller
    /// sees to it): counted, and at the [`TICKS_TRIES`]th it's given up there, taken as had
    /// ([`Ticks::read`]). Whether it was.
    pub fn failed_at_start(&mut self, folder: &str) -> bool {
        if !self.wanted(folder) {
            return false;
        }
        let tries = self.failed.get(folder).map_or(1, |n| n.saturating_add(1));
        // As many folders failed as a list holds: given up rather than the list grown.
        let full = self.failed.len() >= TICKS_FOLDERS_MAX && !self.failed.contains_key(folder);
        if tries >= TICKS_TRIES || full {
            self.read(folder);
            return true;
        }
        self.failed.insert(folder.to_string(), tries);
        false
    }

    /// Once every folder `followed` had the pass, it's done (and its list of folders let go).
    /// Whether it was just finished.
    pub fn finish(&mut self, followed: &[String]) -> bool {
        if self.done || !followed.iter().all(|f| self.folders.contains(f)) {
            return false;
        }
        *self = Ticks { done: true, ..Ticks::default() };
        true
    }

    /// The folders a reader of its own did, taken back, and the starts it counted for those it
    /// failed in: the higher of its count and this one's (both started from the same, and a start
    /// counts once).
    pub(super) fn absorb(&mut self, other: Ticks) {
        for (f, n) in other.failed {
            if self.wanted(&f) && (self.failed.contains_key(&f) || self.failed.len() < TICKS_FOLDERS_MAX) {
                let count = self.failed.entry(f).or_default();
                *count = (*count).max(n);
            }
        }
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

    /// Whether this folder is still to be asked what its messages are.
    pub fn kinds_wanted(&self, folder: &str) -> bool {
        self.kinds.wanted(folder)
    }

    /// Whether this folder is to be asked what its messages are now: still to be, and not failed
    /// in this run.
    pub fn kinds_due(&self, folder: &str) -> bool {
        self.kinds.wanted(folder) && !self.kinds_put_off.contains(folder)
    }

    /// This folder had the pass for what its messages are.
    pub fn kinds_read(&mut self, folder: &str) {
        self.kinds.read(folder);
    }

    /// The pass for what this folder's messages are failed (a timeout, a connection cut): still
    /// to be, it isn't asked again before Quatrelle next starts (at most one try a folder a run),
    /// and at the [`TICKS_TRIES`]th start it failed at, it's given up there (taken as had: its mail
    /// kept before is told by its sources alone). Counted once a run. Whether it was given up.
    pub fn kinds_failed(&mut self, folder: &str) -> bool {
        if self.kinds_put_off.contains(folder) {
            return false;
        }
        self.put_off_kinds(folder);
        self.kinds.failed_at_start(folder)
    }

    /// The pass for what this folder's messages are not asked again in this run (a reader's
    /// failure taken back, [`State::absorb`]: its count comes with its pass).
    pub(super) fn put_off_kinds(&mut self, folder: &str) {
        if self.kinds_put_off.len() < TICKS_FOLDERS_MAX {
            self.kinds_put_off.insert(folder.to_string());
        }
    }

    /// The pass for what messages are done once every folder `followed` had it.
    pub fn finish_kinds(&mut self, followed: &[String]) -> bool {
        self.kinds.finish(followed)
    }
}
