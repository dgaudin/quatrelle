// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Long message text laid out by what is in view (adapted from upstream 7020f7b). A message of a
//! hundred thousand characters was one galley, laid out whole in the frame it opened (60 ms for
//! 127,000 characters of plain text, 100 ms for 300,000), and hashed again each frame after (12
//! and 31 ms). Its text goes in chunks and its formatted lines in blocks (the tables of its
//! layout one by one): those near the view are laid out, the others keep their place by height
//! (measured once laid out, guessed before), and a couple more are measured each frame so the
//! scroll bar settles. The parts a selection starts and ends in are always laid out, and Ctrl+A
//! then a copy takes the whole text.
//!
//! The fork's own: plain text keeps its web and mail addresses as links (as `plain.rs` shows
//! them), and a table not laid out keeps the numbers of its cells (what their heights are kept
//! by, `CellHeights`), so the tables after it keep theirs from frame to frame.

use std::ops::Range;

use super::*;

/// Plain text longer than this (bytes) is laid out by chunks.
pub(crate) const LONG: usize = 20_000;
/// Most bytes in a chunk: one ends at a line's end when there is one in reach.
pub(crate) const CHUNK: usize = 4096;
/// Formatted text of more lines than `MANY_LINES` is laid out in blocks of this many.
pub(crate) const BLOCK_LINES: usize = 64;
pub(crate) const MANY_LINES: usize = 2_000;
/// Parts not measured yet that are laid out in a frame besides those in view.
const MEASURED_PER_FRAME: usize = 2;
/// Texts whose parts are kept (the reading pane's, and those of messages in their own windows).
const KEPT: usize = 8;
/// What the average letter's width is measured on.
const SAMPLE: &str = "The quick brown fox jumps over the lazy dog, 0123456789.";

/// One shown text's parts: their heights, and where a selection in them starts and ends.
#[derive(Default)]
pub(crate) struct LongText {
    /// What it was made for: the text's length and its number of parts.
    of: (usize, usize),
    width_bits: u32,
    /// Heights guessed at that width, and those measured.
    guessed: Vec<f32>,
    measured: Vec<Option<f32>>,
    /// The part a selection was started in, and the one the pointer last pressed or dragged in.
    anchor: Option<usize>,
    head: Option<usize>,
    /// Ctrl+A in it: all of it is selected, and copied whole.
    pub(crate) all: bool,
    /// How many parts the last frame laid out, and where they were.
    pub(crate) laid: usize,
    pub(crate) shown: Vec<(usize, Rect)>,
}

impl LongText {
    /// Fresh for a text of `len` bytes in `n` parts, unless it already is that one's.
    fn fit(&mut self, len: usize, n: usize) {
        if self.of != (len, n) {
            *self = LongText { of: (len, n), measured: vec![None; n], ..Default::default() };
        }
    }

    /// Each part's guessed height, and its measured one once laid out.
    #[cfg(test)]
    pub(crate) fn heights(&self) -> impl Iterator<Item = (f32, Option<f32>)> + '_ {
        self.guessed.iter().copied().zip(self.measured.iter().copied())
    }
}

/// `text` cut in pieces of at most `max` bytes: at a line's end, the line break in neither piece,
/// or for a longer line at a character's edge. No piece is empty.
pub(crate) fn chunks(text: &str, max: usize) -> Vec<Range<usize>> {
    let max = max.max(4);
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut start = 0usize;
    while start < text.len() {
        let cap = start.saturating_add(max);
        if cap >= text.len() {
            out.push(start..text.len());
            break;
        }
        // A line break is one byte that is never part of another character.
        let from = start.saturating_add(1);
        if let Some(at) = bytes.get(from..cap).and_then(|w| w.iter().rposition(|b| *b == b'\n')) {
            let nl = from.saturating_add(at);
            out.push(start..nl);
            start = nl.saturating_add(1);
            continue;
        }
        let mut cut = cap;
        while cut > start && !text.is_char_boundary(cut) {
            cut -= 1;
        }
        if cut == start {
            cut = start.saturating_add(1);
            while cut < text.len() && !text.is_char_boundary(cut) {
                cut = cut.saturating_add(1);
            }
        }
        out.push(start..cut);
        start = cut;
    }
    out
}

