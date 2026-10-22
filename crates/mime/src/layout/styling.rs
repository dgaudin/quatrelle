//! Style sheets written into each element's `style`, and text sized as a title made a heading,
//! before the layout and the text converter read the HTML.

use super::*;

/// The message with its style sheets' rules written into each element's `style` (inline
/// declarations still win, `!important` ones over both), and text sized as a title made a
/// heading, so the layout and the text converter both see them. Unchanged when it has no style
/// sheet and no sizes.
pub fn inline_css(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    if !(lower.contains("<style") || lower.contains("font-size") || lower.contains("<font")) {
        return html.to_string();
    }
    let capped = match html.char_indices().nth(crate::HTML_MAX) {
        Some((i, _)) => html.get(..i).unwrap_or(html),
        None => html,
    };
    let done = std::panic::catch_unwind(|| {
        let mut root = parse(capped);
        let mut text = String::new();
        style_text(&root, &mut text, 0);
        let sheet = crate::css::Sheet::parse(&text);
        let mut chain = Vec::new();
        apply_css(&mut root, &sheet, &mut chain, 0);
        let mut out = String::with_capacity(capped.len() + capped.len() / 4);
        write(&Kid::El(root), &mut out);
        out
    });
    done.unwrap_or_else(|_| html.to_string())
}

/// The text of every `<style>` element.
fn style_text(n: &Node, out: &mut String, depth: usize) {
    if depth > MAX_DEPTH * 4 || out.len() > 256 * 1024 {
        return;
    }
    for k in &n.kids {
        match k {
            Kid::Text(t) if n.tag == "style" => {
                out.push_str(t);
                out.push('\n');
            }
            Kid::El(e) => style_text(e, out, depth + 1),
            Kid::Text(_) => {}
        }
    }
}

/// An element as selectors see it, kept while its children are styled.
struct Ancestor {
    tag: String,
    id: Option<String>,
    classes: Vec<String>,
}

fn apply_css(n: &mut Node, sheet: &crate::css::Sheet, chain: &mut Vec<Ancestor>, depth: usize) {
    if depth > MAX_DEPTH * 4 || matches!(n.tag.as_str(), "head" | "style" | "script" | "title" | "template") {
        return;
    }
    for k in &mut n.kids {
        let Kid::El(kid) = k else { continue };
        chain.push(Ancestor {
            tag: kid.tag.clone(),
            id: kid.attr("id").map(str::to_string),
            classes: kid.attr("class").map(|c| c.split_whitespace().take(16).map(str::to_string).collect()).unwrap_or_default(),
        });
        if !sheet.is_empty() {
            let els: Vec<crate::css::El<'_>> = chain
                .iter()
                .map(|a| crate::css::El { tag: &a.tag, id: a.id.as_deref(), classes: a.classes.iter().map(String::as_str).collect() })
                .collect();
            let matched = sheet.matching(&els);
            if !matched.is_empty() {
                let mut style: Vec<(String, String)> = Vec::new();
                let set = |p: &str, v: &str, style: &mut Vec<(String, String)>| match style.iter_mut().find(|(k, _)| k == p) {
                    Some(e) => e.1 = v.to_string(),
                    None => style.push((p.to_string(), v.to_string())),
                };
                for d in matched.iter().filter(|d| !d.important) {
                    set(&d.prop, &d.value, &mut style);
                }
                for d in crate::css::declarations(kid.attr("style").unwrap_or("")) {
                    set(&d.prop, &d.value, &mut style);
                }
                for d in matched.iter().filter(|d| d.important) {
                    set(&d.prop, &d.value, &mut style);
                }
                let written = style.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join("; ");
                match kid.attrs.iter_mut().find(|(k, _)| k == "style") {
                    Some(a) => a.1 = written,
                    None => kid.attrs.push(("style".into(), written)),
                }
            }
        }
        apply_css(kid, sheet, chain, depth + 1);
        chain.pop();
    }
    // Text sized as a title becomes a heading of that level: a block's content goes inside one;
    // a lone inline element (`<td><font size="5">Title</font></td>`) goes inside one itself.
    let only_child = n
        .kids
        .iter()
        .filter(|k| match k {
            Kid::Text(t) => !t.trim().is_empty(),
            Kid::El(_) => true,
        })
        .count()
        == 1;
    let parent_is_block = block_tag(&n.tag);
    for k in &mut n.kids {
        let Kid::El(kid) = k else { continue };
        let level = kid.font_size().map_or(0, heading_for);
        let is_heading = kid.tag.len() == 2 && kid.tag.starts_with('h') && kid.tag != "hr";
        if level == 0 || is_heading || !kid.has_text(0) || kid.has_block_inside(0) {
            continue;
        }
        let h = format!("h{level}");
        if block_tag(&kid.tag) {
            let inner = std::mem::take(&mut kid.kids);
            kid.kids = vec![Kid::El(Node { tag: h, attrs: Vec::new(), kids: inner })];
        } else if only_child && parent_is_block {
            let inner = std::mem::take(kid);
            *kid = Node { tag: h, attrs: Vec::new(), kids: vec![Kid::El(inner)] };
        }
    }
}
