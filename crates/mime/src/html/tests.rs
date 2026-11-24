// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! HTML pasted into a message: what it costs, however much it brings.

use super::*;

mod pictures;
mod rules;

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

/// The marks the passes write are private-use characters mail doesn't carry: one a message
/// writes itself is taken out, not read as ours (a typeface mark in pasted text was taken for
/// Quatrelle's own). And the signature's mark is its own character, not the rule line's.
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
