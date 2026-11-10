// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The edits a note's window makes in its Markdown: a tick ticked or unticked, a mark put on a
//! selection or taken off it (the bar's Bold, Italic, Title, List, Tick and Link), and what
//! Enter writes in a list. Positions are bytes, held to the text and to its characters.

use std::ops::Range;

use super::{Block, Kind, Line, Prefix, inline, prefix, read};

/// A text edited: all of it, and what is selected in it after (bytes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub text: String,
    pub select: Range<usize>,
}

/// What the note's bar writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// `**words**`.
    Bold,
    /// `*words*`.
    Italic,
    /// `# ` at the start of each line.
    Heading,
    /// `- ` at the start of each line.
    List,
    /// `- [ ] ` at the start of each line.
    Tick,
    /// `[words](https://)`, the address selected to be typed.
    Link,
}

/// `i` held to the text, at the start of a character.
fn floor(text: &str, i: usize) -> usize {
    let mut i = i.min(text.len());
    while i > 0 && !text.is_char_boundary(i) {
        i = i.saturating_sub(1);
    }
    i
}

/// The line byte `at` is on (its end included: a cursor at the end of a line is on it).
fn line_at(lines: &[Line], at: usize) -> Option<&Line> {
    lines.iter().find(|l| l.range.start <= at && at <= l.range.end)
}

/// `text` with `range` replaced by `with`.
fn splice(text: &str, range: Range<usize>, with: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len().saturating_add(with.len()));
    out.push_str(text.get(..range.start)?);
    out.push_str(with);
    out.push_str(text.get(range.end..)?);
    Some(out)
}

/// Tick or untick the tick on the line at byte `at`: only the character in its box changes
/// (`[ ]` and `[x]`). False, and nothing changed, on a line that isn't a tick.
pub fn toggle_tick(text: &mut String, at: usize) -> bool {
    let lines = read(text);
    let Some(line) = line_at(&lines, floor(text, at)) else { return false };
    let Block::Tick { done, .. } = line.block else { return false };
    let Some(inner) = line.spans.iter().find(|s| s.kind == Kind::Tick).map(|s| s.range.start.saturating_add(1)) else { return false };
    let inner = inner..inner.saturating_add(1);
    if !text.get(inner.clone()).is_some_and(|c| matches!(c, " " | "x" | "X")) {
        return false;
    }
    text.replace_range(inner, if done { " " } else { "x" });
    true
}

/// What Enter writes at byte `at`. In a list item or a tick: a new one after it (the next
/// number, an empty box), the words after `at` going with it; in an empty one, the list ends
/// there (its marks go, the line left empty). `None`: an ordinary line break.
pub fn enter(text: &str, at: usize) -> Option<Edit> {
    let at = floor(text, at);
    let lines = read(text);
    let line = line_at(&lines, at)?;
    if !matches!(line.block, Block::Bullet { .. } | Block::Numbered { .. } | Block::Tick { .. }) {
        return None;
    }
    let words = text.get(line.range.clone()).filter(|w| !w.contains('\n'))?;
    let p = prefix(words);
    let start = line.range.start;
    if at < start.saturating_add(p.marks) {
        return None;
    }
    if words.get(p.content(words)..).is_some_and(|w| w.trim().is_empty()) {
        let out = splice(text, line.range.clone(), "")?;
        return Some(Edit { text: out, select: start..start });
    }
    let marker = list_marker(words, &p, 1)?;
    let tick = if p.tick.is_some() { "[ ] " } else { "" };
    let insert = format!("\n{}{marker}{tick}", words.get(..p.indent)?);
    let out = splice(text, at..at, &insert)?;
    let cursor = at.saturating_add(insert.len());
    Some(Edit { text: out, select: cursor..cursor })
}

/// A list item's marker (`- `, `1. `), its number moved on by `step`.
fn list_marker(words: &str, p: &Prefix, step: u32) -> Option<String> {
    let marker = words.get(p.indent..p.marks)?;
    let digits = marker.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return Some(marker.to_string());
    }
    let n = marker.get(..digits)?.parse::<u32>().ok()?.saturating_add(step);
    Some(format!("{n}{}", marker.get(digits..)?))
}

