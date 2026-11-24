// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Character references (`&amp;`, `&#38;`, `&#x26;`) made characters, one way for every pass over
//! a message's HTML that reads them, the way a browser reads them: the named ones from the HTML
//! standard's own table ([`html5ever::data::NAMED_ENTITIES`], longest name first, the legacy forms
//! without `;` as the standard accepts them), the numbered ones of any length with or without
//! `;`, `&#128;`…`&#159;` by its C1 replacements, and U+FFFD for what isn't a character. The same
//! address now reads the same in every pass (a picture's `&#38;` was read in its text, not in the
//! list of pictures to load, and the picture never came; a link's `&amp;#38;` was read twice).

use std::borrow::Cow;

/// The longest name a named reference has, its `;` included (`CounterClockwiseContourIntegral;`):
/// 32 bytes. What is read after a `&` is bounded to this many, not to the end of the text.
const NAME_MAX: usize = 32;

/// How [`decode_refs`] reads what isn't one of the references it makes a character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refs {
    /// As a browser reads an attribute's value: every reference, named or numbered, made its
    /// character (a legacy name without `;` not when an `=` or a letter follows it, as in an
    /// attribute); one that is no character is U+FFFD.
    Value,
    /// For the words in a text: every reference made its character too, except that one which
    /// shows nothing — a Unicode space, a format character (ZWNJ, ZWJ, SHY, LRM, U+034F…) a
    /// preview line is padded with — is a space, so that padding parts words. A letter never is.
    Words,
}

