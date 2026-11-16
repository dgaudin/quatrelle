// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The search of the local copy, in the Teams worker: the UI sends `Job::Search`, the worker
//! answers `Event::Found` with at most [`MAX_HITS`] (chat, message) ids, newest first; a chat
//! found by its name comes with an empty message id. The UI thread does no search work.
//!
//! * The index is the copy's own: a chat's words in tranches (`index/<chat hash>/<n>.json`,
//!   `tranche.rs`), each of at most [`TRANCHE`] messages made one after the other, with their
//!   ids and creation times and each word with the messages that have it (`segment.rs`). It is
//!   kept as [`Copy::upsert`] writes, in the same transaction as the pages: a message's words go
//!   in, an edit replaces them, a deletion takes them out (nothing of a deleted message stays). A
//!   write puts in only the tranches it changes; the index never rebuilds from scratch.
//! * A chat's head, `index/<chat hash>.json`, is a stub written with its first tranches: an empty
//!   segment to the old version, `moved` to this one. The old format, one segment a chat there,
//!   is read back as it was, cut into tranches in memory, and written as tranches at the chat's
//!   next write, the stub in its place. A segment without `moved` beside tranches is the old
//!   version's, written after them; tranches beside a head that can't be read, or without one
//!   (the old version takes it away as it lets a chat's words go), are left from before a
//!   rollback too: they go (they may hold the words of a message the old version deleted since),
//!   the head is taken for what it says (nothing, unless the old segment), and `partial` is set.
//! * Words are those of a message's text and its sender's name, [folded](fold_word) as the mail's
//!   search folds them (case, accents), each with its [parts](parts): at most [`INDEX_WORDS`]
//!   distinct ones a message, each cut to [`INDEX_WORD_CHARS`] characters. A query's words must
//!   all begin words of a message (or of the chat's name).
//! * Bounded: a chat's tranches are at most [`TRANCHES_MAX`] and [`SEGMENT_MAX`] bytes together
//!   (past them, its oldest tranches leave it); the whole index at most a quarter of the copy's
//!   file, between [`FLOOR`] and [`INDEX_MAX`], and in memory at most [`RESIDENT_MAX`] as
//!   estimated (held compact: sorted arrays, nothing twice), the oldest tranches of every chat
//!   leaving first (a chat of the old format whole), while the index is read back as after a
//!   write. After that, every search says it covers recent conversations only (`partial`,
//!   remembered in [`PARTIAL`]). A query reads the tranches of every chat newest first and stops
//!   once it has its hits and the next tranche is older than the last of them; in a tranche, each
//!   word's messages are held in the order they were made, so a query reads them newest first
//!   and stops at its hits: a long chat costs it no more than a short one.
//! * Read back at start in slices, one chat a loop turn ([`load`]): the window never waits for
//!   it, and until it's read whole a search answers "indexing" with no hits. What is read back
//!   is hostile input: a file under another chat's key or another number is passed over, the
//!   caps applied again, and a message a lower-numbered tranche holds, or one made past the next
//!   tranche's first, freed (its tranche written again at the chat's next write).
//! * Chats' names are not kept in the index: they come from the chats' records, read as the index
//!   is, and as they're written.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::time::{Duration, Instant};

use quatrelle_model::pim::teams::{
    self as model, Chat, ChatMessage, INDEX_MAX, INDEX_WORD_CHARS, INDEX_WORDS, MAX_CHATS, NO_NAME, Sender, TEAMS_ACCOUNT, row_name,
};
use quatrelle_model::search::{fold_word, parts, words};
use quatrelle_store::{Result, StoreError};
use serde::{Deserialize, Serialize};

use super::store::{ChatRecord, Copy, Side, chat_hash, edge_down, edge_near, edge_up, id_ok, lowest_free, side};
use super::worker::Worker;

mod load;
mod segment;
mod tranche;
mod write;

#[cfg(test)]
pub(crate) use load::fuzz_index_input;
pub use load::fuzz_teams_index;
use segment::{Draft, Form, Segment, query_words, stub, words_of, words_of_texts};
pub(crate) use tranche::{Goes, Span, candidates, route};
use tranche::{Tranches, by_first};
use write::Removal;
pub(crate) use write::Staged;

