// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The files kept with an appointment or a contact, read back from the lines that hold them: an
//! event's `ATTACH` lines with their bytes inline (RFC 5545 3.8.1.1: `VALUE=BINARY`,
//! `ENCODING=BASE64`), a card's `X-QUATRELLE-ATTACH` lines (`data:` URIs). Both stay among the
//! lines the model keeps as written (`extra`), where a `.pst` import puts them, and are written
//! back with the event or card. An `ATTACH` that only names a file elsewhere (a web address, a
//! CalDAV server's managed attachment) isn't one: it would have to be fetched, and nothing is
//! fetched on its own. An alarm's `ATTACH` (its sound) isn't the event's either.
//!
//! An Exchange appointment's file that isn't kept here (past what's kept of one, or someone
//! else's appointment) is listed by an `X-QUATRELLE-SERVER-ATTACH` line: its name, type and size,
//! and what the account's server knows it by (an EWS attachment id, a Graph attachment's path),
//! to fetch it when it's opened, as Outlook's Cached Exchange Mode fetches one not yet there.
//!
//! The lines are hostile input (a server's, a file's): each is read with caps, and only its head
//! (the name and parameters) is taken apart; the bytes are decoded only when asked for.

use crate::ical::CalEvent;
use crate::vcard::{Card, base64_decode};

/// The most files listed for one item.
pub const MAX_FILES: usize = 64;

/// The longest head (name and parameters) read, in bytes: past it, the line isn't a file's.
const MAX_HEAD: usize = 4096;

/// The line listing a file kept on the account's server, not here.
pub const SERVER_FILE: &str = "X-QUATRELLE-SERVER-ATTACH";

/// The longest reference to a file on the server kept, in bytes (a Graph path holds two ids).
const MAX_REFERENCE: usize = 2048;

/// A file kept with an appointment or a contact: what to show of it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeptFile {
    /// Its name, as written (empty when the line gives none).
    pub name: String,
    /// Its media type, as written (`application/pdf`; empty when not said).
    pub mime: String,
    /// Its size in bytes, from its base64's length (or, on the server, as the server says).
    pub size: usize,
    /// It's on the account's server, not here: fetched when it's opened.
    pub on_server: bool,
}

/// A file's line taken apart: what's shown, and where its bytes are: its base64, or what the
/// server knows it by.
struct FileLine<'a> {
    file: KeptFile,
    base64: &'a str,
}

/// The files kept with an event (its own, not its alarms').
pub fn event_files(e: &CalEvent) -> Vec<KeptFile> {
    event_lines(e).map(|l| l.file).take(MAX_FILES).collect()
}

/// The bytes of an event's file `index` (as [`event_files`] lists them); `None` when there's no
/// such file, it's on the server, or its base64 doesn't decode.
pub fn event_file_data(e: &CalEvent, index: usize) -> Option<Vec<u8>> {
    event_lines(e).take(MAX_FILES).nth(index).filter(|l| !l.file.on_server).and_then(|l| base64_decode(l.base64))
}

/// What the account's server knows an event's file `index` by, when it's there and not here.
pub fn event_file_reference(e: &CalEvent, index: usize) -> Option<String> {
    event_lines(e).take(MAX_FILES).nth(index).filter(|l| l.file.on_server).map(|l| l.base64.to_string())
}

/// An event's files' lines as they're kept (inline or on the server), to keep with the event as
/// it's read again unchanged.
pub fn event_file_lines(e: &CalEvent) -> Vec<String> {
    let mut depth = 0usize;
    e.extra
        .iter()
        .filter(|raw| match split(raw) {
            Some((name, params, value)) => match name.as_str() {
                "BEGIN" => {
                    depth = depth.saturating_add(1);
                    false
                }
                "END" => {
                    depth = depth.saturating_sub(1);
                    false
                }
                "ATTACH" if depth == 0 => inline_attach(&params, value).is_some(),
                SERVER_FILE if depth == 0 => server_attach(&params, value).is_some(),
                _ => false,
            },
            None => false,
        })
        .take(MAX_FILES)
        .cloned()
        .collect()
}