/// How many rows `text` wraps to at `per_row` letters a row.
fn rows(text: &str, per_row: usize) -> usize {
    let per_row = per_row.max(1);
    text.split('\n').map(|l| l.chars().count().div_ceil(per_row).max(1)).fold(0usize, usize::saturating_add)
}

/// The height of a row of `font`, and the width of one of its letters (on average). The row is
/// measured on laid-out lines: they're rounded to whole pixels, a font's row height isn't (at
/// 14.5 points, 17 against 16.66: a guess a third of a pixel short a row, 20 pixels a chunk).
fn metrics(ui: &Ui, font: &egui::FontId) -> (f32, f32) {
    const ROWS: usize = 8;
    ui.fonts_mut(|f| {
        let lines = [SAMPLE; ROWS].join("\n");
        let size = f.layout_no_wrap(lines, font.clone(), Color32::WHITE).size();
        let row = if size.y > 0.0 { size.y / ROWS as f32 } else { f.row_height(font) };
        (row, (size.x / SAMPLE.len() as f32).max(1.0))
    })
}

/// The state of the text shown under `id`, taken out of `st` while it's drawn.
fn take(st: &mut UiState, id: egui::Id) -> LongText {
    st.long_text.remove(&id).unwrap_or_default()
}

fn put_back(st: &mut UiState, id: egui::Id, v: LongText) {
    if st.long_text.len() >= KEPT {
        st.long_text.clear();
    }
    st.long_text.insert(id, v);
}

/// A long plain text (`LONG`) of the message `key`, in chunks: its addresses links, as
/// [`plain::plain_body`] shows them; right-to-left text keeps its own labels.
pub(super) fn plain(st: &mut UiState, ui: &mut Ui, key: &MessageKey, text: &str, acts: &mut Vec<Action>) {
    let id = ui.scope_id().with(("long-text", key.to_id()));
    // Where the addresses laid out start, for the tests to point at.
    #[cfg(test)]
    ui.ctx().data_mut(|d| d.remove::<Vec<Rect>>(egui::Id::unique("long-text-links")));
    let t = st.tokens;
    let font = theme::regular(14.5);
    let rtl = crate::bidi::has_rtl(text);
    let ranges = chunks(text, CHUNK);
    let mut v = take(st, id);
    v.fit(text.len(), ranges.len());
    let (row, letter) = metrics(ui, &font);
    let guess = |i: usize, width: f32| {
        let s = ranges.get(i).and_then(|r| text.get(r.clone())).unwrap_or_default();
        rows(s, (width / letter) as usize) as f32 * row
    };
    ui.scope(|ui| {
        // Chunks one under the other as the lines of one text (and copied so, a line break apart).
        ui.spacing_mut().item_spacing.y = 0.0;
        parts(ui, &mut v, text, guess, |ui, i, lay| {
            let Some(s) = ranges.get(i).and_then(|r| text.get(r.clone())).filter(|_| lay) else { return };
            if rtl {
                let r = crate::bidi::selectable_label(ui, s, font.clone(), t.text);
                rtl_copy_menu(&r, None, text);
            } else {
                linked_chunk(&t, ui, s, &font, acts);
            }
        });
    });
    put_back(st, id, v);
}

