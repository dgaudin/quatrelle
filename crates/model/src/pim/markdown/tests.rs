// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A note's Markdown: each mark, marks left open, nesting, the bar's edits, Enter in a list, a
//! tick ticked; hostile text within its caps; and any text read into its own lines.

use proptest::prelude::*;

use super::*;

/// The first line's segments: kind, style and words.
fn segs(text: &str) -> Vec<(Kind, Style, &str)> {
    let lines = read(text);
    lines.first().map(|l| l.spans.iter().map(|s| (s.kind, s.style, text.get(s.range.clone()).unwrap_or("?"))).collect()).unwrap_or_default()
}

const PLAIN: Style = Style { bold: false, italic: false, strike: false };
const BOLD: Style = Style { bold: true, italic: false, strike: false };
const ITALIC: Style = Style { bold: false, italic: true, strike: false };
const STRUCK: Style = Style { bold: false, italic: false, strike: true };

fn block(text: &str) -> Block {
    read(text).first().map_or(Block::Plain, |l| l.block)
}

#[test]
fn each_mark() {
    // Titles.
    assert_eq!(segs("# Title"), [(Kind::Mark, PLAIN, "# "), (Kind::Text, PLAIN, "Title")]);
    assert_eq!(
        (block("## Two"), block("### Three"), block("#### Four"), block("#tag")),
        (Block::Heading(2), Block::Heading(3), Block::Plain, Block::Plain)
    );
    // Bold, italic, struck, code.
    assert_eq!(
        segs("a **b** c"),
        [(Kind::Text, PLAIN, "a "), (Kind::Mark, PLAIN, "**"), (Kind::Text, BOLD, "b"), (Kind::Mark, PLAIN, "**"), (Kind::Text, PLAIN, " c")]
    );
    assert_eq!(segs("*i*"), [(Kind::Mark, PLAIN, "*"), (Kind::Text, ITALIC, "i"), (Kind::Mark, PLAIN, "*")]);
    assert_eq!(segs("_i_"), [(Kind::Mark, PLAIN, "_"), (Kind::Text, ITALIC, "i"), (Kind::Mark, PLAIN, "_")]);
    assert_eq!(segs("~~s~~"), [(Kind::Mark, PLAIN, "~~"), (Kind::Text, STRUCK, "s"), (Kind::Mark, PLAIN, "~~")]);
    assert_eq!(
        segs("x `a*b*` y"),
        [(Kind::Text, PLAIN, "x "), (Kind::Mark, PLAIN, "`"), (Kind::Code, PLAIN, "a*b*"), (Kind::Mark, PLAIN, "`"), (Kind::Text, PLAIN, " y")]
    );
    assert_eq!(
        segs("snake_case_name 2*3*4"),
        [
            (Kind::Text, PLAIN, "snake_case_name 2"),
            (Kind::Mark, PLAIN, "*"),
            (Kind::Text, ITALIC, "3"),
            (Kind::Mark, PLAIN, "*"),
            (Kind::Text, PLAIN, "4")
        ]
    );
    // Fenced code: nothing read in it.
    let code = "```rust\nlet *x* = 1;\n```\nafter";
    let blocks: Vec<Block> = read(code).iter().map(|l| l.block).collect();
    assert_eq!(blocks, [Block::Fence, Block::Code, Block::Fence, Block::Plain]);
    assert_eq!(read(code)[1].spans.len(), 1);
    // Lists, quotes, a line.
    assert_eq!(segs("- one"), [(Kind::Mark, PLAIN, "- "), (Kind::Text, PLAIN, "one")]);
    assert_eq!(block("1. one"), Block::Numbered { depth: 0, number: 1 });
    assert_eq!(block("12) twelve"), Block::Numbered { depth: 0, number: 12 });
    assert_eq!(block("* star"), Block::Bullet { depth: 0 });
    assert_eq!((block("-no"), block("1.no"), block("1234567890. no")), (Block::Plain, Block::Plain, Block::Plain));
    assert_eq!(segs("> said"), [(Kind::Mark, PLAIN, "> "), (Kind::Text, PLAIN, "said")]);
    assert_eq!(segs("---"), [(Kind::Mark, PLAIN, "---")]);
    assert_eq!(block("---"), Block::Rule);
    // Ticks.
    assert_eq!(segs("- [ ] buy"), [(Kind::Mark, PLAIN, "- "), (Kind::Tick, PLAIN, "[ ]"), (Kind::Text, PLAIN, " buy")]);
    assert_eq!(block("- [x] done"), Block::Tick { depth: 0, done: true });
    assert_eq!(block("2. [X] done"), Block::Tick { depth: 0, done: true });
    assert_eq!(block("- [x]done"), Block::Bullet { depth: 0 });
    // Links, and bare addresses (their last full stop is the sentence's).
    let text = "see [site](https://studio.example/a) now";
    assert_eq!(
        segs(text),
        [
            (Kind::Text, PLAIN, "see "),
            (Kind::Mark, PLAIN, "["),
            (Kind::LinkText, PLAIN, "site"),
            (Kind::Mark, PLAIN, "]("),
            (Kind::Url, PLAIN, "https://studio.example/a"),
            (Kind::Mark, PLAIN, ")"),
            (Kind::Text, PLAIN, " now"),
        ]
    );
    assert_eq!(link_at(text, 6), Some("https://studio.example/a"));
    assert_eq!(link_at(text, 1), None);
    let bare = "go to https://studio.example/x.";
    assert_eq!(segs(bare), [(Kind::Text, PLAIN, "go to "), (Kind::Url, PLAIN, "https://studio.example/x"), (Kind::Text, PLAIN, ".")]);
    assert_eq!(link_at(bare, 10), Some("https://studio.example/x"));
    assert_eq!(
        segs("(https://studio.example/x)"),
        [(Kind::Text, PLAIN, "("), (Kind::Url, PLAIN, "https://studio.example/x"), (Kind::Text, PLAIN, ")")]
    );
    // Only the web's addresses and email: anything else stays as written.
    assert_eq!(segs("[x](javascript:alert(1))"), [(Kind::Text, PLAIN, "[x](javascript:alert(1))")]);
    assert_eq!(segs("xhttps://studio.example"), [(Kind::Text, PLAIN, "xhttps://studio.example")]);
    assert_eq!(segs("[me](mailto:zoe@studio.example)")[1], (Kind::LinkText, PLAIN, "me"));
}

