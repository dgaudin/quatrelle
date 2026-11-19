// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The pictures of an HTML body by their tags, read as a browser reads them: their addresses, and
//! the tracking pixels and spacers among them.

use super::*;

/// Most bytes of a picture's address decoded by the tokenizer; a longer one only has its `&amp;`
/// read (no picture is asked for past 4096).
const ADDRESS_MAX: usize = 8192;

/// Addresses read by the tokenizer for one message, at most: past that many, an address has its
/// `&amp;` alone read.
pub(crate) const MAX_DECODED: usize = 512;

/// Most pictures listed of one HTML body, by its tags and behind its boxes.
pub(crate) const MAX_PICTURES: usize = 200;

/// Pictures with a longer address aren't listed.
const SRC_MAX: usize = 4096;

/// Pictures' addresses as a browser reads them (their character references decoded: `&amp;`,
/// `&#38;`, `&#x26;`, `&quest;`…), for one message: the same in its pictures and in its trackers,
/// so one address written two ways is one (a pixel listed as written and as decoded said two
/// pixels for one, used up the 50 kept, and was asked for). Each distinct one is read once by the
/// tokenizer, in all the readings of the message (as it came and once its style sheet is on its
/// elements), and [`MAX_DECODED`] at most: each `&` cost a tokenizer of its own, three times
/// over, and thousands of pictures took two and a half times as long to read. The sizes and
/// styles a tracking picture is told by are read the same way. One that can't be read any more
/// (past [`MAX_DECODED`]) is unread: its picture is taken for a tracking picture.
#[derive(Default)]
pub(crate) struct Addresses {
    read: HashMap<String, Option<String>>,
}

impl Addresses {
    /// The value written `raw` in a tag (out of its quotes: an address, a size, a style), as a
    /// browser reads it; `None` when it holds a character reference that wasn't read (past
    /// [`MAX_DECODED`], too long, or one the tokenizer couldn't read).
    pub(crate) fn value(&mut self, raw: &str) -> Option<String> {
        self.value_with(raw, layout::first_attr_value)
    }

    /// [`Addresses::value`], `decode` reading the attribute it's given.
    fn value_with(&mut self, raw: &str, decode: impl FnOnce(&str) -> Option<String>) -> Option<String> {
        let raw = raw.trim();
        // No reference but `&amp;` (as a style sheet's writing leaves them): what the tokenizer reads.
        if raw.matches('&').count() == raw.matches("&amp;").count() {
            return Some(raw.replace("&amp;", "&"));
        }
        if let Some(v) = self.read.get(raw) {
            return v.clone();
        }
        let q = if raw.contains('"') { '\'' } else { '"' };
        if raw.len() > ADDRESS_MAX || raw.contains(q) || self.read.len() >= MAX_DECODED {
            return None;
        }
        let v = decode(&format!("<i a={q}{raw}{q}>")).map(|v| v.trim().to_string());
        self.read.insert(raw.to_string(), v.clone());
        v
    }
}

/// Each `<img>` of `html` as a browser finds it ([`deep::tags`]: not in a comment, a script, a
/// style sheet, a `<textarea>`, and past a `>` in a quoted value), an `<image>` too (which it
/// reads as one), outside the `<template>`s, whose content is never shown: its text and where it
/// is, until `each` says no more.
pub(super) fn img_tags<'a>(html: &'a str, mut each: impl FnMut(&'a str, std::ops::Range<usize>) -> bool) {
    let mut templates = 0usize;
    deep::tags(html, |name, end, range| {
        match name {
            "template" if end => templates = templates.saturating_sub(1),
            "template" => templates = templates.saturating_add(1),
            "img" | "image" if !end && templates == 0 => return each(html.get(range.clone()).unwrap_or_default(), range),
            _ => {}
        }
        true
    });
}

/// Whether a picture's description (`alt`) only names it: it starts with "spacer" (a spacer's,
/// any case), or it is its file's name, the last part of its address (`src`) with or without its
/// extension, any case (`spacer-FFFFFF` for `…/spacer-FFFFFF.jpg`). Such a picture lays the message
/// out, or says nothing a reader didn't know. One without a description isn't one, nor one
/// carried in its address (`data:`), which has no file name.
pub fn names_itself(src: &str, alt: &str) -> bool {
    let alt = alt.trim();
    if alt.is_empty() {
        return false;
    }
    if spacer_described(alt) {
        return true;
    }
    if src.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("data:")) {
        return false;
    }
    let path = src.split(['?', '#']).next().unwrap_or(src);
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    name.eq_ignore_ascii_case(alt) || stem.eq_ignore_ascii_case(alt)
}

