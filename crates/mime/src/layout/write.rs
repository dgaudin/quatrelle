// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! A laid-out message written out as HTML again, around text the caller writes: the original
//! under a reply or forward, its blocks of text changed where they stand, as it goes out. Its
//! tables, rows and cells keep their widths, colours, pictures behind them, borders, corners,
//! room (`cellspacing`, padding) and alignment, and the boxes placed over cells their places.

use super::*;

/// `blocks` as HTML, the `n`th block of text ([`text_blocks`]' order) as `text(n)` writes it.
pub fn to_html(blocks: &[Block], text: &mut dyn FnMut(usize) -> String) -> String {
    to_html_without(blocks, &|_| false, text)
}

/// [`to_html`] without the blocks `gone` names (a paragraph, a table and all it holds, taken out
/// of the original under a reply): the blocks of text left keep their numbers.
pub fn to_html_without(blocks: &[Block], gone: &dyn Fn(&Block) -> bool, text: &mut dyn FnMut(usize) -> String) -> String {
    let mut out = String::new();
    let mut n = 0usize;
    write(blocks, gone, text, &mut n, &mut out, 0);
    out
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;")
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn width(w: Option<Width>) -> String {
    match w {
        Some(Width::Px(px)) => format!(" width=\"{px}\""),
        Some(Width::Percent(pc)) => format!(" width=\"{pc}%\""),
        None => String::new(),
    }
}

fn align(a: Align) -> &'static str {
    match a {
        Align::Left => "",
        Align::Center => " align=\"center\"",
        Align::Right => " align=\"right\"",
    }
}

/// A picture behind a box, by its address (as the HTML that came gives it).
fn behind(src: Option<&str>) -> String {
    src.map(|s| format!(" background=\"{}\"", esc(s))).unwrap_or_default()
}

/// A box's borders and corners as CSS declarations.
fn edges(e: &Edges, radius: u8) -> String {
    let mut css = String::new();
    for (side, b) in [("top", e.top), ("right", e.right), ("bottom", e.bottom), ("left", e.left)] {
        if let Some(b) = b {
            css.push_str(&format!(";border-{side}:{}px solid {}", b.width.min(MAX_BORDER), hex(b.color)));
        }
    }
    if radius > 0 {
        css.push_str(&format!(";border-radius:{}px", radius.min(MAX_RADIUS)));
    }
    css
}

fn write(blocks: &[Block], gone: &dyn Fn(&Block) -> bool, text: &mut dyn FnMut(usize) -> String, n: &mut usize, out: &mut String, depth: usize) {
    if depth > MAX_DEPTH * 2 {
        return;
    }
    for b in blocks {
        if gone(b) {
            // Its blocks of text are counted all the same, written nowhere.
            write(std::slice::from_ref(b), &|_| false, &mut |_| String::new(), n, &mut String::new(), depth);
            continue;
        }
        match b {
            Block::Text(_) => {
                out.push_str(&text(*n));
                *n += 1;
            }
            Block::Table(t) => {
                let centred = if t.align == Align::Center { ";margin:0 auto" } else { "" };
                let spacing = t.spacing.unwrap_or(0);
                out.push_str(&format!(
                    "<table cellpadding=\"0\" cellspacing=\"{spacing}\" border=\"0\"{}{}{}{} style=\"border-collapse:{}{centred}{}\">",
                    width(t.width),
                    align(t.align),
                    t.background.map(|c| format!(" bgcolor=\"{}\"", hex(c))).unwrap_or_default(),
                    behind(t.background_image.as_deref()),
                    if spacing == 0 { "collapse" } else { "separate" },
                    edges(&t.borders, t.radius),
                ));
                for row in &t.rows {
                    out.push_str("<tr>");
                    for c in row {
                        let span = if c.span > 1 { format!(" colspan=\"{}\"", c.span) } else { String::new() };
                        // The reader's own room (6 pixels) where the message set none, as it was shown.
                        let [top, right, bottom, left] = c.padding.unwrap_or([6; 4]).map(|p| p.min(MAX_PADDING));
                        let valign = match c.valign {
                            VAlign::Top => "top",
                            VAlign::Middle => "middle",
                            VAlign::Bottom => "bottom",
                        };
                        out.push_str(&format!(
                            "<td valign=\"{valign}\"{}{span}{}{}{} style=\"padding:{top}px {right}px {bottom}px {left}px{}\">",
                            width(c.width),
                            c.background.map(|c| format!(" bgcolor=\"{}\"", hex(c))).unwrap_or_default(),
                            align(c.align),
                            behind(c.background_image.as_deref()),
                            edges(&c.borders, c.radius),
                        ));
                        // Boxes placed over the cell's content: in a box of its own they're placed in.
                        if !c.overlays.is_empty() {
                            out.push_str("<div style=\"position:relative\">");
                        }
                        write(&c.blocks, gone, text, n, out, depth + 1);
                        for o in &c.overlays {
                            let x = if o.from_right { format!("right:{}px", o.x) } else { format!("left:{}px", o.x) };
                            let y = if o.from_bottom { format!("bottom:{}px", o.y) } else { format!("top:{}px", o.y) };
                            let w = match o.width {
                                Some(Width::Px(px)) => format!(";width:{px}px"),
                                Some(Width::Percent(pc)) => format!(";width:{pc}%"),
                                None => String::new(),
                            };
                            let bg = o.background.map(|c| format!(";background-color:{}", hex(c))).unwrap_or_default();
                            out.push_str(&format!("<div style=\"position:absolute;{x};{y}{w}{bg};padding:4px\">"));
                            write(&o.blocks, gone, text, n, out, depth + 1);
                            out.push_str("</div>");
                        }
                        if !c.overlays.is_empty() {
                            out.push_str("</div>");
                        }
                        out.push_str("</td>");
                    }
                    out.push_str("</tr>");
                }
                out.push_str("</table>");
            }
        }
    }
}
