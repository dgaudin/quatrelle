// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The Teams HTML subset, read and written: one test per rule, each with the hostile input it
//! guards against.

use super::*;
use proptest::strategy::Strategy;
use quatrelle_model::pim::teams::{BlockKind, SEND_MAX};

const C: &str = "19:a@thread.v2";

/// Every run of every block, in order.
fn runs(t: &ChatText) -> Vec<&Run> {
    t.blocks.iter().flat_map(|b| b.runs.iter()).collect()
}

/// The run holding `text`.
fn run<'a>(t: &'a ChatText, text: &str) -> &'a Run {
    runs(t).into_iter().find(|r| r.text == text).unwrap_or_else(|| panic!("no run {text:?} in {t:?}"))
}

#[test]
fn bold_italic_links_lists_quotes_code_are_kept() {
    let html = "<p><b>b</b> <i>i</i> <a href=\"https://x.example/\">x</a></p><ul><li>one</li></ul><blockquote>q</blockquote><code>c</code>";
    let t = teams_html(html, C, "1");
    assert!(run(&t, "b").bold && !run(&t, "b").italic);
    assert!(run(&t, "i").italic && !run(&t, "i").bold);
    assert_eq!(run(&t, "x").link.as_deref(), Some("https://x.example/"));
    assert!(run(&t, "c").code);
    let kinds: Vec<BlockKind> = t.blocks.iter().map(|b| b.kind).collect();
    assert_eq!(kinds, [BlockKind::Paragraph, BlockKind::Item { ordered: false, depth: 1 }, BlockKind::Quote, BlockKind::Paragraph]);
    assert_eq!(t.plain(), "b i x\none\nq\nc");
}

#[test]
fn underline_strike_ordered_lists_code_blocks_and_breaks_are_kept() {
    let t = teams_html("<u>u</u><s>s</s><ol><li>a<ul><li>b</li></ul></li></ol><pre>x  y\nz</pre><p>l1<br>l2</p>", C, "1");
    assert!(run(&t, "u").underline);
    assert!(run(&t, "s").strike);
    let kinds: Vec<BlockKind> = t.blocks.iter().map(|b| b.kind).collect();
    assert_eq!(
        kinds,
        [
            BlockKind::Paragraph,
            BlockKind::Item { ordered: true, depth: 1 },
            BlockKind::Item { ordered: false, depth: 2 },
            BlockKind::Code,
            BlockKind::Paragraph
        ]
    );
    assert_eq!(t.plain(), "us\na\nb\nx  y\nz\nl1\nl2");
}

#[test]
fn spaces_collapse_and_entities_are_read() {
    let t = teams_html("<p>\n  a   b&amp;c&lt;d&nbsp;e  </p>\n<p></p><div> </div>", C, "1");
    assert_eq!(t.plain(), "a b&c<d e");
    assert_eq!(t.blocks.len(), 1, "empty blocks are left out");
}

#[test]
fn a_link_other_than_http_keeps_its_text_only() {
    for href in ["javascript:alert(1)", "file:///etc/passwd", "ms-settings:", "data:text/html,x", " https://x.example/ a", ""] {
        let t = teams_html(&format!("<a href=\"{href}\">text</a>"), C, "1");
        assert_eq!(run(&t, "text").link, None, "{href}");
        assert_eq!(t.plain(), "text");
    }
    let t = teams_html("<a href=\"HTTPS://x.example/a?b=1&amp;c=2\">t</a>", C, "1");
    assert_eq!(run(&t, "t").link.as_deref(), Some("https://x.example/a?b=1&c=2"));
}

#[test]
fn scripts_and_styles_go_with_their_content() {
    let t = teams_html("<script>bad()</script><style>x{}</style><p>ok</p>", C, "1");
    assert_eq!(t.plain(), "ok");
    let t = teams_html("<script>document.write('<p>no</p>')</script><title>t</title><iframe>f</iframe>ok", C, "1");
    assert_eq!(t.plain(), "ok");
}

