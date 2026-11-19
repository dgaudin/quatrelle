// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A title over more than one line: each of its lines is the title's, and the line break that
//! ends a title adds no line.

use super::*;

/// What a message shows: each line's words, and the title level of the line.
fn shown(html: &str) -> Vec<(String, u8)> {
    html_to_rich(&layout::inline_css(html))
        .iter()
        .filter(|l| !l.is_empty())
        .map(|l| (l.iter().map(|s| s.text.as_str()).collect::<String>(), l.iter().map(|s| s.heading).max().unwrap_or(0)))
        .collect()
}

/// `<h3>Alpha<br>Beta</h3>`: both lines are the title's; the second came out `### Beta`, its
/// marks written as words.
#[test]
fn a_title_on_two_lines_is_a_title_on_both() {
    let lines = shown("<h3>Alpha<br>Beta</h3><p>Next</p>");
    assert_eq!(lines, vec![("Alpha".into(), 3), ("Beta".into(), 3), ("Next".into(), 0)]);
    let lines = shown("<h2>Alpha<br>Beta<br></h2><p>Next</p>");
    assert_eq!(lines, vec![("Alpha".into(), 2), ("Beta".into(), 2), ("Next".into(), 0)]);
}

/// A greeting sized as a title (18 pixels, so read as one) and ended by a line break, as a
/// mailing's template writes each paragraph: the title alone, no `###` line under it.
#[test]
fn a_sized_title_ended_by_a_line_break_adds_no_line() {
    let html = "<html><head><style>a { padding: 0; }</style></head><body><table width=\"700\"><tr><td>\
                <div style=\"text-align: left;\"><span style=\"font-size: 13.5pt;\"><span>Dear customer,</span><br /></span></div>\
                </td></tr></table><p>Your parcel is ready.</p></body></html>";
    let lines = shown(html);
    assert!(lines.iter().all(|(t, _)| !t.contains('#')), "{lines:?}");
    assert_eq!(lines.first().map(|(t, _)| t.as_str()), Some("Dear customer,"), "{lines:?}");
    assert_eq!(lines.len(), 2, "{lines:?}");
}

/// A paragraph that starts with `#` after a title stays as it's written: only the title's own
/// lines lose their marks.
#[test]
fn a_paragraph_starting_with_marks_after_a_title_keeps_them() {
    let lines = shown("<h3>Alpha</h3><p>### not a title</p>");
    assert_eq!(lines, vec![("Alpha".into(), 3), ("### not a title".into(), 0)]);
}
