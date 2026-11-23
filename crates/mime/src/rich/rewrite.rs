// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The pure half of Rewrite: the part of a message sent to the language model as Markdown, and
//! its answer read back as the editor's text and styles. Pictures travel as markers no text in
//! the part holds; one the model lost comes back at the end of its paragraph, never away.
//!
//! What is hostile in it: the model's answer, of any size and nesting. It is read through the
//! Markdown reader within its cap, its own internal marks are dropped, and only the markers of
//! pictures actually sent become pictures.

use crate::rich::{List, PICTURE, Para, Style, Styles};
use std::collections::HashMap;

/// The form of a picture's marker: no Markdown character, and a nonce no text holds.
pub const MARKER: &str = "QUATRELLE-IMAGE";

/// A picture in a rewrite: the marker sent in its place, its own look and the paragraph it was
/// in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RewritePicture {
    pub(crate) marker: String,
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
            let marker = format!("{MARKER}-{nonce}-{}-END", images.len() + 1);
            let look = style.for_typing();
            let id = marked.intern(look.clone());
            for c in marker.chars() {
                out.push(c);
                marked.chars.push(id);
            }
            images.push(RewritePicture { marker, style, para });
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
    for (n, (text, style)) in runs.iter().enumerate() {
        if text.is_empty() {
            continue;
        }
        // White space has no look Markdown can mark on its own (a code span of only spaces is
        // no code span): it goes as it is.
        if text.chars().all(char::is_whitespace) {
            out.push_str(text);
            continue;
        }
        if style.mono {
            let (open, close) = emphasis(style);
            boundary(&open, text, out);
            out.push_str(&open);
            write_code(text, out);
            out.push_str(&close);
            after_close(runs, n, out);
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
            let (open, close) = emphasis(style);
            let first: String = core.iter().collect();
            boundary(&open, &first, out);
            out.push_str(&open);
            for c in core {
                escape_into(*c, out, esc);
            }
            out.push_str(&close);
            after_close(runs, n, out);
        }
        for c in chars.get(core_end..).unwrap_or_default() {
            escape_into(*c, out, esc);
        }
    }
}

/// After closing marks that another run's text follows: an empty HTML comment keeps the closing
/// run from touching a word, which would stop it closing (its last mark is punctuation).
fn after_close(runs: &[(String, Style)], n: usize, out: &mut String) {
    if runs.iter().skip(n + 1).any(|(t, _)| t.chars().any(|c| !c.is_whitespace())) {
        out.push_str("<!-- -->");
    }
}

/// Where a run's opening marks would touch a word, with punctuation after them, Markdown would
/// not read them as emphasis: an empty HTML comment gives the word boundary without adding a
/// character to the text (the reader drops raw HTML).
fn boundary(open: &str, text: &str, out: &mut String) {
    let touched = out.chars().next_back().is_some_and(char::is_alphanumeric);
    let first = open.chars().next();
    // A second kind of mark right after the first is punctuation to Markdown's delimiter run,
    // whatever the text begins with.
    let more_marks = first.is_some_and(|f| open.chars().any(|c| c != f));
    let starts_with_punctuation = text.chars().next().is_some_and(|c| !c.is_alphanumeric() && !c.is_whitespace());
    if touched && (more_marks || starts_with_punctuation) {
        out.push_str("<!-- -->");
    }
}