#[test]
fn an_img_is_hosted_only_when_it_points_at_this_message() {
    let own = format!("<img src=\"https://graph.microsoft.com/v1.0/chats/{C}/messages/1/hostedContents/aGk=/$value\">");
    assert_eq!(teams_html(&own, C, "1").blocks[0].runs[0].hosted.as_deref(), Some("aGk="));
    assert_eq!(teams_html(&own, C, "1").blocks[0].runs[0].outside, None);
    let other_msg = own.replace("/messages/1/", "/messages/2/");
    assert_eq!(teams_html(&other_msg, C, "1").blocks[0].runs[0].hosted, None);
    assert!(teams_html(&other_msg, C, "1").blocks[0].runs[0].outside.is_some(), "any other img waits for a click");
    let slash = own.replace("aGk=", "a/b");
    assert_eq!(teams_html(&slash, C, "1").blocks[0].runs[0].hosted, None);
    let long = own.replace("aGk=", &"a".repeat(513));
    assert_eq!(teams_html(&long, C, "1").blocks[0].runs[0].hosted, None);
}

#[test]
fn a_hosted_img_is_compared_percent_decoded_and_kept_safe() {
    // Teams writes the chat id with its `:` and `@` escaped.
    let escaped = "<img src=\"https://graph.microsoft.com/v1.0/chats/19%3aa%40thread.v2/messages/1/hostedContents/aGk%3D/$value\">";
    assert_eq!(teams_html(escaped, C, "1").blocks[0].runs[0].hosted.as_deref(), Some("aGk="));
    // Another chat, a query or another host: an outside image.
    for src in [
        "https://graph.microsoft.com/v1.0/chats/19:b@thread.v2/messages/1/hostedContents/aGk=/$value",
        "https://graph.microsoft.com/v1.0/chats/19:a@thread.v2/messages/1/hostedContents/a%2Fb/$value",
        "https://graph.microsoft.com/v1.0/chats/19:a@thread.v2/messages/1/hostedContents/a?b/$value",
        "https://graph.microsoft.com/v1.0/chats/19:a@thread.v2/messages/1/hostedContents/aGk=/$value?x",
        "https://graph.microsoft.com.evil.example/v1.0/chats/19:a@thread.v2/messages/1/hostedContents/aGk=/$value",
        "https://graph.microsoft.com/v1.0/chats/19:a@thread.v2/messages/1/hostedContents//$value",
        // Only dots: the address the worker builds would climb out of the message.
        "https://graph.microsoft.com/v1.0/chats/19:a@thread.v2/messages/1/hostedContents/./$value",
        "https://graph.microsoft.com/v1.0/chats/19:a@thread.v2/messages/1/hostedContents/../$value",
        "https://graph.microsoft.com/v1.0/chats/19:a@thread.v2/messages/1/hostedContents/%2E%2E/$value",
    ] {
        let r = teams_html(&format!("<img src=\"{src}\">"), C, "1").blocks[0].runs[0].clone();
        assert_eq!(r.hosted, None, "{src}");
        assert!(r.outside.is_some(), "{src}");
    }
    // Neither hosted nor on the web: nothing to show.
    for src in ["cid:x", "file:///etc/passwd", "../hostedContents/1/$value", "javascript:x"] {
        let t = teams_html(&format!("<img src=\"{src}\">"), C, "1");
        assert!(t.blocks.is_empty(), "{src}: {t:?}");
    }
}

