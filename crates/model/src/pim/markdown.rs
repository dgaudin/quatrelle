// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A note's Markdown, read as it is typed: for each line, what it is (a title, a list item, a
//! tick, a quote, code...) and its segments (their kind and style, and their bytes), the marks
//! segments of their own so the note's window can draw them pale. A small subset, in
//! Quatrelle's own code rather than CommonMark: what isn't recognised stays text, as written.
//! Reading never changes the text; [`toggle_tick`], [`toggle_mark`] and [`enter`] make the edits
//! the window offers. The text is hostile (a note from a server or a data file): it is read
//! within caps, every range falls on character boundaries, and nothing here panics.

use std::ops::Range;

mod edit;
mod inline;
#[cfg(test)]
mod tests;

pub use edit::{Edit, Mark, enter, toggle_mark, toggle_tick};

/// Most of a text read as Markdown (a note is at most 256 KiB): what's past it is one plain line.
pub const MAX_READ: usize = 256 * 1024;
/// Most lines read as Markdown; past them, the rest of the text is one plain line.
pub const MAX_LINES: usize = 10_000;
/// Most segments in one line; past them, the rest of the line is one segment of text.
pub const MAX_SPANS: usize = 256;
/// Most list levels: indentation past the last is the last.
pub const MAX_DEPTH: u8 = 8;

/// What a line is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Block {
    Plain,
    /// `# `, `## ` or `### `: its level, 1 to 3.
    Heading(u8),
    /// `- ` (or `* `, `+ `), at a depth from 0 (its indentation, two spaces a level).
    Bullet {
        depth: u8,
    },
    /// `1. ` (or `1) `).
    Numbered {
        depth: u8,
        number: u32,
    },
    /// `- [ ] ` or `- [x] ` (or a numbered one).
    Tick {
        depth: u8,
        done: bool,
    },
    /// `> `.
    Quote,
    /// `---`.
    Rule,
    /// A fence (three backticks or tildes) opening or closing a block of code.
    Fence,
    /// A line inside fenced code, as written.
    Code,
}

/// What a segment of a line is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Words.
    Text,
    /// A mark (`# `, `**`, `- `, a backtick, `[`, `](`, `)`...), drawn pale.
    Mark,
    /// Code, in backticks or in a fenced block.
    Code,
    /// A tick's box, `[ ]` or `[x]`: a click ticks or unticks it ([`toggle_tick`]).
    Tick,
    /// A link's text (`[text](…)`): its address is the line's next [`Kind::Url`] ([`link_at`]).
    LinkText,
    /// An address: a link's, or a bare one (`https://…`).
    Url,
}

/// How a segment's words are set (several at once: `***bold and italic***`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
}

/// A segment: its kind and style, and its bytes in the text read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub kind: Kind,
    pub style: Style,
    pub range: Range<usize>,
}

/// A line: what it is, its bytes in the text (without its line break) and its segments, in
/// order, which cover it byte for byte (an empty line has none). Past [`MAX_READ`] or
/// [`MAX_LINES`], the rest of the text is one last plain line, line breaks and all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub block: Block,
    pub range: Range<usize>,
    pub spans: Vec<Span>,
}

/// A text read as Markdown, line by line (as many lines as `text.split('\n')` gives, the last
/// past the caps aside).
pub fn read(text: &str) -> Vec<Line> {
    let mut out = Vec::new();
    // The fence a block of code was opened with (its character and length), until closed.
    let mut fence: Option<(u8, usize)> = None;
    let mut start = 0;
    loop {
        if out.len() >= MAX_LINES || start >= MAX_READ {
            let range = start..text.len();
            let spans = if range.is_empty() { Vec::new() } else { vec![span(Kind::Text, Style::default(), range.clone())] };
            out.push(Line { block: Block::Plain, range, spans });
            break;
        }
        let rest = text.get(start..).unwrap_or("");
        let end = rest.find('\n').map_or(text.len(), |i| start.saturating_add(i));
        // A line break may be `\r\n` (a `\r` at the very end is the line's own).
        let crlf = end < text.len() && end > start && text.as_bytes().get(end.saturating_sub(1)) == Some(&b'\r');
        let stop = if crlf { end.saturating_sub(1) } else { end };
        let line = text.get(start..stop).unwrap_or("");
        out.push(read_line(line, start, &mut fence));
        if end >= text.len() {
            break;
        }
        start = end.saturating_add(1);
    }
    out
}

