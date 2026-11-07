// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Reading what's typed in the search box into a [`Query`].

use super::*;

pub(super) fn split_tokens(input: &str) -> Vec<String> {
    split_tokens_with(input, field_waiting)
}

/// The query's tokens: split at whitespace outside quotes, a bare `field:` (as `waiting` knows
/// fields) joined to the value after it.
pub(crate) fn split_tokens_with(input: &str, waiting: impl Fn(&str) -> bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in input.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                cur.push(c);
            }
            c if c.is_whitespace() && !quoted => {
                // Whitespace after a bare `field:` is skipped, joining the value to the field.
                if !cur.is_empty() && !waiting(&cur) {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Field prefixes the parser recognises.
const FIELDS: &[&str] = &[
    "from",
    "to",
    "cc",
    "subject",
    "body",
    "has",
    "hasattachment",
    "hasattachments",
    "is",
    "isread",
    "isflagged",
    "keyword",
    "category",
    "label",
    "tag",
    "larger",
    "smaller",
    "received",
    "sent",
    "date",
    "after",
    "before",
];

/// True for a bare `field:` (optionally negated) still missing its value.
fn field_waiting(tok: &str) -> bool {
    let name = tok.strip_prefix('-').unwrap_or(tok);
    name.strip_suffix(':').is_some_and(|k| FIELDS.iter().any(|f| f.eq_ignore_ascii_case(k)) || dates::field(k).is_some())
}

fn yes_no(v: &str) -> Option<bool> {
    match v.to_ascii_lowercase().as_str() {
        "yes" | "true" | "1" => Some(true),
        "no" | "false" | "0" => Some(false),
        _ => None,
    }
}

/// A size: `100KB`, `1.5MB`, `2 GB`, `300` (bytes); KB is 1024 bytes, as mail clients count.
fn size(value: &str) -> Option<u64> {
    let v = value.trim().to_ascii_lowercase();
    let at = v.find(|c: char| !c.is_ascii_digit() && c != '.').unwrap_or(v.len());
    let (num, unit) = v.split_at(at);
    let n: f64 = num.parse().ok().filter(|n: &f64| n.is_finite() && *n >= 0.0)?;
    let scale: f64 = match unit.trim() {
        "" | "b" => 1.0,
        "k" | "kb" => 1024.0,
        "m" | "mb" => 1024.0 * 1024.0,
        "g" | "gb" => 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    let bytes = (n * scale).round();
    // Past a terabyte it's no mail size: held there (an `as` cast of a huge float saturates).
    Some(bytes.min(1024f64.powi(4)) as u64)
}

/// Split a list value at commas outside quotes: `"park, zoe",ann` is two. The parts keep their
/// quotes and spaces.
fn split_list(value: &str) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut quoted = false;
    for c in value.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                parts.last_mut().map(|p| p.push(c)).unwrap_or_default();
            }
            ',' if !quoted => parts.push(String::new()),
            c => parts.last_mut().map(|p| p.push(c)).unwrap_or_default(),
        }
    }
    parts
}

/// The people of a query that is only `from:a,"b c",…` (what "Mail from specific people" writes):
/// each trimmed and unquoted, empties dropped. `None` when the query has anything else, or no one.
pub fn from_people(query: &str) -> Option<Vec<String>> {
    let tokens = split_tokens(query);
    let [token] = tokens.as_slice() else { return None };
    let (key, value) = token.split_once(':')?;
    if !key.eq_ignore_ascii_case("from") {
        return None;
    }
    let people: Vec<String> = split_list(value)
        .iter()
        .map(|p| {
            let p = p.trim();
            p.strip_prefix('"').map(|r| r.strip_suffix('"').unwrap_or(r)).unwrap_or(p).trim().to_string()
        })
        .filter(|p| !p.is_empty())
        .collect();
    (!people.is_empty()).then_some(people)
}

/// A field that may hold a list: `from:ana,jun` is any of them.
fn list_term(field: Field, value: &str) -> Option<Term> {
    let parts = split_list(value);
    if parts.len() < 2 {
        return text_term(field, value);
    }
    let any: Vec<Term> = parts.iter().take(50).filter_map(|v| text_term(field.clone(), v.trim())).collect();
    match any.len() {
        // Commas alone: looked for as they are.
        0 => text_term(field, value),
        1 => any.into_iter().next(),
        _ => Some(Term::AnyOf(any)),
    }
}