#[test]
fn mentions_and_emoji_and_attachments_are_placed() {
    let t = teams_html(
        "<p>Hi <at id=\"0\">Ana</at>, <emoji id=\"x\" alt=\"😀\" title=\"t\"><img src=\"https://x.example/e.png\"></emoji></p><attachment id=\"abc\"></attachment>",
        C,
        "1",
    );
    assert_eq!(run(&t, "Ana").mention, Some(0));
    assert_eq!(run(&t, "Hi ").mention, None);
    assert!(t.plain().contains('😀'));
    assert!(runs(&t).iter().all(|r| r.outside.is_none()), "an emoji's own picture is left out");
    let marked: Vec<Option<&str>> = runs(&t).iter().filter_map(|r| r.attachment.as_deref().map(Some)).collect();
    assert_eq!(marked, [Some("abc")]);
    assert_eq!(teams_html("<at id=\"x\">Bo</at>", C, "1").blocks[0].runs[0].mention, None);
    assert_eq!(teams_html("<emoji alt=\"\u{202E}😀\"></emoji>", C, "1").plain(), "😀");
    let long = format!("<attachment id=\"{}\"></attachment>", "a".repeat(513));
    assert!(teams_html(&long, C, "1").blocks.is_empty());
}

#[test]
fn unknown_tags_keep_their_text() {
    assert_eq!(teams_html("<span><font>x</font><marquee>y</marquee></span>", C, "1").plain(), "xy");
    assert_eq!(teams_html("<table><tr><td>a</td><td>b</td></tr><tr><td>c</td></tr></table>", C, "1").plain(), "a b\nc");
}

#[test]
fn input_and_depth_and_output_are_capped() {
    let deep = "<b>".repeat(10_000) + "x";
    assert!(teams_html(&deep, C, "1").plain().contains('x'));
    let huge = "<p>".to_string() + &"é".repeat(300 << 10);
    assert!(teams_html(&huge, C, "1").plain().len() <= TEAMS_HTML_MAX);
    // Entities that grow when read, and emoji alt texts, stay within the cap too.
    let growing = "&nGt;".repeat(60 << 10);
    assert!(teams_html(&growing, C, "1").plain().len() <= TEAMS_HTML_MAX);
    let many = "<b>a</b><i>b</i>".repeat(20_000);
    assert!(runs(&teams_html(&many, C, "1")).len() <= TEAMS_RUNS);
    // Past the depth, the text is kept flat, and styles closed again close.
    let t = teams_html(&("<i>".repeat(40) + "x" + &"</i>".repeat(40) + "y"), C, "1");
    assert!(run(&t, "x").italic);
    assert!(!run(&t, "y").italic);
    let lists = teams_html(&("<ul><li>".repeat(300) + "z"), C, "1");
    assert_eq!(lists.plain(), "z");
    assert!(matches!(lists.blocks[0].kind, BlockKind::Item { depth, .. } if usize::from(depth) <= TEAMS_DEPTH));
}

#[test]
fn the_addresses_runs_carry_are_within_the_budget() {
    // One long link around runs of alternating styles: each run carries the address.
    let html = format!("<a href=\"https://x.example/{}\">{}</a>", "A".repeat(4000), "<b>a</b><i>b</i>".repeat(20_000));
    let t = teams_html(&html, C, "1");
    let carried: usize =
        runs(&t).iter().map(|r| [&r.link, &r.hosted, &r.outside, &r.attachment].into_iter().flatten().map(String::len).sum::<usize>()).sum();
    assert!(carried > 0 && carried <= TEAMS_HTML_MAX, "{carried}");
}

#[test]
fn an_address_with_bidi_characters_is_no_link() {
    for c in ['\u{202E}', '\u{2066}', '\u{200F}'] {
        let t = teams_html(&format!("<a href=\"https://x.example/{c}gpj.exe\">t</a><img src=\"https://x.example/{c}p.png\">"), C, "1");
        assert_eq!(run(&t, "t").link, None, "{c:?}");
        assert!(runs(&t).iter().all(|r| r.outside.is_none()), "{c:?}");
    }
}

#[test]
fn link_texts_lose_control_and_bidi_characters() {
    assert_eq!(teams_html("<a href=\"https://x.example/\">a\u{202E}b</a>", C, "1").plain(), "ab");
    assert_eq!(teams_html("<at id=\"1\">a\u{2066}b\u{7}</at>", C, "1").plain(), "ab");
    // Elsewhere control characters go, but a right-to-left mark in text is the writer's.
    assert_eq!(teams_html("a\u{1}\u{200F}b", C, "1").plain(), "a\u{200F}b");
}

