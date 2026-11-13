// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The lines of pasted Markdown laid out as their marks say: what is before a mark on its line
//! (html2text's bullets, numbers and indents) taken away with it, the line's paragraph made a
//! list item or indented, a quote's text grey, code on its grey with a block's lines made as
//! wide as its widest, a table in columns ([`tables`]), an empty line before and after a list,
//! a quote, a code block and a table.

use super::*;

mod tables;

/// The colour of a quote's text.
const QUOTE_GREY: [u8; 3] = [0x55, 0x55, 0x55];

/// The widest a code block's lines are made, in characters (a wider line stays as it is): each
/// is filled with spaces up to the block's widest, for the block to read as one grey box.
const CODE_WIDTH_MAX: usize = 80;

/// The most spaces added to code blocks' lines in one paste (a block of many empty lines would
/// make many): past it, a block's lines are left as they are.
const CODE_FILL_MAX: usize = MARKDOWN_MAX;

/// What is added to a look ([`Looks::with`]).
const GREY: u8 = 1;
const SHADE: u8 = 2;
const MONO: u8 = 4;

/// The looks of the text laid out: html2text's, and those made from them, each made once.
struct Looks {
    palette: Vec<rich::Style>,
    made: HashMap<(u16, u8), u16>,
}

impl Looks {
    /// The look `id` with `what` added: a quote's grey ([`GREY`]), the code grey
    /// ([`SHADE`]), the monospace font ([`MONO`]). `id` itself when the palette is full.
    fn with(&mut self, id: u16, what: u8) -> u16 {
        if what == 0 {
            return id;
        }
        if let Some(made) = self.made.get(&(id, what)) {
            return *made;
        }
        let mut s = self.palette.get(usize::from(id)).cloned().unwrap_or_default();
        if what & GREY != 0 {
            s.color = s.color.or(Some(QUOTE_GREY));
        }
        if what & MONO != 0 {
            s.mono = true;
        }
        if what & SHADE != 0 {
            s.highlight = Some(rich::CODE_SHADE);
        }
        let made = match self.palette.iter().position(|p| *p == s) {
            Some(i) => u16::try_from(i).unwrap_or(id),
            None if self.palette.len() < usize::from(u16::MAX) => {
                self.palette.push(s);
                u16::try_from(self.palette.len() - 1).unwrap_or(id)
            }
            None => id,
        };
        self.made.insert((id, what), made);
        made
    }

    /// The look of the character at `at` in html2text's text (`st`), with `what` added, and
    /// code (in the monospace font: Markdown's own, nothing else is) on its grey.
    fn of(&mut self, st: &rich::Styles, at: usize, what: u8) -> u16 {
        let id = st.chars.get(at).copied().unwrap_or(0);
        let shade = if self.palette.get(usize::from(id)).is_some_and(|s| s.mono) { SHADE } else { 0 };
        self.with(id, what | shade)
    }
}

/// What a line is part of, for the room around lists, quotes, code blocks and tables.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Block {
    Text,
    List,
    Quote,
    Code,
    Table,
}

/// A line laid out: its paragraph, its characters and their looks, what it is part of, and
/// whether it starts a block of its own (`first`: a code block's or a table's first line). A
/// code block's line has the look its filling takes (`code`).
struct Line {
    para: rich::Para,
    chars: Vec<(char, u16)>,
    block: Block,
    first: bool,
    code: Option<u16>,
}

impl Line {
    /// An empty line between paragraphs.
    fn blank(&self) -> bool {
        self.block == Block::Text && self.chars.is_empty()
    }
}

/// The text and styles being written out.
struct Out {
    text: String,
    chars: Vec<u16>,
    paras: Vec<rich::Para>,
    /// The look of the last line's last character (its line break's).
    last: u16,
}

impl Out {
    fn line(&mut self, chars: &[(char, u16)], para: rich::Para) {
        if !self.paras.is_empty() {
            self.text.push('\n');
            self.chars.push(self.last);
        }
        for (c, id) in chars {
            self.text.push(*c);
            self.chars.push(*id);
        }
        self.last = chars.last().map_or(0, |(_, id)| *id);
        self.paras.push(para);
    }
}

