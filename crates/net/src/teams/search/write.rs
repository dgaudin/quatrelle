// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The index's writes: a chat's tranches staged to be written with the pages ([`Staged`]), only
//! those a write changes, put back in the index once they are, and the index held to its bounds.

use super::*;

/// A chat's tranches once messages went in, waiting for the pages' write.
pub(crate) struct Staged {
    hash: String,
    chat: Tranches,
    /// The files to write with the pages.
    files: Vec<(String, Vec<u8>)>,
    /// The numbers written empty, to be removed once they are.
    gone: Vec<u32>,
    /// The numbers written with messages: spared by the bound that follows.
    touched: Vec<u32>,
    /// The chat's oldest tranches left it to keep it within [`TRANCHES_MAX`] and [`SEGMENT_MAX`].
    trimmed: bool,
}

impl Staged {
    /// Its files, to be written with the pages.
    pub(crate) fn take_files(&mut self) -> Vec<(String, Vec<u8>)> {
        std::mem::take(&mut self.files)
    }
}

/// A draft cut into tranches of at most `cap` messages ([`Draft::cut`]), a tranche of more than
/// one message past [`TRANCHE_BYTES`] cut in two again (its size taken with the widest number,
/// [`TRANCHES_MAX`]: no number written has more digits).
pub(super) fn fitted(d: Draft, side: Side, cap: usize) -> Result<Vec<Draft>> {
    let mut out = Vec::new();
    let mut todo: Vec<Draft> = d.cut(side, cap).into_iter().rev().collect();
    while let Some(piece) = todo.pop() {
        let held = piece.held();
        if held > 1 && piece.to_bytes(TRANCHES_MAX)?.len() > TRANCHE_BYTES {
            todo.extend(piece.cut(Side::Halves, held.div_ceil(2)).into_iter().rev());
            continue;
        }
        out.push(piece);
    }
    Ok(out)
}

/// What a removal from the index takes away: one file, or a folder with everything under it.
#[derive(Clone, Copy)]
pub(super) enum Removal {
    File,
    Folder,
}

impl Copy {
    /// The words of messages [`Copy::upsert`] changed put in their chat's tranches (an edit's
    /// replacing the old ones, a deletion's taken out), the files to be written with the pages:
    /// the tranches it changes, and those it empties or drops written empty.
    /// [`Copy::settle_index`] takes them back once they are. `Err` when the chat's words can't be
    /// read from the file: the pages aren't written either.
    pub(crate) fn stage_index(&self, chat: &str, changed: &[ChatMessage]) -> Result<Option<Staged>> {
        if changed.is_empty() || !id_ok(chat) {
            return Ok(None);
        }
        let hash = chat_hash(chat);
        let mut ix = self.index.borrow_mut();
        let had = match ix.take_chat(&hash) {
            Some(t) => t,
            // A chat new to the index: the stub goes at its head with its first tranches, as for
            // a chat of the old format with nothing in it.
            None => self.chat_index(&mut ix, &hash)?.unwrap_or_else(|| Tranches { legacy: true, ..Tranches::empty(chat) }),
        };
        let caps = Caps { tranche: ix.tranche_cap(), chat: ix.chat_cap() };
        match staged(chat, &hash, had, changed, caps) {
            Ok(s) => Ok(Some(s)),
            Err(e) => {
                ix.reload(hash);
                Err(e)
            }
        }
    }

    /// Staged tranches back in the index once the pages' write is over: kept when they were
    /// written (the numbers left empty removed, and the index held to its bound), else read
    /// again from the file.
    pub(crate) fn settle_index(&self, staged: Option<Staged>, written: bool) {
        let Some(Staged { hash, mut chat, gone, touched, trimmed, .. }) = staged else { return };
        let mut ix = self.index.borrow_mut();
        if !written {
            ix.reload(hash);
            return;
        }
        // A number that can't be removed stays an orphan: written empty again at the chat's next
        // write.
        for n in gone {
            if self.store.remove_account_files(TEAMS_ACCOUNT, &tranche_key(&hash, n)).is_err() {
                log::warn!("Teams search: an emptied tranche couldn't leave the file");
                chat.orphans.push(n);
            }
        }
        ix.put_chat(hash.clone(), chat);
        if trimmed {
            self.mark_partial(&mut ix);
        }
        self.bound(&mut ix, Some((&hash, &touched)));
    }

