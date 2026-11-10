// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A note's text as it is written, in Markdown: a text box drawn from what [`markdown::read`]
//! finds (titles larger and bold, bold, italic, struck, code in a fixed face on a pale ground,
//! quotes, lists, links underlined), its marks kept visible but pale, with no preview to switch
//! to. A tick's box ticks with a click; a link opens with Ctrl+click, through the check every
//! link opened from a message goes through ([`Action::OpenUrl`]); a small bar writes the marks.
//! Enter goes on with a list, Tab and Shift+Tab take it in and out. The model counts bytes and
//! the text box characters: every position crosses over on a character's boundary.

use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;

use egui::text::{CCursor, LayoutJob, TextFormat};
use egui::{Color32, FontId, Galley, Key, Modifiers, Rect, RichText, Sense, Stroke, Ui, vec2};
use quatrelle_engine::model::pim::markdown::{self, Block, Kind, Mark, Span};

use super::INK;
use crate::icons::{self, Icon};
use crate::rich_edit::{caret, set_caret};
use crate::{Action, theme, widgets};

/// Links, in a blue that reads on every note's paper.
const LINK: Color32 = Color32::from_rgb(0x0B, 0x57, 0xB0);
/// Most tick boxes made clickable in one frame (those in view).
const MAX_TICKS: usize = 256;
/// Most characters of a tick's words kept as its name for a screen reader.
const NAME_CHARS: usize = 80;

/// How a note's text is drawn: its size (14 points in its window, 13 on a tile) and its ink.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Look {
    pub size: f32,
    pub ink: Color32,
}

impl Look {
    pub(crate) const WINDOW: Look = Look { size: 14.0, ink: INK };
    pub(crate) const TILE: Look = Look { size: 13.0, ink: INK };

    fn pale(self) -> Color32 {
        self.ink.gamma_multiply(0.38)
    }
}

/// A line's own look, by what it is: its words start from it.
fn line_format(block: Block, look: Look) -> TextFormat {
    let size = look.size;
    let (font_id, color, italics) = match block {
        Block::Heading(1) => (theme::bold(size * 1.45), look.ink, false),
        Block::Heading(2) => (theme::bold(size * 1.25), look.ink, false),
        Block::Heading(_) => (theme::bold(size * 1.1), look.ink, false),
        Block::Code | Block::Fence => (FontId::monospace(size * 0.92), look.ink, false),
        Block::Quote => (theme::regular(size), look.ink.gamma_multiply(0.72), true),
        Block::Tick { done: true, .. } => (theme::regular(size), look.ink.gamma_multiply(0.55), false),
        _ => (theme::regular(size), look.ink, false),
    };
    TextFormat { font_id, color, italics, ..Default::default() }
}

/// A segment's look on its line (`address`: a link's address, after its `](`).
fn span_format(block: Block, span: &Span, address: bool, look: Look) -> TextFormat {
    let mut f = line_format(block, look);
    match span.kind {
        Kind::Mark => {
            f.color = look.pale();
            f.italics = false;
            // `---` a line across.
            if block == Block::Rule {
                f.strikethrough = Stroke::new(1.0, look.pale());
            }
        }
        Kind::Code => {
            let size = if block == Block::Code { f.font_id.size } else { f.font_id.size * 0.92 };
            f.font_id = FontId::monospace(size);
            f.background = Color32::from_black_alpha(20);
        }
        Kind::Tick => {
            f.font_id = FontId::monospace(f.font_id.size);
            f.color = look.ink;
        }
        Kind::Url if address => {
            f.color = look.pale();
            f.underline = Stroke::new(1.0, look.pale());
        }
        Kind::LinkText | Kind::Url => {
            f.color = LINK;
            f.underline = Stroke::new(1.0, LINK);
        }
        Kind::Text => {}
    }
    if matches!(span.kind, Kind::Text | Kind::LinkText) {
        if span.style.bold && !matches!(block, Block::Heading(_)) {
            f.font_id = theme::bold(f.font_id.size);
        }
        f.italics |= span.style.italic;
        if span.style.strike {
            f.strikethrough = Stroke::new(1.0, f.color);
        }
    }
    f
}

