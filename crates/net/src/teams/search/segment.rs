// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A chat's words for the search: those of a message and of a query, folded and cut, and a
//! tranche of them as memory holds it ([`Segment`]), as a write or a read works on it
//! ([`Draft`]), and as the file keeps it: a tranche, the stub, or a chat's words in the old format
//! ([`Form`]). What is read back is hostile input: held to the caps again.

use std::collections::BinaryHeap;

use super::*;

/// A tranche of a chat's words as memory holds it, compact: its messages by place, and each
/// word, in order, with the places of those that have it, in the order those were made (then by
/// place: found by halving, read newest first). Nothing is held twice: what a change needs
/// besides (each message's place) is made for the change alone ([`Draft`]).
pub(super) struct Segment {
    pub(super) chat: Box<str>,
    /// Its number among its chat's tranches (`index/<hash>/<key>.json`).
    pub(super) key: u32,
    /// (message id, creation time) by place; a free place has an empty id.
    pub(super) messages: Box<[(Box<str>, i64)]>,
    words: Box<[(Box<str>, Box<[u32]>)]>,
    /// The creation time of its oldest message, and of its newest.
    pub(super) first: i64,
    pub(super) newest: i64,
    /// How many messages it holds.
    pub(super) held: u32,
    /// The places it holds, by their messages' ids: a message is found by halving.
    by_id: Box<[u32]>,
    /// The bytes it takes in the file, and in memory (as estimated).
    pub(super) bytes: u64,
    pub(super) resident: u64,
    /// Places were freed as it was read back (a message a lower-numbered tranche holds, or one
    /// past the next tranche's first): the file holds more than it, so it's written again at the
    /// chat's next write.
    pub(super) fixed: bool,
}

/// A chat's words, or a tranche of them, while they're read back or changed: the form one write
/// or one read works on, for the time it takes.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Draft {
    pub(super) chat: String,
    /// (message id, creation time) by place; a free place has an empty id.
    pub(super) messages: Vec<(String, i64)>,
    /// Each message's place, and the places free.
    pub(super) places: HashMap<String, u32>,
    pub(super) free: Vec<u32>,
    /// Each word and the places that have it, in the order those were made, then by place.
    pub(super) words: BTreeMap<String, Vec<u32>>,
}

/// What a file of the index holds, as read back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Form {
    /// A chat's words in the old format: one segment, at `index/<hash>.json`.
    Old,
    /// The stub left in the old segment's place once its chat is in tranches: an empty segment
    /// to the old version.
    Stub,
    /// A tranche, by the number it says it is.
    Tranche(u32),
}

/// Where a place goes in a word's list: by when its message was made, then by place (a place out
/// of `messages`, which a list never holds, first). The file keeps the lists in this order too;
/// they're put in it again as they're read back.
pub(super) fn order<T>(messages: &[(T, i64)], p: u32) -> (i64, u32) {
    (usize::try_from(p).ok().and_then(|at| messages.get(at)).map_or(i64::MIN, |m| m.1), p)
}

/// A tranche as it's kept.
#[derive(Serialize)]
struct SegmentOut<'a> {
    chat: &'a str,
    tranche: u32,
    messages: &'a [(String, i64)],
    words: &'a BTreeMap<String, Vec<u32>>,
}

/// The stub as it's kept: the old segment's shape, empty, and `moved`.
#[derive(Serialize)]
struct StubOut<'a> {
    chat: &'a str,
    messages: &'a [(String, i64)],
    words: &'a BTreeMap<String, Vec<u32>>,
    moved: u32,
}

/// A file of the index as it's read back: a tranche says its number, the stub `moved`, an old
/// segment neither.
#[derive(Deserialize)]
struct SegmentIn {
    chat: String,
    messages: Vec<(String, i64)>,
    words: BTreeMap<String, Vec<u32>>,
    #[serde(default)]
    tranche: Option<u32>,
    #[serde(default)]
    moved: Option<u32>,
}

