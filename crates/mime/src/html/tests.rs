// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! HTML pasted into a message: what it costs, however much it brings.

use super::*;

/// Found in review: HTML pasted with thousands of pictures named by `cid:` had each given its
/// style in a pass over the whole text (its palette compacted each time): 10,000 pictures before
/// 2 MB of text froze the window, before those the message hasn't were taken out (in one pass,
/// [`rich::retain`]). The pictures kept are few, styled in one pass.
#[test]
fn thousands_of_pictures_pasted_with_much_text_cost_a_few_passes() {
    let pictures: String = (0..10_000).map(|n| format!("<img src=\"cid:n{n}\">")).collect();
    let words = "Plan for Monday, and the week after. ".repeat((2 << 20) / 37);
    let html = format!("<p>{pictures}</p><p>{words}</p>");
    let chars = words.chars().count() + 10_000;
    let ((text, styles), walked) = rich::work::at_most(8 * chars, || {
        let (mut text, mut styles, _) = pasted_html_with_pictures(&html);
        // As the paste does: the pictures the message hasn't go.
        rich::retain(&mut text, &mut styles, |c, s| c != rich::PICTURE || s.picture.as_deref().is_some_and(|cid| cid == "n0"));
        (text, styles)
    });
    assert!(text.contains("Plan for Monday, and the week after."), "the text came: {walked} characters walked");
    let kept: Vec<usize> = text.chars().enumerate().filter(|(_, c)| *c == rich::PICTURE).map(|(i, _)| i).collect();
    assert_eq!(kept.len(), 1, "the picture the message has stays");
    assert_eq!(kept.first().and_then(|&i| styles.style(i).picture), Some("n0".to_string()));
}

/// A `>` inside a quoted attribute doesn't end the tag: the rest of an `<hr title="x>y">` isn't
/// left in the text after its rule line.
#[test]
fn a_gt_inside_a_quoted_attribute_doesnt_end_a_rule_tag() {
    assert_eq!(mark_rules("<p>a</p><hr title=\"x>y\"><p>b</p>"), format!("<p>a</p><p>{RULE_MARK}</p><p>b</p>"));
}

/// A link's address reads its character references once, as every other pass reads them:
/// `&amp;#38;` is `&#38;`, not `&` (the chain of replacements read it twice), and `&#x3D;` is
/// `=`. A picture's address and a tracker's are read the same way.
#[test]
fn every_pass_reads_an_address_alike() {
    let src = "https://pictures.example/p.png?a=1&b=2";
    let html = r#"<p>Hi</p><img alt="Logo" src="https://pictures.example/p.png?a=1&#38;b=2"><a href="https://links.example/?q=x&amp;#38;y">go</a><img src="https://t.example/o.gif?a=1&#x26;b=2" width="1" height="1">"#;
    assert_eq!(html_images(html), [src, "https://t.example/o.gif?a=1&b=2"]);
    assert_eq!(html_links(html), ["https://links.example/?q=x&#38;y"]);
    assert_eq!(crate::trackers::trackers(html).pictures, ["https://t.example/o.gif?a=1&b=2"]);
    assert_eq!(html_links(r#"<a href="https://links.example/?a&#x3D;1">go</a>"#), ["https://links.example/?a=1"]);
}

/// The marks the passes write are private-use characters mail doesn't carry: one a message
/// writes itself is taken out, not read as ours (a typeface mark in pasted text was taken for
/// MailCraft's own). And the signature's mark is its own character, not the rule line's.
#[test]
fn marks_a_message_writes_are_taken_out() {
    let r = html_to_rich("<p>a\u{E000}b\u{E001}c\u{E020}\u{E100}d\u{E002}e</p>");
    let text: String = r.iter().flat_map(|l| l.iter().map(|sp| sp.text.as_str())).collect();
    assert_eq!(text, "abcde", "{text:?}");
    assert_ne!(crate::rich::SIGNATURE, '\u{E002}', "the rule line's mark");
}

/// The marks written as numeric references (decimal, hex, leading zeros, `;` or not) are taken
/// out too: `&#xE002;` gives no rule line, `&#xE000;` no heading, font marks around text don't
/// size it as the first font's, and a signature's mark in pasted text isn't Quatrelle's.
#[test]
fn marks_written_as_numeric_references_are_taken_out() {
    assert_eq!(without_marks("a&#xE002;b"), "ab");
    assert_eq!(without_marks("a&#57346;b"), "ab", "57346 = 0xE002");
    assert_eq!(without_marks("a&#x0000E002;b"), "ab", "leading zeros");
    assert_eq!(without_marks("a&#57346b"), "ab", "no semicolon");
    assert_eq!(without_marks("a&#xE002b"), "a&#xE002b", "0xE002B is another character");
    assert_eq!(without_marks("a&#x41;b"), "a&#x41;b", "not a mark: as written");
    assert_eq!(without_marks("a&xE002;b"), "a&xE002;b", "not a reference");
    // No rule line.
    let r = html_to_rich("<p>&#xE002;</p>");
    let text: String = r.iter().flat_map(|l| l.iter().map(|sp| sp.text.as_str())).collect();
    assert!(text.trim().is_empty(), "{text:?}");
    // No heading.
    let r = html_to_rich("<p>&#xE000;Pay now</p>");
    let text: String = r.iter().flat_map(|l| l.iter().map(|sp| sp.text.as_str())).collect();
    assert_eq!(text.trim(), "Pay now", "{text:?}");
    // Font marks around text don't draw it as the first font's: same as without them.
    let words = "word ".repeat(200);
    let drawn =
        |html: String| -> String { html_to_rich(&html).iter().flat_map(|l| l.iter().map(|sp| format!("{:?}{:?}", sp.text, sp.size))).collect() };
    assert_eq!(
        drawn(format!("<p style=\"font-size:2px\">&#xE020;&#xE100;{words}&#xE021;</p>")),
        drawn(format!("<p style=\"font-size:2px\">{words}</p>"))
    );
    // Pasted: no signature mark.
    let (text, _) = pasted_html("<p>a&#xE005;b</p>");
    assert!(!text.contains(crate::rich::SIGNATURE), "{text:?}");
    assert_eq!(text, "ab");
}

/// A picture's size is the one CSS picks: the last declaration marked `!important` over a
/// later plain one (`width:600px !important; width:1px` is 600 wide, not 1).
#[test]
fn a_pictures_size_is_the_one_css_picks() {
    let html = r#"<img src="https://x.example/a.png" style="width:600px !important; width:1px"><img src="https://x.example/b.png" style="width:1px; width:600px">"#;
    let sizes = picture_sizes(html);
    assert_eq!(sizes[0].1.width, Some(layout::Width::Px(600)), "{sizes:?}");
    assert_eq!(sizes[1].1.width, Some(layout::Width::Px(600)), "{sizes:?}");
}
