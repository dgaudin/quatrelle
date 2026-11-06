// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The spelling of the message being written: what it's checked by, and its misspelt words.

use super::*;

/// What the spelling of `c` is checked by: its language (the interface's unless one was chosen
/// for it), leaving out its signature and the original under a reply or forward, and the words
/// let be in it (Ignore all).
pub(crate) fn check_for(c: &Compose) -> crate::spellcheck::Check {
    let langs = effective_language(c).into_iter().collect();
    let mut skip = Vec::new();
    if !c.signature.is_empty()
        && let Some(b) = c.body.rfind(&c.signature)
    {
        skip.push(b..b + c.signature.len());
    }
    // The original under a reply or forward: from its rule to the end.
    let mut at = 0usize;
    for (n, line) in c.body.split('\n').enumerate() {
        if c.styles.para(n).rule {
            skip.push(at..c.body.len());
            break;
        }
        at += line.len() + 1;
    }
    crate::spellcheck::Check { langs, skip, ignored: crate::spellcheck::ignored(c.id) }
}

/// The misspelt words of the message (its subject too, when `subject`), as its check says.
pub(crate) fn misspelt_in(c: &Compose, subject: bool) -> Vec<(Range<usize>, String)> {
    let check = check_for(c);
    if subject {
        let check = crate::spellcheck::Check { skip: Vec::new(), ..check };
        return crate::spellcheck::misspelt_with(&c.subject, &check);
    }
    crate::spellcheck::misspelt_with(&c.body, &check)
}

/// The word at `at` (a character index into the text), when it isn't spelt right, with its range
/// in characters: what the right-click menu offers to mend. A click between two words, past the
/// end, or on a word the dictionaries know gives nothing.
pub(super) fn misspelt_at(c: &Compose, at: usize) -> Option<(Range<usize>, String)> {
    let text = &c.body;
    let at = at.min(text.chars().count());
    let byte = text.char_indices().nth(at).map_or(text.len(), |(b, _)| b);
    misspelt_in(c, false).into_iter().find(|(r, _)| r.start <= byte && byte < r.end).map(|(r, word)| {
        let start = text.get(..r.start).unwrap_or("").chars().count();
        (start..start + word.chars().count(), word)
    })
}
