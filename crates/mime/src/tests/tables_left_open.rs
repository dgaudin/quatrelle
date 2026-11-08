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
