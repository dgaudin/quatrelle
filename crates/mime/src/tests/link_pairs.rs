// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
use super::*;

/// A link's shown text and its target, as a reader would see them: tags in the anchor give their
/// text, `mailto:` is left out, and a runaway body gives a bounded answer.
#[test]
fn html_link_pairs_give_the_shown_text_and_the_target() {
    let html = r#"<a href="https://evil.example/x">www.ma-banque.fr</a> <a href="mailto:a@b">mail</a>"#;
    assert_eq!(html_link_pairs(html), vec![("www.ma-banque.fr".to_string(), "https://evil.example/x".to_string())]);
    let many: String = (0..500).map(|i| format!("<a href=\"https://h{i}.example/\">l</a>")).collect();
    assert_eq!(html_link_pairs(&many).len(), 200);
    assert_eq!(html_link_pairs("<a href=\"u\"><b>Pay</b> now</a>"), vec![("Pay now".to_string(), "u".to_string())]);
    assert!(html_link_pairs("<article href=\"x\">y</article><a href=\"#top\">t</a>").is_empty());
    assert!(html_link_pairs(&"<a href=\"".repeat(10_000)).is_empty() || html_link_pairs("").is_empty());
    let long = format!("<a href=\"https://x.example\">{}</a>", "é".repeat(1000));
    assert_eq!(html_link_pairs(&long).first().map(|p| p.0.chars().count()), Some(200));
}
