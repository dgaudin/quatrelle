// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What is selected across the text being written and the blocks of the original under it, taken
//! out in one go as Outlook takes it: the text at both ends, and every block wholly between them
//! (paragraphs, their pictures and rules, tables and all they hold). The text and the original as
//! they stood are kept as a [`Written`], for the editor's undo.

use super::*;

/// A place in a message being written over its original: `block` 0 is its own text, `block`
/// `n + 1` the original's block of text `n` ([`text_blocks`]); `at` counts characters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct QuotePoint {
    pub block: usize,
    pub at: usize,
}

/// A message's text and what was changed in its original, as they stood at one time.
#[derive(Clone, Debug, Default)]
pub struct Written {
    body: String,
    styles: Styles,
    edits: BTreeMap<usize, (String, Styles)>,
    removed: BTreeSet<usize>,
}

fn chars(t: &str) -> usize {
    t.chars().count()
}

impl Compose {
    /// The text and the original as they stand.
    pub fn written(&self) -> Written {
        let (edits, removed) = self.quote.as_ref().map(|q| (q.edits.clone(), q.removed.clone())).unwrap_or_default();
        Written { body: self.body.clone(), styles: self.styles.clone(), edits, removed }
    }

    /// Whether the text and the original read and look as `w` has them.
    pub fn is_written(&self, w: &Written) -> bool {
        let mut styles = self.styles.clone();
        styles.fit(&self.body);
        let mut was = w.styles.clone();
        was.fit(&w.body);
        if !same(&(self.body.clone(), styles), &(w.body.clone(), was)) {
            return false;
        }
        let Some(q) = self.quote.as_ref() else { return w.edits.is_empty() && w.removed.is_empty() };
        if q.removed != w.removed {
            return false;
        }
        // A block changed in one and not in the other: as it came, in the other.
        let came = text_blocks(&q.blocks);
        let of = |edits: &BTreeMap<usize, (String, Styles)>, n: usize| match edits.get(&n) {
            Some(e) => Some(e.clone()),
            None => came.get(n).map(|(lines, align)| cell_text(lines, *align)),
        };
        q.edits.keys().chain(w.edits.keys()).all(|n| match (of(&q.edits, *n), of(&w.edits, *n)) {
            (Some(a), Some(b)) => same(&a, &b),
            (a, b) => a.is_none() && b.is_none(),
        })
    }

    /// The text and the original as `w` has them.
    pub fn set_written(&mut self, w: Written) {
        self.body = w.body;
        self.styles = w.styles;
        if let Some(q) = self.quote.as_mut()
            && (q.edits != w.edits || q.removed != w.removed)
        {
            let q = Arc::make_mut(q);
            q.edits = w.edits;
            q.removed = w.removed;
        }
    }

    /// The blocks a selection can be in, in reading order: the text (0), then each block of text
    /// of the original still there, sent as HTML (as plain text the original is one text).
    pub fn selectable_blocks(&self) -> Vec<usize> {
        let mut out = vec![0];
        if let Some(q) = self.quote.as_ref().filter(|_| !self.plain) {
            let gone = q.gone();
            out.extend((0..q.cells()).filter(|n| !gone.contains(n)).map(|n| n.saturating_add(1)));
        }
        out
    }

    /// Block `block`'s text and styles ([`QuotePoint`]'s numbers).
    pub fn block_text(&self, block: usize) -> Option<(String, Styles)> {
        match block.checked_sub(1) {
            None => {
                let mut styles = self.styles.clone();
                styles.fit(&self.body);
                Some((self.body.clone(), styles))
            }
            Some(n) => self.quote.as_ref()?.cell(n),
        }
    }

    /// The text from `a` to `b` (either way round), in reading order, a block to a line.
    pub fn text_across(&self, a: QuotePoint, b: QuotePoint) -> String {
        let (from, to) = (a.min(b), a.max(b));
        let mut out: Vec<String> = Vec::new();
        for k in self.selectable_blocks().into_iter().filter(|k| (from.block..=to.block).contains(k)) {
            let Some((t, s)) = self.block_text(k) else { continue };
            let start = if k == from.block { from.at } else { 0 };
            let end = if k == to.block { to.at } else { chars(&t) };
            let (t, s) = rich::slice(&t, &s, start..end.max(start));
            out.push(if self.plain { t } else { rich::to_text(&t, &s) });
        }
        out.join("\n")
    }

