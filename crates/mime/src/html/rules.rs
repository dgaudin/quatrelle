// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The lines across a message: an `<hr>`, and the top border Outlook draws over each message a
//! reply quotes (`border:none;border-top:solid #E1E1E1 1.0pt` around its « De : » lines). The
//! text converter keeps neither (an `<hr>` came out as a row of dashes, dropped with the tables'
//! borders): they're marked before it, and made rule lines after it.

use super::*;

/// Where a line goes across the message, alone in a paragraph of its own.
pub(super) const RULE_MARK: char = '\u{E002}';

/// Most lines marked in one message.
const MAX_RULES: usize = 500;

/// `html` with a paragraph of [`RULE_MARK`] in place of each `<hr>`, and before each block whose
/// only border is its top one. Comments are passed over (an `<hr>` kept for one mail client).
pub(super) fn mark_rules(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    if !lower.contains("<hr") && !lower.contains("border-top") {
        return html.to_string();
    }
    let mark = format!("<p>{RULE_MARK}</p>");
    let mut out = String::with_capacity(html.len() + 64);
    // Copied up to `from`; looking from `at`.
    let (mut from, mut at, mut marked) = (0, 0, 0);
    while marked < MAX_RULES {
        let Some(i) = lower.get(at..).and_then(|r| r.find('<')).map(|i| i + at) else { break };
        let rest = lower.get(i + 1..).unwrap_or("");
        if rest.starts_with("!--") {
            at = rest.find("-->").map_or(lower.len(), |e| i + 1 + e + 3);
            continue;
        }
        let Some(end) = rest.find('>').map(|e| i + 1 + e) else { break };
        at = end + 1;
        let name = rest.split(|c: char| !c.is_ascii_alphanumeric()).next().unwrap_or("");
        let tag = html.get(i..end).unwrap_or("");
        match name {
            "hr" => {
                out.push_str(html.get(from..i).unwrap_or(""));
                out.push_str(&mark);
                from = end + 1;
            }
            "div" | "p" if top_border_only(tag) => {
                out.push_str(html.get(from..i).unwrap_or(""));
                out.push_str(&mark);
                from = i;
            }
            _ => continue,
        }
        marked += 1;
    }
    out.push_str(html.get(from..).unwrap_or(""));
    out
}

/// Whether a tag's style draws a line along its top edge and no other.
fn top_border_only(tag: &str) -> bool {
    let Some(style) = tag_attr(tag, "style") else { return false };
    let decls = crate::css::declarations(style);
    let value = |prop: &str| decls.iter().rev().find(|d| d.prop == prop).map(|d| d.value.to_ascii_lowercase());
    let drawn = |prop: &str| value(prop).is_some_and(|v| draws_a_line(&v));
    (drawn("border-top") || drawn("border-top-style")) && !["border", "border-left", "border-right", "border-bottom"].into_iter().any(drawn)
}

/// Whether a border's value draws a line: a style that shows, and no width of nothing.
fn draws_a_line(v: &str) -> bool {
    const SHOWN: [&str; 8] = ["solid", "dotted", "dashed", "double", "groove", "ridge", "inset", "outset"];
    let words: Vec<&str> = v.split_whitespace().collect();
    words.iter().any(|w| SHOWN.contains(w)) && !words.iter().any(|w| matches!(*w, "none" | "hidden" | "0" | "0px" | "0pt" | "0cm" | "0in"))
}
