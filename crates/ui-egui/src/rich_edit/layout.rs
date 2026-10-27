//! The message editor's text laid out: each run in its style, pictures, lists' indents, alignment.

use std::collections::{BTreeSet, HashMap};

use super::*;

/// A look as text formatting (the person's colours adjusted to stay readable on the page), in
/// its typeface when that's loaded (`typefaces`), the usual font otherwise.
pub(super) fn format(s: &Style, t: &Tokens, typefaces: &BTreeSet<String>) -> TextFormat {
    let px = f32::from(s.size()) * PX;
    let font = if s.mono {
        FontId::monospace(px * 0.92)
    } else if let Some(family) = s.font.as_deref().and_then(|name| crate::look::typefaces::family(typefaces, name, s.bold)) {
        FontId::new(px, family)
    } else if s.bold {
        theme::bold(px)
    } else {
        theme::regular(px)
    };
    let (mut color, background) = theme::mail_colors(s.color, s.highlight, t, t.text);
    if s.link.is_some() && s.color.is_none() {
        color = t.accent;
    }
    let line = Stroke::new((px / 14.0).max(1.0), color);
    TextFormat {
        font_id: font,
        color,
        background: background.unwrap_or(Color32::TRANSPARENT),
        italics: s.italic,
        underline: if s.underline || s.link.is_some() { line } else { Stroke::NONE },
        strikethrough: if s.strike { line } else { Stroke::NONE },
        ..Default::default()
    }
}

/// The size of the character a picture stands on: small (it's drawn see-through), the picture's
/// height coming from the line it's given rather than from a glyph as tall as the picture.
const PICTURE_FONT: f32 = 14.0;

/// A picture's place in the text: the height of its line, and the room after its character (an
/// empty run right after it, so the room stays on the picture's line when what follows goes to
/// the next), the two making the picture's size. (As letter spacing the room went before the
/// character, and not at all at the start of a line.)
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Room {
    pub height: f32,
    pub gap: f32,
}

/// Room for a picture: an invisible character the picture is drawn over, on a line `height`
/// tall.
pub(super) fn picture_format(height: f32) -> TextFormat {
    TextFormat { font_id: theme::regular(PICTURE_FONT), color: Color32::TRANSPARENT, line_height: Some(height), ..Default::default() }
}

/// The size a picture whose own isn't known yet shows at.
const UNKNOWN_PICTURE: Vec2 = Vec2::new(PICTURE_PX * 1.4, PICTURE_PX);

/// The size a picture shows at in text `wrap` wide: its own (`size`, in points, as Outlook
/// shows a picture; [`UNKNOWN_PICTURE`] when it isn't known), brought down to the width.
pub(super) fn shown_size(size: Option<Vec2>, wrap: f32) -> Vec2 {
    let size = size.unwrap_or(UNKNOWN_PICTURE);
    let widest = if wrap.is_finite() { (wrap - 4.0).max(24.0) } else { f32::INFINITY };
    let scale = (widest / size.x.max(1.0)).min(1.0);
    vec2(size.x * scale, (size.y * scale).max(8.0))
}

/// Each picture's room ([`shown_size`]); one not in `sizes` gets [`UNKNOWN_PICTURE`]'s (`""`
/// in the map).
pub(super) fn rooms(ui: &Ui, sizes: &HashMap<String, Vec2>, wrap: f32) -> HashMap<String, Room> {
    let advance = ui.fonts_mut(|f| f.layout_no_wrap(PICTURE.to_string(), theme::regular(PICTURE_FONT), Color32::TRANSPARENT)).size().x;
    let room = |size: Option<Vec2>| {
        let s = shown_size(size, wrap);
        Room { height: s.y, gap: (s.x - advance).max(0.0) }
    };
    let mut out: HashMap<String, Room> = sizes.iter().map(|(cid, s)| (cid.clone(), room(Some(*s)))).collect();
    out.insert(String::new(), room(None));
    out
}

