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
/// in columns; no marks left in the text.
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
    let (n, at) = line_of(&text, "A │ B");
    assert!(styles.style(at).bold && styles.style(at).mono && styles.para(n).list == List::None);
    line_of(&text, "──┼──");
    line_of(&text, "1 │ 2");
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

/// Markdown copied from where it is shown as a code block (a web page's `<pre><code>`, an
/// editor's coloured lines) comes with HTML that carries no formatting of its own: its text is
/// the Markdown to paste. HTML with formatting of its own, or text that isn't Markdown, isn't.
#[test]
fn markdown_in_a_code_block_is_told_from_formatted_html() {
    let text = "# Titre\n- **a**\n> b";
    let browser =
        "<meta charset='utf-8'><html><body><!--StartFragment--><pre><code># Titre\n- **a**\n&gt; b</code></pre><!--EndFragment--></body></html>";
    assert_eq!(code_block_markdown(browser, text).as_deref(), Some(text));
    let editor = "<meta charset='utf-8'><div style=\"color: #d4d4d4;background-color: #1e1e1e;white-space: pre;\"><div><span style=\"color: #569cd6;font-weight: bold;\">#</span><span style=\"color: #569cd6;\"> Titre</span></div><div><span style=\"color: #6796e6;\">-</span> <span style=\"color: #569cd6;\">**a**</span></div><br><div><span class=\"q\">&gt; b</span></div></div>";
    assert_eq!(code_block_markdown(editor, text).as_deref(), Some(text));
    // No text with it: the HTML's own.
    let own = code_block_markdown(browser, "").expect("its text");
    assert!(own.contains("# Titre") && own.contains("- **a**") && own.contains("> b"), "{own:?}");
    // Formatting of its own: the HTML, as before.
    for formatted in [
        "<p><strong>Titre</strong></p>",
        "<ul><li>a</li></ul>",
        "<pre>x</pre><a href=\"https://contoso.example\">y</a>",
        "<p class=MsoNormal>a<o:p></o:p></p>",
        "<h1>T</h1>",
        "<table><tr><td>a</td></tr></table>",
    ] {
        assert_eq!(code_block_markdown(formatted, text), None, "{formatted}");
    }
    // A code block of text that isn't Markdown, and one past the cap.
    assert_eq!(code_block_markdown("<pre><code>let x = 1;</code></pre>", "let x = 1;"), None);
    let huge = format!("<pre><code>{}</code></pre>", "a".repeat(8 * MARKDOWN_MAX));
    assert_eq!(code_block_markdown(&huge, text), None);
}

/// This repository's own page on Gmail, pasted whole: every item keeps its bullet or number, the
/// lines an item is written over join with one space, and no mark is left.
#[test]
fn a_whole_page_keeps_its_items_and_their_words() {
    let md = include_str!("../../../../../docs/gmail.md");
    let (text, styles) = pasted_markdown(md).expect("Markdown");
    assert!(!text.chars().any(|c| ('\u{E000}'..='\u{F8FF}').contains(&c)), "no marks left: {text:?}");
    assert!(!text.split('\n').any(|l| l.contains("  ") && !l.contains(['│', '┼'])), "no run of spaces but a table's: {text:?}");
    let listed = |list: List| (0..text.split('\n').count()).filter(|n| styles.para(*n).list == list).count();
    assert_eq!((listed(List::Bullet), listed(List::Number)), (8, 2), "{text:?}");
    for (item, list) in [
        (
            "Sign in with Google, through the browser (OAuth 2.0), in a copy of Quatrelle built with a Google client (oauth.md). The published builds carry none, so they don't offer it.",
            List::Bullet,
        ),
        (
            "Turn on 2-Step Verification for the Google account (Google Account › Security). Google offers app passwords only once it is on.",
            List::Number,
        ),
        (
            "Open https://myaccount.google.com/apppasswords, give the password a name (Quatrelle), and create it. Google shows sixteen letters, once: copy them.",
            List::Number,
        ),
        ("Gmail's labels are folders here; a message with several labels is in each of them.", List::Bullet),
    ] {
        let (n, _) = line_of(&text, item);
        assert_eq!(styles.para(n).list, list, "{item}");
    }
}

