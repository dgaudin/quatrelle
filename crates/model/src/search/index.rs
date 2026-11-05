// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! An inverted index held in memory ([`Index`]), and a folder's index as kept between runs
//! ([`FolderIndex`]).

use std::collections::{BTreeMap, BTreeSet};
use std::ops::{Bound, RangeBounds};

use super::*;
use crate::MessageKey;

/// Words → documents. Document ids are the caller's (positions in its message table).
#[derive(Clone, Debug, Default)]
pub struct Index {
    /// Each word's documents, sorted (an array: a set per word took several times the memory,
    /// gigabytes for a big mailbox).
    postings: BTreeMap<String, Vec<u32>>,
    docs: BTreeSet<u32>,
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

/// A little-endian base-128 number, the short form of the numbers and gaps kept on disk.
pub(super) fn put_varint(out: &mut Vec<u8>, mut v: u32) {
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
