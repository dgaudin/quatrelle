// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The reminder of an attachment said and missing: the words that say something goes with a
//! message, read in what the user wrote of it alone.

use super::*;

/// What a message says when something goes with it, folded (accents gone, lower case), each
/// with how it is bounded in the text ([`Edge`]): the languages the interface speaks. A message
/// carrying none of these and nothing attached goes after a look
/// ([`SendCheck::MissingAttachment`]).
const ATTACHMENT_PHRASES: &[(&str, Edge)] = &[
    // English.
    ("attached", Edge::Whole),
    ("attachment", Edge::Whole),
    ("attaching", Edge::Whole),
    ("enclosed", Edge::Whole),
    ("enclosure", Edge::Whole),
    // French.
    ("ci-joint", Edge::Whole),
    ("ci joint", Edge::Whole),
    ("ci-jointes", Edge::Whole),
    ("ci jointes", Edge::Whole),
    ("piece jointe", Edge::Whole),
    ("pieces jointes", Edge::Whole),
    ("fichier joint", Edge::Whole),
    ("fichiers joints", Edge::Whole),
    ("en annexe", Edge::Whole),
    // Spanish.
    ("adjunto", Edge::Whole),
    ("adjunta", Edge::Whole),
    ("archivo adjunto", Edge::Whole),
    ("documento adjunto", Edge::Whole),
    // German.
    ("anbei", Edge::Whole),
    ("anhang", Edge::Whole),
    ("angehangt", Edge::Whole),
    // Italian.
    ("allegato", Edge::Whole),
    ("allegata", Edge::Whole),
    ("in allegato", Edge::Whole),
    // Portuguese.
    ("em anexo", Edge::Whole),
    ("anexo", Edge::Whole),
    // Dutch.
    ("bijgevoegd", Edge::Whole),
    // Russian (folded: lower case, ё read as е): a noun's cases and a verb stem's endings run
    // on, so the boundary is before the phrase only. "приложение" (also "application") is left
    // out.
    ("вложени", Edge::Start),
    ("вложенн", Edge::Start),
    ("прилагаю", Edge::Whole),
    ("прикрепл", Edge::Start),
    // Arabic.
    ("مرفق", Edge::Prefixed(ARABIC_PREFIXES)),
    ("مرفقات", Edge::Prefixed(ARABIC_PREFIXES)),
    ("أرفق", Edge::PrefixedOpen(ARABIC_PREFIXES)),
    ("ارفق", Edge::PrefixedOpen(ARABIC_PREFIXES)),
    // Hebrew.
    ("מצترف", Edge::Prefixed(HEBREW_PREFIXES)),
    ("מצורפים", Edge::Prefixed(HEBREW_PREFIXES)),
    ("מצורפת", Edge::Prefixed(HEBREW_PREFIXES)),
    ("מצורפות", Edge::Prefixed(HEBREW_PREFIXES)),
    ("צירף", Edge::PrefixedOpen(HEBREW_PREFIXES)),
    ("צירפ", Edge::PrefixedOpen(HEBREW_PREFIXES)),
    // Japanese: written without spaces, so no boundary at all.
    ("添付", Edge::Free),
    ("別添", Edge::Free),
    ("同封", Edge::Free),
    // Korean: a boundary before only, a particle being glued behind the word (첨부파일을).
    ("첨부", Edge::Start),
    ("동봉", Edge::Start),
    // Chinese, simplified and traditional: no boundary; 附 alone is too short.
    ("附件", Edge::Free),
    ("附上", Edge::Free),
];

/// The one-letter prefixes Arabic glues in front of a word: the conjunctions and prepositions
/// و ف ب ل ك, and the ا of the article ال.
const ARABIC_PREFIXES: &[char] = &['و', 'ف', 'ب', 'ل', 'ك', 'ا'];

/// Hebrew's: ו ב ש ה כ ל מ.
const HEBREW_PREFIXES: &[char] = &['ו', 'ב', 'ש', 'ה', 'כ', 'ל', 'מ'];

/// At most this many glued prefix letters may stand before a phrase ([`Edge::Prefixed`]).
const MAX_PREFIX_RUN: usize = 4;

