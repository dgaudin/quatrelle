// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! HTML made safe to send: a signature a person wrote, and the original under a reply or forward
//! as it came. What it keeps, and what goes.

use super::*;

/// Elements a signature written in HTML keeps (layout, text and links); everything else goes,
/// its text kept unless it's a script's, a style sheet's or a form's.
const CLEAN_TAGS: &[&str] = &[
    "a",
    "b",
    "big",
    "blockquote",
    "br",
    "center",
    "code",
    "col",
    "colgroup",
    "div",
    "em",
    "font",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "i",
    "img",
    "li",
    "ol",
    "p",
    "pre",
    "s",
    "small",
    "span",
    "strike",
    "strong",
    "sub",
    "sup",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "tr",
    "u",
    "ul",
];

/// Elements whose content goes with them.
const DROPPED: &[&str] =
    &["script", "style", "iframe", "object", "embed", "form", "input", "button", "select", "textarea", "head", "title", "meta", "link"];

/// Attributes kept on them (all else, `on…` handlers included, goes).
const CLEAN_ATTRS: &[&str] = &[
    "align",
    "alt",
    "bgcolor",
    "border",
    "cellpadding",
    "cellspacing",
    "color",
    "colspan",
    "face",
    "height",
    "hspace",
    "nowrap",
    "rowspan",
    "size",
    "style",
    "title",
    "valign",
    "vspace",
    "width",
    "href",
    "src",
];

/// HTML a person wrote (a signature) made safe to send as it is: its layout, text, styles,
/// links and pictures, without scripts, handlers, forms or frames. Links go to the web or to
/// an address; pictures are the message's own (`cid:`), embedded (`data:image/`), or on the web
/// (`https:`, which the recipients' mail apps load as they choose; Quatrelle doesn't). Hostile
/// input is capped as the rest of this module's is; never panics.
pub fn clean_html(html: &str) -> String {
    clean_with(html, None)
}

/// The original under a reply or forward (a message's HTML, as it came) as it goes out after
/// the reply's text, as Outlook sends it, so its columns, colours and pictures come through: its
/// style sheet's rules written into its tags first ([`inline_css`]), then cleaned as a signature
/// is (no scripts, handlers, forms, frames or head), keeping besides the pictures behind its
/// boxes. Each picture it shows from the message goes by the reply's own `cid:` (`given`: the
/// picture's address as written, and its `cid:`); a tracking picture (`trackers`) goes; one on
/// the web stays at its address, as Outlook keeps it (Quatrelle never loads it).
pub fn quote_html(html: &str, given: &[(String, String)], trackers: &[String]) -> String {
    clean_with(&inline_css(html), Some(&Quoting { given, trackers }))
}

/// What a quoted message keeps beyond what a signature does, and how its pictures are named.
struct Quoting<'a> {
    given: &'a [(String, String)],
    trackers: &'a [String],
}

impl Quoting<'_> {
    /// A picture's address as it goes: the reply's `cid:` for one it carries, the address for one
    /// on the web or in it; `None` for a tracking picture or anything else.
    fn picture(&self, src: &str) -> Option<String> {
        let plain = |s: &str| s.trim().replace("&amp;", "&");
        let src = plain(src);
        if self.trackers.iter().any(|t| plain(t) == src) {
            return None;
        }
        if let Some((_, cid)) = self.given.iter().find(|(s, _)| plain(s) == src) {
            return Some(format!("cid:{}", cid.replace(['"', '\'', '<', '>', '(', ')', ' '], "")));
        }
        let lower = src.to_ascii_lowercase();
        (lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("cid:") || lower.starts_with("data:image/"))
            .then_some(src)
    }

    /// A style with its pictures (`url(…)`) named as [`Quoting::picture`] says, the others gone;
    /// `None` for one that could run or load anything else.
    fn style(&self, v: &str) -> Option<String> {
        let lower = v.to_ascii_lowercase();
        if lower.contains("expression(") || lower.contains("javascript:") || lower.contains("@import") || lower.contains("behavior") {
            return None;
        }
        let mut out = String::with_capacity(v.len());
        let mut rest = v;
        while let Some(i) = rest.to_ascii_lowercase().find("url(") {
            out.push_str(rest.get(..i).unwrap_or(""));
            let after = rest.get(i + 4..).unwrap_or("");
            let Some(end) = after.find(')') else { return None };
            let target = after.get(..end).unwrap_or("").trim().trim_matches(['"', '\'']);
            match self.picture(target).filter(|p| !p.contains(['"', '\'', '(', ')'])) {
                Some(p) => out.push_str(&format!("url('{p}')")),
                None => out.push_str("none"),
            }
            rest = after.get(end + 1..).unwrap_or("");
        }
        out.push_str(rest);
        Some(out)
    }
}

