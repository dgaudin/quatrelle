// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A selection that goes across the text being written and the blocks of the original under
//! it, as Outlook's does: each is a text box with a selection of its own, so a drag from one
//! stopped at its end. This holds one selection over them, from a place in one block to a place
//! in another ([`QuotePoint`]): the block it started in keeps the keyboard and shows its own part,
//! the others are lit here. A drag, Shift with a click or the arrows, Ctrl+A; Delete, Backspace,
//! a cut or typing take out what it holds, whole blocks with it ([`Compose::delete_across`]);
//! Ctrl+Z puts it all back in one step, Ctrl+Y takes it out again.

use egui::Key;
use egui::text::CCursor;
use quatrelle_engine::{Compose, QuotePoint, Written};

use super::quote::cell_box;
use super::*;
use crate::reading::body::text_selection::edge_delta;

/// Most steps kept to undo.
const STEPS: usize = 100;

/// The selection: from where it started to where its moving end is.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Span {
    pub(crate) anchor: QuotePoint,
    pub(crate) head: QuotePoint,
    /// The pointer is still held.
    dragging: bool,
    /// Where Shift+Up and Shift+Down keep to, across.
    h_x: Option<f32>,
    /// Bring its moving end into view (it was moved with the keys).
    reveal: bool,
}

impl Span {
    fn new(anchor: QuotePoint, head: QuotePoint) -> Span {
        Span { anchor, head, dragging: false, h_x: None, reveal: false }
    }

    /// Whether it holds more than one block (else the block's own selection has it).
    pub(crate) fn spans(&self) -> bool {
        self.anchor.block != self.head.block
    }
}

/// What a selection across blocks took out: the message before and after, what was selected and
/// where the cursor was left.
struct Step {
    before: Written,
    after: Written,
    sel: (QuotePoint, QuotePoint),
    at: QuotePoint,
}

/// The selections across blocks of the messages being written, each message its own: several
/// are drawn in one frame (the one in the main window, each one in a window of its own), and one
/// state for them all was emptied by each in turn, so no drag ever left its block while another
/// message was open.
#[derive(Default)]
pub(crate) struct Across(std::collections::HashMap<u64, AcrossState>);

impl Across {
    fn take(&mut self, compose: u64) -> AcrossState {
        self.0.remove(&compose).unwrap_or_default()
    }

    fn put(&mut self, compose: u64, a: AcrossState) {
        self.0.insert(compose, a);
    }

    /// Those of messages no longer written go.
    pub(crate) fn keep(&mut self, live: &std::collections::HashSet<u64>) {
        self.0.retain(|id, _| live.contains(id));
    }

    #[cfg(test)]
    pub(crate) fn get(&self, compose: u64) -> Option<&AcrossState> {
        self.0.get(&compose)
    }

    #[cfg(test)]
    pub(crate) fn of(&mut self, compose: u64) -> &mut AcrossState {
        self.0.entry(compose).or_default()
    }
}

/// `QUATRELLE_TRACE_SELECTION=1`: what a press, a drag leaving its block and a release in a
/// message being written were taken for, in `quatrelle.log`: a selection that doesn't go on into
/// the next block on someone's machine can be followed there.
fn trace(say: impl FnOnce() -> String) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("QUATRELLE_TRACE_SELECTION").is_some_and(|v| !v.is_empty())) {
        log::info!("selection: {}", say());
    }
}

/// The selection across the blocks of one message being written, between frames.
#[derive(Default)]
pub(crate) struct AcrossState {
    pub(crate) sel: Option<Span>,
    /// The block a press landed in, while the pointer is held.
    pub(crate) pressed: Option<usize>,
    /// The block with the keyboard when the frame began.
    focus: Option<usize>,
    /// Each block as last drawn.
    pub(crate) drawn: Vec<(usize, rich_edit::Drawn)>,
    undo: Vec<Step>,
    redo: Vec<Step>,
}

impl AcrossState {
    fn drawn_of(&self, block: usize) -> Option<&rich_edit::Drawn> {
        self.drawn.iter().find(|(k, _)| *k == block).map(|(_, d)| d)
    }

    /// Where character `at` of block `block` was last drawn.
    #[cfg(test)]
    pub(crate) fn place(&self, p: QuotePoint) -> Option<egui::Pos2> {
        let d = self.drawn_of(p.block)?;
        Some(d.galley.pos_from_cursor(CCursor::new(p.at)).translate(d.pos.to_vec2()).center())
    }