    fn set_block(&mut self, block: usize, text: String, mut styles: Styles) {
        styles.fit(&text);
        match block.checked_sub(1) {
            None => {
                self.body = text;
                self.styles = styles;
            }
            Some(n) => {
                if let Some(q) = self.quote.as_mut() {
                    Arc::make_mut(q).edits.insert(n, (text, styles));
                }
            }
        }
    }

    /// Take out what's between `a` and `b` (either way round): the text selected at both ends,
    /// and every block of the original wholly between them, a table with all it holds; a cell of
    /// a table taken in part is only emptied. Two paragraphs outside tables left end to end
    /// become one, as in a single text. Where the cursor is left: the place they meet.
    pub fn delete_across(&mut self, a: QuotePoint, b: QuotePoint) -> Option<QuotePoint> {
        let (from, to) = (a.min(b), a.max(b));
        let (mut head, mut head_styles) = self.block_text(from.block)?;
        let from = QuotePoint { block: from.block, at: from.at.min(chars(&head)) };
        if from.block == to.block {
            rich::splice(&mut head, &mut head_styles, from.at..to.at.max(from.at), "", &Styles::plain(""));
            self.set_block(from.block, head, head_styles);
            return Some(from);
        }
        let (tail, tail_styles) = self.block_text(to.block)?;
        let tail_len = chars(&tail);
        let whole_tail = to.at >= tail_len;
        let (tail, tail_styles) = rich::slice(&tail, &tail_styles, to.at.min(tail_len)..tail_len);
        rich::splice(&mut head, &mut head_styles, from.at..usize::MAX, "", &Styles::plain(""));
        // The original's blocks of text wholly taken (their numbers there): those between the
        // ends, and the last one when it's taken to its end.
        let last = to.block.checked_sub(1)?;
        let taken = from.block..if whole_tail { last.saturating_add(1) } else { last };
        let first = from.block.checked_sub(1);
        let (mut removed, mut emptied, mut joined) = (Vec::new(), Vec::new(), false);
        {
            let q = self.quote.as_ref()?;
            let all = all_blocks(&q.blocks);
            let top = |n: usize| all.iter().position(|b| b.depth == 0 && matches!(b.block, Block::Text(_)) && b.texts.start == n);
            for (o, b) in all.iter().enumerate() {
                match b.block {
                    Block::Text(_) if taken.contains(&b.texts.start) => {
                        if b.depth == 0 {
                            removed.push(o);
                        } else {
                            emptied.push(b.texts.start);
                        }
                    }
                    Block::Text(_) => {}
                    // One with no text at all (a spacer, a banner): gone when it stands between
                    // the two ends.
                    Block::Table(_) if b.texts.is_empty() => {
                        if first.is_none_or(|f| b.texts.start > f) && b.texts.start <= last {
                            removed.push(o);
                        }
                    }
                    Block::Table(_) => {
                        if b.texts.start >= taken.start && b.texts.end <= taken.end {
                            removed.push(o);
                        }
                    }
                }
            }
            // Two paragraphs outside tables: what's left of the second goes on after the first.
            if !whole_tail
                && let (Some(f), Some(o)) = (first, top(last))
                && top(f).is_some()
            {
                rich::append(&mut head, &mut head_styles, &tail, &tail_styles);
                removed.push(o);
                joined = true;
            }
        }
        self.set_block(from.block, head, head_styles);
        if !whole_tail && !joined {
            self.set_block(to.block, tail, tail_styles);
        }
        if let Some(q) = self.quote.as_mut() {
            let q = Arc::make_mut(q);
            for n in emptied {
                q.edits.insert(n, (String::new(), Styles::plain("")));
            }
            q.removed.extend(removed);
        }
        Some(from)
    }
}
