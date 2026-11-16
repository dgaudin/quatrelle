// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A chat's words in tranches ([`Tranches`]): each holds at most [`TRANCHE`] messages, made one
//! after the other, and is cut as the pages are ([`Side`]): never between two messages of one
//! millisecond, but for a run of them longer than a tranche. A write finds a message, and routes
//! a new one, by the tranches' creation times ([`candidates`], [`route`]), as the pages do.

use std::ops::Range;

use super::*;

/// A chat's words as the index holds them: its tranches, the oldest first.
pub(super) struct Tranches {
    /// The chat's id, when a file of it says it (else empty).
    pub(super) chat: String,
    /// Its tranches, by their first message's time, then their newest's, then their number.
    pub(super) list: Vec<Segment>,
    /// The file has no stub at its head (`index/<hash>.json`): its words in the old format there
    /// (one segment), a head that can't be read, or none. Its tranches are in memory only (cut
    /// from the old segment, if any), written with the stub at the chat's next write, and leave
    /// the index whole.
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

/// What a write routes a chat's messages by, for each tranche: its oldest message's creation
/// time, its newest's, and how many it holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    pub(crate) first: i64,
    pub(crate) newest: i64,
    pub(crate) held: usize,
}

/// Where a message goes among a chat's tranches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Goes {
    /// Into the tranche at this place in the list.
    Into(usize),
    /// Into a new tranche, before them all.
    Before,
    /// Into a new tranche, after them all.
    After,
}

/// Where a message made at `created` goes among tranches ordered by their first times: into the
/// newest that begins at or before it (the first, when it's older than them all), but into a new
/// one, never into a full one (`cap` messages or more), when it's older than them all or newer
/// than the last. Of the last tranche's newest millisecond, it goes into it (a run of one
/// millisecond isn't parted by a new tranche), and so of the first's first.
pub(crate) fn route(spans: &[Span], created: i64, cap: usize) -> Goes {
    let Some(head) = spans.first() else { return Goes::After };
    if created < head.first {
        return if head.held >= cap { Goes::Before } else { Goes::Into(0) };
    }
    // The newest that begins at or before it: there's one, the first does.
    let at = spans.partition_point(|s| s.first <= created).saturating_sub(1);
    match spans.get(at) {
        Some(s) if at + 1 == spans.len() && created > s.newest && s.held >= cap => Goes::After,
        _ => Goes::Into(at),
    }
}

/// The tranches a message made at `created` may be held by, as a page is found
/// (`Copy::find`): the one that begins before it, and those that begin at it.
pub(crate) fn candidates(spans: &[Span], created: i64) -> Range<usize> {
    let below = spans.partition_point(|s| s.first < created);
    let upto = spans.partition_point(|s| s.first <= created);
    below.saturating_sub(1)..upto
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
}
