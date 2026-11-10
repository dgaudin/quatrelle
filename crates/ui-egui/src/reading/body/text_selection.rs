// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! A message's text selected as one text, as in Outlook's reading pane (adapted from upstream
//! 0eeb5d6a and eb02aee6). The text is drawn in runs
//! (a formatted line's spans and links, a table's cells, a long text's chunks, a right-to-left
//! line, a plain message whole), which were egui labels, each with a selection of its own: Ctrl+A
//! took the paragraph under the selection, Shift with the arrows stopped at its edges, and egui
//! let go of a selection whose ends it no longer drew, so a drag couldn't scroll the message.
//! The runs are now laid out as labels are and drawn here, and the selection is two marks in the
//! text (a run, and a letter in it): a press in the text starts one, a drag moves its end (held
//! at or past the view's top or bottom, it scrolls, faster the farther out), Shift+click,
//! Shift with the arrows, Home and End (Ctrl too) move it across runs, keeping it in view, and
//! Ctrl+A takes the whole text. A copy joins what's selected in reading order, a line break
//! between lines. The keys are the text's while it has the keyboard: from a press in it until a
//! press elsewhere (the message list keeps Ctrl+A and the arrows) or a box taking the keys; a
//! message's own window gives them to its text from the start. Another message shown in its
//! place lets go of a text's selection and keys. On a touch screen a finger dragged over the
//! text scrolls it, as over egui's labels: a press there only places the selection's start.
//!
//! The fork's: a run drawn outside a message's text (a signature's preview, the original under a
//! reply being written) is an egui label as before, selected on its own; a plain message is runs
//! too (it was a text box that couldn't be typed in), its addresses still links that open with a
//! click; the find bar's finds are lit in the runs. The keys and the moves across runs are in
//! [`moves`].

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use egui::emath::GuiRounding as _;
use egui::text::{CCursor, CharIndex, Galley, LayoutJob};
use egui::text_selection::CCursorRange;
use egui::text_selection::text_cursor_state::{is_word_char, slice_char_range};
use egui::{Color32, Key, Modifiers, Pos2, Rect, Response, Sense, Stroke, Ui, pos2, vec2};

use crate::UiState;

mod moves;
use moves::keys;

/// Within this distance of the view's top or bottom (its last pixels), or past it, a selection
/// drag scrolls; not on the text before it, which would run away under a still pointer.
const EDGE: f32 = 2.0;
/// How fast: points a second for each point the pointer is into that band, or past the view.
const EDGE_SPEED: f32 = 20.0;
/// The widest gap between two runs side by side on a row whose shading is joined (pixel
/// rounding's; a table's next cell is farther).
const MAX_SEAM: f32 = 3.0;
/// Most line breaks copied between two runs (a paragraph's gap).
const MAX_BREAKS: u8 = 2;
/// Texts whose selections are kept (the reading pane's, and those of messages in their own windows).
const KEPT: usize = 8;

/// One end of a selection: a run (by the part of a long text it's in, and its place there) and
/// a letter in it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Mark {
    part: u32,
    run: u32,
    at: CCursor,
}

impl Mark {
    /// Before and after every run: what Ctrl+A, Ctrl+Shift+Home and Ctrl+Shift+End select to,
    /// whether or not the runs at the text's ends are laid out.
    const START: Mark = Mark { part: 0, run: 0, at: CCursor { index: CharIndex(0), prefer_next_row: false } };
    const END: Mark = Mark { part: u32::MAX, run: u32::MAX, at: CCursor { index: CharIndex(usize::MAX), prefer_next_row: false } };

    fn run_key(self) -> (u32, u32) {
        (self.part, self.run)
    }

    fn key(self) -> (u32, u32, usize) {
        (self.part, self.run, self.at.index.0)
    }
}

/// A run of text laid out this frame.
struct Run {
    part: u32,
    n: u32,
    /// Line breaks before it: 0 on the line of the run before, 1 on the next, 2 after a gap.
    breaks: u8,
    galley: Arc<Galley>,
    pos: Pos2,
    color: Color32,
    underline: Stroke,
    painter: egui::Painter,
    slot: egui::layers::ShapeIdx,
    response: Response,
    link: bool,
}

