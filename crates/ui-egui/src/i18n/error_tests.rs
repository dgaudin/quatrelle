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
/// What the session says when the list of accounts couldn't be read and isn't saved
/// (`engine/src/session/accounts/unread.rs`).
const ACCOUNTS: &str = "the list of accounts couldn't be read, so it isn't saved: {error}";

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

/// The list of accounts that couldn't be read: its wording is the engine's, and every catalog has
/// a line for it.
#[test]
fn the_accounts_notice_has_a_line_in_every_catalog() {
    let engine = include_str!("../../../engine/src/session/accounts/unread.rs");
    let head = ACCOUNTS.trim_end_matches(": {error}");
    assert!(engine.contains(&format!("pub const ACCOUNTS_UNREAD: &str = \"{head}\";")), "not in the engine any more");
    assert!(!head.contains(": "), "the head must be free of ': ': `i18n::error` cuts at the first");
    for lang in LANGUAGES.iter().filter(|l| l.code != "en") {
        let line = lang.catalog().get(ACCOUNTS).unwrap_or_else(|| panic!("{}: no line for {ACCOUNTS}", lang.code));
        assert!(line.ends_with("{error}"), "{}: the value goes at the end: {line}", lang.code);
    }
}

/// The command's error is said in the interface's language, and the file it names stays as it came.
#[test]
fn the_accounts_notice_is_translated() {
    set_current(Lang::from_code("fr").unwrap_or(Lang::EN));
    let file = "/home/studio/.local/share/quatrelle/accounts.json";
    let head = ACCOUNTS.trim_end_matches(": {error}");
    let said = error(&format!("{head}: a file Quatrelle keeps is damaged: {file}"));
    set_current(Lang::EN);
    assert!(said.contains("liste des comptes") && said.contains("abîmé"), "{said}");
    assert!(said.contains(file), "the path was changed: {said}");
    assert!(!said.contains("list of accounts") && !said.contains("damaged"), "left in English: {said}");
}

/// What an account whose folder list is damaged says as Quatrelle opens: its server's listing makes
/// it again; with no server, what goes wrong follows (`engine/src/session/opening/docs.rs`).
const FOLDERS: &str = "this account's list of folders is damaged and is read again from the server";
const FOLDERS_HERE: &str = "this account's list of folders is damaged, so nothing is filed in it for now: {error}";

/// The damaged folder list's two notices: their wording is the engine's, and every catalog has a
/// line for each.
#[test]
fn the_damaged_folder_list_notices_have_a_line_in_every_catalog() {
    let engine = include_str!("../../../engine/src/session/opening/docs.rs");
    let head = FOLDERS_HERE.trim_end_matches(": {error}");
    assert!(engine.contains(&format!("pub(crate) const FOLDERS_DAMAGED: &str = \"{FOLDERS}\";")), "not in the engine any more");
    assert!(engine.contains(&format!("pub(crate) const FOLDERS_DAMAGED_HERE: &str = \"{head}\";")), "not in the engine any more");
    assert!(!head.contains(": "), "the head must be free of ': ': `i18n::error` cuts at the first");
    for lang in LANGUAGES.iter().filter(|l| l.code != "en") {
        assert!(lang.catalog().get(FOLDERS).is_some(), "{}: no line for {FOLDERS}", lang.code);
        let line = lang.catalog().get(FOLDERS_HERE).unwrap_or_else(|| panic!("{}: no line for {FOLDERS_HERE}", lang.code));
        assert!(line.ends_with("{error}"), "{}: the value goes at the end: {line}", lang.code);
    }
}

/// Both are said in the interface's language, after the account's address, and the file named
/// stays as it came.
#[test]
fn the_damaged_folder_list_notices_are_translated() {
    set_current(Lang::from_code("fr").unwrap_or(Lang::EN));
    let file = "/home/studio/.local/share/quatrelle/a1/folders.json";
    let server = error(FOLDERS);
    let here = error(&format!("{}: a file Quatrelle keeps is damaged: {file}", FOLDERS_HERE.trim_end_matches(": {error}")));
    set_current(Lang::EN);
    assert!(server.contains("liste des dossiers") && server.contains("relue"), "{server}");
    assert!(here.contains("liste des dossiers") && here.contains("rangé") && here.contains("abîmé"), "{here}");
    assert!(here.ends_with(file), "the path was changed: {here}");
    assert!(
        !server.contains("list of folders") && !here.contains("list of folders") && !here.contains("damaged"),
        "left in English: {server} / {here}"
    );
}
