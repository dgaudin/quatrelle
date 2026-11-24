// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
//! HTML nested deeper than any mail is (tables left open one in another by the thousand, say):
//! read as its words and links, flat, rather than through the text converter and the layout,
//! whose time grows with each table nested (a megabyte took over ten minutes). Mail as written,
//! newsletters with a dozen tables one in another included, is nowhere near the limits.

/// Tables open one in another past which HTML is read flat.
const MAX_TABLES: usize = 48;
/// Boxes (`div`, `blockquote`) open one in another past which HTML is read flat.
const MAX_BOXES: usize = 2048;

/// Elements whose content a browser reads as text, not markup, up to their own end tag. Not
/// `<noscript>`: the text converter and the layout read HTML without scripts, and what it holds
/// is markup for them (tables nested in it went past the guard against nesting).
const RAW: &[&str] = &["script", "style", "textarea", "title", "xmp", "iframe", "noembed", "noframes"];

#[cfg(test)]
thread_local! {
    /// The bytes looked through for the ends of comments (and of what's read as one), on this
    /// test thread.
    static SEARCHED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// White space between a tag's parts, as HTML has it.
fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0C)
}

/// The attributes of a tag, from `from` (after its name) in `html`, as a browser's tokenizer reads
/// them, each given to `each` as (its name, as written; its value, as written: in its quotes,
/// `""` for a name alone) until `each` says no more. A value in quotes runs to its closing quote,
/// `>` and all; a quote elsewhere (`a"b`) is a character like any. Where the tag ends (its `>`);
/// `None` when it doesn't (a browser drops a tag left open at the end).
pub(super) fn attributes<'a>(html: &'a str, from: usize, mut each: impl FnMut(&'a str, &'a str) -> bool) -> Option<usize> {
    let bytes = html.as_bytes();
    let spaces = |mut i: usize, slash: bool| {
        while bytes.get(i).is_some_and(|&c| is_space(c) || (slash && c == b'/')) {
            i += 1;
        }
        i
    };
    let mut i = from;
    let mut asking = true;
    loop {
        // Before a name: white space, and a `/` that doesn't end the tag.
        i = spaces(i, true);
        if *bytes.get(i)? == b'>' {
            return Some(i);
        }
        // Its name: its first character whatever it is (an `=` too), then up to a space, `/`,
        // `>` or `=`.
        let name_at = i;
        i += 1;
        while bytes.get(i).is_some_and(|&c| !is_space(c) && !matches!(c, b'/' | b'>' | b'=')) {
            i += 1;
        }
        let name = html.get(name_at..i).unwrap_or_default();
        i = spaces(i, false);
        let mut open = false;
        let value = if bytes.get(i) == Some(&b'=') {
            i = spaces(i + 1, false);
            match bytes.get(i).copied() {
                Some(q @ (b'"' | b'\'')) => {
                    let close = bytes.get(i + 1..).and_then(|r| r.iter().position(|&c| c == q)).map(|p| i + 1 + p);
                    let v = html.get(i + 1..close.unwrap_or(bytes.len())).unwrap_or_default();
                    // Left open: to the end (and the tag doesn't end).
                    open = close.is_none();
                    i = close.map_or(bytes.len(), |c| c + 1);
                    v
                }
                _ => {
                    let at = i;
                    while bytes.get(i).is_some_and(|&c| !is_space(c) && c != b'>') {
                        i += 1;
                    }
                    let bare = html.get(at..i).unwrap_or_default();
                    // Unquoted, as programs write it (`src=https://…/c.png/>`): a slash before
                    // the `>` is the tag's, not the address's (a `//` stays: a protocol-relative
                    // address starts with it).
                    bare.strip_suffix('/').filter(|b| !b.ends_with('/')).unwrap_or(bare)
                }
            }
        } else {
            ""
        };
        if asking {
            asking = each(name, value);
        }
        if open {
            return None;
        }
    }
}

