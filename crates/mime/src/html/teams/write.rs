// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A message typed in a chat's box written as the HTML sent to Teams: the text a paragraph a line,
//! its bold and italic runs, escaped. The writer writes only its own tags (`p`, `b`, `i`, `u`,
//! `s`, `code`, `br`, and `a` to a web address): never a picture, a script, a style or any other
//! attribute, whatever the text says. What it writes is capped at [`SEND_MAX`] bytes.

use std::ops::Range;

use quatrelle_model::pim::teams::{Block, BlockKind, ChatText, Run, SEND_MAX};

use super::{TEAMS_RUNS, web_address};

/// A message longer than Teams takes: its HTML passes [`SEND_MAX`] bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooLong {
    /// The bytes written when the writing stopped, more than [`SEND_MAX`].
    pub bytes: usize,
}

/// What the user typed in a chat's box (`text`, with the byte ranges of it in `bold` and in
/// `italic`) as a message's text: a paragraph a line, its runs split where bold or italic starts
/// or stops. A range off the text, or off a character's boundary, is ignored; control characters
/// are left out but line breaks; blank lines at the start and at the end are left out, one between
/// two others is kept as an empty paragraph.
///
/// At most [`TEAMS_RUNS`] runs and empty paragraphs are kept. Past them the rest is left out, and
/// what's kept is too long to send already (each one writes more than 4 bytes on average), unless
/// what's left is only blank lines at the end.
pub fn teams_compose(text: &str, bold: &[Range<usize>], italic: &[Range<usize>]) -> ChatText {
    let (mut bold, mut italic) = (Spans::of(text, bold), Spans::of(text, italic));
    let mut lines = Lines::default();
    for (i, c) in text.char_indices() {
        let room = match c {
            '\n' => lines.end(),
            _ if c.is_control() => true,
            _ => lines.put(c, bold.covers(i), italic.covers(i)),
        };
        if !room {
            if text.get(i..).is_some_and(|rest| rest.chars().any(|c| !c.is_whitespace() && !c.is_control())) {
                // More words: left as they are, too long to send.
                let Lines { mut blocks, line, .. } = lines;
                blocks.push(Block { kind: BlockKind::Paragraph, runs: line });
                return ChatText { blocks };
            }
            break;
        }
    }
    lines.end();
    while lines.blocks.last().is_some_and(|b| b.runs.is_empty()) {
        lines.blocks.pop();
    }
    ChatText { blocks: lines.blocks }
}

/// A message's text as the HTML sent to Teams: each block as a paragraph (`<p>…</p>`, an empty one
/// `<p>&nbsp;</p>`); a run's styles as `<b>`, `<i>`, `<u>`, `<s>` and `<code>`, its link as
/// `<a href="…">` when it's to the web (`http`, `https`; else the text alone), its text escaped
/// ([`html_escape`]), a line break in it as `<br>`, other control characters but tabs left out.
/// A mention, a picture or an attachment is written as its text, in its run's styles.
///
/// `Err` once past [`SEND_MAX`] bytes, where the writing stops.
pub fn teams_write(t: &ChatText) -> Result<String, TooLong> {
    let mut w = Writer::default();
    for block in &t.blocks {
        w.push("<p>")?;
        let start = w.out.len();
        for run in &block.runs {
            w.run(run)?;
        }
        if w.out.len() == start {
            w.push("&nbsp;")?;
        }
        w.push("</p>")?;
    }
    Ok(w.out)
}

/// `s` as HTML text or as an attribute's value: `&`, `<`, `>`, `"` and `'` written as entities.
pub fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match entity(c) {
            Some(e) => out.push_str(e),
            None => out.push(c),
        }
    }
    out
}

/// The entity `c` is written as in HTML, when it isn't written as itself.
fn entity(c: char) -> Option<&'static str> {
    match c {
        '&' => Some("&amp;"),
        '<' => Some("&lt;"),
        '>' => Some("&gt;"),
        '"' => Some("&quot;"),
        '\'' => Some("&#39;"),
        _ => None,
    }
}

/// Byte ranges of a text, sorted and merged, asked about in the text's order.
struct Spans {
    ranges: Vec<Range<usize>>,
    at: usize,
}

