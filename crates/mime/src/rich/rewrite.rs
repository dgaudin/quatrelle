// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The pure half of Rewrite: the part of a message sent to the language model as Markdown, and
//! its answer read back as the editor's text and styles. Pictures travel as markers no text in
//! the part holds; one the model lost comes back at the end of its paragraph, never away.
//!
//! What is hostile in it: the model's answer, of any size and nesting. It is read through the
//! Markdown reader within its cap, its own internal marks are dropped, and only the markers of
//! pictures actually sent become pictures.

use crate::rich::{List, PICTURE, Style, Styles};

/// The form of a picture's marker: no Markdown character, and a nonce no text holds.
pub const MARKER: &str = "QUATRELLE-IMAGE";

/// A picture in a rewrite: the marker sent in its place, the `cid:` it stands for, its own look
/// and the paragraph it was in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RewritePicture {
    pub(crate) marker: String,
    pub(crate) cid: String,
    pub(crate) style: Style,
    pub(crate) para: usize,
}

/// The part of a message sent for a rewrite, with each picture replaced by a marker no text
/// holds (the marker's characters take the picture's look without being one).
fn mark_pictures(text: &str, styles: &Styles, nonce: u64) -> (String, Styles, Vec<RewritePicture>) {
    let mut out = String::with_capacity(text.len());
    let mut marked = Styles::default();
    marked.palette.push(Style::default());
    let mut images = Vec::new();
    let mut para = 0usize;
    for (i, ch) in text.chars().enumerate() {
        if ch == PICTURE {
            let style = styles.style(i);
            if let Some(cid) = style.picture.clone().filter(|c| !c.is_empty()) {
                let marker = format!("{MARKER}-{nonce}-{}-END", images.len() + 1);
                let look = style.for_typing();
                let id = marked.intern(look.clone());
                for c in marker.chars() {
                    out.push(c);
                    marked.chars.push(id);
                }
                images.push(RewritePicture { marker, cid, style, para });
            }
            continue;
        }
        if ch == '\n' {
            para += 1;
        }
        out.push(ch);
        let id = marked.intern(styles.style(i));
        marked.chars.push(id);
    }
    marked.paras = styles.paras.clone();
    marked.fit(&out);
    (out, marked, images)
}

/// The part of a message sent to the model: its pictures are markers no text in it holds, and
/// the rest is Markdown (emphasis, code, lists, headings and links, its paragraphs and rules).
/// Markdown carries no colour, typeface or size, so those are left for the answer's base style.
pub fn outbound(text: &str, styles: &Styles, nonce: u64) -> (String, Vec<RewritePicture>) {
    let (marked, marked_styles, images) = mark_pictures(text, styles, nonce);
    (markdown_of(&marked, &marked_styles), images)
}

/// A written text as Markdown, guided by its paragraphs rather than by what the text looks
/// like: lists from their paragraphs, headings from a whole line in bold at a heading's size,
/// an unambiguous rule of its own, and every character that would start a block or open a mark
/// escaped. Pictures are already markers in `text`.
fn markdown_of(text: &str, styles: &Styles) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out = String::with_capacity(text.len() + text.len() / 4);
    let mut start = 0usize;
    for (n, line) in lines.iter().enumerate() {
        if n > 0 {
            out.push('\n');
        }
        let para = styles.para(n);
        let chars: Vec<char> = line.chars().collect();
        let heading = heading_level(&chars, start, styles);
        if para.rule && chars.iter().all(|c| c.is_whitespace()) {
            out.push_str("***");
            start += chars.len() + 1;
            continue;
        }
        let indent = "    ".repeat(usize::from(para.indent));
        let mut content_started = false;
        match para.list {
            List::Bullet if heading == 0 => {
                out.push_str(&indent);
                out.push_str("- ");
                content_started = true;
            }
            List::Number if heading == 0 => {
                out.push_str(&indent);
                out.push_str("1. ");
                content_started = true;
            }
            _ => {}
        }
        if heading > 0 {
            out.push_str(&"#".repeat(usize::from(heading)));
            out.push(' ');
            content_started = true;
        }
        let mut esc = Escaping { at_line_start: !content_started, digits: 0 };
        let runs = runs_of(&chars, start, heading > 0, styles);
        write_runs(&runs, &mut out, &mut esc);
        start += chars.len() + 1;
    }
    out
}

