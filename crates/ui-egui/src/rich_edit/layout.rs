//! The message editor's text laid out: each run in its style, pictures, lists' indents, alignment.

use std::collections::BTreeSet;

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

/// Room for a picture: an invisible, wide character the picture is drawn over.
pub(super) fn picture_format() -> TextFormat {
    TextFormat { font_id: theme::regular(PICTURE_PX), color: Color32::TRANSPARENT, extra_letter_spacing: PICTURE_PX * 0.8, ..Default::default() }
}

/// The text laid out with its styles; `check` says how its spelling is checked, `typefaces`
/// which typefaces can be drawn.
pub(super) fn job(text: &str, st: &Styles, plain: bool, t: &Tokens, wrap: f32, check: &Check, typefaces: &BTreeSet<String>) -> LayoutJob {
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
                let f = if plain {
                    plain_fmt.clone()
                } else if piece.starts_with(PICTURE) && st.style(i + run_start).picture.is_some() {
                    picture_format()
                } else {
                    format(&st.style(i + run_start), t, typefaces)
                };
                job.append(piece, lead, f);
                lead = 0.0;
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
pub(super) fn layout(ui: &Ui, text: &str, st: &Styles, plain: bool, t: &Tokens, wrap: f32, check: &Check) -> Arc<Galley> {
    indent_empty_rows(layout_aligned(ui, text, st, plain, t, wrap, check), st, plain)
}

/// The text laid out, each row of a centred or right-aligned paragraph moved over by what's
/// left of the width (every row, so a paragraph longer than a line is aligned whole).
pub(super) fn layout_aligned(ui: &Ui, text: &str, st: &Styles, plain: bool, t: &Tokens, wrap: f32, check: &Check) -> Arc<Galley> {
    let typefaces = crate::look::typefaces::loaded(ui.ctx());
    let first = ui.painter().layout_job(job(text, st, plain, t, wrap, check, &typefaces));
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