#[test]
fn plain_text_is_kept_as_it_came() {
    let t = teams_text("line 1\r\n  line\u{7} 2");
    assert_eq!(t.plain(), "line 1\n  line 2");
    assert!(teams_text("").blocks.is_empty());
    assert!(teams_text(&"é".repeat(300 << 10)).plain().len() <= TEAMS_HTML_MAX);
}

/// Pieces of the subset, and of what a hostile sender adds, put together at random.
const PIECES: &[&str] = &[
    "<p>",
    "</p>",
    "<b>",
    "</b>",
    "<i>",
    "</i>",
    "<ul>",
    "<ol>",
    "<li>",
    "</li>",
    "</ul>",
    "<blockquote>",
    "<pre>",
    "</pre>",
    "<br>",
    "<a href=\"https://x.example/\">",
    "<a href=\"javascript:x\">",
    "</a>",
    "<at id=\"3\">",
    "</at>",
    "<emoji alt=\"\u{202E}😀\">",
    "</emoji>",
    "<attachment id=\"a\">",
    "</attachment>",
    "<script>",
    "</script>",
    "<style>",
    "<td>",
    "<tr>",
    "<img src=\"https://graph.microsoft.com/v1.0/chats/19:a@thread.v2/messages/1/hostedContents/aGk=/$value\">",
    "<img src=\"https://graph.microsoft.com/v1.0/chats/19:a@thread.v2/messages/1/hostedContents/a%2Fb/$value\">",
    "<img src=\"HTTP://x.example/p.png\">",
    "&nGt;",
    "&nbsp;",
    " ",
    "\n",
    "word",
    "é",
    "\u{202E}",
    "\u{1}",
    "<",
    "\"",
    "<!--",
];

proptest::proptest! {
    #[test]
    fn any_mix_of_the_subset_stays_within_its_caps(picks in proptest::collection::vec(0..PIECES.len(), 0..400)) {
        let html: String = picks.iter().filter_map(|&i| PIECES.get(i).copied()).collect();
        let t = teams_html(&html, C, "1");
        proptest::prop_assert!(t.plain().len() <= TEAMS_HTML_MAX);
        let all = runs(&t);
        proptest::prop_assert!(all.len() <= TEAMS_RUNS);
        for r in all {
            if let Some(id) = &r.hosted {
                proptest::prop_assert!(id == "aGk=", "{id:?}");
            }
            for a in [&r.link, &r.outside].into_iter().flatten() {
                proptest::prop_assert!(a.starts_with("https://") || a.starts_with("http://"), "{a:?}");
            }
            if r.link.is_some() || r.mention.is_some() {
                proptest::prop_assert!(!r.text.contains('\u{202E}'), "{:?}", r.text);
            }
            proptest::prop_assert!(!r.text.chars().any(|c| c.is_control() && c != '\n' && c != '\t'), "{:?}", r.text);
        }
        for b in &t.blocks {
            proptest::prop_assert!(!b.runs.is_empty());
        }
    }
}

// The writer: what the user types in a chat's box, as the HTML sent to Teams.

/// The writer's own markup: its tags and the entities it writes.
const OWN: &[&str] = &[
    "<p>", "</p>", "<b>", "</b>", "<i>", "</i>", "<u>", "</u>", "<s>", "</s>", "<code>", "</code>", "<br>", "</a>", "&amp;", "&lt;", "&gt;",
    "&quot;", "&#39;", "&nbsp;",
];

