// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The search of the local copy, in the Teams worker: the UI sends `Job::Search`, the worker
//! answers `Event::Found` with at most [`MAX_HITS`] (chat, message) ids, newest first; a chat
//! found by its name comes with an empty message id. The UI thread does no search work.
//!
//! * The index is the copy's own (`index/<chat hash>.json`, one segment a chat): the chat's
//!   messages (id, creation time) and each word with the messages that have it. It is kept as
//!   [`Copy::upsert`] writes, in the same transaction as the pages: a message's words go in, an
//!   edit replaces them, a deletion takes them out (nothing of a deleted message stays). It never
//!   rebuilds from scratch.
//! * Words are those of a message's text and its sender's name, [folded](fold_word) as the mail's
//!   search folds them (case, accents), each with its [parts](parts): at most [`INDEX_WORDS`]
//!   distinct ones a message, each cut to [`INDEX_WORD_CHARS`] characters. A query's words must
//!   all begin words of a message (or of the chat's name).
//! * Bounded: a segment is at most [`SEGMENT_MAX`] bytes (past it, the chat's oldest messages
//!   leave it); the whole index at most a quarter of the copy's file, between [`FLOOR`] and
//!   [`INDEX_MAX`], the chats with the oldest messages leaving first. After that, every search
//!   says it covers recent conversations only (`partial`, remembered in [`PARTIAL`]).
//! * Read back at start in slices, one chat a loop turn ([`load`]): the window never waits for
//!   it, and until it's read whole a search answers "indexing" with no hits. What is read back
//!   is hostile input: a segment under another chat's key is passed over, the caps applied again.
//! * Chats' names are not kept in the index: they come from the chats' records, read as the index
//!   is, and as they're written.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::time::{Duration, Instant};

use quatrelle_model::pim::teams::{
    self as model, Chat, ChatMessage, INDEX_MAX, INDEX_WORD_CHARS, INDEX_WORDS, MAX_CHATS, NO_NAME, Sender, TEAMS_ACCOUNT, row_name,
};
use quatrelle_model::search::{fold_word, parts, words};
use quatrelle_store::{Result, StoreError};
use serde::{Deserialize, Serialize};

use super::store::{ChatRecord, Copy, chat_hash, id_ok};
use super::worker::Worker;

/// The most hits a search gives.
pub(crate) const MAX_HITS: usize = 200;
/// The most bytes of a chat's segment, as written and as read back.
const SEGMENT_MAX: usize = 16 << 20;
/// The least the index may take, whatever the copy's size: a small copy's index isn't cut.
const FLOOR: u64 = 16 << 20;
/// The record that says the oldest chats' words left the index.
const PARTIAL: &str = "index/partial";
/// The most words of a query looked for, and the most characters of it read.
const QUERY_WORDS: usize = 16;
const QUERY_CHARS: usize = 1_000;
/// How soon the loop comes back for the next slice while the index is read back.
const LOAD_TURN: Duration = Duration::from_millis(2);

/// What a search found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Found {
    /// (chat id, message id; empty for a chat found by its name), newest first.
    pub(crate) hits: Vec<(String, String)>,
    /// The index isn't read back whole yet: nothing was looked for.
    pub(crate) indexing: bool,
    /// The oldest chats' words left the index: only recent conversations were searched.
    pub(crate) partial: bool,
}

/// The search's index of the copy, in memory: the segments read back or written, and the chats'
/// names.
#[derive(Default)]
pub(crate) struct Index {
    /// The user's own id: a chat's name is said without them.
    me: String,
    /// The chats whose words are still to be read back (hashes); `None` until they're listed.
    todo: Option<Vec<String>>,
    /// Each chat's words, by the chat's hash.
    segments: HashMap<String, Segment>,
    /// Each chat's name, by its hash.
    names: HashMap<String, Named>,
    partial: bool,
    /// The bound a test sets, in place of the copy's quarter.
    #[cfg(test)]
    pub(crate) cap: Option<u64>,
}