/// `s` with its character references made characters, as `refs` says. `s` itself when it has no
/// `&`. Never panics; what is read after a `&` is bounded to the few bytes a reference takes.
pub(crate) fn decode_refs(s: &str, refs: Refs) -> Cow<'_, str> {
    if !s.contains('&') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(rest.get(..at).unwrap_or(""));
        let after = rest.get(at + 1..).unwrap_or("");
        match reference(after, refs) {
            Some((made, took)) => {
                out.push_str(&made);
                rest = after.get(took..).unwrap_or("");
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// The character reference at the start of `after` (just past its `&`), as `refs` reads it, and how
/// many bytes of `after` it takes. `None` when it isn't one: the `&` stays as written.
fn reference(after: &str, refs: Refs) -> Option<(String, usize)> {
    let bytes = after.as_bytes();
    if bytes.first() == Some(&b'#') {
        // A number, decimal or hex, of any length, with or without `;`.
        let mut i = 1;
        let hex = matches!(bytes.get(i), Some(b'x' | b'X'));
        if hex {
            i += 1;
        }
        let digits = i;
        let mut n: u64 = 0;
        let mut too_big = false;
        while let Some(&b) = bytes.get(i) {
            match (b as char).to_digit(if hex { 16 } else { 10 }) {
                Some(d) => {
                    n = n.saturating_mul(u64::from(if hex { 16u32 } else { 10u32 })).saturating_add(u64::from(d));
                    too_big |= n > 0x10FFFF;
                    i += 1;
                }
                None => break,
            }
        }
        if i == digits {
            return None; // no digit: `&#` stays as written
        }
        if bytes.get(i) == Some(&b';') {
            i += 1;
        }
        return Some((word_char(numeric_char(n, too_big), refs).to_string(), i));
    }
    // A name: its letters and digits, up to the longest name there is, plus a `;` if it follows.
    let mut end = 0;
    while end < NAME_MAX && bytes.get(end).is_some_and(u8::is_ascii_alphanumeric) {
        end += 1;
    }
    if end < NAME_MAX && bytes.get(end) == Some(&b';') {
        end += 1;
    }
    let name = after.get(..end).unwrap_or("");
    // The longest name the table knows (its prefixes are in it too, mapped to no character).
    let mut best: Option<(usize, u32, u32)> = None;
    for len in (1..=name.len()).rev() {
        if let Some(&(c1, c2)) = html5ever::data::NAMED_ENTITIES.get(name.get(..len).unwrap_or(""))
            && c1 != 0
        {
            best = Some((len, c1, c2));
            break;
        }
    }
    let (len, c1, c2) = best?;
    let semicolon = name.as_bytes().get(len.wrapping_sub(1)) == Some(&b';');
    if !semicolon {
        // A legacy name without `;`: in a value, not made when an `=` or a letter follows it.
        let next = name.as_bytes().get(len).copied().or_else(|| bytes.get(end).copied());
        if refs == Refs::Value && next.is_some_and(|b| b == b'=' || b.is_ascii_alphanumeric()) {
            return None;
        }
    }
    let mut made = String::new();
    made.push(word_char(char::from_u32(c1).unwrap_or('\u{fffd}'), refs));
    if c2 != 0 {
        made.push(word_char(char::from_u32(c2).unwrap_or('\u{fffd}'), refs));
    }
    Some((made, len))
}

/// The character a numeric reference's value stands for, as a browser reads it: U+FFFD for what
/// isn't a character, the C1 replacements for `&#128;`…`&#159;`.
fn numeric_char(n: u64, too_big: bool) -> char {
    if too_big || n > 0x10FFFF || n == 0 || (0xD800..=0xDFFF).contains(&n) {
        return '\u{fffd}';
    }
    if (0x80..=0x9F).contains(&n) {
        return html5ever::data::C1_REPLACEMENTS[(n - 0x80) as usize].unwrap_or_else(|| char::from_u32(n as u32).unwrap_or('\u{fffd}'));
    }
    char::from_u32(n as u32).unwrap_or('\u{fffd}')
}

/// The character as `Refs::Words` reads it.
fn word_char(c: char, refs: Refs) -> char {
    match refs {
        Refs::Value => c,
        Refs::Words if invisible(c) => ' ',
        Refs::Words => c,
    }
}

/// Whether `c` shows nothing: a Unicode space, or a format character (Cf) like ZWNJ, ZWJ, SHY,
/// LRM or U+034F.
fn invisible(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '\u{00ad}' | '\u{034f}' | '\u{061c}' | '\u{180e}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206f}' | '\u{feff}'
                | '\u{fff9}'..='\u{fffb}' | '\u{e0001}' | '\u{e0020}'..='\u{e007f}'
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Text made of what references are made of, so that most `&` start one.
    fn refs_text() -> impl Strategy<Value = String> {
        let piece = prop_oneof![
            Just("&".to_string()),
            Just(";".to_string()),
            Just("#".to_string()),
            Just("x".to_string()),
            Just("=".to_string()),
            "[0-9]{1,20}",
            "[a-fA-F0-9]{1,6}",
            prop_oneof![
                Just("amp"),
                Just("lt"),
                Just("gt"),
                Just("quot"),
                Just("apos"),
                Just("nbsp"),
                Just("shy"),
                Just("thinsp"),
                Just("commat"),
                Just("colon"),
                Just("Pcy"),
                Just("eacute"),
                Just("bogus")
            ]
            .prop_map(String::from),
            "[a-z \u{e9}\u{20ac}+-]{1,4}",
        ];
        proptest::collection::vec(piece, 0..24).prop_map(|p| p.concat())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(4000))]

        /// Reading never grows the text (every reference is at least as long as what it becomes),
        /// and never panics, whatever the pieces.
        #[test]
        fn reading_is_bounded_and_never_panics(s in refs_text()) {
            for refs in [Refs::Value, Refs::Words] {
                let out = decode_refs(&s, refs).into_owned();
                prop_assert!(out.len() <= s.len(), "{s:?} → {out:?}");
            }
        }
    }

    /// A numbered reference of any length is read in a value too, with or without `;`.
    #[test]
    fn long_numbered_references_are_read() {
        assert_eq!(decode_refs("a&#0000000038;b", Refs::Value), "a&b");
        assert_eq!(decode_refs("a&#0000000038;b", Refs::Words), "a&b");
        assert_eq!(decode_refs("&#64", Refs::Value), "@");
        assert_eq!(decode_refs("&#0000000000064;", Refs::Value), "@");
        assert_eq!(decode_refs("&#x0000000041;", Refs::Value), "A");
        assert_eq!(decode_refs("&#0;", Refs::Value), "\u{fffd}");
        assert_eq!(decode_refs("&#xD800;", Refs::Value), "\u{fffd}");
        assert_eq!(decode_refs("&#1114112;", Refs::Value), "\u{fffd}");
        // `&#128;`…`&#159;` by the C1 replacements.
        assert_eq!(decode_refs("&#128;", Refs::Value), "\u{20ac}");
        assert_eq!(decode_refs("&#159;", Refs::Value), "\u{0178}");
        // No digit: not a reference.
        assert_eq!(decode_refs("&#;", Refs::Value), "&#;");
        assert_eq!(decode_refs("&#x;", Refs::Value), "&#x;");
    }

    /// Named references of the standard's table: with `;`, and the legacy forms without it.
    #[test]
    fn named_references_read_as_a_browser_reads_them() {
        assert_eq!(decode_refs("&commat;", Refs::Value), "@");
        assert_eq!(decode_refs("&colon;", Refs::Value), ":");
        assert_eq!(decode_refs("&eacute;", Refs::Value), "é");
        assert_eq!(decode_refs("pr&eacute;sent", Refs::Words), "présent");
        assert_eq!(decode_refs("&Pcy;&rcy;", Refs::Words), "Пр", "a letter is a word, not a space");
        assert_eq!(decode_refs("&iopf;&gopf;", Refs::Words), "\u{1d55a}\u{1d558}");
        // Legacy without `;`: read in a text always, and in a value unless an `=` or a letter
        // follows it.
        assert_eq!(decode_refs("&amp x", Refs::Words), "& x");
        assert_eq!(decode_refs("&amp x", Refs::Value), "& x", "a space follows: read");
        assert_eq!(decode_refs("&amp=", Refs::Value), "&amp=");
        assert_eq!(decode_refs("&ampx", Refs::Value), "&ampx", "a letter follows: as written");
        assert_eq!(decode_refs("&ampx", Refs::Words), "&x");
        assert_eq!(decode_refs("&amp;", Refs::Value), "&");
        assert_eq!(decode_refs("&bogus;", Refs::Value), "&bogus;", "no such name: as written");
        assert_eq!(decode_refs("&bogus;", Refs::Words), "&bogus;");
        // Two code points where a name has two.
        assert_eq!(decode_refs("&NotEqualTilde;", Refs::Value), "\u{2242}\u{0338}");
    }

    #[test]
    fn values_and_words_read_their_own_way() {
        let s = "a&amp;b&#38;c&#x26;d&nbsp;e&#8203;f&bogus;g&#;h& i&#0;";
        assert_eq!(decode_refs(s, Refs::Value), "a&b&c&d\u{a0}e\u{200b}f&bogus;g&#;h& i\u{fffd}");
        assert_eq!(decode_refs(s, Refs::Words), "a&b&c&d e f&bogus;g&#;h& i\u{fffd}");
        assert!(matches!(decode_refs("no references", Refs::Value), Cow::Borrowed(_)));
        // A name longer than the longest there is isn't looked for past it.
        let long = format!("&{};", "a".repeat(40));
        assert_eq!(decode_refs(&long, Refs::Words), long);
    }
}