/// The text laid out as its Markdown says, `wrap` points wide: the same text byte for byte (a
/// text box needs it so), only its look changes.
pub(crate) fn job(text: &str, wrap: f32, look: Look) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    let mut at = 0;
    let mut last = line_format(Block::Plain, look);
    for line in markdown::read(text) {
        // The line break before it, in the look of the line it ends.
        push(&mut job, text, at..line.range.start, &last);
        at = at.max(line.range.start);
        let own = line_format(line.block, look);
        for (i, s) in line.spans.iter().enumerate() {
            push(&mut job, text, at..s.range.start, &own);
            let address = s.kind == Kind::Url && i > 0 && line.spans.get(i - 1).is_some_and(|p| p.kind == Kind::Mark);
            push(&mut job, text, s.range.start.max(at)..s.range.end, &span_format(line.block, s, address, look));
            at = at.max(s.range.end);
        }
        push(&mut job, text, at..line.range.end, &own);
        at = at.max(line.range.end);
        last = own;
    }
    push(&mut job, text, at..text.len(), &last);
    job
}

/// `text`'s bytes `r` added in `format` (nothing when empty or off the text).
fn push(job: &mut LayoutJob, text: &str, r: Range<usize>, format: &TextFormat) {
    if r.start < r.end
        && let Some(words) = text.get(r)
    {
        job.append(words, 0.0, format.clone());
    }
}

/// A note's title as its window and tile say it: its first line's words, without their marks.
pub(crate) fn title(text: &str) -> String {
    let first = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let first = first.get(..char_floor(first, 1024)).unwrap_or("");
    let words = words_of(first, markdown::read(first).first().map(|l| l.spans.as_slice()).unwrap_or(&[]));
    words.trim().chars().take(NAME_CHARS).collect()
}

/// A line's words: its segments but its marks and a link's address.
fn words_of(text: &str, spans: &[Span]) -> String {
    let mut out = String::new();
    for (i, s) in spans.iter().enumerate() {
        let address = s.kind == Kind::Url && i > 0 && spans.get(i - 1).is_some_and(|p| p.kind == Kind::Mark);
        if matches!(s.kind, Kind::Text | Kind::Code | Kind::LinkText | Kind::Url) && !address {
            out.push_str(text.get(s.range.clone()).unwrap_or(""));
        }
    }
    out
}

/// `b` held to `text`, at the start of a character.
fn char_floor(text: &str, b: usize) -> usize {
    let mut b = b.min(text.len());
    while b > 0 && !text.is_char_boundary(b) {
        b = b.saturating_sub(1);
    }
    b
}

/// Character `ch`'s byte in `text` (the end, past its last).
pub(crate) fn byte_at(text: &str, ch: usize) -> usize {
    text.char_indices().nth(ch).map_or(text.len(), |(b, _)| b)
}

/// How many characters come before byte `b` (held to a character's start).
pub(crate) fn char_at(text: &str, b: usize) -> usize {
    text.get(..char_floor(text, b)).map_or(0, |s| s.chars().count())
}

fn hash(text: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

/// A note's text box between frames.
#[derive(Default)]
pub(crate) struct Editor {
    /// The text as written in the window.
    pub text: String,
    /// The selection at the end of the last frame (characters): the bar's buttons act on it.
    pub sel: Range<usize>,
    /// Give the text the keyboard (back), with this selection.
    pub refocus: Option<Range<usize>>,
    /// The text has the keyboard as the next frame begins (it had it, or it's given it back).
    pub focused: bool,
    /// The text as last laid out: its hash and width, and the galley.
    laid: Option<((u64, u32), Arc<Galley>)>,
    /// The text's ticks, and the hash of the text they were found in.
    ticks: (u64, Vec<Tick>),
}

impl Editor {
    pub(crate) fn new(text: String, sel: Range<usize>) -> Editor {
        Editor { text, sel, ..Default::default() }
    }
}

/// A tick's box: its first byte and character, ticked or not, and its words (its name).
struct Tick {
    byte: usize,
    char: usize,
    done: bool,
    name: String,
}

/// The ticks of a text, in order.
fn find_ticks(text: &str) -> Vec<Tick> {
    let mut out = Vec::new();
    let (mut byte, mut chars) = (0usize, 0usize);
    for line in markdown::read(text) {
        let Block::Tick { done, .. } = line.block else { continue };
        let Some(b) = line.spans.iter().find(|s| s.kind == Kind::Tick).map(|s| s.range.start) else { continue };
        chars = chars.saturating_add(text.get(byte..b).map_or(0, |s| s.chars().count()));
        byte = b;
        let words = words_of(text, &line.spans);
        let name: String = words.trim().chars().take(NAME_CHARS).collect();
        let name = if name.is_empty() { tl!("Tick box").to_string() } else { name };
        out.push(Tick { byte: b, char: chars, done, name });
    }
    out
}

/// What a button of the bar shows.
#[derive(Clone, Copy)]
pub(crate) enum Face {
    /// A letter (italic or not).
    Letter(&'static str, bool),
    Icon(Icon),
    /// Three dots: more.
    More,
}

/// A button of a note's bar, named `name` (its tip, and what a screen reader says).
pub(crate) fn button(ui: &mut Ui, face: Face, name: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(24.0, 22.0), Sense::click());
    widgets::named(ui, &resp, name, true, None);
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, Color32::from_black_alpha(24));
    }
    let ink = INK.gamma_multiply(0.85);
    match face {
        Face::Letter(s, italics) => {
            let format = TextFormat { font_id: theme::bold(14.0), color: ink, italics, ..Default::default() };
            let g = ui.painter().layout_job(LayoutJob::single_section(s.to_string(), format));
            ui.painter().galley(rect.center() - g.size() / 2.0, g, ink);
        }
        Face::Icon(i) => icons::paint(ui.painter(), icons::centered(rect, 15.0), i, ink),
        Face::More => {
            for dx in [-5.0, 0.0, 5.0] {
                ui.painter().circle_filled(rect.center() + vec2(dx, 0.0), 1.5, ink);
            }
        }
    }
    resp.on_hover_text(name)
}

