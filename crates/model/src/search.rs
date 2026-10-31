//! Search: the query language of the search box and an inverted index for fast offline search.
//!
//! The syntax follows the familiar mail-client form: bare words match anywhere (as word
//! prefixes, so `proj` finds "project"), `"quoted phrases"` match as written, and keywords narrow
//! the field: `from:`, `to:` (also `cc:`), `subject:`, `body:`, `has:attachment` /
//! `hasattachments:yes`, `is:unread` / `is:read`, `is:flagged`, `is:awaiting` (sent and not
//! answered yet), `larger:` / `smaller:` (a size: `100KB`, `2MB`, bytes alone). A leading `-`
//! negates a term. `from:` and `to:` take a list,
//! `from:ana,jun`, matching any of them (terms otherwise must all match).
//!
//! [`Index`] maps every word of every message to the messages containing it (a `BTreeMap`, so a
//! prefix is one range scan). A query asks the index for candidates and then checks each
//! candidate with [`Query::matches`], so the index may over-approximate but never misses.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::{Bound, RangeBounds};

use unicode_segmentation::UnicodeSegmentation;

use crate::{EmailAddress, MessageKey, Summary};

mod advanced;
pub use advanced::Advanced;

/// Lower-cased words of a text (UAX #29 word boundaries).
pub fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.unicode_words().map(str::to_lowercase)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Field {
    Any,
    From,
    To,
    Subject,
    Body,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Term {
    /// Words that must all occur as word prefixes in the field.
    Words(Field, Vec<String>),
    /// A phrase that must occur as written (case-insensitive).
    Phrase(Field, String),
    HasAttachment(bool),
    Unread(bool),
    Flagged(bool),
    /// Carries a keyword (`keyword:x`; `category:` names are mapped to keywords by the caller,
    /// and match the keyword spelled as the name too). `is:pinned` is the `$Pinned` keyword.
    Keyword(String),
    /// Marked high importance by its sender (`is:important`).
    Important,
    /// Carries one of these keywords (`is:categorized`: the caller fills in its categories'
    /// keywords; left empty, any keyword that isn't a system one, `$…`, counts).
    Categorized(Vec<String>),
    /// Bigger than this many bytes (`larger:`), or (`false`) smaller (`smaller:`).
    Size(bool, u64),
    /// Any one of these (`from:ana,jun`).
    AnyOf(Vec<Term>),
    /// Mail the user sent that nobody has answered yet (`is:awaiting`): the caller works out
    /// which, from every conversation ([`Doc::awaiting`]).
    Awaiting,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clause {
    pub term: Term,
    pub negated: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    pub clauses: Vec<Clause>,
}

/// What a query is checked against: the summary and, when cached, the body text.
pub struct Doc<'a> {
    pub summary: &'a Summary,
    pub body: Option<&'a str>,
    /// It came from an [`Index`] search for the query's words: words looked for anywhere are known
    /// to be there, without its body (reading a big mailbox's bodies again for every search was
    /// most of its time).
    pub indexed: bool,
    /// The user sent it a while ago and no answer has come ([`Term::Awaiting`]).
    pub awaiting: bool,
}

fn split_tokens(input: &str) -> Vec<String> {
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
                if !cur.is_empty() && !field_waiting(&cur) {
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
];

/// True for a bare `field:` (optionally negated) still missing its value.
fn field_waiting(tok: &str) -> bool {
    let name = tok.strip_prefix('-').unwrap_or(tok);
    name.strip_suffix(':').is_some_and(|k| FIELDS.iter().any(|f| f.eq_ignore_ascii_case(k)))
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

impl Query {
    pub fn parse(input: &str) -> Query {
        let mut clauses = Vec::new();
        for tok in split_tokens(input).into_iter().take(64) {
            let (negated, tok) = match tok.strip_prefix('-') {
                Some(rest) if !rest.is_empty() => (true, rest.to_string()),
                _ => (false, tok),
            };
            let term = match tok.split_once(':') {
                Some((k, v)) if !v.is_empty() && !k.starts_with('"') => match k.to_ascii_lowercase().as_str() {
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
                        "important" => Some(Term::Important),
                        "categorized" | "categorised" => Some(Term::Categorized(Vec::new())),
                        "awaiting" | "awaitingreply" | "unanswered" => Some(Term::Awaiting),
                        _ => text_term(Field::Any, &tok),
                    },
                    "isread" => yes_no(v).map(|b| Term::Unread(!b)),
                    "isflagged" => yes_no(v).map(Term::Flagged),
                    "keyword" | "category" | "label" | "tag" => Some(Term::Keyword(v.trim_matches('"').to_string())),
                    _ => text_term(Field::Any, &tok),
                },
                _ => text_term(Field::Any, &tok),
            };
            if let Some(term) = term {
                clauses.push(Clause { term, negated });
            }
        }
        Query { clauses }
    }

    pub fn is_empty(&self) -> bool {
        self.clauses.is_empty()
    }

    /// The words the index can narrow by: words of positive text clauses.
    pub fn index_words(&self) -> Vec<String> {
        let mut v = Vec::new();
        for c in self.clauses.iter().filter(|c| !c.negated) {
            match &c.term {
                Term::Words(_, w) => v.extend(w.iter().cloned()),
                Term::Phrase(_, p) => v.extend(words(p)),
                _ => {}
            }
        }
        v
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

fn addr_text(list: &[EmailAddress]) -> String {
    let mut s = String::new();
    for a in list {
        s.push_str(&a.name);
        s.push(' ');
        s.push_str(&a.email);
        s.push(' ');
    }
    s
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
        Term::Important => m.importance > 0,
        Term::Awaiting => doc.awaiting,
        Term::Categorized(ks) if ks.is_empty() => m.flags.keywords.iter().any(|k| !k.starts_with('$')),
        Term::Categorized(ks) => ks.iter().any(|k| m.flags.has(k)),
        Term::Size(true, b) => u64::from(m.size) > *b,
        Term::Size(false, b) => u64::from(m.size) < *b,
        Term::AnyOf(ts) => ts.iter().any(|t| clause_matches(t, doc)),
        Term::Phrase(f, p) => field_text(f, doc).to_lowercase().contains(p.as_str()),
        // Found through the index for these words: they're there (summary or body).
        Term::Words(Field::Any, ws) if doc.indexed && doc.body.is_none() && ws.iter().all(|w| !w.contains(['@', '.'])) => true,
        Term::Words(f, ws) => {
            let text = field_text(f, doc);
            let have: Vec<String> = words(&text).collect();
            let lower = text.to_lowercase();
            ws.iter().all(|w| have.iter().any(|h| h.starts_with(w.as_str())) || (w.contains(['@', '.']) && lower.contains(w.as_str())))
        }
    }
}

/// Words → documents. Document ids are the caller's (positions in its message table).
#[derive(Clone, Debug, Default)]
pub struct Index {
    /// Each word's documents, sorted (an array: a set per word took several times the memory,
    /// gigabytes for a big mailbox).
    postings: BTreeMap<String, Vec<u32>>,
    docs: BTreeSet<u32>,
}

/// The text a summary contributes to the index.
pub fn summary_text(m: &Summary) -> String {
    let mut s = String::with_capacity(128 + m.preview.len());
    s.push_str(&m.subject);
    s.push(' ');
    s.push_str(&addr_text(std::slice::from_ref(&m.from)));
    s.push_str(&addr_text(&m.to));
    s.push_str(&addr_text(&m.cc));
    s.push_str(&m.preview);
    s
}

impl Index {
    pub fn new() -> Index {
        Index::default()
    }

    pub fn len(&self) -> usize {
        self.docs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }

    /// Add a document's text (call again with more text, e.g. the body once fetched).
    pub fn add(&mut self, doc: u32, text: &str) {
        self.docs.insert(doc);
        for w in words(text) {
            // Very long "words" (base64 debris) only bloat the index.
            if w.chars().count() <= 40 {
                let list = self.postings.entry(w).or_default();
                // Documents mostly come in order: appended; one added later goes in its place.
                match list.last() {
                    Some(&last) if last == doc => {}
                    Some(&last) if last > doc => {
                        if let Err(at) = list.binary_search(&doc) {
                            list.insert(at, doc);
                        }
                    }
                    _ => list.push(doc),
                }
            }
        }
    }

    /// Documents containing a word starting with `prefix`.
    pub fn prefix(&self, prefix: &str) -> BTreeSet<u32> {
        self.prefix_in(prefix, ..)
    }

    /// Documents numbered within `docs` containing a word starting with `prefix` (each word's
    /// list is sorted: the ones outside are skipped, not read).
    fn prefix_in(&self, prefix: &str, docs: impl RangeBounds<u32>) -> BTreeSet<u32> {
        let (start, end) = span(&docs);
        let mut out = BTreeSet::new();
        for (_, list) in self.postings.range(prefix.to_string()..).take_while(|(k, _)| k.starts_with(prefix)) {
            let lo = list.partition_point(|&d| u64::from(d) < start);
            let hi = list.partition_point(|&d| u64::from(d) < end);
            out.extend(list.get(lo..hi.max(lo)).unwrap_or_default().iter().copied());
        }
        out
    }

    /// Take another index in, its documents numbered from `offset` on: a folder read on its own
    /// keeps its numbers once joined to the mailbox's. The words are moved rather than copied (a
    /// big folder's would take twice its memory while it joins).
    pub fn absorb(&mut self, other: Index, offset: u32) {
        for (w, mut docs) in other.postings {
            match self.postings.entry(w) {
                std::collections::btree_map::Entry::Vacant(e) => {
                    for d in &mut docs {
                        *d = d.saturating_add(offset);
                    }
                    e.insert(docs);
                }
                std::collections::btree_map::Entry::Occupied(mut e) => {
                    let list = e.get_mut();
                    let first = docs.first().map(|&d| d.saturating_add(offset));
                    // Everything in the whole index comes before the folder's documents: the
                    // list is simply extended.
                    if list.last().is_none_or(|&last| first.is_some_and(|f| f > last)) {
                        list.extend(docs.into_iter().map(|d| d.saturating_add(offset)));
                    } else {
                        for d in docs {
                            let d = d.saturating_add(offset);
                            match list.last() {
                                Some(&last) if last >= d => {
                                    if let Err(at) = list.binary_search(&d) {
                                        list.insert(at, d);
                                    }
                                }
                                _ => list.push(d),
                            }
                        }
                    }
                }
            }
        }
        for d in other.docs {
            self.docs.insert(d.saturating_add(offset));
        }
    }

    /// The index as bytes, for keeping it between runs: the documents, then each word and the
    /// documents holding it as differences (a big folder's list stays a couple of bytes a
    /// message). [`Index::from_bytes`] reads it back.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64 + self.postings.len() * 8);
        out.extend_from_slice(INDEX_MAGIC);
        out.push(INDEX_VERSION);
        put_varint(&mut out, u32::try_from(self.docs.len()).unwrap_or(u32::MAX));
        let mut prev = 0u32;
        for &d in &self.docs {
            put_varint(&mut out, d.saturating_sub(prev));
            prev = d;
        }
        // A word too long for its length byte (none are: `add` caps them) is dropped.
        let kept = self.postings.iter().filter(|(w, _)| !w.is_empty() && w.len() <= MAX_WORD_BYTES);
        put_varint(&mut out, u32::try_from(kept.clone().count()).unwrap_or(u32::MAX));
        for (w, docs) in kept {
            out.push(w.len() as u8);
            out.extend_from_slice(w.as_bytes());
            put_varint(&mut out, u32::try_from(docs.len()).unwrap_or(u32::MAX));
            let mut prev = 0u32;
            for &d in docs {
                put_varint(&mut out, d.saturating_sub(prev));
                prev = d;
            }
        }
        out
    }

    /// Read back an index [`Index::to_bytes`] wrote; `None` for anything else: a damaged cache
    /// file is rebuilt, never trusted.
    pub fn from_bytes(bytes: &[u8]) -> Option<Index> {
        let mut r = Bytes { b: bytes, at: 0 };
        if r.take(INDEX_MAGIC.len())? != INDEX_MAGIC || r.u8()? != INDEX_VERSION {
            return None;
        }
        let docs = r.varint()? as usize;
        if docs > MAX_INDEX_DOCS {
            return None;
        }
        let mut ix = Index::new();
        let mut prev = 0u32;
        for i in 0..docs {
            let delta = r.varint()?;
            let d = if i == 0 { delta } else { prev.checked_add(delta)? };
            if i > 0 && d <= prev {
                return None;
            }
            ix.docs.insert(d);
            prev = d;
        }
        let words = r.varint()? as usize;
        if words > MAX_INDEX_WORDS {
            return None;
        }
        for _ in 0..words {
            let n = usize::from(r.u8()?);
            if n == 0 || n > MAX_WORD_BYTES {
                return None;
            }
            let w = std::str::from_utf8(r.take(n)?).ok()?;
            let count = r.varint()? as usize;
            // Each document takes at least a byte: a count past the rest of the bytes is a lie.
            if count > MAX_INDEX_DOCS || count > bytes.len() - r.at {
                return None;
            }
            let mut list = Vec::with_capacity(count.min(1024));
            let mut prev = 0u32;
            for i in 0..count {
                let delta = r.varint()?;
                let d = if i == 0 { delta } else { prev.checked_add(delta)? };
                if i > 0 && d <= prev {
                    return None;
                }
                list.push(d);
                prev = d;
            }
            if !list.is_empty() {
                ix.postings.insert(w.to_string(), list);
            }
        }
        if r.at != bytes.len() {
            return None;
        }
        Some(ix)
    }

    /// Candidates for a query: documents with every index word as a prefix of some word, or all
    /// documents when the query has no words (flag-only queries).
    pub fn candidates(&self, q: &Query) -> BTreeSet<u32> {
        self.candidates_in(q, ..)
    }

    /// Candidates numbered `from` on: a search carried on while the index grows looks at the
    /// documents added since it last looked, not at all of them again.
    pub fn candidates_from(&self, q: &Query, from: u32) -> BTreeSet<u32> {
        self.candidates_in(q, from..)
    }

    /// Candidates numbered within `docs`: a search looks again at the messages of a folder that
    /// changed (one marked unread may answer it now), not at the whole index.
    pub fn candidates_in(&self, q: &Query, docs: impl RangeBounds<u32> + Clone) -> BTreeSet<u32> {
        let (start, end) = span(&docs);
        if start >= end {
            return BTreeSet::new();
        }
        let ws = q.index_words();
        if ws.is_empty() {
            let from = u32::try_from(start).unwrap_or(u32::MAX);
            return match u32::try_from(end) {
                Ok(end) => self.docs.range(from..end).copied().collect(),
                Err(_) => self.docs.range(from..).copied().collect(),
            };
        }
        let mut acc: Option<BTreeSet<u32>> = None;
        for w in ws {
            let hits = self.prefix_in(&w, docs.clone());
            acc = Some(match acc {
                None => hits,
                Some(a) => a.intersection(&hits).copied().collect(),
            });
            if acc.as_ref().is_some_and(BTreeSet::is_empty) {
                break;
            }
        }
        acc.unwrap_or_default()
    }
}

/// A range of document numbers as `start..end` (`end` past the last number when unbounded): an
/// empty or reversed range is simply empty, never a panic.
fn span(docs: &impl RangeBounds<u32>) -> (u64, u64) {
    let start = match docs.start_bound() {
        Bound::Included(&s) => u64::from(s),
        Bound::Excluded(&s) => u64::from(s) + 1,
        Bound::Unbounded => 0,
    };
    let end = match docs.end_bound() {
        Bound::Included(&e) => u64::from(e) + 1,
        Bound::Excluded(&e) => u64::from(e),
        Bound::Unbounded => u64::from(u32::MAX) + 1,
    };
    (start, end)
}

/// What [`Index::to_bytes`] writes first, and the format's version.
const INDEX_MAGIC: &[u8] = b"MCIX";
const INDEX_VERSION: u8 = 1;
/// What [`FolderIndex::to_bytes`] writes first, and the format's version.
const FOLDER_MAGIC: &[u8] = b"MCFI";
const FOLDER_VERSION: u8 = 1;
/// Bounds read back from a file: past them the file isn't ours any more (or is damaged), and it
/// is rebuilt.
const MAX_INDEX_DOCS: usize = 8_000_000;
const MAX_INDEX_WORDS: usize = 8_000_000;
const MAX_WORD_BYTES: usize = 160;

/// A little-endian base-128 number, the short form of the numbers and gaps kept on disk.
fn put_varint(out: &mut Vec<u8>, mut v: u32) {
    while v >= 0x80 {
        out.push((v as u8 & 0x7f) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

/// Reading bytes that may come from a damaged or hostile file: every read is checked.
struct Bytes<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Bytes<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(n)?;
        let s = self.b.get(self.at..end)?;
        self.at = end;
        Some(s)
    }

    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    fn varint(&mut self) -> Option<u32> {
        let mut v = 0u32;
        for shift in (0..35).step_by(7) {
            let b = self.u8()?;
            v |= u32::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Some(v);
            }
        }
        None
    }
}

/// A folder's index as kept between runs: the state it was read from (its `stamp`), the messages
/// in the order of their number (and whether each had a body), and the words.
///
/// Kept per folder: a folder that changed is read again on its own, the others are taken as they
/// are. The stamp covers the messages' text, not their flags: marking mail read doesn't throw the
/// index away. Bodies read since are added when the index is read back (see the session).
#[derive(Clone, Debug, Default)]
pub struct FolderIndex {
    pub stamp: u64,
    pub keys: Vec<MessageKey>,
    pub has_body: Vec<bool>,
    pub index: Index,
}

impl FolderIndex {
    pub fn new(stamp: u64) -> FolderIndex {
        FolderIndex { stamp, ..Default::default() }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(32 + self.keys.len() * 10);
        out.extend_from_slice(FOLDER_MAGIC);
        out.push(FOLDER_VERSION);
        out.extend_from_slice(&self.stamp.to_le_bytes());
        let n = self.keys.len().min(self.has_body.len());
        put_varint(&mut out, u32::try_from(n).unwrap_or(u32::MAX));
        for (i, k) in self.keys.iter().take(n).enumerate() {
            out.extend_from_slice(&k.validity.to_le_bytes());
            out.extend_from_slice(&k.uid.to_le_bytes());
            out.push(u8::from(self.has_body.get(i) == Some(&true)));
        }
        let words = self.index.to_bytes();
        put_varint(&mut out, u32::try_from(words.len()).unwrap_or(u32::MAX));
        out.extend_from_slice(&words);
        out
    }

    /// Read back a folder index; `None` for a damaged file or one of another format. `account`
    /// and `folder` aren't kept in it: the file's place says which folder it is.
    pub fn from_bytes(account: &str, folder: &str, bytes: &[u8]) -> Option<FolderIndex> {
        let mut r = Bytes { b: bytes, at: 0 };
        if r.take(FOLDER_MAGIC.len())? != FOLDER_MAGIC || r.u8()? != FOLDER_VERSION {
            return None;
        }
        let stamp = r.u64()?;
        let n = r.varint()? as usize;
        if n > MAX_INDEX_DOCS {
            return None;
        }
        let mut fi = FolderIndex::new(stamp);
        fi.keys.reserve(n.min(1024));
        fi.has_body.reserve(n.min(1024));
        for _ in 0..n {
            let (validity, uid, body) = (r.u32()?, r.u32()?, r.u8()? != 0);
            fi.keys.push(MessageKey { account: account.to_string(), folder: folder.to_string(), validity, uid });
            fi.has_body.push(body);
        }
        let len = r.varint()? as usize;
        fi.index = Index::from_bytes(r.take(len)?)?;
        if r.at != bytes.len() {
            return None;
        }
        Some(fi)
    }
}

#[cfg(test)]
mod tests;
