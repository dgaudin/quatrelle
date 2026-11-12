// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Exchange's policy tags: the personal tags the server offers, which move a message or a
//! folder's mail to the online archive after so many days (archive tags), or delete it
//! (retention tags), or never.

use serde::{Deserialize, Serialize};

/// Most tags an account is offered (a server may list hundreds; a person reads a few).
pub const MAX_ARCHIVE_TAGS: usize = 100;

/// The longest name a tag is shown with, in characters.
pub const MAX_TAG_NAME: usize = 120;

/// The most days a tag may say (a hundred years): a bigger number is the server's way of saying
/// "unlimited" or a damaged answer.
pub const MAX_TAG_DAYS: u32 = 36_500;

/// What a policy tag does when its days are up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PolicyKind {
    /// It moves the mail to the online archive (`item:ArchiveTag`, `folder:ArchiveTag`).
    #[default]
    Archive,
    /// It deletes the mail (`item:PolicyTag`, `folder:PolicyTag`).
    Retention,
}

impl PolicyKind {
    /// Both kinds, archive first.
    pub const ALL: [PolicyKind; 2] = [PolicyKind::Archive, PolicyKind::Retention];

    pub fn is_archive(&self) -> bool {
        *self == PolicyKind::Archive
    }
}

/// One of the server's personal policy tags (archive or retention).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveTag {
    /// Its id, a GUID (checked with [`is_tag_id`]).
    pub id: String,
    /// Its name as the server gives it, bounded and without control characters.
    pub name: String,
    /// After how many days the server moves the mail, when it says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub days: Option<u32>,
    /// The tag that keeps mail where it is (out of the archive, or never deleted).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub never: bool,
    /// Whether it archives or deletes; a tag kept without it is an archive tag.
    #[serde(default, skip_serializing_if = "PolicyKind::is_archive")]
    pub kind: PolicyKind,
}

/// Whether `s` is a GUID (`8-4-4-4-12` hexadecimal digits), which is what Exchange names a tag by.
pub fn is_tag_id(s: &str) -> bool {
    s.len() == 36 && s.char_indices().all(|(i, c)| if matches!(i, 8 | 13 | 18 | 23) { c == '-' } else { c.is_ascii_hexdigit() })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tag written before retention tags existed reads as an archive tag; a retention tag
    /// says so.
    #[test]
    fn a_tag_without_a_kind_is_an_archive_tag() {
        let old: ArchiveTag = serde_json::from_str(r#"{"id":"x","name":"1 Year","days":365}"#).unwrap();
        assert_eq!(old.kind, PolicyKind::Archive);
        let gone = ArchiveTag { id: "y".into(), name: "1 Week".into(), days: Some(7), never: false, kind: PolicyKind::Retention };
        let json = serde_json::to_string(&gone).unwrap();
        assert!(json.contains(r#""kind":"retention""#), "{json}");
        assert_eq!(serde_json::from_str::<ArchiveTag>(&json).unwrap(), gone);
        assert!(!serde_json::to_string(&old).unwrap().contains("kind"));
    }

    /// A change in the journal from before retention tags is an archive one; a retention change
    /// keeps its kind, and its log line says which.
    #[test]
    fn a_journal_change_without_a_kind_is_an_archive_one() {
        use crate::Op;
        let old: Op = serde_json::from_str(r#"{"op":"setArchiveTag","folder":"Projects","tag":"t"}"#).unwrap();
        assert!(matches!(old, Op::SetArchiveTag { kind: PolicyKind::Archive, .. }), "{old:?}");
        assert_eq!(old.summary(), "SetArchiveTag of Projects");
        let gone = Op::SetArchiveTag { keys: Vec::new(), folder: Some("Projects".into()), tag: None, kind: PolicyKind::Retention };
        let back: Op = serde_json::from_str(&serde_json::to_string(&gone).unwrap()).unwrap();
        assert_eq!(back, gone);
        assert_eq!(gone.summary(), "SetRetentionTag of Projects");
    }

    #[test]
    fn only_a_guid_is_a_tag_id() {
        assert!(is_tag_id("0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0"));
        for bad in [
            "",
            "x",
            "0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1fg",
            "0f1e2d3c4b5a69788796a5b4c3d2e1f0",
            "{0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0}",
            "é1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0",
        ] {
            assert!(!is_tag_id(bad), "{bad:?}");
        }
    }
}
