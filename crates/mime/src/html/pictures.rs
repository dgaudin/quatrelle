// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The pictures of an HTML body by their tags: their addresses as a browser reads them, and the
//! tracking pixels and spacers by the size they set.

use super::*;

/// Most bytes of a picture's address decoded by the tokenizer; a longer one only has its `&amp;`
/// read (no picture is asked for past 4096).
const ADDRESS_MAX: usize = 8192;

/// Addresses read by the tokenizer for one message, at most: past that many, an address has its
/// `&amp;` alone read.
pub(crate) const MAX_DECODED: usize = 512;

/// Pictures' addresses as a browser reads them (their character references decoded: `&amp;`,
/// `&#38;`, `&#x26;`, `&quest;`…), for one message: the same in its pictures and in its trackers,
/// so one address written two ways is one (a pixel listed as written and as decoded said two
/// pixels for one, used up the 50 kept, and was asked for). Each distinct one is read once by the
/// tokenizer, in all the readings of the message (its pictures, its trackers as it came and once
/// its style sheet is on its elements), and [`MAX_DECODED`] at most: each `&` cost a tokenizer of
/// its own, three times over, and thousands of pictures took two and a half times as long to read.
#[derive(Default)]
pub(crate) struct Addresses {
    read: HashMap<String, String>,
}

impl Addresses {
    /// The address written `raw` in its tag (out of its quotes), as a browser reads it.
    pub(crate) fn read(&mut self, raw: &str) -> String {
        self.read_with(raw, layout::first_attr_value)
    }

    /// [`Addresses::read`], `decode` reading the attribute it's given (`None`: it couldn't, and
    /// the `&amp;` alone is read).
    fn read_with(&mut self, raw: &str, decode: impl FnOnce(&str) -> Option<String>) -> String {
        let raw = raw.trim();
        let folded = || raw.replace("&amp;", "&");
        // No reference but `&amp;` (as a style sheet's writing leaves them): what the tokenizer reads.
        if raw.matches('&').count() == raw.matches("&amp;").count() {
            return folded();
        }
        if let Some(v) = self.read.get(raw) {
            return v.clone();
        }
        let q = if raw.contains('"') { '\'' } else { '"' };
        if raw.len() > ADDRESS_MAX || raw.contains(q) || self.read.len() >= MAX_DECODED {
            return folded();
        }
        let v = decode(&format!("<i a={q}{raw}{q}>")).map_or_else(folded, |v| v.trim().to_string());
        self.read.insert(raw.to_string(), v.clone());
        v
    }
}

/// Every `<img src>` of an HTML body, in order, without repeats (at most 200). Pictures without
/// a description (tracking pixels, mostly) leave no trace in the formatted text, so they are
/// found here, to be blocked or shown like the rest.
pub fn html_images(html: &str) -> Vec<String> {
    html_images_with(html, &mut Addresses::default())
}

/// [`html_images`], its addresses read with the message's others (`addresses`).
pub(crate) fn html_images_with(html: &str, addresses: &mut Addresses) -> Vec<String> {
    let lower = html.to_ascii_lowercase();
    let mut out: Vec<String> = Vec::new();
    let mut from = 0;
    while let Some(i) = lower.get(from..).and_then(|r| r.find("<img")) {
        let start = from + i;
        let end = lower.get(start..).and_then(|r| r.find('>')).map_or(lower.len(), |e| start + e);
        let tag_lower = lower.get(start..end).unwrap_or("");
        let tag = html.get(start..end).unwrap_or("");
        if let Some(p) = tag_lower.find(" src=").or_else(|| tag_lower.find("\nsrc=")) {
            let v = tag.get(p + 5..).unwrap_or("");
            let src = match v.chars().next() {
                Some(q @ ('"' | '\'')) => v.get(1..).and_then(|r| r.split(q).next()).unwrap_or(""),
                _ => v.split([' ', '\t', '\n', '/']).next().unwrap_or(""),
            };
            let src = addresses.read(src);
            if !src.is_empty() && src.len() < 4096 && !out.contains(&src) {
                out.push(src);
            }
        }
        from = end.max(start + 4);
        if out.len() >= 200 {
            break;
        }
    }
    out
}

/// Whether an `<img …>` tag (`tag`) is a tracking pixel or a spacer by the size it sets: 2 pixels
/// or less across or down, by its `width` or `height`, or by its style's (`width: 1px
/// !important`, `max-height: 0`, `width: 0%`), the property itself (a `border-width: 0` isn't
/// one). When in doubt, a pixel: any of its style's sizes counts, not only the one a browser
/// applies (`width:1px !important; width:300px` is 1 pixel there, `width:1px; width:auto`
/// isn't, and both are taken for one); the tag's size counts even when its style sets another
/// (`width="0"` with `style="width:28px"`, which a browser shows 28 pixels wide, unlike
/// [`picture_sizes`]); and one side is enough (a line 600 by 1 is taken for one too).
pub(crate) fn tiny_picture(tag: &str) -> bool {
    let attr = |a: &str| tag_attr(tag, a).is_some_and(|v| v.trim().trim_end_matches("px").trim().parse::<f32>().is_ok_and(|n| n <= 2.0));
    let style = tag_attr(tag, "style").map(css::declarations).unwrap_or_default();
    let css = style.iter().any(|d| matches!(d.prop.as_str(), "width" | "height" | "max-width" | "max-height") && tiny_length(&d.value));
    attr("width") || attr("height") || css
}

/// A CSS length of 2 pixels or less, or of nothing in any unit (`0%`, `0vw`, which aren't read as
/// pixels).
fn tiny_length(v: &str) -> bool {
    if let Some(n) = layout::length_px(v) {
        return n <= 2.0;
    }
    let number: String = v.trim().chars().take_while(|c| c.is_ascii_digit() || matches!(c, '.' | '+' | '-')).take(32).collect();
    number.parse::<f32>().is_ok_and(|n| n == 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An address the tokenizer couldn't read (a panic in it, which [`layout::first_attr_value`]
    /// catches) has its `&amp;` alone read, as before the tokenizer read them: a sane address, the
    /// same in every reading of the message.
    #[test]
    fn an_address_the_tokenizer_cant_read_has_its_amp_read() {
        let raw = " https://t.example/o.gif?a=1&#38;b=2&amp;c=3 ";
        let mut a = Addresses::default();
        assert_eq!(a.read_with(raw, |_| None), "https://t.example/o.gif?a=1&#38;b=2&c=3");
        assert_eq!(a.read(raw), "https://t.example/o.gif?a=1&#38;b=2&c=3", "kept so");
        assert_eq!(Addresses::default().read(raw), "https://t.example/o.gif?a=1&b=2&c=3", "read by the tokenizer");
        assert_eq!(layout::first_attr_value("<i a=\"x&quest;y\">").as_deref(), Some("x?y"));
    }
}