fn text_term(field: Field, value: &str) -> Option<Term> {
    if let Some(inner) = value.strip_prefix('"') {
        let phrase = inner.strip_suffix('"').unwrap_or(inner).trim().to_lowercase();
        return (!phrase.is_empty()).then_some(Term::Phrase(field, phrase));
    }
    let w: Vec<String> = words(value).collect();
    if w.is_empty() {
        // Punctuation-only input (`@`, `.`) is still a substring to look for.
        let v = value.trim().to_lowercase();
        return (!v.is_empty()).then_some(Term::Phrase(field, v));
    }
    Some(Term::Words(field, w))
}

/// A date term, or the problem with it: a date that isn't one is left out, never looked for as
/// words (`received:2024` became two words, and matched nearly everything). A value that isn't
/// meant as a date (`before: launch`, `after:party`) is still the words typed, as it was before
/// these were fields, with the problem said all the same.
fn date_term(spec: Option<DateSpec>, value: &str, tok: &str, problems: &mut Vec<Problem>) -> Option<Term> {
    match spec {
        Some(s) => Some(Term::Date(s)),
        None => {
            problems.push(Problem::Date(tok.chars().take(100).collect()));
            // Apart: UAX #29 reads `before:launch` as one word, which no text holds.
            let (key, _) = tok.split_once(':').unwrap_or((tok, ""));
            if dates::looks_like_date(value) { None } else { text_term(Field::Any, &format!("{key} {value}")) }
        }
    }
}

impl Query {
    pub fn parse(input: &str) -> Query {
        let mut clauses = Vec::new();
        let mut problems = Vec::new();
        for tok in split_tokens(input).into_iter().take(64) {
            let (negated, tok) = match tok.strip_prefix('-') {
                Some(rest) if !rest.is_empty() => (true, rest.to_string()),
                _ => (false, tok),
            };
            let term = match tok.split_once(':') {
                Some((k, v)) if !v.is_empty() && !k.starts_with('"') => {
                    match dates::field(k).map_or_else(|| k.to_ascii_lowercase(), str::to_string).as_str() {
                        "received" => date_term(dates::period(v), v, &tok, &mut problems),
                        "sent" | "date" => date_term(dates::period(v).map(|s| DateSpec { sent: true, ..s }), v, &tok, &mut problems),
                        "after" => date_term(dates::after(v), v, &tok, &mut problems),
                        "before" => date_term(dates::before(v), v, &tok, &mut problems),
                        "from" => list_term(Field::From, v),
                        "to" | "cc" => list_term(Field::To, v),
                        "larger" => size(v).map(|b| Term::Size(true, b)),
                        "smaller" => size(v).map(|b| Term::Size(false, b)),
                        "subject" => text_term(Field::Subject, v),
                        "body" => text_term(Field::Body, v),
                        "has" if v.eq_ignore_ascii_case("attachment") || v.eq_ignore_ascii_case("attachments") => Some(Term::HasAttachment(true)),
                        "hasattachment" | "hasattachments" => yes_no(v).map(Term::HasAttachment),
                        "is" => match v.to_ascii_lowercase().as_str() {
                            "unread" => Some(Term::Unread(true)),
                            "read" => Some(Term::Unread(false)),
                            "flagged" => Some(Term::Flagged(true)),
                            "unflagged" => Some(Term::Flagged(false)),
                            "pinned" => Some(Term::Keyword(crate::PINNED.to_string())),
                            "complete" | "completed" => Some(Term::Complete),
                            "important" => Some(Term::Important),
                            "categorized" | "categorised" => Some(Term::Categorized(Vec::new())),
                            "awaiting" | "awaitingreply" | "unanswered" => Some(Term::Awaiting),
                            _ => text_term(Field::Any, &tok),
                        },
                        "isread" => yes_no(v).map(|b| Term::Unread(!b)),
                        "isflagged" => yes_no(v).map(Term::Flagged),
                        "keyword" | "category" | "label" | "tag" => Some(Term::Keyword(v.trim_matches('"').to_string())),
                        _ => text_term(Field::Any, &tok),
                    }
                }
                _ => text_term(Field::Any, &tok),
            };
            if let Some(term) = term {
                clauses.push(Clause { term, negated });
            }
        }
        Query { clauses, problems }
    }
}