/// A chat's name as searched: its words, and its time (its last activity).
struct Named {
    chat: String,
    words: Vec<String>,
    at: i64,
}

/// A chat's words: its messages by place, and each word with the places of those that have it.
#[derive(Default)]
struct Segment {
    chat: String,
    /// (message id, creation time) by place; a free place has an empty id.
    messages: Vec<(String, i64)>,
    /// Each message's place, and the places free.
    places: HashMap<String, u32>,
    free: Vec<u32>,
    /// Each word and the places that have it, in order.
    words: BTreeMap<String, Vec<u32>>,
    /// The bytes it takes in the file.
    bytes: u64,
}

/// A segment as it's kept.
#[derive(Serialize)]
struct SegmentOut<'a> {
    chat: &'a str,
    messages: &'a [(String, i64)],
    words: &'a BTreeMap<String, Vec<u32>>,
}

/// A segment as it's read back.
#[derive(Deserialize)]
struct SegmentIn {
    chat: String,
    messages: Vec<(String, i64)>,
    words: BTreeMap<String, Vec<u32>>,
}

/// A chat's segment once messages went in, waiting for the pages' write.
pub(crate) struct Staged {
    hash: String,
    segment: Segment,
    file: Option<(String, Vec<u8>)>,
    /// The chat's oldest messages left it to keep it within [`SEGMENT_MAX`].
    trimmed: bool,
}

impl Staged {
    /// Its file, to be written with the pages.
    pub(crate) fn take_file(&mut self) -> Option<(String, Vec<u8>)> {
        self.file.take()
    }
}

fn segment_key(hash: &str) -> String {
    format!("index/{hash}.json")
}

/// A word cut to [`INDEX_WORD_CHARS`] characters.
fn cut(w: &str) -> &str {
    match w.char_indices().nth(INDEX_WORD_CHARS) {
        Some((at, _)) => w.get(..at).unwrap_or(w),
        None => w,
    }
}

/// A word as the index may hold one: not empty, not too long, no space nor control character.
fn word_ok(w: &str) -> bool {
    !w.is_empty() && w.chars().nth(INDEX_WORD_CHARS).is_none() && !w.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// The distinct words of texts, folded, each with its parts and cut, at most `most`.
fn words_of_texts<'a>(texts: impl Iterator<Item = &'a str>, most: usize) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for text in texts {
        for w in words(text) {
            let folded = fold_word(&w).into_owned();
            let pieces: Vec<&str> = std::iter::once(folded.as_str()).chain(parts(&folded)).collect();
            for piece in pieces {
                let piece = cut(piece);
                if word_ok(piece) && seen.insert(piece.to_string()) {
                    out.push(piece.to_string());
                    if out.len() >= most {
                        return out;
                    }
                }
            }
        }
    }
    out
}

/// A message's words: its sender's name, then its text.
fn words_of(m: &ChatMessage) -> Vec<String> {
    let name = match &m.sender {
        Sender::User { name, .. } | Sender::App { name, .. } => name.as_str(),
        Sender::Nobody => "",
    };
    let texts = std::iter::once(name).chain(m.body.blocks.iter().flat_map(|b| &b.runs).map(|r| r.text.as_str()));
    words_of_texts(texts, INDEX_WORDS)
}

/// A query's words, as the index holds words.
fn query_words(query: &str) -> Vec<String> {
    let query: String = query.chars().take(QUERY_CHARS).collect();
    words_of_texts(std::iter::once(query.as_str()), QUERY_WORDS)
}

impl Segment {
    fn empty(chat: &str) -> Segment {
        Segment { chat: chat.to_string(), ..Segment::default() }
    }

