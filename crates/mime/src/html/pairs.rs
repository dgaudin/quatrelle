// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! An HTML body's links as pairs, the text shown and where it goes: a link that shows one web
//! address and goes to another is how a phishing page is hidden. Bounded in what it reads and
//! returns.

use super::HTML_MAX;

/// The most pairs returned.
const MAX_PAIRS: usize = 200;
/// The most characters kept of a link's shown text.
const MAX_SHOWN: usize = 200;

/// The links of an HTML body as (shown text, target): the text between `<a …>` and `</a>` with
/// its tags taken out and its spaces collapsed, and the `href`. At most 200 pairs, the text cut
/// at 200 characters; `mailto:` and `#…` targets and empty ones are left out.
pub fn html_link_pairs(html: &str) -> Vec<(String, String)> {
    let html = match html.char_indices().nth(HTML_MAX) {
        Some((i, _)) => html.get(..i).unwrap_or(html),
        None => html,
    };
    let lower = html.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut at = 0;
    while out.len() < MAX_PAIRS {
        let Some(rel) = lower.get(at..).and_then(|r| r.find("<a")) else { break };
        let start = at + rel;
        at = start + 2;
        // `<a` then a space, not `<abbr` or `<article`.
        if !lower.get(at..).is_some_and(|r| r.starts_with(|c: char| c.is_ascii_whitespace())) {
            continue;
        }
        let Some(tag_end) = html.get(at..).and_then(|r| r.find('>')).map(|i| at + i) else { break };
        let tag = html.get(at..tag_end).unwrap_or("");
        let Some(target) = attribute(tag, "href") else { continue };
        let inner_start = tag_end + 1;
        let inner_end = lower.get(inner_start..).and_then(|r| r.find("</a")).map_or(html.len(), |i| inner_start + i);
        at = inner_end.max(inner_start);
        let target = target.trim();
        if target.is_empty() || target.starts_with('#') || target.len() > 8192 || target.to_ascii_lowercase().starts_with("mailto:") {
            continue;
        }
        let shown = shown_text(html.get(inner_start..inner_end).unwrap_or(""));
        out.push((shown, decode(target)));
    }
    out
}

/// The value of attribute `name` in a tag's text (quoted either way, or not).
fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(rel) = lower.get(from..).and_then(|r| r.find(name)) {
        let i = from + rel;
        from = i + name.len();
        let before_ok = i == 0 || lower.get(..i).is_some_and(|b| b.ends_with(|c: char| c.is_ascii_whitespace() || c == '"' || c == '\''));
        let rest = tag.get(from..).unwrap_or("").trim_start();
        let Some(value) = rest.strip_prefix('=').filter(|_| before_ok) else { continue };
        let value = value.trim_start();
        return Some(match value.chars().next() {
            Some(q @ ('"' | '\'')) => {
                let body = value.get(1..).unwrap_or("");
                body.get(..body.find(q).unwrap_or(body.len())).unwrap_or("").to_string()
            }
            _ => value.get(..value.find(char::is_whitespace).unwrap_or(value.len())).unwrap_or("").to_string(),
        });
    }
    None
}

/// An anchor's inner HTML as the text a reader sees: tags out, entities read, spaces collapsed,
/// cut at 200 characters.
fn shown_text(inner: &str) -> String {
    let mut text = String::new();
    let mut in_tag = false;
    for c in inner.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                text.push(' ');
            }
            _ if !in_tag => text.push(c),
            _ => {}
        }
        if text.len() > MAX_SHOWN * 8 {
            break;
        }
    }
    let words = decode(&text).split_whitespace().collect::<Vec<_>>().join(" ");
    words.chars().take(MAX_SHOWN).collect()
}

/// The few entities that matter in a link or its text.
fn decode(s: &str) -> String {
    s.replace("&nbsp;", " ").replace("&quot;", "\"").replace("&#39;", "'").replace("&#38;", "&").replace("&amp;", "&")
}