#[test]
fn marks_left_open_stay_text() {
    for open in ["**bold", "*a", "~~a", "`code", "[a](studio.example", "a ** b", "~a~", "[](x)"] {
        assert_eq!(segs(open), [(Kind::Text, PLAIN, open)], "{open}");
    }
    // A link left open: its address is still one.
    assert_eq!(segs("[a](https://studio.example"), [(Kind::Text, PLAIN, "[a]("), (Kind::Url, PLAIN, "https://studio.example")]);
    // A fence left open is code to the end, as typed.
    let blocks: Vec<Block> = read("```\ncode\n# not a title").iter().map(|l| l.block).collect();
    assert_eq!(blocks, [Block::Fence, Block::Code, Block::Code]);
    // One closed, one left open.
    assert_eq!(segs("**a** **b"), [(Kind::Mark, PLAIN, "**"), (Kind::Text, BOLD, "a"), (Kind::Mark, PLAIN, "**"), (Kind::Text, PLAIN, " **b")]);
}

#[test]
fn marks_nest() {
    let both = Style { bold: true, italic: true, strike: false };
    assert_eq!(segs("***both***")[2], (Kind::Text, both, "both"));
    let s = segs("**a *b* c**");
    assert_eq!(s[1], (Kind::Text, BOLD, "a "));
    assert_eq!(s[3], (Kind::Text, both, "b"));
    assert_eq!(s[5], (Kind::Text, BOLD, " c"));
    assert_eq!(segs("**[l](https://studio.example)**")[2], (Kind::LinkText, BOLD, "l"));
    assert_eq!(segs("~~**x**~~")[2], (Kind::Text, Style { bold: true, italic: false, strike: true }, "x"));
    // Lists by their indentation, two spaces (or half a tab) a level, eight levels at most.
    assert_eq!(block("  - two"), Block::Bullet { depth: 1 });
    assert_eq!(block("\t- tab"), Block::Bullet { depth: 2 });
    assert_eq!(block("    1. three"), Block::Numbered { depth: 2, number: 1 });
    assert_eq!(block(&format!("{}- deep", " ".repeat(40))), Block::Bullet { depth: MAX_DEPTH - 1 });
    assert_eq!(segs("  - [ ] *a*")[4], (Kind::Text, ITALIC, "a"));
}

