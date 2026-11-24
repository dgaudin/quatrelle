// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! HTML mail as text and as rich text: pasted and received HTML, its pictures, headings, links.

use super::*;
use std::collections::{HashMap, HashSet, VecDeque};

mod deep;
pub(crate) use deep::{tag_end, void};
mod footers;
pub(crate) use footers::footers_last;
pub(crate) mod links;
mod markdown;
pub use markdown::{MARKDOWN_HTML_MAX, MARKDOWN_MAX, code_block_markdown, looks_like_markdown, markdown_html, pasted_markdown, read_markdown};
mod pairs;
pub(crate) mod pictures;
mod rules;
mod teams;
mod to_markdown;
pub use pairs::html_link_pairs;
pub(crate) use pictures::{Pictures, tiny_picture, tiny_size};
pub use pictures::{html_images, names_itself, spacer_described};
use rules::{RULE_MARK, mark_rules};
pub use teams::{
    TEAMS_DEPTH, TEAMS_HTML_MAX, TEAMS_RUNS, TooLong, Typed, html_escape, teams_compose, teams_compose_typed, teams_foreign_markup, teams_html,
    teams_text, teams_typed, teams_write, teams_write_mentions, teams_write_parts,
};
pub use to_markdown::{MARKDOWN_WRITTEN_MAX, html_markdown, rich_markdown};

/// Most lines kept of a rich rendering (the rest is shown as text).
pub(super) const RICH_MAX_LINES: usize = 20_000;

/// A picture's size as the message sets it, by its `width` and `height` or its style's (CSS
/// pixels; a width can be a share of the space it's in), as Outlook and browsers show it: a
/// picture twice as big as it says (a sharp one for screens that have the pixels) shows at the
/// size it says, and the layout around it stays as it was made (issue #10).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageSize {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<layout::Width>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// `max-width` in pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_width: Option<u32>,
}

/// Most pictures whose sizes are read from one piece of HTML.
const MAX_SIZED: usize = 2000;

/// A length in CSS pixels (`300`, `300px`, `225pt`, `3.125in`), at most 4000; `None` for anything
/// else (`auto`, `50%`, a length below nothing).
fn css_px(v: &str) -> Option<u32> {
    layout::length_px(v).filter(|n| *n >= 0.0).map(|n| n.min(4000.0) as u32)
}

/// A picture's width as written (`300`, `300px`, `50%`).
fn picture_width(v: &str) -> Option<layout::Width> {
    let v = v.trim();
    match v.strip_suffix('%') {
        Some(p) => p.trim().parse::<f32>().ok().filter(|p| p.is_finite() && *p > 0.0).map(|p| layout::Width::Percent(p.min(100.0) as u8)),
        None => css_px(v).map(layout::Width::Px),
    }
}

/// The size each `<img>` of `html` sets, in order, with its address (`src`).
pub(super) fn picture_sizes(html: &str) -> Vec<(String, ImageSize)> {
    let mut out = Vec::new();
    pictures::img_tags(html, |tag, _| {
        let Some(src) = tag_attr(tag, "src").map(str::trim).filter(|s| !s.is_empty()) else { return true };
        let style = tag_attr(tag, "style").map(css::declarations).unwrap_or_default();
        let css = |prop: &str| style.iter().rev().find(|d| d.prop == prop).map(|d| d.value.as_str());
        // The style's size wins over the attributes', as in a browser: `auto` too, which leaves
        // the side to the picture's shape. A length not read here (Word's inches) leaves the tag's.
        let auto = |prop: &str| css(prop).is_some_and(|v| v.trim().eq_ignore_ascii_case("auto"));
        let width = css("width").and_then(picture_width).or_else(|| tag_attr(tag, "width").and_then(picture_width)).filter(|_| !auto("width"));
        let height = css("height").and_then(css_px).or_else(|| tag_attr(tag, "height").and_then(css_px)).filter(|_| !auto("height"));
        // A share of a box as wide as the tag says, when the style's is a percentage: the width
        // the message was made for, and the most Outlook, which reads only the tag, would show.
        let made_for = match width {
            Some(layout::Width::Percent(_)) => tag_attr(tag, "width").and_then(css_px),
            _ => None,
        };
        let max_width = css("max-width").and_then(css_px).or(made_for);
        out.push((src.to_string(), ImageSize { width, height, max_width }));
        out.len() < MAX_SIZED
    });
    out
}

