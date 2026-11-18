// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What goes with a message going out, as the outbox keeps it ([`PartRecord`]): a file by its
//! path, size and modification time, and once it went up what OneDrive made of it ([`Uploaded`]);
//! a pasted picture by its name, type and length, its bytes kept beside the record
//! (`outbox-parts/<seq>/<n>`, the message's number in 20 digits and the picture's place among the
//! parts) until the message goes.
//!
//! Read back as hostile as the rest of the outbox: a record whose parts break their rules
//! ([`broken`]) is passed over and never written, and a picture past [`PICTURE_INLINE_MAX`] bytes
//! is never read back.

use std::path::Path;

use quatrelle_mime::teams_write_parts;
use quatrelle_model::pim::teams::{
    ChatText, FILE_MAX, FILE_NAME_MAX, Mention, OutPart, PARTS_MAX, PATH_MAX, PICTURE_INLINE_MAX, is_bidi, is_object_id,
};

use super::super::graph::{drive_name, on_microsofts_hosts, own_segment};
use super::*;
use crate::http::Url;

/// The folder of the pictures' bytes.
const PARTS: &str = "outbox-parts";
/// The longest address of a file uploaded.
const MAX_URL: usize = 8 << 10;
/// An id as long as a file's GUID: what a message's places are counted with before its files have
/// their ids.
const GUID_PLACE: &str = "00000000-0000-0000-0000-000000000000";

/// A part of a message going out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "part", rename_all = "camelCase")]
pub(crate) enum PartRecord {
    /// A file, by its path, its name as OneDrive takes it ([`drive_name`]), its size and when it
    /// was last changed (milliseconds since 1970) as they were when it was attached; `up` once it
    /// went up.
    File {
        path: String,
        name: String,
        size: u64,
        modified: i64,
        #[serde(default)]
        up: Option<Uploaded>,
    },
    /// A pasted picture: its name, its type (PNG or JPEG) and its length in bytes.
    Picture { name: String, mime: String, len: u32 },
}

/// A file gone up to the user's OneDrive: its item, its eTag's GUID, its WebDAV address and the
/// name OneDrive gave it (numbered when the name was taken), and whether it is shared with the
/// chat's other members. Never uploaded again for its message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Uploaded {
    pub(crate) item: String,
    pub(crate) guid: String,
    pub(crate) url: String,
    pub(crate) name: String,
    pub(crate) shared: bool,
}

impl PartRecord {
    /// As the conversation shows it: a file's bytes all gone up once it is uploaded.
    pub(crate) fn shown(&self) -> OutPart {
        match self {
            PartRecord::File { name, size, up, .. } => {
                OutPart { name: name.clone(), size: *size, picture: false, sent: if up.is_some() { *size } else { 0 } }
            }
            PartRecord::Picture { name, len, .. } => OutPart { name: name.clone(), size: u64::from(*len), picture: true, sent: 0 },
        }
    }
}

/// Whether a message's text and the places of `pictures` pictures and `files` files fit what Teams
/// takes once written ([`quatrelle_model::pim::teams::SEND_MAX`]), each file's place as long as
/// its GUID makes it.
pub(crate) fn fits(body: &ChatText, mentions: &[Mention], pictures: usize, files: usize) -> bool {
    let ids = vec![GUID_PLACE; files.min(PARTS_MAX)];
    teams_write_parts(body, mentions, pictures, &ids).is_ok()
}

/// Whether a part can be kept by `path`: absolute, at most [`PATH_MAX`] bytes, without a control
/// character.
pub(crate) fn path_ok(path: &str) -> bool {
    !path.is_empty() && path.len() <= PATH_MAX && !path.chars().any(char::is_control) && Path::new(path).is_absolute()
}

/// Whether a part can be shown and sent by `name`: its own [`drive_name`], of 1 to
/// [`FILE_NAME_MAX`] characters.
fn name_ok(name: &str) -> bool {
    !name.is_empty() && name.chars().count() <= FILE_NAME_MAX && drive_name(name) == name
}

