// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! vCard content lines read one by one: a structured value's components, the kind a line's
//! parameters give, the quoted-printable text of the older versions, and an address's or
//! number's line kept as written ([`Card::as_written`]).

use super::Card;

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

/// Where a structured value's components are parted: its unescaped `;`s.
pub(super) fn separators(v: &str) -> Vec<usize> {
    let mut at = Vec::new();
    let mut escaped = false;
    for (i, c) in v.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            ';' => at.push(i),
            _ => {}
        }
    }
    at
}

/// What follows the first `n` components of a structured value, as written (`None` when it has
/// no more).
pub(super) fn after_components(v: &str, n: usize) -> Option<&str> {
    let i = *separators(v).get(n.checked_sub(1)?)?;
    v.get(i + 1..)
}

/// A content line's head and value, split at its first colon outside quotes.
pub(super) fn split_line(raw: &str) -> Option<(&str, &str)> {
    let mut quoted = false;
    let colon = raw.char_indices().find(|(_, c)| {
        if *c == '"' {
            quoted = !quoted;
        }
        *c == ':' && !quoted
    })?;
    Some((raw.get(..colon.0).unwrap_or(""), raw.get(colon.0 + 1..).unwrap_or("")))
}

/// A head's parameters, keys upper-cased (a bare one, 3.0's `WORK`, with an empty value).
pub(super) fn params_of(head: &str) -> Vec<(String, String)> {
    head.split(';')
        .skip(1)
        .map(|p| p.split_once('=').map_or((p.trim().to_ascii_uppercase(), String::new()), |(k, v)| (k.trim().to_ascii_uppercase(), v.to_string())))
        .collect()
}

/// Where an address's or number's line as written is kept ([`Card::as_written`]).
pub(super) fn written_key(prop: &str, value: &str) -> String {
    format!("{prop}:{value}")
}

/// The longest line kept as written: a longer one (hostile, or a mistake) is written back the
/// plain way, so a card can't grow the cache by what its lines carry.
const MAX_WRITTEN: usize = 2048;

/// Keep an address's or number's line as written, when it can go into the 4.0 card written
/// back as it is: not 2.1's quoted-printable or character set, nor 3.0's bare `WORK`. Of two
/// with one value (a number both work and home), the first.
pub(super) fn keep_written(c: &mut Card, prop: &str, value: &str, raw: &str, params: &[(String, String)]) {
    let fits = raw.len() <= MAX_WRITTEN && params.iter().all(|(k, v)| !v.is_empty() && k != "ENCODING" && k != "CHARSET");
    if fits {
        c.as_written.entry(written_key(prop, value)).or_insert_with(|| raw.to_string());
    }
}

/// An address's or number's line as it goes back: as written while it's of the kind read from
/// it, else with its group and other parameters and the new kind as its type. `None` when it
/// isn't a line of `prop` (a cache edited by hand can't make a card write another property).
pub(super) fn written_again(line: &str, prop: &str, kind_now: &str) -> Option<String> {
    let (head, value) = split_line(line)?;
    let name = head.split(';').next().unwrap_or("").rsplit('.').next().unwrap_or("").trim();
    if !name.eq_ignore_ascii_case(prop) {
        return None;
    }
    if kind(&params_of(head)) == kind_now {
        return Some(line.replace(['\r', '\n'], ""));
    }
    let mut parts = head.split(';');
    let mut out = parts.next().unwrap_or("").to_string();
    for p in parts.filter(|p| p.split_once('=').is_some_and(|(k, _)| !k.trim().eq_ignore_ascii_case("TYPE"))) {
        out.push(';');
        out.push_str(p);
    }
    if !kind_now.is_empty() {
        out.push_str(";TYPE=");
        out.push_str(kind_now);
    }
    Some(format!("{out}:{value}").replace(['\r', '\n'], ""))
}

#[cfg(test)]
mod tests {
    use super::super::*;

    /// A card from a server written back as it was read: all of its name (a middle name, a
    /// prefix and a suffix were dropped), and its addresses' and numbers' lines whole, with their
    /// group (an Apple label beside one was left naming nothing), every type and their preference.
    /// One whose kind was changed is written with the new kind, its group and preference kept.
    #[test]
    fn a_server_card_is_written_back_whole() {
        let card = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:zoe\r\nFN:Dr. Zoe Marie Park Jr.\r\nN:Park;Zoe;Marie;Dr.;Jr.\r\n\
item1.TEL;TYPE=WORK,FAX;PREF=1:+1 555 0100\r\nitem1.X-ABLabel:Studio fax\r\n\
EMAIL;TYPE=INTERNET,WORK;PREF=1:zoe@studio.example\r\nTEL;VALUE=uri;TYPE=cell:tel:+49-151-234\r\nEND:VCARD\r\n";
        let c = parse(card).remove(0);
        assert_eq!((c.given.as_str(), c.family.as_str()), ("Zoe", "Park"));
        assert_eq!(c.phones[1], Typed { value: "+49-151-234".into(), kind: "cell".into() });
        let out = write(&c);
        for line in [
            "N:Park;Zoe;Marie;Dr.;Jr.",
            "item1.TEL;TYPE=WORK,FAX;PREF=1:+1 555 0100",
            "item1.X-ABLabel:Studio fax",
            "EMAIL;TYPE=INTERNET,WORK;PREF=1:zoe@studio.example",
            "TEL;VALUE=uri;TYPE=cell:tel:+49-151-234",
        ] {
            assert!(out.lines().any(|l| l == line), "{line}:\n{out}");
        }
        let mut changed = c.clone();
        changed.phones[0].kind = "home".into();
        changed.given = "Zoë".into();
        let out = write(&changed);
        assert!(out.lines().any(|l| l == "item1.TEL;PREF=1;TYPE=home:+1 555 0100"), "{out}");
        assert!(out.lines().any(|l| l == "N:Park;Zoë;Marie;Dr.;Jr."), "{out}");
        // A card made here is written as before.
        let mine = Card { name: "Ann Lee".into(), given: "Ann".into(), family: "Lee".into(), ..Default::default() };
        assert!(write(&mine).lines().any(|l| l == "N:Lee;Ann;;;"));
    }

    /// The name's other parts read as text for the servers that keep them in fields of their
    /// own, and are set from those fields back into the card's `N`; a hostile rest is bounded.
    #[test]
    fn the_names_other_parts_read_and_set() {
        let c = parse("BEGIN:VCARD\nFN:Dr. Zoe Park\nN:Park;Zoe;Marie\\, Ann;Dr.;Jr.\nEND:VCARD\n").remove(0);
        assert_eq!(c.name_extras(), ["Marie, Ann".to_string(), "Dr.".to_string(), "Jr.".to_string()]);
        let mut d = Card { name: "Zoe Park".into(), ..Default::default() };
        d.set_name_extras("Marie, Ann", "Dr.", "Jr.");
        assert_eq!(d.name_rest, "Marie\\, Ann;Dr.;Jr.");
        assert_eq!(d.name_extras(), c.name_extras());
        d.set_name_extras(" ", "", "");
        assert_eq!(d.name_rest, "", "nothing to keep");
        d.set_name_extras("", "", "Jr.");
        assert!(write(&d).lines().any(|l| l == "N:;;;;Jr."), "{}", write(&d));
        let long = format!("BEGIN:VCARD\nFN:A\nN:A;B;{}\nEND:VCARD\n", "x".repeat(5000));
        assert_eq!(parse(&long)[0].name_rest, "", "too long to keep");
        d.set_name_extras(&"y".repeat(5000), "", "");
        assert!(d.name_rest.len() < 1100);
    }
}