/// Each picture of `lines` given the size its `<img>` sets: the pictures with one address in
/// turn, in order (the converter keeps them in the order they're written).
fn size_pictures(lines: &mut RichText, sizes: Vec<(String, ImageSize)>) {
    if sizes.is_empty() {
        return;
    }
    let mut by_src: HashMap<String, VecDeque<ImageSize>> = HashMap::new();
    for (src, size) in sizes {
        by_src.entry(src).or_default().push_back(size);
    }
    let mut last: HashMap<String, ImageSize> = HashMap::new();
    for sp in lines.iter_mut().flatten() {
        let Some(src) = sp.image.as_deref().map(str::trim) else { continue };
        let size = by_src.get_mut(src).and_then(VecDeque::pop_front).or_else(|| last.get(src).copied());
        if let Some(size) = size {
            last.insert(src.to_string(), size);
            sp.image_size = (size != ImageSize::default()).then_some(size);
        }
    }
}

/// Word's and Outlook's paragraphs (`class=MsoNormal`) have no space between them (their style
/// sheet, which a copied fragment leaves behind, says so): as lines, not paragraphs.
pub(super) fn word_paragraphs(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    if !lower.contains("mso") {
        return html.to_string();
    }
    let mut out = String::with_capacity(html.len());
    let mut i = 0;
    while let Some(off) = lower.get(i..).and_then(|rest| rest.find('<')) {
        let at = i + off;
        out.push_str(html.get(i..at).unwrap_or(""));
        let tag = lower.get(at..).unwrap_or("");
        let after = |n: usize| tag.as_bytes().get(n).is_none_or(|c| c.is_ascii_whitespace() || matches!(c, b'>' | b'/'));
        if tag.starts_with("<p") && after(2) {
            out.push_str("<div");
            i = at + 2;
        } else if tag.starts_with("</p") && after(3) {
            out.push_str("</div");
            i = at + 3;
        } else {
            out.push('<');
            i = at + 1;
        }
    }
    out.push_str(html.get(i..).unwrap_or(""));
    out
}

/// HTML pasted from another program (a signature copied from Outlook or Word, part of a web
/// page) as text and styles to put into a message: its formatting and links, without pictures
/// (they'd point at the other program's files or the web) or the empty lines around it.
pub fn pasted_html(html: &str) -> (String, rich::Styles) {
    pasted_html_at(html, 2000, false)
}

/// [`pasted_html`] at a given render width: 2000 columns keeps its paragraphs on one line, and a
/// rewrite's reader passes more so that no paragraph is ever cut in two.
pub(crate) fn pasted_html_at(html: &str, width: usize, keep_box_lines: bool) -> (String, rich::Styles) {
    // Sizes and typefaces come with the spans (`html_to_rich` reads them).
    let mut lines = html_to_rich_at(&word_paragraphs(html), width, keep_box_lines);
    for line in &mut lines {
        line.retain(|sp| sp.image.is_none());
    }
    let blank = |l: &Vec<Span>| !l.iter().any(|sp| sp.rule) && l.iter().all(|sp| sp.text.trim().is_empty());
    while lines.last().is_some_and(blank) {
        lines.pop();
    }
    let first = lines.iter().position(|l| !blank(l)).unwrap_or(lines.len());
    lines.drain(..first);
    rich::from_rich(&lines)
}

/// Most pictures taken from pasted HTML, and the most each may weigh.
const PASTED_PICTURES: usize = 20;
const PASTED_PICTURE_MAX: usize = 5 << 20;