/// The first rule the parts of a record break (`send`: the record is a message sent, not a
/// change); `None` when they keep them. At most [`PARTS_MAX`], and none but on a message sent. A
/// file: its path [`path_ok`], its name [`name_ok`], 1 byte to [`FILE_MAX`], a time a date can
/// hold either side of 1970; once uploaded, its item one segment of an address, its GUID an object
/// id, an address of at most 8 KiB without white space, control or bidi character on Microsoft's
/// file hosts ([`on_microsofts_hosts`]: never elsewhere), its name [`name_ok`]. A picture: its
/// name [`name_ok`], `image/png` or `image/jpeg`, 1 byte to [`PICTURE_INLINE_MAX`].
pub(super) fn broken(parts: &[PartRecord], send: bool) -> Option<&'static str> {
    if parts.len() > PARTS_MAX {
        return Some("more parts than a message takes");
    }
    if !send && !parts.is_empty() {
        return Some("parts on a change to a message");
    }
    parts.iter().find_map(part_broken)
}

fn part_broken(part: &PartRecord) -> Option<&'static str> {
    match part {
        PartRecord::File { path, name, size, modified, up } => match () {
            _ if !path_ok(path) => Some("a file's path outside its rules"),
            _ if !name_ok(name) => Some("a part's name outside its rules"),
            _ if !(1..=FILE_MAX).contains(size) => Some("a file's size outside its rules"),
            _ if !(-MAX_MS..=MAX_MS).contains(modified) => Some("a time outside what a date holds"),
            _ if up.as_ref().is_some_and(|u| !uploaded_ok(u)) => Some("a file uploaded outside its rules"),
            _ => None,
        },
        PartRecord::Picture { name, mime, len } => match () {
            _ if !name_ok(name) => Some("a part's name outside its rules"),
            _ if !matches!(mime.as_str(), "image/png" | "image/jpeg") => Some("a picture of a type not sent"),
            _ if *len == 0 || u64::from(*len) > PICTURE_INLINE_MAX => Some("a picture's length outside its rules"),
            _ => None,
        },
    }
}

fn uploaded_ok(u: &Uploaded) -> bool {
    let address = u.url.len() <= MAX_URL && !u.url.chars().any(|c| c.is_whitespace() || c.is_control() || is_bidi(c));
    own_segment(&u.item) && is_object_id(&u.guid) && name_ok(&u.name) && address && Url::parse(&u.url).is_ok_and(|p| on_microsofts_hosts(&p))
}

/// Where the `n`th part of record `seq`, a picture, keeps its bytes.
fn picture_key(seq: u64, n: usize) -> String {
    format!("{PARTS}/{seq:020}/{n}")
}

impl Copy {
    /// The bytes of the `n`th part of record `seq`, a picture, kept until the message goes: 1 byte
    /// to [`PICTURE_INLINE_MAX`], else refused.
    pub(crate) fn put_picture(&self, seq: u64, n: usize, bytes: &[u8]) -> Result<()> {
        if bytes.is_empty() || u64::try_from(bytes.len()).map_or(true, |len| len > PICTURE_INLINE_MAX) {
            return Err(refused("a picture past its size"));
        }
        self.store.write_account_files(TEAMS_ACCOUNT, &[(picture_key(seq, n), bytes.to_vec())])
    }

    /// The bytes kept of the `n`th part of record `seq`, when there are some, at most
    /// [`PICTURE_INLINE_MAX`].
    pub(crate) fn picture(&self, seq: u64, n: usize) -> Option<Vec<u8>> {
        let cap = usize::try_from(PICTURE_INLINE_MAX).unwrap_or(usize::MAX);
        let bytes = self.store.account_file_within(TEAMS_ACCOUNT, &picture_key(seq, n), cap).ok()??;
        (!bytes.is_empty() && u64::try_from(bytes.len()).is_ok_and(|len| len <= PICTURE_INLINE_MAX)).then_some(bytes)
    }

    /// The pictures of record `seq` gone; none there is no error.
    pub(crate) fn remove_pictures(&self, seq: u64) -> Result<()> {
        self.store.remove_account_files(TEAMS_ACCOUNT, &format!("{PARTS}/{seq:020}"))
    }

    /// Every picture of the outbox gone, in the copy in `store`, open or not.
    pub(super) fn clear_pictures(store: &Store) -> Result<()> {
        store.remove_account_files(TEAMS_ACCOUNT, PARTS)
    }
}
