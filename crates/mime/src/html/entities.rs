// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Character references (`&amp;`, `&#38;`, `&#x26;`) made characters, one way for every pass over
//! a message's HTML that reads them: each had its own, with its own rules, so the same address
//! read differently in different passes (a picture's `&#38;` was read in its text, not in the
//! list of pictures to load, and the picture never came; a link's `&amp;#38;` was read twice).

use std::borrow::Cow;

/// The longest name a reference is looked for with (`#1114111`, `#x10FFFF`, `thinsp`): its `;` is
/// looked for in the bytes after the `&` up to this many, not to the end of the text.
const NAME_MAX: usize = 11;

/// How [`decode_refs`] reads what isn't one of the references it makes a character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refs {
    /// As a browser reads an attribute's value: `&amp;`, `&lt;`, `&gt;`, `&quot;`, `&apos;` and
    /// numbered ones (`&#38;`, `&#x26;`; one that is no character `U+FFFD`). Other names stay as
    /// written.
    Value,
    /// For the words in a text: the five, and numbered ones that are a letter, digit or mark of
    /// punctuation; any other (`&nbsp;`, `&shy;`, `&zwj;`, a preview line's padding) a space, so
    /// that padding between words parts them.
    Words,
}

/// `s` with its character references made characters, as `refs` says. `s` itself when it has no
/// `&`. Never panics; each `;` is looked for in the few bytes a name takes.
pub(crate) fn decode_refs(s: &str, refs: Refs) -> Cow<'_, str> {
    if !s.contains('&') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(rest.get(..at).unwrap_or(""));
        let after = rest.get(at + 1..).unwrap_or("");
        // Its `;`, looked for in the bytes (`;` is ASCII: where it is, a character ends).
        let len = after.as_bytes().iter().take(NAME_MAX + 1).position(|b| *b == b';');
        match len.and_then(|len| Some((character(after.get(..len)?, refs)?, len))) {
            Some((c, len)) => {
                out.push(c);
                rest = after.get(len + 1..).unwrap_or("");
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

/// The character the reference named `name` (between its `&` and `;`) stands for, as `refs`
/// reads it; `None` to leave it as written.
fn character(name: &str, refs: Refs) -> Option<char> {
    let known = match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => None,
    };
    if known.is_some() {
        return known;
    }
    if let Some(n) = name.strip_prefix('#') {
        let code = match n.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16),
            None => n.parse::<u32>(),
        }
        .ok();
        return match refs {
            Refs::Value => code.map(|c| char::from_u32(c).filter(|c| *c != '\0').unwrap_or('\u{fffd}')),
            Refs::Words => Some(code.and_then(char::from_u32).filter(|c| c.is_alphanumeric() || c.is_ascii_punctuation()).unwrap_or(' ')),
        };
    }
    (refs == Refs::Words && !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric())).then_some(' ')
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// The decoder the hidden text was read with before [`decode_refs`], as it was.
    fn old_words(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut rest = s;
        while let Some(at) = rest.find('&') {
            out.push_str(rest.get(..at).unwrap_or(""));
            let after = rest.get(at + 1..).unwrap_or("");
            let Some(len) = after.as_bytes().iter().take(12).position(|b| *b == b';') else {
                out.push('&');
                rest = after;
                continue;
            };
            let name = after.get(..len).unwrap_or("");
            match name {
                "nbsp" | "ensp" | "emsp" | "thinsp" => out.push(' '),
                "lt" => out.push('<'),
                "gt" => out.push('>'),
                "quot" => out.push('"'),
                "apos" => out.push('\''),
                "amp" => out.push('&'),
                n if n.starts_with('#') => {
                    let code = match n.get(1..).and_then(|d| d.strip_prefix(['x', 'X']).map(|h| (h, 16)).or(Some((d, 10)))) {
                        Some((d, radix)) => u32::from_str_radix(d, radix).ok(),
                        None => None,
                    };
                    match code.and_then(char::from_u32) {
                        Some(c) if c.is_alphanumeric() || c.is_ascii_punctuation() => out.push(c),
                        _ => out.push(' '),
                    }
                }
                n if n.chars().all(|c| c.is_ascii_alphanumeric()) && !n.is_empty() => out.push(' '),
                _ => {
                    out.push('&');
                    rest = after;
                    continue;
                }
            }
            rest = after.get(len + 1..).unwrap_or("");
        }
        out.push_str(rest);
        out
    }

    /// Text made of what references are made of, so that most `&` start one.
    fn refs_text() -> impl Strategy<Value = String> {
        let piece = prop_oneof![
            Just("&".to_string()),
            Just(";".to_string()),
            Just("#".to_string()),
            Just("x".to_string()),
            "[0-9]{1,8}",
            "[a-fA-F0-9]{1,6}",
            prop_oneof![Just("amp"), Just("lt"), Just("gt"), Just("quot"), Just("apos"), Just("nbsp"), Just("shy"), Just("thinsp")]
                .prop_map(String::from),
            "[a-z \u{e9}\u{20ac}+-]{1,4}",
        ];
        proptest::collection::vec(piece, 0..24).prop_map(|p| p.concat())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(4000))]

        /// The words are read as they were.
        #[test]
        fn words_read_as_they_were(s in refs_text()) {
            prop_assert_eq!(decode_refs(&s, Refs::Words).into_owned(), old_words(&s));
        }
    }

    /// A numbered reference of 11 bytes is read in a value too.
    #[test]
    fn a_long_numbered_reference_is_read() {
        let v = "a&#0000000038;b";
        assert_eq!(decode_refs(v, Refs::Value), "a&b");
        assert_eq!(decode_refs(v, Refs::Words), "a&b");
    }

    #[test]
    fn values_and_words_read_their_own_way() {
        let s = "a&amp;b&#38;c&#x26;d&nbsp;e&#8203;f&bogus;g&#;h& i&#0;";
        assert_eq!(decode_refs(s, Refs::Value), "a&b&c&d&nbsp;e\u{200b}f&bogus;g&#;h& i\u{fffd}");
        assert_eq!(decode_refs(s, Refs::Words), "a&b&c&d e f g h& i ");
        assert!(matches!(decode_refs("no references", Refs::Value), Cow::Borrowed(_)));
        // A `;` far off isn't looked for to the end, and ends no reference.
        let long = format!("&{};", "a".repeat(20));
        assert_eq!(decode_refs(&long, Refs::Words), long);
    }
}