#[test]
fn a_tick_toggles_only_its_box() {
    let before = "- [ ] buy *paper*\n  - [x] done\n```\n- [ ] code\n```";
    let mut t = before.to_string();
    assert!(toggle_tick(&mut t, 3));
    assert_eq!(t, "- [x] buy *paper*\n  - [x] done\n```\n- [ ] code\n```");
    assert!(toggle_tick(&mut t, 30), "at the end of its line");
    assert_eq!(t, "- [x] buy *paper*\n  - [ ] done\n```\n- [ ] code\n```");
    let differ = before.bytes().zip(t.bytes()).filter(|(a, b)| a != b).count();
    assert_eq!((t.len(), differ), (before.len(), 2), "only the two boxes changed");
    // Not a tick: code, a plain line, past the text.
    let kept = t.clone();
    assert!(!toggle_tick(&mut t, 38));
    assert!(!toggle_tick(&mut t, 9999));
    let mut plain = "buy [ ] paper".to_string();
    assert!(!toggle_tick(&mut plain, 5));
    assert_eq!(plain, "buy [ ] paper");
    assert_eq!(t, kept);
}

#[test]
fn enter_goes_on_with_a_list_or_ends_it() {
    let e = |t: &str, at: usize| enter(t, at).map(|e| (e.text, e.select));
    assert_eq!(e("- one", 5), Some(("- one\n- ".into(), 8..8)));
    assert_eq!(e("1. one", 6), Some(("1. one\n2. ".into(), 10..10)));
    assert_eq!(e("9) nine", 7), Some(("9) nine\n10) ".into(), 12..12)));
    assert_eq!(e("  - [x] done", 12), Some(("  - [x] done\n  - [ ] ".into(), 21..21)), "a new tick, not ticked, at the same depth");
    assert_eq!(e("- one two", 5), Some(("- one\n-  two".into(), 8..8)), "the words after go with the new item");
    // An empty item ends the list.
    assert_eq!(e("- ", 2), Some((String::new(), 0..0)));
    assert_eq!(e("a\n- [ ] ", 8), Some(("a\n".into(), 2..2)));
    // Elsewhere, an ordinary line break.
    assert_eq!(e("plain", 5), None);
    assert_eq!(e("- one", 0), None, "before its marker");
    assert_eq!(e("```\n- one\n```", 9), None, "in code");
}

#[test]
fn the_bar_puts_marks_on_and_takes_them_off() {
    let m = |t: &str, r: std::ops::Range<usize>, mark| {
        let e = toggle_mark(t, r, mark);
        (e.text, e.select)
    };
    assert_eq!(m("say hi", 4..6, Mark::Bold), ("say **hi**".into(), 6..8));
    assert_eq!(m("say **hi**", 6..8, Mark::Bold), ("say hi".into(), 4..6));
    assert_eq!(m("say **hi**", 4..10, Mark::Bold), ("say hi".into(), 4..6), "the marks selected with the words");
    assert_eq!(m("say hi ", std::ops::Range { start: 6, end: 3 }, Mark::Italic), ("say *hi* ".into(), 5..7), "either way round, without the spaces");
    assert_eq!(m("**hi**", 2..4, Mark::Italic), ("***hi***".into(), 3..5));
    assert_eq!(m("***hi***", 3..5, Mark::Italic), ("**hi**".into(), 2..4));
    assert_eq!(m("ab", 1..1, Mark::Bold), ("a****b".into(), 3..3));
    assert_eq!(m("a\nb", 0..3, Mark::Bold), ("**a**\n**b**".into(), 0..11));
    // Titles, lists and ticks, on every line touched.
    assert_eq!(m("one\ntwo", 0..5, Mark::Heading), ("# one\n# two".into(), 2..9));
    assert_eq!(m("# one\n# two", 2..9, Mark::Heading), ("one\ntwo".into(), 0..5));
    assert_eq!(m("a\n- b", 0..5, Mark::List), ("- a\n- b".into(), 2..7));
    assert_eq!(m("- a\n- b", 0..7, Mark::List), ("a\nb".into(), 0..3));
    assert_eq!(m("  a", 3..3, Mark::List), ("  - a".into(), 5..5), "indentation kept");
    assert_eq!(m("- a", 3..3, Mark::Tick), ("- [ ] a".into(), 7..7));
    assert_eq!(m("1. a", 4..4, Mark::Tick), ("1. [ ] a".into(), 8..8));
    assert_eq!(m("- [x] a", 7..7, Mark::Tick), ("a".into(), 1..1));
    assert_eq!(m("```\ncode\n```", 5..5, Mark::Heading).0, "```\ncode\n```", "code stays as it is");
    // Links: the address selected to be typed; taken off, the words stay.
    let (t, s) = m("see site", 4..8, Mark::Link);
    assert_eq!((t.as_str(), t.get(s)), ("see [site](https://)", Some("https://")));
    assert_eq!(m("ab", 1..1, Mark::Link), ("a[](https://)b".into(), 2..2));
    assert_eq!(m("x [site](https://studio.example) y", 4..4, Mark::Link), ("x site y".into(), 2..6));
}

