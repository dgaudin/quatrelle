// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A chat's words for the search: those of a message and of a query, folded and cut, and the
//! chat's segment as memory holds it ([`Segment`]), as a write or a read works on it ([`Draft`]),
//! and as the file keeps it. What is read back is hostile input: held to the caps again.

use super::*;

/// A chat's words as memory holds them, compact: its messages by place, and each word, in
/// order, with the places of those that have it, in order (found by halving). Nothing is held
/// twice: what a change needs besides (each message's place) is made for the change alone
/// ([`Draft`]).
pub(super) struct Segment {
    pub(super) chat: Box<str>,
    /// (message id, creation time) by place; a free place has an empty id.
    pub(super) messages: Box<[(Box<str>, i64)]>,
    words: Box<[(Box<str>, Box<[u32]>)]>,
    /// The creation time of its newest message.
    pub(super) newest: i64,
    /// The bytes it takes in the file, and in memory (as estimated).
    pub(super) bytes: u64,
    pub(super) resident: u64,
}

/// A chat's words while they're read back or changed: the form one write or one read works on,
/// for the time it takes.
#[derive(Default)]
pub(super) struct Draft {
    chat: String,
    /// (message id, creation time) by place; a free place has an empty id.
    messages: Vec<(String, i64)>,
    /// Each message's place, and the places free.
    pub(super) places: HashMap<String, u32>,
    free: Vec<u32>,
    /// Each word and the places that have it, in order.
    words: BTreeMap<String, Vec<u32>>,
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

    /// As memory holds it, `bytes` the size of its file.
    pub(super) fn settle(self, bytes: usize) -> Segment {
        use std::mem::size_of;
        let n = |x: usize| u64::try_from(x).unwrap_or(u64::MAX);
        // The segment, its key in the index's map (a hash), each message's id and time, each
        // word and its places.
        let mut resident = n(size_of::<Segment>() + 96 + self.chat.len());
        let newest = self.messages.iter().filter(|(id, _)| !id.is_empty()).map(|(_, at)| *at).max().unwrap_or(i64::MIN);
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
        Segment { chat: self.chat.into_boxed_str(), messages, words, newest, bytes: n(bytes), resident }
    }

    /// A segment read back, when it's `hash`'s and can be read: held to the caps.
    pub(super) fn read(bytes: &[u8], hash: &str) -> Option<Draft> {
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
        Some(seg)
    }

    pub(super) fn to_bytes(&self) -> Result<Vec<u8>> {
        let out = SegmentOut { chat: &self.chat, messages: &self.messages, words: &self.words };
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
        for w in words {
            let list = self.words.entry(w).or_default();
            if let Err(at) = list.binary_search(&place) {
                list.insert(at, place);
            }
        }
    }

    /// The oldest quarter of its messages taken out (at least one).
    pub(super) fn trim_oldest(&mut self) {
        let mut held: Vec<(i64, u32)> = self.places.values().filter_map(|p| Some((self.messages.get(usize::try_from(*p).ok()?)?.1, *p))).collect();
        held.sort_unstable();
        let n = (held.len() / 4).max(1);
        let gone: BTreeSet<u32> = held.into_iter().take(n).map(|(_, p)| p).collect();
        self.clear(&gone);
    }
}

impl Segment {
    /// The places of the messages with a word beginning with each of `terms`.
    pub(super) fn find(&self, terms: &[String]) -> BTreeSet<u32> {
        let mut found: Option<BTreeSet<u32>> = None;
        for t in terms {
            let from = self.words.partition_point(|(w, _)| &**w < t.as_str());
            let mut with: BTreeSet<u32> = BTreeSet::new();
            for (_, list) in self.words.get(from..).unwrap_or_default().iter().take_while(|(w, _)| w.starts_with(t.as_str())) {
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
