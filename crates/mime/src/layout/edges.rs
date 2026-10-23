//! The edges of boxes and cells: borders, rounded corners, padding and where a cell's content
//! sits in its row, read from CSS (`border`, its sides and their parts, `border-radius`,
//! `padding`, `vertical-align`) and the table attributes (`border`, `cellpadding`, `valign`).

use super::*;

/// The widest border drawn, in pixels.
pub const MAX_BORDER: u8 = 8;
/// The most padding on one side of a cell, in pixels.
pub const MAX_PADDING: u8 = 64;
/// The roundest corner, in pixels.
pub const MAX_RADIUS: u8 = 40;
/// A border without a colour of its own (`border: 1px solid`, a table's `border="1"`): grey.
const PLAIN: [u8; 3] = [0x99, 0x99, 0x99];

/// A line along one edge of a box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Border {
    /// In pixels, 1 to [`MAX_BORDER`].
    pub width: u8,
    pub color: [u8; 3],
}

/// The lines a box draws along its edges.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edges {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top: Option<Border>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right: Option<Border>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bottom: Option<Border>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left: Option<Border>,
}

impl Edges {
    /// Whether no edge draws a line.
    pub fn is_none(&self) -> bool {
        self.top.is_none() && self.right.is_none() && self.bottom.is_none() && self.left.is_none()
    }

    /// The one border all four edges share, when they do (drawn as a frame, round corners too).
    pub fn uniform(&self) -> Option<Border> {
        let b = self.top?;
        (self.right == Some(b) && self.bottom == Some(b) && self.left == Some(b)).then_some(b)
    }
}

/// Where a cell's content sits in its row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VAlign {
    #[default]
    Top,
    Middle,
    Bottom,
}

/// A cell's padding: top, right, bottom and left, in pixels.
pub type Padding = [u8; 4];

/// A CSS length in pixels: `px`, `pt`, `em`/`rem` (16 pixels), `cm`, `mm`, `in`, `pc`, or a
/// bare number (pixels); `None` for anything else (`auto`, `50%`).
pub(super) fn length_px(v: &str) -> Option<f32> {
    let v = v.trim().to_ascii_lowercase();
    let at = v.find(|c: char| c.is_ascii_alphabetic() || c == '%').unwrap_or(v.len());
    let (n, unit) = (v.get(..at)?, v.get(at..)?);
    let n = n.trim().parse::<f32>().ok().filter(|n| n.is_finite())?;
    let px = match unit {
        "" | "px" => n,
        "pt" => n * 4.0 / 3.0,
        "em" | "rem" => n * 16.0,
        "cm" => n * 96.0 / 2.54,
        "mm" => n * 96.0 / 25.4,
        "in" => n * 96.0,
        "pc" => n * 16.0,
        _ => return None,
    };
    Some(px.clamp(-4000.0, 4000.0))
}

/// Colours by name, the common ones (`red`, `gray`, `silver`…).
pub(super) fn named_color(name: &str) -> Option<[u8; 3]> {
    Some(match name {
        "white" => [255, 255, 255],
        "black" => [0, 0, 0],
        "red" => [255, 0, 0],
        "blue" => [0, 0, 255],
        "green" => [0, 128, 0],
        "lime" => [0, 255, 0],
        "yellow" => [255, 255, 0],
        "orange" => [255, 165, 0],
        "navy" => [0, 0, 128],
        "maroon" => [128, 0, 0],
        "purple" => [128, 0, 128],
        "teal" => [0, 128, 128],
        "olive" => [128, 128, 0],
        "aqua" | "cyan" => [0, 255, 255],
        "fuchsia" | "magenta" => [255, 0, 255],
        "gray" | "grey" => [128, 128, 128],
        "dimgray" | "dimgrey" => [105, 105, 105],
        "darkgray" | "darkgrey" => [169, 169, 169],
        "silver" => [192, 192, 192],
        "lightgray" | "lightgrey" => [211, 211, 211],
        "gainsboro" => [220, 220, 220],
        "whitesmoke" => [245, 245, 245],
        _ => return None,
    })
}

