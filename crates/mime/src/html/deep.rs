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

/// Elements whose content a browser reads as text, not markup, up to their own end tag (a
/// browser runs scripts: what `<noscript>` holds is text too).
const RAW: &[&str] = &["script", "style", "textarea", "title", "xmp", "iframe", "noembed", "noframes", "noscript"];

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
                    html.get(at..i).unwrap_or_default()
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

/// The tags of `html` in order, as a browser's tokenizer finds them: outside comments (`<!-- -->`,
/// and `<!…>`, `<?…>` read as one to their `>`), the text of the elements of [`RAW`], and all that
/// follows a `<plaintext>`; a tag ends at its `>` past quoted values, and one left open at the end
/// isn't one. Each as (its name, lowercase; whether it ends an element; where it is) until `each`
/// says no more: the one pass [`too_deep`], [`flattened`], the pictures (`pictures.rs`), the
/// headings and the look for a code block of Markdown on the clipboard (`markdown.rs`) make,
/// linear in `html`.
pub(crate) fn tags(html: &str, mut each: impl FnMut(&str, bool, std::ops::Range<usize>) -> bool) {
    let bytes = html.as_bytes();
    let mut at = 0;
    let mut raw: Option<&'static str> = None;
    // Where a comment, or anything else read as one, ends (past its `>`).
    let past = |from: usize, end: &str| html.get(from..).and_then(|s| s.find(end)).map(|e| from + e + end.len());
    while let Some(lt) = bytes.get(at..).and_then(|b| b.iter().position(|&c| c == b'<')).map(|p| p + at) {
        let end = bytes.get(lt + 1) == Some(&b'/');
        let start = lt + 1 + usize::from(end);
        if let Some(r) = raw {
            // Inside raw text only its own end tag counts.
            let own = end
                && bytes.get(start..start + r.len()).is_some_and(|n| n.eq_ignore_ascii_case(r.as_bytes()))
                && bytes.get(start + r.len()).is_none_or(|&c| is_space(c) || matches!(c, b'/' | b'>'));
            if !own {
                at = lt + 1;
                continue;
            }
        } else if bytes.get(lt..lt + 4) == Some(b"<!--") {
            // `<!-->` and `<!--->` end where they start; the others at `-->` (or `--!>`).
            let short = [b"<!-->".as_slice(), b"<!--->".as_slice()].into_iter().find(|s| bytes.get(lt..lt + s.len()) == Some(*s));
            let long = || [past(lt + 4, "-->"), past(lt + 4, "--!>")].into_iter().flatten().min();
            let Some(next) = short.map(|s| lt + s.len()).or_else(long) else { return };
            at = next;
            continue;
        } else {
            let named = |i: usize| bytes.get(i).is_some_and(u8::is_ascii_alphabetic);
            if matches!(bytes.get(lt + 1), Some(b'!' | b'?')) || (end && bytes.get(lt + 2).is_some() && !named(lt + 2)) {
                // `<!doctype …>`, `<?xml …>`, `</ x>`: a comment to the first `>` (`</>`: nothing).
                let Some(next) = past(lt + 2, ">") else { return };
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
                each(&name, end, lt..next);
                return;
            }
            raw = RAW.iter().copied().find(|r| *r == name);
        }
        if !each(&name, end, lt..next) {
            return;
        }
        at = next;
    }
}

/// Whether `html` nests tables, or boxes, past what any mail does.
pub(crate) fn too_deep(html: &str) -> bool {
    // Too few to nest that deep: looked at no further.
    let lower = html.as_bytes();
    let count = |needle: &[u8]| lower.windows(needle.len()).filter(|w| w.eq_ignore_ascii_case(needle)).count();
    if count(b"<table") <= MAX_TABLES && count(b"<div") + count(b"<blockquote") <= MAX_BOXES {
        return false;
    }
    let (mut tables, mut boxes, mut deep) = (0usize, 0usize, false);
    tags(html, |name, end, _| {
        let depth = match name {
            "table" => &mut tables,
            "div" | "blockquote" => &mut boxes,
            _ => return true,
        };
        *depth = if end { depth.saturating_sub(1) } else { depth.saturating_add(1) };
        deep = tables > MAX_TABLES || boxes > MAX_BOXES;
        !deep
    });
    deep
}

/// `html` with nothing nested: each block's tags a line break, links, pictures and what marks
/// words (bold, italic…) as they were, everything else left out but its text.
pub(crate) fn flattened(html: &str) -> String {
    const KEPT: &[&str] = &["a", "b", "strong", "i", "em", "u", "s", "strike", "code", "img", "br", "head", "style", "script", "title"];
    const BLOCKS: &[&str] =
        &["p", "div", "table", "tr", "li", "ul", "ol", "h1", "h2", "h3", "h4", "h5", "h6", "blockquote", "pre", "hr", "dl", "dt", "dd", "center"];
    let mut out = String::with_capacity(html.len() / 2);
    let mut at = 0;
    tags(html, |name, _, range| {
        out.push_str(html.get(at..range.start).unwrap_or_default());
        if KEPT.contains(&name) {
            out.push_str(html.get(range.clone()).unwrap_or_default());
        } else if BLOCKS.contains(&name) {
            out.push_str("<br>");
        } else if matches!(name, "td" | "th") {
            out.push(' ');
        }
        at = range.end;
        true
    });
    out.push_str(html.get(at..).unwrap_or_default());
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
        // Tags in a comment, a style sheet or a script, or a `>` in a value, nest nothing.
        let quiet = format!("<!--{}--><style>{}</style><a title=\"{}\">x</a>", "<table>".repeat(100), "<table>".repeat(100), "<table>".repeat(100));
        assert!(!too_deep(&quiet));
    }

    #[test]
    fn flat_html_keeps_words_links_and_lines() {
        let flat =
            flattened(r#"<table><tr><td>One<td><b>Two</b></tr><tr><td><a href="h">Three</a></table><!-- <p>no --><script>if (a<b) x()</script>"#);
        assert_eq!(flat, r#"<br><br> One <b>Two</b><br><br> <a href="h">Three</a><br><!-- <p>no --><script>if (a<b) x()</script>"#);
    }
}
