// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A chat's words in tranches ([`Tranches`]): each holds at most [`TRANCHE`] messages, made one
//! after the other, and is cut as the pages are ([`Side`]): never between two messages of one
//! millisecond, but for a run of them longer than a tranche.

use std::ops::Range;

use super::*;

/// A chat's words as the index holds them: its tranches, the oldest first.
pub(super) struct Tranches {
    /// The chat's id, when a file of it says it (else empty).
    pub(super) chat: String,
    /// Its tranches, by their first message's time, then their newest's, then their number.
    pub(super) list: Vec<Segment>,
    /// Its words are in the old format in the file (one segment, at `index/<hash>.json`): cut in
    /// memory only, written as tranches at the chat's next write, and leaving the index whole.
    pub(super) legacy: bool,
    /// Numbers in the file the index doesn't hold (a tranche that couldn't be read or removed):
    /// written empty at the chat's next write, then removed.
    pub(super) orphans: Vec<u32>,
}

impl Tranches {
    pub(super) fn empty(chat: &str) -> Tranches {
        Tranches { chat: chat.to_string(), list: Vec::new(), legacy: false, orphans: Vec::new() }
    }

    /// A chat's words in the old format, `bytes` its segment's size, cut into tranches of at most
    /// `cap` messages (the newest [`TRANCHES_MAX`] kept), numbered 0, 1, … from the oldest; the
    /// segment's bytes shared among them by their messages.
    pub(super) fn from_legacy(d: Draft, bytes: usize, cap: usize) -> Tranches {
        let chat = d.chat.clone();
        let total = u64::try_from(d.held()).unwrap_or(u64::MAX).max(1);
        let bytes = u64::try_from(bytes).unwrap_or(u64::MAX);
        let mut pieces = d.cut(Side::Front, cap);
        let over = pieces.len().saturating_sub(usize::try_from(TRANCHES_MAX).unwrap_or(usize::MAX));
        pieces.drain(..over);
        let mut list: Vec<Segment> = pieces
            .into_iter()
            .zip(0u32..)
            .map(|(piece, n)| {
                let share = bytes.saturating_mul(u64::try_from(piece.held()).unwrap_or(u64::MAX)) / total;
                piece.settle(n, usize::try_from(share).unwrap_or(usize::MAX))
            })
            .collect();
        by_first(&mut list);
        Tranches { chat, list, legacy: true, orphans: Vec::new() }
    }
}

/// Tranches in the order a chat holds them: by their first message's time, then their newest's
/// (of two that begin in one millisecond, the one that can hold a newer message last), then by
/// number.
pub(super) fn by_first(list: &mut [Segment]) {
    list.sort_unstable_by_key(|s| (s.first, s.newest, s.key));
}

impl Draft {
    /// The messages it holds, by creation time then id, as compact pieces (their places 0, 1, …
    /// in that order, none free) of at most `cap` messages each: full pieces from its start
    /// ([`Side::Front`]), from its end ([`Side::Back`]), or halves again until each fits
    /// ([`Side::Halves`]). No edge falls between two messages of one millisecond, but in a run of
    /// them longer than `cap`. Nothing for a draft that holds nothing.
    pub(super) fn cut(self, side: Side, cap: usize) -> Vec<Draft> {
        let cap = cap.max(1);
        let Draft { chat, messages, places, words, .. } = self;
        let mut held: Vec<(i64, String, u32)> =
            places.into_iter().filter_map(|(id, p)| Some((messages.get(usize::try_from(p).ok()?)?.1, id, p))).collect();
        held.sort_unstable();
        let at = |h: &(i64, String, u32)| h.0;
        let n = held.len();
        let mut parts: Vec<Range<usize>> = Vec::new();
        match side {
            Side::Front => {
                let mut from = 0;
                while n.saturating_sub(from) > cap {
                    let k = edge_down(held.get(from..).unwrap_or_default(), cap, at);
                    parts.push(from..from + k);
                    from += k;
                }
                parts.push(from..n);
            }
            Side::Back => {
                let mut to = n;
                while to > cap {
                    let k = edge_up(held.get(..to).unwrap_or_default(), to - cap, at);
                    parts.push(k..to);
                    to = k;
                }
                parts.push(0..to);
                parts.reverse();
            }
            Side::Halves => parts.push(0..n),
        }
        // Halved until each fits.
        let mut ranges: Vec<Range<usize>> = Vec::new();
        let mut todo: Vec<Range<usize>> = parts.into_iter().rev().collect();
        while let Some(r) = todo.pop() {
            if r.len() > cap {
                let k = r.start + edge_near(held.get(r.clone()).unwrap_or_default(), r.len() / 2, at);
                todo.push(k..r.end);
                todo.push(r.start..k);
                continue;
            }
            if !r.is_empty() {
                ranges.push(r);
            }
        }
        // Each held place's piece, and its place there.
        let mut to: Vec<Option<(usize, u32)>> = vec![None; messages.len()];
        let mut out: Vec<Draft> = Vec::with_capacity(ranges.len());
        let mut held = held.into_iter();
        for (i, r) in ranges.iter().enumerate() {
            let mut piece = Draft::empty(&chat);
            for (q, (created, id, p)) in (0u32..).zip(held.by_ref().take(r.len())) {
                if let Some(slot) = usize::try_from(p).ok().and_then(|p| to.get_mut(p)) {
                    *slot = Some((i, q));
                }
                piece.places.insert(id.clone(), q);
                piece.messages.push((id, created));
            }
            out.push(piece);
        }
        // Each word's places, piece by piece: in a piece, a place's order is its message's.
        let mut mapped: Vec<(usize, u32)> = Vec::new();
        for (w, list) in words {
            mapped.clear();
            mapped.extend(list.iter().filter_map(|p| *to.get(usize::try_from(*p).ok()?)?));
            mapped.sort_unstable();
            mapped.dedup();
            for group in mapped.chunk_by(|a, b| a.0 == b.0) {
                if let Some(piece) = group.first().and_then(|(i, _)| out.get_mut(*i)) {
                    piece.words.insert(w.clone(), group.iter().map(|(_, q)| *q).collect());
                }
            }
        }
        out
    }

    /// A chat's tranches as one draft, compact (an id held twice kept once, its first).
    pub(super) fn merge(list: Vec<Segment>, chat: &str) -> Draft {
        let mut out = Draft::empty(chat);
        for seg in &list {
            let d = Draft::of(seg);
            // Its places in the merged draft.
            let mut to: Vec<Option<u32>> = Vec::with_capacity(d.messages.len());
            for (id, created) in d.messages {
                let place = u32::try_from(out.messages.len()).ok().filter(|_| !id.is_empty() && !out.places.contains_key(&id));
                if let Some(p) = place {
                    out.places.insert(id.clone(), p);
                    out.messages.push((id, created));
                }
                to.push(place);
            }
            for (w, places) in d.words {
                let into = out.words.entry(w).or_default();
                into.extend(places.iter().filter_map(|p| *to.get(usize::try_from(*p).ok()?)?));
            }
        }
        let Draft { messages, words, .. } = &mut out;
        words.retain(|_, list| {
            list.sort_unstable_by_key(|&p| order(messages, p));
            list.dedup();
            !list.is_empty()
        });
        out
    }
}
