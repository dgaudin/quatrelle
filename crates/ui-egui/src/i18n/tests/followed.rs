// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Catalog tests: what is said, in French, when the list of public folders to keep in sync can't
//! be read.

use super::*;

/// The list of public folders to keep in sync that can't be read: its wording is said in French,
/// and what follows it (the store's error) as it came.
#[test]
fn public_folders_followed_that_cant_be_read_are_said_in_french() {
    set_current(Lang::from_code("fr").unwrap_or(Lang::EN));
    let said =
        error("the public folders to keep in sync couldn't be read, so they aren't changed: memory/a1/public-followed.json: failed on purpose");
    set_current(Lang::EN);
    assert_eq!(
        said,
        "Les dossiers publics à synchroniser n’ont pas pu être lus, ils ne sont donc pas modifiés : memory/a1/public-followed.json: failed on purpose"
    );
}