/// [`pasted_html`] keeping the pictures that come along: those embedded in the HTML (`data:`
/// addresses, which web pages and some apps copy) become pictures of the message, each where
/// a [`rich::PICTURE`] stands; those named by `cid:` (text copied from this editor, whose
/// message has them) keep their id. Pictures on the web or in another program's files go, and
/// those past the [`MAX_PICTURES`](pictures::MAX_PICTURES) first.
pub fn pasted_html_with_pictures(html: &str) -> (String, rich::Styles, Vec<InlinePicture>) {
    const MARK: char = '\u{E000}';
    // Sizes and typefaces come with the spans (`html_to_rich` reads them).
    let mut lines = html_to_rich(&word_paragraphs(html));
    let mut pictures: Vec<InlinePicture> = Vec::new();
    let mut cids: Vec<String> = Vec::new();
    for line in &mut lines {
        let mut kept: Vec<Span> = Vec::with_capacity(line.len());
        for mut sp in line.drain(..) {
            let Some(src) = sp.image.take() else {
                kept.push(sp);
                continue;
            };
            // At most as many as a message's pictures are listed: past them, a picture goes (and
            // one embedded isn't kept for nothing).
            if cids.len() >= pictures::MAX_PICTURES {
                continue;
            }
            let cid = match src.strip_prefix("cid:") {
                Some(cid) if !cid.is_empty() => Some(cid.to_string()),
                _ => data_picture(&src).filter(|_| pictures.len() < PASTED_PICTURES).map(|p| {
                    let cid = p.cid.clone();
                    if !pictures.iter().any(|x| x.cid == cid) {
                        pictures.push(p);
                    }
                    cid
                }),
            };
            if let Some(cid) = cid {
                cids.push(cid);
                sp.text = MARK.to_string();
                kept.push(sp);
            }
        }
        *line = kept;
    }
    let blank = |l: &Vec<Span>| !l.iter().any(|sp| sp.rule) && l.iter().all(|sp| sp.text.trim().is_empty());
    while lines.last().is_some_and(blank) {
        lines.pop();
    }
    let first = lines.iter().position(|l| !blank(l)).unwrap_or(lines.len());
    lines.drain(..first);
    let (mut text, mut styles) = rich::from_rich(&lines);
    let marks: Vec<usize> = text.chars().enumerate().filter(|(_, c)| *c == MARK).map(|(i, _)| i).collect();
    text = text.replace(MARK, &rich::PICTURE.to_string());
    // Each its picture's style, in one pass (one restyle each walked the whole text again).
    styles.fit(&text);
    for (at, cid) in marks.into_iter().zip(cids) {
        let id = styles.intern(rich::Style { picture: Some(cid), ..styles.style(at) });
        if let Some(c) = styles.chars.get_mut(at) {
            *c = id;
        }
    }
    (text, styles, pictures)
}

/// A picture embedded in HTML (`data:image/png;base64,…`) as a picture of the message, its id
/// made from its bytes (the same picture twice is one). `None` for anything else, or one too
/// big.
fn data_picture(src: &str) -> Option<InlinePicture> {
    use std::hash::{Hash, Hasher};
    let rest = src.trim().strip_prefix("data:")?;
    let (head, body) = rest.split_once(',')?;
    let mut parts = head.split(';');
    let mime = parts.next().unwrap_or("").trim().to_ascii_lowercase();
    if !mime.starts_with("image/") || !parts.any(|p| p.trim().eq_ignore_ascii_case("base64")) || body.len() > PASTED_PICTURE_MAX / 3 * 4 + 4 {
        return None;
    }
    let data = base64_decode(body)?;
    if data.is_empty() || data.len() > PASTED_PICTURE_MAX {
        return None;
    }
    let ext = match mime.as_str() {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        "image/svg+xml" => "svg",
        _ => "png",
    };
    let mut h = std::hash::DefaultHasher::new();
    data.hash(&mut h);
    let cid = format!("{:016x}@pasted.quatrelle", h.finish());
    Some(InlinePicture { cid, name: format!("picture.{ext}"), mime, data })
}

/// Base64 (standard or URL-safe alphabet, whitespace skipped) as bytes; `None` on any other
/// character.
fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let (mut buf, mut bits) = (0u32, 0u8);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return None,
        };
        buf = (buf << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((buf >> bits) & 0xFF).unwrap_or(0));
            buf &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// HTML → lines of formatted spans: paragraphs, emphasis, links, list bullets, quotes, tables
/// laid out as text, and pictures by reference. No scripts, styles or remote loads: what comes
/// out is plain data. Never panics; empty when conversion fails.
pub fn html_to_rich(html: &str) -> RichText {
    html_to_rich_at(html, 2000, false)
}

