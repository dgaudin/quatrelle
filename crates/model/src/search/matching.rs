// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Checking a message against a [`Query`]: the words the index narrows by, and each clause.
//! Both sides are read [folded](fold_word): `resume` finds "Résumé", `résumé` "resume".

use super::fold::fold_spaced;
use super::*;

impl Query {
    /// The words the index can narrow by: words of positive text clauses, [folded](fold_word)
    /// as the index holds them.
    pub fn index_words(&self) -> Vec<String> {
        let mut v = Vec::new();
        for c in self.clauses.iter().filter(|c| !c.negated) {
            match &c.term {
                Term::Words(_, w) => v.extend(w.iter().map(|w| fold_word(w).into_owned()).filter(|w| !w.is_empty())),
                Term::Phrase(_, p) => v.extend(tokens(p).map(Cow::into_owned)),
                _ => {}
            }
        }
        v
    }

    /// Whether a message dated `t` meets the query's dates (`received:`, `after:`, `before:`,
    /// once [resolved](Query::resolve)); a query without any holds for every date. What a server
    /// found for the query's words is checked by this, as the servers aren't asked the dates.
    pub fn dates_hold(&self, t: i64) -> bool {
        self.clauses.iter().all(|c| match &c.term {
            Term::Date(spec) => spec.holds(t).unwrap_or(false) != c.negated,
            _ => true,
        })
    }

    pub fn matches(&self, doc: &Doc<'_>) -> bool {
        self.clauses.iter().all(|c| clause_matches(&c.term, doc) != c.negated)
    }

    /// Whether checking a message found through the index needs its body: a phrase, words in the
    /// body alone, words that must be missing, or addresses (matched inside words). Plain words
    /// looked for anywhere are settled by the index.
    pub fn needs_body(&self) -> bool {
        self.clauses.iter().any(|c| match &c.term {
            Term::Phrase(Field::Any | Field::Body, _) | Term::Words(Field::Body, _) => true,
            Term::Words(Field::Any, ws) => c.negated || ws.iter().any(|w| w.contains(['@', '.'])),
            _ => false,
        })
    }
}

fn field_text(field: &Field, doc: &Doc<'_>) -> String {
    let m = doc.summary;
    match field {
        Field::From => addr_text(std::slice::from_ref(&m.from)),
        Field::To => {
            let mut s = addr_text(&m.to);
            s.push_str(&addr_text(&m.cc));
            s
        }
        Field::Subject => m.subject.clone(),
        Field::Body => doc.body.unwrap_or(&m.preview).to_string(),
        Field::Any => {
            let mut s = String::with_capacity(256);
            s.push_str(&m.subject);
            s.push(' ');
            s.push_str(&addr_text(std::slice::from_ref(&m.from)));
            s.push_str(&addr_text(&m.to));
            s.push_str(&addr_text(&m.cc));
            s.push_str(doc.body.unwrap_or(&m.preview));
            s
        }
    }
}

fn clause_matches(term: &Term, doc: &Doc<'_>) -> bool {
    let m = doc.summary;
    match term {
        Term::HasAttachment(b) => m.has_attachments == *b,
        Term::Unread(b) => m.flags.seen != *b,
        Term::Flagged(b) => m.flags.flagged == *b,
        Term::Keyword(k) => m.flags.has(k) || m.flags.has(&crate::keyword_for(k)),
        Term::Complete => m.flags.follow_up_complete(),
        Term::Important => m.importance > 0,
        Term::Awaiting => doc.awaiting,
        // `sent:` and `received:` both read the list's date; nothing holds before it's placed.
        Term::Date(spec) => spec.holds(m.date).unwrap_or(false),
        Term::Categorized(ks) if ks.is_empty() => m.flags.keywords.iter().any(|k| !k.starts_with('$')),
        Term::Categorized(ks) => ks.iter().any(|k| m.flags.has(k)),
        Term::Size(true, b) => u64::from(m.size) > *b,
        Term::Size(false, b) => u64::from(m.size) < *b,
        Term::AnyOf(ts) => ts.iter().any(|t| clause_matches(t, doc)),
        // Folded a letter at a time on both sides, white space as one space: the phrase's signs
        // stay as typed, and a line break in the text is a space.
        Term::Phrase(f, p) => fold_spaced(&field_text(f, doc)).contains(fold_spaced(p).as_str()),
        // Found through the index for these words: they're there (summary or body).
        Term::Words(Field::Any, ws) if doc.indexed && doc.body.is_none() && ws.iter().all(|w| !w.contains(['@', '.'])) => true,
        Term::Words(f, ws) => {
            let text = field_text(f, doc);
            // A word, as one of the parts of one too (`word` in "site.word"), as the index has it.
            let have: Vec<Cow<'_, str>> = words_and_parts(&text).collect();
            // An address is also found inside a word (`ana@studio.example` in `<ana@…>`).
            let inside = ws.iter().any(|w| w.contains(['@', '.'])).then(|| fold_word(&text));
            ws.iter().all(|w| {
                let w = fold_word(w);
                have.iter().any(|h| h.starts_with(w.as_ref())) || (w.contains(['@', '.']) && inside.as_ref().is_some_and(|t| t.contains(w.as_ref())))
            })
        }
    }
}