    /// The block, and the letter in it, nearest `p`: the block nearest up or down, then across.
    fn nearest(&self, p: egui::Pos2) -> Option<QuotePoint> {
        let gap = |v: f32, r: egui::Rangef| if v < r.min { r.min - v } else { (v - r.max).max(0.0) };
        let mut best: Option<((f32, f32, f32), usize)> = None;
        for (k, d) in &self.drawn {
            // Of two that hold it (one drawn over another), the smaller.
            let score = (gap(p.y, d.rect.y_range()), gap(p.x, d.rect.x_range()), d.rect.area());
            if best.is_none_or(|(b, _)| score < b) {
                best = Some((score, *k));
            }
        }
        let (_, block) = best?;
        let d = self.drawn_of(block)?;
        Some(QuotePoint { block, at: d.galley.cursor_from_pos(p - d.pos).index.0 })
    }
}

/// The text box of block `block` of `c`.
fn box_of(c: &Compose, block: usize) -> egui::Id {
    match block.checked_sub(1) {
        None => rich_edit::body_id(c),
        Some(n) => cell_box(c.id, n),
    }
}

fn len_of(c: &Compose, block: usize) -> usize {
    c.block_text(block).map_or(0, |(t, _)| t.chars().count())
}

/// Nothing stays selected in the blocks other than `keep`: a text box without the keyboard still
/// shows the selection its editor remembers (for a menu or the ribbon to work on), which stayed
/// lit under a new selection made elsewhere.
fn clear_others(c: &Compose, ed: &mut rich_edit::Editor, eds: &mut QuoteEditors, keep: Option<usize>) {
    if keep != Some(0) {
        ed.sel = ed.sel.end..ed.sel.end;
    }
    for ((id, n), e) in eds.iter_mut() {
        if *id == c.id && keep != Some(n.saturating_add(1)) {
            e.sel = e.sel.end..e.sel.end;
        }
    }
}

/// The cursor at `at`, alone, its block with the keyboard.
fn put_cursor(st: &mut UiState, ui: &Ui, c: &Compose, ed: &mut rich_edit::Editor, eds: &mut QuoteEditors, at: QuotePoint) {
    let id = box_of(c, at.block);
    ui.memory_mut(|m| m.request_focus(id));
    rich_edit::set_caret(ui.ctx(), id, at.at, at.at);
    clear_others(c, ed, eds, Some(at.block));
    match at.block.checked_sub(1) {
        None => {
            ed.settle(at.at);
            st.quote_focus = None;
        }
        Some(n) => {
            if let Some(e) = eds.get_mut(&(c.id, n)) {
                e.settle(at.at);
            }
            st.quote_focus = Some((c.id, n));
        }
    }
    ui.ctx().request_repaint();
}

/// What a key asks of the selection.
enum Ask {
    Copy,
    Cut,
    /// Take it out (what is typed then goes in its place).
    Delete,
    All,
    Undo,
    Redo,
    /// Shift with an arrow, Home or End (`true`: with Ctrl, to the very start or end).
    Move(Key, bool),
    /// An arrow alone: the cursor to its start (`false`) or its end.
    Collapse(bool),
    Escape,
}

fn is_move(k: Key) -> bool {
    matches!(k, Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown | Key::Home | Key::End | Key::PageUp | Key::PageDown)
}

fn forward(k: Key) -> bool {
    matches!(k, Key::ArrowRight | Key::ArrowDown | Key::End | Key::PageDown)
}

/// The selection's keys, taken before the text boxes see them: with a selection across blocks,
/// and where one starts (Ctrl+A, Shift with an arrow at a block's edge).
pub(super) fn before(st: &mut UiState, ui: &mut Ui, c: &mut Compose, ed: &mut rich_edit::Editor, eds: &mut QuoteEditors) {
    let mut a = st.across.take(c.id);
    keys(&mut a, st, ui, c, ed, eds);
    // The block it started in shows its own part, to its end on the side the selection goes.
    if let Some(s) = a.sel.as_ref().filter(|s| s.spans()) {
        let to = if s.head > s.anchor { len_of(c, s.anchor.block) } else { 0 };
        rich_edit::set_caret(ui.ctx(), box_of(c, s.anchor.block), s.anchor.at, to);
    }
    st.across.put(c.id, a);
}

