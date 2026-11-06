// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! An HTML body: converted to text, to formatted lines and to columns, and searched for its
//! pictures and links; quoted under a reply as it came (its columns as blocks of text, cleaned
//! and its pictures renamed, its layout written out again); plus the plain-text markup
//! Quatrelle writes itself.
#![no_main]

use libfuzzer_sys::fuzz_target;
use quatrelle_mime::{self as mime, markup, rich};

fuzz_target!(|input: &[u8]| {
    let page = String::from_utf8_lossy(input);
    // The ways Quatrelle reads a received HTML body.
    let formatted = mime::html_to_rich(&page);
    let _ = mime::remote_images(&formatted);
    let _ = mime::html_to_text(&page);
    let _ = mime::html_links(&page);
    let _ = mime::html_images(&page);
    let _ = mime::layout::inline_css(&page);
    let layout = mime::layout::html_layout(&page);
    // Quoted under a reply: its columns made blocks of text to edit, its HTML as it goes out
    // (pictures given `cid:`s by their addresses, the input's own words standing in for them),
    // and its layout written out again around its blocks of text.
    let _ = mime::layout::flatten(&layout);
    let words: Vec<String> = page.split_whitespace().take(8).map(str::to_string).collect();
    let given: Vec<(String, String)> = words.iter().map(|w| (w.clone(), format!("{w}@x"))).collect();
    let _ = mime::layout::quote_html(&page, &given, &words);
    let blocks = mime::layout::text_blocks(&layout);
    let _ = mime::layout::to_html(&layout, &mut |n| {
        blocks.get(n).map(|(lines, align)| mime::layout::cell_text(lines, *align)).map(|(t, s)| rich::to_html(&t, &s)).unwrap_or_default()
    });
    let _ = mime::split_marked(&page, mime::QUOTE_CLASS);
    // Pasting it into a draft, and the round trip back out.
    let (text, styles) = mime::pasted_html(&page);
    let _ = rich::to_text(&text, &styles);
    let _ = rich::to_html(&text, &styles);
    // The same input taken as Quatrelle's own plain-text markup.
    let _ = rich::from_markup(&page);
    let _ = markup::to_text(&page);
    let _ = markup::to_html(&page);
});