/// What [`read`] finds in HTML, in order.
pub(super) enum Found<'n> {
    /// A tag: its name (lowercase), whether it ends an element, where it is.
    Tag(&'n str, bool, std::ops::Range<usize>),
    /// A comment, or what's read as one (`<!doctype …>`, `<?xml …>`, `</ x>`, and a CDATA
    /// section, to its first `>` as outside an SVG picture): where it is.
    Comment(std::ops::Range<usize>),
}

/// Where a comment whose text starts at `from` ends (past its `-->` or `--!>`), each byte looked
/// at once; `None` when it runs to the end. (Looking for `-->`, then for `--!>`, ran to the end of
/// the document for each comment when there was no `--!>` in it.)
fn comment_end(html: &str, from: usize) -> Option<usize> {
    let bytes = html.as_bytes();
    let mut i = from;
    let end = loop {
        let Some(p) = html.get(i..).and_then(|r| r.find("--")).map(|p| p + i) else { break None };
        match (bytes.get(p + 2), bytes.get(p + 3)) {
            (Some(b'>'), _) => break Some(p + 3),
            (Some(b'!'), Some(b'>')) => break Some(p + 4),
            _ => i = p + 1,
        }
    };
    #[cfg(test)]
    SEARCHED.with(|n| n.set(n.get() + end.unwrap_or(html.len()).saturating_sub(from)));
    end
}

/// How far into a script's text it is, as a browser reads it: `<!--` in it hides its end until
/// `-->`, but for one after a `<script>` written in that comment (which ends at its own
/// `</script>`).
#[derive(Clone, Copy, PartialEq)]
enum Script {
    Text,
    Commented,
    Twice,
}