/// [`html_to_rich`] rendered at `width` columns, keeping a line made only of box-drawing
/// characters as text (`keep_box_lines`) rather than taking it for a table's border.
fn html_to_rich_at(html: &str, width: usize, keep_box_lines: bool) -> RichText {
    let html = match html.char_indices().nth(HTML_MAX) {
        Some((i, _)) => html.get(..i).unwrap_or(html),
        None => html,
    };
    let html = flat_if_too_deep(html);
    let html = html.as_ref();
    let html = footers_last(html);
    let html = html.as_ref();
    // Wide enough that paragraphs stay one line each (the UI wraps them to the pane). Sizes,
    // typefaces and CSS bold are marked first: the converter keeps none of them.
    // Lines across it (an `<hr>`, Outlook's border over a quoted message) are marked first.
    let ruled = mark_rules(html);
    let (fonts_marked, fonts) = crate::fonts::mark(&ruled);
    let marked = close_css_rules(&mark_pictures(&mark_headings(&fonts_marked)));
    // The message's own CSS is read for colours and hidden parts (`display: none`), nothing else.
    let result = std::panic::catch_unwind(|| html2text::config::rich().use_doc_css().lines_from_read(marked.as_bytes(), width));
    let Ok(Ok(lines)) = result else { return Vec::new() };
    let mut out: RichText = Vec::new();
    let mut blank = 0;
    let mut open: Vec<usize> = Vec::new();
    // The title level of the line before (0: not a title's).
    let mut title = 0u8;
    for line in lines.into_iter().take(RICH_MAX_LINES) {
        let mut spans: Vec<Span> = Vec::new();
        // A line that held only marks (where a sized paragraph starts, alone before a picture or
        // a table) isn't a line of the message: it goes, not left as a blank one.
        let only_marks = line.tagged_strings().any(|ts| crate::fonts::has_marks(&ts.s))
            && line
                .tagged_strings()
                .all(|ts| crate::fonts::only_marks(&ts.s) && !ts.tag.iter().any(|a| matches!(a, html2text::render::RichAnnotation::Image(_))));
        let mut start = crate::fonts::LineStart::default();
        for ts in line.tagged_strings() {
            // Preformatted text keeps its spaces where its line starts.
            let s = if ts.tag.iter().any(|a| matches!(a, html2text::render::RichAnnotation::Code | html2text::render::RichAnnotation::Preformat(_))) {
                start = crate::fonts::LineStart::Words;
                std::borrow::Cow::Borrowed(ts.s.as_str())
            } else {
                crate::fonts::without_spaces_after_marks(&ts.s, &mut start)
            };
            for (text, font) in crate::fonts::runs(&s, &mut open, &fonts) {
                let mut sp = Span {
                    text,
                    bold: font.bold,
                    italic: font.italic,
                    underline: font.underline,
                    size: font.size,
                    font: font.family,
                    ..Default::default()
                };
                for a in &ts.tag {
                    match a {
                        html2text::render::RichAnnotation::Strong => sp.bold = true,
                        html2text::render::RichAnnotation::Emphasis => sp.italic = true,
                        html2text::render::RichAnnotation::Strikeout => sp.strike = true,
                        html2text::render::RichAnnotation::Code | html2text::render::RichAnnotation::Preformat(_) => sp.code = true,
                        html2text::render::RichAnnotation::Link(l) => sp.link = Some(l.clone()),
                        html2text::render::RichAnnotation::Image(src) => sp.image = Some(src.clone()),
                        html2text::render::RichAnnotation::Colour(c) => sp.color = Some([c.r, c.g, c.b]),
                        html2text::render::RichAnnotation::BgColour(c) => sp.background = Some([c.r, c.g, c.b]),
                        _ => {}
                    }
                }
                // Merge with the previous span when the formatting is the same.
                match spans.last_mut() {
                    Some(last) if Span { text: String::new(), ..last.clone() } == Span { text: String::new(), ..sp.clone() } => {
                        last.text.push_str(&sp.text)
                    }
                    _ => spans.push(sp),
                }
            }
        }
        if let Some(last) = spans.last_mut() {
            let trimmed = last.text.trim_end().len();
            last.text.truncate(trimmed);
        }
        // Headings come out as `## <mark>Title` (see `mark_headings`): drop both, keep the level.
        let mut level = 0u8;
        for sp in &mut spans {
            if let Some(i) = sp.text.find(HEADING_MARK) {
                let before = sp.text.get(..i).unwrap_or("");
                level = u8::try_from(before.chars().filter(|c| *c == '#').count().clamp(1, 6)).unwrap_or(1);
                sp.text = sp.text.get(i + HEADING_MARK.len_utf8()..).unwrap_or("").to_string();
            }
        }
        // A title's next lines (`<h3>Alpha<br>Beta</h3>`) come out `### Beta`, the mark on its
        // first line only: they're the title's too, and one with nothing more (the line break
        // that ends it, in a template's sized greeting) is no line at all.
        let mut more_of_title = false;
        if level == 0
            && title > 0
            && let Some(first) = spans.iter_mut().find(|s| !s.text.is_empty())
        {
            let marks = "#".repeat(usize::from(title));
            let rest = first.text.trim_start().strip_prefix(marks.as_str()).filter(|r| r.is_empty() || r.starts_with(' '));
            if let Some(rest) = rest {
                first.text = rest.trim_start().to_string();
                level = title;
                more_of_title = true;
            }
        }
        title = level;
        if more_of_title && spans.iter().all(|s| s.text.trim().is_empty() && s.image.is_none()) {
            continue;
        }
        if level > 0 {
            if let Some(first) = spans.first_mut()
                && first.text.trim_start_matches('#').trim().is_empty()
                && first.image.is_none()
            {
                first.text.clear();
            }
            for sp in &mut spans {
                sp.heading = level;
            }
        }
        // A line across the message: alone on its line, drawn by the reader.
        if spans.iter().map(|s| s.text.as_str()).collect::<String>().trim() == RULE_MARK.to_string() {
            out.push(vec![Span { rule: true, ..Default::default() }]);
            blank = 0;
            continue;
        }
        for sp in &mut spans {
            sp.text = sp.text.replace([HEADING_MARK, PICTURE_MARK, RULE_MARK], "");
        }
        // List bullets (`* item`, indented when nested) as real bullets.
        if let Some(first) = spans.first_mut() {
            let indent = first.text.len() - first.text.trim_start_matches(' ').len();
            if first.text.get(indent..).is_some_and(|r| r.starts_with("* ")) {
                first.text = format!("{}• {}", " ".repeat(indent), first.text.get(indent + 2..).unwrap_or(""));
            }
        }
        // Tables come laid out with box-drawing borders. A layout table (one column) only adds
        // rules around its text: drop those. A data table's rows line up only in a monospace font.
        let whole: String = spans.iter().map(|s| s.text.as_str()).collect();
        if !keep_box_lines && !whole.trim().is_empty() && whole.chars().all(|c| matches!(c, '─' | '━' | '═' | ' ')) {
            spans.clear();
        } else if whole.contains(['│', '┼', '┬', '┴']) {
            for sp in &mut spans {
                sp.code = true;
            }
        }
        spans.retain(|s| !s.text.is_empty() || s.image.is_some());
        // Addresses written out without a link (`www.…`, `https://…`, `name@host`) open as links.
        let mut spans = crate::links::link_written_addresses(spans);
        // White space alone beside a picture (the indent of the markup around it): it made the
        // line wider than the picture's box, and the picture went to the next line.
        if spans.iter().any(|s| s.image.is_some()) {
            spans.retain(|s| s.image.is_some() || !s.text.trim().is_empty());
        }
        if spans.is_empty() && only_marks {
            continue;
        }
        if spans.is_empty() {
            blank += 1;
            if blank > 1 || out.is_empty() {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push(spans);
    }
    while out.last().is_some_and(Vec::is_empty) {
        out.pop();
    }
    size_pictures(&mut out, picture_sizes(html));
    out
}

/// Marks where a heading starts, so a paragraph that just begins with `#` isn't taken for one.
pub(super) const HEADING_MARK: char = '\u{E000}';

/// The description given to pictures that have none (a report's charts, a logo), which the
/// converter would otherwise leave out: removed again once they're pictures.
pub(super) const PICTURE_MARK: char = '\u{E001}';

/// An attribute's value in a tag's text (`<img alt="x" …>`), as written, read as a browser reads
/// the tag ([`deep::attributes`]: a value in quotes that reads like another attribute,
/// `alt=" width=500"`, isn't one); its first, as a browser keeps; `Some("")` when it's there
/// and empty.
pub(super) fn tag_attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    // Past `<` (or `</`, either can be left out) and the element's name.
    let named = tag.strip_prefix('<').unwrap_or(tag);
    let named = named.strip_prefix('/').unwrap_or(named);
    let from = tag.len() - named.len() + named.find(|c: char| c.is_ascii_whitespace() || c == '/' || c == '>').unwrap_or(named.len());
    let mut found = None;
    deep::attributes(tag, from, |n, v| {
        if n.eq_ignore_ascii_case(name) {
            found = Some(v);
        }
        found.is_none()
    });
    found
}

/// `html` with a description on every picture that has none, except tracking pixels and spacers
/// ([`tiny_picture`]).
pub(super) fn mark_pictures(html: &str) -> String {
    let html = html.replace(PICTURE_MARK, "");
    let mut out = String::with_capacity(html.len() + 64);
    let mut last = 0;
    pictures::img_tags(&html, |tag, range| {
        let described = tag_attr(tag, "alt").is_some_and(|a| !a.trim().is_empty());
        if !described && !tiny_picture(tag) && tag_attr(tag, "src").is_some_and(|s| !s.trim().is_empty()) {
            // After the element's name (`<img`, `<image`).
            let named = range.start + 1 + tag.get(1..).and_then(|t| t.find(|c: char| c.is_ascii_whitespace() || c == '/' || c == '>')).unwrap_or(0);
            out.push_str(html.get(last..named).unwrap_or(""));
            out.push_str(&format!(" alt=\"{PICTURE_MARK}\""));
            last = named;
        }
        true
    });
    out.push_str(html.get(last..).unwrap_or(""));
    out
}

/// The attachment a picture of the HTML is, by its address: `cid:` (RFC 2392, %-escapes
/// decoded) matched to a `Content-ID`, or to a file name when there's no such id; any other
/// address that isn't on the web (`logo.png`, `images/chart.png`) matched to a part's
/// `Content-Location` (RFC 2557) or its file name, as Outlook finds them.
pub fn picture_attachment(p: &Parsed, src: &str) -> Option<usize> {
    let src = src.trim();
    let lower = src.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("data:") || src.is_empty() || src.len() > 4096 {
        return None;
    }
    let decoded = |s: &str| {
        let b = s.as_bytes();
        let mut out = Vec::with_capacity(b.len());
        let mut i = 0;
        while let Some(&c) = b.get(i) {
            let hex = b.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
            match (c, hex) {
                (b'%', Some(v)) => {
                    out.push(v);
                    i += 3;
                }
                _ => {
                    out.push(c);
                    i += 1;
                }
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    };
    let last_part = |s: &str| s.rsplit(['/', '\\']).next().unwrap_or(s).to_string();
    let name_is = |a: &AttachmentInfo, n: &str| !n.is_empty() && a.name.eq_ignore_ascii_case(n);
    if let Some(cid) = src.get(..4).filter(|s| s.eq_ignore_ascii_case("cid:")).and_then(|_| src.get(4..)) {
        let cid = cid.trim().trim_matches(['<', '>']);
        let plain = decoded(cid);
        let by_id = |id: &str| p.attachments.iter().find(|a| a.content_id.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(id)));
        return by_id(cid)
            .or_else(|| by_id(&plain))
            .or_else(|| p.attachments.iter().find(|a| name_is(a, &plain)))
            .or_else(|| p.attachments.iter().find(|a| a.content_location.as_deref().is_some_and(|l| l.eq_ignore_ascii_case(&plain))))
            .map(|a| a.index);
    }
    let path = decoded(src.split(['?', '#']).next().unwrap_or(src));
    let file = last_part(&path);
    p.attachments
        .iter()
        .find(|a| a.content_location.as_deref().is_some_and(|l| l.eq_ignore_ascii_case(&path) || last_part(l).eq_ignore_ascii_case(&file)))
        .or_else(|| p.attachments.iter().find(|a| name_is(a, &file) || a.content_id.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(&file))))
        .map(|a| a.index)
}

/// End every CSS declaration block in `<style>` elements with a `;` (the last declaration may
/// lack one, which html2text's parser would otherwise skip with the whole rule), and every
/// `<style>` with an empty rule: html2text 0.17 panics on a style sheet that stops inside an
/// attribute selector (`[s=`), which a cut-off or hostile message can end with.
pub(super) fn close_css_rules(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len() + 64);
    let mut at = 0;
    while let Some(start) = lower.get(at..).and_then(|r| r.find("<style")).map(|i| i + at) {
        // An unclosed `<style>` runs to the end of the document.
        let end = lower.get(start..).and_then(|r| r.find("</style")).map_or(html.len(), |i| i + start);
        out.push_str(html.get(at..start).unwrap_or(""));
        out.push_str(&html.get(start..end).unwrap_or("").replace('}', ";}"));
        out.push_str("\n*{}");
        at = end;
    }
    out.push_str(html.get(at..).unwrap_or(""));
    out
}

