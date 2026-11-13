// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Tests of a received message's formatting written as Markdown: each construct, nesting,
//! tables and pictures, links as the message has them or cleaned, and hostile HTML bounded.

use super::*;

const PAGE: &str = r#"<h1>Title</h1><p>Some <b>bold</b>, <i>it</i>, <s>gone</s>, <code>x()</code> and <a href="https://contoso.example/a">a link</a> and https://contoso.example/b.</p>
<ul><li>one</li><li>two<ul><li>under</li></ul></li></ul><ol start="3"><li>three</li><li>four</li></ol>
<blockquote><p>said</p><p>more</p></blockquote>
<table border=1><tr><th>A</th><th>B</th></tr><tr><td>1</td><td><b>2</b></td></tr><tr><td>3</td><td>4 | 5</td></tr></table>
<pre>let x = 1;

let y = 2;</pre>
<p><img src="cid:logo@contoso.example" alt="the logo"> <img src="https://contoso.example/p.png" alt="photo"><img src="https://t.contoso.example/p.gif" width=1 height=1></p>
<hr><p style="display:none">hidden words</p>
<h3>Small</h3><p>a * star, _under_ score, [brackets] and `ticks`</p><p>2 + 2</p><p># not a title</p>"#;

/// Each construct as Markdown writes it.
#[test]
fn each_construct_is_written_as_markdown() {
    let md = html_markdown(PAGE);
    let lines: Vec<&str> = md.lines().collect();
    for want in [
        "# Title",
        "Some **bold**, *it*, ~~gone~~, `x()` and [a link](https://contoso.example/a) and https://contoso.example/b.",
        "- one",
        "- two",
        "  - under",
        "3. three",
        "4. four",
        "> said",
        ">",
        "> more",
        "| A | B |",
        "|---|---|",
        "| 1 | **2** |",
        "| 3 | 4 \\| 5 |",
        "```",
        "let x = 1;",
        "let y = 2;",
        // (The white space between two pictures goes, as in the formatted view.)
        "![the logo](cid:logo@contoso.example)![photo](https://contoso.example/p.png)",
        "---",
        "### Small",
        "a \\* star, \\_under\\_ score, \\[brackets\\] and \\`ticks\\`",
        "\\# not a title",
    ] {
        assert!(lines.iter().any(|l| l.trim_end() == want), "{want:?} in\n{md}");
    }
    // A table's rule once, under its first row; a fenced block around both lines, the blank one
    // between them inside it.
    assert_eq!(md.matches("|---|").count(), 1, "{md}");
    assert!(md.contains("```\nlet x = 1;\n\nlet y = 2;\n```"), "{md}");
    // Nothing hidden, no tracking picture, no strikethrough characters left.
    assert!(!md.contains("hidden words") && !md.contains("t.contoso.example") && !md.contains('\u{336}'), "{md}");
}

/// Two lines of text one after the other stay two lines (a hard break), not one paragraph.
#[test]
fn lines_of_a_signature_stay_lines() {
    let md = html_markdown("<div>Ann Example</div><div>Contoso</div><div>+1 555 0100</div>");
    assert!(md.contains("Ann Example  \nContoso  \n+1 555 0100"), "{md:?}");
}

/// Lists and quotes nested in each other keep their depth.
#[test]
fn nesting_is_kept() {
    let md = html_markdown("<ul><li>a<ul><li>b<ul><li>c</li></ul></li></ul></li></ul><blockquote>out<blockquote>in</blockquote></blockquote>");
    for want in ["- a", "  - b", "    - c", "> out", "> > in"] {
        assert!(md.lines().any(|l| l.trim_end() == want), "{want:?} in\n{md}");
    }
}

/// A link reads as the message has it, or without its tracking tags when links are cleaned (as
/// the formatted view shows it); code with backticks in it is fenced by more of them.
#[test]
fn links_as_the_message_has_them_or_cleaned() {
    let html = r#"<p><a href="https://shop.contoso.example/sale?utm_source=news&amp;id=7">Sale</a> <code>a`b</code></p>"#;
    let rich = html_to_rich(html);
    assert!(rich_markdown(&rich, false).contains("[Sale](https://shop.contoso.example/sale?utm_source=news&id=7)"));
    let cleaned = rich_markdown(&rich, true);
    assert!(cleaned.contains("[Sale](https://shop.contoso.example/sale?id=7)") && cleaned.contains("``a`b``"), "{cleaned}");
    // An address with spaces or brackets goes between angle brackets.
    let odd = html_markdown(r#"<a href="https://contoso.example/a (1).pdf">file</a>"#);
    assert!(odd.contains("[file](<https://contoso.example/a (1).pdf>)"), "{odd}");
}

/// What Markdown pasted into a message becomes reads back as the same Markdown.
#[test]
fn markdown_round_trips() {
    let md = "# Plan\n\n**Bold** and *it* and ~~old~~\n\n- one\n- two\n\n1. first\n2. second\n\n> said\n\nSee [the site](https://contoso.example/)";
    let back = html_markdown(&markdown_html(md));
    for line in md.lines().filter(|l| !l.is_empty()) {
        assert!(back.lines().any(|l| l.trim_end() == line), "{line:?} in\n{back}");
    }
}

/// Hostile HTML stays bounded: nested past any mail, or huge; formatted lines past the cap.
#[test]
fn hostile_html_is_bounded() {
    let deep = format!("{}text{}", "<div><ul><li><blockquote>".repeat(20_000), "</blockquote></li></ul></div>".repeat(20_000));
    let md = html_markdown(&deep);
    assert!(md.len() < deep.len(), "{}", md.len());
    let wide = "<table><tr><td>a</td><td>b</td></tr></table>".repeat(5_000);
    let _ = html_markdown(&wide);
    let line = vec![Span { text: "x".repeat(10_000), bold: true, ..Default::default() }];
    let many: RichText = vec![line; 1_000];
    let md = rich_markdown(&many, false);
    assert!(md.len() <= MARKDOWN_WRITTEN_MAX, "{}", md.len());
    assert!(!md.is_empty());
}

/// A picture without a description, alone on its line (a logo, a banner), is still written,
/// as text: `![](address)`, nothing fetched.
#[test]
fn a_picture_without_a_description_is_written() {
    let md = html_markdown("<p><img src=\"cid:logo@contoso.example\"></p><p>text</p>");
    assert!(md.contains("![](cid:logo@contoso.example)"), "{md:?}");
}
