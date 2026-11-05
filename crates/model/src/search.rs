// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
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
//! candidate with [`Query::matches`], so the index may over-approximate but never misses. Words
//! are read [folded](fold_word) on both sides: `resume` finds "Résumé", `résumé` "resume"; a
//! server is asked for what was typed.

use std::borrow::Cow;

use unicode_segmentation::UnicodeSegmentation;

use crate::{EmailAddress, Summary};

mod advanced;
mod fold;
mod fold_table;
mod index;
mod matching;
mod parse;

pub use advanced::Advanced;
pub(crate) use fold::words_and_parts;
pub use fold::{MAX_TOKEN_CHARS, fold_word, index_tokens, parts, tokens};
#[cfg(test)]
use index::put_varint;
pub use index::{FolderIndex, Index};
pub use parse::from_people;
use parse::split_tokens;
pub(crate) use parse::split_tokens_with;

/// Lower-cased words of a text (UAX #29 word boundaries), as typed: what a server is asked for.
/// Search itself reads them [folded](fold_word).
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

impl Query {
    pub fn is_empty(&self) -> bool {
        self.clauses.is_empty()
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

/// What [`Index::to_bytes`] writes first, and the format's version.
const INDEX_MAGIC: &[u8] = b"MCIX";
const INDEX_VERSION: u8 = 1;
/// What [`FolderIndex::to_bytes`] writes first, and the format's version.
const FOLDER_MAGIC: &[u8] = b"MCFI";
/// Raised whenever what the index holds of a text changes, so a folder kept by an older run is
/// read again (2: words folded; 3: the parts of joined words).
const FOLDER_VERSION: u8 = 3;
/// Bounds read back from a file: past them the file isn't ours any more (or is damaged), and it
/// is rebuilt.
const MAX_INDEX_DOCS: usize = 8_000_000;
const MAX_INDEX_WORDS: usize = 8_000_000;
const MAX_WORD_BYTES: usize = 160;

#[cfg(test)]
mod tests;