    /// A segment read back, when it's `hash`'s and can be read: held to the caps.
    fn read(bytes: &[u8], hash: &str) -> Option<Segment> {
        if bytes.len() > SEGMENT_MAX {
            log::warn!("Teams search: a chat's words past {SEGMENT_MAX} bytes passed over");
            return None;
        }
        let Ok(kept) = serde_json::from_slice::<SegmentIn>(bytes) else {
            log::warn!("Teams search: a damaged index segment passed over");
            return None;
        };
        if !id_ok(&kept.chat) || chat_hash(&kept.chat) != hash {
            return None;
        }
        let mut seg = Segment::empty(&kept.chat);
        seg.messages = kept.messages;
        for (p, (id, created)) in seg.messages.iter_mut().enumerate() {
            let p = u32::try_from(p).ok()?;
            // A usable id, once: else a free place.
            if id_ok(id) && !seg.places.contains_key(id.as_str()) {
                seg.places.insert(id.clone(), p);
            } else {
                id.clear();
                *created = 0;
                seg.free.push(p);
            }
        }
        let mut counts = vec![0usize; seg.messages.len()];
        for (w, mut list) in kept.words {
            if !word_ok(&w) {
                continue;
            }
            list.sort_unstable();
            list.dedup();
            list.retain(|p| {
                let Ok(at) = usize::try_from(*p) else { return false };
                let used = seg.messages.get(at).is_some_and(|(id, _)| !id.is_empty());
                match counts.get_mut(at) {
                    Some(n) if used && *n < INDEX_WORDS => {
                        *n += 1;
                        true
                    }
                    _ => false,
                }
            });
            if !list.is_empty() {
                seg.words.insert(w, list);
            }
        }
        seg.bytes = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        Some(seg)
    }

    fn to_bytes(&self) -> Result<Vec<u8>> {
        let out = SegmentOut { chat: &self.chat, messages: &self.messages, words: &self.words };
        serde_json::to_vec(&out).map_err(|source| StoreError::Json { path: "index".into(), source })
    }

    /// The words of these messages taken out, their places freed.
    fn remove(&mut self, ids: &HashSet<&str>) {
        let gone: BTreeSet<u32> = ids.iter().filter_map(|id| self.places.remove(*id)).collect();
        self.clear(&gone);
    }

    /// The words of the messages at these places taken out, the places freed.
    fn clear(&mut self, gone: &BTreeSet<u32>) {
        if gone.is_empty() {
            return;
        }
        self.words.retain(|_, list| {
            list.retain(|p| !gone.contains(p));
            !list.is_empty()
        });
        for p in gone {
            if let Some(slot) = usize::try_from(*p).ok().and_then(|at| self.messages.get_mut(at)) {
                self.places.remove(slot.0.as_str());
                *slot = (String::new(), 0);
                self.free.push(*p);
            }
        }
    }

    /// A message's words put in, at a free place or a new one.
    fn add(&mut self, id: &str, created: i64, words: Vec<String>) {
        let place = match self.free.pop() {
            Some(p) => p,
            None => {
                let Ok(p) = u32::try_from(self.messages.len()) else { return };
                self.messages.push((String::new(), 0));
                p
            }
        };
        let Some(slot) = usize::try_from(place).ok().and_then(|at| self.messages.get_mut(at)) else { return };
        *slot = (id.to_string(), created);
        self.places.insert(id.to_string(), place);
        for w in words {
            let list = self.words.entry(w).or_default();
            if let Err(at) = list.binary_search(&place) {
                list.insert(at, place);
            }
        }
    }

    /// The oldest quarter of its messages taken out (at least one).
    fn trim_oldest(&mut self) {
        let mut held: Vec<(i64, u32)> = self.places.values().filter_map(|p| Some((self.messages.get(usize::try_from(*p).ok()?)?.1, *p))).collect();
        held.sort_unstable();
        let n = (held.len() / 4).max(1);
        let gone: BTreeSet<u32> = held.into_iter().take(n).map(|(_, p)| p).collect();
        self.clear(&gone);
    }