/// A line's heading level when every non-blank character is in bold at the same heading size
/// (20, 16 or 14 points): 0 for ordinary text.
fn heading_level(chars: &[char], start: usize, styles: &Styles) -> u8 {
    let mut level = 0u8;
    for (i, ch) in chars.iter().enumerate() {
        if ch.is_whitespace() {
            continue;
        }
        let style = styles.style(start + i);
        let one = match (style.bold, style.size) {
            (true, Some(20)) => 1,
            (true, Some(16)) => 2,
            (true, Some(14)) => 3,
            _ => 0,
        };
        if one == 0 || (level != 0 && level != one) {
            return 0;
        }
        level = one;
    }
    level
}

/// A run of characters with one Markdown look: the attributes Markdown carries, not colour,
/// typeface or size.
fn runs_of(chars: &[char], start: usize, heading: bool, styles: &Styles) -> Vec<(String, Style)> {
    let mut runs: Vec<(String, Style)> = Vec::new();
    for (i, ch) in chars.iter().enumerate() {
        let style = styles.style(start + i);
        let look = Style {
            bold: style.bold && !heading,
            italic: style.italic,
            strike: style.strike,
            mono: style.mono,
            link: style.link.clone().filter(|l| super::safe_link(l)),
            ..Style::default()
        };
        match runs.last_mut() {
            Some((text, last)) if *last == look => text.push(*ch),
            _ => runs.push((ch.to_string(), look)),
        }
    }
    runs
}

/// What a line's next character would start, as Markdown reads the line's beginning.
struct Escaping {
    at_line_start: bool,
    digits: usize,
}

/// A run's text escaped, keeping only the white space at its ends outside any emphasis.
fn write_runs(runs: &[(String, Style)], out: &mut String, esc: &mut Escaping) {
    let mut i = 0usize;
    while let Some((_, style)) = runs.get(i) {
        let link = style.link.clone();
        let end = runs.iter().skip(i + 1).position(|(_, s)| s.link != link).map_or(runs.len(), |p| p + i + 1);
        let group = runs.get(i..end).unwrap_or_default();
        match &link {
            Some(url) => {
                let plain: String = group.iter().map(|(t, _)| t.as_str()).collect();
                let bare = group.iter().all(|(_, s)| !(s.bold || s.italic || s.strike || s.mono))
                    && (plain.trim() == url.trim() || url.strip_prefix("mailto:").is_some_and(|m| m == plain.trim()));
                if bare {
                    write_plain(group, out, esc);
                } else {
                    out.push('[');
                    write_plain(group, out, esc);
                    out.push_str("](");
                    out.push_str(&destination(url));
                    out.push(')');
                }
            }
            None => write_plain(group, out, esc),
        }
        i = end;
    }
}

/// Text runs with their emphasis and code, no link around them.
fn write_plain(runs: &[(String, Style)], out: &mut String, esc: &mut Escaping) {
    for (text, style) in runs {
        if text.is_empty() {
            continue;
        }
        if style.mono {
            write_code(text, out);
            if text.chars().any(|c| !c.is_whitespace()) {
                esc.at_line_start = false;
                esc.digits = 0;
            }
            continue;
        }
        let chars: Vec<char> = text.chars().collect();
        let lead = chars.iter().take_while(|c| c.is_whitespace()).count();
        let trail = chars.iter().rev().take_while(|c| c.is_whitespace()).count().min(chars.len().saturating_sub(lead));
        let core_end = chars.len().saturating_sub(trail);
        for c in chars.get(..lead).unwrap_or_default() {
            escape_into(*c, out, esc);
        }
        let core = chars.get(lead..core_end).unwrap_or_default();
        if !core.is_empty() {
            let open =
                format!("{}{}{}", if style.bold { "**" } else { "" }, if style.italic { "*" } else { "" }, if style.strike { "~~" } else { "" });
            out.push_str(&open);
            for c in core {
                escape_into(*c, out, esc);
            }
            let close: String = open.chars().rev().collect();
            out.push_str(&close);
        }
        for c in chars.get(core_end..).unwrap_or_default() {
            escape_into(*c, out, esc);
        }
    }
}

/// Code as a code span, in enough backticks for its own, with breathing space when it starts or
/// ends with a backtick or a space.
fn write_code(text: &str, out: &mut String) {
    let mut longest = 0usize;
    let mut now = 0usize;
    for c in text.chars() {
        now = if c == '`' { now + 1 } else { 0 };
        longest = longest.max(now);
    }
    let fence = "`".repeat(longest.saturating_add(1).max(1));
    let pad = if text.starts_with(['`', ' ']) || text.ends_with(['`', ' ']) { " " } else { "" };
    out.push_str(&fence);
    out.push_str(pad);
    out.push_str(text);
    out.push_str(pad);
    out.push_str(&fence);
}