/// The line listing a file on the account's server (`reference`: what it knows it by): its name,
/// type and size. `None` for a reference that can't be one (empty, too long, or with characters
/// no server id or path has).
pub fn server_file_line(name: &str, mime: &str, size: u64, reference: &str) -> Option<String> {
    let reference = reference.trim();
    let fits = |c: char| c.is_ascii_alphanumeric() || "+/=-_.%:~".contains(c);
    if reference.is_empty() || reference.len() > MAX_REFERENCE || !reference.chars().all(fits) {
        return None;
    }
    let name: String = name.chars().filter(|c| !c.is_control() && *c != '"').take(255).collect();
    let name = if name.trim().is_empty() { "attachment".to_string() } else { name };
    let token = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$&^_.+-".contains(&b));
    let mime = mime.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
    let mime = match mime.split_once('/') {
        Some((t, s)) if token(t) && token(s) => format!(";FMTTYPE={mime}"),
        _ => String::new(),
    };
    Some(format!("{SERVER_FILE}{mime};SIZE={size};X-FILENAME=\"{name}\":{reference}"))
}

/// The files kept with a contact.
pub fn card_files(c: &Card) -> Vec<KeptFile> {
    card_lines(c).map(|l| l.file).take(MAX_FILES).collect()
}

/// The bytes of a contact's file `index` (as [`card_files`] lists them).
pub fn card_file_data(c: &Card, index: usize) -> Option<Vec<u8>> {
    card_lines(c).take(MAX_FILES).nth(index).and_then(|l| base64_decode(l.base64))
}

/// Whether an event has a file kept with it.
pub fn event_has_files(e: &CalEvent) -> bool {
    event_lines(e).next().is_some()
}

/// An event's files' lines, outside the components kept whole with it (an alarm's sound is no
/// file of the event's). Only the lines that may be one, or a component's start or end, are read.
fn event_lines(e: &CalEvent) -> impl Iterator<Item = FileLine<'_>> {
    let mut depth = 0usize;
    e.extra.iter().filter_map(move |raw| {
        let (name, params, value) = split(raw)?;
        match name.as_str() {
            "BEGIN" => depth = depth.saturating_add(1),
            "END" => depth = depth.saturating_sub(1),
            "ATTACH" if depth == 0 => return inline_attach(&params, value),
            SERVER_FILE if depth == 0 => return server_attach(&params, value),
            _ => {}
        }
        None
    })
}

/// A file listed on the server: its name, type and size as the line says, and its reference.
fn server_attach<'a>(params: &[(String, String)], value: &'a str) -> Option<FileLine<'a>> {
    let param = |n: &str| params.iter().find(|(k, _)| k == n).map(|(_, v)| v.trim_matches('"').trim());
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_REFERENCE {
        return None;
    }
    let mime = param("FMTTYPE").unwrap_or_default().to_ascii_lowercase();
    let size = param("SIZE").and_then(|s| s.parse::<u64>().ok()).and_then(|s| usize::try_from(s).ok()).unwrap_or(0);
    Some(FileLine { file: KeptFile { name: file_name(params), mime, size, on_server: true }, base64: value })
}

/// A contact's files' lines.
fn card_lines(c: &Card) -> impl Iterator<Item = FileLine<'_>> {
    c.extra.iter().filter_map(|raw| {
        let (name, params, value) = split(raw)?;
        // A card's own property names may carry a group (`item1.X-QUATRELLE-ATTACH`).
        let name = name.rsplit('.').next().unwrap_or_default();
        if name != "X-QUATRELLE-ATTACH" {
            return None;
        }
        // `data:type;base64,…`
        let rest = value.trim().strip_prefix("data:")?;
        let (head, base64) = rest.split_once(',')?;
        let mut parts = head.split(';');
        let mime = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
        if !parts.any(|p| p.trim().eq_ignore_ascii_case("base64")) {
            return None;
        }
        Some(FileLine { file: KeptFile { name: file_name(&params), mime, size: decoded_size(base64), on_server: false }, base64 })
    })
}

/// An `ATTACH` line holding its file: base64, as RFC 5545 has it (`VALUE=BINARY`, or the
/// `ENCODING=BASE64` some writers give alone). One naming a file elsewhere (a URI) is none.
fn inline_attach<'a>(params: &[(String, String)], value: &'a str) -> Option<FileLine<'a>> {
    let param = |n: &str| params.iter().find(|(k, _)| k == n).map(|(_, v)| v.trim_matches('"').trim());
    let binary = param("VALUE").is_some_and(|v| v.eq_ignore_ascii_case("BINARY"));
    let base64 = param("ENCODING").is_some_and(|v| v.eq_ignore_ascii_case("BASE64"));
    if !(binary || base64) {
        return None;
    }
    let mime = param("FMTTYPE").unwrap_or_default().to_ascii_lowercase();
    let value = value.trim();
    Some(FileLine { file: KeptFile { name: file_name(params), mime, size: decoded_size(value), on_server: false }, base64: value })
}

