// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Finding events (issue #33): what's typed in the calendar's search box.
//!
//! As the mail search box reads it: bare words match anywhere (as word prefixes, so `budg` finds
//! "Budget"), `"quoted phrases"` as written, and a field narrows them: `subject:` (`title:`),
//! `location:` (`where:`, `room:`), `notes:` (`body:`, `description:`), `organizer:` (`from:`),
//! `attendee:` (`to:`, `cc:`, `with:`), `category:`. A field takes a list, `organizer:ana,jun`
//! being either. `is:private` (`is:confidential`) and `is:recurring` pick those; `after:` and `before:` a date
//! (`YYYY-MM-DD`), its occurrences starting on or after it, or before it. A leading `-` turns any
//! of them round. The caller places the dates in its zone and finds the occurrences. Words are
//! read [folded](crate::search::fold_word) on both sides, with the parts of words joined by an
//! apostrophe or a dot, as the mail search reads them: `equipe` finds "Réunion d’Équipe".

use std::borrow::Cow;

use unicode_segmentation::UnicodeSegmentation;

use super::*;
use crate::search::{fold_word, words, words_and_parts};

/// Where in an event a term looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventField {
    Any,
    Subject,
    Location,
    Notes,
    Organizer,
    Attendees,
    Categories,
}

/// What a term looks for: words (each a word's start), or a phrase as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Needle {
    Words(Vec<String>),
    Phrase(String),
}

/// One part of a query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventTerm {
    /// Any one of these in the field.
    Text(EventField, Vec<Needle>),
    /// Kept private or confidential (`is:private`).
    Private,
    /// A series (`is:recurring`).
    Recurring,
}

/// A parsed calendar search.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventQuery {
    /// Each with whether it's turned round (`-`); all must hold.
    pub terms: Vec<(EventTerm, bool)>,
    /// The first day (days since the epoch, in the caller's zone) occurrences may start on.
    pub after: Option<i64>,
    /// The day occurrences must start before.
    pub before: Option<i64>,
}

/// The fields the box knows, and where each looks.
const FIELDS: &[(&str, EventField)] = &[
    ("subject", EventField::Subject),
    ("title", EventField::Subject),
    ("location", EventField::Location),
    ("where", EventField::Location),
    ("room", EventField::Location),
    ("notes", EventField::Notes),
    ("body", EventField::Notes),
    ("description", EventField::Notes),
    ("organizer", EventField::Organizer),
    ("organiser", EventField::Organizer),
    ("from", EventField::Organizer),
    ("attendee", EventField::Attendees),
    ("attendees", EventField::Attendees),
    ("to", EventField::Attendees),
    ("cc", EventField::Attendees),
    ("with", EventField::Attendees),
    ("category", EventField::Categories),
    ("categories", EventField::Categories),
];

/// The most of an event's notes looked through, in characters.
const NOTES_SEARCHED: usize = 64 * 1024;

fn is_field(k: &str) -> bool {
    FIELDS.iter().any(|(f, _)| f.eq_ignore_ascii_case(k)) || ["is", "after", "before"].iter().any(|f| f.eq_ignore_ascii_case(k))
}

/// A date: `2026-10-22`, `2026/10/22` or `20261022`, as days since the epoch.
fn date_value(v: &str) -> Option<i64> {
    let digits: String = v.trim().chars().filter(|c| c.is_ascii_digit()).collect();
    let sep_ok = v.trim().chars().all(|c| c.is_ascii_digit() || c == '-' || c == '/');
    (sep_ok && digits.len() == 8).then(|| date_days(&digits)).flatten()
}

fn needle(value: &str) -> Option<Needle> {
    if let Some(inner) = value.strip_prefix('"') {
        let phrase = inner.strip_suffix('"').unwrap_or(inner).trim().to_lowercase();
        return (!phrase.is_empty()).then_some(Needle::Phrase(phrase));
    }
    let w: Vec<String> = words(value).collect();
    if w.is_empty() {
        // Punctuation alone (`@`, `2.14`'s dot) is looked for as written.
        let v = value.trim().to_lowercase();
        return (!v.is_empty()).then_some(Needle::Phrase(v));
    }
    Some(Needle::Words(w))
}

/// A field's value: one needle, or a list at commas outside quotes (any of them).
fn needles(value: &str) -> Vec<Needle> {
    let mut parts = vec![String::new()];
    let mut quoted = false;
    for c in value.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                if let Some(p) = parts.last_mut() {
                    p.push(c);
                }
            }
            ',' if !quoted && parts.len() < 50 => parts.push(String::new()),
            c => {
                if let Some(p) = parts.last_mut() {
                    p.push(c);
                }
            }
        }
    }
    let list: Vec<Needle> = parts.iter().filter_map(|p| needle(p.trim())).collect();
    if list.is_empty() { needle(value).into_iter().collect() } else { list }
}

