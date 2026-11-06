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

    /// Add a document's text (call again with more text, e.g. the body once fetched). Its words
    /// are [folded](super::fold_word), with their [parts](super::parts).
    pub fn add(&mut self, doc: u32, text: &str) {
        self.docs.insert(doc);
        // Folded, with their parts ("site.word" is `site` and `word` too), and long "words"
        // (base64 debris) left out: they only bloat it.
        for w in index_tokens(text) {
            let list = self.postings.entry(w.into_owned()).or_default();
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

    /// How big it is: its words, their documents (each counted once per word holding it) and the
    /// words' bytes. For measuring what an index costs in memory.
    pub fn sizes(&self) -> (usize, usize, usize) {
        let postings = self.postings.values().map(Vec::len).sum();
        let bytes = self.postings.keys().map(String::len).sum();
        (self.postings.len(), postings, bytes)
    }

    /// Documents containing a word starting with `prefix` (folded, as the index holds it).
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
        // Into nothing (the first folder a search joins): taken as it is.
        if offset == 0 && self.postings.is_empty() && self.docs.is_empty() {
            *self = other;
            return;
        }
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

    /// Keep the documents `map` gives a new number (`map[d]`), numbered so, and drop the others:
    /// one pass over the words, however few documents go. The new numbers must keep the old
    /// ones' order (each word's list stays sorted, never sorted again).
    pub fn renumber(&mut self, map: &[Option<u32>]) {
        let new = |d: u32| map.get(d as usize).copied().flatten();
        self.postings.retain(|_, list| {
            list.retain_mut(|d| match new(*d) {
                Some(n) => {
                    *d = n;
                    true
                }
                None => false,
            });
            !list.is_empty()
        });
        self.docs = self.docs.iter().filter_map(|&d| new(d)).collect();
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

/// Keep the items of `v` whose place `kept` says.
fn keep_at<T>(v: &mut Vec<T>, kept: impl Fn(usize) -> bool) {
    *v = std::mem::take(v).into_iter().enumerate().filter(|(i, _)| kept(*i)).map(|(_, x)| x).collect();
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
/// in the order of their number (whether each had a body, and a sum of what its summary gave the
/// index, [`summary_sum`]), and the words.
///
/// Kept per folder. The stamp covers the messages' text, not their flags: marking mail read
/// doesn't throw the index away. A folder that changed keeps what didn't: the messages gone, or
/// whose summary changed (their sum), are taken out ([`FolderIndex::retain`]), and those come in
/// are read alone (see the session); bodies read since are added when the index is read back.
#[derive(Clone, Debug, Default)]
pub struct FolderIndex {
    pub stamp: u64,
    pub keys: Vec<MessageKey>,
    pub has_body: Vec<bool>,
    /// Each message's [`summary_sum`]; empty when read from a file of version 3, which had none.
    pub sums: Vec<u64>,
    pub index: Index,
}

impl FolderIndex {
    pub fn new(stamp: u64) -> FolderIndex {
        FolderIndex { stamp, ..Default::default() }
    }

    /// Keep the messages `keep` says (by their number), numbered again from zero in their order:
    /// how many went.
    pub fn retain(&mut self, keep: &[bool]) -> usize {
        let mut map = Vec::with_capacity(self.keys.len());
        let mut next = 0u32;
        for i in 0..self.keys.len() {
            if keep.get(i) == Some(&true) {
                map.push(Some(next));
                next = next.saturating_add(1);
            } else {
                map.push(None);
            }
        }
        let gone = self.keys.len() - next as usize;
        if gone == 0 {
            return 0;
        }
        self.index.renumber(&map);
        let kept = |i: usize| map.get(i).is_some_and(Option::is_some);
        keep_at(&mut self.keys, kept);
        keep_at(&mut self.has_body, kept);
        keep_at(&mut self.sums, kept);
        gone
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(32 + self.keys.len() * 18);
        out.extend_from_slice(FOLDER_MAGIC);
        out.push(FOLDER_VERSION);
        out.extend_from_slice(&self.stamp.to_le_bytes());
        let n = self.keys.len().min(self.has_body.len());
        put_varint(&mut out, u32::try_from(n).unwrap_or(u32::MAX));
        for (i, k) in self.keys.iter().take(n).enumerate() {
            out.extend_from_slice(&k.validity.to_le_bytes());
            out.extend_from_slice(&k.uid.to_le_bytes());
            out.push(u8::from(self.has_body.get(i) == Some(&true)));
            // A sum not known is one no summary has: the message is read again.
            out.extend_from_slice(&self.sums.get(i).copied().unwrap_or(0).to_le_bytes());
        }
        let words = self.index.to_bytes();
        put_varint(&mut out, u32::try_from(words.len()).unwrap_or(u32::MAX));
        out.extend_from_slice(&words);
        out
    }

    /// Read back a folder index; `None` for a damaged file or one of another format. `account`
    /// and `folder` aren't kept in it: the file's place says which folder it is. A file of
    /// version 3 (the same words, no sums) is read too, its sums left empty: the session takes it
    /// only for a folder that didn't change.
    pub fn from_bytes(account: &str, folder: &str, bytes: &[u8]) -> Option<FolderIndex> {
        let mut r = Bytes { b: bytes, at: 0 };
        if r.take(FOLDER_MAGIC.len())? != FOLDER_MAGIC {
            return None;
        }
        let version = r.u8()?;
        if version != FOLDER_VERSION && version != FOLDER_VERSION_UNSUMMED {
            return None;
        }
        let summed = version == FOLDER_VERSION;
        let stamp = r.u64()?;
        let n = r.varint()? as usize;
        if n > MAX_INDEX_DOCS {
            return None;
        }
        let mut fi = FolderIndex::new(stamp);
        fi.keys.reserve(n.min(1024));
        fi.has_body.reserve(n.min(1024));
        if summed {
            fi.sums.reserve(n.min(1024));
        }
        for _ in 0..n {
            let (validity, uid, body) = (r.u32()?, r.u32()?, r.u8()? != 0);
            fi.keys.push(MessageKey { account: account.to_string(), folder: folder.to_string(), validity, uid });
            fi.has_body.push(body);
            if summed {
                fi.sums.push(r.u64()?);
            }
        }
        let len = r.varint()? as usize;
        fi.index = Index::from_bytes(r.take(len)?)?;
        if r.at != bytes.len() {
            return None;
        }
        Some(fi)
    }
}