/// Whether a picture's description says it's a spacer: "spacer" as a whole word at its start
/// (any case), followed by nothing, a dash, an underscore, a digit or a space, never a letter
/// (`spacer-FFFFFF`, `spacer_2`; not a product's photo, `Spacers M8 nylon`).
pub fn spacer_described(alt: &str) -> bool {
    let alt = alt.trim();
    alt.get(..6).is_some_and(|p| p.eq_ignore_ascii_case("spacer"))
        && alt.get(6..).and_then(|rest| rest.chars().next()).is_none_or(|c| matches!(c, '-' | '_') || c.is_ascii_digit() || c.is_whitespace())
}

/// `html` without the pictures whose description only names them ([`names_itself`]): for the
/// list's preview, where their words say nothing (at most [`MAX_PICTURES`] taken out).
pub(super) fn without_named_pictures(html: &str) -> std::borrow::Cow<'_, str> {
    // A value as written, out of its quotes.
    let unquoted = |v: &str| v.trim().trim_matches(['"', '\'']).trim().to_string();
    let mut out: Vec<std::ops::Range<usize>> = Vec::new();
    img_tags(html, |tag, range| {
        let src = tag_attr(tag, "src").map(unquoted).unwrap_or_default();
        let alt = tag_attr(tag, "alt").map(unquoted).unwrap_or_default();
        if names_itself(&src, &alt) {
            out.push(range);
        }
        out.len() < MAX_PICTURES
    });
    if out.is_empty() {
        return std::borrow::Cow::Borrowed(html);
    }
    let mut kept = String::with_capacity(html.len());
    let mut last = 0;
    for r in out {
        kept.push_str(html.get(last..r.start).unwrap_or(""));
        last = r.end.max(last);
    }
    kept.push_str(html.get(last..).unwrap_or(""));
    std::borrow::Cow::Owned(kept)
}

/// Every `<img src>` of an HTML body, in order, without repeats (at most [`MAX_PICTURES`]).
/// Pictures without a description (tracking pixels, mostly) leave no trace in the formatted
/// text, so they are found here, to be blocked or shown like the rest.
pub fn html_images(html: &str) -> Vec<String> {
    let mut p = Pictures::default();
    p.read(html, true);
    p.images
}

/// The pictures of an HTML body (by its `<img>`s, and behind its boxes) and which of them are
/// tracking pictures, by every reading of it (the message as it came, and once its style sheet is
/// written on its elements): every tag of a picture listed, in each reading, is asked whether it
/// reports back ([`crate::trackers::tracking_picture`]), and so is every box it's behind. The
/// trackers were listed apart, up to 50, and the pictures up to 200: 50 pixels of any address
/// first, and the next was asked for.
#[derive(Default)]
pub(crate) struct Pictures {
    /// Their addresses as a browser reads them, in order, once each (at most [`MAX_PICTURES`]).
    pub(crate) images: Vec<String>,
    /// Those a tag or a box took for a tracking picture.
    pixels: HashSet<String>,
    addresses: Addresses,
}