#[test]
fn hostile_text_is_read_within_its_caps() {
    for piece in ["*", "_a", "[", "`", "~~", "**a", "[a](", "https://", "é*"] {
        let text = piece.repeat(60_000 / piece.len());
        let lines = read(&text);
        assert!(lines.iter().all(|l| l.spans.len() <= MAX_SPANS), "{piece}");
        check(&text);
    }
    let many = "- a\n".repeat(MAX_LINES + 5);
    let lines = read(&many);
    assert_eq!(lines.len(), MAX_LINES + 1, "the rest is one plain line");
    assert_eq!(lines.last().map(|l| l.block), Some(Block::Plain));
    check(&many);
    let long = format!("{}\n# title", "a".repeat(MAX_READ + 10));
    assert_eq!(read(&long).len(), 2);
    check(&long);
    // Positions past the text or inside a character are held to it.
    for mark in MARKS {
        let e = toggle_mark("é\nü", 1..99, mark);
        check_edit(&e);
    }
    assert!(enter("- é", 4).is_some_and(|e| e.text == "- é\n- "));
    assert!(enter("- é", 3).is_some_and(|e| e.text == "- \n- é"), "inside a character: before it");
}

const MARKS: [Mark; 6] = [Mark::Bold, Mark::Italic, Mark::Heading, Mark::List, Mark::Tick, Mark::Link];

/// Lines in order, separated by their line breaks, covering the text; each line's segments
/// in order, never empty, on characters' boundaries, covering it.
fn check(text: &str) {
    let lines = read(text);
    let mut end = 0;
    for (i, l) in lines.iter().enumerate() {
        assert!(l.range.start <= l.range.end && l.range.end <= text.len(), "{l:?}");
        assert!(text.is_char_boundary(l.range.start) && text.is_char_boundary(l.range.end));
        let gap = text.get(end..l.range.start);
        assert!(if i == 0 { l.range.start == 0 } else { gap == Some("\n") || gap == Some("\r\n") }, "{gap:?} before line {i}");
        assert!(l.spans.len() <= MAX_SPANS);
        let mut at = l.range.start;
        for s in &l.spans {
            assert_eq!(s.range.start, at, "{:?} in {l:?}", s);
            assert!(s.range.end > s.range.start && text.is_char_boundary(s.range.end));
            at = s.range.end;
        }
        assert_eq!(at, l.range.end, "{l:?}");
        end = l.range.end;
    }
    assert_eq!(end, text.len());
}

fn check_edit(e: &Edit) {
    assert!(e.select.start <= e.select.end && e.select.end <= e.text.len(), "{e:?}");
    assert!(e.text.is_char_boundary(e.select.start) && e.text.is_char_boundary(e.select.end));
    check(&e.text);
}

/// Text made of Markdown's pieces, with others' characters in it.
fn markdownish() -> impl Strategy<Value = String> {
    const PIECES: [&str; 32] = [
        "# ",
        "## ",
        "### ",
        "- ",
        "* ",
        "1. ",
        "[ ] ",
        "[x] ",
        "**",
        "*",
        "_",
        "~~",
        "`",
        "```",
        "> ",
        "---",
        "[",
        "](",
        ")",
        "https://studio.example/a",
        " ",
        "  ",
        "\t",
        "\n",
        "\r\n",
        "\r",
        "word",
        "é",
        "中",
        "x",
        "mailto:a@b",
        "(",
    ];
    prop_oneof![prop::collection::vec(prop::sample::select(PIECES.to_vec()), 0..48).prop_map(|v| v.concat()), any::<String>()]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    /// Any text gives ranges inside it, in order, without overlap, and reading changes nothing.
    #[test]
    fn any_text_reads_into_its_own_lines(text in markdownish()) {
        check(&text);
    }

    /// The edits, anywhere: a selection inside the text they give; a tick changes one byte or none.
    #[test]
    fn edits_hold_anywhere(text in markdownish(), a in 0usize..400, b in 0usize..400, m in 0usize..6) {
        let mark = MARKS.get(m).copied().unwrap_or(Mark::Bold);
        check_edit(&toggle_mark(&text, a..b, mark));
        if let Some(e) = enter(&text, a) {
            check_edit(&e);
        }
        let mut t = text.clone();
        let ticked = toggle_tick(&mut t, a);
        let differ = text.bytes().zip(t.bytes()).filter(|(x, y)| x != y).count();
        prop_assert_eq!((t.len(), differ), (text.len(), usize::from(ticked)));
        let _ = link_at(&text, a);
    }
}