/// `html` with a mark after every `<h1>`…`<h6>` start tag (and any mark already in it removed).
pub(super) fn mark_headings(html: &str) -> String {
    let html = html.replace(HEADING_MARK, "");
    let mut out = String::with_capacity(html.len() + 64);
    let mut last = 0;
    deep::tags(&html, |name, end, range| {
        if !end && matches!(name, "h1" | "h2" | "h3" | "h4" | "h5" | "h6") {
            out.push_str(html.get(last..range.end).unwrap_or(""));
            out.push(HEADING_MARK);
            last = range.end;
        }
        true
    });
    out.push_str(html.get(last..).unwrap_or(""));
    out
}

/// Web pictures a rich message refers to (`http:`/`https:` only), in order, without repeats.
pub fn remote_images(rich: &RichText) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for sp in rich.iter().flatten() {
        if let Some(src) = &sp.image
            && (src.starts_with("https://") || src.starts_with("http://"))
            && !out.contains(src)
        {
            out.push(src.clone());
        }
    }
    out
}

/// An HTML body's words for a preview, in reading order: each table cell after the one before
/// (raw mode: cells side by side came out interleaved line by line, "[[Picture ## Title 0]]"),
/// with no marks at all (no `[…]` around links and pictures, no `#` before titles, no `**`).
/// It is made for each message downloaded: raw mode lays out no columns.
pub(crate) fn html_to_words(html: &str) -> String {
    let html = flat_if_too_deep(html);
    let html = html.as_ref();
    let html = footers_last(html);
    let html = html.as_ref();
    let result = std::panic::catch_unwind(|| {
        html2text::config::with_decorator(html2text::render::TrivialDecorator::new()).raw_mode(true).string_from_read(html.as_bytes(), 400)
    });
    match result {
        Ok(Ok(s)) => tidy_text(&s),
        _ => String::new(),
    }
}