fn keys(a: &mut AcrossState, st: &mut UiState, ui: &mut Ui, c: &mut Compose, ed: &mut rich_edit::Editor, eds: &mut QuoteEditors) {
    let blocks = c.selectable_blocks();
    if a.sel.as_ref().is_some_and(|s| !blocks.contains(&s.anchor.block) || !blocks.contains(&s.head.block)) {
        a.sel = None;
    }
    a.focus = blocks.iter().copied().find(|k| ui.memory(|m| m.has_focus(box_of(c, *k))));
    let Some(focus) = a.focus else { return };
    if c.sending {
        a.sel = None;
        return;
    }
    // With nothing left of the original there's nothing to select across; what was taken out
    // can still be put back.
    let alone = blocks.len() < 2;
    if alone {
        a.sel = None;
    }
    let spans = a.sel.as_ref().is_some_and(Span::spans);
    let caret = rich_edit::caret(ui.ctx(), box_of(c, focus));
    let (first, last) = (blocks.first().copied().unwrap_or(0), blocks.last().copied().unwrap_or(0));
    // Whether Shift with this key leaves the block with the keyboard: its cursor is at the edge.
    let leaves = |k: Key, ctrl: bool| {
        let Some((_, at)) = caret else { return false };
        match k {
            Key::Home | Key::End => ctrl && focus != if k == Key::End { last } else { first },
            Key::ArrowRight | Key::ArrowDown | Key::PageDown => at >= len_of(c, focus) && focus != last,
            Key::ArrowLeft | Key::ArrowUp | Key::PageUp => at == 0 && focus != first,
            _ => false,
        }
    };
    // What was taken out is put back by Ctrl+Z while the message is as that left it (what was
    // typed since is undone first, by its own text box), and taken out again by Ctrl+Y while
    // it's as it was before: looked at only when one of those keys comes.
    let asked = ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { key: Key::Z | Key::Y, pressed: true, .. })));
    let can_undo = asked && a.undo.last().is_some_and(|s| c.is_written(&s.after));
    let can_redo = asked && a.redo.last().is_some_and(|s| c.is_written(&s.before));
    let mac = cfg!(target_os = "macos");
    let mut asks: Vec<Ask> = Vec::new();
    ui.input_mut(|i| {
        i.events.retain(|e| match e {
            egui::Event::Copy if spans => {
                asks.push(Ask::Copy);
                false
            }
            egui::Event::Cut if spans => {
                asks.push(Ask::Cut);
                false
            }
            egui::Event::Text(_) | egui::Event::Paste(_) | egui::Event::PasteImage(_) if spans => {
                asks.push(Ask::Delete);
                true
            }
            egui::Event::Key { key, pressed: true, modifiers, .. } => {
                let (k, m) = (*key, *modifiers);
                let ask = match k {
                    Key::Delete | Key::Backspace if spans => Some((Ask::Delete, false)),
                    Key::Enter if spans => Some((Ask::Delete, true)),
                    Key::Escape if spans => Some((Ask::Escape, true)),
                    Key::A if m.command && !m.shift && !m.alt && !alone => Some((Ask::All, false)),
                    Key::Z if m.command && m.shift && mac && can_redo => Some((Ask::Redo, false)),
                    Key::Y if m.command && can_redo => Some((Ask::Redo, false)),
                    Key::Z if m.command && !m.shift && can_undo => Some((Ask::Undo, false)),
                    k if is_move(k) && m.shift && !alone && (spans || leaves(k, m.command)) => Some((Ask::Move(k, m.command), false)),
                    k if is_move(k) && !m.shift && spans => Some((Ask::Collapse(forward(k)), false)),
                    _ => None,
                };
                match ask {
                    Some((ask, kept)) => {
                        asks.push(ask);
                        kept
                    }
                    None => true,
                }
            }
            _ => true,
        });
    });
    for ask in asks {
        let sel = a.sel.clone().filter(Span::spans);
        match (ask, sel) {
            (Ask::Copy, Some(s)) => ui.ctx().copy_text(c.text_across(s.anchor, s.head)),
            (Ask::Cut, Some(s)) => {
                ui.ctx().copy_text(c.text_across(s.anchor, s.head));
                delete(a, st, ui, c, ed, eds, &s);
            }
            (Ask::Delete, Some(s)) => delete(a, st, ui, c, ed, eds, &s),
            (Ask::All, _) => {
                a.sel = Some(Span::new(QuotePoint { block: first, at: 0 }, QuotePoint { block: last, at: len_of(c, last) }));
                ui.memory_mut(|m| m.request_focus(box_of(c, first)));
            }
            (Ask::Undo, _) => {
                let Some(step) = a.undo.pop() else { continue };
                c.set_written(step.before.clone());
                // As it was selected, the block it started in with the keyboard again.
                a.sel = Some(Span::new(step.sel.0, step.sel.1));
                ui.memory_mut(|m| m.request_focus(box_of(c, step.sel.0.block)));
                st.quote_focus = step.sel.0.block.checked_sub(1).map(|n| (c.id, n));
                a.redo.push(step);
            }
            (Ask::Redo, _) => {
                let Some(step) = a.redo.pop() else { continue };
                c.set_written(step.after.clone());
                a.sel = None;
                put_cursor(st, ui, c, ed, eds, step.at);
                a.undo.push(step);
            }
            (Ask::Move(k, ctrl), sel) => {
                // From the block's own selection, when it's only now leaving it.
                let from = sel.or_else(|| {
                    let (anchor, head) = rich_edit::caret(ui.ctx(), box_of(c, focus))?;
                    Some(Span::new(QuotePoint { block: focus, at: anchor }, QuotePoint { block: focus, at: head }))
                });
                let Some(mut s) = from else { continue };
                let (head, h_x) = moved(a, c, &blocks, s.head, s.h_x, k, ctrl);
                (s.head, s.h_x, s.reveal) = (head, h_x, true);
                if s.spans() {
                    a.sel = Some(s);
                } else {
                    // Back in the block it started in: that block's own selection again.
                    a.sel = None;
                    let id = box_of(c, s.anchor.block);
                    ui.memory_mut(|m| m.request_focus(id));
                    rich_edit::set_caret(ui.ctx(), id, s.anchor.at, s.head.at);
                }
            }
            (Ask::Collapse(end), Some(s)) => {
                a.sel = None;
                put_cursor(st, ui, c, ed, eds, if end { s.anchor.max(s.head) } else { s.anchor.min(s.head) });
            }
            (Ask::Escape, Some(s)) => {
                a.sel = None;
                put_cursor(st, ui, c, ed, eds, s.head);
            }
            _ => {}
        }
    }
}