impl Run {
    fn len(&self) -> usize {
        self.galley.end().index.0
    }

    /// Its rows on the screen, from their leftmost letter to their rightmost.
    fn rows(&self) -> impl Iterator<Item = Rect> + '_ {
        self.galley.rows.iter().map(|r| {
            let mut rect = r.rect_without_leading_space();
            // egui's leading space runs to a row's first letter, which in a right-to-left row is on
            // its right: the row was taken for that letter alone, and a press left of it went to
            // that letter (a drag across a line of Hebrew copied one letter, upstream's too).
            let glyphs = &r.row.glyphs;
            let first = glyphs.first().map(|g| g.pos.x);
            let left = glyphs.iter().map(|g| g.pos.x).fold(f32::INFINITY, f32::min);
            if r.row.rtl_paragraph || first.is_some_and(|f| left < f) {
                let right = glyphs.iter().map(|g| g.pos.x + g.advance_width).fold(f32::NEG_INFINITY, f32::max);
                if left.is_finite() && right.is_finite() && right >= left {
                    rect.min.x = r.pos.x + left;
                    rect.max.x = r.pos.x + right;
                }
            }
            rect.translate(self.pos.to_vec2())
        })
    }

    /// Where `c` is, on the screen.
    fn caret(&self, c: CCursor) -> Rect {
        self.galley.pos_from_cursor(c).translate(self.pos.to_vec2())
    }
}

/// One text's selection, and its runs this frame.
#[derive(Default)]
pub(crate) struct Doc {
    anchor: Option<Mark>,
    head: Option<Mark>,
    /// A drag from a press in the text is under way.
    dragging: bool,
    /// It has the keyboard.
    active: bool,
    /// Where on the screen Shift+Up and Shift+Down keep to.
    h_x: Option<f32>,
    /// Where the text was last frame (a press there starts a selection), and the pointer.
    rect: Option<Rect>,
    pointer: Option<Pos2>,
    /// It's in a message's own window.
    window: bool,
    /// The pass it was last drawn in.
    pass: u64,
    part: u32,
    n: u32,
    breaks: u8,
    top: f32,
    runs: Vec<Run>,
    /// What's under the runs, between and beside them.
    under: Option<Response>,
}

/// The selections of the texts shown, and the one being drawn.
#[derive(Default)]
pub(crate) struct TextSelection {
    docs: HashMap<egui::Id, Doc>,
    cur: Option<(egui::Id, Doc)>,
    /// The main window's message text had the keyboard last frame: its keys aren't the list's.
    pub(crate) keys: bool,
}

impl TextSelection {
    /// Whether a text shown is selected whole (Ctrl+A).
    #[cfg(test)]
    pub(crate) fn all(&self) -> bool {
        self.docs.values().any(Doc::all)
    }
}

impl Doc {
    /// Its two ends, in reading order.
    fn selection(&self) -> Option<(Mark, Mark)> {
        let (a, h) = (self.anchor?, self.head?);
        (a != h).then(|| if a.key() <= h.key() { (a, h) } else { (h, a) })
    }

    fn all(&self) -> bool {
        self.selection().is_some_and(|(lo, hi)| lo == Mark::START && hi == Mark::END)
    }

    fn mark(&self, i: usize, at: CCursor) -> Option<Mark> {
        self.runs.get(i).map(|r| Mark { part: r.part, run: r.n, at })
    }

    /// The letters of run `i` selected.
    fn range_of(&self, i: usize) -> Option<Range<usize>> {
        let (lo, hi) = self.selection()?;
        let r = self.runs.get(i)?;
        let k = (r.part, r.n);
        if k < lo.run_key() || k > hi.run_key() {
            return None;
        }
        let len = r.len();
        let end = if k == hi.run_key() { hi.at.index.0.min(len) } else { len };
        let start = if k == lo.run_key() { lo.at.index.0.min(end) } else { 0 };
        Some(start..end)
    }