/// The address a click at byte `at` follows: on a bare address, that one; on a link's text or
/// its address, the link's. `None` elsewhere. Where it leads is checked before it opens.
pub fn link_at(text: &str, at: usize) -> Option<&str> {
    let lines = read(text);
    let line = lines.iter().find(|l| l.range.start <= at && at < l.range.end)?;
    let i = line.spans.iter().position(|s| s.range.contains(&at))?;
    let url = match line.spans.get(i)?.kind {
        Kind::Url => line.spans.get(i)?,
        Kind::LinkText => line.spans.iter().skip(i).find(|s| s.kind == Kind::Url)?,
        _ => return None,
    };
    text.get(url.range.clone())
}

fn span(kind: Kind, style: Style, range: Range<usize>) -> Span {
    Span { kind, style, range }
}

/// One line (`line`, at byte `base` of the text), `fence` the block of code it's in, if any.
fn read_line(line: &str, base: usize, fence: &mut Option<(u8, usize)>) -> Line {
    let at = |r: Range<usize>| r.start.saturating_add(base)..r.end.saturating_add(base);
    let range = at(0..line.len());
    let whole = |kind| if line.is_empty() { Vec::new() } else { vec![span(kind, Style::default(), range.clone())] };
    if let Some((ch, len)) = *fence {
        if fence_of(line).is_some_and(|(c, n, info)| c == ch && n >= len && info.trim().is_empty()) {
            *fence = None;
            return Line { block: Block::Fence, spans: whole(Kind::Mark), range };
        }
        return Line { block: Block::Code, spans: whole(Kind::Code), range };
    }
    if let Some((ch, n, info)) = fence_of(line)
        && !(ch == b'`' && info.contains('`'))
    {
        *fence = Some((ch, n));
        return Line { block: Block::Fence, spans: whole(Kind::Mark), range };
    }
    let p = prefix(line);
    if p.block == Block::Rule {
        return Line { block: Block::Rule, spans: whole(Kind::Mark), range };
    }
    let mut spans = Vec::new();
    if p.marks > 0 {
        spans.push(span(Kind::Mark, Style::default(), at(0..p.marks)));
    }
    if let Some(b) = p.tick {
        spans.push(span(Kind::Tick, Style::default(), at(b..b.saturating_add(3))));
    }
    inline::spans(line, p.words(), base, &mut spans);
    Line { block: p.block, range, spans }
}

/// A fence: up to three spaces, then three or more backticks or tildes, then what follows
/// (its "info", a language's name).
fn fence_of(line: &str) -> Option<(u8, usize, &str)> {
    let lead = line.bytes().take_while(|b| *b == b' ').count();
    if lead > 3 {
        return None;
    }
    let rest = line.get(lead..)?;
    let ch = *rest.as_bytes().first().filter(|c| **c == b'`' || **c == b'~')?;
    let n = rest.bytes().take_while(|b| *b == ch).count();
    (n >= 3).then(|| (ch, n, rest.get(n..).unwrap_or("")))
}

/// A line's own marks (outside fenced code): what it is, its indentation (bytes), where its
/// marks end and, for a tick, where its box starts (three bytes: `[ ]` or `[x]`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Prefix {
    pub block: Block,
    pub indent: usize,
    pub marks: usize,
    pub tick: Option<usize>,
}