/// A chunk of plain text, selectable with the rest of it, its web and mail addresses links: a
/// hand and the address under the pointer, opened on a click (a drag still selects).
fn linked_chunk(t: &Tokens, ui: &mut Ui, s: &str, font: &egui::FontId, acts: &mut Vec<Action>) {
    let links = mime::find_links(s);
    let plain = egui::TextFormat { font_id: font.clone(), color: t.text, ..Default::default() };
    let link = egui::TextFormat { font_id: font.clone(), color: t.accent, underline: egui::Stroke::new(1.0, t.accent), ..Default::default() };
    let mut job = egui::text::LayoutJob::default();
    let mut at = 0;
    // Each address in characters too (the galley counts characters, the text's ranges bytes).
    let mut chars: Vec<Range<usize>> = Vec::with_capacity(links.len());
    let mut count = 0usize;
    for (r, _) in &links {
        if let (Some(before), Some(address)) = (s.get(at..r.start), s.get(r.clone())) {
            job.append(before, 0.0, plain.clone());
            job.append(address, 0.0, link.clone());
            count = count.saturating_add(before.chars().count());
            let len = address.chars().count();
            chars.push(count..count.saturating_add(len));
            count = count.saturating_add(len);
            at = r.end;
        }
    }
    job.append(s.get(at..).unwrap_or(""), 0.0, plain);
    job.wrap.max_width = ui.available_width();
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let r = ui.add(egui::Label::new(galley.clone()).selectable(true));
    if links.is_empty() {
        return;
    }
    #[cfg(test)]
    {
        let at: Vec<Rect> = chars.iter().map(|c| galley.pos_from_cursor(egui::text::CCursor::new(c.start)).translate(r.rect.min.to_vec2())).collect();
        ui.ctx().data_mut(|d| d.get_temp_mut_or_default::<Vec<Rect>>(egui::Id::unique("long-text-links")).extend(at));
    }
    let under = |p: egui::Pos2| {
        let at = galley.cursor_from_pos(p - r.rect.min).index.0;
        chars.iter().position(|c| c.contains(&at)).and_then(|i| links.get(i)).map(|(_, url)| url.clone())
    };
    if let Some(url) = r.hover_pos().and_then(under) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        egui::Response::clone(&r).on_hover_text_at_pointer(url);
    }
    if r.clicked()
        && let Some(url) = r.interact_pointer_pos().and_then(under)
    {
        acts.push(Action::OpenUrl(url));
    }
}

/// A part of formatted text: some of its lines, or a table of its layout.
enum Unit<'a> {
    Lines(&'a [Vec<mime::Span>]),
    Table(&'a mime::layout::Block),
}

fn units(p: &mime::Parsed) -> Vec<Unit<'_>> {
    if p.layout.is_empty() {
        return p.rich.chunks(BLOCK_LINES).map(Unit::Lines).collect();
    }
    let mut out = Vec::new();
    for b in &p.layout {
        match b {
            mime::layout::Block::Text(lines) => out.extend(lines.chunks(BLOCK_LINES).map(Unit::Lines)),
            mime::layout::Block::Table(_) => out.push(Unit::Table(b)),
        }
    }
    out
}

/// Whether the formatted text has lines enough (`MANY_LINES`) to be laid out by blocks.
pub(super) fn many_lines(p: &mime::Parsed) -> bool {
    let n = if p.layout.is_empty() {
        p.rich.len()
    } else {
        p.layout
            .iter()
            .map(|b| match b {
                mime::layout::Block::Text(lines) => lines.len(),
                mime::layout::Block::Table(_) => 1,
            })
            .fold(0usize, usize::saturating_add)
    };
    n > MANY_LINES
}

/// How many cell numbers drawing `blocks` can take (`Body::cells`), with room to spare: a box
/// placed from the bottom is drawn twice (measured, then drawn), so its cells count twice.
fn cell_numbers(blocks: &[mime::layout::Block], depth: usize) -> usize {
    if depth > 32 {
        return 0;
    }
    let mut n = 0usize;
    for b in blocks {
        if let mime::layout::Block::Table(t) = b {
            for row in &t.rows {
                n = n.saturating_add(row.len());
                for c in row {
                    n = n.saturating_add(cell_numbers(&c.blocks, depth + 1));
                    for o in &c.overlays {
                        n = n.saturating_add(cell_numbers(&o.blocks, depth + 2).saturating_mul(2));
                    }
                }
            }
        }
    }
    n
}

