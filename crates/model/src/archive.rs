// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The local archive: the account on this computer that IMAP mail is moved into, and the folder
//! each archived message lands in (the source account's name, then its folders as they were).

use serde::{Deserialize, Serialize};

/// What an account on this computer is for, besides holding mail. Found by this mark, so renaming
/// the account doesn't lose it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AccountRole {
    /// The account IMAP mail is archived into; one at most.
    Archives,
}

/// Longest account name kept as a folder name in the archive.
const MAX_NAME: usize = 100;

/// The Archives folder for a message of `folder_path` in the account named `account_name`: the
/// account's name (control characters out, the delimiter turned into `-`, at most 100 characters,
/// `Account` when nothing is left), then the source path with the source's own delimiter replaced
/// by the Archives account's `delimiter`.
pub fn archive_folder(account_name: &str, folder_path: &str, source_delimiter: Option<char>, delimiter: char) -> String {
    let name: String = account_name.chars().filter(|c| !c.is_control()).map(|c| if c == delimiter { '-' } else { c }).take(MAX_NAME).collect();
    let name = name.trim();
    let name = if name.is_empty() { "Account" } else { name };
    let path = match source_delimiter {
        Some(d) if d != delimiter => folder_path.replace(d, &delimiter.to_string()),
        _ => folder_path.to_string(),
    };
    format!("{name}{delimiter}{path}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AccountConfig;

    #[test]
    fn an_account_written_before_reads_back_without_archive_settings() {
        // A file written before: today's, less the new keys.
        let mut v = serde_json::to_value(AccountConfig { id: "a1".into(), ..Default::default() }).unwrap_or_default();
        if let Some(o) = v.as_object_mut() {
            for k in ["role", "archive_local", "archive_after_months"] {
                o.remove(k);
            }
        }
        let c: AccountConfig = serde_json::from_value(v).unwrap_or_default();
        assert_eq!(c.id, "a1");
        assert_eq!((c.role, c.archive_local, c.archive_after_months), (None, false, None));
        let mut c = c;
        c.role = Some(AccountRole::Archives);
        c.archive_local = true;
        c.archive_after_months = Some(12);
        let json = serde_json::to_string(&c).unwrap_or_default();
        assert!(json.contains(r#""role":"archives""#), "{json}");
        let back: AccountConfig = serde_json::from_str(&json).unwrap_or_default();
        assert_eq!(back, c);
        // The empty settings aren't written.
        let json = serde_json::to_string(&AccountConfig::default()).unwrap_or_default();
        assert!(!json.contains("archive") && !json.contains("role"), "{json}");
    }

    #[test]
    fn archive_folders_keep_the_accounts_tree() {
        assert_eq!(archive_folder("Gmail", "INBOX/2025", Some('/'), '/'), "Gmail/INBOX/2025");
        assert_eq!(archive_folder("a/b", "Sent", Some('/'), '/'), "a-b/Sent");
        assert_eq!(archive_folder("", "INBOX", Some('/'), '/'), "Account/INBOX");
        assert_eq!(archive_folder("Work", "INBOX.Old.2024", Some('.'), '/'), "Work/INBOX/Old/2024");
        assert_eq!(archive_folder("Bad\u{7}\nname", "X", None, '/'), "Badname/X");
        assert_eq!(archive_folder(&"x".repeat(500), "X", None, '/').len(), 100 + 2);
    }
}