/// The first piece of `html` that isn't the writer's own markup, if any: a `<`, `>`, `"`, `'` or
/// `&` that doesn't begin one of its tags, a link to the web or one of its entities. Escaped text
/// may say `src=` or `style=`; this sees whether any attribute but a web link's `href` is written.
fn foreign(html: &str) -> Option<&str> {
    let mut rest = html;
    while let Some(i) = rest.find(['<', '>', '"', '\'', '&']) {
        let at = &rest[i..];
        let Some(len) = OWN.iter().find(|own| at.starts_with(**own)).map(|own| own.len()).or_else(|| link_tag(at)) else { return Some(at) };
        rest = &at[len..];
    }
    None
}

/// The length of the tag `at` starts with when it's a link to the web, its address escaped.
fn link_tag(at: &str) -> Option<usize> {
    let href = at.strip_prefix("<a href=\"")?;
    let end = href.find('"')?;
    let address = &href[..end];
    let web = address.starts_with("http://") || address.starts_with("https://");
    (web && !address.contains(['<', '>', '\'']) && href[end..].starts_with("\">")).then(|| "<a href=\"".len() + end + 2)
}

/// A message of one paragraph of `runs`.
fn message(runs: Vec<Run>) -> ChatText {
    ChatText { blocks: vec![Block { kind: BlockKind::Paragraph, runs }] }
}

/// A run of `text` in no style.
fn plain(text: &str) -> Run {
    Run { text: text.into(), ..Run::default() }
}

#[test]
fn the_check_of_the_writers_markup_sees_any_other() {
    for other in [
        "<p><img src=x></p>",
        "<p style=\"x\">a</p>",
        "<a href=\"javascript:x\">a</a>",
        "<a href=\"https://x.example/\" onclick=\"y\">a</a>",
        "<script>x()</script>",
        "a > b",
        "'a'",
        "&#60;",
    ] {
        assert!(foreign(other).is_some(), "{other}");
    }
    assert_eq!(foreign("<p><a href=\"https://x.example/?a=1&amp;b\"><b>&lt;img src=&quot;x&quot;&gt;</b></a><br>&nbsp;</p>"), None);
}

#[test]
fn the_users_text_is_escaped_and_reads_back_the_same() {
    let typed = "a < b && c > \"d\" 'e' <img src=\"https://evil.example/x.png\"> <script>x()</script>";
    let html = teams_write(&teams_compose(typed, &[], &[])).unwrap();
    for never in ["<img", "<script"] {
        assert!(!html.contains(never), "{html}");
    }
    // `src=` stays in the text, escaped: no tag but the writer's own is written.
    assert_eq!(foreign(&html), None, "{html}");
    assert_eq!(teams_html(&html, C, "1").plain(), typed);
}

#[test]
fn html_escape_writes_the_five_marks_as_entities() {
    assert_eq!(
        html_escape("<b>x</b> & <img src=\"https://e.example/a.png\"> 'q'"),
        "&lt;b&gt;x&lt;/b&gt; &amp; &lt;img src=&quot;https://e.example/a.png&quot;&gt; &#39;q&#39;"
    );
}

/// The desktop's notifications on Linux and the BSDs, whose body the desktop may read as markup,
/// go through `html_escape`: a stranger's tags are shown as they were written, never drawn.
#[test]
fn markup_is_escaped_for_the_desktop() {
    assert_eq!(
        html_escape("<b>x</b> & <img src=\"https://e.example/a.png\">"),
        "&lt;b&gt;x&lt;/b&gt; &amp; &lt;img src=&quot;https://e.example/a.png&quot;&gt;"
    );
}

#[test]
// A list of one bold (or italic) range, as the box sends it: not the numbers in that range.
#[allow(clippy::single_range_in_vec_init)]
fn bold_and_italic_ranges_make_runs() {
    let html = teams_write(&teams_compose("one two three", &[4..7], &[8..13])).unwrap();
    assert_eq!(html, "<p>one <b>two</b> <i>three</i></p>");
}

#[test]
fn lines_are_paragraphs_and_a_blank_line_is_kept() {
    assert_eq!(teams_write(&teams_compose("\na\n\nb\n\n", &[], &[])).unwrap(), "<p>a</p><p>&nbsp;</p><p>b</p>");
}

