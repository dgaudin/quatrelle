// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Tests of Markdown pasted into a message: told from ordinary text, read into HTML within its
//! caps, then into the editor's text and styles.

use super::*;
use crate::rich::List;

/// One sign of Markdown is ordinary text (a dash in a sentence, a single list line, bold twice);
/// two different ones are Markdown.
#[test]
fn markdown_is_told_by_two_different_signs() {
    for plain in [
        "Call me - tomorrow at 10.",
        "- one item only",
        "Use **bold** and **more bold**.",
        "> quoted\n> more of it",
        "| a | b |\n| c | d |\nno rule under the first row, and `code`",
        "Price: 5 * 3 = 15, see [the notes]",
        "#hashtag and 1.5 litres",
        "",
    ] {
        assert!(!looks_like_markdown(plain), "{plain:?}");
    }
    for md in [
        "# Title\n\n- a\n- b",
        "Use **bold** and `code`",
        "```\nlet x = 1;\n```\n1. first",
        "| a | b |\n|---|---|\n| 1 | 2 |\n\nSee [the site](https://contoso.example)",
        "> quoted\n\n__strong__ words",
        "* star item\n## Second level",
    ] {
        assert!(looks_like_markdown(md), "{md:?}");
    }
    // Past the cap, only its first part is looked at.
    let late = format!("{}\n# Title\n- item", "words ".repeat(MARKDOWN_MAX / 6 + 10));
    assert!(!looks_like_markdown(&late));
}

/// Each construct becomes its HTML: headings, emphasis, code, lists, quotes, links; a table one
/// line a row.
#[test]
fn markdown_becomes_html() {
    let h = markdown_html("# Title\n\n## Part");
    assert!(h.contains("<h1>Title</h1>") && h.contains("<h2>Part</h2>"), "{h}");
    let h = markdown_html("**b** *i* ~~s~~ `c < d`");
    assert!(h.contains("<strong>b</strong>") && h.contains("<em>i</em>") && h.contains("<del>s</del>"), "{h}");
    assert!(h.contains("<code>c &lt; d</code>"), "{h}");
    let h = markdown_html("```rust\nif x < y {}\n```");
    assert!(h.contains("<pre><code>if x &lt; y {}"), "{h}");
    let h = markdown_html("- a\n- b\n\n3. three\n4. four");
    assert!(h.contains("<ul>") && h.contains("<li>a</li>") && h.contains("<ol start=\"3\">") && h.contains("<li>four</li>"), "{h}");
    let h = markdown_html("> said");
    assert!(h.contains("<blockquote>") && h.contains("said"), "{h}");
    let h = markdown_html("[site](https://contoso.example/a?b=1&c=2) and [mail](mailto:ann@contoso.example)");
    assert!(h.contains("<a href=\"https://contoso.example/a?b=1&amp;c=2\">site</a>"), "{h}");
    assert!(h.contains("<a href=\"mailto:ann@contoso.example\">mail</a>"), "{h}");
    let h = markdown_html("| A | B |\n|---|---|\n| 1 | 2 |");
    assert!(!h.contains("<table"), "{h}");
    assert!(h.contains("<strong>A</strong> · <strong>B</strong>") && h.contains("1 · 2"), "{h}");
}

/// What a stranger's text could slip in: raw HTML is dropped, a picture is its description and
/// nothing is fetched, a link to anything but the web or mail is its text alone.
#[test]
fn markdown_html_keeps_nothing_hostile() {
    let h = markdown_html("<script>alert(1)</script>\n\ntext <b onclick=\"x()\">bold</b> <img src=\"https://contoso.example/p.png\">");
    assert!(!h.contains("<script") && !h.contains("onclick") && !h.contains("<b ") && !h.contains("<img"), "{h}");
    assert!(h.contains("text") && h.contains("bold"), "{h}");
    let h = markdown_html("![the logo](http://contoso.example/logo.png)");
    assert!(h.contains("the logo") && !h.contains("<img") && !h.contains("contoso.example/logo"), "{h}");
    for bad in ["javascript:alert(1)", "data:text/html,x", "file:///etc/passwd", "/local/page", "vbscript:x"] {
        let h = markdown_html(&format!("[click]({bad})"));
        assert!(h.contains("click") && !h.contains("href"), "{bad}: {h}");
    }
    // Marks Quatrelle uses inside its own conversions can't be forged.
    let h = markdown_html("a\u{E000}b\u{E041}c");
    assert!(h.contains("abc"), "{h:?}");
}

/// Past the cap the text is cut; nesting past its own cap is read flat; nothing panics.
#[test]
fn markdown_html_is_bounded() {
    let long = format!("{}TAIL", "a".repeat(MARKDOWN_MAX + 10));
    let h = markdown_html(&long);
    assert!(!h.contains("TAIL") && h.len() < MARKDOWN_MAX + 1024, "{}", h.len());
    let deep = format!("{} x", ">".repeat(100_000));
    let h = markdown_html(&deep);
    assert!(h.matches("<blockquote>").count() <= 32, "{}", h.matches("<blockquote>").count());
    let lists: String = (0..2000).map(|i| format!("{}- item\n", "  ".repeat(i))).collect();
    let h = markdown_html(&lists);
    assert!(h.matches("<ul>").count() <= 32, "{}", h.matches("<ul>").count());
    let _ = markdown_html("[[[[[[[[[[(((((((*_*_*_`````~~~~|||");
}