/// The marks around a run's emphasis (none for plain text), and the marks that close them.
fn emphasis(style: &Style) -> (String, String) {
    let open = format!("{}{}{}", if style.bold { "**" } else { "" }, if style.italic { "*" } else { "" }, if style.strike { "~~" } else { "" });
    let close: String = open.chars().rev().collect();
    (open, close)
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

/// The look Markdown cannot carry (colour, typeface, size, highlight, underline) that most of
/// the text's characters have, leaving out title lines, links, code and pictures: what the
/// rewritten text takes where Markdown says nothing.
pub fn base_style(text: &str, styles: &Styles) -> Style {
    type Look = (Option<[u8; 3]>, Option<String>, Option<u8>, Option<[u8; 3]>, bool);
    let mut counts: HashMap<Look, usize> = HashMap::new();
    let mut order: Vec<Look> = Vec::new();
    let mut start = 0usize;
    for line in text.split('\n') {
        let chars: Vec<char> = line.chars().collect();
        if heading_level(&chars, start, styles) == 0 {
            for i in 0..chars.len() {
                let s = styles.style(start + i);
                if s.link.is_some() || s.mono || s.picture.is_some() {
                    continue;
                }
                let look = (s.color, s.font.clone(), s.size, s.highlight, s.underline);
                let n = counts.entry(look.clone()).or_insert(0);
                if *n == 0 {
                    order.push(look);
                }
                *n = n.saturating_add(1);
            }
        }
        start = start.saturating_add(chars.len()).saturating_add(1);
    }
    let mut best: Option<(Look, usize)> = None;
    for look in order {
        let n = counts.get(&look).copied().unwrap_or(0);
        if best.as_ref().is_none_or(|(_, most)| n > *most) {
            best = Some((look, n));
        }
    }
    match best {
        Some(((color, font, size, highlight, underline), _)) => Style { color, font, size, highlight, underline, ..Style::default() },
        None => Style::default(),
    }
}

/// The model's answer as the editor's text and styles: its Markdown read back (bounded), its
/// markers put back as their pictures, one it lost at the end of its paragraph, and the colours,
/// typefaces and sizes Markdown doesn't carry taken from `base`.
pub fn inbound(answer: &str, base: Style, images: &[RewritePicture], before: &str, before_styles: &Styles) -> (String, Styles) {
    // The model's answer is hostile: its own copy of Quatrelle's internal marks (a signature
    // standing, a rule) and of a picture would be read as ours. None may pass.
    let answer = without_outer_fence(answer);
    let answer: String = answer.trim().chars().filter(|c| *c != PICTURE && !('\u{E000}'..='\u{E06F}').contains(c)).collect();
    // One reader, the Markdown one: its backslash escapes are what the outbound writer wrote.
    let Some((mut text, mut styles)) = crate::read_markdown(&answer) else { return (String::new(), Styles::plain("")) };
    // The HTML reader draws a strikethrough with a combining mark over each character; the
    // editor keeps the style instead.
    crate::rich::retain(&mut text, &mut styles, |c, _| c != '\u{336}');
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut out_styles = Styles::default();
    out_styles.palette.push(Style::default());
    let mut seen = vec![false; images.len()];
    let markers: HashMap<&str, usize> = images.iter().enumerate().map(|(k, im)| (im.marker.as_str(), k)).collect();
    let nonce = images.first().and_then(|im| marker_shape(&im.marker.chars().collect::<Vec<_>>(), 0).map(|(_, n, _)| n));
    let mut i = 0usize;
    while i < chars.len() {
        if let Some((len, token_nonce, has_end)) = marker_shape(&chars, i) {
            let token: String = chars.get(i..i + len).map(|c| c.iter().collect()).unwrap_or_default();
            let exact = markers.get(token.as_str()).copied().filter(|k| !seen.get(*k).copied().unwrap_or(true));
            if let Some(k) = exact
                && let Some(image) = images.get(k)
            {
                if let Some(s) = seen.get_mut(k) {
                    *s = true;
                }
                out.push(PICTURE);
                let id = out_styles.intern(image.style.clone());
                out_styles.chars.push(id);
                i += len;
                continue;
            }
            // The user's own words that happen to look like a marker stay; anything else of the
            // marker's shape came from the model and goes.
            if before.contains(&token) {
                for k in i..i + len {
                    copy_char(&chars, &styles, &mut out, &mut out_styles, k);
                }
                i += len;
                continue;
            }
            if has_end && len > 0 {
                i += len;
                continue;
            }
            if nonce == Some(token_nonce) {
                let mut end = i + len;
                let limit = i.saturating_add(MARKER_MAX);
                while end < chars.len() && end < limit && !chars.get(end).is_some_and(|c| c.is_whitespace()) {
                    if chars.get(end..).is_some_and(|c| c.starts_with(&['-', 'E', 'N', 'D'])) {
                        end += 4;
                        break;
                    }
                    end += 1;
                }
                i = end.max(i + 1);
                continue;
            }
        }
        copy_char(&chars, &styles, &mut out, &mut out_styles, i);
        i += 1;
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
    let (out, mut out_styles) = restore_breaks(&out, &out_styles, before, before_styles);
    out_styles.restyle(&out, 0..out.chars().count(), |s| {
        if s.picture.is_some() {
            return s.clone();
        }
        let heading = s.bold && matches!(s.size, Some(20 | 16 | 14));
        Style {
            color: s.color.or(base.color),
            font: s.font.clone().or(base.font.clone()),
            size: if heading { s.size } else { s.size.or(base.size) },
            highlight: s.highlight.or(base.highlight),
            underline: s.underline || base.underline,
            ..s.clone()
        }
    });
    (out, out_styles)
}

/// The answer with the line breaks of the text sent, when its characters (all but the breaks)
/// are the same: the Markdown reader separates blocks with empty lines of its own, and a model
/// that kept the text must not add or lose a line. Paragraph layouts follow the answer's lines,
/// in order.
fn restore_breaks(text: &str, styles: &Styles, before: &str, before_styles: &Styles) -> (String, Styles) {
    let chars = |s: &str| s.chars().filter(|c| *c != '\n').collect::<Vec<_>>();
    if chars(text) != chars(before) {
        return (text.to_string(), styles.clone());
    }
    // The answer's paragraphs that hold text or a rule, in order.
    let mut answer_paras = Vec::new();
    for (n, line) in text.split('\n').enumerate() {
        let para = styles.para(n);
        if !line.trim().is_empty() || para.rule {
            answer_paras.push(para);
        }
    }
    let answer: Vec<(char, Style)> = text.chars().enumerate().filter(|(_, c)| *c != '\n').map(|(i, c)| (c, styles.style(i))).collect();
    // The characters' own look (colour, typeface, size, highlight, underline) as it was: what
    // Markdown does not carry stays exactly where it was when the text did not change.
    let before_chars: Vec<Style> = before.chars().enumerate().filter(|(_, c)| *c != '\n').map(|(i, _)| before_styles.style(i)).collect();
    let line_count = before.split('\n').count();
    let mut out = String::with_capacity(before.len());
    let mut kept =
        Styles { chars: Vec::with_capacity(before.chars().count()), palette: styles.palette.clone(), paras: Vec::with_capacity(line_count) };
    if kept.palette.is_empty() {
        kept.palette.push(Style::default());
    }
    let mut next = 0usize;
    let mut next_para = 0usize;
    for (n, line) in before.split('\n').enumerate() {
        let ours = !line.trim().is_empty() || before_styles.para(n).rule;
        let para = if ours {
            let p = answer_paras.get(next_para).copied().unwrap_or_default();
            next_para += 1;
            p
        } else {
            Para::default()
        };
        kept.paras.push(para);
        for _ in line.chars() {
            let (ch, style) = answer.get(next).cloned().unwrap_or((' ', Style::default()));
            let was = before_chars.get(next).cloned().unwrap_or_default();
            let style = Style { color: was.color, font: was.font, size: was.size, highlight: was.highlight, underline: was.underline, ..style };
            out.push(ch);
            next += 1;
            let id = kept.intern(style);
            kept.chars.push(id);
        }
        if n + 1 < line_count {
            out.push('\n');
            let id = kept.intern(Style::default());
            kept.chars.push(id);
        }
    }
    kept.fit(&out);
    (out, kept)
}

#[cfg(test)]
mod tests;

/// An answer written whole inside one fenced code block (a model asked for Markdown often does
/// it): the fence goes, the text stays.
fn without_outer_fence(answer: &str) -> String {
    let text = answer.trim();
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() >= 2 && lines.first().is_some_and(|l| l.trim_start().starts_with("```")) && lines.last().is_some_and(|l| l.trim() == "```") {
        lines.get(1..lines.len() - 1).map(|l| l.join("\n")).unwrap_or_else(|| text.to_string())
    } else {
        text.to_string()
    }
}

/// The most characters of a marker read, so a stranger's answer can't make a long walk.
const MARKER_MAX: usize = 160;

/// A marker's shape at `i`: its length when it ends (`…-<n>-END`), the nonce read, and whether
/// it ended; a marker stopped before `-END` has the length read so far.
fn marker_shape(chars: &[char], i: usize) -> Option<(usize, u64, bool)> {
    let prefix: Vec<char> = MARKER.chars().chain(['-']).collect();
    if !chars.get(i..).is_some_and(|c| c.starts_with(&prefix)) {
        return None;
    }
    let mut at = i + prefix.len();
    let start = at;
    let mut nonce = 0u64;
    while let Some(d) = chars.get(at).and_then(|c| c.to_digit(10)) {
        nonce = nonce.saturating_mul(10).saturating_add(u64::from(d));
        at += 1;
        if at - start > 20 {
            return None;
        }
    }
    if at == start || chars.get(at) != Some(&'-') {
        return None;
    }
    at += 1;
    let number = at;
    while chars.get(at).is_some_and(char::is_ascii_digit) {
        at += 1;
    }
    if at == number {
        return Some((at - i, nonce, false));
    }
    if chars.get(at..).is_some_and(|c| c.starts_with(&['-', 'E', 'N', 'D'])) { Some((at + 4 - i, nonce, true)) } else { Some((at - i, nonce, false)) }
}

/// One character with its look, a line break with none.
fn copy_char(chars: &[char], styles: &Styles, out: &mut String, out_styles: &mut Styles, i: usize) {
    let Some(ch) = chars.get(i).copied() else { return };
    out.push(ch);
    let id = if ch == '\n' { out_styles.intern(Style::default()) } else { out_styles.intern(styles.style(i)) };
    out_styles.chars.push(id);
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