    /// What's selected, in reading order; `whole` when all of it is (a long text, not all laid out).
    fn text(&self, whole: Option<&str>) -> String {
        if let Some(w) = whole.filter(|_| self.all()) {
            return w.to_string();
        }
        let mut out = String::new();
        let mut first = true;
        for (i, r) in self.runs.iter().enumerate() {
            let Some(range) = self.range_of(i) else { continue };
            if !first {
                (0..r.breaks).for_each(|_| out.push('\n'));
            }
            first = false;
            out.push_str(slice_char_range(r.galley.text(), CharIndex(range.start)..CharIndex(range.end)));
        }
        out
    }

    /// Run `i` selected (`range`) to its end, and the next beside it on the row selected from its
    /// start: where, in the last row of run `i` (its own coordinates), its shading ends to meet
    /// the next run's. Each run shades its own letters, and drawn at their places rounded to
    /// pixels (as text is) two runs' shadings overlap a little or leave a hairline between them:
    /// a darker or a lighter line down the selection, the shading being translucent.
    fn seam(&self, i: usize, range: &Range<usize>, pixels_per_point: f32) -> Option<f32> {
        let r = self.runs.get(i)?;
        let j = i.checked_add(1)?;
        let next = self.runs.get(j)?;
        if range.end < r.len() || next.breaks > 0 || self.range_of(j).is_none_or(|n| n.is_empty() || n.start > 0) {
            return None;
        }
        let last = r.galley.rows.last()?;
        // Side by side on a row: not a table's next cell, nor a run wrapped onto the next row.
        let (a, b) = (r.rows().last()?, next.rows().next()?);
        if last.row.rtl_paragraph || a.y_range().intersection(b.y_range()).span() <= 0.0 || (b.left() - a.right()).abs() > MAX_SEAM {
            return None;
        }
        let drawn = |r: &Run| r.pos.round_to_pixels(pixels_per_point);
        let starts = drawn(next).x + next.galley.pos_from_cursor(CCursor::default()).min.x;
        Some(starts - drawn(r).x - last.pos.x)
    }
}

/// How far a selection drag scrolls this frame with the pointer at `y`, at or past the top or
/// bottom of `view`; faster the farther it goes. A positive delta moves the content down.
/// (PdfCraft's, for its pages, with its band of the view's edge narrowed to the last pixels.)
fn edge_delta(y: f32, view: Rect, dt: f32) -> f32 {
    if !y.is_finite() || !dt.is_finite() || view.height() < 4.0 * EDGE {
        return 0.0;
    }
    let distance = if y > view.bottom() - EDGE {
        y - (view.bottom() - EDGE)
    } else if y < view.top() + EDGE {
        y - (view.top() + EDGE)
    } else {
        return 0.0;
    };
    -distance.clamp(-1_000_000.0, 1_000_000.0) * EDGE_SPEED * dt.clamp(0.0, 0.05)
}

/// The text shown under `id` starts here; `window`: in a message's own window.
pub(crate) fn begin(st: &mut UiState, ui: &mut Ui, id: egui::Id, window: bool) {
    let ts = &mut st.text_selection;
    if let Some((old, d)) = ts.cur.take() {
        ts.docs.insert(old, d);
    }
    let kept = ts.docs.remove(&id);
    let seen = kept.is_some();
    let mut d = kept.unwrap_or_default();
    // Not shown last pass (another message was, or none): it's shown again unselected and
    // without the keyboard, as Outlook does when the list goes back to it; a message's own
    // window has nothing else to give the keys to.
    if !seen || d.pass.saturating_add(1) < ui.ctx().cumulative_pass_nr() {
        (d.anchor, d.head, d.dragging, d.h_x) = (None, None, false, None);
        d.active = window;
    }
    d.window = window;
    d.runs.clear();
    (d.part, d.n, d.breaks) = (0, 0, 0);
    d.top = ui.cursor().top();
    // Under the text (its runs are on top): a press or drag between lines is the text's, not a
    // drag of the scroll area. Not on a touch screen, where a finger's drag scrolls (as over
    // egui's labels there).
    let touch = ui.input(|i| i.has_touch_screen());
    d.under = d.rect.filter(|_| !touch).map(|r| ui.interact(r, id.with("text-under"), Sense::drag()));
    ts.cur = Some((id, d));
}