impl Prefix {
    /// Where the line's words are read from: after its marks and its tick's box.
    pub fn words(&self) -> usize {
        self.tick.map_or(self.marks, |b| b.saturating_add(3))
    }

    /// Where what the line says starts: after its marks, and its tick's box and the space after it.
    pub fn content(&self, line: &str) -> usize {
        match self.tick {
            Some(b) => {
                let after = b.saturating_add(3);
                if line.as_bytes().get(after) == Some(&b' ') { after.saturating_add(1) } else { after }
            }
            None => self.marks,
        }
    }
}

/// What a line is by its first characters (fences aside: see [`read`]).
pub(super) fn prefix(line: &str) -> Prefix {
    let b = line.as_bytes();
    let plain = |indent| Prefix { block: Block::Plain, indent, marks: 0, tick: None };
    let spaces = |from: usize| b.get(from..).map_or(0, |r| r.iter().take_while(|c| **c == b' ' || **c == b'\t').count());
    // A title: one to three `#`, then a space (or nothing).
    let hashes = b.iter().take_while(|c| **c == b'#').count();
    if (1..=3).contains(&hashes) && matches!(b.get(hashes), None | Some(b' ' | b'\t')) {
        let level = u8::try_from(hashes).unwrap_or(1);
        return Prefix { block: Block::Heading(level), indent: 0, marks: hashes.saturating_add(spaces(hashes)), tick: None };
    }
    let lead = spaces(0);
    let rest = b.get(lead..).unwrap_or(&[]);
    // A line: three dashes or more, and nothing else.
    let trimmed = line.trim();
    if lead <= 3 && trimmed.len() >= 3 && trimmed.bytes().all(|c| c == b'-') {
        return Prefix { block: Block::Rule, indent: lead, marks: line.len(), tick: None };
    }
    if lead <= 3 && rest.first() == Some(&b'>') {
        let after = lead.saturating_add(1);
        let marks = if b.get(after) == Some(&b' ') { after.saturating_add(1) } else { after };
        return Prefix { block: Block::Quote, indent: lead, marks, tick: None };
    }
    let depth = depth_of(b.get(..lead).unwrap_or(&[]));
    // A list's marker, and the space after it.
    let marker = match rest.first() {
        Some(b'-' | b'*' | b'+') => Some((1, None)),
        Some(c) if c.is_ascii_digit() => {
            let digits = rest.iter().take_while(|c| c.is_ascii_digit()).count();
            let number = if digits <= 9 { line.get(lead..lead.saturating_add(digits)).and_then(|d| d.parse::<u32>().ok()) } else { None };
            match (number, rest.get(digits)) {
                (Some(n), Some(b'.' | b')')) => Some((digits.saturating_add(1), Some(n))),
                _ => None,
            }
        }
        _ => None,
    };
    let Some((len, number)) = marker else { return plain(lead) };
    let after = lead.saturating_add(len);
    if !matches!(b.get(after), Some(b' ' | b'\t')) {
        return plain(lead);
    }
    let marks = after.saturating_add(spaces(after));
    let tick = match b.get(marks..marks.saturating_add(3)) {
        Some([b'[', c @ (b' ' | b'x' | b'X'), b']']) if matches!(b.get(marks.saturating_add(3)), None | Some(b' ' | b'\t')) => Some(*c != b' '),
        _ => None,
    };
    let block = match (tick, number) {
        (Some(done), _) => Block::Tick { depth, done },
        (None, Some(number)) => Block::Numbered { depth, number },
        (None, None) => Block::Bullet { depth },
    };
    Prefix { block, indent: lead, marks, tick: tick.map(|_| marks) }
}

/// A list item's depth by its indentation: two spaces a level, a tab two levels.
fn depth_of(indent: &[u8]) -> u8 {
    let columns: usize = indent.iter().map(|c| if *c == b'\t' { 4 } else { 1 }).sum();
    u8::try_from(columns / 2).unwrap_or(u8::MAX).min(MAX_DEPTH.saturating_sub(1))
}
