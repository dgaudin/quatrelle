// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The errors of the layers below that come up to the screen word for word, said in the
//! interface's language through `i18n::error`: their wording in the code, and a line for it in
//! every catalog.

use super::*;

/// What a file Quatrelle keeps says when it is damaged (`store/src/lib.rs`).
const DAMAGED: &str = "a file Quatrelle keeps is damaged: {error}";
/// What an account says when its numbering of messages can't be read (`jmap/src/state.rs`).
const NUMBERING: &str = "this account's message numbering couldn't be read, so its mail isn't synchronized for now: {error}";

/// The wording is still the code's, and every catalog has a line for it.
#[test]
fn the_numbering_notice_has_a_line_in_every_catalog() {
    let store = include_str!("../../../store/src/lib.rs");
    assert!(store.contains("#[error(\"a file Quatrelle keeps is damaged: {path}\")]"), "not in the store any more");
    let jmap = include_str!("../../../jmap/src/state.rs");
    let head = NUMBERING.trim_end_matches(": {error}");
    assert!(jmap.contains(&format!("pub const NUMBERING_UNREAD: &str = \"{head}\";")), "not in the JMAP crate any more");
    assert!(!head.contains(": "), "the head must be free of ': ': `i18n::error` cuts at the first");
    for lang in LANGUAGES.iter().filter(|l| l.code != "en") {
        for key in [NUMBERING, DAMAGED] {
            let line = lang.catalog().get(key).unwrap_or_else(|| panic!("{}: no line for {key}", lang.code));
            assert!(line.ends_with("{error}"), "{}: the value goes at the end: {line}", lang.code);
        }
    }
}

/// An account's numbering that can't be read is said in the interface's language, and the file
/// it names (and what the system said) stay as they came.
#[test]
fn the_numbering_notice_is_translated() {
    set_current(Lang::from_code("fr").unwrap_or(Lang::EN));
    let file = "/home/studio/.local/share/quatrelle/a1/jmap.json";
    let said = error(&format!("{}: a file Quatrelle keeps is damaged: {file}", NUMBERING.trim_end_matches(": {error}")));
    set_current(Lang::EN);
    assert!(said.contains("numérotation") && said.contains("abîmé"), "{said}");
    assert!(said.contains(file), "the path was changed: {said}");
    assert!(!said.contains("numbering") && !said.contains("damaged"), "left in English: {said}");
    // Read as it came when the system's own words follow (the disk failing).
    set_current(Lang::from_code("fr").unwrap_or(Lang::EN));
    let said = error(&format!("{}: {file}: Permission denied (os error 13)", NUMBERING.trim_end_matches(": {error}")));
    set_current(Lang::EN);
    assert!(said.contains("numérotation"), "{said}");
    assert!(said.ends_with(&format!("{file}: Permission denied (os error 13)")), "the system's words changed: {said}");
}