    /// The creation time of its newest message.
    fn newest(&self) -> i64 {
        self.messages.iter().filter(|(id, _)| !id.is_empty()).map(|(_, at)| *at).max().unwrap_or(i64::MIN)
    }

    /// The places of the messages with a word beginning with each of `terms`.
    fn find(&self, terms: &[String]) -> BTreeSet<u32> {
        let mut found: Option<BTreeSet<u32>> = None;
        for t in terms {
            let mut with: BTreeSet<u32> = BTreeSet::new();
            for (_, list) in self
                .words
                .range::<str, _>((std::ops::Bound::Included(t.as_str()), std::ops::Bound::Unbounded))
                .take_while(|(w, _)| w.starts_with(t.as_str()))
            {
                with.extend(list.iter().copied());
            }
            let now = match found {
                None => with,
                Some(f) => f.intersection(&with).copied().collect(),
            };
            if now.is_empty() {
                return now;
            }
            found = Some(now);
        }
        found.unwrap_or_default()
    }
}

/// Hits kept to the newest [`MAX_HITS`]: by time, newest first, then by chat and message.
fn keep_newest(hits: &mut Vec<(i64, &str, &str)>) {
    hits.sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)).then_with(|| a.2.cmp(b.2)));
    hits.dedup();
    hits.truncate(MAX_HITS);
}

impl Index {
    /// The index of the copy made for `me`, not read back yet.
    pub(crate) fn new(me: &str) -> Index {
        Index { me: me.to_string(), ..Index::default() }
    }

    /// Whether it's read back whole.
    pub(crate) fn loaded(&self) -> bool {
        self.todo.as_ref().is_some_and(Vec::is_empty)
    }

    /// A chat's name, as it's now.
    pub(crate) fn name(&mut self, chat: &Chat) {
        if !id_ok(&chat.id) {
            return;
        }
        let hash = chat_hash(&chat.id);
        if self.names.len() >= MAX_CHATS && !self.names.contains_key(&hash) {
            return;
        }
        let name = row_name(chat, &self.me);
        let words = if name == NO_NAME { Vec::new() } else { words_of_texts(std::iter::once(name.as_str()), INDEX_WORDS) };
        let at = chat.last_activity.or(chat.updated).unwrap_or(i64::MIN);
        self.names.insert(hash, Named { chat: chat.id.clone(), words, at });
    }

    /// A chat's segment to be read back again from the file (its write failed).
    fn reload(&mut self, hash: String) {
        if let Some(todo) = self.todo.as_mut() {
            todo.push(hash);
        }
    }

    /// What a query finds.
    fn found(&self, query: &str) -> Found {
        if !self.loaded() {
            return Found { hits: Vec::new(), indexing: true, partial: self.partial };
        }
        let terms = query_words(query);
        if terms.is_empty() {
            return Found { hits: Vec::new(), indexing: false, partial: self.partial };
        }
        let mut hits: Vec<(i64, &str, &str)> = Vec::new();
        for seg in self.segments.values() {
            for p in seg.find(&terms) {
                if let Some((id, at)) = usize::try_from(p).ok().and_then(|at| seg.messages.get(at))
                    && !id.is_empty()
                {
                    hits.push((*at, seg.chat.as_str(), id.as_str()));
                    if hits.len() >= 4 * MAX_HITS {
                        keep_newest(&mut hits);
                    }
                }
            }
        }
        for n in self.names.values() {
            if terms.iter().all(|t| n.words.iter().any(|w| w.starts_with(t.as_str()))) {
                hits.push((n.at, n.chat.as_str(), ""));
                if hits.len() >= 4 * MAX_HITS {
                    keep_newest(&mut hits);
                }
            }
        }
        keep_newest(&mut hits);
        Found { hits: hits.into_iter().map(|(_, c, m)| (c.to_string(), m.to_string())).collect(), indexing: false, partial: self.partial }
    }
}

