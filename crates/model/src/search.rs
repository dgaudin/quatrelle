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

/// A field that may hold a list: `from:ana,jun` is any of them.
fn list_term(field: Field, value: &str) -> Option<Term> {
    // Split at commas outside quotes: `"park, zoe",ann` is two.
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
        let mut out = BTreeSet::new();
        for (_, docs) in self.postings.range(prefix.to_string()..).take_while(|(k, _)| k.starts_with(prefix)) {
            out.extend(docs.iter().copied());
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
        let ws = q.index_words();
        if ws.is_empty() {
            return self.docs.clone();
        }
        let mut acc: Option<BTreeSet<u32>> = None;
        for w in ws {
            let hits = self.prefix(&w);
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
mod tests {
    use super::*;
    use crate::{EmailAddress, Flags};

    fn sample() -> Summary {
        Summary {
            subject: "Project kickoff: storyboards".into(),
            from: EmailAddress::new("Zoe Park", "zoe@studio.example"),
            to: vec![EmailAddress::new("Ann", "ann@studio.example")],
            preview: "Bring the sketches for the opening scene".into(),
            has_attachments: true,
            flags: Flags { seen: false, flagged: true, ..Default::default() },
            ..Default::default()
        }
    }

    fn q(s: &str, m: &Summary) -> bool {
        Query::parse(s).matches(&Doc { summary: m, body: None, indexed: false, awaiting: false })
    }

    /// Whitespace between `field:` and its value is tolerated.
    #[test]
    fn whitespace_between_field_and_value() {
        let m = sample();
        assert_eq!(Query::parse("from: zoe").clauses, Query::parse("from:zoe").clauses);
        assert_eq!(Query::parse("FROM:   \"zoe park\"  -is:  read").clauses, Query::parse("from:\"zoe park\" -is:read").clauses);
        assert!(q("from: zoe", &m) && !q("from: ann", &m) && q("to: ann", &m) && q("-is: read", &m));
        // Unknown prefixes stay plain words, and so does a trailing `from:`.
        assert_eq!(Query::parse("note: zoe").clauses.len(), 2);
        assert_eq!(Query::parse("zoe from:").clauses.len(), 2);
        assert!(Query::parse("from:\t\n").clauses.len() == 1);
    }

    /// Sizes, and lists matching any of their names.
    #[test]
    fn sizes_and_lists() {
        let mut m = sample();
        m.size = 300 * 1024;
        assert!(q("larger:100KB", &m) && q("larger:100kb", &m) && !q("larger:1MB", &m) && q("smaller:0.5mb", &m) && !q("smaller:300K", &m));
        assert!(q("-larger:1MB", &m) && q("larger:307199", &m) && !q("larger:307200", &m));
        // Not sizes: plain words, never a crash.
        for junk in ["larger:", "larger:big", "larger:-5", "larger:1e999MB", "larger:NaN", "smaller:12XB", "larger:99999999999999999999GB"] {
            let _ = q(junk, &m);
        }
        assert_eq!(Query::parse("larger:99999999999999999999GB").clauses[0].term, Term::Size(true, 1024u64.pow(4)));
        assert!(q("from:ana,zoe", &m) && q("from:\"zoe park\",ann", &m) && !q("from:ana,jun", &m) && q("to:x,ann", &m));
        assert!(!q("-from:ana,zoe", &m) && q("from:zoe,", &m) && q("from:,zoe", &m));
        assert_eq!(Query::parse("from:,,").clauses.len(), 1, "commas alone: a phrase to look for");
        assert!(matches!(&Query::parse("from:a,b,c").clauses[0].term, Term::AnyOf(v) if v.len() == 3));
    }

    #[test]
    fn parsing() {
        let p = Query::parse("from:zoe \"opening scene\" -is:read has:attachment");
        assert_eq!(p.clauses.len(), 4);
        assert_eq!(p.clauses[0].term, Term::Words(Field::From, vec!["zoe".into()]));
        assert_eq!(p.clauses[1].term, Term::Phrase(Field::Any, "opening scene".into()));
        assert!(p.clauses[2].negated);
        assert!(Query::parse("   ").is_empty());
        assert_eq!(Query::parse("subject:\"a b\"").clauses[0].term, Term::Phrase(Field::Subject, "a b".into()));
    }

    #[test]
    fn matching() {
        let m = sample();
        assert!(q("proj", &m));
        assert!(q("from:zoe", &m));
        assert!(!q("from:ann", &m));
        assert!(q("to:ann", &m));
        assert!(q("subject:kickoff is:unread is:flagged", &m));
        assert!(q("hasattachments:yes", &m));
        assert!(!q("is:read", &m));
        assert!(q("\"opening scene\"", &m));
        assert!(!q("\"scene opening\"", &m));
        assert!(q("-from:ann", &m));
        assert!(q("zoe@studio.example", &m));
        assert!(q("body:sketches", &m));
        assert!(!q("is:important", &m) && !q("is:categorized", &m));
        let mut m2 = m.clone();
        m2.importance = 1;
        m2.flags.keywords = vec!["$Pinned".into(), "work".into()];
        assert!(q("is:important is:categorized", &m2));
        let label = Query { clauses: vec![Clause { term: Term::Categorized(vec!["$label1".into()]), negated: false }] };
        assert!(!label.matches(&Doc { summary: &m2, body: None, indexed: false, awaiting: false }));
        assert!(Query::parse("").matches(&Doc { summary: &m, body: None, indexed: false, awaiting: false }));
    }

    #[test]
    fn index_narrows_without_missing() {
        let a = sample();
        let mut b = sample();
        b.subject = "Invoice October".into();
        b.preview = "Payment due".into();
        let mut ix = Index::new();
        ix.add(0, &summary_text(&a));
        ix.add(1, &summary_text(&b));
        assert_eq!(ix.candidates(&Query::parse("invo")), BTreeSet::from([1]));
        assert_eq!(ix.candidates(&Query::parse("zoe proj")), BTreeSet::from([0]));
        assert_eq!(ix.candidates(&Query::parse("is:unread")).len(), 2);
        assert!(ix.candidates(&Query::parse("nothing-here")).is_empty());
        ix.add(1, "late body text");
        assert_eq!(ix.candidates(&Query::parse("late")), BTreeSet::from([1]));
    }

    /// The index keeps each word's documents sorted and once, whatever order they come in; plain
    /// words found through it need no body, while phrases, missing words and addresses do.
    #[test]
    fn the_index_settles_plain_words() {
        let mut ix = Index::new();
        ix.add(5, "storyboard review");
        ix.add(2, "storyboard storyboard");
        ix.add(9, "budget");
        ix.add(5, "storyboard again");
        assert_eq!(ix.prefix("story").into_iter().collect::<Vec<_>>(), [2, 5]);
        let q = Query::parse("storyboard review");
        assert_eq!(ix.candidates(&q).into_iter().collect::<Vec<_>>(), [5]);
        assert!(!q.needs_body());
        let m = Summary { subject: "Weekly".into(), ..Default::default() };
        assert!(q.matches(&Doc { summary: &m, body: None, indexed: true, awaiting: false }), "the index found both words");
        assert!(!q.matches(&Doc { summary: &m, body: None, indexed: false, awaiting: false }), "without it, the words aren't in the summary");
        for needs in ["\"storyboard review\"", "-budget storyboard", "body:storyboard", "zoe@studio.example"] {
            assert!(Query::parse(needs).needs_body(), "{needs}");
        }
        for not in ["from:zoe storyboard", "is:unread budget", "subject:weekly"] {
            assert!(!Query::parse(not).needs_body(), "{not}");
        }
    }

    /// An index kept between runs comes back with the same words for the same documents, and a
    /// folder's index with the same messages.
    #[test]
    fn an_index_is_kept_and_read_back() {
        let mut a = Index::new();
        a.add(0, &summary_text(&sample()));
        a.add(1, "invoice payment due");
        a.add(2, "invoice again, storyboard review");
        a.add(7, ""); // a document with no word at all still exists
        let bytes = a.to_bytes();
        let b = Index::from_bytes(&bytes).expect("it reads back");
        assert_eq!(b.len(), 4);
        for q in ["invo", "storyboard review", "opening scene", "nothing-here"] {
            assert_eq!(a.candidates(&Query::parse(q)), b.candidates(&Query::parse(q)), "{q}");
        }
        // Nothing left over, and no word kept twice.
        assert_eq!(b.to_bytes(), bytes);
    }

    #[test]
    fn merging_indexes_numbers_them_apart() {
        let mut folder = Index::new();
        folder.add(0, "storyboard");
        folder.add(1, "budget");
        let mut whole = Index::new();
        whole.add(0, "storyboard review");
        whole.absorb(folder, 1);
        assert_eq!(whole.len(), 3);
        assert_eq!(whole.candidates(&Query::parse("storyboard")).into_iter().collect::<Vec<_>>(), [0, 1]);
        assert_eq!(whole.candidates(&Query::parse("budget")).into_iter().collect::<Vec<_>>(), [2]);
        // The joined index is itself kept and read back.
        let back = Index::from_bytes(&whole.to_bytes()).expect("it reads back");
        assert_eq!(back.candidates(&Query::parse("storyboard")).into_iter().collect::<Vec<_>>(), [0, 1]);
    }

    /// Bytes that aren't an index — cut short, from another format, with a lying count or a
    /// posting out of order — are refused rather than trusted.
    #[test]
    fn damaged_index_bytes_are_refused() {
        let mut ix = Index::new();
        ix.add(0, "storyboard review");
        ix.add(3, "budget");
        let good = ix.to_bytes();
        assert!(Index::from_bytes(&good).is_some());
        for cut in 0..good.len() {
            assert!(Index::from_bytes(&good[..cut]).is_none(), "cut at {cut}");
        }
        let mut trailing = good.clone();
        trailing.push(0);
        assert!(Index::from_bytes(&trailing).is_none());
        let mut magic = good.clone();
        magic[0] = b'X';
        assert!(Index::from_bytes(&magic).is_none());
        // A word count of four billion in four bytes.
        let mut lying = good[..6].to_vec();
        lying.extend_from_slice(&[0xff, 0xff, 0xff, 0xff, 0x0f]);
        assert!(Index::from_bytes(&lying).is_none());
        // A posting list with the same document twice (its gaps must grow).
        let mut twice = Vec::new();
        twice.extend_from_slice(INDEX_MAGIC);
        twice.push(INDEX_VERSION);
        put_varint(&mut twice, 2);
        put_varint(&mut twice, 5);
        put_varint(&mut twice, 1);
        put_varint(&mut twice, 1);
        twice.push(1);
        twice.push(b'w');
        put_varint(&mut twice, 2);
        put_varint(&mut twice, 5);
        put_varint(&mut twice, 0);
        assert!(Index::from_bytes(&twice).is_none());
    }

    #[test]
    fn a_folder_index_is_kept_and_read_back() {
        let mut fi = FolderIndex::new(42);
        for uid in 1..=3u32 {
            fi.keys.push(MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 7, uid });
            fi.has_body.push(uid == 2);
        }
        fi.index.add(0, "storyboard");
        fi.index.add(1, "body words, budget");
        fi.index.add(2, "storyboard");
        let bytes = fi.to_bytes();
        let back = FolderIndex::from_bytes("a1", "INBOX", &bytes).expect("it reads back");
        assert_eq!(back.stamp, 42);
        assert_eq!(back.keys, fi.keys);
        assert_eq!(back.has_body, [false, true, false]);
        assert_eq!(back.index.candidates(&Query::parse("storyboard")).into_iter().collect::<Vec<_>>(), [0, 2]);
        // The folder isn't kept in the file: it's the one it's read for.
        let other = FolderIndex::from_bytes("a1", "Sent", &bytes).expect("it reads back");
        assert_eq!(other.keys[0].folder, "Sent");
        for cut in [0, 3, 6, 12, bytes.len() - 1] {
            assert!(FolderIndex::from_bytes("a1", "INBOX", &bytes[..cut]).is_none(), "cut at {cut}");
        }
        let mut version = bytes.clone();
        version[4] = FOLDER_VERSION + 1;
        assert!(FolderIndex::from_bytes("a1", "INBOX", &version).is_none());
    }
}