/// Whether the character at `at` is code as pasted: in the monospace font, on the code grey.
fn shaded(styles: &rich::Styles, at: usize) -> bool {
    let s = styles.style(at);
    s.mono && s.highlight == Some(rich::CODE_SHADE)
}

/// Code pasted from Markdown is in the monospace font on a light grey, a highlight of the
/// editor's (sent as one is, so every mail program shows it): a span of it inline, a block of it
/// line by line. A block keeps its lines as written (an empty one, an indent) and each is made as
/// wide as the widest, so that it reads as one grey box.
#[test]
fn code_is_mono_on_a_light_grey_and_a_block_reads_as_one_box() {
    let md = "Run `git status` and **then**:\n\n```\nfn main() {\n\n    go();\n}\n```\n\nDone.";
    let (text, styles) = pasted_markdown(md).expect("Markdown");
    let (_, at) = line_of(&text, "Run git status and then:");
    assert!((at + 4..at + 14).all(|i| shaded(&styles, i)), "the span: {:?}", styles.style(at + 4));
    assert!(!shaded(&styles, at) && !shaded(&styles, at + 3) && !shaded(&styles, at + 14), "around the span");
    let (n, at) = line_of(&text, "fn main() {");
    let lines: Vec<&str> = text.split('\n').collect();
    assert_eq!(lines.get(n..n + 4), Some(&["fn main() {", "           ", "    go();  ", "}          "][..]), "{text:?}");
    let block = lines.get(n..n + 4).map_or(0, |l| l.iter().map(|l| l.chars().count() + 1).sum::<usize>()) - 1;
    assert!((at..at + block).filter(|i| text.chars().nth(*i) != Some('\n')).all(|i| shaded(&styles, i)), "the whole block");
    for k in n..n + 4 {
        assert_eq!(styles.para(k).list, List::None);
    }
    let (_, at) = line_of(&text, "Done.");
    assert!(!shaded(&styles, at));
    // A block of many empty lines under a wide one isn't made many times bigger.
    let md = format!("# T\n\n```\n{}\n{}```\n", "x".repeat(80), "\n".repeat(50_000));
    let (text, _) = pasted_markdown(&md).expect("Markdown");
    assert!(text.len() <= 2 * md.len(), "{} from {}", text.len(), md.len());
}

/// Sent, pasted code keeps its look: in `<code>`, on the grey, the padding of a block's lines
/// kept as spaces that don't collapse.
#[test]
fn pasted_code_is_sent_on_its_grey() {
    let (text, styles) = pasted_markdown("Run `git status` now, **then**\n\n```\nlet x = 1;\nx\n```\n").expect("Markdown");
    let html = rich::to_html(&text, &styles);
    assert!(html.contains("<span style=\"background-color:#eeeeee\"><code>git status</code></span>"), "{html}");
    let first = html.lines().find(|l| l.contains("let x = 1;")).expect("the block's first line");
    assert!(first.contains("<span style=\"background-color:#eeeeee\"><code>let x = 1;</code></span>"), "{first}");
    let second = html.lines().find(|l| l.contains("<code>x ")).expect("the block's second line");
    assert!(
        second.contains("<code>x &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;</code>") && second.contains("background-color:#eeeeee"),
        "{second}"
    );
}