impl Copy {
    /// One slice of the index read back: the chats listed, or one chat's name and words. `false`
    /// when it's read whole.
    pub(crate) fn index_step(&self) -> bool {
        let mut ix = self.index.borrow_mut();
        let next = ix.todo.as_mut().map(Vec::pop);
        match next {
            None => {
                let mut hashes = match self.store.account_files(TEAMS_ACCOUNT, "chats") {
                    Ok(names) => names,
                    Err(StoreError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                    Err(_) => {
                        log::warn!("Teams search: the copy's chats couldn't be listed; nothing is searched");
                        Vec::new()
                    }
                };
                hashes
                    .retain(|n| n.strip_suffix(".json").is_some_and(|h| h.len() == 32 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))));
                hashes.sort_unstable();
                hashes.truncate(MAX_CHATS);
                // Popped from the end: read in the listing's order.
                let todo: Vec<String> = hashes.iter().rev().filter_map(|n| n.strip_suffix(".json").map(str::to_string)).collect();
                ix.partial = self.read(PARTIAL).is_some();
                ix.todo = Some(todo);
                true
            }
            Some(None) => false,
            Some(Some(hash)) => {
                if let Some(r) = self.record_by_hash(&hash) {
                    ix.name(&r.chat);
                }
                if !ix.segments.contains_key(&hash)
                    && let Some(seg) = self.segment_from_file(&hash)
                {
                    ix.segments.insert(hash, seg);
                }
                if ix.loaded() {
                    self.bound(&mut ix, None);
                }
                true
            }
        }
    }

    /// What a query finds in the copy.
    pub(crate) fn found(&self, query: &str) -> Found {
        self.index.borrow().found(query)
    }

    /// A chat's record by its hash, when it's that chat's.
    fn record_by_hash(&self, hash: &str) -> Option<ChatRecord> {
        let r: ChatRecord = serde_json::from_slice(&self.read(&format!("chats/{hash}.json"))?).ok()?;
        (id_ok(&r.chat.id) && chat_hash(&r.chat.id) == hash).then_some(r)
    }

    fn segment_from_file(&self, hash: &str) -> Option<Segment> {
        Segment::read(&self.read(&segment_key(hash))?, hash)
    }

    /// The words of messages [`Copy::upsert`] changed put in their chat's segment (an edit's
    /// replacing the old ones, a deletion's taken out), the segment's file to be written with
    /// the pages. [`Copy::settle_index`] takes it back once they are.
    pub(crate) fn stage_index(&self, chat: &str, changed: &[ChatMessage]) -> Result<Option<Staged>> {
        if changed.is_empty() || !id_ok(chat) {
            return Ok(None);
        }
        let hash = chat_hash(chat);
        let mut ix = self.index.borrow_mut();
        let mut seg = match ix.segments.remove(&hash) {
            Some(seg) => seg,
            None => self.segment_from_file(&hash).unwrap_or_else(|| Segment::empty(chat)),
        };
        let ids: HashSet<&str> = changed.iter().map(|m| m.id.as_str()).collect();
        seg.remove(&ids);
        for m in changed.iter().filter(|m| m.chat == chat && m.deleted.is_none() && id_ok(&m.id)) {
            seg.add(&m.id, m.created, words_of(m));
        }
        let mut trimmed = false;
        let mut bytes = match seg.to_bytes() {
            Ok(b) => b,
            Err(e) => {
                ix.reload(hash);
                return Err(e);
            }
        };
        for _ in 0..16 {
            if bytes.len() <= SEGMENT_MAX || seg.places.is_empty() {
                break;
            }
            seg.trim_oldest();
            trimmed = true;
            bytes = match seg.to_bytes() {
                Ok(b) => b,
                Err(e) => {
                    ix.reload(hash);
                    return Err(e);
                }
            };
        }
        seg.bytes = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let key = segment_key(&hash);
        Ok(Some(Staged { hash, segment: seg, file: Some((key, bytes)), trimmed }))
    }

    /// A staged segment back in the index once the pages' write is over: kept when it was
    /// written (and the index held to its bound), else read again from the file.
    pub(crate) fn settle_index(&self, staged: Option<Staged>, written: bool) {
        let Some(s) = staged else { return };
        let mut ix = self.index.borrow_mut();
        if !written {
            ix.reload(s.hash);
            return;
        }
        let hash = s.hash.clone();
        ix.segments.insert(s.hash, s.segment);
        if s.trimmed {
            self.mark_partial(&mut ix);
        }
        if ix.loaded() {
            self.bound(&mut ix, Some(&hash));
        }
    }

    /// The index held to its bound: the chats with the oldest messages leave it first (`keep`
    /// apart, the chat just written).
    fn bound(&self, ix: &mut Index, keep: Option<&str>) {
        let cap = self.index_cap(ix);
        let mut total: u64 = ix.segments.values().map(|s| s.bytes).fold(0, u64::saturating_add);
        if total <= cap {
            return;
        }
        let mut oldest: Vec<(i64, String)> =
            ix.segments.iter().filter(|(h, _)| Some(h.as_str()) != keep).map(|(h, s)| (s.newest(), h.clone())).collect();
        oldest.sort_unstable();
        for (_, hash) in oldest {
            if total <= cap {
                break;
            }
            if let Some(seg) = ix.segments.remove(&hash) {
                total = total.saturating_sub(seg.bytes);
                if self.store.remove_account_files(TEAMS_ACCOUNT, &segment_key(&hash)).is_err() {
                    log::warn!("Teams search: an old chat's words couldn't leave the file");
                }
            }
        }
        log::info!("Teams search: the oldest chats' words left the index ({total} bytes kept of {cap})");
        self.mark_partial(ix);
    }

    fn mark_partial(&self, ix: &mut Index) {
        if ix.partial {
            return;
        }
        ix.partial = true;
        if self.store.write_account_files(TEAMS_ACCOUNT, &[(PARTIAL.to_string(), b"true".to_vec())]).is_err() {
            log::warn!("Teams search: that only recent chats are searched couldn't be written");
        }
    }

    /// The most bytes the index takes: a quarter of the copy's file, at least [`FLOOR`], at
    /// most [`INDEX_MAX`].
    fn index_cap(&self, ix: &Index) -> u64 {
        #[cfg(test)]
        if let Some(cap) = ix.cap {
            return cap;
        }
        #[cfg(not(test))]
        let _ = ix;
        match self.store.data_file(TEAMS_ACCOUNT).and_then(|f| std::fs::metadata(f).ok()) {
            Some(m) => (m.len() / 4).clamp(FLOOR, INDEX_MAX),
            None => INDEX_MAX,
        }
    }
}

/// One slice of the index read back, when the copy is open and it isn't whole yet: called each
/// turn of the loop, a chat's words at most (no request to Teams).
pub(crate) fn load(w: &Worker) {
    if let Some(copy) = &w.copy
        && !copy.index.borrow().loaded()
    {
        copy.index_step();
    }
}

/// When the loop looks again: at once (a turn apart) while the index is read back, whatever
/// else it waits for; else as `wake` says.
pub(crate) fn wake(w: &Worker, now: Instant, wake: Option<Instant>) -> Option<Instant> {
    let loading = w.copy.as_ref().is_some_and(|c| !c.index.borrow().loaded());
    if !loading {
        return wake;
    }
    let soon = now.checked_add(LOAD_TURN).unwrap_or(now);
    Some(wake.map_or(soon, |at| at.min(soon)))
}

/// `Job::Search`: the copy searched, the hits said. Signed out (no copy): nothing found.
pub(crate) fn search(w: &Worker, query: String) {
    let found = w.copy.as_ref().map(|c| c.found(&query)).unwrap_or_default();
    w.say(model::Event::Found { query, hits: found.hits, indexing: found.indexing, partial: found.partial });
}
