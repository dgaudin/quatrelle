// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! How search reads words: split at UAX #29 word boundaries, then folded, lower-cased with their
//! accents gone and a few letters spelled out (ß → ss, æ → ae, ø → o, ł → l, ё → е, fullwidth
//! forms → ASCII), so `resume` finds "Résumé" and `strasse` "Straße". The same fold is applied to
//! what's searched and to what's typed; a server is still asked for what was typed
//! ([`super::words`]). Cyrillic keeps its letters but ё (й isn't и), and other scripts (Arabic,
//! Hebrew, Indic, CJK) pass through as they are. A typographic apostrophe is read as a straight
//! one, and the parts of words joined by `.`, `:`, an apostrophe or `_` are words too ([`parts`]):
//! `equipe` finds "l’équipe", `word` "site.word".
//!
//! The letters come from a table made by `cargo xtask fold` from Unicode's decompositions
//! ([`super::fold_table`]), so no normalizer is carried at run time (nor in the browser build).

use std::borrow::Cow;

use unicode_segmentation::UnicodeSegmentation;

use super::fold_table::FOLD;

/// Longer "words" (base64 debris, long links) aren't indexed: they only bloat it.
pub const MAX_TOKEN_CHARS: usize = 40;

/// An accent that letters carry (combining diacritical marks), dropped: so text written with
/// its accents apart (macOS file names, NFD) folds as written together, and `İ` (lower-cased to
/// `i` and a dot) is `i`.
fn is_mark(c: char) -> bool {
    ('\u{300}'..='\u{36F}').contains(&c)
}

/// A word (or any text, a letter at a time) folded: lower-case, accents gone, letters spelled
/// out. Borrowed when it's lower-case ASCII already, the common case.
pub fn fold_word(w: &str) -> Cow<'_, str> {
    if w.is_ascii() {
        return if w.bytes().any(|b| b.is_ascii_uppercase()) { Cow::Owned(w.to_ascii_lowercase()) } else { Cow::Borrowed(w) };
    }
    let mut out = String::with_capacity(w.len());
    let mut rest = w;
    while !rest.is_empty() {
        // A run of ASCII is copied in one go, lower-cased where it lands (most of a text, even
        // one with accents: folding a letter at a time made a calendar search four times slower).
        let ascii = rest.bytes().position(|b| !b.is_ascii()).unwrap_or(rest.len());
        let Some((run, tail)) = rest.split_at_checked(ascii) else { break };
        let start = out.len();
        out.push_str(run);
        if let Some(added) = out.get_mut(start..) {
            added.make_ascii_lowercase();
        }
        let mut chars = tail.chars();
        let Some(c) = chars.next() else { break };
        rest = chars.as_str();
        push_folded(c, &mut out);
    }
    Cow::Owned(out)
}

/// One letter that isn't ASCII, folded onto `out` (nothing for an accent written apart).
fn push_folded(c: char, out: &mut String) {
    // "l’équipe" is "l'équipe", as typed on a keyboard.
    let c = if matches!(c, '\u{2018}' | '\u{2019}') { '\'' } else { c };
    for l in c.to_lowercase().filter(|l| !is_mark(*l)) {
        match FOLD.binary_search_by(|(k, _)| k.cmp(&l)).ok().and_then(|i| FOLD.get(i)) {
            Some((_, s)) => out.push_str(s),
            None => out.push(l),
        }
    }
}

/// A text [folded](fold_word), and where each byte of the fold comes from: the start, in `text`,
/// of the letter it was folded from (what was found in the fold is shown in the text as written:
/// "equipe" found in "l'équipe" lights up "équipe"). The fold is [`fold_word`]'s, byte for byte.
pub fn fold_mapped(text: &str) -> (String, Vec<u32>) {
    let mut out = String::with_capacity(text.len());
    let mut from: Vec<u32> = Vec::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        let at = u32::try_from(text.len() - rest.len()).unwrap_or(u32::MAX);
        // A run of ASCII in one go, a byte for a byte (as `fold_word` copies it).
        let ascii = rest.bytes().position(|b| !b.is_ascii()).unwrap_or(rest.len());
        let Some((run, tail)) = rest.split_at_checked(ascii) else { break };
        let start = out.len();
        out.push_str(run);
        if let Some(added) = out.get_mut(start..) {
            added.make_ascii_lowercase();
        }
        from.extend((0..run.len()).map(|i| at.saturating_add(u32::try_from(i).unwrap_or(u32::MAX))));
        let mut chars = tail.chars();
        let Some(c) = chars.next() else { break };
        rest = chars.as_str();
        let at = u32::try_from(text.len() - tail.len()).unwrap_or(u32::MAX);
        push_folded(c, &mut out);
        from.resize(out.len(), at);
    }
    (out, from)
}