/// `html`, or its words and links flat when it nests deeper than any mail does
/// ([`deep::too_deep`]): the text converter's time grows with each table nested.
fn flat_if_too_deep(html: &str) -> std::borrow::Cow<'_, str> {
    if deep::too_deep(html) { std::borrow::Cow::Owned(deep::flattened(html)) } else { std::borrow::Cow::Borrowed(html) }
}

/// Whether `html` nests deeper than any mail does, and is read flat ([`deep::too_deep`]).
pub(crate) fn too_deep(html: &str) -> bool {
    deep::too_deep(html)
}

/// How much of an HTML body is read for its preview, in characters, from its `<body>`: a first
/// look, then more only when that holds too few words (the first look is what most need, and
/// reading costs in proportion to what is read, for each message downloaded).
const PREVIEW_READS: [usize; 2] = [16 * 1024, 64 * 1024];

/// HTML read for a short piece of text shown as is (a MailTip), in characters.
const TIP_HTML_MAX: usize = 16 * 1024;

/// An HTML body's preview ([`crate::preview`] of [`html_to_words`]): read from its `<body>`
/// (what comes before is styles and the like), a little first, more when that is too short;
/// without the pictures whose description only names them ([`names_itself`]: a row of spacers
/// read `spacer-FFFFFF spacer-FFFFFF…`).
pub(crate) fn html_preview(html: &str) -> String {
    // Where `<body` starts, in any case (an ASCII `<` starts a character).
    let from = html.as_bytes().windows(5).position(|w| w.eq_ignore_ascii_case(b"<body")).unwrap_or(0);
    let body = html.get(from..).unwrap_or(html);
    let mut out = String::new();
    for n in PREVIEW_READS {
        let cut = body.char_indices().nth(n).map(|(i, _)| i);
        let read = pictures::without_named_pictures(body.get(..cut.unwrap_or(body.len())).unwrap_or(body));
        out = crate::preview(&html_to_words(&read));
        if cut.is_none() || out.chars().count() >= crate::PREVIEW_CHARS {
            break;
        }
    }
    out
}

