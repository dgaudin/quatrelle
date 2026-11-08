// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
use super::*;

/// An invoice's total sits in a `<tfoot>` written before its rows (the probe for
/// `html/footers.rs`): it shows in the message's text, at the end of the table.
#[test]
fn a_table_footer_with_the_total_shows_in_the_text() {
    let html = "<table><thead><tr><th>Item</th><th>Price</th></tr></thead><tfoot><tr><td>Total</td><td>42.00</td></tr></tfoot>\
                <tbody><tr><td>Widget</td><td>42.00</td></tr></tbody></table><p>Thanks</p>";
    let text = html_to_text(html);
    assert!(text.contains("Total") && text.contains("42.00"), "{text:?}");
    assert!(text.find("Widget") < text.find("Total"), "{text:?}");
    let p = parse(format!("Content-Type: text/html\r\n\r\n{html}").as_bytes()).unwrap();
    assert!(p.text.contains("Total"), "{:?}", p.text);
}
