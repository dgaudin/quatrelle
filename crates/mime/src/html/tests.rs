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