#[test]
fn control_characters_go_and_a_line_of_spaces_is_blank() {
    assert_eq!(teams_write(&teams_compose(" \r\na\u{7}b\r\n \t \r\nc\n  ", &[], &[])).unwrap(), "<p>ab</p><p>&nbsp;</p><p>c</p>");
}

#[test]
fn only_web_links_are_written_as_links() {
    let linked = |link: &str| message(vec![Run { text: "t".into(), link: Some(link.into()), ..Run::default() }]);
    for other in ["javascript:alert(1)", "file:///etc/passwd", "data:text/html,x", "https://x.example/ a", "https://x.example/\u{202E}"] {
        assert_eq!(teams_write(&linked(other)).unwrap(), "<p>t</p>", "{other}");
    }
    let html = teams_write(&linked("https://x.example/?q=\"a\"&b=1")).unwrap();
    assert_eq!(html, "<p><a href=\"https://x.example/?q=&quot;a&quot;&amp;b=1\">t</a></p>");
    assert_eq!(run(&teams_html(&html, C, "1"), "t").link.as_deref(), Some("https://x.example/?q=\"a\"&b=1"));
}

#[test]
fn styles_and_line_breaks_are_tags_and_mentions_and_pictures_their_text() {
    let all = Run { text: "a\nb".into(), bold: true, italic: true, underline: true, strike: true, code: true, ..Run::default() };
    assert_eq!(teams_write(&message(vec![all])).unwrap(), "<p><b><i><u><s><code>a<br>b</code></s></u></i></b></p>");
    let mention = Run { text: "Ana".into(), mention: Some(0), ..Run::default() };
    let hosted = Run { hosted: Some("aGk=".into()), ..Run::default() };
    let outside = Run { outside: Some("https://e.example/a.png".into()), bold: true, ..Run::default() };
    let attachment = Run { text: "f".into(), attachment: Some("a1".into()), ..Run::default() };
    assert_eq!(teams_write(&message(vec![mention, hosted.clone(), outside, attachment])).unwrap(), "<p>Anaf</p>");
    assert_eq!(teams_write(&message(vec![hosted])).unwrap(), "<p>&nbsp;</p>");
}

#[test]
#[allow(clippy::single_range_in_vec_init)] // a list of one italic range
fn ranges_off_char_boundaries_or_the_text_are_ignored() {
    assert_eq!(teams_compose("éa", &[1..2, 0..99], &[]), message(vec![plain("éa")]));
    assert_eq!(teams_write(&teams_compose("éa", &[], &[0..2])).unwrap(), "<p><i>é</i>a</p>");
}

#[test]
fn past_80_kib_is_too_long_counted_after_escaping() {
    assert!(teams_write(&teams_compose(&"a".repeat(81_000), &[], &[])).is_ok());
    // 21,000 × "&lt;" = 84,000 bytes.
    let written = teams_write(&teams_compose(&"<".repeat(21_000), &[], &[]));
    assert!(matches!(written, Err(TooLong { .. })));
    assert!(written.is_err_and(|e| e.bytes > SEND_MAX));
}

#[test]
fn past_the_runs_cap_what_is_kept_is_still_too_long() {
    // Every other character bold: a run each.
    let text = "ab".repeat(TEAMS_RUNS);
    let bold: Vec<_> = (0..TEAMS_RUNS).map(|k| 2 * k..2 * k + 1).collect();
    let t = teams_compose(&text, &bold, &[]);
    assert_eq!(runs(&t).len(), TEAMS_RUNS);
    assert!(teams_write(&t).is_err());
    // Blank lines between two words count too; after the last word, or before the first, they go.
    let gap = "\n".repeat(TEAMS_RUNS + 10);
    assert!(teams_write(&teams_compose(&format!("a{gap}b"), &[], &[])).is_err());
    assert_eq!(teams_write(&teams_compose(&format!("a{gap}"), &[], &[])).unwrap(), "<p>a</p>");
    assert_eq!(teams_write(&teams_compose(&format!("{gap}b"), &[], &[])).unwrap(), "<p>b</p>");
}

