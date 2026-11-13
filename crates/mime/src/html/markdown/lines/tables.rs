// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A table of pasted Markdown laid out in columns, in the monospace font (a message's text holds
//! no table, and is sent as it shows): each column as wide as its widest cell and aligned as the
//! Markdown says, " │ " between them, a rule under the first row. A table wider than
//! [`TABLE_WIDTH_MAX`] is one line a row instead, its cells between " · ".
//!
//! Widths are counted in characters: a character drawn wider (Chinese, Japanese, Korean) or
//! none at all (a combining accent) puts its row's next columns out of line.

use super::*;

/// The widest a table is laid out in columns, in characters (about a message's line).
const TABLE_WIDTH_MAX: usize = 120;

/// What goes between two columns, and under it in the rule under the first row.
const BETWEEN: [char; 3] = [' ', '│', ' '];
const BETWEEN_RULE: [char; 3] = ['─', '┼', '─'];

/// Whether `c` is the mark a cell starts with, and if so whether the cell is in the table's
/// first row and how its column is aligned.
pub(super) fn cell_mark(c: char) -> Option<(bool, u8)> {
    let code = u32::from(c);
    (CELL_FIRST..=CELL_LAST).contains(&code).then(|| {
        let k = code - CELL_FIRST;
        (k & 4 != 0, u8::try_from(k & 3).unwrap_or(0))
    })
}

/// A cell: its characters and their looks (white space at its ends left out), and how its
/// column is aligned (0 as written, 1 left, 2 centred, 3 right).
struct Cell {
    chars: Vec<(char, u16)>,
    align: u8,
}

/// A table's row: its paragraph, whether it is in a quote, its cells, whether it is the table's
/// first, and the look of what goes between its cells.
pub(super) struct Row {
    para: rich::Para,
    quote: bool,
    cells: Vec<Cell>,
    pub(super) head: bool,
    between: u16,
}

/// The row a line of html2text's text is (`line`, at `at` in the text, its looks in `st`), its
/// first cell's mark at `from`.
pub(super) fn row(line: &[char], from: usize, at: usize, para: rich::Para, quote: bool, st: &rich::Styles, looks: &mut Looks) -> Row {
    let grey = if quote { GREY } else { 0 };
    let mut cells: Vec<Cell> = Vec::new();
    let mut head = false;
    for (k, c) in line.iter().enumerate().skip(from) {
        if let Some((first, align)) = cell_mark(*c) {
            head |= first;
            cells.push(Cell { chars: Vec::new(), align });
        } else if !is_own_mark(*c)
            && let Some(cell) = cells.last_mut()
        {
            cell.chars.push((*c, looks.of(st, at + k, grey)));
        }
    }
    for cell in &mut cells {
        let end = cell.chars.iter().rposition(|(c, _)| !c.is_whitespace()).map_or(0, |e| e + 1);
        cell.chars.truncate(end);
        let start = cell.chars.iter().position(|(c, _)| !c.is_whitespace()).unwrap_or(cell.chars.len());
        cell.chars.drain(..start);
    }
    let between = looks.with(st.chars.get(at + from).copied().unwrap_or(0), grey);
    Row { para, quote, cells, head, between }
}

/// The characters of a row's cell `i` (none when the row has fewer cells).
fn cell(row: &Row, i: usize) -> &[(char, u16)] {
    row.cells.get(i).map_or(&[][..], |c| c.chars.as_slice())
}

/// A table's rows as lines: in columns, with the rule under the first row; or one line a row
/// past [`TABLE_WIDTH_MAX`].
pub(super) fn lay_out(rows: &[Row], looks: &mut Looks) -> Vec<Line> {
    let columns = rows.iter().map(|r| r.cells.len()).max().unwrap_or(0);
    let widths: Vec<usize> = (0..columns).map(|i| rows.iter().map(|r| cell(r, i).len()).max().unwrap_or(0)).collect();
    let aligns: Vec<u8> = (0..columns).map(|i| rows.iter().find_map(|r| r.cells.get(i)).map_or(0, |c| c.align)).collect();
    let wide = widths.iter().sum::<usize>().saturating_add(BETWEEN.len() * columns.saturating_sub(1)) > TABLE_WIDTH_MAX;
    let mut lines = Vec::with_capacity(rows.len() + 1);
    for (n, row) in rows.iter().enumerate() {
        let block = if row.quote { Block::Quote } else { Block::Table };
        let mut chars = Vec::new();
        if wide {
            for i in 0..columns {
                if i > 0 {
                    chars.extend(" · ".chars().map(|c| (c, row.between)));
                }
                chars.extend_from_slice(cell(row, i));
            }
            let end = chars.iter().rposition(|(c, _)| !c.is_whitespace()).map_or(0, |e| e + 1);
            chars.truncate(end);
            let start = chars.iter().position(|(c, _)| !c.is_whitespace()).unwrap_or(chars.len());
            chars.drain(..start);
        } else {
            let pad = looks.with(row.between, MONO);
            for (i, (width, align)) in widths.iter().zip(&aligns).enumerate() {
                if i > 0 {
                    chars.extend(BETWEEN.map(|c| (c, pad)));
                }
                let text = cell(row, i);
                let extra = width.saturating_sub(text.len());
                let left = match align {
                    2 => extra / 2,
                    3 => extra,
                    _ => 0,
                };
                chars.extend(std::iter::repeat_n((' ', pad), left));
                for (c, id) in text {
                    chars.push((*c, looks.with(*id, MONO)));
                }
                chars.extend(std::iter::repeat_n((' ', pad), extra - left));
            }
            while chars.last().is_some_and(|(c, _)| *c == ' ') {
                chars.pop();
            }
        }
        lines.push(Line { para: row.para, chars, block, first: n == 0, code: None });
        if n == 0 && row.head && !wide {
            let rule = looks.with(row.between, MONO);
            let mut chars = Vec::new();
            for (i, width) in widths.iter().enumerate() {
                if i > 0 {
                    chars.extend(BETWEEN_RULE.map(|c| (c, rule)));
                }
                chars.extend(std::iter::repeat_n(('─', rule), *width));
            }
            lines.push(Line { para: row.para, chars, block, first: false, code: None });
        }
    }
    lines
}