/// The stub kept at `index/<hash>.json` once a chat of the old format is in tranches: the old
/// version reads it as a chat with no words.
pub(super) fn stub(chat: &str) -> Result<Vec<u8>> {
    let out = StubOut { chat, messages: &[], words: &BTreeMap::new(), moved: MOVED };
    serde_json::to_vec(&out).map_err(|source| StoreError::Json { path: "index".into(), source })
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
pub(super) fn words_of_texts<'a>(texts: impl Iterator<Item = &'a str>, most: usize) -> Vec<String> {
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
pub(super) fn words_of(m: &ChatMessage) -> Vec<String> {
    let name = match &m.sender {
        Sender::User { name, .. } | Sender::App { name, .. } => name.as_str(),
        Sender::Nobody => "",
    };
    let texts = std::iter::once(name).chain(m.body.blocks.iter().flat_map(|b| &b.runs).map(|r| r.text.as_str()));
    words_of_texts(texts, INDEX_WORDS)
}

/// A query's words, as the index holds words.
pub(super) fn query_words(query: &str) -> Vec<String> {
    let query: String = query.chars().take(QUERY_CHARS).collect();
    words_of_texts(std::iter::once(query.as_str()), QUERY_WORDS)
}

impl Draft {
    pub(super) fn empty(chat: &str) -> Draft {
        Draft { chat: chat.to_string(), ..Draft::default() }
    }

    /// A segment to be changed.
    pub(super) fn of(seg: &Segment) -> Draft {
        let mut d = Draft::empty(&seg.chat);
        d.messages = seg.messages.iter().map(|(id, at)| (id.to_string(), *at)).collect();
        for (p, (id, _)) in d.messages.iter().enumerate() {
            let Ok(p) = u32::try_from(p) else { break };
            if id.is_empty() {
                d.free.push(p);
            } else {
                d.places.insert(id.clone(), p);
            }
        }
        d.words = seg.words.iter().map(|(w, list)| (w.to_string(), list.to_vec())).collect();
        d
    }

    /// How many messages it holds.
    pub(super) fn held(&self) -> usize {
        self.places.len()
    }

    /// Its range and size; `None` when it holds nothing.
    pub(super) fn span(&self) -> Option<Span> {
        let mut times = self.messages.iter().filter(|(id, _)| !id.is_empty()).map(|(_, at)| *at);
        let at = times.next()?;
        let (first, newest) = times.fold((at, at), |(first, newest), at| (first.min(at), newest.max(at)));
        Some(Span { first, newest, held: self.held() })
    }

    /// As memory holds it, tranche `key`, `bytes` the size of its file.
    pub(super) fn settle(self, key: u32, bytes: usize) -> Segment {
        use std::mem::size_of;
        let n = |x: usize| u64::try_from(x).unwrap_or(u64::MAX);
        // The tranche, its chat's key in the index's map and its place among the newest (a hash
        // each), each message's id and time, each word and its places.
        let mut resident = n(size_of::<Segment>() + 96 + 80 + self.chat.len());
        let times = || self.messages.iter().filter(|(id, _)| !id.is_empty()).map(|(_, at)| *at);
        let (first, newest) = (times().min().unwrap_or(i64::MIN), times().max().unwrap_or(i64::MIN));
        let held = u32::try_from(self.held()).unwrap_or(u32::MAX);
        let messages: Box<[(Box<str>, i64)]> = self
            .messages
            .into_iter()
            .map(|(id, at)| {
                resident = resident.saturating_add(n(size_of::<(Box<str>, i64)>() + id.len()));
                (id.into_boxed_str(), at)
            })
            .collect();
        let words: Box<[(Box<str>, Box<[u32]>)]> = self
            .words
            .into_iter()
            .map(|(w, list)| {
                resident = resident.saturating_add(n(size_of::<(Box<str>, Box<[u32]>)>() + w.len()).saturating_add(n(list.len()).saturating_mul(4)));
                (w.into_boxed_str(), list.into_boxed_slice())
            })
            .collect();
        let id_at = |p: &u32| usize::try_from(*p).ok().and_then(|at| messages.get(at)).map_or("", |(id, _)| &**id);
        let mut by_id: Vec<u32> = (0u32..).zip(messages.iter()).filter(|(_, (id, _))| !id.is_empty()).map(|(p, _)| p).collect();
        by_id.sort_unstable_by(|a, b| id_at(a).cmp(id_at(b)));
        resident = resident.saturating_add(n(by_id.len()).saturating_mul(4));
        let by_id = by_id.into_boxed_slice();
        Segment { chat: self.chat.into_boxed_str(), key, messages, words, first, newest, held, by_id, bytes: n(bytes), resident, fixed: false }
    }

    /// A file of the index read back, when it's `hash`'s, at most `max` bytes, and can be read:
    /// what it holds, held to the caps (a stub holds nothing).
    pub(super) fn read(bytes: &[u8], hash: &str, max: usize) -> Option<(Draft, Form)> {
        if bytes.len() > max {
            log::warn!("Teams search: a file of the index past {max} bytes passed over");
            return None;
        }
        let Ok(kept) = serde_json::from_slice::<SegmentIn>(bytes) else {
            log::warn!("Teams search: a damaged file of the index passed over");
            return None;
        };
        if !id_ok(&kept.chat) || chat_hash(&kept.chat) != hash {
            return None;
        }
        let form = match (kept.tranche, kept.moved) {
            (Some(_), Some(_)) => return None,
            (Some(n), None) => Form::Tranche(n),
            (None, Some(_)) => return Some((Draft::empty(&kept.chat), Form::Stub)),
            (None, None) => Form::Old,
        };
        let mut seg = Draft::empty(&kept.chat);
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
            list.sort_unstable_by_key(|&p| order(&seg.messages, p));
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
        Some((seg, form))
    }

    /// Within the caps it's read back to: each message's id usable and held once; each word one
    /// the index may hold, at most [`INDEX_WORDS`] a message; each word's places in [`order`],
    /// none twice, free or past the messages. `Err` names what isn't.
    pub(super) fn within_caps(&self) -> std::result::Result<(), String> {
        let mut ids: HashSet<&str> = HashSet::new();
        for (p, (id, _)) in self.messages.iter().enumerate().filter(|(_, (id, _))| !id.is_empty()) {
            if !id_ok(id) || !ids.insert(id.as_str()) {
                return Err(format!("the message at place {p}: its id unusable or held twice"));
            }
        }
        let mut counts = vec![0usize; self.messages.len()];
        for (w, list) in &self.words {
            if !word_ok(w) {
                return Err(format!("a word the index can't hold ({} bytes)", w.len()));
            }
            let at = |p: &u32| order(&self.messages, *p);
            if !list.windows(2).all(|pair| match pair {
                [a, b] => at(a) < at(b),
                _ => true,
            }) {
                return Err(format!("a word's places out of order or twice ({} of them)", list.len()));
            }
            for p in list {
                let held = usize::try_from(*p).ok().filter(|at| self.messages.get(*at).is_some_and(|(id, _)| !id.is_empty()));
                match held.and_then(|at| counts.get_mut(at)) {
                    Some(n) => *n += 1,
                    None => return Err(format!("a word at place {p}: free or past the messages")),
                }
            }
        }
        match counts.iter().position(|n| *n > INDEX_WORDS) {
            Some(p) => Err(format!("the message at place {p}: more than {INDEX_WORDS} words")),
            None => Ok(()),
        }
    }

    /// As the file keeps it, tranche `tranche` of its chat.
    pub(super) fn to_bytes(&self, tranche: u32) -> Result<Vec<u8>> {
        let out = SegmentOut { chat: &self.chat, tranche, messages: &self.messages, words: &self.words };
        serde_json::to_vec(&out).map_err(|source| StoreError::Json { path: "index".into(), source })
    }

    /// The words of these messages taken out, their places freed.
    pub(super) fn remove(&mut self, ids: &HashSet<&str>) {
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
    pub(super) fn add(&mut self, id: &str, created: i64, words: Vec<String>) {
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
        let Draft { messages, words: lists, .. } = self;
        for w in words {
            let list = lists.entry(w).or_default();
            let at = list.partition_point(|&q| order(messages, q) < (created, place));
            if list.get(at) != Some(&place) {
                list.insert(at, place);
            }
        }
    }
}

impl Segment {
    /// The place of the message it holds under `id`.
    pub(super) fn place_of(&self, id: &str) -> Option<u32> {
        let id_at = |p: u32| usize::try_from(p).ok().and_then(|at| self.messages.get(at)).map_or("", |(id, _)| &**id);
        let at = self.by_id.binary_search_by(|&p| id_at(p).cmp(id)).ok()?;
        self.by_id.get(at).copied()
    }

    /// Its range and size.
    pub(super) fn span(&self) -> Span {
        Span { first: self.first, newest: self.newest, held: usize::try_from(self.held).unwrap_or(usize::MAX) }
    }

    /// As read back, with the messages under these ids freed: `fixed`, its file as it was.
    pub(super) fn freed(&self, ids: &HashSet<&str>) -> Segment {
        let mut d = Draft::of(self);
        d.remove(ids);
        let mut seg = d.settle(self.key, usize::try_from(self.bytes).unwrap_or(usize::MAX));
        seg.fixed = true;
        seg
    }

    /// The lists of the words beginning with `term`.
    fn lists(&self, term: &str) -> Vec<&[u32]> {
        let from = self.words.partition_point(|(w, _)| &**w < term);
        self.words.get(from..).unwrap_or_default().iter().take_while(|(w, _)| w.starts_with(term)).map(|(_, list)| &**list).collect()
    }

    /// The places of the newest messages (at most `most`, newest first) with a word beginning
    /// with each of `terms`: the term with the fewest messages read newest first, its words'
    /// lists merged, each message kept when every other term's lists have it. `looked` counts
    /// the messages read.
    pub(super) fn newest(&self, terms: &[String], most: usize, looked: &mut usize) -> Vec<u32> {
        let mut by_term: Vec<Vec<&[u32]>> = terms.iter().map(|t| self.lists(t)).collect();
        if by_term.iter().any(Vec::is_empty) {
            return Vec::new();
        }
        by_term.sort_by_key(|lists| lists.iter().map(|l| l.len()).sum::<usize>());
        let Some((walked, others)) = by_term.split_first() else { return Vec::new() };
        let at = |p: u32| order(&self.messages, p);
        let has = |lists: &Vec<&[u32]>, p: u32| lists.iter().any(|l| l.binary_search_by_key(&at(p), |&q| at(q)).is_ok());
        // Each list's last place not read yet (its newest), the newest of them on top.
        let mut heads: BinaryHeap<((i64, u32), usize, usize)> =
            walked.iter().enumerate().filter_map(|(j, l)| Some((at(*l.last()?), j, l.len().saturating_sub(1)))).collect();
        let (mut out, mut last) = (Vec::new(), None);
        while let Some(((_, p), j, i)) = heads.pop() {
            if let Some(next) = i.checked_sub(1).and_then(|n| Some((at(*walked.get(j)?.get(n)?), j, n))) {
                heads.push(next);
            }
            // The same message under two words of the term.
            if last == Some(p) {
                continue;
            }
            last = Some(p);
            *looked = looked.saturating_add(1);
            if others.iter().all(|lists| has(lists, p)) {
                out.push(p);
                if out.len() >= most {
                    break;
                }
            }
        }
        out
    }
}