/// The most hits a search gives.
pub(crate) const MAX_HITS: usize = 200;
/// The most bytes of a chat's words, all its tranches together, as written and as read back; and
/// of a segment of the old format read back.
const SEGMENT_MAX: usize = 16 << 20;
/// The most messages of a tranche as written.
const TRANCHE: usize = 1_000;
/// A tranche of more than one message is cut past this many bytes.
const TRANCHE_BYTES: usize = 256 << 10;
/// The most bytes of one tranche read back.
const TRANCHE_READ_MAX: usize = 2 << 20;
/// The most tranches of a chat.
const TRANCHES_MAX: u32 = 1_024;
/// What the stub says in `moved`: the chat's words moved to tranches.
const MOVED: u32 = 2;
/// The most memory the index may take, as estimated (its words, ids and places as held).
const RESIDENT_MAX: u64 = 256 << 20;
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

/// The search's index of the copy, in memory: the tranches read back or written, and the chats'
/// names. A query goes through the chats whose names have its words (found by those words, never
/// by going through every name), and through the chats' words newest first until it has what it
/// may give: through every chat's words (a halving each) only when it finds fewer.
#[derive(Default)]
pub(crate) struct Index {
    /// The user's own id: a chat's name is said without them.
    me: String,
    /// The chats whose words are still to be read back (hashes); `None` until they're listed.
    todo: Option<Vec<String>>,
    /// Each chat's words, by the chat's hash.
    segments: HashMap<String, Tranches>,
    /// The tranches of every chat (a tranche's newest message's time, its chat's hash, its
    /// number), the one with the newest message first.
    by_newest: BTreeSet<(Reverse<i64>, String, u32)>,
    /// Each chat's name, by its hash.
    names: HashMap<String, Named>,
    /// Each word of the names, and the chats (hashes) whose name has it.
    name_words: BTreeMap<String, BTreeSet<String>>,
    partial: bool,
    /// The bound a test sets, in place of the copy's quarter.
    #[cfg(test)]
    pub(crate) cap: Option<u64>,
    /// The resident bound a test sets, in place of [`RESIDENT_MAX`].
    #[cfg(test)]
    pub(crate) resident_cap: Option<u64>,
    /// The most messages of a tranche a test sets, in place of [`TRANCHE`].
    #[cfg(test)]
    pub(crate) tranche_cap: Option<usize>,
    /// The most bytes of a chat's tranches as written a test sets, in place of [`SEGMENT_MAX`].
    #[cfg(test)]
    pub(crate) chat_cap: Option<usize>,
    /// How many tranches the last query looked at.
    #[cfg(test)]
    pub(crate) visited: std::cell::Cell<usize>,
    /// How many messages the last query looked at in the chats' words.
    #[cfg(test)]
    pub(crate) looked: std::cell::Cell<usize>,
    /// How many chats, by their names and by their words, the last query went through.
    #[cfg(test)]
    pub(crate) went_through: std::cell::Cell<usize>,
}

/// A chat's name as searched: its words, and its time (its last activity).
struct Named {
    chat: String,
    words: Vec<String>,
    at: i64,
}

/// A chat's stub, or its words in the old format.
fn segment_key(hash: &str) -> String {
    format!("index/{hash}.json")
}

/// The folder of a chat's tranches.
fn tranches_key(hash: &str) -> String {
    format!("index/{hash}")
}