/// Take out what `s` holds, kept as a step to undo; the cursor where its two ends meet.
fn delete(a: &mut AcrossState, st: &mut UiState, ui: &Ui, c: &mut Compose, ed: &mut rich_edit::Editor, eds: &mut QuoteEditors, s: &Span) {
    let before = c.written();
    a.sel = None;
    let Some(at) = c.delete_across(s.anchor, s.head) else { return };
    a.undo.push(Step { before, after: c.written(), sel: (s.anchor, s.head), at });
    if a.undo.len() > STEPS {
        a.undo.remove(0);
    }
    a.redo.clear();
    put_cursor(st, ui, c, ed, eds, at);
}

/// Where `head` goes with key `k` (`ctrl`: to the very start or end), and the place across
/// Shift+Up and Shift+Down keep to: within its block as that block is laid out, into the next
/// one past its edge.
fn moved(a: &AcrossState, c: &Compose, blocks: &[usize], head: QuotePoint, h_x: Option<f32>, k: Key, ctrl: bool) -> (QuotePoint, Option<f32>) {
    let i = blocks.iter().position(|b| *b == head.block).unwrap_or(0);
    let next = blocks.get(i.saturating_add(1)).copied();
    let prev = i.checked_sub(1).and_then(|p| blocks.get(p)).copied();
    let len = len_of(c, head.block);
    let at = |block: usize, at: usize| QuotePoint { block, at };
    let galley = |block: usize| a.drawn_of(block).map(|d| d.galley.clone());
    // Across, in the window: where the cursor is, unless a place is kept to.
    let x_of = |block: usize, n: usize| a.drawn_of(block).map(|d| d.galley.pos_from_cursor(CCursor::new(n)).center().x + d.pos.x);
    let x = h_x.or_else(|| x_of(head.block, head.at));
    let local = |block: usize| x.zip(a.drawn_of(block)).map(|(x, d)| x - d.pos.x);
    match k {
        Key::Home if ctrl => (at(blocks.first().copied().unwrap_or(0), 0), None),
        Key::End if ctrl => {
            let last = blocks.last().copied().unwrap_or(0);
            (at(last, len_of(c, last)), None)
        }
        Key::Home => (at(head.block, galley(head.block).map_or(0, |g| g.cursor_begin_of_paragraph(&CCursor::new(head.at)).index.0)), None),
        Key::End => (at(head.block, galley(head.block).map_or(len, |g| g.cursor_end_of_paragraph(&CCursor::new(head.at)).index.0)), None),
        Key::ArrowRight if head.at < len => (at(head.block, head.at + 1), None),
        Key::ArrowRight => (next.map_or(head, |n| at(n, 0)), None),
        Key::ArrowLeft if head.at > 0 => (at(head.block, head.at - 1), None),
        Key::ArrowLeft => (prev.map_or(head, |p| at(p, len_of(c, p))), None),
        Key::ArrowDown | Key::PageDown if head.at < len => {
            let to = galley(head.block).map_or(len, |g| g.cursor_down_one_row(&CCursor::new(head.at), local(head.block)).0.index.0);
            (at(head.block, to.min(len)), x)
        }
        Key::ArrowDown | Key::PageDown => match next {
            // Into the next block: its first row.
            Some(n) => {
                let to = galley(n).map_or(0, |g| g.cursor_down_one_row(&CCursor::new(0), local(n)).0.index.0);
                (at(n, to.min(len_of(c, n))), x)
            }
            None => (head, x),
        },
        Key::ArrowUp | Key::PageUp if head.at > 0 => {
            let to = galley(head.block).map_or(0, |g| g.cursor_up_one_row(&CCursor::new(head.at), local(head.block)).0.index.0);
            (at(head.block, to.min(len)), x)
        }
        Key::ArrowUp | Key::PageUp => match prev {
            Some(p) => {
                let end = len_of(c, p);
                let to = galley(p).map_or(end, |g| g.cursor_up_one_row(&CCursor::new(end), local(p)).0.index.0);
                (at(p, to.min(end)), x)
            }
            None => (head, x),
        },
        _ => (head, h_x),
    }
}