impl Pictures {
    /// The `<img>`s of `html`: their addresses listed (when `list`, up to [`MAX_PICTURES`]), and
    /// each tag of an address listed asked whether it's a tracking picture. An address that can't
    /// be read any more (past [`MAX_DECODED`]) is a tracking picture's: listed so; in a reading
    /// that lists none, it's one of those listed, but which isn't known, and all those whose tag
    /// isn't read there are held back (512 values to read before it, and a pixel sized
    /// `width:1px !important; width:300px` under a style sheet was asked for).
    pub(crate) fn read(&mut self, html: &str, list: bool) {
        let mut seen: HashSet<String> = HashSet::new();
        let mut unread = false;
        img_tags(html, |tag, _| {
            let Some(raw) = tag_attr(tag, "src") else { return true };
            let Some(src) = self.addresses.value(raw) else {
                unread = true;
                let folded = raw.trim().replace("&amp;", "&");
                if list && !folded.is_empty() && folded.len() < SRC_MAX && (self.images.contains(&folded) || self.images.len() < MAX_PICTURES) {
                    if !self.images.contains(&folded) {
                        self.images.push(folded.clone());
                    }
                    self.pixels.insert(folded);
                }
                return true;
            };
            if src.is_empty() || src.len() >= SRC_MAX {
                return true;
            }
            if !self.images.contains(&src) {
                if !list || self.images.len() >= MAX_PICTURES {
                    return true;
                }
                self.images.push(src.clone());
            }
            if !list && !seen.contains(&src) {
                seen.insert(src.clone());
            }
            if !self.pixels.contains(&src) && self.tracks(tag, &src) {
                self.pixels.insert(src);
            }
            true
        });
        if unread && !list {
            let unseen: Vec<String> = self.images.iter().filter(|s| !seen.contains(*s)).cloned().collect();
            self.pixels.extend(unseen);
        }
    }

    /// The pictures behind a layout's boxes (`behind`), listed after the others (up to
    /// [`MAX_PICTURES`]), and those of them a box takes for a tracking picture (`pixels`).
    pub(crate) fn behind(&mut self, behind: Vec<String>, pixels: Vec<String>) {
        for src in behind {
            if !self.images.contains(&src) && self.images.len() < MAX_PICTURES {
                self.images.push(src);
            }
        }
        self.pixels.extend(pixels);
    }

    /// Whether the `<img>` `tag` (its address `src`) is a tracking picture, by its size and style
    /// as a browser reads them (`style="width&#58;1px"` is 1 pixel wide); when in doubt (one of
    /// them that couldn't be read), it is.
    fn tracks(&mut self, tag: &str, src: &str) -> bool {
        let mut unread = false;
        let mut value = |name: &str| {
            let v = self.addresses.value(tag_attr(tag, name)?);
            unread |= v.is_none();
            v
        };
        let (width, height, style) = (value("width"), value("height"), value("style"));
        crate::trackers::tracking_picture(src, width.as_deref(), height.as_deref(), style.as_deref()) || (unread && crate::trackers::on_the_web(src))
    }

    /// The tracking pictures, in the order of [`Pictures::images`].
    pub(crate) fn trackers(&self) -> Vec<String> {
        self.images.iter().filter(|s| self.pixels.contains(*s)).cloned().collect()
    }
}

/// Whether an `<img …>` tag (`tag`) is a tracking pixel or a spacer by the size it sets
/// ([`tiny_size`]).
pub(crate) fn tiny_picture(tag: &str) -> bool {
    let style = tag_attr(tag, "style").map(css::declarations).unwrap_or_default();
    tiny_size(tag_attr(tag, "width"), tag_attr(tag, "height"), &style)
}

/// Whether a picture (or a box) is a tracking pixel or a spacer by the size it sets: 2 pixels or
/// less across or down, by its `width` or `height`, or by its style's (`width: 1px !important`,
/// `max-height: 0`, `width: 0%`), the property itself (a `border-width: 0` isn't one). When in
/// doubt, a pixel: any of its style's sizes counts, not only the one a browser applies
/// (`width:1px !important; width:300px` is 1 pixel there, `width:1px; width:auto` isn't, and
/// both are taken for one); the tag's size counts even when its style sets another (`width="0"`
/// with `style="width:28px"`, which a browser shows 28 pixels wide, unlike [`picture_sizes`]);
/// and one side is enough (a line 600 by 1 is taken for one too).
pub(crate) fn tiny_size(width: Option<&str>, height: Option<&str>, style: &[css::Decl]) -> bool {
    let attr = |v: Option<&str>| v.and_then(tag_px).is_some_and(|n| n <= 2.0);
    let css = style.iter().any(|d| matches!(d.prop.as_str(), "width" | "height" | "max-width" | "max-height") && tiny_length(&d.value));
    attr(width) || attr(height) || css
}

