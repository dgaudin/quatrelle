//! HTML a person wrote (a signature) made safe to send: what it keeps, and what goes.

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
        clean_into(k, &mut out, 0);
    }
    out
}

fn clean_esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn clean_into(k: &Kid, out: &mut String, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    match k {
        Kid::Text(t) => out.push_str(&clean_esc(t)),
        Kid::El(n) if DROPPED.contains(&n.tag.as_str()) => {}
        Kid::El(n) if !CLEAN_TAGS.contains(&n.tag.as_str()) => {
            for c in &n.kids {
                clean_into(c, out, depth + 1);
            }
        }
        Kid::El(n) => {
            out.push('<');
            out.push_str(&n.tag);
            for (name, value) in &n.attrs {
                if !CLEAN_ATTRS.contains(&name.as_str()) {
                    continue;
                }
                let v = value.trim();
                let lower = v.to_ascii_lowercase();
                let ok = match name.as_str() {
                    "href" => lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:"),
                    "src" => lower.starts_with("https://") || lower.starts_with("cid:") || lower.starts_with("data:image/"),
                    // No scripts or remote loads through a style.
                    "style" => {
                        !lower.contains("expression(") && !lower.contains("url(") && !lower.contains("javascript:") && !lower.contains("@import")
                    }
                    _ => true,
                };
                if ok {
                    out.push_str(&format!(" {name}=\"{}\"", clean_esc(v)));
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
                clean_into(c, out, depth + 1);
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
}
