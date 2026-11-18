// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The reminder of an attachment said and missing: the words that say something goes with a
//! message, read in what the user wrote of it alone.

use super::*;

/// What a message says when something goes with it, folded (accents gone, lower case): the
/// languages the interface speaks. A message carrying none of these and nothing attached goes
/// after a look ([`SendCheck::MissingAttachment`]).
const ATTACHMENT_PHRASES: [&str; 27] = [
    "attached",
    "attachment",
    "attaching",
    "enclosed",
    "enclosure",
    "ci-joint",
    "ci joint",
    "ci-jointes",
    "ci jointes",
    "piece jointe",
    "pieces jointes",
    "fichier joint",
    "fichiers joints",
    "en annexe",
    "adjunto",
    "adjunta",
    "archivo adjunto",
    "documento adjunto",
    "anbei",
    "anhang",
    "angehangt",
    "allegato",
    "allegata",
    "in allegato",
    "em anexo",
    "anexo",
    "bijgevoegd",
];

/// Whether what was written says something goes with the message. Only whole words count:
/// "unattached" isn't a promise of anything, and neither is a link.
pub(super) fn mentions_attachment(subject: &str, body: &str) -> bool {
    let whole = format!("{subject}\n{body}");
    let text = quatrelle_model::search::fold_word(&whole);
    ATTACHMENT_PHRASES.iter().any(|p| {
        text.match_indices(p).any(|(i, _)| {
            let before = text.get(..i).and_then(|s| s.chars().next_back());
            let after = text.get(i + p.len()..).and_then(|s| s.chars().next());
            !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
        })
    })
}

/// What the user wrote of message `c`: its text without the signature put in it (its text as it
/// went in, or where one written in HTML stands) and, in a reply or a forward, without the
/// original under it, from its header on (who sent it, when, to whom, its subject, written in
/// `words`); and its subject, empty when it's the original's (`Re: …` as the reply came).
pub(super) fn own_words<'a>(c: &'a Compose, words: &mime::QuoteWords) -> (String, &'a str) {
    let mut text = c.body.as_str();
    let mut subject = c.subject.as_str();
    if c.mode.is_some()
        && let Some((at, original)) = quote_header(text, words)
    {
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
