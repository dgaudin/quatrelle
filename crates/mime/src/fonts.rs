//! Font sizes and typefaces of HTML (a message read, a signature copied from Outlook or Word):
//! the HTML converter keeps bold, colours and links but not sizes, fonts or underlines, so
//! where each element with a `font-size` or `font-family` (or `<font size face>`, or bold,
//! italic or underline set with CSS, or `<u>`) starts and ends is marked in its text first,
//! and the marks become the spans' sizes, fonts and underlines afterwards ([`runs`]).

use crate::rich::DEFAULT_SIZE;

/// Starts a marked element; the next character (from [`INDEX_BASE`]) says which.
const START: char = '\u{E020}';
/// Ends the innermost marked element.
const END: char = '\u{E021}';
const INDEX_BASE: u32 = 0xE100;
/// Most marked elements (the rest keep the size around them).
const MAX_MARKS: usize = 3000;

/// What a marked element sets: its size in points and its typeface, and bold, italic or
/// underlined when its style says so (`font-weight: bold`, `text-decoration: underline`: the
/// converter knows only `<b>` and `<i>`) or the element does (`<u>`, `<ins>`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Font {
    pub size: Option<u8>,
    pub family: Option<String>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

/// Elements that never hold anything (no end tag).
fn void(tag: &str) -> bool {
    matches!(tag, "br" | "img" | "hr" | "meta" | "link" | "input" | "col" | "area" | "base" | "wbr" | "source" | "track" | "embed" | "param")
}

/// An attribute of a start tag (`name="v"`, `name='v'`, `name=v`), case-insensitive.
fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower.get(from..).and_then(|r| r.find(name)) {
        let at = from + i;
        from = at + name.len();
        let before_ok = lower.get(..at).and_then(|b| b.chars().last()).is_some_and(|c| c.is_whitespace());
        let rest = lower.get(at + name.len()..).unwrap_or("").trim_start();
        if !before_ok || !rest.starts_with('=') {
            continue;
        }
        let off = tag.len() - rest.len() + 1;
        let v = tag.get(off..).unwrap_or("").trim_start();
        return Some(match v.chars().next() {
            Some(q @ ('"' | '\'')) => v.get(1..).and_then(|x| x.split(q).next()).unwrap_or("").to_string(),
            _ => v.split(|c: char| c.is_whitespace() || c == '>').next().unwrap_or("").to_string(),
        });
    }
    None
}

/// A typeface's name from a CSS `font-family` list: the first, unquoted, or `None` for a
/// generic one or Quatrelle's own (Aptos).
fn family(v: &str) -> Option<String> {
    let first = v.split(',').next()?.trim().trim_matches(['"', '\'']).trim();
    let clean: String = first.chars().filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_')).take(64).collect();
    let generic = matches!(
        clean.to_ascii_lowercase().as_str(),
        "" | "serif" | "sans-serif" | "monospace" | "cursive" | "fantasy" | "system-ui" | "inherit" | "aptos"
    );
    (!generic).then_some(clean)
}

/// Points from pixels, as a whole size (7.5 is taken as 8).
fn points(px: f32) -> Option<u8> {
    let pt = (px * 0.75).round();
    (pt.is_finite() && pt >= 1.0).then(|| u8::try_from(pt.min(96.0) as u32).unwrap_or(DEFAULT_SIZE))
}