    /// The index held to its bounds, in the file ([`Copy::index_cap`]) and in memory
    /// ([`RESIDENT_MAX`]): the oldest tranches of every chat leave it first (`keep` apart, the
    /// tranches just written), a chat of the old format whole.
    pub(super) fn bound(&self, ix: &mut Index, keep: Option<(&str, &[u32])>) {
        let cap = self.index_cap(ix);
        let held = ix.resident_cap();
        let mut total: u64 = ix.tranches().map(|s| s.bytes).fold(0, u64::saturating_add);
        let mut resident: u64 = ix.tranches().map(|s| s.resident).fold(0, u64::saturating_add);
        if total <= cap && resident <= held {
            return;
        }
        let spared = |hash: &str, key: u32| keep.is_some_and(|(h, keys)| h == hash && keys.contains(&key));
        let oldest: Vec<(String, u32)> = ix.by_newest.iter().rev().filter(|(_, h, k)| !spared(h, *k)).map(|(_, h, k)| (h.clone(), *k)).collect();
        for (hash, key) in oldest {
            if total <= cap && resident <= held {
                break;
            }
            if ix.segments.get(&hash).is_some_and(|t| t.legacy) {
                // Its tranches are in memory only. Its folder goes first (tranches a copy rolled
                // back left there), then its head: while the folder stays, so does the head, and
                // the chat is read again as rolled back.
                let Some(t) = ix.take_chat(&hash) else { continue };
                for s in &t.list {
                    total = total.saturating_sub(s.bytes);
                    resident = resident.saturating_sub(s.resident);
                }
                if !self.removed(&tranches_key(&hash), Removal::Folder) || !self.removed(&segment_key(&hash), Removal::File) {
                    log::warn!("Teams search: an old chat's words couldn't leave the file");
                }
            } else if let Some(seg) = ix.take_tranche(&hash, key) {
                total = total.saturating_sub(seg.bytes);
                resident = resident.saturating_sub(seg.resident);
                if !self.removed(&tranche_key(&hash, key), Removal::File) {
                    // An orphan of its chat: emptied at the chat's next write.
                    log::warn!("Teams search: an old tranche couldn't leave the file");
                    ix.segments.entry(hash).or_insert_with(|| Tranches::empty(&seg.chat)).orphans.push(key);
                }
            }
        }
        log::info!("Teams search: the oldest tranches left the index ({total} bytes kept of {cap}, about {resident} in memory)");
        self.mark_partial(ix);
    }

    /// A file or a folder of the index removed, then seen gone: the file not there any more, the
    /// folder listing no file. The store answers `Ok` to a removal it couldn't look at (a data
    /// file that can't be read), so its `Ok` alone isn't taken for done: `false` is a failure.
    pub(super) fn removed(&self, key: &str, what: Removal) -> bool {
        if self.store.remove_account_files(TEAMS_ACCOUNT, key).is_err() {
            return false;
        }
        match what {
            // Looked at without being read: anything there is refused past its cap of nothing.
            Removal::File => matches!(self.store.account_file_within(TEAMS_ACCOUNT, key, 0), Ok(None)),
            Removal::Folder => match self.store.account_files(TEAMS_ACCOUNT, key) {
                Ok(names) => names.is_empty(),
                Err(StoreError::Io { source, .. }) => source.kind() == std::io::ErrorKind::NotFound,
                Err(_) => false,
            },
        }
    }
}

/// The most messages of a tranche, and the most bytes of a chat's tranches, as written.
#[derive(Clone, Copy)]
struct Caps {
    tranche: usize,
    chat: usize,
}

/// A tranche of the chat a write works on: kept as it is until a message goes in or out of it,
/// then opened; its range followed as messages go in and out, so the next is routed by it.
struct Part {
    /// Its number in the file; `None` for one the write makes.
    key: Option<u32>,
    /// As it holds now; as it held last when it holds nothing.
    span: Span,
    kept: Option<Segment>,
    draft: Option<Draft>,
    /// Where messages went in it: before all it held, after them, among them.
    at_start: bool,
    at_end: bool,
    inside: bool,
}

impl Part {
    fn kept(seg: Segment) -> Part {
        Part { key: Some(seg.key), span: seg.span(), kept: Some(seg), draft: None, at_start: false, at_end: false, inside: false }
    }

    /// A new one, holding nothing yet.
    fn new(chat: &str, created: i64) -> Part {
        let span = Span { first: created, newest: created, held: 0 };
        Part { key: None, span, kept: None, draft: Some(Draft::empty(chat)), at_start: false, at_end: false, inside: false }
    }