/// A size written in a tag (`width="1"`) as a browser reads it, by its leading digits (`1x`,
/// `1em` and `1.5` are 1 and 1.5 pixels; `1x` was read as nothing, and the pixel was asked for);
/// `None` without one, or for a share of the space (`1%`).
fn tag_px(v: &str) -> Option<f32> {
    let v = v.trim_start();
    let sign = usize::from(v.starts_with(['+', '-']));
    let digits = |s: &str| s.bytes().take_while(u8::is_ascii_digit).count();
    let whole = digits(v.get(sign..)?);
    let fraction = v.get(sign + whole..).filter(|r| r.starts_with('.')).map_or(0, |r| 1 + digits(r.get(1..).unwrap_or_default()));
    let number = v.get(..sign + whole + fraction)?;
    // No space before it (`1 %` is 1 pixel, as a browser reads it).
    let share = v.get(number.len()..).is_some_and(|r| r.starts_with('%'));
    if whole + fraction.saturating_sub(1) == 0 || share {
        return None;
    }
    number.parse::<f32>().ok()
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

    /// "spacer" says a spacer as a whole word only: followed by nothing, a dash, an underscore, a
    /// digit or a space, never a letter (a product's photo, "Spacers M8 nylon").
    #[test]
    fn spacer_is_said_as_a_whole_word() {
        for alt in ["spacer", "Spacer", " SPACER ", "spacer-FFFFFF", "spacer_line", "spacer2", "spacer 3"] {
            assert!(spacer_described(alt), "{alt}");
        }
        for alt in ["Spacers M8 nylon", "Spacerail", "spacerfill", "spaceé", "a spacer"] {
            assert!(!spacer_described(alt), "{alt}");
        }
        assert!(!names_itself("https://cdn.example/img/m8.jpg", "Spacers M8 nylon"));
    }

    /// A description that only repeats its picture's file name, or a spacer's, names it; words
    /// don't, and nor does nothing at all.
    #[test]
    fn a_description_that_only_repeats_its_file_name_names_it() {
        for (src, alt) in [
            ("https://cdn.example/img/spacer-FFFFFF.jpg", "spacer-FFFFFF"),
            ("https://cdn.example/img/spacer-FFFFFF.jpg", " SPACER-ffffff "),
            ("https://cdn.example/img/line-grey.gif?v=3#top", "line-grey"),
            ("https://cdn.example/img/line-grey.gif", "Line-Grey.GIF"),
            ("https://cdn.example/img/gap.png", "spacer"),
            ("https://cdn.example/img/gap.png", "Spacer line"),
            ("https://cdn.example/4321.jpg", "4321"),
        ] {
            assert!(names_itself(src, alt), "{src} / {alt}");
        }
        for (src, alt) in [
            ("https://cdn.example/img/hall.jpg", "Grande salle"),
            ("https://cdn.example/img/hall.jpg", "hall photo"),
            ("https://cdn.example/img/hall.jpg", ""),
            ("https://cdn.example/img/hall.jpg", "   "),
            ("https://cdn.example/", "cdn.example"),
            // Carried in its address: no file name, whatever follows its last slash.
            ("data:image/gif;base64,R0lG/AA", "AA"),
            // A letter across the sixth byte: no "spacer" there, and no slicing through it.
            ("https://cdn.example/img/a.png", "spaceé"),
        ] {
            assert!(!names_itself(src, alt), "{src} / {alt}");
        }
    }

    /// A value the tokenizer couldn't read (a panic in it, which [`layout::first_attr_value`]
    /// catches) is unread, as one past [`MAX_DECODED`] (its picture is taken for a tracking
    /// picture), in every reading of the message; `&amp;` alone is read without it.
    #[test]
    fn a_value_the_tokenizer_cant_read_is_unread() {
        let raw = " https://t.example/o.gif?a=1&#38;b=2&amp;c=3 ";
        let mut a = Addresses::default();
        assert_eq!(a.value_with(raw, |_| None), None);
        assert_eq!(a.value(raw), None, "kept so");
        assert_eq!(Addresses::default().value(raw).as_deref(), Some("https://t.example/o.gif?a=1&b=2&c=3"), "read by the tokenizer");
        assert_eq!(Addresses::default().value(" https://t.example/o.gif?a=1&amp;c=3").as_deref(), Some("https://t.example/o.gif?a=1&c=3"));
        assert_eq!(layout::first_attr_value("<i a=\"x&quest;y\">").as_deref(), Some("x?y"));
    }
}