/// A chat's tranche `n`.
fn tranche_key(hash: &str, n: u32) -> String {
    format!("index/{hash}/{n}.json")
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
        // The words of the name it had go.
        for w in self.names.get(&hash).map(|n| n.words.as_slice()).unwrap_or_default() {
            if let Some(chats) = self.name_words.get_mut(w) {
                chats.remove(&hash);
                if chats.is_empty() {
                    self.name_words.remove(w);
                }
            }
        }
        for w in &words {
            self.name_words.entry(w.clone()).or_default().insert(hash.clone());
        }
        self.names.insert(hash, Named { chat: chat.id.clone(), words, at });
    }

    /// A chat's words kept, in place of what it had, each tranche in its place among the newest:
    /// a chat that holds nothing (no tranche, no orphan, not in the old format) isn't kept.
    fn put_chat(&mut self, hash: String, t: Tranches) {
        self.take_chat(&hash);
        if t.list.is_empty() && t.orphans.is_empty() && !t.legacy {
            return;
        }
        for s in &t.list {
            self.by_newest.insert((Reverse(s.newest), hash.clone(), s.key));
        }
        self.segments.insert(hash, t);
    }

    /// A chat's words taken out.
    fn take_chat(&mut self, hash: &str) -> Option<Tranches> {
        let t = self.segments.remove(hash)?;
        for s in &t.list {
            self.by_newest.remove(&(Reverse(s.newest), hash.to_string(), s.key));
        }
        Some(t)
    }

    /// A chat's tranche taken out, by its number: the chat, left holding nothing, too.
    fn take_tranche(&mut self, hash: &str, key: u32) -> Option<Segment> {
        let t = self.segments.get_mut(hash)?;
        let at = t.list.iter().position(|s| s.key == key)?;
        let seg = t.list.remove(at);
        if t.list.is_empty() && t.orphans.is_empty() && !t.legacy {
            self.segments.remove(hash);
        }
        self.by_newest.remove(&(Reverse(seg.newest), hash.to_string(), key));
        Some(seg)
    }

    /// Every tranche it holds.
    fn tranches(&self) -> impl Iterator<Item = &Segment> {
        self.segments.values().flat_map(|t| &t.list)
    }

    /// The bytes its tranches take in the file.
    #[cfg(test)]
    pub(crate) fn bytes(&self) -> u64 {
        self.tranches().map(|s| s.bytes).sum()
    }

    /// What it takes in memory, as estimated.
    #[cfg(test)]
    pub(crate) fn resident(&self) -> u64 {
        self.tranches().map(|s| s.resident).sum()
    }

    /// The most messages of a tranche written.
    fn tranche_cap(&self) -> usize {
        #[cfg(test)]
        if let Some(cap) = self.tranche_cap {
            return cap;
        }
        TRANCHE
    }

    /// The most bytes of a chat's tranches written.
    fn chat_cap(&self) -> usize {
        #[cfg(test)]
        if let Some(cap) = self.chat_cap {
            return cap;
        }
        SEGMENT_MAX
    }

    /// The most memory its tranches may take, as estimated.
    fn resident_cap(&self) -> u64 {
        #[cfg(test)]
        if let Some(cap) = self.resident_cap {
            return cap;
        }
        RESIDENT_MAX
    }

    /// Within its caps, as read back and as written: each chat's tranches at most
    /// [`TRANCHES_MAX`], under distinct numbers below it, at most [`SEGMENT_MAX`] bytes together,
    /// each of the chat (its id said with them), a message of it in one of them only, in the
    /// chat's order ([`by_first`]), each within a tranche's caps and its range what it holds; and
    /// among the newest, exactly the tranches held. `Err` names what isn't.
    pub(crate) fn within_caps(&self) -> std::result::Result<(), String> {
        let most = usize::try_from(TRANCHES_MAX).unwrap_or(usize::MAX);
        let mut held: BTreeSet<(Reverse<i64>, String, u32)> = BTreeSet::new();
        for (hash, t) in &self.segments {
            if t.list.len() > most {
                return Err(format!("a chat of {} tranches", t.list.len()));
            }
            let bytes = t.list.iter().map(|s| s.bytes).fold(0, u64::saturating_add);
            if bytes > u64::try_from(SEGMENT_MAX).unwrap_or(u64::MAX) {
                return Err(format!("a chat's tranches of {bytes} bytes"));
            }
            if !t.list.is_empty() && chat_hash(&t.chat) != *hash {
                return Err("a chat's tranches said of another chat".into());
            }
            let (mut numbers, mut ids) = (HashSet::new(), HashSet::new());
            for s in &t.list {
                if s.key >= TRANCHES_MAX || !numbers.insert(s.key) {
                    return Err(format!("a chat's tranche under number {}, past the cap or twice", s.key));
                }
                if chat_hash(&s.chat) != *hash {
                    return Err(format!("tranche {} of another chat", s.key));
                }
                let d = Draft::of(s);
                d.within_caps().map_err(|e| format!("tranche {}: {e}", s.key))?;
                if d.span() != Some(s.span()) {
                    return Err(format!("tranche {}: its range or count isn't what it holds", s.key));
                }
                if s.messages.iter().any(|(id, _)| !id.is_empty() && !ids.insert(&**id)) {
                    return Err(format!("tranche {}: a message another tranche of the chat holds", s.key));
                }
                held.insert((Reverse(s.newest), hash.clone(), s.key));
            }
            let order = |s: &Segment| (s.first, s.newest, s.key);
            if !t.list.windows(2).all(|pair| match pair {
                [a, b] => order(a) < order(b),
                _ => true,
            }) {
                return Err("a chat's tranches out of its order".into());
            }
        }
        if held != self.by_newest {
            return Err(format!("{} tranches among the newest, {} held", self.by_newest.len(), held.len()));
        }
        Ok(())
    }

    /// A chat's words to be read back again from the file (its write failed).
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
        #[cfg(test)]
        self.went_through.set(0);
        // The chats whose name has a word beginning with each of the terms.
        let mut named: Option<BTreeSet<&str>> = None;
        for t in &terms {
            let from = (std::ops::Bound::Included(t.as_str()), std::ops::Bound::Unbounded);
            let words = self.name_words.range::<str, _>(from).take_while(|(w, _)| w.starts_with(t.as_str()));
            let with: BTreeSet<&str> = words.flat_map(|(_, chats)| chats.iter().map(String::as_str)).collect();
            let both: BTreeSet<&str> = match named {
                None => with,
                Some(n) => n.intersection(&with).copied().collect(),
            };
            let none = both.is_empty();
            named = Some(both);
            if none {
                break;
            }
        }
        for n in named.unwrap_or_default().into_iter().filter_map(|hash| self.names.get(hash)) {
            #[cfg(test)]
            self.went_through.set(self.went_through.get() + 1);
            hits.push((n.at, n.chat.as_str(), ""));
            if hits.len() >= 4 * MAX_HITS {
                keep_newest(&mut hits);
            }
        }
        // The tranches with the newest messages first, of every chat: once there are enough hits,
        // a tranche whose newest message is older than the last of them can't give one, nor can
        // those after it.
        #[cfg(test)]
        self.visited.set(0);
        #[cfg(test)]
        self.looked.set(0);
        // Each chat's tranches by number, made as the query first comes to the chat.
        let mut by_key: HashMap<&str, HashMap<u32, &Segment>> = HashMap::new();
        let tranches = self.by_newest.iter().filter_map(|(_, hash, key)| {
            let t = self.segments.get(hash)?;
            let keys = by_key.entry(hash.as_str()).or_insert_with(|| t.list.iter().map(|s| (s.key, s)).collect());
            Some((t.chat.as_str(), *keys.get(key)?))
        });
        for (chat, seg) in tranches {
            if hits.len() >= MAX_HITS {
                keep_newest(&mut hits);
                if hits.last().is_some_and(|last| last.0 > seg.newest) {
                    break;
                }
            }
            #[cfg(test)]
            self.visited.set(self.visited.get() + 1);
            #[cfg(test)]
            self.went_through.set(self.went_through.get() + 1);
            let mut looked = 0usize;
            let newest = seg.newest(&terms, MAX_HITS, &mut looked);
            #[cfg(test)]
            self.looked.set(self.looked.get() + looked);
            for p in newest {
                if let Some((id, at)) = usize::try_from(p).ok().and_then(|at| seg.messages.get(at))
                    && !id.is_empty()
                {
                    hits.push((*at, chat, &**id));
                    if hits.len() >= 4 * MAX_HITS {
                        keep_newest(&mut hits);
                    }
                }
            }
        }
        keep_newest(&mut hits);
        Found { hits: hits.into_iter().map(|(_, c, m)| (c.to_string(), m.to_string())).collect(), indexing: false, partial: self.partial }
    }
}

impl Copy {
    /// What a query finds in the copy.
    pub(crate) fn found(&self, query: &str) -> Found {
        self.index.borrow().found(query)
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