/// A file's name, as its line's parameters give it: Quatrelle's own and Outlook's (`X-FILENAME`),
/// RFC 8607's (`FILENAME`), Apple's (`X-APPLE-FILENAME`). Without quotes or control characters.
fn file_name(params: &[(String, String)]) -> String {
    ["X-FILENAME", "FILENAME", "X-APPLE-FILENAME"]
        .iter()
        .find_map(|n| params.iter().find(|(k, _)| k == n).map(|(_, v)| v))
        .map(|v| v.chars().filter(|c| !c.is_control() && *c != '"').collect::<String>().trim().to_string())
        .unwrap_or_default()
}

/// How many bytes `base64` decodes to, from its length (padding aside): no decoding.
fn decoded_size(base64: &str) -> usize {
    let digits = base64.trim().trim_end_matches('=').len();
    digits.saturating_mul(3) / 4
}

/// A content line's name (upper case), its parameters (names upper case, values as written) and
/// its value, borrowed. Only the head is taken apart; `None` past [`MAX_HEAD`] or without a value.
fn split(raw: &str) -> Option<(String, Vec<(String, String)>, &str)> {
    let mut quoted = false;
    let mut colon = None;
    for (i, c) in raw.char_indices() {
        if i > MAX_HEAD {
            return None;
        }
        match c {
            '"' => quoted = !quoted,
            ':' if !quoted => {
                colon = Some(i);
                break;
            }
            _ => {}
        }
    }
    let colon = colon?;
    let head = raw.get(..colon)?;
    let value = raw.get(colon + 1..)?;
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut q = false;
    for c in head.chars() {
        match c {
            '"' => {
                q = !q;
                cur.push(c);
            }
            ';' if !q => parts.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    parts.push(cur);
    let mut it = parts.into_iter();
    let name = it.next()?.trim().to_ascii_uppercase();
    if name.is_empty() {
        return None;
    }
    let params = it.filter_map(|p| p.split_once('=').map(|(k, v)| (k.trim().to_ascii_uppercase(), v.to_string()))).collect();
    Some((name, params, value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vcard::base64_encode;

    fn event(extra: &[&str]) -> CalEvent {
        CalEvent { uid: "e".into(), extra: extra.iter().map(|l| l.to_string()).collect(), ..Default::default() }
    }

    #[test]
    fn an_events_inline_files_are_listed_and_read() {
        let pdf = base64_encode(b"%PDF-1.7 agenda");
        let txt = base64_encode(b"notes");
        let e = event(&[
            &format!("ATTACH;FMTTYPE=application/pdf;ENCODING=BASE64;VALUE=BINARY;X-FILENAME=\"Agenda draft.pdf\":{pdf}"),
            "ATTACH;FMTTYPE=application/msword:https://files.example/minutes.doc",
            "BEGIN:VALARM",
            &format!("ATTACH;VALUE=BINARY;ENCODING=BASE64;X-FILENAME=\"ding.wav\":{txt}"),
            "END:VALARM",
            &format!("attach;encoding=base64;value=binary;filename=\"notes; 1.txt\":{txt}"),
        ]);
        let files = event_files(&e);
        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["Agenda draft.pdf", "notes; 1.txt"], "a link and the alarm's sound aren't the event's files");
        assert_eq!((files[0].mime.as_str(), files[0].size), ("application/pdf", 15));
        assert_eq!(files[1].size, 5);
        assert_eq!(event_file_data(&e, 0).as_deref(), Some(&b"%PDF-1.7 agenda"[..]));
        assert_eq!(event_file_data(&e, 1).as_deref(), Some(&b"notes"[..]));
        assert_eq!(event_file_data(&e, 2), None);
        assert!(event_has_files(&e) && !event_has_files(&event(&["ATTACH:https://x.example/a"])));
    }

    #[test]
    fn a_cards_files_are_listed_and_read() {
        let docx = base64_encode(b"PK docx");
        let c = Card {
            uid: "c".into(),
            extra: vec![
                format!(
                    "X-QUATRELLE-ATTACH;X-FILENAME=\"CV.docx\":data:application/vnd.openxmlformats-officedocument.wordprocessingml.document;base64,{docx}"
                ),
                "X-QUATRELLE-ATTACH;X-FILENAME=\"link\":https://x.example/a".into(),
                "X-OTHER:data:text/plain;base64,AAAA".into(),
                "item1.X-QUATRELLE-ATTACH:data:text/plain;base64,aGk=".into(),
            ],
            ..Default::default()
        };
        let files = card_files(&c);
        assert_eq!(files.len(), 2, "{files:?}");
        assert_eq!((files[0].name.as_str(), files[0].size), ("CV.docx", 7));
        assert_eq!((files[1].name.as_str(), files[1].mime.as_str()), ("", "text/plain"));
        assert_eq!(card_file_data(&c, 0).as_deref(), Some(&b"PK docx"[..]));
        assert_eq!(card_file_data(&c, 1).as_deref(), Some(&b"hi"[..]));
    }

    #[test]
    fn hostile_lines_are_no_files() {
        let long = format!("ATTACH;X-FILENAME=\"{}\";VALUE=BINARY:AAAA", "a".repeat(MAX_HEAD + 10));
        let e = event(&[&long, "ATTACH;VALUE=BINARY", ":", "ATTACH;VALUE=BINARY:not base64 at all!", "ATTACH;X-FILENAME=\"a\u{0}b\";VALUE=BINARY:"]);
        let files = event_files(&e);
        assert_eq!(files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["", "ab"]);
        assert_eq!(event_file_data(&e, 0), None, "a wrong alphabet decodes to nothing");
        let many: Vec<String> = (0..MAX_FILES + 5).map(|i| format!("ATTACH;VALUE=BINARY;X-FILENAME=\"{i}\":aGk=")).collect();
        let e = CalEvent { extra: many, ..Default::default() };
        assert_eq!(event_files(&e).len(), MAX_FILES);
        assert_eq!(event_file_data(&e, MAX_FILES), None);
    }

    #[test]
    fn a_file_on_the_server_is_listed_and_named_but_not_read() {
        let line = server_file_line("Deck \"Q4\".pptx", "Application/PDF; x=y", 1_500_000, "AAMkAD+/=").unwrap();
        assert_eq!(line, "X-QUATRELLE-SERVER-ATTACH;FMTTYPE=application/pdf;SIZE=1500000;X-FILENAME=\"Deck Q4.pptx\":AAMkAD+/=");
        let inline = format!("ATTACH;VALUE=BINARY;ENCODING=BASE64;X-FILENAME=\"a.txt\":{}", base64_encode(b"hi"));
        let e = event(&[&inline, &line, "BEGIN:VALARM", &line, "END:VALARM", "X-OTHER:1"]);
        let files = event_files(&e);
        assert_eq!(files.len(), 2, "the alarm's line isn't the event's");
        assert_eq!(
            (files[1].name.as_str(), files[1].mime.as_str(), files[1].size, files[1].on_server),
            ("Deck Q4.pptx", "application/pdf", 1_500_000, true)
        );
        assert!(!files[0].on_server);
        assert_eq!(event_file_data(&e, 1), None, "its bytes aren't here");
        assert_eq!(event_file_reference(&e, 1).as_deref(), Some("AAMkAD+/="));
        assert_eq!(event_file_reference(&e, 0), None);
        assert_eq!(event_file_lines(&e), [inline.clone(), line.clone()]);
        // A reference no server gives isn't written; a name-less file is still one.
        assert_eq!(server_file_line("x", "", 1, "a b"), None);
        assert_eq!(server_file_line("x", "", 1, "a\r\nb"), None);
        assert_eq!(server_file_line("x", "", 1, ""), None);
        assert_eq!(server_file_line("x", "", 1, &"a".repeat(MAX_REFERENCE + 1)), None);
        assert!(server_file_line("", "not a type", 0, "id").unwrap().contains("X-FILENAME=\"attachment\":id"));
        // A hostile line: no size, no value.
        let e = event(&["X-QUATRELLE-SERVER-ATTACH;SIZE=lots;X-FILENAME=\"b\":id", "X-QUATRELLE-SERVER-ATTACH;X-FILENAME=\"c\":"]);
        let files = event_files(&e);
        assert_eq!((files.len(), files[0].size), (1, 0));
    }
}
