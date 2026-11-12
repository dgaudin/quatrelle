// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The Teams HTML subset: one test per rule, each with the hostile input it guards against.

use super::*;
use quatrelle_model::pim::teams::BlockKind;

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