impl Spans {
    /// Those of `ranges` within `text` and on its characters' boundaries.
    fn of(text: &str, ranges: &[Range<usize>]) -> Spans {
        let mut kept: Vec<Range<usize>> = ranges
            .iter()
            .filter(|r| r.start < r.end && r.end <= text.len() && text.is_char_boundary(r.start) && text.is_char_boundary(r.end))
            .cloned()
            .collect();
        kept.sort_unstable_by_key(|r| r.start);
        let mut merged: Vec<Range<usize>> = Vec::with_capacity(kept.len());
        for r in kept {
            match merged.last_mut() {
                Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
                _ => merged.push(r),
            }
        }
        Spans { ranges: merged, at: 0 }
    }

    /// Whether byte `i` is in a range; `i` is never less than the one asked before.
    fn covers(&mut self, i: usize) -> bool {
        while self.ranges.get(self.at).is_some_and(|r| r.end <= i) {
            self.at += 1;
        }
        self.ranges.get(self.at).is_some_and(|r| r.start <= i)
    }
}

/// The paragraphs of a text being composed.
#[derive(Default)]
struct Lines {
    blocks: Vec<Block>,
    /// The runs of the line being read.
    line: Vec<Run>,
    /// Runs and empty paragraphs kept, at most [`TEAMS_RUNS`].
    kept: usize,
}

impl Lines {
    /// Adds `c` to the line in its styles; false when no run is left for it.
    fn put(&mut self, c: char, bold: bool, italic: bool) -> bool {
        if let Some(run) = self.line.last_mut().filter(|r| (r.bold, r.italic) == (bold, italic)) {
            run.text.push(c);
            return true;
        }
        if self.kept >= TEAMS_RUNS {
            return false;
        }
        self.kept += 1;
        self.line.push(Run { text: c.to_string(), bold, italic, ..Run::default() });
        true
    }

    /// Ends the line: a paragraph, an empty one when it's blank, none for a blank line before the
    /// first that isn't; false when no room is left for it.
    fn end(&mut self) -> bool {
        let runs = std::mem::take(&mut self.line);
        if !runs.iter().all(|r| r.text.chars().all(char::is_whitespace)) {
            self.blocks.push(Block { kind: BlockKind::Paragraph, runs });
            return true;
        }
        self.kept = self.kept.saturating_sub(runs.len());
        if self.blocks.is_empty() {
            return true;
        }
        if self.kept >= TEAMS_RUNS {
            return false;
        }
        self.kept += 1;
        self.blocks.push(Block::default());
        true
    }
}

/// The HTML being written, within [`SEND_MAX`].
#[derive(Default)]
struct Writer {
    out: String,
}

impl Writer {
    /// Adds `s`; `Err` once past [`SEND_MAX`].
    fn push(&mut self, s: &str) -> Result<(), TooLong> {
        self.out.push_str(s);
        if self.out.len() > SEND_MAX { Err(TooLong { bytes: self.out.len() }) } else { Ok(()) }
    }

    /// A run in its styles and its link, nothing when its text shows nothing.
    fn run(&mut self, run: &Run) -> Result<(), TooLong> {
        if !run.text.chars().any(|c| c == '\n' || c == '\t' || !c.is_control()) {
            return Ok(());
        }
        let link = run.link.as_deref().and_then(web_address);
        let tags = [(run.bold, "b"), (run.italic, "i"), (run.underline, "u"), (run.strike, "s"), (run.code, "code")];
        if let Some(link) = &link {
            // No space nor control character is left in it (`web_address`): it's only escaped.
            self.push("<a href=\"")?;
            self.text(link)?;
            self.push("\">")?;
        }
        for (_, tag) in tags.iter().filter(|(on, _)| *on) {
            self.push("<")?;
            self.push(tag)?;
            self.push(">")?;
        }
        self.text(&run.text)?;
        for (_, tag) in tags.iter().rev().filter(|(on, _)| *on) {
            self.push("</")?;
            self.push(tag)?;
            self.push(">")?;
        }
        if link.is_some() {
            self.push("</a>")?;
        }
        Ok(())
    }

    /// `text` escaped, a line break as `<br>`, other control characters but tabs left out.
    fn text(&mut self, text: &str) -> Result<(), TooLong> {
        let mut one = [0; 4];
        for c in text.chars() {
            match c {
                '\n' => self.push("<br>")?,
                '\t' => self.push("\t")?,
                _ if c.is_control() => {}
                _ => match entity(c) {
                    Some(e) => self.push(e)?,
                    None => self.push(c.encode_utf8(&mut one))?,
                },
            }
        }
        Ok(())
    }
}