/// What a start tag sets, if it sets a size or typeface.
fn font_of(tag: &str, name: &str) -> Option<Font> {
    let mut f = Font::default();
    if let Some(style) = attr(tag, "style") {
        // Its entities first: Word writes `font-family:&quot;Arial&quot;`, whose `;` would
        // split the declaration.
        let style = style.replace("&quot;", "\"").replace("&#39;", "'").replace("&apos;", "'").replace("&amp;", "&");
        for decl in style.split(';') {
            let Some((k, v)) = decl.split_once(':') else { continue };
            let v = v.trim().trim_end_matches("!important").trim();
            match k.trim().to_ascii_lowercase().as_str() {
                "font-size" => f.size = crate::layout::font_px(v).and_then(points),
                "font-family" => f.family = family(v),
                "font-weight" => {
                    let w = v.to_ascii_lowercase();
                    f.bold = matches!(w.as_str(), "bold" | "bolder") || w.parse::<u16>().is_ok_and(|n| n >= 600);
                }
                "font-style" => f.italic = matches!(v.to_ascii_lowercase().as_str(), "italic" | "oblique"),
                // (`none` inside an underlined element takes nothing away, as in CSS.)
                "text-decoration" | "text-decoration-line" => f.underline = v.to_ascii_lowercase().split_whitespace().any(|w| w == "underline"),
                "font" => {
                    // The shorthand: `[style] [weight] size[/line] family`.
                    for word in v.split_whitespace() {
                        let w = word.to_ascii_lowercase();
                        f.bold |= matches!(w.as_str(), "bold" | "bolder") || w.parse::<u16>().is_ok_and(|n| (600..=1000).contains(&n));
                        f.italic |= matches!(w.as_str(), "italic" | "oblique");
                        if let Some(px) = crate::layout::font_px(word.split('/').next().unwrap_or("")) {
                            f.size = points(px);
                            let rest = v.split_once(word).map(|(_, r)| r).unwrap_or("");
                            f.family = family(rest.trim_start_matches(|c: char| c != ' ' && c != ',').trim());
                            break;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    if name == "font" {
        if let Some(size) = attr(tag, "size") {
            f.size = crate::layout::font_attr_px(&size).and_then(points).or(f.size);
        }
        if let Some(face) = attr(tag, "face") {
            f.family = family(&face).or(f.family);
        }
    }
    if name == "u" || name == "ins" {
        f.underline = true;
    }
    (f != Font::default()).then_some(f)
}

/// Mark where each element with a size or typeface starts and ends; the fonts, by mark.
pub(crate) fn mark(html: &str) -> (String, Vec<Font>) {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len() + 64);
    let mut fonts: Vec<Font> = Vec::new();
    // Open elements: (name, marked).
    let mut open: Vec<(String, bool)> = Vec::new();
    let mut raw: Option<String> = None;
    let mut i = 0;
    while let Some(off) = lower.get(i..).and_then(|r| r.find('<')) {
        let at = i + off;
        out.push_str(html.get(i..at).unwrap_or(""));
        let Some(close) = lower.get(at..).and_then(|r| r.find('>')).map(|c| at + c) else {
            out.push_str(html.get(at..).unwrap_or(""));
            return (out, fonts);
        };
        let tag = html.get(at..=close).unwrap_or("");
        let inner = lower.get(at + 1..close).unwrap_or("");
        let end_tag = inner.starts_with('/');
        let name: String = inner.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric() || *c == ':').collect();
        if let Some(r) = &raw {
            // Inside <style> or <script>: only its end matters.
            if end_tag && name == *r {
                raw = None;
            }
            out.push_str(tag);
            i = close + 1;
            continue;
        }
        if name.is_empty() || inner.starts_with('!') || inner.starts_with('?') {
            out.push_str(tag);
        } else if end_tag {
            if let Some(pos) = open.iter().rposition(|(n, _)| *n == name) {
                for (_, marked) in open.drain(pos..).rev() {
                    if marked {
                        out.push(END);
                    }
                }
            }
            out.push_str(tag);
        } else {
            out.push_str(tag);
            if matches!(name.as_str(), "style" | "script" | "title" | "head" | "xml") {
                raw = Some(name);
            } else if !void(&name) && !inner.trim_end().ends_with('/') {
                let font = font_of(tag, &name).filter(|_| fonts.len() < MAX_MARKS);
                let marked = font.is_some();
                if let Some(f) = font
                    && let Some(index) = char::from_u32(INDEX_BASE + u32::try_from(fonts.len()).unwrap_or(0))
                {
                    out.push(START);
                    out.push(index);
                    fonts.push(f);
                }
                if open.len() < 500 {
                    open.push((name, marked));
                }
            }
        }
        i = close + 1;
    }
    out.push_str(html.get(i..).unwrap_or(""));
    (out, fonts)
}

/// Whether a piece of converted text holds marks.
pub(crate) fn has_marks(text: &str) -> bool {
    text.contains([START, END])
}

/// Whether a piece of converted text holds nothing but marks: not even a space (an empty
/// paragraph's `&nbsp;` is a line of the message, Outlook's blank line).
pub(crate) fn only_marks(text: &str) -> bool {
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            START => {
                chars.next();
            }
            END => {}
            _ => return false,
        }
    }
    true
}

/// A converted text's marks as runs (the reader's): each piece of text, and what the marked
/// elements around it set. `open`: the elements still open from the text before, carried from
/// piece to piece and line to line. Marks out of order (text laid out in columns) are taken as
/// they come: a piece may get its neighbour's size, nothing worse.
pub(crate) fn runs(text: &str, open: &mut Vec<usize>, fonts: &[Font]) -> Vec<(String, Font)> {
    let around = |open: &[usize]| Font {
        size: open.iter().rev().find_map(|k| fonts.get(*k).and_then(|f| f.size)),
        family: open.iter().rev().find_map(|k| fonts.get(*k).and_then(|f| f.family.clone())),
        bold: open.iter().any(|k| fonts.get(*k).is_some_and(|f| f.bold)),
        italic: open.iter().any(|k| fonts.get(*k).is_some_and(|f| f.italic)),
        underline: open.iter().any(|k| fonts.get(*k).is_some_and(|f| f.underline)),
    };
    let mut out: Vec<(String, Font)> = Vec::new();
    let mut piece = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            START | END => {
                if !piece.is_empty() {
                    out.push((std::mem::take(&mut piece), around(open)));
                }
                if c == END {
                    open.pop();
                } else if let Some(index) = chars.next()
                    && open.len() < 500
                {
                    open.push((index as u32).saturating_sub(INDEX_BASE) as usize);
                }
            }
            c => piece.push(c),
        }
    }
    if !piece.is_empty() {
        out.push((piece, around(open)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_follow_the_elements() {
        let (m, f) = mark(
            "<p style='font-size:7.5pt;font-family:\"Tahoma\",sans-serif'>a<br>b</p><span>c</span><font size=5 face=Arial>d</font><style>p{font-size:9pt}</style>",
        );
        assert_eq!(
            f,
            vec![
                Font { size: Some(8), family: Some("Tahoma".into()), ..Default::default() },
                Font { size: Some(18), family: Some("Arial".into()), ..Default::default() }
            ]
        );
        assert_eq!(m.matches(START).count(), 2);
        assert_eq!(m.matches(END).count(), 2);
        assert!(m.find(END) < m.find("<span"), "closed with its element");
        assert_eq!(family("Aptos, sans-serif"), None);
        assert_eq!(family("'Bahnschrift Light'"), Some("Bahnschrift Light".into()));
        for junk in ["<", "<p style=", "<p style='font-size:", "</p></p><p>", "<font size=x>", "<p style='font:bold 12pt/1.2 Georgia'>x"] {
            let _ = mark(junk);
        }
        assert_eq!(
            mark("<p style='font:bold 12pt/1.2 Georgia, serif'>x").1,
            vec![Font { size: Some(12), family: Some("Georgia".into()), bold: true, ..Default::default() }]
        );
        // Underlined: the element, or its style; `none` isn't a mark.
        let under = Font { underline: true, ..Default::default() };
        assert_eq!(
            mark("<u>a</u><ins>b</ins><span style='text-decoration: underline'>c</span>").1,
            vec![under.clone(), under.clone(), under.clone()]
        );
        assert_eq!(mark("<span style='text-decoration-line: underline dotted'>c</span>").1, vec![under]);
        assert!(mark("<a style='text-decoration:none'>c</a>").1.is_empty());
    }
}