/// The runs that follow are part `i` of a long text.
pub(crate) fn part(st: &mut UiState, i: usize) {
    if let Some((_, d)) = &mut st.text_selection.cur {
        d.part = u32::try_from(i).unwrap_or(u32::MAX - 1);
        d.n = 0;
    }
}

/// The next run starts a line.
pub(crate) fn line(st: &mut UiState) {
    if let Some((_, d)) = &mut st.text_selection.cur {
        d.breaks = d.breaks.max(1);
    }
}

/// An empty line: the next run starts a paragraph.
pub(crate) fn blank(st: &mut UiState) {
    if let Some((_, d)) = &mut st.text_selection.cur {
        d.breaks = d.breaks.max(1).saturating_add(1).min(MAX_BREAKS);
    }
}

/// The parts of a long text of `n` parts the selection's ends are in, and those it spans.
pub(crate) fn parts(st: &UiState, n: usize) -> (Vec<usize>, Range<usize>, bool) {
    let Some((_, d)) = &st.text_selection.cur else { return (Vec::new(), 0..0, false) };
    let last = n.saturating_sub(1);
    let part = |m: Mark| usize::try_from(m.part).map_or(last, |p| p.min(last));
    let ends: Vec<usize> = [d.anchor, d.head].into_iter().flatten().map(part).collect();
    let span = d.selection().map_or(0..0, |(lo, hi)| part(lo)..part(hi).saturating_add(1));
    (ends, span, d.all())
}

/// A run of a message's text: laid out as a label, drawn with the selection. `link`: the colour
/// of a link's underline, shown on hover (a click on it is the caller's).
pub(crate) fn run(st: &mut UiState, ui: &mut Ui, text: impl Into<egui::WidgetText>, link: Option<Color32>) -> Response {
    // Not a message's text (a signature's preview, the original under a reply), or laid out
    // unseen to be measured: a label or a link, as it was.
    if st.text_selection.cur.is_none() || !ui.is_visible() {
        return if link.is_some() { ui.add(egui::Link::new(text)) } else { ui.label(text) };
    }
    let mut label = egui::Label::new(text).selectable(true);
    if link.is_some() {
        label = label.sense(Sense::click());
    }
    let (pos, galley, response) = label.layout_in_ui(ui);
    let role = if link.is_some() { egui::Role::Link } else { egui::Role::Label };
    response.widget_info(|| egui::WidgetInfo::labeled(role, ui.is_enabled(), galley.text()));
    let hovered = response.hovered();
    let underline = link.filter(|_| hovered).map_or(Stroke::NONE, |c| Stroke::new(1.0, c));
    if link.is_some() && hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let color = ui.visuals().text_color();
    let Some((_, d)) = st.text_selection.cur.as_mut() else { return response };
    let painter = ui.painter().clone();
    let slot = painter.add(egui::Shape::Noop);
    let breaks = std::mem::take(&mut d.breaks);
    d.runs.push(Run { part: d.part, n: d.n, breaks, galley, pos, color, underline, painter, slot, response: response.clone(), link: link.is_some() });
    d.n = d.n.saturating_add(1);
    response
}

/// The run just drawn has a link under the pointer (an address in plain text): the pointer is a
/// hand there, not the text's.
pub(crate) fn pointing(st: &mut UiState) {
    if let Some((_, d)) = &mut st.text_selection.cur
        && let Some(r) = d.runs.last_mut()
    {
        r.link = true;
    }
}

/// A run that may read right to left: wrapped at the width there is, a right-to-left paragraph
/// against the right edge, where it starts (as [`crate::bidi::add_label`]).
pub(crate) fn rtl_run(st: &mut UiState, ui: &mut Ui, mut job: LayoutJob) -> Response {
    job.wrap.max_width = ui.available_width();
    if crate::bidi::has_rtl(&job.text) && crate::bidi::paragraph_rtl(&job.text) {
        ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| run(st, ui, job, None)).inner
    } else {
        run(st, ui, job, None)
    }
}

