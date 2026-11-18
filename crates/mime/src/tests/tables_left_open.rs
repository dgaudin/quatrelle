// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
use super::*;

/// A megabyte of tables left open one in another, a cell, a box and a link at a time, opens at
/// once, its words and links kept (the probe for `html/deep.rs`).
#[test]
fn a_megabyte_of_tables_left_open_is_read_at_once() {
    let unit = r#"<table><tr><td><div><a href="https://x.example/a">link</a>"#;
    let html = unit.repeat(1_000_000 / unit.len());
    let started = std::time::Instant::now();
    let p = parse(format!("Content-Type: text/html\r\n\r\n{html}").as_bytes()).unwrap();
    let took = started.elapsed();
    eprintln!("tables left open: {took:?}");
    assert!(took < std::time::Duration::from_secs(5), "{took:?}");
    assert!(p.text.contains("link") && p.text.contains("https://x.example/a"));
}

/// A MailTip's text (any colleague writes their automatic reply) that nests deeper than any mail
/// does is read flat, as the other conversions read it: tables of two cells, one in another,
/// within the 16 KB read, came out as more than a megabyte of rules, the words lost among them.
#[test]
fn a_tip_nested_deeper_than_mail_is_read_flat() {
    let html = format!("{}Back on Monday.", "<table><tr><td>x</td><td>".repeat(600));
    let text = html_to_plain(&html);
    assert!(text.len() < 4 * html.len(), "{} bytes out of {}", text.len(), html.len());
    assert!(text.contains("Back on Monday"), "{:?}", text.get(..200));
}