/// After the text and the original's blocks are drawn (`drawn`, each with its number): the
/// pointer's part (a drag that leaves its block, Shift with a click, a drag held at the view's
/// edge scrolling), and the selection lit in the blocks other than the one it started in.
pub(super) fn after(
    st: &mut UiState,
    ui: &mut Ui,
    c: &Compose,
    ed: &mut rich_edit::Editor,
    eds: &mut QuoteEditors,
    drawn: Vec<(usize, rich_edit::Drawn)>,
) {
    let mut a = st.across.take(c.id);
    a.drawn = drawn;
    if a.drawn.len() < 2 || c.plain {
        a.sel = None;
        a.pressed = None;
        st.across.put(c.id, a);
        return;
    }
    let ctx = ui.ctx().clone();
    let (clip, layer) = (ui.clip_rect(), ui.layer_id());
    let (pressed, down, shift, origin, latest, dt) = ui.input(|i| {
        (
            i.pointer.button_pressed(egui::PointerButton::Primary),
            i.pointer.button_down(egui::PointerButton::Primary),
            i.modifiers.shift,
            i.pointer.press_origin(),
            i.pointer.latest_pos(),
            i.stable_dt,
        )
    });
    let over = |p: egui::Pos2| clip.contains(p) && ctx.layer_id_at(p).is_none_or(|l| l == layer);
    if pressed && let Some(p) = origin.or(latest).filter(|p| over(*p)) {
        let hit = a.drawn.iter().find(|(_, d)| d.rect.contains(p)).map(|(k, _)| *k);
        // Shift with a click: on from where the selection started, in whichever block.
        let anchor = a.sel.as_ref().map(|s| s.anchor).or_else(|| {
            let block = a.focus?;
            Some(QuotePoint { block, at: rich_edit::caret(&ctx, box_of(c, block))?.0 })
        });
        match (hit, anchor.filter(|_| shift)) {
            (Some(k), Some(anchor)) if anchor.block != k || a.sel.is_some() => {
                if let Some(head) = a.nearest(p) {
                    a.sel = Some(Span { dragging: true, ..Span::new(anchor, head) });
                    // The click gave the block clicked the keyboard: back where it started.
                    ui.memory_mut(|m| m.request_focus(box_of(c, anchor.block)));
                }
                a.pressed = None;
            }
            (hit, _) => {
                // A press without Shift: the cursor there, and nothing else selected anywhere.
                a.sel = None;
                a.pressed = hit;
                clear_others(c, ed, eds, hit);
            }
        }
        trace(|| format!("press at {p:?} in block {hit:?}, shift {shift}: pressed {:?}, selection {:?}", a.pressed, a.sel));
    } else if pressed {
        trace(|| {
            format!(
                "press at {:?} not taken: view {clip:?}, layer there {:?}, ours {layer:?}",
                origin.or(latest),
                origin.or(latest).and_then(|p| ctx.layer_id_at(p))
            )
        });
    }
    // The text box egui itself is dragging a selection in is where it started, whatever became
    // of the press (one this didn't see, or took for another's).
    if down && a.pressed.is_none() && !a.sel.as_ref().is_some_and(|s| s.dragging) {
        a.pressed = a.drawn.iter().map(|(k, _)| *k).find(|k| ctx.is_being_dragged(box_of(c, *k)));
        if let Some(k) = a.pressed {
            a.sel = None;
            trace(|| format!("drag found in block {k} without its press"));
        }
    }
    if !down {
        if a.pressed.is_some() || a.sel.as_ref().is_some_and(|s| s.dragging) {
            trace(|| format!("release: selection {:?}", a.sel));
        }
        a.pressed = None;
        if let Some(s) = a.sel.as_mut().filter(|s| s.dragging) {
            s.dragging = false;
            if !s.spans() {
                rich_edit::set_caret(&ctx, box_of(c, s.anchor.block), s.anchor.at, s.head.at);
                a.sel = None;
            }
        }
    } else if let Some(p) = latest
        && (a.pressed.is_some() || a.sel.as_ref().is_some_and(|s| s.dragging))
    {
        // Held at or past the view's top or bottom, it scrolls.
        let dy = edge_delta(p.y, clip, dt);
        if dy != 0.0 {
            ui.scroll_with_delta_animation(vec2(0.0, dy), egui::style::ScrollAnimation::none());
            ctx.request_repaint();
        }
        let head = a.nearest(if clip.is_positive() { clip.clamp(p) } else { p });
        match (a.sel.as_mut().filter(|s| s.dragging), a.pressed, head) {
            (Some(s), _, Some(head)) => s.head = head,
            // The drag leaves the block it started in: from where that block's own selection
            // started.
            (None, Some(k), Some(head)) if head.block != k => {
                let caret = rich_edit::caret(&ctx, box_of(c, k));
                trace(|| format!("drag from block {k} (its selection {caret:?}) reaches {head:?} at {p:?}"));
                if let Some((at, _)) = caret {
                    a.sel = Some(Span { dragging: true, ..Span::new(QuotePoint { block: k, at }, head) });
                }
            }
            _ => {}
        }
    }
    if let Some(s) = a.sel.as_mut().filter(|s| s.spans()) {
        let (from, to) = (s.anchor.min(s.head), s.anchor.max(s.head));
        let color = ui.visuals().selection.bg_fill.gamma_multiply(0.45);
        for (k, d) in a.drawn.iter().filter(|(k, _)| *k != s.anchor.block && (from.block..=to.block).contains(k)) {
            let len = d.galley.text().chars().count();
            let range = if *k == from.block { from.at..len } else { 0..if *k == to.block { to.at.min(len) } else { len } };
            for r in rich_edit::selection_rects(&d.galley, range) {
                ui.painter().rect_filled(r.translate(d.pos.to_vec2()), 0.0, color);
            }
        }
        if std::mem::take(&mut s.reveal)
            && let Some((_, d)) = a.drawn.iter().find(|(k, _)| *k == s.head.block)
        {
            ui.scroll_to_rect(d.galley.pos_from_cursor(CCursor::new(s.head.at)).translate(d.pos.to_vec2()).expand(8.0), None);
        }
    }
    st.across.put(c.id, a);
}