/// The text started with [`begin`] ends here: the pointer and keys, the copy, and the runs drawn
/// with what's selected. `whole` is what a copy of all of it takes, when not all of it is laid out.
pub(crate) fn finish(st: &mut UiState, ui: &mut Ui, whole: Option<&str>) {
    let Some((id, mut d)) = st.text_selection.cur.take() else { return };
    let ctx = ui.ctx().clone();
    let left = ui.max_rect().left();
    let rect = Rect::from_min_max(pos2(left, d.top), pos2(ui.max_rect().right().max(left), ui.min_rect().bottom().max(d.top)));
    d.rect = Some(rect);
    let clip = ui.clip_rect();
    let layer = ui.layer_id();
    d.pass = ctx.cumulative_pass_nr();
    let (touch, pressed, down, shift, latest, origin, dt, double, triple) = ctx.input(|i| {
        let p = &i.pointer;
        let b = egui::PointerButton::Primary;
        (
            i.has_touch_screen(),
            p.primary_pressed(),
            p.primary_down(),
            i.modifiers.shift,
            p.latest_pos(),
            p.press_origin(),
            i.stable_dt,
            p.button_double_clicked(b),
            p.button_triple_clicked(b),
        )
    });
    if latest.is_some() {
        d.pointer = latest;
    }
    let over = |p: Pos2| rect.intersect(clip).contains(p) && ctx.layer_id_at(p).is_none_or(|l| l == layer);
    // A press in the text (on a run or between them): a selection starts there (or, with Shift,
    // goes on to there). On the scroll bar (in the margin beside the text, or over its edge) or a
    // picture in it, the selection stays as it is.
    let ours = d.runs.iter().map(|r| &r.response).chain(&d.under).any(Response::is_pointer_button_down_on);
    let beside = |p: Pos2| clip.contains(p) && !rect.x_range().contains(p.x);
    // Where it was pressed, not where the pointer went on to in the same frame (a quick drag).
    if pressed && let Some(p) = origin.or(d.pointer) {
        if over(p) && ours {
            d.active = true;
            d.h_x = None;
            if let Some((i, c)) = d.nearest(0..d.runs.len(), p, |_| true) {
                let m = d.mark(i, c);
                if !(shift && d.anchor.is_some()) {
                    d.anchor = m;
                }
                d.head = m;
                // A finger's drag scrolls the text instead (the scroll area has it).
                d.dragging = !touch;
            }
        } else if !over(p) && !beside(p) && ctx.layer_id_at(p).is_none_or(|l| l.order == egui::Order::Background) {
            // Elsewhere in the window (the message list, the folders): the keyboard is theirs.
            d.active = false;
            d.dragging = false;
            d.anchor = None;
            d.head = None;
        }
    }
    if d.dragging {
        if !down {
            d.dragging = false;
        } else if let Some(p) = d.pointer {
            // At or past the view's top or bottom, it scrolls; the end follows the text that
            // comes under the pointer, kept in the view.
            let dy = edge_delta(p.y, clip, dt);
            if dy != 0.0 {
                ui.scroll_with_delta_animation(vec2(0.0, dy), egui::style::ScrollAnimation::none());
                ctx.request_repaint();
            }
            let view = rect.intersect(clip);
            let q = if view.is_positive() { view.clamp(p) } else { p };
            if let Some((i, c)) = d.nearest(0..d.runs.len(), q, |_| true) {
                d.head = d.mark(i, c);
            }
        }
    }
    if (double || triple)
        && let Some(p) = d.pointer.filter(|p| over(*p))
        && let Some((i, c)) = d.nearest(0..d.runs.len(), p, |_| true)
    {
        d.select_unit(i, c, triple);
    }
    // A message's own window (a viewport of its own, its keys its own): the text has them,
    // clicked into or not, the toolbar pressed or not.
    if d.window && ctx.viewport_id() != egui::ViewportId::ROOT {
        d.active = true;
    }
    if ctx.text_edit_focused() {
        d.active = false;
    }
    if d.active {
        keys(&mut d, ui, rect, whole);
        if ctx.viewport_id() == egui::ViewportId::ROOT {
            st.text_selection.keys = true;
        }
    }
    // The text's pointer over it (a hand over a link, an address in plain text too), and while a
    // selection is dragged.
    if d.dragging || d.runs.iter().any(|r| !r.link && r.response.hovered()) {
        ctx.set_cursor_icon(egui::CursorIcon::Text);
    }
    paint(&d, ui);
    let ts = &mut st.text_selection;
    if ts.docs.len() >= KEPT {
        ts.docs.retain(|_, v| v.active);
    }
    d.runs.clear();
    ts.docs.insert(id, d);
}