fn clean_with(html: &str, q: Option<&Quoting<'_>>) -> String {
    let html = match html.char_indices().nth(crate::HTML_MAX) {
        Some((i, _)) => html.get(..i).unwrap_or(html),
        None => html,
    };
    let root = match std::panic::catch_unwind(|| parse(html)) {
        Ok(r) => r,
        Err(_) => return String::new(),
    };
    let mut out = String::new();
    for k in &root.kids {
        clean_into(k, &mut out, 0, q);
    }
    out
}

fn clean_esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn clean_into(k: &Kid, out: &mut String, depth: usize, q: Option<&Quoting<'_>>) {
    // A quoted newsletter nests its tables deeper than a signature does: as deep as it's read.
    if depth > if q.is_some() { MAX_DEPTH * 4 } else { MAX_DEPTH } {
        return;
    }
    match k {
        Kid::Text(t) => out.push_str(&clean_esc(t)),
        Kid::El(n) if DROPPED.contains(&n.tag.as_str()) => {}
        Kid::El(n) if !CLEAN_TAGS.contains(&n.tag.as_str()) => {
            for c in &n.kids {
                clean_into(c, out, depth + 1, q);
            }
        }
        // A tracking picture in a quoted message goes.
        Kid::El(n) if n.tag == "img" && q.is_some_and(|q| n.attrs.iter().any(|(a, v)| a == "src" && q.picture(v).is_none())) => {}
        Kid::El(n) => {
            out.push('<');
            out.push_str(&n.tag);
            for (name, value) in &n.attrs {
                let quoting = q.filter(|_| matches!(name.as_str(), "src" | "background" | "style"));
                if !CLEAN_ATTRS.contains(&name.as_str()) && quoting.is_none() {
                    continue;
                }
                let v = value.trim();
                let lower = v.to_ascii_lowercase();
                let kept = match (name.as_str(), quoting) {
                    // A quoted message's pictures by the reply's `cid:`s, its boxes' too.
                    ("src" | "background", Some(q)) => q.picture(v),
                    ("style", Some(q)) => q.style(v),
                    ("href", _) => {
                        (lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:")).then(|| v.to_string())
                    }
                    ("src", _) => {
                        (lower.starts_with("https://") || lower.starts_with("cid:") || lower.starts_with("data:image/")).then(|| v.to_string())
                    }
                    // No scripts or remote loads through a style.
                    ("style", _) => {
                        (!lower.contains("expression(") && !lower.contains("url(") && !lower.contains("javascript:") && !lower.contains("@import"))
                            .then(|| v.to_string())
                    }
                    _ => Some(v.to_string()),
                };
                if let Some(v) = kept {
                    out.push_str(&format!(" {name}=\"{}\"", clean_esc(&v)));
                }
            }
            if n.tag == "a" {
                out.push_str(" target=\"_blank\"");
            }
            if void(&n.tag) {
                out.push('>');
                return;
            }
            out.push('>');
            for c in &n.kids {
                clean_into(c, out, depth + 1, q);
            }
            out.push_str(&format!("</{}>", n.tag));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A signature keeps its tables, styles, links and pictures; scripts, handlers, forms and
    /// remote loads through styles go; junk never breaks it.
    #[test]
    fn signatures_are_cleaned_but_keep_their_look() {
        let sig = r#"<table align="left" style="float:left;"><tr><td valign="middle"><a href="https://www.contoso.example?sign" target="_top"><img src="https://www.contoso.example/images/logo.gif" width="194px" style="width:194px;height:75px" onerror="alert(1)"></a></td></tr></table>
<p><span style="font-weight:bold;color:#4B5763">Riley MORENO</span><br>T&eacute;l.&nbsp;+33<br><a href="mailto:riley@contoso.example" onmouseover="this.style.color='#0C4790'">riley@contoso.example</a></p>
<script>alert(2)</script><form><input value="x"></form><a href="javascript:alert(3)">bad</a><div style="background:url(https://track.example/p.gif)">t</div>"#;
        let c = clean_html(sig);
        assert!(c.contains(r#"<table align="left" style="float:left;">"#), "{c}");
        assert!(c.contains(r#"src="https://www.contoso.example/images/logo.gif""#));
        assert!(c.contains("Riley MORENO") && c.contains("Tél.\u{a0}+33"), "{c}");
        assert!(c.contains(r#"href="mailto:riley@contoso.example""#));
        for gone in ["onerror", "onmouseover", "<script", "alert(2)", "<form", "<input", "javascript:", "url(", "target=\"_top\""] {
            assert!(!c.contains(gone), "{gone} in {c}");
        }
        for junk in ["", "<", "<<table", "<td><tr></table></table>", &"<div>".repeat(5000)] {
            let _ = clean_html(junk);
        }
    }

    /// The original under a reply goes out as it came: its style sheet written into its tags,
    /// its pictures under the reply's `cid:`s (those behind its boxes too), one on the web at its
    /// address; without its head, scripts, handlers or tracking picture.
    #[test]
    fn a_quoted_message_keeps_its_look_and_pictures() {
        let html = r#"<html><head><title>T</title><style>.warn{color:#ff0000}</style><script>alert(1)</script></head><body>
<table width="640" background="banner.png"><tr><td style="background-image:url('banner.png');padding:8px"><b class="warn" onclick="steal()">A warning</b>
<img src="logo.png" width="120"><img src="https://cdn.example/web.png" width="20"><img src="https://tracker.example/o.gif?a=1&amp;b=2" width="1">
<div style="background:url(javascript:alert(1))">x</div></td></tr></table></body></html>"#;
        let given = [("logo.png".to_string(), "logo@q".to_string()), ("banner.png".to_string(), "banner@q".to_string())];
        let out = quote_html(html, &given, &["https://tracker.example/o.gif?a=1&b=2".to_string()]);
        assert!(out.contains("<table width=\"640\" background=\"cid:banner@q\""), "{out}");
        assert!(out.contains("url(&#x27;cid:banner@q&#x27;)") || out.contains("url('cid:banner@q')"), "{out}");
        assert!(out.contains("color:#ff0000") || out.contains("color: #ff0000"), "{out}");
        assert!(out.contains("src=\"cid:logo@q\"") && out.contains("src=\"https://cdn.example/web.png\""), "{out}");
        for gone in ["<script", "alert(", "onclick", "steal()", "<style", "<title", "tracker.example", "javascript:"] {
            assert!(!out.contains(gone), "{gone} in {out}");
        }
        let deep = format!("{}deep{}", "<table><tr><td>".repeat(10), "</td></tr></table>".repeat(10));
        assert!(quote_html(&deep, &[], &[]).contains("deep"), "a newsletter's nested tables keep their text");
        for junk in ["", "<img src=\"cid:\">", "<td style=\"background:url(\">", &"<table><tr><td>".repeat(3000)] {
            let _ = quote_html(junk, &given, &[]);
        }
    }
}