impl EventQuery {
    pub fn parse(input: &str) -> EventQuery {
        let mut q = EventQuery::default();
        let waiting = |tok: &str| tok.strip_prefix('-').unwrap_or(tok).strip_suffix(':').is_some_and(is_field);
        for tok in crate::search::split_tokens_with(input, waiting).into_iter().take(64) {
            let (negated, tok) = match tok.strip_prefix('-') {
                Some(rest) if !rest.is_empty() => (true, rest.to_string()),
                _ => (false, tok),
            };
            let field = tok.split_once(':').filter(|(k, v)| !v.is_empty() && !k.starts_with('"'));
            let term = match field {
                Some((k, v)) if k.eq_ignore_ascii_case("after") || k.eq_ignore_ascii_case("before") => {
                    // A date that can't be read is looked for as words.
                    match date_value(v) {
                        Some(d) => {
                            // `-after:x` is before it, `-before:x` from it on.
                            if k.eq_ignore_ascii_case("after") != negated {
                                q.after = Some(q.after.map_or(d, |a| a.max(d)));
                            } else {
                                q.before = Some(q.before.map_or(d, |b| b.min(d)));
                            }
                            continue;
                        }
                        None => Some(EventTerm::Text(EventField::Any, needles(&tok))),
                    }
                }
                Some((k, v)) if k.eq_ignore_ascii_case("is") => match v.to_ascii_lowercase().as_str() {
                    "private" | "confidential" => Some(EventTerm::Private),
                    "recurring" | "series" => Some(EventTerm::Recurring),
                    _ => Some(EventTerm::Text(EventField::Any, needles(&tok))),
                },
                Some((k, v)) => match FIELDS.iter().find(|(f, _)| f.eq_ignore_ascii_case(k)) {
                    Some((_, f)) => Some(EventTerm::Text(*f, needles(v))),
                    None => Some(EventTerm::Text(EventField::Any, needles(&tok))),
                },
                None => Some(EventTerm::Text(EventField::Any, needles(&tok))),
            };
            if let Some(t) = term.filter(|t| !matches!(t, EventTerm::Text(_, n) if n.is_empty())) {
                q.terms.push((t, negated));
            }
        }
        q
    }

    /// Nothing to look for.
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty() && self.after.is_none() && self.before.is_none()
    }

    /// Whether an event is one looked for (the dates aside: they pick its occurrences).
    pub fn matches(&self, e: &CalEvent) -> bool {
        self.terms.iter().all(|(t, negated)| term_matches(t, e) != *negated)
    }
}

fn people(list: &[&Attendee]) -> String {
    let mut s = String::new();
    for a in list {
        s.push_str(&a.name);
        s.push(' ');
        s.push_str(&a.email);
        s.push(' ');
    }
    s
}

/// What joins the parts of a field (the location and its link, categories) and, for a bare
/// word's look anywhere, the fields themselves: no word goes across it.
const SEP: &str = " \u{1f} ";

/// Where a bare word looks, in this order: the notes, the longest, last.
const ANY_FIELDS: [EventField; 6] =
    [EventField::Subject, EventField::Location, EventField::Organizer, EventField::Attendees, EventField::Categories, EventField::Notes];

/// Where it is: the location, then the link that joins it, then its conference's label and
/// link, each when the location doesn't already hold it.
fn place(e: &CalEvent) -> Cow<'_, str> {
    let conference = e.conference.iter().flat_map(|c| [c.label.as_str(), c.url.as_str()]);
    let more: Vec<&str> = std::iter::once(e.join_url.as_str()).chain(conference).filter(|p| !p.is_empty() && !e.location.contains(*p)).collect();
    if more.is_empty() {
        return Cow::Borrowed(&e.location);
    }
    let mut s = e.location.clone();
    for p in more {
        s.push_str(SEP);
        s.push_str(p);
    }
    Cow::Owned(s)
}

