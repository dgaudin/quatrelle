// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The journal of a data file's import (`import.json`), read back as a hostile cache file: an
//! error or a journal within its caps, never a panic; what is read is written and read again
//! the same.
#![no_main]

use libfuzzer_sys::fuzz_target;
use quatrelle_model::{ImportJournal, MAX_IMPORT_FOLDERS, MAX_IMPORT_IDS};

fuzz_target!(|data: &[u8]| {
    let Ok(journal) = ImportJournal::from_json(data) else { return };
    assert!(journal.folders.len() <= MAX_IMPORT_FOLDERS, "{} folders", journal.folders.len());
    assert!(journal.ids() <= MAX_IMPORT_IDS && journal.fits(), "{} ids", journal.ids());
    assert_eq!(journal.unfinished(), journal.is_some() && !journal.finished);
    let again = ImportJournal::from_json(&journal.to_json()).expect("a journal written is read back");
    assert_eq!(again, journal);
});