/// Put a mark on the selection `select` (bytes, either way round), or take it off where it is:
/// Bold and Italic around the words selected (each line's, over several lines), Title, List and
/// Tick at the start of each line selected, Link around the words selected. With nothing
/// selected, the marks go where the cursor is, and it goes between them.
pub fn toggle_mark(text: &str, select: Range<usize>, mark: Mark) -> Edit {
    let (x, y) = (floor(text, select.start), floor(text, select.end));
    let (a, b) = (x.min(y), x.max(y));
    let edit = match mark {
        Mark::Bold => around(text, a, b, "**"),
        Mark::Italic => around(text, a, b, "*"),
        Mark::Link => link(text, a, b),
        Mark::Heading | Mark::List | Mark::Tick => lines(text, a, b, mark),
    };
    edit.unwrap_or_else(|| Edit { text: text.to_string(), select: a..b })
}

/// Bold or italic (`m`) put on the words `a..b`, or taken off them.
fn around(text: &str, a: usize, b: usize, m: &str) -> Option<Edit> {
    let sel = text.get(a..b)?;
    let len = m.len();
    if sel.contains('\n') {
        // Each line's part: marks don't run across lines.
        let mut out = String::with_capacity(text.len().saturating_add(64));
        out.push_str(text.get(..a)?);
        for (i, part) in sel.split('\n').enumerate() {
            if i > 0 {
                out.push('\n');
            }
            let t = part.trim();
            let lead = part.len().saturating_sub(part.trim_start().len());
            if t.is_empty() {
                out.push_str(part);
                continue;
            }
            out.push_str(part.get(..lead)?);
            for piece in [m, t, m] {
                out.push_str(piece);
            }
            out.push_str(part.get(lead.saturating_add(t.len())..)?);
        }
        let end = out.len();
        out.push_str(text.get(b..)?);
        return Some(Edit { text: out, select: a..end });
    }
    let t = sel.trim();
    if t.is_empty() {
        let out = splice(text, b..b, &format!("{m}{m}"))?;
        let cursor = b.saturating_add(len);
        return Some(Edit { text: out, select: cursor..cursor });
    }
    let a = a.saturating_add(sel.len().saturating_sub(sel.trim_start().len()));
    let b = a.saturating_add(t.len());
    // A run of stars says bold (two or more) or italic (an odd number) around it.
    let has = |run: usize| if len == 2 { run >= 2 } else { run % 2 == 1 };
    let stars = |s: &str| s.bytes().take_while(|c| *c == b'*').count();
    let before = text.get(..a)?.bytes().rev().take_while(|c| *c == b'*').count();
    let after = stars(text.get(b..)?);
    if has(before) && has(after) {
        let mut out = String::with_capacity(text.len());
        out.push_str(text.get(..a.saturating_sub(len))?);
        out.push_str(t);
        out.push_str(text.get(b.saturating_add(len)..)?);
        let from = a.saturating_sub(len);
        return Some(Edit { text: out, select: from..from.saturating_add(t.len()) });
    }
    let (inner_lead, inner_trail) = (stars(t), t.bytes().rev().take_while(|c| *c == b'*').count());
    if inner_lead < t.len() && t.len() >= len.saturating_mul(2) && has(inner_lead) && has(inner_trail) {
        let inner = t.get(len..t.len().saturating_sub(len))?;
        let out = splice(text, a..b, inner)?;
        return Some(Edit { text: out, select: a..a.saturating_add(inner.len()) });
    }
    let out = splice(text, a..b, &format!("{m}{t}{m}"))?;
    Some(Edit { text: out, select: a.saturating_add(len)..b.saturating_add(len) })
}