/// HTML → readable text. Never panics; returns an empty string when conversion fails.
pub fn html_to_text(html: &str) -> String {
    let html = match html.char_indices().nth(HTML_MAX) {
        Some((i, _)) => html.get(..i).unwrap_or(html),
        None => html,
    };
    let html = flat_if_too_deep(html);
    let html = html.as_ref();
    let html = footers_last(html);
    let html = html.as_ref();
    // Without table borders: mail lays itself out with tables, which came out boxed in `─` and
    // `│` rules (issue #41).
    let result = std::panic::catch_unwind(|| html2text::config::plain().no_table_borders().string_from_read(html.as_bytes(), 400));
    match result {
        Ok(Ok(s)) => tidy_text(&s),
        _ => String::new(),
    }
}

/// HTML → words for a short piece of text shown as is (a MailTip's automatic reply): no
/// Markdown decoration (`**` around bold words), no links as footnotes, bounded. Never panics;
/// returns an empty string when conversion fails.
pub fn html_to_plain(html: &str) -> String {
    let html = match html.char_indices().nth(TIP_HTML_MAX) {
        Some((i, _)) => html.get(..i).unwrap_or(html),
        None => html,
    };
    let html = flat_if_too_deep(html);
    let html = html.as_ref();
    let result = std::panic::catch_unwind(|| html2text::config::plain_no_decorate().string_from_read(html.as_bytes(), 400));
    match result {
        Ok(Ok(s)) => tidy_text(&s),
        _ => String::new(),
    }
}