/// The line of `text` that `needle` starts, and its first character's index.
fn line_of(text: &str, needle: &str) -> (usize, usize) {
    let mut at = 0;
    for (n, line) in text.split('\n').enumerate() {
        if line.starts_with(needle) {
            return (n, at);
        }
        at += line.chars().count() + 1;
    }
    panic!("{needle:?} isn't a line of {text:?}");
}

/// Pasted, Markdown takes the editor's own styles: headings bold and larger, emphasis, code in
/// the monospace font, lists as lists (nested ones indented), quotes indented and grey, a table
/// one line a row; no marks left in the text.
#[test]
fn pasted_markdown_takes_the_editors_styles() {
    let md = "# Plan\n\n**Bold** and *it*\n\n- one\n- two\n  - under\n\n1. first\n2. second\n\n> said\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n```\nlet x = 1;\n```\n";
    let (text, styles) = pasted_markdown(md).expect("Markdown");
    assert!(!text.chars().any(|c| ('\u{E000}'..='\u{F8FF}').contains(&c)), "no marks left: {text:?}");
    let (_, at) = line_of(&text, "Plan");
    assert!(styles.style(at).bold && styles.style(at).size == Some(20), "{:?}", styles.style(at));
    let (_, at) = line_of(&text, "Bold and it");
    assert!(styles.style(at).bold && !styles.style(at + 5).bold && styles.style(at + 9).italic);
    for (item, list, indent) in
        [("one", List::Bullet, 0), ("two", List::Bullet, 0), ("under", List::Bullet, 1), ("first", List::Number, 0), ("second", List::Number, 0)]
    {
        let (n, _) = line_of(&text, item);
        assert_eq!((styles.para(n).list, styles.para(n).indent), (list, indent), "{item}: {text:?}");
    }
    let (n, at) = line_of(&text, "said");
    assert!(styles.para(n).indent == 1 && styles.style(at).color.is_some(), "a quote: {:?}", styles.para(n));
    let (n, at) = line_of(&text, "A · B");
    assert!(styles.style(at).bold && styles.para(n).list == List::None);
    line_of(&text, "1 · 2");
    let (_, at) = line_of(&text, "let x = 1;");
    assert!(styles.style(at).mono);
}

/// Text that isn't Markdown, and Markdown past the cap, are left to be pasted as they are.
#[test]
fn only_markdown_within_the_cap_is_pasted_formatted() {
    assert!(pasted_markdown("just text - here").is_none());
    assert!(pasted_markdown("").is_none());
    let big = format!("# Title\n\n- item\n\n{}", "words ".repeat(MARKDOWN_MAX / 6 + 10));
    assert!(pasted_markdown(&big).is_none());
    // Lines of an item after its first, and a link, as the editor has them.
    let (text, styles) = pasted_markdown("- see [the plan](https://contoso.example/plan)\n- **two**").expect("Markdown");
    let (n, at) = line_of(&text, "see the plan");
    assert_eq!(styles.para(n).list, List::Bullet);
    assert_eq!(styles.style(at + 4).link.as_deref(), Some("https://contoso.example/plan"));
}

/// Tables whose missing cells pulldown-cmark fills in (each row padded to its header's width, up
/// to a quarter of a million cells a table): a paste of them stays bounded in time and in what
/// it writes. Past the budget its tables are read as the text they are written as; a real table
/// is still one.
#[test]
fn a_table_cell_bomb_stays_bounded() {
    let began = std::time::Instant::now();
    // The review's: 90 tables of 363 columns over 724 rows of one cell.
    let t = format!("{}a\n{}-\n{}", "a|".repeat(362), "-|".repeat(362), "a\n".repeat(724));
    let md = format!("# T\n\n{}", vec![t.clone(); 90].join("\n"));
    assert!(md.len() <= MARKDOWN_MAX, "{}", md.len());
    let h = markdown_html(&md);
    assert!(h.len() <= MARKDOWN_HTML_MAX, "{}", h.len());
    assert!(pasted_markdown(&md).is_none_or(|(text, _)| text.len() <= MARKDOWN_HTML_MAX));
    // As many of them as the cap lets in.
    let mut full = String::from("# T\n\n");
    while full.len() + t.len() < MARKDOWN_MAX {
        full.push_str(&t);
        full.push('\n');
    }
    let h = markdown_html(&full);
    assert!(h.len() <= MARKDOWN_HTML_MAX, "{}", h.len());
    assert!(pasted_markdown(&full).is_none_or(|(text, _)| text.len() <= MARKDOWN_HTML_MAX));
    assert!(began.elapsed() < std::time::Duration::from_secs(20), "{:?}", began.elapsed());
    // Real tables, a hundred rows of twenty columns, are read as tables.
    let row = |c: &str| format!("|{}\n", format!(" {c} |").repeat(20));
    let real = format!("# T\n\n{}{}{}", row("H"), row("---"), row("1").repeat(100));
    assert!(markdown_html(&real).contains("1 · 1 · 1"), "a real table");
}
