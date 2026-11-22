// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Catalog tests: what an import's end says, in French.

use super::*;

/// A data file copied in whole whose folder list couldn't be read back: the engine's notice is
/// said in French, its values put in.
#[test]
fn a_copy_whose_folder_list_cant_be_read_back_is_said_in_french() {
    set_current(Lang::from_code("fr").unwrap_or(Lang::EN));
    let said = tfmt(
        "{name}: {messages} messages imported, but its folder list couldn't be read back: it shows when Quatrelle starts again",
        &[("name", "Old"), ("messages", "4")],
    );
    set_current(Lang::EN);
    assert_eq!(
        said,
        "Old : 4 messages importés, mais la liste de ses dossiers n’a pas pu être relue : elle s’affichera au prochain démarrage de Quatrelle"
    );
}
