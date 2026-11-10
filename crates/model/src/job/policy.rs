// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Exchange's archive policy tags: the personal tags the server offers, which move a message or
//! a folder's mail to the online archive after so many days (or never).

use serde::{Deserialize, Serialize};

/// Most tags an account is offered (a server may list hundreds; a person reads a few).
pub const MAX_ARCHIVE_TAGS: usize = 100;

/// The longest name a tag is shown with, in characters.
pub const MAX_TAG_NAME: usize = 120;

/// The most days a tag may say (a hundred years): a bigger number is the server's way of saying
/// "unlimited" or a damaged answer.
pub const MAX_TAG_DAYS: u32 = 36_500;

/// One of the server's personal archive tags.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveTag {
    /// Its id, a GUID (checked with [`is_tag_id`]).
    pub id: String,
    /// Its name as the server gives it, bounded and without control characters.
    pub name: String,
    /// After how many days the server moves the mail, when it says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub days: Option<u32>,
    /// The tag that keeps mail out of the archive.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub never: bool,
}

/// Whether `s` is a GUID (`8-4-4-4-12` hexadecimal digits), which is what Exchange names a tag by.
pub fn is_tag_id(s: &str) -> bool {
    s.len() == 36 && s.char_indices().all(|(i, c)| if matches!(i, 8 | 13 | 18 | 23) { c == '-' } else { c.is_ascii_hexdigit() })
}

#[cfg(test)]
mod tests {
    use super::*;

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