/// A list, a quote and a code block have an empty line before and after them, as paragraphs
/// have between them, where the text around doesn't leave one already: never two in a row, and
/// none inside a list (between an item and the list under it).
#[test]
fn lists_quotes_and_code_blocks_have_room_around_them() {
    for (md, want) in [
        ("Intro:\n- a\n- b\n\nNext **one**", &["Intro:", "", "a", "b", "", "Next one"][..]),
        ("## Steps\n1. a\n2. b\n\nNext **one**", &["Steps", "", "a", "b", "", "Next one"]),
        ("Intro **here**\n> said\n\nAfter", &["Intro here", "", "said", "", "After"]),
        ("Intro **here**\n```\nx\n```\nAfter", &["Intro here", "", "x", "", "After"]),
        ("Intro:\n\n- a\n\n> q\n\n```\nx\n```\n\nEnd", &["Intro:", "", "a", "", "q", "", "x", "", "End"]),
        ("- a\n  - b\n- c\n\n**x**", &["a", "b", "c", "", "x"]),
    ] {
        let (text, _) = pasted_markdown(md).expect("Markdown");
        assert_eq!(text.split('\n').collect::<Vec<_>>(), want, "{md:?}");
    }
    // This repository's page on Gmail: its first list no longer sticks to the paragraph above.
    let (text, styles) = pasted_markdown(include_str!("../../../../../docs/gmail.md")).expect("Markdown");
    assert!(!text.contains("\n\n\n"), "two empty lines in a row: {text:?}");
    let (n, _) = line_of(&text, "Google no longer accepts");
    let lines: Vec<&str> = text.split('\n').collect();
    assert_eq!(lines.get(n + 1), Some(&""), "{text:?}");
    assert_eq!(styles.para(n + 2).list, List::Bullet);
    let (n, _) = line_of(&text, "Make an app password");
    assert_eq!((lines.get(n + 1), styles.para(n + 2).list), (Some(&""), List::Number), "{text:?}");
}

/// The rows of the table on this repository's page on Gmail, laid out, and the rule under its
/// first row.
const GMAIL_TABLE: [&str; 4] = [
    "                │ Server         │ Port │ Security                   │ User name",
    "────────────────┼────────────────┼──────┼────────────────────────────┼──────────────────",
    "Incoming (IMAP) │ imap.gmail.com │ 993  │ SSL/TLS                    │ the whole address",
    "Outgoing (SMTP) │ smtp.gmail.com │ 465  │ SSL/TLS (or 587, STARTTLS) │ the whole address",
];

/// A table is laid out in the monospace font in columns, each as wide as its widest cell, " │ "
/// between them and a rule under the first row, which is bold; each column is aligned as the
/// Markdown says (`:---:`, `---:`), and a cell keeps its own look (code on its grey). A table
/// wider than a message's line is one line a row, its cells between " · ", as before.
#[test]
fn tables_are_laid_out_in_aligned_columns() {
    let (text, styles) = pasted_markdown(include_str!("../../../../../docs/gmail.md")).expect("Markdown");
    let lines: Vec<&str> = text.split('\n').collect();
    let (n, at) = line_of(&text, GMAIL_TABLE[0]);
    assert_eq!(lines.get(n..n + 4), Some(&GMAIL_TABLE[..]), "{text:?}");
    assert_eq!((lines.get(n - 1), lines.get(n + 4)), (Some(&""), Some(&"")), "room around the table");
    let block: usize = GMAIL_TABLE.iter().map(|l| l.chars().count() + 1).sum::<usize>() - 1;
    assert!((at..at + block).filter(|i| text.chars().nth(*i) != Some('\n')).all(|i| styles.style(i).mono), "all of it monospace");
    let (_, head) = line_of(&text, GMAIL_TABLE[0]);
    assert!(styles.style(head + 18).bold && !styles.style(head + 24).bold, "the first row's words bold, not their spaces");
    let (_, row) = line_of(&text, GMAIL_TABLE[2]);
    assert!(!styles.style(row).bold && styles.style(row).highlight.is_none());
    assert!(shaded(&styles, row + 18), "a cell's code: {:?}", styles.style(row + 18));
    for k in n..n + 4 {
        assert_eq!((styles.para(k).list, styles.para(k).indent), (List::None, 0));
    }
    // Aligned as written: left, right, centred.
    let (text, styles) =
        pasted_markdown("| Name | Qty | Mid |\n|:---|---:|:---:|\n| apple | 3 | x |\n| kiwi | 12 | yyy |\n\n**done**").expect("Markdown");
    let (n, _) = line_of(&text, "Name");
    let lines: Vec<&str> = text.split('\n').collect();
    assert_eq!(lines.get(n..n + 4), Some(&["Name  │ Qty │ Mid", "──────┼─────┼────", "apple │   3 │  x", "kiwi  │  12 │ yyy"][..]), "{text:?}");
    assert!(styles.style(0).bold && styles.style(0).mono);
    // Up to 120 characters wide in columns; past it, as before.
    let table = |a: usize, b: usize| format!("| H | I |\n|---|---|\n| {} | {} |\n\n**done**", "x".repeat(a), "y".repeat(b));
    let (text, _) = pasted_markdown(&table(60, 57)).expect("Markdown");
    assert!(text.contains(&format!("{} │ {}", "x".repeat(60), "y".repeat(57))), "{text:?}");
    let (text, styles) = pasted_markdown(&table(60, 58)).expect("Markdown");
    let (_, at) = line_of(&text, "H · I");
    assert!(styles.style(at).bold && !styles.style(at).mono, "{text:?}");
    let (_, at) = line_of(&text, &format!("{} · {}", "x".repeat(60), "y".repeat(58)));
    assert!(!styles.style(at).mono);
}