/// Formatted text of many lines, `width` wide on `page`, by blocks of lines (and its layout's
/// tables one by one). Pictures in the blocks passed over count as shown in place.
pub(super) fn rich(body: &mut Body, st: &mut UiState, ui: &mut Ui, width: f32, page: [u8; 3]) {
    let id = ui.scope_id().with(("long-text", body.key.to_id()));
    let p = body.p;
    let zoom = body.zoom;
    let parts_of = units(p);
    // Each table's first cell number, the same whatever is laid out above it.
    let mut first_cell = Vec::with_capacity(parts_of.len());
    let mut cells = body.cells;
    for u in &parts_of {
        first_cell.push(cells);
        if let Unit::Table(b) = u {
            cells = cells.saturating_add(cell_numbers(std::slice::from_ref(*b), 0));
        }
    }
    let mut v = take(st, id);
    v.fit(p.text.len(), parts_of.len());
    let (row, letter) = metrics(ui, &theme::regular(14.5 * zoom));
    let gap = ui.spacing().item_spacing.y;
    let guess = |i: usize, width: f32| match parts_of.get(i) {
        Some(Unit::Lines(lines)) => lines
            .iter()
            .map(|line| {
                let text: usize = line.iter().map(|sp| sp.text.chars().count()).fold(0, usize::saturating_add);
                let pictures = if line.iter().any(|sp| sp.image.is_some()) { 120.0 * zoom } else { 0.0 };
                if line.is_empty() { 9.0 * zoom } else { text.div_ceil(((width / letter) as usize).max(1)).max(1) as f32 * row + gap + pictures }
            })
            .sum(),
        Some(Unit::Table(mime::layout::Block::Table(t))) => t.rows.len().min(1_000) as f32 * 3.0 * row,
        _ => 0.0,
    };
    parts(ui, &mut v, &p.text, guess, |ui, i, lay| {
        body.cells = first_cell.get(i).copied().unwrap_or(cells);
        match (parts_of.get(i), lay) {
            (Some(Unit::Lines(lines)), true) => body.lines(st, ui, lines, width, page, Align::Min),
            (Some(Unit::Table(b)), true) => body.blocks(st, ui, std::slice::from_ref(*b), width, page, mime::layout::Align::Left, 0),
            (Some(Unit::Lines(lines)), false) => pictures_of_lines(lines, body.shown),
            (Some(Unit::Table(b)), false) => pictures_of_blocks(std::slice::from_ref(*b), body.shown, 0),
            (None, _) => {}
        }
    });
    body.cells = cells;
    put_back(st, id, v);
}

fn pictures_of_lines(lines: &[Vec<mime::Span>], out: &mut Vec<String>) {
    out.extend(lines.iter().flatten().filter_map(|sp| sp.image.clone()));
}

/// The pictures laid-out parts show (as [`Body::blocks`] goes, as deep).
fn pictures_of_blocks(blocks: &[mime::layout::Block], out: &mut Vec<String>, depth: usize) {
    for b in blocks {
        match b {
            mime::layout::Block::Text(lines) => pictures_of_lines(lines, out),
            mime::layout::Block::Table(t) if depth < 16 => {
                out.extend(t.background_image.clone());
                for c in t.rows.iter().flatten() {
                    out.extend(c.background_image.clone());
                    pictures_of_blocks(&c.blocks, out, depth + 1);
                    for o in &c.overlays {
                        pictures_of_blocks(&o.blocks, out, depth + 2);
                    }
                }
            }
            mime::layout::Block::Table(_) => {}
        }
    }
}