    fn holds(&self, id: &str) -> bool {
        match (&self.draft, &self.kept) {
            (Some(d), _) => d.places.contains_key(id),
            (None, Some(seg)) => seg.place_of(id).is_some(),
            (None, None) => false,
        }
    }

    fn held(&self) -> usize {
        match (&self.draft, &self.kept) {
            (Some(d), _) => d.held(),
            (None, Some(seg)) => usize::try_from(seg.held).unwrap_or(usize::MAX),
            (None, None) => 0,
        }
    }

    /// Its draft: the tranche as it's kept opened, the first time.
    fn opened(&mut self) -> &mut Draft {
        let kept = self.kept.take();
        self.draft.get_or_insert_with(|| kept.as_ref().map(Draft::of).unwrap_or_default())
    }

    /// The message under `id` taken out; its range followed by [`Part::follow`].
    fn take_out(&mut self, id: &str) {
        self.opened().remove(&HashSet::from([id]));
    }

    /// Its range as it holds now (the last one when it holds nothing).
    fn follow(&mut self) {
        if let Some(span) = self.draft.as_ref().and_then(Draft::span) {
            self.span = span;
        }
    }

    /// A message put in, where it went among those it holds noted.
    fn put_in(&mut self, m: &ChatMessage) {
        let (was, had) = (self.span, self.held());
        if had > 0 {
            if m.created < was.first {
                self.at_start = true;
            } else if m.created > was.newest {
                self.at_end = true;
            } else {
                self.inside = true;
            }
        }
        let d = self.opened();
        d.add(&m.id, m.created, words_of(m));
        let held = d.held();
        self.span = if had > 0 {
            Span { first: was.first.min(m.created), newest: was.newest.max(m.created), held }
        } else {
            Span { first: m.created, newest: m.created, held }
        };
    }
}

/// A tranche of the chat once the write cut those it opened: kept as it was, or to be written.
enum Piece {
    Kept(Segment),
    Open(Draft),
}

/// A tranche of the chat as the write leaves it, numbered: kept, or written with these bytes.
enum Out {
    Kept(Segment),
    Written(Draft, Vec<u8>),
}

impl Out {
    fn bytes(&self) -> usize {
        match self {
            Out::Kept(seg) => usize::try_from(seg.bytes).unwrap_or(usize::MAX),
            Out::Written(_, bytes) => bytes.len(),
        }
    }
}