/// The bar's buttons, in the row they're drawn in: each writes its mark on the text's
/// selection (as it was when the text last had the keyboard), which then has it back.
pub(crate) fn bar(ui: &mut Ui, ed: &mut Editor) {
    let buttons = [
        (Face::Letter("B", false), tl!("Bold (Ctrl+B)"), Mark::Bold),
        (Face::Letter("I", true), tl!("Italic (Ctrl+I)"), Mark::Italic),
        (Face::Letter("H", false), tl!("Heading"), Mark::Heading),
        (Face::Icon(Icon::ListBullet), tl!("Bulleted list"), Mark::List),
        (Face::Icon(Icon::Tasks), tl!("Tick box"), Mark::Tick),
        (Face::Icon(Icon::Link), tl!("Link"), Mark::Link),
    ];
    for (face, name, mark) in buttons {
        if button(ui, face, name).clicked() {
            let sel = byte_at(&ed.text, ed.sel.start)..byte_at(&ed.text, ed.sel.end);
            let e = markdown::toggle_mark(&ed.text, sel, mark);
            ed.text = e.text;
            ed.refocus = Some(char_at(&ed.text, e.select.start)..char_at(&ed.text, e.select.end));
        }
    }
}

/// The keys the text box takes from egui's own, while it has the keyboard: Ctrl+B and Ctrl+I,
/// Enter in a list item, Tab and Shift+Tab on list items.
fn keys(ui: &mut Ui, ed: &mut Editor, id: egui::Id) {
    let ctx = ui.ctx().clone();
    let Some((from, to)) = caret(&ctx, id) else { return };
    let sel = byte_at(&ed.text, from.min(to))..byte_at(&ed.text, from.max(to));
    let (bold, italic, enter, tab) = ui.input_mut(|i| {
        // With the modifiers each key came with.
        let pressed = |k: Key| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key { key, pressed: true, modifiers, .. } if *key == k && !modifiers.command && !modifiers.alt => Some(*modifiers),
                _ => None,
            })
        };
        let enter = pressed(Key::Enter).is_some_and(|m| !m.shift);
        let tab = pressed(Key::Tab).map(|m| m.shift);
        (i.consume_key(Modifiers::COMMAND, Key::B), i.consume_key(Modifiers::COMMAND, Key::I), enter, tab)
    });
    let edit = if bold {
        Some(markdown::toggle_mark(&ed.text, sel, Mark::Bold))
    } else if italic {
        Some(markdown::toggle_mark(&ed.text, sel, Mark::Italic))
    } else if enter && sel.is_empty() {
        markdown::enter(&ed.text, sel.start).inspect(|_| {
            ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
        })
    } else if let Some(out) = tab {
        markdown::indent(&ed.text, sel, out).inspect(|_| {
            ui.input_mut(|i| i.consume_key(if out { Modifiers::SHIFT } else { Modifiers::NONE }, Key::Tab));
        })
    } else {
        None
    };
    if let Some(e) = edit {
        ed.text = e.text;
        let (a, b) = (char_at(&ed.text, e.select.start), char_at(&ed.text, e.select.end));
        set_caret(&ctx, id, a, b);
        ed.sel = a..b;
    }
}