/// HTML's text as shown: its tags taken out, the spaces that don't collapse made spaces.
fn shown(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&amp;", "&")
}

/// Sent, a pasted table keeps its columns: each row's text in `<code>` (the monospace font), its
/// spaces ones that don't collapse, its first row bold, a cell's code on the grey.
#[test]
fn a_pasted_table_is_sent_in_its_columns() {
    let (text, styles) = pasted_markdown(include_str!("../../../../../docs/gmail.md")).expect("Markdown");
    let html = rich::to_html(&text, &styles);
    for line in GMAIL_TABLE {
        let row = html.lines().find(|l| shown(l) == line).unwrap_or_else(|| panic!("{line:?} isn't sent: {html}"));
        let outside: String = row.split("<code>").map(|p| p.split_once("</code>").map_or(p, |(_, after)| after)).collect();
        assert!(shown(&outside).is_empty(), "outside <code>: {row}");
    }
    assert!(html.contains("<b><code>Server</code></b>"), "{html}");
    assert!(html.contains("<span style=\"background-color:#eeeeee\"><code>imap.gmail.com</code></span>"), "{html}");
}

/// This repository's page on Gmail as a terminal gives it once printed (`cat`) and copied: each
/// line's trailing spaces gone, tabs and indents as spaces, one or two line ends after it, the
/// shell's prompt lines before and after. Each is pasted formatted, its lists and table kept.
#[test]
fn a_page_copied_from_a_terminal_is_pasted_formatted() {
    let md = include_str!("../../../../../docs/gmail.md");
    let trimmed: String = md.lines().map(|l| format!("{}\n", l.trim_end())).collect();
    let prompt = "ann@desk:~/quatrelle$";
    for (what, text) in [
        ("trailing spaces trimmed", trimmed.clone()),
        ("two line ends after it", format!("{trimmed}\n")),
        ("no line end after it", trimmed.trim_end().to_string()),
        ("indents as a tab shows", trimmed.replace("\n  ", "\n        ")),
        ("tabs as spaces", trimmed.replace("\n  ", "\n\t").replace('\t', "        ")),
        ("the prompt around it", format!("{prompt} cat docs/gmail.md\n{trimmed}{prompt} ")),
    ] {
        let (got, styles) = pasted_markdown(&text).unwrap_or_else(|| panic!("{what}: not pasted formatted"));
        let listed = |list: List| (0..got.split('\n').count()).filter(|n| styles.para(*n).list == list).count();
        assert_eq!((listed(List::Bullet), listed(List::Number)), (8, 2), "{what}: {got:?}");
        let (n, _) = line_of(&got, GMAIL_TABLE[0]);
        assert_eq!(got.split('\n').nth(n + 2), Some(GMAIL_TABLE[2]), "{what}: {got:?}");
        assert!(!got.chars().any(|c| ('\u{E000}'..='\u{F8FF}').contains(&c)), "{what}: no marks left");
    }
}
