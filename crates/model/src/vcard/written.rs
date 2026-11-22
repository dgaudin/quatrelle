// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! vCard content lines read one by one: a structured value's components, the kind a line's
//! parameters give, and the quoted-printable text of the older versions.

/// A component of a structured value (`N`, `ORG`): split on unescaped `;`, unescaped.
pub(super) fn components(v: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut it = v.chars();
    while let Some(c) = it.next() {
        match c {
            '\\' => match it.next() {
                Some('n' | 'N') => out.last_mut().into_iter().for_each(|s| s.push('\n')),
                Some(o) => out.last_mut().into_iter().for_each(|s| s.push(o)),
                None => {}
            },
            ';' => out.push(String::new()),
            c => out.last_mut().into_iter().for_each(|s| s.push(c)),
        }
    }
    out
}

/// The kind from `TYPE` parameters (`TYPE=work`, `TYPE=WORK,VOICE`, 3.0's bare `WORK`).
pub(super) fn kind(params: &[(String, String)]) -> String {
    let mut kinds: Vec<String> = Vec::new();
    for (k, v) in params {
        let v = v.trim_matches('"');
        if k == "TYPE" {
            kinds.extend(v.split(',').map(|x| x.trim().to_ascii_lowercase()));
        } else if v.is_empty() {
            kinds.push(k.to_ascii_lowercase());
        }
    }
    kinds.retain(|k| !matches!(k.as_str(), "internet" | "voice" | "pref" | "x400"));
    kinds.into_iter().next().unwrap_or_default()
}

/// Whether a property's head (`NOTE;ENCODING=QUOTED-PRINTABLE`, 2.1's bare
/// `NOTE;QUOTED-PRINTABLE`) says its value is quoted-printable.
pub(super) fn quoted_printable_line(line: &str) -> bool {
    let head = line.split(':').next().unwrap_or("");
    head.split(';').skip(1).any(|p| {
        let v = p.split_once('=').map_or(p, |(k, v)| if k.trim().eq_ignore_ascii_case("ENCODING") { v } else { "" });
        v.trim().trim_matches('"').eq_ignore_ascii_case("QUOTED-PRINTABLE")
    })
}

/// The longest property head read: past it a line's head is taken as settled.
const MAX_HEAD: usize = 4096;

/// A line's (quoted-printable, head still open) after it grew: its head is read again only
/// while it has no colon and is short, so each line costs at most [`MAX_HEAD`] bytes.
pub(super) fn reread_head(line: &str, qp: bool, open: bool) -> (bool, bool) {
    if !open {
        return (qp, false);
    }
    let mut end = line.len().min(MAX_HEAD);
    while !line.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    let head = line.get(..end).unwrap_or_default();
    (quoted_printable_line(head), !head.contains(':') && line.len() < MAX_HEAD)
}

/// Upper-cased value of `CHARSET=` in a property head; empty means UTF-8.
pub(super) fn charset_of(head: &str) -> String {
    head.split(';')
        .skip(1)
        .find_map(|p| p.split_once('=').filter(|(k, _)| k.trim().eq_ignore_ascii_case("CHARSET")))
        .map(|(_, v)| v.trim().trim_matches('"').to_ascii_uppercase())
        .unwrap_or_default()
}

/// A quoted-printable value as text: `=XX` bytes read in its character set (UTF-8, or the
/// single-byte Western sets older phones and address books wrote).
pub(super) fn quoted_printable(v: &str, charset: String) -> String {
    let mut bytes = Vec::with_capacity(v.len());
    let b = v.as_bytes();
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        let hex = |j: usize| b.get(j).and_then(|h| (*h as char).to_digit(16));
        match (c, hex(i + 1), hex(i + 2)) {
            (b'=', Some(hi), Some(lo)) => {
                bytes.push(u8::try_from(hi * 16 + lo).unwrap_or(b'?'));
                i += 3;
            }
            _ => {
                bytes.push(c);
                i += 1;
            }
        }
    }
    let single_byte = charset.starts_with("ISO-8859")
        || charset.starts_with("WINDOWS-125")
        || charset.starts_with("CP125")
        || charset == "LATIN1"
        || charset == "US-ASCII";
    match String::from_utf8(bytes) {
        Ok(s) if !single_byte => s,
        Ok(s) if s.is_ascii() => s,
        Ok(s) => s.bytes().map(western).collect(),
        Err(e) => e.into_bytes().into_iter().map(western).collect(),
    }
}

/// A byte of Windows-1252 (a superset of Latin-1 for text) as a character.
fn western(b: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—',
        '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
    ];
    match b {
        0x80..=0x9f => HIGH.get(usize::from(b - 0x80)).copied().unwrap_or('?'),
        _ => char::from(b),
    }
}