/// A link put around the words `a..b` (on one line), or taken off the link they're in.
fn link(text: &str, a: usize, b: usize) -> Option<Edit> {
    let lines = read(text);
    let line = line_at(&lines, a)?;
    let words = text.get(line.range.clone())?;
    let start = line.range.start;
    if !matches!(line.block, Block::Code | Block::Fence) {
        for (whole, inner) in inline::links(words, prefix(words).words()) {
            let (from, to) = (start.saturating_add(whole.start), start.saturating_add(whole.end));
            if from <= a && b <= to {
                let t = words.get(inner)?;
                let out = splice(text, from..to, t)?;
                return Some(Edit { text: out, select: from..from.saturating_add(t.len()) });
            }
        }
    }
    let b = b.min(line.range.end);
    let sel = text.get(a..b)?;
    let out = splice(text, a..b, &format!("[{sel}](https://)"))?;
    if sel.is_empty() {
        let cursor = a.saturating_add(1);
        return Some(Edit { text: out, select: cursor..cursor });
    }
    let address = a.saturating_add(sel.len()).saturating_add(3);
    Some(Edit { text: out, select: address..address.saturating_add("https://".len()) })
}

/// Title, List or Tick put at the start of each line `a..b` touches, or taken off them all
/// when they all have it. Code, fences and lines are left as they are.
fn lines(text: &str, a: usize, b: usize, mark: Mark) -> Option<Edit> {
    let all = read(text);
    let has = |l: &Line| match mark {
        Mark::Heading => matches!(l.block, Block::Heading(_)),
        Mark::List => matches!(l.block, Block::Bullet { .. }),
        _ => matches!(l.block, Block::Tick { .. }),
    };
    let touched: Vec<&Line> = all
        .iter()
        .filter(|l| l.range.start <= b && a <= l.range.end)
        .filter(|l| !matches!(l.block, Block::Code | Block::Fence | Block::Rule))
        .filter(|l| text.get(l.range.clone()).is_some_and(|w| !w.contains('\n')))
        .collect();
    if touched.is_empty() {
        return None;
    }
    let off = touched.iter().all(|l| has(l));
    let mut out = String::with_capacity(text.len().saturating_add(touched.len().saturating_mul(8)));
    let (mut na, mut nb) = (None, None);
    // Where a selection's end goes: kept with the text, or after the new marks when it was in
    // the old ones.
    let place = |slot: &mut Option<usize>, pos: usize, below: usize, at: usize| {
        if slot.is_none() && pos < below {
            *slot = Some(at);
        }
    };
    let mut done = 0;
    for l in touched.into_iter().filter(|l| off || !has(l)) {
        let s = l.range.start;
        let words = text.get(l.range.clone())?;
        let p = prefix(words);
        let (old, new) = line_marks(words, &p, l.block, mark, off)?;
        let from = out.len();
        out.push_str(text.get(done..s)?);
        place(&mut na, a, s, from.saturating_add(a.saturating_sub(done)));
        place(&mut nb, b, s, from.saturating_add(b.saturating_sub(done)));
        out.push_str(&new);
        place(&mut na, a, s.saturating_add(old), out.len());
        place(&mut nb, b, s.saturating_add(old), out.len());
        done = s.saturating_add(old);
    }
    let from = out.len();
    out.push_str(text.get(done..)?);
    let na = na.unwrap_or(from.saturating_add(a.saturating_sub(done)));
    let nb = nb.unwrap_or(from.saturating_add(b.saturating_sub(done)));
    Some(Edit { text: out, select: na..nb })
}

/// A line's marks to replace (how many bytes from its start) and what replaces them.
fn line_marks(words: &str, p: &Prefix, block: Block, mark: Mark, off: bool) -> Option<(usize, String)> {
    let list = matches!(block, Block::Plain | Block::Bullet { .. } | Block::Numbered { .. } | Block::Tick { .. });
    let old = match block {
        Block::Plain => p.indent,
        Block::Tick { .. } => p.content(words),
        _ => p.marks,
    };
    let indent = if list { words.get(..p.indent)? } else { "" };
    let new = match (mark, off) {
        (Mark::Heading, true) => String::new(),
        (_, true) => indent.to_string(),
        (Mark::Heading, false) => "# ".to_string(),
        (Mark::List, false) => format!("{indent}- "),
        (_, false) => {
            let marker = if matches!(block, Block::Bullet { .. } | Block::Numbered { .. }) { list_marker(words, p, 0)? } else { "- ".to_string() };
            format!("{indent}{marker}[ ] ")
        }
    };
    Some((old, new))
}