/// Each character of `t` with its styles (bold, italic, underline, strike, code) and its link, a
/// line break between two blocks.
fn styled(t: &ChatText) -> Vec<(char, [bool; 5], Option<&str>)> {
    let mut out = Vec::new();
    for (i, b) in t.blocks.iter().enumerate() {
        if i > 0 {
            out.push(('\n', [false; 5], None));
        }
        for r in &b.runs {
            out.extend(r.text.chars().map(|c| (c, [r.bold, r.italic, r.underline, r.strike, r.code], r.link.as_deref())));
        }
    }
    out
}

/// A run of words one space apart (letters, the marks HTML escapes, an accented letter, an
/// emoji), in any styles, a link to the web or none.
fn any_run() -> impl Strategy<Value = Run> {
    let words = proptest::string::string_regex("[a-z<>&\"'é😀]{1,4}( [a-z<>&\"'é😀]{1,4}){0,3}").unwrap();
    (words, proptest::array::uniform5(proptest::bool::ANY), proptest::option::of(0..100u8)).prop_map(
        |(text, [bold, italic, underline, strike, code], link)| Run {
            text,
            bold,
            italic,
            underline,
            strike,
            code,
            link: link.map(|n| format!("https://x.example/{n}")),
            ..Run::default()
        },
    )
}

/// A message of 1 to 20 paragraphs of 1 to 10 runs.
fn any_message() -> impl Strategy<Value = ChatText> {
    let paragraph = proptest::collection::vec(any_run(), 1..=10).prop_map(|runs| Block { kind: BlockKind::Paragraph, runs });
    proptest::collection::vec(paragraph, 1..=20).prop_map(|blocks| ChatText { blocks })
}

/// Byte ranges, some off the text, some the wrong way round.
fn any_ranges() -> impl Strategy<Value = Vec<std::ops::Range<usize>>> {
    proptest::collection::vec((0..400usize, 0..400usize).prop_map(|(a, b)| a..b), 0..8)
}

proptest::proptest! {
    #[test]
    fn written_text_reads_back_the_same(t in any_message()) {
        let html = teams_write(&t).map_err(|e| proptest::test_runner::TestCaseError::fail(format!("{e:?}")))?;
        proptest::prop_assert_eq!(foreign(&html), None, "{}", html);
        let back = teams_html(&html, C, "1");
        proptest::prop_assert_eq!(styled(&back), styled(&t), "{}", html);
    }

    #[test]
    fn any_text_and_ranges_are_written_safely(text in proptest::arbitrary::any::<String>(), bold in any_ranges(), italic in any_ranges()) {
        let t = teams_compose(&text, &bold, &italic);
        let all = runs(&t);
        proptest::prop_assert!(all.len() <= TEAMS_RUNS);
        for r in all {
            proptest::prop_assert!(!r.text.is_empty() && !r.text.contains(char::is_control), "{:?}", r.text);
        }
        proptest::prop_assert!(t.blocks.first().is_none_or(|b| !b.runs.is_empty()) && t.blocks.last().is_none_or(|b| !b.runs.is_empty()));
        // Every character is kept but control characters.
        let words = |s: &str| s.chars().filter(|c| !c.is_whitespace() && !c.is_control()).collect::<String>();
        proptest::prop_assert_eq!(words(&t.plain()), words(&text));
        match teams_write(&t) {
            Ok(html) => {
                proptest::prop_assert!(html.len() <= SEND_MAX);
                proptest::prop_assert_eq!(foreign(&html), None, "{}", html);
                proptest::prop_assert_eq!(words(&teams_html(&html, C, "1").plain()), words(&text));
            }
            Err(TooLong { bytes }) => proptest::prop_assert!(bytes > SEND_MAX),
        }
    }
}
