// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! A laid-out message as its blocks of formatted text, in reading order: the original under a
//! reply or forward, edited a block at a time where it stands, or quoted as one column of text
//! (its tables' cells one after another, each lined up as it was, with its pictures, links and
//! colours) rather than its tables drawn in lines of `│` and `─`.

use super::*;
use crate::rich::{self, Styles};

/// `blocks` as editable text and styles: each block of text on lines of its own, the empty ones
/// (a spacer cell) left out.
pub fn flatten(blocks: &[Block]) -> (String, Styles) {
    let mut text = String::new();
    let mut styles = Styles::plain("");
    for (lines, align) in text_blocks(blocks) {
        let (t, st) = cell_text(lines, align);
        if t.trim().is_empty() && !t.contains(rich::PICTURE) {
            continue;
        }
        if !text.is_empty() {
            rich::append(&mut text, &mut styles, "\n", &Styles::plain("\n"));
        }
        rich::append(&mut text, &mut styles, &t, &st);
    }
    (text, styles)
}

/// The blocks of formatted text of `blocks`, in order: each table's cells row by row, a cell's
/// own text then the boxes placed over it, each with how it lines up (a cell that doesn't say
/// lines up as what it's in).
pub fn text_blocks(blocks: &[Block]) -> Vec<(&RichText, Align)> {
    fn walk<'a>(blocks: &'a [Block], align: Align, out: &mut Vec<(&'a RichText, Align)>, depth: usize) {
        if depth > MAX_DEPTH * 2 {
            return;
        }
        for b in blocks {
            match b {
                Block::Text(lines) => out.push((lines, align)),
                Block::Table(t) => {
                    for cell in t.rows.iter().flatten() {
                        let a = if cell.align == Align::Left { align } else { cell.align };
                        walk(&cell.blocks, a, out, depth + 1);
                        for o in &cell.overlays {
                            walk(&o.blocks, a, out, depth + 1);
                        }
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(blocks, Align::Left, &mut out, 0);
    out
}

/// A block of formatted lines as text and styles to edit, its paragraphs lined up as `align`.
pub fn cell_text(lines: &RichText, align: Align) -> (String, Styles) {
    let (t, mut st) = rich::from_rich(lines);
    let a = match align {
        Align::Left => rich::Align::Left,
        Align::Center => rich::Align::Center,
        Align::Right => rich::Align::Right,
    };
    if a != rich::Align::Left {
        for p in &mut st.paras {
            p.align = a;
        }
    }
    (t, st)
}

/// Every formatted run in `blocks`, its cells' and the boxes over them, for `f` to change.
pub(crate) fn each_span(blocks: &mut [Block], f: &mut dyn FnMut(&mut crate::Span)) {
    fn go(blocks: &mut [Block], f: &mut dyn FnMut(&mut crate::Span), depth: usize) {
        if depth > MAX_DEPTH * 2 {
            return;
        }
        for b in blocks {
            match b {
                Block::Text(lines) => lines.iter_mut().flatten().for_each(&mut *f),
                Block::Table(t) => {
                    for cell in t.rows.iter_mut().flatten() {
                        go(&mut cell.blocks, f, depth + 1);
                        for o in &mut cell.overlays {
                            go(&mut o.blocks, f, depth + 1);
                        }
                    }
                }
            }
        }
    }
    go(blocks, f, 0);
}