/// The text and styles html2text's reading of the marked HTML gave, laid out as its line marks
/// say. Linear in the text's length.
pub(super) fn lay_out(text: &str, styles: &rich::Styles) -> (String, rich::Styles) {
    let mut st = styles.clone();
    st.fit(text);
    let mut looks = Looks { palette: std::mem::take(&mut st.palette), made: HashMap::new() };
    let mut lines: Vec<Line> = Vec::with_capacity(st.paras.len());
    // The rows of the table being read, laid out once it ends.
    let mut rows: Vec<tables::Row> = Vec::new();
    let mut next = 0usize;
    for (n, line) in text.split('\n').enumerate() {
        let line_chars: Vec<char> = line.chars().collect();
        let at = next;
        next += line_chars.len() + 1;
        let mut para = st.para(n);
        let mark = line_chars.iter().position(|c| (MARK_FIRST..=MARK_LAST).contains(&u32::from(*c)));
        let mut quote = false;
        if let Some(code) = mark.and_then(|p| line_chars.get(p)).map(|c| u32::from(*c) - MARK_FIRST) {
            para.list = match code / 16 {
                1 => rich::List::Bullet,
                2 => rich::List::Number,
                _ => rich::List::None,
            };
            quote = code % 16 >= 8;
            para.indent = u8::try_from(code % 8).unwrap_or(0);
        }
        // A code block's line: its text after its own mark (its indent is the code's).
        let code_mark = line_chars.iter().position(|c| *c == CODE_FIRST || *c == CODE_LINE);
        let grey = if quote { GREY } else { 0 };
        // A table's row: its cells from its first one's mark on.
        if let Some(p) = line_chars.iter().position(|c| tables::cell_mark(*c).is_some()) {
            let row = tables::row(&line_chars, p, at, para, quote, &st, &mut looks);
            if row.head && !rows.is_empty() {
                lines.extend(tables::lay_out(&std::mem::take(&mut rows), &mut looks));
            }
            rows.push(row);
            continue;
        }
        if !rows.is_empty() {
            lines.extend(tables::lay_out(&std::mem::take(&mut rows), &mut looks));
        }
        let code = code_mark.map(|p| looks.with(st.chars.get(at + p).copied().unwrap_or(0), grey | SHADE | MONO));
        let first = code_mark.and_then(|p| line_chars.get(p)) == Some(&CODE_FIRST);
        let skip = code_mark.or(mark).map_or(0, |p| p + 1);
        let mut chars = Vec::with_capacity(line_chars.len());
        for (k, c) in line_chars.iter().enumerate().skip(skip) {
            if !is_own_mark(*c) {
                chars.push((*c, looks.of(&st, at + k, grey)));
            }
        }
        let block = if quote {
            Block::Quote
        } else if code.is_some() {
            Block::Code
        } else if para.list != rich::List::None || para.indent > 0 {
            Block::List
        } else {
            Block::Text
        };
        lines.push(Line { para, chars, block, first, code });
    }
    lines.extend(tables::lay_out(&rows, &mut looks));
    fill_code_blocks(&mut lines);
    let mut out = Out { text: String::with_capacity(text.len()), chars: Vec::with_capacity(st.chars.len()), paras: Vec::new(), last: 0 };
    let mut before: Option<&Line> = None;
    for line in &lines {
        // Room between a block and what is around it, unless an empty line is there already.
        if let Some(b) = before
            && !b.blank()
            && !line.blank()
            && (b.block != line.block || line.first)
        {
            out.line(&[], rich::Para::default());
        }
        out.line(&line.chars, line.para);
        before = Some(line);
    }
    let mut laid = rich::Styles { chars: out.chars, palette: looks.palette, paras: out.paras };
    laid.fit(&out.text);
    (out.text, laid)
}

/// Each code block's lines filled with spaces in its look up to its widest
/// ([`CODE_WIDTH_MAX`] at most), within [`CODE_FILL_MAX`] spaces in all.
fn fill_code_blocks(lines: &mut [Line]) {
    let mut budget = CODE_FILL_MAX;
    let mut i = 0;
    while i < lines.len() {
        if lines.get(i).is_none_or(|l| l.code.is_none()) {
            i += 1;
            continue;
        }
        let mut end = i + 1;
        while lines.get(end).is_some_and(|l| l.code.is_some() && !l.first) {
            end += 1;
        }
        let block = lines.get_mut(i..end).unwrap_or_default();
        let width = block.iter().map(|l| l.chars.len()).max().unwrap_or(0).min(CODE_WIDTH_MAX);
        let fill: usize = block.iter().map(|l| width.saturating_sub(l.chars.len())).sum();
        if fill <= budget {
            budget -= fill;
            for line in block {
                let look = line.code.unwrap_or(0);
                while line.chars.len() < width {
                    line.chars.push((' ', look));
                }
            }
        }
        i = end;
    }
}
