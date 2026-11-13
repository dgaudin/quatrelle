// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Text pasted into a message, read as Markdown: told from ordinary text, written as HTML within
//! its caps (no raw HTML, no picture, a link only to the web or to mail), and pasted as the
//! editor's text and styles with none of the line marks left in them, then sent.
#![no_main]

use libfuzzer_sys::fuzz_target;
use quatrelle_mime::{self as mime, rich};

fuzz_target!(|input: &[u8]| {
    let text = String::from_utf8_lossy(input);
    let _ = mime::looks_like_markdown(&text);
    let html = mime::markdown_html(&text);
    assert!(html.len() <= mime::MARKDOWN_HTML_MAX, "{} bytes of HTML", html.len());
    let lower = html.to_ascii_lowercase();
    assert!(!lower.contains("<script") && !lower.contains("<img") && !lower.contains("<iframe"), "{html}");
    for (i, _) in lower.match_indices("href=\"") {
        let to = lower.get(i + 6..).unwrap_or("");
        assert!(to.starts_with("http://") || to.starts_with("https://") || to.starts_with("mailto:"), "{html}");
    }
    if let Some((pasted, styles)) = mime::pasted_markdown(&text) {
        assert!(!pasted.chars().any(|c| ('\u{E040}'..='\u{E06F}').contains(&c)), "a line mark left in {pasted:?}");
        let _ = rich::to_html(&pasted, &styles);
        let _ = rich::to_text(&pasted, &styles);
    }
});