/// A field's text, borrowed when it's kept as it is. `Any`'s is every field joined: only a
/// phrase written across fields needs it.
fn field_text(f: EventField, e: &CalEvent) -> Cow<'_, str> {
    match f {
        EventField::Subject => Cow::Borrowed(&e.summary),
        EventField::Location => place(e),
        EventField::Notes => {
            // Characters are never fewer than bytes: short notes are read whole.
            let cut = if e.description.len() <= NOTES_SEARCHED {
                e.description.len()
            } else {
                e.description.char_indices().nth(NOTES_SEARCHED).map_or(e.description.len(), |(i, _)| i)
            };
            Cow::Borrowed(e.description.get(..cut).unwrap_or_default())
        }
        EventField::Organizer => Cow::Owned(people(&e.organizer.iter().collect::<Vec<_>>())),
        EventField::Attendees => Cow::Owned(people(&e.attendees.iter().collect::<Vec<_>>())),
        EventField::Categories => match e.categories.as_slice() {
            [one] => Cow::Borrowed(one.as_str()),
            list => Cow::Owned(list.join(SEP)),
        },
        EventField::Any => Cow::Owned(ANY_FIELDS.into_iter().map(|f| field_text(f, e)).collect::<Vec<_>>().join(SEP)),
    }
}

/// A field's text, and the same folded.
struct Text<'a> {
    raw: Cow<'a, str>,
    folded: String,
}

impl<'a> Text<'a> {
    fn new(raw: Cow<'a, str>) -> Text<'a> {
        let folded = fold_word(&raw).into_owned();
        Text { raw, folded }
    }

    /// One of its words, or of their parts, starts with `w`, folded (or, an address or a room's
    /// `2.14`, it's inside one).
    fn has_word(&self, w: &str) -> bool {
        // The text is folded a letter at a time, so each of its words folded, and each part of
        // one, is in it: a word can only start with `w` where the folded text holds it, so most
        // fields, and most lines of the others, are never cut into words.
        if !self.folded.contains(w) {
            return false;
        }
        if w.contains(['@', '.']) {
            return true;
        }
        // A line break always ends a word (Unicode's word bounds, WB3a and WB3b), and folds to
        // itself: each line is cut into words, and folded, on its own as within the text.
        self.raw.split('\n').zip(self.folded.split('\n')).any(|(raw, folded)| folded.contains(w) && words_and_parts(raw).any(|h| h.starts_with(w)))
    }
}

/// The text of the `i`th of `fields`, made the first time it's asked for.
fn nth_text<'s, 'e>(seen: &'s mut [Option<Text<'e>>], fields: &[EventField], i: usize, e: &'e CalEvent) -> Option<&'s Text<'e>> {
    let f = *fields.get(i)?;
    let slot = seen.get_mut(i)?;
    Some(slot.get_or_insert_with(|| Text::new(after_sep(field_text(f, e), i > 0))))
}

/// A field as it's read after the separator, when it isn't the first of those looked through:
/// one starting with a mark that joins the space before it (a combining one) keeps the separator
/// in front, so that its words are those of the fields joined.
fn after_sep(raw: Cow<'_, str>, after: bool) -> Cow<'_, str> {
    let joins = after && raw.chars().next().filter(|c| !c.is_ascii()).is_some_and(|c| format!(" {c}").split_word_bounds().nth(1).is_none());
    if joins { Cow::Owned(format!("{SEP}{raw}")) } else { raw }
}

/// Whether one of `fields` passes `test`, looked through in order up to the first that does.
fn in_any<'e>(seen: &mut [Option<Text<'e>>], fields: &[EventField], e: &'e CalEvent, test: impl Fn(&Text) -> bool) -> bool {
    (0..fields.len()).any(|i| nth_text(seen, fields, i, e).is_some_and(&test))
}

fn term_matches(t: &EventTerm, e: &CalEvent) -> bool {
    match t {
        EventTerm::Private => e.private,
        EventTerm::Recurring => e.rrule.is_some() || e.recurrence_id.is_some(),
        EventTerm::Text(f, list) => {
            // A bare word's fields are looked through one by one, each read only if the ones
            // before it don't have it; a phrase written across them (the separator in it)
            // needs them joined.
            let across = list.iter().any(|n| matches!(n, Needle::Phrase(p) if p.contains('\u{1f}')));
            let fields: &[EventField] = match f {
                EventField::Any if !across => &ANY_FIELDS,
                f => std::slice::from_ref(f),
            };
            let mut seen: [Option<Text>; ANY_FIELDS.len()] = Default::default();
            list.iter().any(|n| match n {
                Needle::Phrase(p) => {
                    let p = fold_word(p);
                    in_any(&mut seen, fields, e, |t| t.folded.contains(p.as_ref()))
                }
                // Each word in one of the fields (not all in the same one).
                Needle::Words(ws) => ws.iter().all(|w| {
                    let w = fold_word(w);
                    in_any(&mut seen, fields, e, |t| t.has_word(&w))
                }),
            })
        }
    }
}