/// A character outside code, escaped when Markdown would read it as a mark or a block's start.
fn escape_into(c: char, out: &mut String, esc: &mut Escaping) {
    let start = esc.at_line_start;
    let mark = match c {
        '\\' | '`' | '*' | '_' | '[' | ']' | '~' | '<' | '>' | '&' | '|' | '=' => true,
        '#' | '-' | '+' if start => true,
        '.' | ')' if esc.digits > 0 => true,
        _ => false,
    };
    if mark {
        out.push('\\');
    }
    out.push(c);
    if c.is_whitespace() {
        return;
    }
    esc.at_line_start = false;
    esc.digits = if esc.digits > 0 || c.is_ascii_digit() { esc.digits.saturating_add(1) } else { 0 };
    if !c.is_ascii_digit() {
        esc.digits = 0;
    }
}

/// An address as a link destination: between angle brackets when it has spaces or brackets.
fn destination(url: &str) -> String {
    let url = url.trim().replace(['\n', '\r'], "");
    if url.contains([' ', '(', ')', '<', '>']) { format!("<{}>", url.replace('<', "%3C").replace('>', "%3E")) } else { url }
}

/// The model's answer as the editor's text and styles: its Markdown read back (bounded), its
/// markers put back as their pictures, one it lost at the end of its paragraph, and the colours,
/// typefaces and sizes Markdown doesn't carry taken from `base`.
pub fn inbound(answer: &str, base: Style, images: &[RewritePicture], max_chars: usize) -> (String, Styles) {
    let answer: String = answer.trim().chars().take(max_chars).collect();
    // One reader, the Markdown one: its backslash escapes are what the outbound writer wrote.
    let Some((text, styles)) = crate::read_markdown(&answer) else { return (String::new(), Styles::plain("")) };
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut out_styles = Styles::default();
    out_styles.palette.push(Style::default());
    let mut seen = vec![false; images.len()];
    let mut i = 0usize;
    while i < chars.len() {
        let found = images
            .iter()
            .enumerate()
            .position(|(k, im)| !seen[k] && chars.get(i..i + im.marker.chars().count()).is_some_and(|s| s.iter().copied().eq(im.marker.chars())));
        if let Some(k) = found {
            let image = &images[k];
            seen[k] = true;
            out.push(PICTURE);
            let id = out_styles.intern(image.style.clone());
            out_styles.chars.push(id);
            i += image.marker.chars().count();
            continue;
        }
        let ch = chars[i];
        i += 1;
        // A picture the model made up is no picture: only our markers carry one back.
        if ch == PICTURE {
            continue;
        }
        out.push(ch);
        let id = out_styles.intern(styles.style(i - 1));
        out_styles.chars.push(id);
    }
    out_styles.paras = styles.paras.clone();
    // A picture whose marker was lost goes at the end of the paragraph it was in (never away).
    let mut missing: Vec<(usize, usize, Style)> = Vec::new();
    for (k, image) in images.iter().enumerate() {
        if !seen[k] {
            let line = image.para.min(out_styles.paras.len().saturating_sub(1));
            missing.push((line_end(&out, line), k, image.style.clone()));
        }
    }
    // From the end: inserting at an index moves the characters after it.
    missing.sort_by_key(|(at, k, _)| (std::cmp::Reverse(*at), std::cmp::Reverse(*k)));
    for (at, _, style) in missing {
        let byte = out.char_indices().nth(at).map_or(out.len(), |(b, _)| b);
        out.insert(byte, PICTURE);
        let id = out_styles.intern(style);
        out_styles.chars.insert(at, id);
    }
    out_styles.fit(&out);
    out_styles.restyle(&out, 0..out.chars().count(), |s| {
        if s.picture.is_some() {
            return s.clone();
        }
        Style {
            color: s.color.or(base.color),
            font: s.font.clone().or(base.font.clone()),
            size: s.size.or(base.size),
            highlight: s.highlight.or(base.highlight),
            underline: s.underline || base.underline,
            ..s.clone()
        }
    });
    (out, out_styles)
}

#[cfg(test)]
mod tests;

/// Where a paragraph ends in `text` (the character before its newline, or the text's end).
fn line_end(text: &str, line: usize) -> usize {
    let mut seen = 0usize;
    for (i, ch) in text.chars().enumerate() {
        if ch == '\n' {
            if seen == line {
                return i;
            }
            seen += 1;
        }
    }
    text.chars().count()
}