/// How a phrase is bounded in the folded text it is looked for in.
#[derive(Clone, Copy)]
enum Edge {
    /// A whole word, as the Latin-script and Cyrillic languages write theirs: the characters
    /// either side must not be alphanumeric ("unattached" isn't a promise of anything).
    Whole,
    /// No boundary at all: Han and kana are written without spaces, so a phrase's neighbours are
    /// letters and nothing would ever match.
    Free,
    /// A boundary before only: Hangul glues its particles behind a word ("첨부파일을"), and a
    /// Russian verb stem carries its ending ("прикреплённый").
    Start,
    /// A boundary after, and before it a short run of the one-letter prefixes Arabic and Hebrew
    /// glue in front of a word ("والمرفق", "הקובץ מצترف").
    Prefixed(&'static [char]),
    /// As [`Edge::Prefixed`], with anything after: an Arabic or Hebrew verb takes its ending or a
    /// glued pronoun ("أرفقه", "צירפתי").
    PrefixedOpen(&'static [char]),
}

impl Edge {
    /// Whether the phrase at `at..at + len` in the folded `text` counts.
    fn allows(self, text: &str, at: usize, len: usize) -> bool {
        match self {
            Edge::Whole => !letter_before(text, at) && !letter_after(text, at + len),
            Edge::Free => true,
            Edge::Start => !letter_before(text, at),
            Edge::Prefixed(prefixes) => !prefix_blocked(text, at, prefixes) && !letter_after(text, at + len),
            Edge::PrefixedOpen(prefixes) => !prefix_blocked(text, at, prefixes),
        }
    }
}

/// Whether a letter stands just before `at` in `text`.
fn letter_before(text: &str, at: usize) -> bool {
    text.get(..at).and_then(|s| s.chars().next_back()).is_some_and(char::is_alphanumeric)
}

/// Whether a letter stands just after `at` in `text`.
fn letter_after(text: &str, at: usize) -> bool {
    text.get(at..).and_then(|s| s.chars().next()).is_some_and(char::is_alphanumeric)
}

/// Whether, walking back over at most [`MAX_PREFIX_RUN`] glued prefix letters from `at`, a
/// letter still stands before them: the phrase is then part of a longer word, not a prefixed
/// one.
fn prefix_blocked(text: &str, mut at: usize, prefixes: &[char]) -> bool {
    for _ in 0..MAX_PREFIX_RUN {
        let Some(c) = text.get(..at).and_then(|s| s.chars().next_back()) else { break };
        if !prefixes.contains(&c) {
            break;
        }
        at -= c.len_utf8();
    }
    letter_before(text, at)
}

/// Whether what was written says something goes with the message. Only whole words count:
/// "unattached" isn't a promise of anything, and neither is a link — except where a script
/// glues letters onto a word, which [`Edge`] says per phrase.
pub(super) fn mentions_attachment(subject: &str, body: &str) -> bool {
    let whole = format!("{subject}\n{body}");
    let text = quatrelle_model::search::fold_word(&whole);
    ATTACHMENT_PHRASES.iter().any(|(phrase, edge)| text.match_indices(phrase).any(|(i, _)| edge.allows(&text, i, phrase.len())))
}

/// What the user wrote of message `c`: its text without the signature put in it (its text as it
/// went in, or where one written in HTML stands) and without an original quoted under it, from
/// its header on (who sent it, when, to whom, its subject, written in `words`), whether or not the
/// message still knows it's a reply (a draft saved elsewhere doesn't); and its subject, empty
/// when it's the original's (`Re: …` as the reply came).
pub(super) fn own_words<'a>(c: &'a Compose, words: &mime::QuoteWords) -> (String, &'a str) {
    let mut text = c.body.as_str();
    let mut subject = c.subject.as_str();
    if let Some((at, original)) = quote_header(text, words) {
        text = text.get(..at).unwrap_or(text);
        if quatrelle_model::thread::base_subject(subject).eq_ignore_ascii_case(&quatrelle_model::thread::base_subject(original)) {
            subject = "";
        }
    }
    let mut text = text.replace(mime::rich::SIGNATURE, "");
    let signature = c.signature.trim();
    if !signature.is_empty()
        && let Some(i) = text.find(signature)
    {
        text.replace_range(i..i + signature.len(), "");
    }
    (text, subject)
}

/// Where the original's header starts in a reply's text (a line `From: …`, in `words`, with
/// `Sent: …` right under it), and the subject it gives.
fn quote_header<'a>(text: &'a str, words: &mime::QuoteWords) -> Option<(usize, &'a str)> {
    let (from, sent, subject) = (format!("{} ", words.from), format!("{} ", words.sent), format!("{} ", words.subject));
    let mut at = 0;
    let mut lines = text.split_inclusive('\n').peekable();
    while let Some(line) = lines.next() {
        if line.starts_with(&from) && lines.peek().is_some_and(|next| next.starts_with(&sent)) {
            let header = text.get(at..).unwrap_or_default();
            let original = header.lines().take(8).find_map(|l| l.strip_prefix(&subject)).unwrap_or_default();
            return Some((at, original.trim()));
        }
        at += line.len();
    }
    None
}

#[cfg(test)]
mod tests;
