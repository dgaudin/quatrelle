// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The files of a contact or calendar item as the lines of its card or event that hold them: an
//! event's each an `ATTACH` with its bytes inline (RFC 5545 3.8.1.1), a contact's each an
//! `X-QUATRELLE-ATTACH` (a card has no property for a file; RFC 6350 6.10 keeps one of its own
//! name with the card). Both are lines the fork's model keeps as written (`extra`) and writes back
//! in a calendar or address book file. A contact's picture goes in the card's own `photo`.

use quatrelle_model::vcard::base64_encode;

/// The longest file name kept in a parameter, in characters.
const MAX_NAME: usize = 255;

/// The event's line holding the file `name`, of type `mime`.
pub(crate) fn event_file(name: &str, mime: &str, data: &[u8]) -> String {
    format!("ATTACH;FMTTYPE={};ENCODING=BASE64;VALUE=BINARY;X-FILENAME=\"{}\":{}", media_type(mime, name), param(name), base64_encode(data))
}

/// The card's line holding the file `name`, of type `mime`.
pub(crate) fn card_file(name: &str, mime: &str, data: &[u8]) -> String {
    format!("X-QUATRELLE-ATTACH;X-FILENAME=\"{}\":data:{};base64,{}", param(name), media_type(mime, name), base64_encode(data))
}

/// A media type as a parameter or `data:` URI takes it: `type/subtype` of token characters, or
/// the one `name` says when it isn't one.
fn media_type(mime: &str, name: &str) -> String {
    let token = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$&^_.+-".contains(&b));
    let mime = mime.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
    match mime.split_once('/') {
        Some((t, s)) if token(t) && token(s) => mime,
        _ => quatrelle_mime::mime_from_name(name).to_string(),
    }
}

/// A file name as a quoted parameter value: no quotes, control characters or line breaks
/// (RFC 5545 3.1's QSAFE-CHAR), at most [`MAX_NAME`] characters.
fn param(name: &str) -> String {
    let safe: String = name.chars().filter(|c| !c.is_control() && *c != '"').take(MAX_NAME).collect();
    if safe.trim().is_empty() { "attachment".into() } else { safe }
}
