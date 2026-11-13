// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A received HTML body written as Markdown for the reading pane's Markdown view, its links as
//! the message has them and cleaned: within the cap, whatever the HTML; then that Markdown read
//! back as pasted Markdown is.
#![no_main]

use libfuzzer_sys::fuzz_target;
use quatrelle_mime as mime;

fuzz_target!(|input: &[u8]| {
    let page = String::from_utf8_lossy(input);
    let md = mime::html_markdown(&page);
    assert!(md.len() <= mime::MARKDOWN_WRITTEN_MAX, "{} bytes", md.len());
    let rich = mime::html_to_rich(&page);
    let cleaned = mime::rich_markdown(&rich, true);
    assert!(cleaned.len() <= mime::MARKDOWN_WRITTEN_MAX, "{} bytes", cleaned.len());
    // What it wrote, read again as Markdown.
    let _ = mime::markdown_html(&md);
    let _ = mime::pasted_markdown(&md);
});
