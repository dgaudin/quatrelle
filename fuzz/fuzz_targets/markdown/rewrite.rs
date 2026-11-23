// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A language model's answer read for Rewrite, with 0 to 4 pictures sent: never a panic, each
//! picture back exactly once (never lost), no token of the sent nonce and no internal mark left,
//! and a bounded result whatever the answer.
#![no_main]

use libfuzzer_sys::fuzz_target;
use quatrelle_mime::{self as mime, rich};

fuzz_target!(|input: &[u8]| {
    let pictures = input.len() % 5;
    let mut before = String::from("Dear");
    for _ in 0..pictures {
        before.push(' ');
        before.push(rich::PICTURE);
        before.push_str(" and");
    }
    before.push_str(" Zoé.");
    let mut styles = rich::Styles::plain(&before);
    for (i, _) in before.chars().enumerate().filter(|(_, c)| *c == rich::PICTURE) {
        styles.restyle(&before, i..i + 1, |s| rich::Style { picture: Some(format!("logo-{i}@studio.example")), ..s.clone() });
    }
    let (sent, images) = rich::outbound(&before, &styles, 7);
    assert_eq!(images.len(), pictures);
    if pictures > 0 {
        assert!(sent.contains("QUATRELLE-IMAGE-7-"));
    }
    let answer = String::from_utf8_lossy(input);
    let (back, _) = rich::inbound(&answer, rich::Style::default(), &images, &before, &styles);
    assert_eq!(back.chars().filter(|c| *c == rich::PICTURE).count(), pictures, "every picture once: {back:?}");
    assert!(!back.contains("QUATRELLE-IMAGE-7-"), "no sent marker left: {back:?}");
    assert!(!back.chars().any(|c| ('\u{E000}'..='\u{E06F}').contains(&c)), "no internal mark left: {back:?}");
    assert!(back.chars().count() <= mime::MARKDOWN_MAX + pictures + 8, "{} characters", back.chars().count());
});
