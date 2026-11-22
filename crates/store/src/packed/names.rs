// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Which data file is each account's (`datafiles.json` in the root): what each one is called,
//! and the list written whole after each change.

use super::*;

/// What an account's data file is called, before its extension and any number: its address, as
/// Outlook names a `.pst` (an opened data file's own name), else the account's folder name.
pub(super) fn stem(root: &Path, account: &str) -> String {
    let email = fs::read(root.join("accounts.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Vec<quatrelle_model::AccountConfig>>(&b).ok())
        .and_then(|list| list.into_iter().find(|a| crate::safe_segment(&a.id) == account).map(|a| a.email));
    let stem = email.as_deref().map(str::trim).filter(|e| !e.is_empty()).unwrap_or(account);
    let stem = stem
        .strip_suffix(".pst")
        .or_else(|| stem.strip_suffix(".ost"))
        .or_else(|| stem.strip_suffix(".quatrelle"))
        .or_else(|| stem.strip_suffix(&format!(".{}", crate::legacy::OLD_EXTENSION)))
        .unwrap_or(stem);
    let stem: String = stem
        .chars()
        .map(|c| if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') { '_' } else { c })
        .take(80)
        .collect();
    let stem = stem.trim_matches(|c: char| c == '.' || c.is_whitespace());
    if stem.is_empty() { account.to_string() } else { stem.to_string() }
}

/// Write the list whole.
pub(super) fn save(root: &Path, names: &BTreeMap<String, String>) {
    if let Ok(json) = serde_json::to_vec_pretty(names) {
        let _ = write_atomic(&root.join(NAMES), &json);
    }
}
