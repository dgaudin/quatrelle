// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The journal of a data file's import (`import.json`, an account document): which file, how far
//! it came, and which of its items are written, so an import stopped (or cut short by a quit or a
//! crash) goes on from there and brings nothing in twice. Read back as hostile input like any
//! cache file: its lists are capped as they are read.

use serde::de::{Deserializer, Error as _, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};

/// Where an account's import journal is kept in its cache.
pub const IMPORT_DOC: &str = "import.json";
/// The most item ids a journal holds, over all its folders…
pub const MAX_IMPORT_IDS: usize = 2_000_000;
/// …and the most folders.
pub const MAX_IMPORT_FOLDERS: usize = 10_000;
/// The error of [`crate::Event::Imported`] when the file isn't the one the import began with (its
/// size or its date changed) and bringing in what is new wasn't asked: nothing was imported.
pub const IMPORT_FILE_CHANGED: &str = "the data file has changed since this import began";
/// The error when an import can't go on for want of its journal (none, or a damaged one set
/// aside): it can only be started over.
pub const IMPORT_NO_JOURNAL: &str =
    "this import can't go on: what it had brought in was not noted (or the note is damaged), so it can only be started over";

/// An import's journal.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportJournal {
    /// The data file, as it was when the import began: where it is, its size in bytes and when
    /// it was last changed (milliseconds since 1970).
    pub path: String,
    pub size: u64,
    pub modified: i64,
    /// The messages of the folders that are imported, counted before the first; `None` when
    /// their counts couldn't be read.
    pub total: Option<usize>,
    /// The whole file was read (an import stopped, or cut short, isn't).
    pub finished: bool,
    /// Messages written, and items left out, when the journal was last written.
    pub messages: usize,
    pub skipped: usize,
    /// The number the next contact, calendar item, note or task takes in its id, the bytes of
    /// the files they keep, and how many calendar items the account held (those after them were
    /// written after the journal, or made by the user since).
    pub pim_next: usize,
    pub pim_files: usize,
    pub pim_events: usize,
    /// The file's folders already begun.
    #[serde(deserialize_with = "folders")]
    pub folders: Vec<ImportedFolder>,
}

/// What is written of one of the data file's folders.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportedFolder {
    /// The folder's node in the data file.
    pub node: u32,
    /// The folder it became in the account; empty for a folder of contacts, calendar items,
    /// notes or tasks, which has none.
    pub path: String,
    /// The number its next message takes.
    pub uid_next: u32,
    /// The nodes of its items already dealt with (written, or left out), sorted.
    #[serde(deserialize_with = "ids")]
    pub ids: Vec<u32>,
}

impl ImportJournal {
    /// A journal from its file's bytes; an error when it isn't one, or holds more than the caps.
    pub fn from_json(bytes: &[u8]) -> Result<ImportJournal, String> {
        serde_json::from_slice(bytes).map_err(|e| e.to_string())
    }

    /// The journal as its file's bytes.
    pub fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// Whether this is the journal of an import (an account that never had one reads as an empty
    /// journal).
    pub fn is_some(&self) -> bool {
        !self.path.is_empty()
    }

    /// Whether the account's import is still to be finished.
    pub fn unfinished(&self) -> bool {
        self.is_some() && !self.finished
    }

    /// The item ids it holds.
    pub fn ids(&self) -> usize {
        self.folders.iter().fold(0usize, |n, f| n.saturating_add(f.ids.len()))
    }

    /// Whether it is within the caps (one that isn't is not written: it couldn't be read back).
    pub fn fits(&self) -> bool {
        self.folders.len() <= MAX_IMPORT_FOLDERS && self.ids() <= MAX_IMPORT_IDS
    }
}

/// A list read up to `cap` entries, each checked by `each` (an error past the cap: nothing is
/// allocated for more).
struct Capped<T, F> {
    cap: usize,
    what: &'static str,
    each: F,
    of: std::marker::PhantomData<T>,
}

impl<'de, T: Deserialize<'de>, F: FnMut(&T) -> bool> Visitor<'de> for Capped<T, F> {
    type Value = Vec<T>;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "a list of at most {} {}", self.cap, self.what)
    }

    fn visit_seq<A: SeqAccess<'de>>(mut self, mut seq: A) -> Result<Vec<T>, A::Error> {
        let mut out = Vec::new();
        while let Some(v) = seq.next_element::<T>()? {
            if out.len() >= self.cap || !(self.each)(&v) {
                return Err(A::Error::custom(format!("more than {} {}", self.cap, self.what)));
            }
            out.push(v);
        }
        Ok(out)
    }
}

fn ids<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u32>, D::Error> {
    d.deserialize_seq(Capped { cap: MAX_IMPORT_IDS, what: "item ids", each: |_: &u32| true, of: std::marker::PhantomData })
}

fn folders<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<ImportedFolder>, D::Error> {
    let mut held = 0usize;
    let each = |f: &ImportedFolder| {
        held = held.saturating_add(f.ids.len());
        held <= MAX_IMPORT_IDS
    };
    d.deserialize_seq(Capped { cap: MAX_IMPORT_FOLDERS, what: "folders (or item ids in all)", each, of: std::marker::PhantomData })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A journal reads back as written; an account without one reads as none.
    #[test]
    fn a_journal_round_trips() {
        let j = ImportJournal {
            path: "/mail/old.pst".into(),
            size: 271_360,
            modified: 1_700_000_000_123,
            total: Some(12),
            messages: 5,
            skipped: 1,
            pim_next: 2,
            folders: vec![ImportedFolder { node: 0x8022, path: "INBOX".into(), uid_next: 6, ids: vec![0x20_0024, 0x20_0044] }],
            ..Default::default()
        };
        assert_eq!(ImportJournal::from_json(&j.to_json()).unwrap(), j);
        assert!(j.unfinished() && j.fits() && j.ids() == 2);
        let none = ImportJournal::from_json(b"{}").unwrap();
        assert!(!none.is_some() && !none.unfinished());
    }

    /// Damaged journals are errors, never a panic; one past the caps is refused as it is read.
    #[test]
    fn a_damaged_journal_is_an_error() {
        for bad in ["", "[]", "{\"folders\":7}", "{\"folders\":[{\"ids\":[-1]}]}", "{\"size\":\"big\"}", "{\"folders\":[{\"ids\":[1,2"] {
            assert!(ImportJournal::from_json(bad.as_bytes()).is_err(), "{bad}");
        }
        let many = format!("{{\"path\":\"p\",\"folders\":[{}]}}", vec!["{}"; MAX_IMPORT_FOLDERS + 1].join(","));
        assert!(ImportJournal::from_json(many.as_bytes()).unwrap_err().contains("folders"));
        let half = format!("{{\"ids\":[{}]}}", vec!["1"; MAX_IMPORT_IDS / 2 + 1].join(","));
        let over = format!("{{\"path\":\"p\",\"folders\":[{half},{half}]}}");
        assert!(ImportJournal::from_json(over.as_bytes()).is_err(), "too many ids over two folders");
        let one = format!("{{\"path\":\"p\",\"folders\":[{half}]}}");
        assert!(ImportJournal::from_json(one.as_bytes()).is_ok_and(|j| j.fits()));
    }
}