/// Lay out `v`'s parts one under the other: those near the view, those a selection starts or ends
/// in (and their neighbours), all a copied selection spans, and a few not measured yet.
/// `draw(ui, i, true)` lays part `i` out; `draw(ui, i, false)` says it's passed over, its room kept
/// by its height; `guess(i, width)` is that before it's measured. `whole` is what Ctrl+A copies.
fn parts(ui: &mut Ui, v: &mut LongText, whole: &str, guess: impl Fn(usize, f32) -> f32, mut draw: impl FnMut(&mut Ui, usize, bool)) {
    let n = v.measured.len();
    let width = ui.available_width();
    if v.width_bits != width.to_bits() || v.guessed.len() != n {
        v.width_bits = width.to_bits();
        v.guessed = (0..n).map(|i| guess(i, width)).collect();
        v.measured.iter_mut().for_each(|h| *h = None);
    }
    let (pressed, down, shift, pos, copying, select_all, escape) = ui.input(|i| {
        let ctrl_a = i.events.iter().any(|e| matches!(e, egui::Event::Key { key: egui::Key::A, pressed: true, modifiers, .. } if modifiers.command));
        let copying = i.events.iter().any(|e| matches!(e, egui::Event::Copy | egui::Event::Cut));
        let p = &i.pointer;
        (p.primary_pressed(), p.primary_down(), i.modifiers.shift, p.interact_pos(), copying, ctrl_a, i.key_pressed(egui::Key::Escape))
    });
    let selecting = ui.ctx().plugin_opt::<egui::text_selection::LabelSelectionState>().is_some_and(|p| p.lock().has_selection());
    // Keys gone to a box (Ctrl+E to the search box): Ctrl+A and a copy are its own.
    let typing = ui.memory(|m| m.focused().is_some());
    if pressed || escape || !selecting || typing {
        v.all = false;
    }
    if pressed && !shift {
        v.anchor = None;
        v.head = None;
    }
    if select_all && selecting && !typing && v.anchor.is_some() {
        v.all = true;
    }
    // Laid out whatever the view: the ends of a selection (egui lets go of one whose ends it
    // doesn't see), and for a copy, everything between them.
    let ends: Vec<usize> = [v.anchor, v.head].into_iter().flatten().collect();
    let span = match (v.anchor, v.head) {
        (Some(a), Some(h)) if copying && !v.all => a.min(h)..a.max(h).saturating_add(1),
        _ => 0..0,
    };
    let clip = ui.clip_rect();
    let near = clip.expand2(vec2(0.0, clip.height()));
    let left = ui.cursor().left();
    let fill = ui.visuals().selection.bg_fill.gamma_multiply(0.5);
    // None measured in the frame the text first shows, which has enough to lay out in view.
    let mut budget = if v.measured.iter().any(Option::is_some) { MEASURED_PER_FRAME } else { 0 };
    v.laid = 0;
    v.shown.clear();
    for i in 0..n {
        let top = ui.cursor().top();
        let known = v.measured.get(i).copied().flatten();
        let h = known.or_else(|| v.guessed.get(i).copied()).unwrap_or(0.0);
        let in_view = top <= near.bottom() && top + h >= near.top();
        let pinned = ends.iter().any(|e| e.abs_diff(i) <= 1) || span.contains(&i);
        let measure = known.is_none() && !in_view && !pinned && budget > 0;
        if measure {
            budget -= 1;
        }
        let lay = in_view || pinned || measure;
        // egui keeps a selection only while it sees both its ends, and copies only what it sees:
        // a selection's part out of sight is laid out unseen, in a sliver of view of its own.
        let unseen = pinned && !Rect::from_min_size(egui::pos2(left, top), vec2(width, h.max(1.0))).intersects(clip);
        let r = ui.push_id(i, |ui| {
            if !lay {
                draw(ui, i, false);
                ui.allocate_space(vec2(width, h));
                return None;
            }
            ui.scope(|ui| {
                if unseen {
                    ui.set_clip_rect(Rect::from_min_size(egui::pos2(left, top), vec2(width, 1.0)));
                    ui.multiply_opacity(0.0);
                    ui.disable();
                }
                let slot = v.all.then(|| ui.painter().add(egui::Shape::Noop));
                draw(ui, i, true);
                slot
            })
            .inner
        });
        if !lay {
            continue;
        }
        let rect = Rect::from_x_y_ranges(left..=left + width, r.response.rect.y_range());
        if let Some(slot) = r.inner {
            ui.painter().set(slot, egui::Shape::rect_filled(rect, 0.0, fill));
        }
        if let Some(m) = v.measured.get_mut(i) {
            *m = Some(r.response.rect.height());
        }
        v.laid += 1;
        v.shown.push((i, rect));
        if let Some(p) = pos
            && rect.intersect(clip).contains(p)
        {
            if pressed && (!shift || v.anchor.is_none()) {
                v.anchor = Some(i);
            }
            if pressed || down {
                v.head = Some(i);
            }
        }
    }
    if v.head.is_none() {
        v.head = v.anchor;
    }
    // All of it selected: the copy is the whole text, not what's laid out of it.
    if v.all && copying {
        ui.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Copy | egui::Event::Cut)));
        ui.ctx().copy_text(whole.to_string());
    }
    if budget < MEASURED_PER_FRAME && v.measured.iter().any(Option::is_none) {
        ui.ctx().request_repaint();
    }
}
