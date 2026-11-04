// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! How wide a message's boxes, cells and pictures want to be.

use super::*;

impl Node {
    /// How wide it wants to be: `width`, held to `max-width` (a 600-pixel column that narrows on
    /// a phone is `width: 100%; max-width: 600px`).
    pub(super) fn width(&self) -> Option<Width> {
        let own = self.style("width").or_else(|| self.attr("width").map(str::to_string)).and_then(|v| parse_width(&v));
        match (own, self.style("max-width").and_then(|v| parse_width(&v))) {
            (Some(Width::Px(w)), Some(Width::Px(m))) => Some(Width::Px(w.min(m))),
            (None | Some(Width::Percent(100)), Some(Width::Px(m))) => Some(Width::Px(m)),
            (own, _) => own,
        }
    }

    /// A box of a set number of pixels across (a message's centred 600-pixel column).
    pub(super) fn fixed_width(&self) -> bool {
        matches!(self.tag.as_str(), "div" | "center" | "section" | "article" | "main" | "header" | "footer")
            && matches!(self.width(), Some(Width::Px(_)))
    }

    /// The content width of a floated box or a table cell when it's one picture of a known width
    /// (see [`Cell::fit`]).
    pub(super) fn picture_width(&self, depth: usize) -> Option<u32> {
        let mut found: Option<u32> = None;
        let mut pictures = 0usize;
        fn walk(n: &Node, depth: usize, found: &mut Option<u32>, pictures: &mut usize) -> bool {
            if depth > MAX_DEPTH * 4 || n.hidden() {
                return depth <= MAX_DEPTH * 4;
            }
            if n.tag == "img" {
                *pictures += 1;
                // Its `width` in pixels first: Word writes it so, and its style in inches.
                let attr = n.attr("width").and_then(|w| w.trim().trim_end_matches("px").trim().parse::<f32>().ok()).filter(|w| *w >= 1.0);
                *found = match (attr, n.width()) {
                    (Some(w), _) => Some(w.min(4000.0) as u32),
                    (None, Some(Width::Px(px))) => Some(px),
                    _ => None,
                };
                return true;
            }
            n.kids.iter().all(|k| match k {
                Kid::Text(t) => t.chars().all(|c| c.is_whitespace() || c == '\u{a0}'),
                Kid::El(e) => walk(e, depth + 1, found, pictures),
            })
        }
        (walk(self, depth, &mut found, &mut pictures) && pictures == 1).then_some(found).flatten()
    }
}

/// A CSS or attribute width: `600`, `600px`, `80%`.
fn parse_width(v: &str) -> Option<Width> {
    let v = v.trim();
    if let Some(p) = v.strip_suffix('%') {
        return p.trim().parse::<f32>().ok().filter(|p| *p > 0.0).map(|p| Width::Percent(p.min(100.0) as u8));
    }
    let px = v.trim_end_matches("px").trim().parse::<f32>().ok()?;
    (px >= 1.0).then(|| Width::Px(px.min(4000.0) as u32))
}