/// The note's text box, filling what is left of the window. True while it has the keyboard
/// ([`Editor::focused`]: or is given it back).
pub(crate) fn show(ui: &mut Ui, ed: &mut Editor, id: egui::Id, acts: &mut Vec<Action>) -> bool {
    let ctx = ui.ctx().clone();
    // The keyboard given back once the text is drawn: a click let go elsewhere this frame (on
    // the bar) would take it away again from a text that had it already.
    let refocus = ed.refocus.take();
    if let Some(r) = &refocus {
        set_caret(&ctx, id, r.start, r.end);
    }
    if ctx.memory(|m| m.has_focus(id)) {
        keys(ui, ed, id);
    }
    let mut laid = ed.laid.take();
    let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let text = buf.as_str();
        let key = (hash(text), wrap.to_bits());
        if let Some((_, g)) = laid.as_ref().filter(|(k, _)| *k == key) {
            return g.clone();
        }
        let galley = ui.fonts_mut(|f| f.layout_job(job(text, wrap, Look::WINDOW)));
        laid = Some((key, galley.clone()));
        galley
    };
    let text = &mut ed.text;
    // As small as the window lets it: the window's size is the note's, never the text's.
    let out = egui::ScrollArea::vertical()
        .id_salt(id.with("scroll"))
        .auto_shrink(false)
        .min_scrolled_height(0.0)
        .min_scrolled_width(0.0)
        .show(ui, |ui| {
            let room = ui.available_size();
            let out = egui::TextEdit::multiline(text)
                .id(id)
                .frame(egui::Frame::NONE)
                .lock_focus(true)
                .hint_text(RichText::new(tl!("Write your note…")).color(INK.gamma_multiply(0.45)))
                .desired_width(f32::INFINITY)
                .min_size(room)
                .layouter(&mut layouter)
                .show(ui);
            let clicked = ticks(ui, &mut ed.ticks, text, &out, id);
            (out, clicked)
        })
        .inner;
    ed.laid = laid;
    let (out, tick) = out;
    if let Some(r) = out.cursor_range {
        let r = r.as_sorted_char_range();
        ed.sel = r.start.0..r.end.0;
    }
    if let Some(b) = tick
        && markdown::toggle_tick(&mut ed.text, b)
    {
        ed.refocus = Some(ed.sel.clone());
        ctx.request_repaint();
    }
    links(ui, &ed.text, &out, acts);
    if refocus.is_some() {
        out.response.request_focus();
    }
    let focused = out.response.has_focus();
    ed.focused = focused || refocus.is_some();
    focused
}

/// A click on a tick's box: each box in view is a check box of its own over the text (named by
/// its words, ticked or not); the byte of the one clicked, if any.
fn ticks(ui: &mut Ui, found: &mut (u64, Vec<Tick>), text: &str, out: &egui::text_edit::TextEditOutput, id: egui::Id) -> Option<usize> {
    let key = hash(text);
    if found.0 != key {
        *found = (key, find_ticks(text));
    }
    let clip = ui.clip_rect();
    let origin = out.galley_pos.to_vec2();
    let mut clicked = None;
    let mut made = 0;
    for (n, t) in found.1.iter().enumerate() {
        let a = out.galley.pos_from_cursor(CCursor::new(t.char));
        let b = out.galley.pos_from_cursor(CCursor::new(t.char.saturating_add(3)));
        let right = if (b.min.y - a.min.y).abs() < 0.5 { b.min.x } else { a.min.x + 12.0 };
        let rect = Rect::from_min_max(a.min, egui::pos2(right.max(a.min.x + 4.0), a.max.y)).translate(origin);
        if !clip.intersects(rect) {
            continue;
        }
        made += 1;
        if made > MAX_TICKS {
            break;
        }
        let resp = ui.interact(rect, id.with(("tick", n)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, t.done, &t.name));
        if resp.clicked() {
            clicked = Some(t.byte);
        }
    }
    clicked
}

/// Ctrl+click on a link opens it, as a link in a message is ([`Action::OpenUrl`]: only web
/// pages and mail addresses, cleaned of trackers, a share asked about first); with Ctrl held, a
/// link under the pointer says where it leads.
fn links(ui: &Ui, text: &str, out: &egui::text_edit::TextEditOutput, acts: &mut Vec<Action>) {
    if !ui.input(|i| i.modifiers.command) {
        return;
    }
    let under = |p: egui::Pos2| {
        let c = out.galley.cursor_from_pos(p - out.galley_pos).index.0;
        [c, c.saturating_sub(1)].into_iter().find_map(|c| markdown::link_at(text, byte_at(text, c)).map(str::to_string))
    };
    let r = &out.response;
    if r.clicked()
        && let Some(url) = r.interact_pointer_pos().and_then(under)
    {
        acts.push(Action::OpenUrl(url));
    } else if let Some(url) = r.hover_pos().and_then(under) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        egui::Response::clone(r).on_hover_text_at_pointer(url);
    }
}