/// The web addresses an HTML body links to (`href`), once each and in order: whole, however
/// long, where [`html_to_text`] keeps them as footnotes it wraps at its line width (a long Teams
/// join link came out cut). Only `http` and `https`; bounded in what it reads and returns.
pub fn html_links(html: &str) -> Vec<String> {
    let html = match html.char_indices().nth(HTML_MAX) {
        Some((i, _)) => html.get(..i).unwrap_or(html),
        None => html,
    };
    let lower = html.to_ascii_lowercase();
    let mut out: Vec<String> = Vec::new();
    let mut at = 0;
    while out.len() < 50 {
        let Some(rel) = lower.get(at..).and_then(|r| r.find("href")) else { break };
        let mut i = at + rel + 4;
        at = i;
        let rest = html.get(i..).unwrap_or("");
        let trimmed = rest.trim_start();
        let Some(after_eq) = trimmed.strip_prefix('=') else { continue };
        i += rest.len() - trimmed.len() + 1;
        let value = after_eq.trim_start();
        i += after_eq.len() - value.len();
        let (quote, body) = match value.chars().next() {
            Some(q @ ('"' | '\'')) => (Some(q), value.get(1..).unwrap_or("")),
            _ => (None, value),
        };
        let end = match quote {
            Some(q) => body.find(q),
            None => body.find(|c: char| c.is_whitespace() || c == '>'),
        }
        .unwrap_or(body.len());
        at = i + end.max(1);
        let raw = body.get(..end).unwrap_or("").trim();
        if raw.len() > 8192 {
            continue;
        }
        let url = raw.replace("&amp;", "&").replace("&#38;", "&").replace("&quot;", "\"").replace("&#39;", "'");
        let lower_url = url.to_ascii_lowercase();
        if (lower_url.starts_with("https://") || lower_url.starts_with("http://")) && !url.contains(char::is_whitespace) && !out.contains(&url) {
            out.push(url);
        }
    }
    out
}

#[cfg(test)]
mod tests;

/// Trim trailing spaces and collapse runs of blank lines.
pub(super) fn tidy_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blank = 0;
    for line in s.lines() {
        let l = line.trim_end();
        if l.is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(l);
        out.push('\n');
    }
    out.trim_matches('\n').to_string()
}