/// The tags and comments of `html` in order, as a browser's tokenizer finds them
/// (without scripts, as the text converter and the layout read it): the text of the elements of
/// [`RAW`] and all that follows a `<plaintext>` hold none; a tag ends at its `>` past quoted
/// values, and one left open at the end isn't one (nor is anything after it). Given to `each`
/// until it says no more; each byte looked at a few times at most.
pub(super) fn read(html: &str, mut each: impl FnMut(Found<'_>) -> bool) {
    let bytes = html.as_bytes();
    let mut at = 0;
    let mut raw: Option<&'static str> = None;
    let mut script = Script::Text;
    // Where `-->` is looked for in a script's comment (between one `<` and the next).
    let mut since = 0;
    // Where what's read as a comment ends (past its `>`).
    let past = |from: usize, end: &str| {
        let found = html.get(from..).and_then(|s| s.find(end));
        #[cfg(test)]
        SEARCHED.with(|n| n.set(n.get() + found.map_or(html.len().saturating_sub(from), |e| e + end.len())));
        found.map(|e| from + e + end.len())
    };
    // Whether `name` (lowercase) is written at `i`, an element's name ending there.
    let named_at = |i: usize, name: &str| {
        bytes.get(i..i + name.len()).is_some_and(|n| n.eq_ignore_ascii_case(name.as_bytes()))
            && bytes.get(i + name.len()).is_none_or(|&c| is_space(c) || matches!(c, b'/' | b'>'))
    };
    while let Some(lt) = bytes.get(at..).and_then(|b| b.iter().position(|&c| c == b'<')).map(|p| p + at) {
        let end = bytes.get(lt + 1) == Some(&b'/');
        let start = lt + 1 + usize::from(end);
        if let Some(r) = raw {
            if r == "script" {
                if script != Script::Text && html.get(since..lt).is_some_and(|t| t.contains("-->")) {
                    script = Script::Text;
                }
                since = lt + 1;
                let script_tag = named_at(start, "script");
                // Its end, but in a `<script>` written in its comment (that one's own).
                let ends = end && script_tag && script != Script::Twice;
                script = match script {
                    Script::Text if bytes.get(lt..lt + 4) == Some(b"<!--") => {
                        // `<!-->` and `<!--->` end at once: `-->` is looked for from the `--`.
                        since = lt + 2;
                        Script::Commented
                    }
                    Script::Commented if !end && script_tag => Script::Twice,
                    Script::Twice if end && script_tag => Script::Commented,
                    s => s,
                };
                if !ends {
                    at = lt + 1;
                    continue;
                }
            } else if !(end && named_at(start, r)) {
                // Inside raw text only its own end tag counts.
                at = lt + 1;
                continue;
            }
        } else if bytes.get(lt..lt + 4) == Some(b"<!--") {
            // `<!-->` and `<!--->` end where they start; the others at `-->` (or `--!>`).
            let short = [b"<!-->".as_slice(), b"<!--->".as_slice()].into_iter().find(|s| bytes.get(lt..lt + s.len()) == Some(*s));
            let Some(next) = short.map(|s| lt + s.len()).or_else(|| comment_end(html, lt + 4)) else {
                each(Found::Comment(lt..html.len()));
                return;
            };
            if !each(Found::Comment(lt..next)) {
                return;
            }
            at = next;
            continue;
        } else {
            let named = |i: usize| bytes.get(i).is_some_and(u8::is_ascii_alphabetic);
            if matches!(bytes.get(lt + 1), Some(b'!' | b'?')) || (end && bytes.get(lt + 2).is_some() && !named(lt + 2)) {
                // `<!doctype …>`, `<?xml …>`, `</ x>`: a comment to the first `>` (`</>`: nothing).
                let Some(next) = past(lt + 2, ">") else {
                    each(Found::Comment(lt..html.len()));
                    return;
                };
                if !each(Found::Comment(lt..next)) {
                    return;
                }
                at = next;
                continue;
            }
            if !named(start) {
                // A `<` in the text.
                at = lt + 1;
                continue;
            }
        }
        let name_len = bytes.get(start..).map_or(0, |b| b.iter().take_while(|&&c| !is_space(c) && c != b'/' && c != b'>').count());
        // Left open at the end: not a tag, and nothing after it is one.
        let Some(close) = attributes(html, start + name_len, |_, _| false) else { return };
        let next = close + 1;
        let name = html.get(start..start + name_len).unwrap_or_default().to_ascii_lowercase();
        if raw.is_some() {
            // Its own end tag.
            raw = None;
        } else if !end {
            if name == "plaintext" {
                // All that follows is text.
                each(Found::Tag(&name, end, lt..next));
                return;
            }
            raw = RAW.iter().copied().find(|r| *r == name);
            script = Script::Text;
        }
        if !each(Found::Tag(&name, end, lt..next)) {
            return;
        }
        at = next;
    }
}

/// The tags of `html` in order ([`read`]), each as (its name, lowercase; whether it ends an
/// element; where it is) until `each` says no more: what the pictures (`pictures.rs`), the
/// headings and the look for a code block of Markdown on the clipboard (`markdown.rs`) read.
pub(crate) fn tags(html: &str, mut each: impl FnMut(&str, bool, std::ops::Range<usize>) -> bool) {
    read(html, |found| match found {
        Found::Tag(name, end, range) => each(name, end, range),
        Found::Comment(_) => true,
    });
}

/// How deep tables and boxes are, as [`too_deep`] counts them.
#[derive(Default)]
struct Depth {
    tables: usize,
    boxes: usize,
    /// The boxes open when each table open was (at most [`MAX_TABLES`] of them, past which it's
    /// deep): an end in a table closes only a box opened in it, and the table's end closes those.
    floors: Vec<usize>,
    deep: bool,
}

impl Depth {
    /// The tables and boxes `text` opens, wherever it does (in a script, a style sheet, a title,
    /// a value: when in doubt, opened).
    fn opened(&mut self, text: &[u8]) {
        let mut at = 0;
        while let Some(lt) = text.get(at..).and_then(|b| b.iter().position(|&c| c == b'<')).map(|p| p + at) {
            let is = |name: &[u8]| text.get(lt + 1..lt + 1 + name.len()).is_some_and(|n| n.eq_ignore_ascii_case(name));
            if is(b"table") {
                self.tables = self.tables.saturating_add(1);
                if self.floors.len() <= MAX_TABLES {
                    self.floors.push(self.boxes);
                }
            } else if is(b"div") || is(b"blockquote") {
                self.boxes = self.boxes.saturating_add(1);
            }
            self.deep |= self.tables > MAX_TABLES || self.boxes > MAX_BOXES;
            at = lt + 1;
        }
    }

    /// An end a browser surely reads: a box's closes one opened in the table it's in, not one
    /// opened before it (a cell bounds what an end in it can close: `<div><table><tr><td></div>`
    /// left the box open, and a box more was nested each time); a table's closes the boxes opened
    /// in it.
    fn closed(&mut self, name: &str) {
        let floor = self.floors.last().copied().unwrap_or(0);
        match name {
            "table" => {
                self.tables = self.tables.saturating_sub(1);
                if let Some(floor) = self.floors.pop() {
                    self.boxes = self.boxes.min(floor);
                }
            }
            "div" | "blockquote" if self.boxes > floor => self.boxes -= 1,
            _ => {}
        }
    }
}

/// Elements around which the text converter's tree is no longer the tokenizer's alone: an SVG or
/// MathML picture (its `<title>` and `<style>` are markup, its CDATA sections run to `]]>`), a
/// template (which ignores the ends in it), and those past which a `</div>` isn't one. (A CDATA
/// section outside a picture is read here as they read it, to its first `>`.)
const DISAGREE: &[&[u8]] = &[b"<svg", b"<math", b"<template", b"<select", b"<object", b"<applet", b"<marquee"];

/// Whether `html` nests tables, or boxes, past what any mail does, counted as no reader of it can
/// nest deeper: every table or box opened counts, wherever it's written but in a comment (in a
/// `<noscript>`, which the text converter reads as markup; in a script, a style sheet, a title,
/// a value), and only the ends a browser surely reads close one (not one in a script's comment).
/// With any of [`DISAGREE`] in it, an end or a comment seen here can be none for the text
/// converter: every table or box opened counts then, comments too, and no end. Counting too many
/// only reads a harmless message flat.
pub(crate) fn too_deep(html: &str) -> bool {
    // Too few to nest that deep: looked at no further.
    let bytes = html.as_bytes();
    let count = |needle: &[u8]| bytes.windows(needle.len()).filter(|w| w.eq_ignore_ascii_case(needle)).count();
    if count(b"<table") <= MAX_TABLES && count(b"<div") + count(b"<blockquote") <= MAX_BOXES {
        return false;
    }
    if DISAGREE.iter().any(|tag| bytes.windows(tag.len()).any(|w| w.eq_ignore_ascii_case(tag))) {
        return true;
    }
    let mut depth = Depth::default();
    // Up to where the tables and boxes opened are counted.
    let mut counted = 0;
    read(html, |found| {
        match found {
            Found::Comment(range) => {
                depth.opened(bytes.get(counted..range.start).unwrap_or_default());
                counted = range.end;
            }
            Found::Tag(name, true, range) if matches!(name, "table" | "div" | "blockquote") => {
                depth.opened(bytes.get(counted..range.start).unwrap_or_default());
                counted = range.start;
                depth.closed(name);
            }
            _ => {}
        }
        !depth.deep
    });
    if !depth.deep {
        depth.opened(bytes.get(counted.min(bytes.len())..).unwrap_or_default());
    }
    depth.deep
}

/// `html` with nothing nested: each block's tags a line break, links, pictures and what marks
/// words (bold, italic…) as they were, everything else left out but its text. The elements whose
/// text is copied as it is (a style sheet, a script, a `<textarea>`…) keep their tags, for it to
/// be read as text again.
pub(crate) fn flattened(html: &str) -> String {
    const KEPT: &[&str] = &[
        "a",
        "b",
        "strong",
        "i",
        "em",
        "u",
        "s",
        "strike",
        "code",
        "img",
        "br",
        "head",
        "style",
        "script",
        "title",
        "textarea",
        "xmp",
        "iframe",
        "noembed",
        "noframes",
        "plaintext",
    ];
    const BLOCKS: &[&str] =
        &["p", "div", "table", "tr", "li", "ul", "ol", "h1", "h2", "h3", "h4", "h5", "h6", "blockquote", "pre", "hr", "dl", "dt", "dd", "center"];
    let mut out = String::with_capacity(html.len() / 2);
    let mut at = 0;
    read(html, |found| {
        let range = match &found {
            Found::Tag(_, _, r) | Found::Comment(r) => r.clone(),
        };
        out.push_str(html.get(at..range.start).unwrap_or_default());
        match found {
            Found::Tag(name, ..) if KEPT.contains(&name) => out.push_str(html.get(range.clone()).unwrap_or_default()),
            Found::Tag(name, ..) if BLOCKS.contains(&name) => out.push_str("<br>"),
            Found::Tag("td" | "th", ..) => out.push(' '),
            Found::Tag(..) => {}
            Found::Comment(_) => out.push_str(html.get(range.clone()).unwrap_or_default()),
        }
        at = range.end;
        true
    });
    out.push_str(html.get(at.min(html.len())..).unwrap_or_default());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Found in review: tables left open one in another (a cell, a box, a link, again and
    /// again) took seconds to read at 64 KB, and over ten minutes at a megabyte, in the text
    /// converter and the layout. Read flat, they take no time, their words and links kept; mail
    /// that nests as mail does is read as before.
    #[test]
    fn tables_left_open_by_the_thousand_are_read_flat_and_quick() {
        let unit = r#"<table><tr><td><div><a href="https://x.example/a">link</a>"#;
        let html = unit.repeat(1_000_000 / unit.len());
        assert!(too_deep(&html));
        let started = std::time::Instant::now();
        let p = crate::parse(format!("Content-Type: text/html\r\n\r\n{html}").as_bytes());
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "{:?}", started.elapsed());
        let p = p.unwrap();
        assert!(p.text.contains("link") && p.text.contains("https://x.example/a"), "{:?}", &p.text[..p.text.len().min(200)]);
        assert!(p.rich.iter().flatten().any(|s| s.link.as_deref() == Some("https://x.example/a")));
        // As mail nests: as before.
        let nested = format!("{}<p>Deep</p>{}", "<table><tr><td>".repeat(MAX_TABLES), "</td></tr></table>".repeat(MAX_TABLES));
        assert!(!too_deep(&nested));
        assert!(!too_deep(&"<div>".repeat(MAX_BOXES)));
        assert!(too_deep(&"<div>".repeat(MAX_BOXES + 1)));
        // Tags in a comment nest nothing; in a style sheet or a value they're counted, when in doubt.
        assert!(!too_deep(&format!("<!--{}--><p>x</p>", "<table>".repeat(100))));
        assert!(too_deep(&format!("<style>{}</style>", "<table>".repeat(100))));
        assert!(too_deep(&format!("<a title=\"{}\">x</a>", "<table>".repeat(100))));
    }

    /// Found in review: tables nested by the thousand after an element whose content this reading
    /// takes for text, where the text converter, the layout or a browser read markup (a
    /// `<noscript>`, read without scripts; a `<title>` or `<style>` in an SVG picture; a script,
    /// a style sheet or a title never closed), went unseen, and took minutes to read. The guard
    /// counts every table opened, in any text; only the ends a browser surely reads close one:
    /// not one in a comment, in a CDATA section, or in a script that hides its end in a comment.
    #[test]
    fn tables_after_text_that_may_be_markup_are_too_deep() {
        let tables = "<table><tr><td><div>".repeat(200);
        for before in ["<noscript>", "<svg><title>", "<svg><style>", "<math><style>", "<script>", "<style>", "<title>", "<textarea>", "<xmp>"] {
            assert!(too_deep(&format!("<p>x</p>{before}{tables}")), "{before}");
        }
        for unit in [
            "<table><tr><td><!--</td></tr></table>-->",
            "<table><tr><td><svg><![CDATA[ > </td></tr></table> ]]></svg>",
            "<table><tr><td><script><!--<script></script></td></tr></table></script>",
        ] {
            assert!(too_deep(&unit.repeat(100)), "{unit}");
        }
        // Tables closed as they're opened, by the hundred: not deep, Outlook's conditional
        // comments around them too.
        assert!(!too_deep(&"<table><tr><td>x</td></tr></table>".repeat(500)));
        assert!(!too_deep(&"<!--[if mso]><table><tr><td><![endif]--><div>x</div><!--[if mso]></td></tr></table><![endif]-->".repeat(500)));
    }

    /// Found in review: where the text converter's tree isn't the tokenizer's alone, an end or a
    /// comment this reading sees can be none for it, and tables nested past them went unseen: a
    /// CDATA section outside an SVG picture, which ends at its first `>` (T1-1); an SVG picture's
    /// `<title>`, which is markup there (T1-2); a `</table>` in a `<template>`, which is ignored
    /// (T1-3); a `</div>` after an `<object>`, a `<marquee>`, an `<applet>` or in a `<select>`
    /// (T1-4). With any of these in a message, every table or box opened counts, and no end.
    #[test]
    fn tables_where_the_readers_may_disagree_are_counted_whole() {
        let tables = "<table><tr><td>".repeat(200);
        let cases = [
            ("T1-1", format!("<![CDATA[ > <style> ]]><!--</style>{tables}")),
            ("T1-2", format!("<svg><title><style></title><!--</style>{tables}")),
            ("T1-3", "<table><tr><td><template></table></template>".repeat(200)),
            ("T1-4 object", "<div><object></div>".repeat(2100)),
            ("T1-4 marquee", "<div><marquee></div>".repeat(2100)),
            ("T1-4 applet", "<div><applet></div>".repeat(2100)),
            ("T1-4 select", "<div><select></div>".repeat(2100)),
        ];
        let unseen: Vec<&str> = cases.iter().filter(|(_, html)| !too_deep(&format!("<p>x</p>{html}"))).map(|(what, _)| *what).collect();
        assert!(unseen.is_empty(), "nesting not seen: {unseen:?}");
    }

    /// Found in review: a CDATA section anywhere had every table opened counted, ends aside: one
    /// in a script or a style sheet (read as the readers read it) flattened a letter of a hundred
    /// tables for nothing.
    #[test]
    fn a_cdata_section_in_a_script_flattens_nothing() {
        let closed = "<table><tr><td>x</td></tr></table>".repeat(100);
        for cdata in ["<script>//<![CDATA[\nx()\n//]]></script>", "<style>/*<![CDATA[*/ p{} /*]]>*/</style>"] {
            assert!(!too_deep(&format!("{cdata}{closed}")), "{cdata}");
        }
    }

    /// Found in review: a `</div>` in a table's cell, for a box opened before the table, is none
    /// for the text converter (the cell bounds what it can close), but the guard took it for one:
    /// a box more nested each time, the 2048 passed unseen (T2-1). An end closes only a box opened
    /// in the table it's in; a table's end closes those.
    #[test]
    fn boxes_left_open_across_a_cell_are_counted() {
        let forms = [
            "<div><table><tr><td></div></table>",
            "<div><table><caption></div></table>",
            "<div><table><tr><th></div></table>",
            "<blockquote><table><tr><td></blockquote></table>",
        ];
        let unseen: Vec<&str> = forms.into_iter().filter(|unit| !too_deep(&unit.repeat(2100))).collect();
        assert!(unseen.is_empty(), "nesting not seen: {unseen:?}");
        // Boxes closed in their cells, tables closed in turn: not deep.
        assert!(!too_deep(&"<div><table><tr><td><div>x</div></td></tr></table></div>".repeat(2100)));
        assert!(!too_deep(&"<table><tr><td><div>x</div></td></tr></table><div>y</div>".repeat(2100)));
    }

    /// Found in review: the end of each comment was looked for twice, `-->` and `--!>`, and the
    /// second ran to the end of the document when there was none: 4 MB of `<!---->` had some
    /// 10^12 bytes read, in each reading of a message. The document is looked through once.
    #[test]
    fn comments_by_the_hundred_thousand_are_looked_through_once() {
        for size in [400 << 10, 4 << 20] {
            let html = format!("{}<img src=\"https://t.example/o.gif\">", "<!---->".repeat(size / 7));
            let before = SEARCHED.with(std::cell::Cell::get);
            assert_eq!(crate::html_images(&html), ["https://t.example/o.gif"]);
            let read = SEARCHED.with(std::cell::Cell::get) - before;
            assert!(read <= 2 * html.len(), "{read} bytes looked through for {} bytes of comments", html.len());
        }
    }

    /// … and read flat, what may be markup for the text converter isn't nested any more: the
    /// text of an element kept as text again keeps its tags.
    #[test]
    fn text_that_may_be_markup_is_read_flat_and_quick() {
        let tables = r#"<table><tr><td><div><a href="https://x.example/a">link</a>"#.repeat(200_000 / 60);
        for before in [
            "<noscript>",
            "<svg><title>",
            "<svg><style>",
            "<script>",
            "<textarea>",
            "<xmp>",
            "<svg><![CDATA[ > ",
            "<plaintext>",
            "<![CDATA[ > <style> ]]><!--</style>",
            "<svg><title><style></title><!--</style>",
            "<table><tr><td><template></table></template>",
        ] {
            let html = format!("<p>Hello</p>{before}{tables}");
            assert!(too_deep(&html), "{before}");
            let started = std::time::Instant::now();
            let p = crate::parse(format!("Content-Type: text/html\r\n\r\n{html}").as_bytes());
            assert!(started.elapsed() < std::time::Duration::from_secs(5), "{before}: {:?}", started.elapsed());
            assert!(p.is_some_and(|p| p.text.contains("Hello")), "{before}");
        }
    }

    /// `<` with no name after it, and `<a` inside a `style` left open, were each read to the
    /// end of the message, a megabyte of them for minutes.
    #[test]
    fn a_megabyte_of_stray_angle_brackets_is_read_once() {
        let tables = "<table></table>".repeat(MAX_TABLES + 1);
        let started = std::time::Instant::now();
        for junk in ["<".repeat(1 << 20), "< ".repeat(1 << 19), format!("<style>{}", "<a ".repeat(1 << 18)), "<\"".repeat(1 << 19)] {
            let html = format!("{tables}{junk}");
            assert!(!too_deep(&html));
            let _ = flattened(&html);
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(20), "{:?}", started.elapsed());
        // What the tags are is as before.
        let mut seen = Vec::new();
        tags("<p>a < b <i>c</i><style>x<y</style><script>if (a</b>) {}</script><b>", |name, end, _| {
            seen.push(format!("{}{name}", if end { "/" } else { "" }));
            true
        });
        assert_eq!(seen, ["p", "i", "/i", "style", "/style", "script", "/script", "b"]);
    }

    #[test]
    fn flat_html_keeps_words_links_and_lines() {
        let flat =
            flattened(r#"<table><tr><td>One<td><b>Two</b></tr><tr><td><a href="h">Three</a></table><!-- <p>no --><script>if (a<b) x()</script>"#);
        assert_eq!(flat, r#"<br><br> One <b>Two</b><br><br> <a href="h">Three</a><br><!-- <p>no --><script>if (a<b) x()</script>"#);
    }
}