/// A text [folded](fold_word), each run of white space one space: what a phrase is looked for
/// in (and the phrase itself), so it's found where the text breaks its line (HTML mail made text
/// wraps its lines).
pub(super) fn fold_spaced(text: &str) -> String {
    let folded = fold_word(text);
    let mut out = String::with_capacity(folded.len());
    let mut space = false;
    for c in folded.chars() {
        if c.is_whitespace() {
            if !space {
                out.push(' ');
            }
            space = true;
        } else {
            out.push(c);
            space = false;
        }
    }
    out
}

/// A text's words, folded, as search indexes them: none empty, none longer than
/// [`MAX_TOKEN_CHARS`].
pub fn tokens(text: &str) -> impl Iterator<Item = Cow<'_, str>> {
    all_words(text).filter(|w| w.len() <= MAX_TOKEN_CHARS || w.chars().count() <= MAX_TOKEN_CHARS)
}

/// A text's words, folded, long ones too (what a message's own fields are checked against).
fn all_words(text: &str) -> impl Iterator<Item = Cow<'_, str>> {
    text.unicode_words().map(fold_word).filter(|w| !w.is_empty())
}

/// A text's [`tokens`], each followed by its [`parts`]: the words search indexes, so `word` finds
/// "site.word".
pub fn index_tokens(text: &str) -> impl Iterator<Item = Cow<'_, str>> {
    words_and_parts(text).filter(|w| w.len() <= MAX_TOKEN_CHARS || w.chars().count() <= MAX_TOKEN_CHARS)
}

/// A text's words, folded, long ones too, each followed by its [`parts`]: what a text is checked
/// against, here and in the calendar's search.
pub(crate) fn words_and_parts(text: &str) -> impl Iterator<Item = Cow<'_, str>> {
    with_parts(all_words(text))
}

/// Words, each followed by its [`parts`].
fn with_parts<'a>(words: impl Iterator<Item = Cow<'a, str>>) -> impl Iterator<Item = Cow<'a, str>> {
    words.flat_map(|w| {
        // Most words join nothing: no list made for them.
        let parts: Vec<Cow<'a, str>> = if w.contains(joins) { parts(&w).map(|p| Cow::Owned(p.to_string())).collect() } else { Vec::new() };
        std::iter::once(w).chain(parts)
    })
}

/// Whether UAX #29 keeps `c` inside a word, between letters (MidLetter, MidNumLet) or joining
/// its pieces (ExtendNumLet): "site.word", "l'avion", "ratio:word" and "snake_case" are each
/// one word. Between digits only (MidNum: `10,000`) isn't split.
fn joins(c: char) -> bool {
    matches!(
        c,
        '.' | ':'
            | '\''
            | '_'
            | '\u{b7}'
            | '\u{387}'
            | '\u{55f}'
            | '\u{5f4}'
            | '\u{2018}'
            | '\u{2019}'
            | '\u{2024}'
            | '\u{2027}'
            | '\u{203f}'
            | '\u{2040}'
            | '\u{2054}'
            | '\u{fe13}'
            | '\u{fe33}'
            | '\u{fe34}'
            | '\u{fe4d}'..='\u{fe4f}' | '\u{fe52}' | '\u{fe55}' | '\u{ff07}' | '\u{ff0e}' | '\u{ff1a}' | '\u{ff3f}'
    )
}

/// The pieces of a folded word that joins them ([`joins`]), each found by itself too: a word
/// seen only as "xx.word" was never found by `word`. None for a word that doesn't, and none of
/// one letter (the `t` of "don't", the `l` of "l'avion").
pub fn parts(w: &str) -> impl Iterator<Item = &str> {
    let split = w.contains(joins);
    w.split(joins).filter(move |p| split && p.chars().nth(1).is_some())
}
