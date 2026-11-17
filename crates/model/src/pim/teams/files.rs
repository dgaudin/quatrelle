// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What goes with a Teams message (files, pasted pictures), a file opened, the people the
//! directory names, and the caps on each.

use serde::{Deserialize, Serialize};

use super::{is_bidi, one_line};

/// The most parts (files and pictures) one message takes.
pub const PARTS_MAX: usize = 10;
/// The most bytes of a file sent or opened (100 MiB).
pub const FILE_MAX: u64 = 100 << 20;
/// The most bytes of a picture sent inside its message (4 MiB).
pub const PICTURE_INLINE_MAX: u64 = 4 << 20;
/// The most bytes of a part's path.
pub const PATH_MAX: usize = 4096;
/// The most characters of a file's name.
pub const FILE_NAME_MAX: usize = 255;
/// The most people one directory search keeps.
pub const PEOPLE_FOUND: usize = 25;
/// The most characters of a directory search.
pub const PEOPLE_QUERY_CHARS: usize = 64;
/// The most people a new conversation names.
pub const NEW_CHAT_PEOPLE: usize = 20;
/// The most people a file is shared with.
pub const SHARE_PEOPLE: usize = 100;
/// The most characters of an address.
pub const ADDRESS_MAX: usize = 320;

/// What goes with a message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "part", rename_all = "camelCase")]
pub enum Part {
    /// A file, by its path.
    File { path: String },
    /// A pasted picture, by the path of the file it was put in.
    Picture { path: String },
}

/// A part as the conversation shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutPart {
    pub name: String,
    pub size: u64,
    pub picture: bool,
    /// The bytes gone up so far.
    pub sent: u64,
}

/// A person the directory names.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Person {
    pub id: String,
    pub name: String,
    pub address: String,
    /// A guest of the organisation.
    pub guest: bool,
}

/// Whether `id` is an object id: 8-4-4-4-12 hexadecimal digits.
pub fn is_object_id(id: &str) -> bool {
    let groups: Vec<&str> = id.split('-').collect();
    groups.len() == 5 && groups.iter().zip([8, 4, 4, 4, 12]).all(|(g, n)| g.len() == n && g.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Whether a person can be used: an object id, a name that is its own [`one_line`] and not empty,
/// an address of at most [`ADDRESS_MAX`] characters with no white space, control or bidi character.
pub fn person_ok(p: &Person) -> bool {
    is_object_id(&p.id)
        && !p.name.is_empty()
        && one_line(&p.name) == p.name
        && p.address.chars().count() <= ADDRESS_MAX
        && !p.address.chars().any(|c| c.is_whitespace() || c.is_control() || is_bidi(c))
}

/// How a file asked for ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "file", rename_all = "camelCase")]
pub enum FileOutcome {
    /// Saved at `path`.
    Saved { path: String },
    /// Not saved; `code` is Teams' answer when it gave one.
    Failed { why: FileWhy, code: Option<String> },
}

/// Why a file wasn't saved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "why", rename_all = "camelCase")]
pub enum FileWhy {
    TooBig,
    NoRoom,
    Refused,
    Offline,
    Stalled,
    Busy,
    Gone,
}

/// Why a conversation wasn't created.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "notCreated", rename_all = "camelCase")]
pub enum NotCreated {
    Offline,
    Refused,
    TooMany,
}