/// The runs, with what's selected of them shaded, a run's shading meeting the next's on a row.
fn paint(d: &Doc, ui: &Ui) {
    let bg = ui.visuals().selection.bg_fill;
    for (i, r) in d.runs.iter().enumerate() {
        if !r.painter.clip_rect().intersects(r.response.rect) {
            continue;
        }
        let mut galley = r.galley.clone();
        if let Some(range) = d.range_of(i).filter(|g| !g.is_empty()) {
            let sel = CCursorRange::two(CCursor::new(range.start), CCursor::new(range.end));
            egui::text_selection::visuals::paint_text_selection(&mut galley, ui.visuals(), &sel, None);
            if let Some(x) = d.seam(i, &range, ui.ctx().pixels_per_point()) {
                meet(&mut galley, x, bg);
            }
        }
        r.painter.set(r.slot, egui::epaint::TextShape::new(r.pos, galley, r.color).with_underline(r.underline));
    }
}

/// The shading of `galley`'s last row (its rectangle's right edge) moved to end at `x`.
fn meet(galley: &mut Arc<Galley>, x: f32, bg: Color32) {
    let Some(placed) = Arc::make_mut(galley).rows.last_mut() else { return };
    let visuals = &mut Arc::make_mut(&mut placed.row).visuals;
    let shaded = |v: &&mut egui::epaint::Vertex| v.color == bg;
    let Some(right) = visuals.mesh.vertices.iter_mut().filter(shaded).map(|v| v.pos.x).reduce(f32::max) else { return };
    if (x - right).abs() > MAX_SEAM {
        return;
    }
    for v in visuals.mesh.vertices.iter_mut().filter(shaded).filter(|v| v.pos.x >= right) {
        v.pos.x = x;
    }
    visuals.mesh_bounds = visuals.mesh.calc_bounds();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drag_scrolls_toward_the_edge_it_is_at_faster_the_farther_out() {
        let view = Rect::from_min_max(pos2(0.0, 100.0), pos2(400.0, 500.0));
        assert_eq!(edge_delta(300.0, view, 0.016), 0.0, "nothing away from the edges");
        // On the text by the edges (its last line in view), it stays: it ran away under the pointer.
        assert_eq!(edge_delta(495.0, view, 0.016), 0.0, "nothing on the last line in view");
        assert_eq!(edge_delta(105.0, view, 0.016), 0.0, "or the first");
        assert!(edge_delta(499.5, view, 0.016) < 0.0, "the bottom edge moves the content up");
        assert!(edge_delta(100.5, view, 0.016) > 0.0, "the top edge moves it down");
        assert!(edge_delta(600.0, view, 0.016) < edge_delta(510.0, view, 0.016), "faster the farther past the view");
        assert!(edge_delta(0.0, view, 0.016) > edge_delta(90.0, view, 0.016), "and above it");
        for junk in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(edge_delta(junk, view, 0.016), 0.0);
            assert_eq!(edge_delta(600.0, view, junk), 0.0);
        }
        assert!(edge_delta(f32::MAX, view, 0.016).is_finite());
        assert!(edge_delta(600.0, view, 100.0).abs() <= edge_delta(600.0, view, 0.05).abs(), "a stalled frame doesn't jump");
        let small = Rect::from_min_max(pos2(0.0, 0.0), pos2(400.0, 6.0));
        assert_eq!(edge_delta(5.5, small, 0.016), 0.0, "a view too short to have edges");
    }
}