/// A chat's tranches with what changed: each message, oldest first, taken out of the tranche that
/// holds it and, unless it's deleted, put in the one its creation time routes it to ([`route`]);
/// only the tranches that changed cut and written (a chat without the stub's all, and the stub;
/// those freed of messages as they were read back), each tranche a write empties or drops (past
/// [`TRANCHES_MAX`] or its bytes, the oldest) and each orphan written empty, then removed.
fn staged(chat: &str, hash: &str, had: Tranches, changed: &[ChatMessage], caps: Caps) -> Result<Staged> {
    let Tranches { mut list, legacy, orphans, .. } = had;
    by_first(&mut list);
    // A chat without the stub's tranches are in memory only, and a fixed tranche's file holds
    // more than it: written.
    let mut parts: Vec<Part> = list
        .into_iter()
        .map(|seg| {
            let open = legacy || seg.fixed;
            let mut part = Part::kept(seg);
            if open {
                part.opened();
            }
            part
        })
        .collect();
    let mut spans: Vec<Span> = parts.iter().map(|p| p.span).collect();
    // Routed oldest first, whatever order they came in (Graph sends the newest first): past a full
    // tranche, new messages open one tranche, and the full one isn't written again.
    let mut ordered: Vec<&ChatMessage> = changed.iter().filter(|m| m.chat == chat && id_ok(&m.id)).collect();
    ordered.sort_by(|a, b| (a.created, a.id.as_str()).cmp(&(b.created, b.id.as_str())));
    for m in ordered {
        // Where it's held: among the tranches its creation time may be in, else in any (the time
        // moved, or the pages took it for a new one).
        let mut found = candidates(&spans, m.created)
            .find(|i| parts.get(*i).is_some_and(|p| p.holds(&m.id)))
            .or_else(|| parts.iter().position(|p| p.holds(&m.id)));
        if let Some(part) = found.and_then(|i| parts.get_mut(i)) {
            part.take_out(&m.id);
        }
        // Routed by the tranches as they were before it was taken out: an edit goes back where it
        // was.
        let mut routed = None;
        if m.deleted.is_none() {
            let at = match route(&spans, m.created, caps.tranche) {
                Goes::Into(i) => i,
                Goes::Before => {
                    let part = Part::new(chat, m.created);
                    spans.insert(0, part.span);
                    parts.insert(0, part);
                    found = found.map(|i| i.saturating_add(1));
                    0
                }
                Goes::After => {
                    let part = Part::new(chat, m.created);
                    spans.push(part.span);
                    parts.push(part);
                    parts.len().saturating_sub(1)
                }
            };
            if let Some(part) = parts.get_mut(at) {
                part.put_in(m);
            }
            routed = Some(at);
        }
        if let Some(part) = found.and_then(|i| parts.get_mut(i)) {
            part.follow();
        }
        for i in [found, routed].into_iter().flatten() {
            if let (Some(span), Some(part)) = (spans.get_mut(i), parts.get(i)) {
                *span = part.span;
            }
        }
    }

    // Each opened tranche cut to fit, as the pages are: its first piece keeps its number; emptied,
    // it frees it.
    let mut freed: Vec<u32> = Vec::new();
    let mut pieces: Vec<(Option<u32>, Piece)> = Vec::new();
    let last = parts.len().saturating_sub(1);
    for (p, part) in parts.into_iter().enumerate() {
        let fills = side(part.at_start, part.at_end, part.inside, p, last);
        match (part.draft, part.kept) {
            (Some(d), _) => {
                let mut cut = fitted(d, fills, caps.tranche)?.into_iter();
                match cut.next() {
                    Some(first) => pieces.push((part.key, Piece::Open(first))),
                    None => freed.extend(part.key),
                }
                pieces.extend(cut.map(|d| (None, Piece::Open(d))));
            }
            (None, Some(seg)) => pieces.push((part.key, Piece::Kept(seg))),
            (None, None) => freed.extend(part.key),
        }
    }
    // Past TRANCHES_MAX, the oldest go.
    let mut trimmed = false;
    let over = pieces.len().saturating_sub(usize::try_from(TRANCHES_MAX).unwrap_or(usize::MAX));
    for (key, _) in pieces.drain(..over) {
        freed.extend(key);
        trimmed = true;
    }
    // A new piece takes the lowest number no tranche holds, the oldest first (an orphan's is
    // free: it's written over).
    let mut used: BTreeSet<u32> = pieces.iter().filter_map(|(key, _)| *key).collect();
    let mut out: Vec<(u32, Out)> = Vec::with_capacity(pieces.len());
    for (key, piece) in pieces {
        let Some(n) = key.or_else(|| lowest_free(&used, TRANCHES_MAX)) else {
            trimmed = true;
            continue;
        };
        used.insert(n);
        out.push((
            n,
            match piece {
                Piece::Kept(seg) => Out::Kept(seg),
                Piece::Open(d) => {
                    let bytes = d.to_bytes(n)?;
                    Out::Written(d, bytes)
                }
            },
        ));
    }
    // Past the chat's bytes, the oldest go: the newest stays, whatever its size.
    let mut total: usize = out.iter().map(|(_, o)| o.bytes()).fold(0, usize::saturating_add);
    let mut dropped = 0;
    while total > caps.chat && out.len().saturating_sub(dropped) > 1 {
        total = total.saturating_sub(out.get(dropped).map_or(0, |(_, o)| o.bytes()));
        dropped += 1;
    }
    for (n, _) in out.drain(..dropped) {
        freed.push(n);
        trimmed = true;
    }

    let (mut files, mut list, mut touched) = (Vec::new(), Vec::new(), Vec::new());
    for (n, o) in out {
        match o {
            Out::Kept(seg) => list.push(seg),
            Out::Written(d, bytes) => {
                list.push(d.settle(n, bytes.len()));
                files.push((tranche_key(hash, n), bytes));
                touched.push(n);
            }
        }
    }
    // Whatever the file may hold that the chat doesn't any more, written empty in the same
    // transaction, then removed: not a number freed then taken again by a new piece (written over).
    let held: BTreeSet<u32> = list.iter().map(|s| s.key).collect();
    let gone: BTreeSet<u32> = freed.into_iter().chain(orphans).filter(|n| !held.contains(n)).collect();
    let empty = Draft::empty(chat);
    for n in &gone {
        files.push((tranche_key(hash, *n), empty.to_bytes(*n)?));
    }
    if legacy {
        files.push((segment_key(hash), stub(chat)?));
    }
    by_first(&mut list);
    let chat = Tranches { chat: chat.to_string(), list, legacy: false, orphans: Vec::new() };
    Ok(Staged { hash: hash.to_string(), chat, files, gone: gone.into_iter().collect(), touched, trimmed })
}
