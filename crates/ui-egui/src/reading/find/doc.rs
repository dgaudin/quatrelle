// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What the find bar looks in: the text a message shows, in the order it reads (its plain text,
//! or its formatted text's spans, cells and boxes as they're drawn), folded once as search folds
//! it, with the way back from the fold to the text as written. Made from the text already
//! prepared for the pane, never from the message's source.

use std::ops::Range;

use quatrelle_engine::model::search::{fold_mapped, fold_word};

use super::super::long_text::{BLOCK_LINES, CHUNK, LONG, chunks};
use super::MOST;
use quatrelle_engine::mime;

/// A text span of a formatted message: where it is (its address, stable while the message is
/// shown), where its text starts in [`Doc::text`], how long it is.
#[derive(Clone, Copy)]
struct SpanAt {
    addr: usize,
    start: usize,
    len: usize,
}

/// A message's shown text, folded.
pub(crate) struct Doc {
    /// The text looked in: the plain text, or the formatted spans one after the other, a line
    /// break after each line.
    text: String,
    folded: String,
    /// For each byte of `folded`, where in `text` the letter it comes from starts.
    from: Vec<u32>,
    /// The formatted text's spans, by address.
    spans: Vec<SpanAt>,
    /// Where each part of a long message starts in `text` (the chunks of a long plain text, the
    /// blocks and tables of a long formatted one, as `long_text` lays them out).
    parts: Vec<usize>,
}

/// A span's address: what it is known by while it's drawn.
pub(super) fn addr(sp: &mime::Span) -> usize {
    std::ptr::from_ref(sp).addr()
}

impl Doc {
    /// A plain text (`p.text`), cut in chunks as [`super::super::long_text::plain`] lays it out
    /// when it's long.
    pub(crate) fn plain(text: &str) -> Doc {
        let parts = if text.len() > LONG { chunks(text, CHUNK).into_iter().map(|r| r.start).collect() } else { Vec::new() };
        Doc::made(text.to_string(), Vec::new(), parts)
    }

    /// A formatted message's text: its lines (or its layout's lines, tables, cells and the boxes
    /// placed over them), in the order they're drawn.
    pub(crate) fn formatted(p: &mime::Parsed) -> Doc {
        let mut b = Builder::default();
        if p.layout.is_empty() {
            for lines in p.rich.chunks(BLOCK_LINES) {
                b.parts.push(b.text.len());
                b.lines(lines);
            }
        } else {
            for block in &p.layout {
                match block {
                    mime::layout::Block::Text(lines) => {
                        for lines in lines.chunks(BLOCK_LINES) {
                            b.parts.push(b.text.len());
                            b.lines(lines);
                        }
                    }
                    mime::layout::Block::Table(t) => {
                        b.parts.push(b.text.len());
                        b.table(t, 0);
                    }
                }
            }
        }
        b.spans.sort_unstable_by_key(|s| s.addr);
        Doc::made(b.text, b.spans, b.parts)
    }

    fn made(text: String, spans: Vec<SpanAt>, parts: Vec<usize>) -> Doc {
        let (folded, from) = fold_mapped(&text);
        Doc { text, folded, from, spans, parts }
    }

    /// Where `query` is found (folded as search folds it: capitals and accents aside), in the
    /// text as written, in order, at most [`MOST`]; and whether there were more.
    pub(crate) fn search(&self, query: &str) -> (Vec<Range<usize>>, bool) {
        let q = fold_word(query);
        if q.is_empty() {
            return (Vec::new(), false);
        }
        let mut out: Vec<Range<usize>> = Vec::new();
        for (at, found) in self.folded.match_indices(q.as_ref()) {
            let Some(r) = self.back(at, at.saturating_add(found.len())) else { continue };
            // Two finds in one letter (`s` twice in "ß"): one.
            if out.last().is_some_and(|l| r.start < l.end) {
                continue;
            }
            if out.len() >= MOST {
                return (out, true);
            }
            out.push(r);
        }
        (out, false)
    }

    /// The text written for the fold's bytes `start..end`: from the letter the first comes from
    /// to the end of the one the last comes from, accents written apart after it included.
    fn back(&self, start: usize, end: usize) -> Option<Range<usize>> {
        let from = |i: usize| self.from.get(i).map(|f| *f as usize);
        let s = from(start)?;
        let last = from(end.checked_sub(1)?)?;
        let mut e = last.saturating_add(self.text.get(last..)?.chars().next()?.len_utf8());
        while let Some(c) = self.text.get(e..).and_then(|t| t.chars().next()).filter(|c| ('\u{300}'..='\u{36f}').contains(c)) {
            e = e.saturating_add(c.len_utf8());
        }
        (s < e).then_some(s..e)
    }

    /// Where a formatted span's text is in [`Doc::text`]: its start and length.
    pub(super) fn span(&self, sp: &mime::Span) -> Option<(usize, usize)> {
        let a = addr(sp);
        let i = self.spans.binary_search_by_key(&a, |s| s.addr).ok()?;
        self.spans.get(i).map(|s| (s.start, s.len))
    }

    /// The part of a long message that `at` is in.
    pub(super) fn part_of(&self, at: usize) -> Option<usize> {
        self.parts.partition_point(|p| *p <= at).checked_sub(1)
    }
}

/// The formatted text gathered, as `Body` draws it.
#[derive(Default)]
struct Builder {
    text: String,
    spans: Vec<SpanAt>,
    parts: Vec<usize>,
}

impl Builder {
    /// Lines as `Body::lines` draws them: a rule is no text, a picture neither (its description
    /// stands in for it only when it can't be shown).
    fn lines(&mut self, lines: &[Vec<mime::Span>]) {
        for line in lines {
            if !line.iter().any(|sp| sp.rule) {
                for sp in line.iter().filter(|sp| sp.image.is_none() && !sp.text.is_empty()) {
                    self.spans.push(SpanAt { addr: addr(sp), start: self.text.len(), len: sp.text.len() });
                    self.text.push_str(&sp.text);
                }
            }
            self.text.push('\n');
        }
    }

    /// Blocks as `Body::blocks` draws them, tables as deep as it goes.
    fn blocks(&mut self, blocks: &[mime::layout::Block], depth: usize) {
        for b in blocks {
            match b {
                mime::layout::Block::Text(lines) => self.lines(lines),
                mime::layout::Block::Table(t) if depth < 16 => self.table(t, depth),
                mime::layout::Block::Table(_) => {}
            }
        }
    }

    /// A table's rows, each cell's content then the boxes placed over it.
    fn table(&mut self, t: &mime::layout::Table, depth: usize) {
        for c in t.rows.iter().flatten() {
            self.blocks(&c.blocks, depth + 1);
            for o in &c.overlays {
                self.blocks(&o.blocks, depth + 1);
            }
        }
    }
}
