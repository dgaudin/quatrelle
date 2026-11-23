// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The pure half of Rewrite: the part of a message sent to the language model as Markdown, and
//! its answer read back as the editor's text and styles. Pictures travel as markers no text in
//! the part holds; one the model lost comes back at the end of its paragraph, never away.
//!
//! What is hostile in it: the model's answer, of any size and nesting. It is read through the
//! Markdown reader within its cap, its own internal marks are dropped, and only the markers of
//! pictures actually sent become pictures.

use crate::Span;
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

/// A written text as Markdown: emphasis, code, lists, headings and links, with its paragraphs
/// and rules; pictures are already markers in `text`. Markdown carries no colour, typeface or
/// size, so those are left for the answer's base style.
fn markdown_of(text: &str, styles: &Styles) -> String {
    let mut lines: Vec<Vec<Span>> = Vec::new();
    let mut start = 0usize;
    for (n, line) in text.split('\n').enumerate() {
        let para = styles.para(n);
        if para.rule {
            lines.push(vec![Span { rule: true, ..Span::default() }]);
            start += line.chars().count() + 1;
            continue;
        }
        let mut spans = Vec::new();
        let mark = match para.list {
            List::None => String::new(),
            List::Bullet => "- ".to_string(),
            List::Number => "1. ".to_string(),
        };
        let indent = "    ".repeat(usize::from(para.indent));
        if !mark.is_empty() {
            spans.push(Span { text: format!("{indent}{mark}"), ..Span::default() });
        }
        // A heading the editor keeps as bold text of one of the heading sizes.
        let first = styles.style(start);
        let heading = if first.bold && first.size == Some(20) {
            1
        } else if first.bold && first.size == Some(16) {
            2
        } else if first.bold && first.size == Some(14) {
            3
        } else {
            0
        };
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0usize;
        while i < chars.len() {
            let style = styles.style(start + i);
            let mut j = i + 1;
            while j < chars.len() && styles.style(start + j) == style {
                j += 1;
            }
            spans.push(Span {
                text: chars.get(i..j).map(|c| c.iter().collect()).unwrap_or_default(),
                bold: style.bold && heading == 0,
                italic: style.italic,
                strike: style.strike,
                underline: style.underline,
                code: style.mono,
                link: style.link.clone(),
                heading,
                color: style.color,
                background: style.highlight,
                size: style.size,
                font: style.font.clone(),
                ..Span::default()
            });
            i = j;
        }
        lines.push(spans);
        start += chars.len() + 1;
    }
    crate::rich_markdown(&lines, false)
}

/// The model's answer as the editor's text and styles: its Markdown read back (bounded), its
/// markers put back as their pictures, one it lost at the end of its paragraph, and the colours,
/// typefaces and sizes Markdown doesn't carry taken from `base`.
pub fn inbound(answer: &str, base: Style, images: &[RewritePicture], max_chars: usize) -> (String, Styles) {
    let answer: String = answer.trim().chars().take(max_chars).collect();
    // The paste path the Markdown fuzz target reads; a single sign it doesn't call Markdown
    // (one bold word) still goes through the editor's lighter reader.
    let (text, styles) = crate::pasted_markdown(&answer).unwrap_or_else(|| crate::rich::from_markup(&answer));
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