/// The text laid out with its styles; `check` says how its spelling is checked, `typefaces`
/// which typefaces can be drawn.
pub(super) fn job(
    text: &str,
    st: &Styles,
    plain: bool,
    t: &Tokens,
    wrap: f32,
    check: &Check,
    typefaces: &BTreeSet<String>,
    rooms: &HashMap<String, Room>,
) -> LayoutJob {
    let unknown = rooms.get("").copied().unwrap_or(Room { height: PICTURE_PX, gap: PICTURE_PX });
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    // Where a paragraph goes is decided here (`layout_aligned`), against the editor's width;
    // egui would put a right-to-left one against the widest line instead.
    job.halign_min_is_start = false;
    let plain_fmt = TextFormat { font_id: theme::regular(14.5), color: t.text, ..Default::default() };
    let mut i = 0usize;
    for (n, line) in text.split('\n').enumerate() {
        if n > 0 {
            let f = if plain { plain_fmt.clone() } else { format(&st.style(i.saturating_sub(1)), t, typefaces) };
            job.append("\n", 0.0, f);
            i += 1;
        }
        let p = st.para(n);
        let mut lead = if plain { 0.0 } else { f32::from(p.indent) * INDENT + if p.list == List::None { 0.0 } else { INDENT } };
        let mut run_start = 0usize;
        let chars: Vec<(usize, char)> = line.char_indices().collect();
        for k in 0..chars.len() {
            let cur = st.chars.get(i + k).copied();
            let next = st.chars.get(i + k + 1).copied();
            let pic = chars.get(k).is_some_and(|(_, ch)| *ch == PICTURE) || chars.get(k + 1).is_some_and(|(_, ch)| *ch == PICTURE);
            if k + 1 == chars.len() || cur != next || pic {
                let from = chars.get(run_start).map_or(line.len(), |(b, _)| *b);
                let to = chars.get(k + 1).map_or(line.len(), |(b, _)| *b);
                let piece = line.get(from..to).unwrap_or("");
                let cid = if plain || !piece.starts_with(PICTURE) { None } else { st.style(i + run_start).picture };
                let room = cid.as_ref().map(|c| rooms.get(c).copied().unwrap_or(unknown));
                let f = match room {
                    _ if plain => plain_fmt.clone(),
                    Some(r) => picture_format(r.height),
                    None => format(&st.style(i + run_start), t, typefaces),
                };
                job.append(piece, lead, f);
                lead = 0.0;
                // The picture's room, on its line whatever comes next.
                if let Some(r) = room {
                    job.append("", r.gap, picture_format(r.height));
                }
                run_start = k + 1;
            }
        }
        if chars.is_empty() && lead > 0.0 {
            let f = if plain { plain_fmt.clone() } else { format(&st.style(i), t, typefaces) };
            job.append("", lead, f);
        }
        i += chars.len();
    }
    // What isn't spelt right gets a red line under it (nothing before the dictionaries are read).
    crate::spellcheck::underline_with(&mut job, text, t.danger, check);
    job
}

/// egui puts an empty line at the left edge whatever its indent, so the caret on an empty
/// bulleted or indented line sat before its bullet: such rows are moved to their indent.
pub(super) fn indent_empty_rows(g: Arc<Galley>, st: &Styles, plain: bool) -> Arc<Galley> {
    if plain {
        return g;
    }
    let lead = |n: usize| {
        let p = st.para(n);
        f32::from(p.indent) * INDENT + if p.list == List::None { 0.0 } else { INDENT }
    };
    let mut para = 0usize;
    let mut moves = Vec::new();
    for (i, r) in g.rows.iter().enumerate() {
        if r.row.glyphs.is_empty() && lead(para) > 0.0 {
            moves.push((i, lead(para)));
        }
        if r.ends_with_newline {
            para += 1;
        }
    }
    if moves.is_empty() {
        return g;
    }
    let mut out = (*g).clone();
    for (i, x) in moves {
        if let Some(r) = out.rows.get_mut(i) {
            r.pos.x = x;
        }
    }
    Arc::new(out)
}

/// Lay the text out; centred and right-aligned paragraphs are moved over, row by row.
/// `pictures`: each picture's own size (points), for its room.
pub(super) fn layout(
    ui: &Ui,
    text: &str,
    st: &Styles,
    plain: bool,
    t: &Tokens,
    wrap: f32,
    check: &Check,
    pictures: &HashMap<String, Vec2>,
) -> Arc<Galley> {
    indent_empty_rows(layout_aligned(ui, text, st, plain, t, wrap, check, pictures), st, plain)
}

/// The text laid out, each row of a centred or right-aligned paragraph moved over by what's
/// left of the width (every row, so a paragraph longer than a line is aligned whole).
pub(super) fn layout_aligned(
    ui: &Ui,
    text: &str,
    st: &Styles,
    plain: bool,
    t: &Tokens,
    wrap: f32,
    check: &Check,
    pictures: &HashMap<String, Vec2>,
) -> Arc<Galley> {
    let typefaces = crate::look::typefaces::loaded(ui.ctx());
    let rooms = rooms(ui, pictures, wrap);
    let first = ui.painter().layout_job(job(text, st, plain, t, wrap, check, &typefaces, &rooms));
    // A right-to-left paragraph left as it was sits against the right edge, in plain text too
    // (which has no alignment of its own).
    let lines: Vec<&str> = text.split('\n').collect();
    let align_of = |n: usize, a: Align| {
        let a = if plain { Align::Left } else { a };
        if a == Align::Left && lines.get(n).is_some_and(|l| crate::bidi::has_rtl(l) && crate::bidi::paragraph_rtl(l)) { Align::Right } else { a }
    };
    if !wrap.is_finite() || st.paras.iter().enumerate().all(|(n, p)| align_of(n, p.align) == Align::Left) {
        return first;
    }
    let mut out = (*first).clone();
    let mut row = 0;
    let mut bounds: Option<Rect> = None;
    for (n, p) in st.paras.iter().enumerate() {
        // The rows this paragraph takes.
        let start = row;
        while let Some(r) = out.rows.get(row) {
            row += 1;
            if r.ends_with_newline {
                break;
            }
        }
        let align = align_of(n, p.align);
        if align == Align::Left {
            continue;
        }
        for r in out.rows.iter_mut().take(row).skip(start) {
            let room = (wrap - r.rect().width()).max(0.0);
            r.pos.x = if align == Align::Center { room / 2.0 } else { room };
            let rect = r.rect();
            bounds = Some(bounds.map_or(rect, |b| b.union(rect)));
        }
    }
    // What's drawn reaches further right now: the galley's bounds say so, or it would be
    // clipped as if it still sat at the left.
    if let Some(b) = bounds {
        out.rect = out.rect.union(b);
        out.mesh_bounds = out.mesh_bounds.union(b);
    }
    Arc::new(out)
}