/// A value's words, a function's arguments kept together (`1px solid rgb(1, 2, 3)`).
fn words(v: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut depth = 0usize;
    for c in v.chars() {
        match c {
            '(' => {
                depth = depth.saturating_add(1);
                word.push(c);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                word.push(c);
            }
            c if c.is_whitespace() && depth == 0 => {
                if !word.is_empty() {
                    out.push(std::mem::take(&mut word));
                }
            }
            // Spaces inside the parentheses go, so the colour reads as one word.
            c if c.is_whitespace() => {}
            c => word.push(c),
        }
        if out.len() > 16 {
            break;
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

/// The declarations of an inline style, in order (lower case, `!important` dropped).
fn declarations(style: &str) -> Vec<(String, String)> {
    style
        .split(';')
        .filter_map(|d| {
            let (k, v) = d.split_once(':')?;
            let v = v.trim().to_ascii_lowercase();
            let v = v.get(..v.find("!important").unwrap_or(v.len())).unwrap_or("").trim().to_string();
            (!v.is_empty()).then(|| (k.trim().to_ascii_lowercase(), v))
        })
        .take(200)
        .collect()
}

/// A property of one to four values (`padding: 4px 8px`) as top, right, bottom and left.
fn four(words: &[String]) -> Option<[&str; 4]> {
    let w = |i: usize| words.get(i).map(String::as_str);
    Some(match words.len() {
        1 => [w(0)?; 4],
        2 => [w(0)?, w(1)?, w(0)?, w(1)?],
        3 => [w(0)?, w(1)?, w(2)?, w(1)?],
        4 => [w(0)?, w(1)?, w(2)?, w(3)?],
        _ => return None,
    })
}

/// One edge's parts while the declarations are read, as CSS keeps them apart: its width, whether
/// its style draws it, and its colour (`clear`: transparent).
#[derive(Clone, Copy, Default)]
struct Side {
    width: Option<f32>,
    drawn: Option<bool>,
    color: Option<[u8; 3]>,
    clear: bool,
}

impl Side {
    /// One word of a shorthand (`1px`, `solid`, `#ccc`); whether it meant anything.
    fn word(&mut self, w: &str) -> bool {
        match w {
            "none" | "hidden" => self.drawn = Some(false),
            "solid" | "dotted" | "dashed" | "double" | "groove" | "ridge" | "inset" | "outset" => self.drawn = Some(true),
            "thin" => self.width = Some(1.0),
            "medium" => self.width = Some(3.0),
            "thick" => self.width = Some(5.0),
            "transparent" => self.clear = true,
            "currentcolor" | "initial" | "inherit" | "unset" => {}
            _ => {
                if let Some(px) = length_px(w) {
                    self.width = Some(px);
                } else if let Some(c) = color(w) {
                    self.color = Some(c);
                    self.clear = false;
                } else {
                    return false;
                }
            }
        }
        true
    }

    /// The border it draws, if any: a drawn style, a width of at least half a pixel, not
    /// transparent; without a colour of its own, grey.
    fn border(self) -> Option<Border> {
        if self.drawn != Some(true) || self.clear {
            return None;
        }
        let px = self.width.unwrap_or(3.0);
        if px < 0.5 {
            return None;
        }
        let width = u8::try_from(px.round().max(1.0).min(f32::from(MAX_BORDER)) as u32).unwrap_or(MAX_BORDER);
        Some(Border { width, color: self.color.unwrap_or(PLAIN) })
    }
}

/// Pixels as a cell's padding or a corner's radius: rounded, never negative, capped.
fn capped(px: f32, most: u8) -> u8 {
    u8::try_from(px.round().max(0.0).min(f32::from(most)) as u32).unwrap_or(most)
}

impl Node {
    /// What a table's `border="N"` attribute draws, as browsers do: `N` pixels of grey around
    /// the table, and 1 pixel around each of its cells.
    pub(super) fn border_attr(&self) -> (Option<Border>, Option<Border>) {
        let n = self.attr("border").and_then(|b| b.trim().parse::<f32>().ok()).filter(|n| n.is_finite() && *n >= 0.5);
        match n {
            Some(n) => (Some(Border { width: capped(n, MAX_BORDER).max(1), color: PLAIN }), Some(Border { width: 1, color: PLAIN })),
            None => (None, None),
        }
    }

    /// Its borders: `from` on every edge to start with (a table's `border` attribute, for it or
    /// its cells), then its style's `border` shorthand, the sides (`border-left`) and their
    /// parts (`border-left-color`, `border-width`…), in order: the later wins, as in CSS.
    pub(super) fn borders(&self, from: Option<Border>) -> Edges {
        let start = from.map_or(Side::default(), |b| Side { width: Some(f32::from(b.width)), drawn: Some(true), color: Some(b.color), clear: false });
        let mut sides = [start; 4];
        let names = ["top", "right", "bottom", "left"];
        for (prop, value) in declarations(self.attr("style").unwrap_or("")) {
            let Some(rest) = prop.strip_prefix("border") else { continue };
            let ws = words(&value);
            match rest {
                // The whole border, every edge: unsaid parts go back to their defaults.
                "" => {
                    let mut s = Side::default();
                    for w in &ws {
                        s.word(w);
                    }
                    sides = [s; 4];
                }
                "-width" | "-style" | "-color" => {
                    let Some(each) = four(&ws) else { continue };
                    for (side, w) in sides.iter_mut().zip(each) {
                        side.word(w);
                    }
                }
                _ => {
                    let Some(i) = names.iter().position(|n| rest.strip_prefix('-').is_some_and(|r| r.starts_with(n))) else { continue };
                    let part = rest.get(1 + names.get(i).map_or(0, |n| n.len())..).unwrap_or("");
                    let Some(side) = sides.get_mut(i) else { continue };
                    match part {
                        "" => {
                            let mut s = Side::default();
                            for w in &ws {
                                s.word(w);
                            }
                            *side = s;
                        }
                        "-width" | "-style" | "-color" => {
                            if let Some(w) = ws.first() {
                                side.word(w);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        let [top, right, bottom, left] = sides;
        Edges { top: top.border(), right: right.border(), bottom: bottom.border(), left: left.border() }
    }

    /// How round its corners are (`border-radius`), in pixels; a percentage (a pill, a circle)
    /// is taken as the roundest.
    pub(super) fn radius(&self) -> u8 {
        let Some(v) = self.style("border-radius") else { return 0 };
        let Some(first) = words(&v).into_iter().next() else { return 0 };
        if first.ends_with('%') {
            return if first.trim_end_matches('%').trim().parse::<f32>().is_ok_and(|p| p >= 1.0) { MAX_RADIUS } else { 0 };
        }
        length_px(&first).map_or(0, |px| capped(px, MAX_RADIUS))
    }

    /// Its padding, when its style sets any (`padding`, `padding-left`…, in order) or the table's
    /// `cellpadding` gives its cells one; `None` for the reader's own.
    pub(super) fn padding(&self, cellpadding: Option<u8>) -> Option<Padding> {
        let mut pad: Option<Padding> = cellpadding.map(|p| [p; 4]);
        let names = ["top", "right", "bottom", "left"];
        for (prop, value) in declarations(self.attr("style").unwrap_or("")) {
            let Some(rest) = prop.strip_prefix("padding") else { continue };
            let ws = words(&value);
            if rest.is_empty() {
                if let Some(each) = four(&ws) {
                    let px = |w: &str| length_px(w).map_or(0, |px| capped(px, MAX_PADDING));
                    pad = Some([px(each[0]), px(each[1]), px(each[2]), px(each[3])]);
                }
            } else if let Some(i) = names.iter().position(|n| rest.strip_prefix('-') == Some(*n))
                && let Some(px) = ws.first().and_then(|w| length_px(w))
            {
                let mut p = pad.unwrap_or([0; 4]);
                if let Some(side) = p.get_mut(i) {
                    *side = capped(px, MAX_PADDING);
                }
                pad = Some(p);
            }
        }
        pad
    }

    /// A table's `cellpadding` attribute, in pixels.
    pub(super) fn cellpadding(&self) -> Option<u8> {
        let v = self.attr("cellpadding")?;
        let px = v.trim().trim_end_matches("px").trim().parse::<f32>().ok().filter(|n| n.is_finite())?;
        Some(capped(px, MAX_PADDING))
    }

    /// Where its content sits in its row: its `vertical-align` or `valign`, else its row's, else
    /// the middle (HTML's default for a table cell).
    pub(super) fn valign(&self, row: Option<&Node>) -> VAlign {
        let own = |n: &Node| {
            let v = n.style("vertical-align").or_else(|| n.attr("valign").map(str::to_ascii_lowercase))?;
            Some(match v.trim() {
                "top" | "text-top" => VAlign::Top,
                "bottom" | "text-bottom" => VAlign::Bottom,
                "middle" | "center" | "baseline" => VAlign::Middle,
                _ => return None,
            })
        };
        own(self).or_else(|| row.and_then(own)).unwrap_or(VAlign::Middle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(style: &str, attrs: &[(&str, &str)]) -> Node {
        let mut attrs: Vec<(String, String)> = attrs.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect();
        attrs.push(("style".into(), style.into()));
        Node { tag: "td".into(), attrs, kids: Vec::new() }
    }

    #[test]
    fn borders_follow_css_in_order() {
        let grey = Border { width: 1, color: [0xcc, 0xcc, 0xcc] };
        // Word's signature: the attribute's border taken away, one rule kept down the left.
        let word = node("border:none;border-left:solid #CCCCCC 1.0pt", &[("border", "1")]);
        let (own, cells) = word.border_attr();
        assert_eq!((own, cells), (Some(Border { width: 1, color: PLAIN }), Some(Border { width: 1, color: PLAIN })));
        assert_eq!(word.borders(own), Edges { left: Some(grey), ..Default::default() });
        // The shorthand in any order, with a named colour and a function.
        let all = node("border: solid red 2px", &[]).borders(None);
        assert_eq!(all.uniform(), Some(Border { width: 2, color: [255, 0, 0] }));
        let fun = node("border:1px solid rgb(10, 20, 30)", &[]).borders(None);
        assert_eq!(fun.top, Some(Border { width: 1, color: [10, 20, 30] }));
        // The parts apart, and the sides over the whole.
        let parts = node("border-style: solid; border-width: 1px 3px; border-color: #000 #fff", &[]).borders(None);
        assert_eq!((parts.top, parts.right), (Some(Border { width: 1, color: [0, 0, 0] }), Some(Border { width: 3, color: [255, 255, 255] })));
        let sided = node("border:1px solid #000; border-bottom-width: 0; border-top-color: #00f", &[]).borders(None);
        assert_eq!(sided.bottom, None);
        assert_eq!(sided.top, Some(Border { width: 1, color: [0, 0, 255] }));
        assert_eq!(sided.left, Some(Border { width: 1, color: [0, 0, 0] }));
        // None, hidden, zero, transparent, a style alone (medium, grey), absurd widths capped.
        assert!(node("border: 0", &[]).borders(Some(grey)).is_none());
        assert!(node("border: hidden", &[]).borders(None).is_none());
        assert!(node("border: 1px solid transparent", &[]).borders(None).is_none());
        assert_eq!(node("border: solid", &[]).borders(None).top, Some(Border { width: 3, color: PLAIN }));
        assert_eq!(node("border: 999px solid #000", &[]).borders(None).top.map(|b| b.width), Some(MAX_BORDER));
        assert_eq!(node("border: 0.2pt solid #000", &[]).borders(None).top, None, "too thin to draw");
        // The table attribute alone, and with a style that doesn't speak of borders.
        let plain = node("padding:2px", &[("border", "2")]);
        assert_eq!(plain.borders(plain.border_attr().0).uniform(), Some(Border { width: 2, color: PLAIN }));
        assert_eq!(node("", &[("border", "0")]).border_attr(), (None, None));
    }

    #[test]
    fn padding_corners_and_alignment() {
        assert_eq!(node("padding:0cm 0cm 0cm 7.5pt", &[]).padding(None), Some([0, 0, 0, 10]));
        assert_eq!(node("padding: 4px 8px", &[]).padding(None), Some([4, 8, 4, 8]));
        assert_eq!(node("padding: 1em; padding-left: 2px", &[]).padding(None), Some([16, 16, 16, 2]));
        assert_eq!(node("padding-top: 500px", &[]).padding(None), Some([MAX_PADDING, 0, 0, 0]));
        assert_eq!(node("", &[]).padding(Some(5)), Some([5; 4]));
        assert_eq!(node("padding: 0", &[]).padding(Some(5)), Some([0; 4]), "the cell's own over the table's");
        assert_eq!(node("", &[]).padding(None), None);
        assert_eq!(node("", &[("cellpadding", "12")]).cellpadding(), Some(12));
        assert_eq!(node("border-radius: 6px", &[]).radius(), 6);
        assert_eq!(node("border-radius: 50%", &[]).radius(), MAX_RADIUS);
        assert_eq!(node("border-radius: 4px 0 0 4px", &[]).radius(), 4);
        assert_eq!(node("", &[]).radius(), 0);
        let row = node("", &[("valign", "bottom")]);
        assert_eq!(node("", &[]).valign(None), VAlign::Middle);
        assert_eq!(node("", &[]).valign(Some(&row)), VAlign::Bottom);
        assert_eq!(node("vertical-align: top", &[("valign", "bottom")]).valign(Some(&row)), VAlign::Top);
        assert_eq!(node("", &[("valign", "TOP")]).valign(None), VAlign::Top);
        assert_eq!(length_px("1.0pt"), Some(4.0 / 3.0));
        assert_eq!(length_px("50%"), None);
        assert_eq!(length_px("auto"), None);
        assert_eq!(words("1px solid rgb(1, 2, 3)"), ["1px", "solid", "rgb(1,2,3)"]);
    }
}
