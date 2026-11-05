// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Find in the message (R20, the owner's design of 5 November): a bar at the bottom of the
//! reading pane, or of a message's window, as a browser has. `/` opens it when no box is being
//! typed in (also More actions › Find in message); the words are looked for as they're typed,
//! capitals and accents aside (search's fold, [`fold_mapped`]), in the text the message shows,
//! quotes included, not its header nor its attachments' names. Every find is lit, the current
//! one more, and brought into view (a long message's part laid out for it). Enter and F3 go to
//! the next, with Shift to the one before; Escape closes it. Another message opened keeps it
//! open, its words looked for in that one.
//!
//! The bar is the window's own (what's typed, which find is the current one): what a message
//! shows is laid out here, not in the engine, so it has no engine command. `--cmd ui.find`
//! (optionally `={"query":…}`) opens it at start-up.
//!
//! [`fold_mapped`]: quatrelle_engine::model::search::fold_mapped

use std::ops::Range;
use std::sync::Arc;

use super::*;

mod doc;
mod marks;

pub(crate) use doc::Doc;
pub(crate) use marks::{Lit, Mark, place};

/// Most finds counted (and lit): a giant message never holds the window up counting.
pub(crate) const MOST: usize = 10_000;
/// Frames the current find is followed for into view, as a long message's heights settle.
const FOLLOW: u8 = 6;
/// The bar's height.
const BAR_H: f32 = 40.0;

/// The find bars: the reading pane's, and those of messages in windows of their own.
#[derive(Default)]
pub(crate) struct Finds {
    pub(crate) pane: Bar,
    windows: Vec<(MessageKey, Bar)>,
}

impl Finds {
    /// The bar of the reading pane, or of `key`'s window.
    pub(crate) fn bar(&mut self, in_window: bool, key: &MessageKey) -> &mut Bar {
        if !in_window {
            return &mut self.pane;
        }
        let i = match self.windows.iter().position(|(k, _)| k == key) {
            Some(i) => i,
            None => {
                if self.windows.len() >= 8 {
                    self.windows.remove(0);
                }
                self.windows.push((key.clone(), Bar::default()));
                self.windows.len() - 1
            }
        };
        match self.windows.get_mut(i) {
            Some((_, b)) => b,
            None => &mut self.pane,
        }
    }

    /// Whether `key`'s window shows its bar (Escape is the bar's then, not the window's).
    pub(crate) fn shown_in_window(&self, ctx: &egui::Context, key: &MessageKey) -> bool {
        self.windows.iter().any(|(k, b)| k == key && b.shown(ctx))
    }
}

/// One find bar.
#[derive(Default)]
pub(crate) struct Bar {
    pub(crate) open: bool,
    /// What's typed in it (kept when it closes, for the next time).
    pub(crate) query: String,
    /// Its box is to take the keyboard, its words selected.
    focus: bool,
    /// The current find.
    current: usize,
    /// What was looked in (the message, its text, and whether formatted), and for what.
    looked: Option<(MessageKey, Arc<mime::Parsed>, bool)>,
    searched: Option<String>,
    doc: Option<Arc<Doc>>,
    hits: Arc<Vec<Range<usize>>>,
    more: bool,
    /// Frames left following the current find into view, and the scroll asked for the next one.
    follow: u8,
    offset: Option<f32>,
    /// The frame it was last drawn in.
    drawn: Option<u64>,
}

impl Bar {
    /// Open it, its box taking the keyboard.
    pub(crate) fn open(&mut self) {
        self.open = true;
        self.focus = true;
        self.follow = FOLLOW;
    }

    pub(crate) fn close(&mut self) {
        self.open = false;
        self.follow = 0;
        self.offset = None;
        // What was looked in is let go (a long message's text, its fold).
        self.looked = None;
        self.doc = None;
        self.searched = None;
        self.hits = Arc::default();
    }

    /// To the next find (`forward`) or the one before, round from the end.
    pub(crate) fn step(&mut self, forward: bool) {
        let n = self.hits.len();
        if n == 0 {
            return;
        }
        self.current = if forward { (self.current + 1) % n } else { self.current.checked_sub(1).unwrap_or(n - 1) };
        self.follow = FOLLOW;
    }

    /// Whether it was drawn in this frame or the last (its keys are its own then).
    pub(crate) fn shown(&self, ctx: &egui::Context) -> bool {
        self.open && self.drawn.is_some_and(|d| ctx.cumulative_frame_nr().saturating_sub(d) <= 1)
    }

    /// The finds in the message `key` shows: looked for again when its text or the words change.
    fn refresh(&mut self, key: &MessageKey, p: &Arc<mime::Parsed>, formatted: bool) {
        let same = self.looked.as_ref().is_some_and(|(k, q, f)| k == key && Arc::ptr_eq(q, p) && *f == formatted);
        if !same {
            let other = self.looked.as_ref().is_none_or(|(k, _, _)| k != key);
            self.doc = Some(Arc::new(if formatted { Doc::formatted(p) } else { Doc::plain(&p.text) }));
            self.looked = Some((key.clone(), p.clone(), formatted));
            self.searched = None;
            if other {
                // Another message: from its first find, brought into view.
                self.current = 0;
                self.follow = FOLLOW;
            }
        }
        if self.searched.as_deref() != Some(self.query.as_str()) {
            let (hits, more) = self.doc.as_ref().map(|d| d.search(&self.query)).unwrap_or_default();
            if self.searched.is_some() {
                self.current = 0;
                self.follow = FOLLOW;
            }
            self.hits = Arc::new(hits);
            self.more = more;
            self.searched = Some(self.query.clone());
        }
        if self.current >= self.hits.len() {
            self.current = 0;
        }
    }

    /// What the message is drawn with: none until something is found.
    fn lit(&self, t: &Tokens) -> Option<Lit> {
        let doc = self.doc.clone()?;
        if self.hits.is_empty() || !self.open {
            return None;
        }
        let (all, now, ink) = fills(t);
        let mut lit = Lit { doc, hits: self.hits.clone(), current: self.current, target: None, all, now, ink };
        if self.follow > 0 {
            lit.target = lit.current_part();
        }
        Some(lit)
    }

    /// The scroll the body is to take this frame, if the current find is being followed.
    fn take_offset(&mut self) -> Option<f32> {
        self.offset.take()
    }

    /// After the body is drawn: the current find (`hit`, where it was drawn) brought into the
    /// middle of `view` when it isn't in it, the body scrolled `offset` down `content` high.
    fn follow_up(&mut self, hit: Option<Rect>, view: Rect, offset: f32, content: f32) {
        if self.follow == 0 {
            return;
        }
        self.follow -= 1;
        let Some(hit) = hit else { return };
        let inner = view.shrink2(vec2(0.0, (view.height() * 0.1).min(40.0)));
        if inner.contains_rect(hit) || (hit.height() > inner.height() && hit.intersects(inner)) {
            self.follow = 0;
            return;
        }
        let y = hit.center().y - view.top() + offset - view.height() / 2.0;
        self.offset = Some(y.clamp(0.0, (content - view.height()).max(0.0)));
    }

    /// The count: "3 of 12", or that nothing is found (empty while nothing is typed).
    fn count(&self) -> Option<(String, bool)> {
        if self.query.is_empty() {
            return None;
        }
        let n = self.hits.len();
        if n == 0 {
            return Some((tl!("No match").to_string(), false));
        }
        let (i, total) = (widgets::count_text(self.current as u64 + 1), widgets::count_text(n as u64));
        Some((if self.more { tf!("{i} of {n} or more", i = i, n = total) } else { tf!("{i} of {n}", i = i, n = total) }, true))
    }
}

impl crate::MailApp {
    /// Open the reading pane's find bar, with `query` typed in it when one is given (`--cmd
    /// ui.find`).
    pub fn find_in_message(&mut self, query: Option<&str>) {
        let bar = &mut self.ui.find.pane;
        bar.open();
        if let Some(q) = query {
            bar.query = q.to_string();
        }
    }
}

/// The finds' fill, the current one's, and the current one's text: a pale and a stronger amber,
/// readable on light and on dark.
pub(crate) fn fills(t: &Tokens) -> (Color32, Color32, Color32) {
    if t.dark {
        (Color32::from_rgb(0x5C, 0x4A, 0x14), Color32::from_rgb(0xF2, 0xB1, 0x3D), Color32::from_rgb(0x1F, 0x23, 0x28))
    } else {
        (Color32::from_rgb(0xFF, 0xEC, 0x99), Color32::from_rgb(0xFF, 0xA9, 0x3A), t.text)
    }
}

/// The bar's box, in the reading pane or `key`'s window.
pub(crate) fn field_id(in_window: bool, key: Option<&MessageKey>) -> egui::Id {
    match key.filter(|_| in_window) {
        Some(k) => egui::Id::unique(("find-in-message", k.to_id())),
        None => egui::Id::unique("find-in-message"),
    }
}

/// A box of text that can't be typed in (a plain message, which selects as a box does): `/`
/// there is for the find bar.
pub(crate) fn read_only(ctx: &egui::Context, id: egui::Id) {
    ctx.data_mut(|d| {
        let ids = d.get_temp_mut_or_default::<Vec<egui::Id>>(egui::Id::unique("read-only-text"));
        if !ids.contains(&id) {
            if ids.len() >= 16 {
                ids.remove(0);
            }
            ids.push(id);
        }
    });
}

/// Whether a box is being typed in (one that can be).
fn typing(ctx: &egui::Context) -> bool {
    let Some(id) = ctx.memory(|m| m.focused()) else { return false };
    let fixed = ctx.data(|d| d.get_temp::<Vec<egui::Id>>(egui::Id::unique("read-only-text"))).is_some_and(|ids| ids.contains(&id));
    ctx.text_edit_focused() && !fixed
}

/// `/` typed (not with Ctrl or Alt): taken, so no box gets it.
fn slash(ui: &Ui) -> bool {
    ui.input_mut(|i| {
        let is = |e: &egui::Event| match e {
            egui::Event::Text(t) => t == "/",
            egui::Event::Key { key: egui::Key::Slash, pressed: true, modifiers, .. } => !(modifiers.command || modifiers.ctrl || modifiers.alt),
            _ => false,
        };
        let found = i.events.iter().any(is);
        if found {
            i.events.retain(|e| !is(e));
        }
        found
    })
}

/// The keys of the bar of the message shown (`can`: a message's text is shown, no dialog over
/// it): `/` opens it, F3 and Shift+F3 go from find to find while it's shown, and in a window,
/// Escape closes it (the main window's Escape is `app/keys.rs`'s, which closes it first).
pub(super) fn keys(st: &mut UiState, ui: &Ui, in_window: bool, key: &MessageKey, can: bool) {
    if !can {
        return;
    }
    let ctx = ui.ctx().clone();
    let editing = typing(&ctx);
    let field = field_id(in_window, Some(key));
    let bar = st.find.bar(in_window, key);
    if !editing && slash(ui) {
        bar.open();
    }
    if !bar.shown(&ctx) {
        return;
    }
    let (back, on) = ui.input_mut(|i| (i.consume_key(egui::Modifiers::SHIFT, egui::Key::F3), i.consume_key(egui::Modifiers::NONE, egui::Key::F3)));
    if back {
        bar.step(false);
    } else if on {
        bar.step(true);
    }
    if in_window && (!editing || ctx.memory(|m| m.has_focus(field))) && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        bar.close();
    }
}

/// The bar, at the bottom of what `ui` has left, and the finds the message `key` (its text
/// `p`, `formatted` or not) is drawn with; the scroll its body is to take this frame.
pub(super) fn bar(
    st: &mut UiState,
    ui: &mut Ui,
    in_window: bool,
    key: &MessageKey,
    p: &Arc<mime::Parsed>,
    formatted: bool,
) -> (Option<Lit>, Option<f32>) {
    let t = st.tokens;
    let frame_nr = ui.ctx().cumulative_frame_nr();
    let field = field_id(in_window, Some(key));
    let bar = st.find.bar(in_window, key);
    if !bar.open {
        return (None, None);
    }
    bar.drawn = Some(frame_nr);
    let panel = egui::Panel::bottom(egui::Id::unique(("find-bar", in_window.then(|| key.to_id()))))
        .exact_size(BAR_H)
        .resizable(false)
        .show_separator_line(false)
        .frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(16, 6)));
    panel.show(ui, |ui| {
        let r = ui.max_rect();
        ui.painter().hline(r.x_range().expand(16.0), r.top() - 5.5, egui::Stroke::new(1.0, t.border));
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            // Enter and Shift+Enter in the box go from find to find (taken before the box, which
            // would let go of the keyboard).
            if ui.memory(|m| m.has_focus(field)) {
                let (back, on) = ui
                    .input_mut(|i| (i.consume_key(egui::Modifiers::SHIFT, egui::Key::Enter), i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)));
                if back {
                    bar.step(false);
                } else if on {
                    bar.step(true);
                }
            }
            let focused = ui.memory(|m| m.has_focus(field));
            let stroke = egui::Stroke::new(1.0, if focused { t.accent } else { t.border });
            egui::Frame::new().fill(t.surface).stroke(stroke).corner_radius(6).inner_margin(Margin::symmetric(8, 3)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (ir, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                    icons::paint(ui.painter(), ir, Icon::Search, t.dim);
                    let te = egui::TextEdit::singleline(&mut bar.query)
                        .id(field)
                        .hint_text(tl!("Find in message"))
                        .frame(egui::Frame::NONE)
                        .desired_width(220.0)
                        .char_limit(200)
                        .text_color(t.text);
                    let resp = ui.add(te);
                    if std::mem::take(&mut bar.focus) {
                        resp.request_focus();
                        // Its words selected, so typing replaces them.
                        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), field) {
                            let end = egui::text::CCursor::new(bar.query.chars().count());
                            state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), end)));
                            state.store(ui.ctx(), field);
                        }
                    }
                });
            });
            bar.refresh(key, p, formatted);
            if let Some((said, found)) = bar.count() {
                ui.label(RichText::new(said).color(if found { t.dim } else { t.danger }).small());
            }
            if widgets::tiny_icon_button(ui, Icon::ChevronUp, "Previous result", &t).clicked() {
                bar.step(false);
            }
            if widgets::tiny_icon_button(ui, Icon::ChevronDown, "Next result", &t).clicked() {
                bar.step(true);
            }
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if widgets::tiny_icon_button(ui, Icon::Close, "Close", &t).clicked() {
                    bar.close();
                }
            });
        });
    });
    if !bar.open {
        return (None, None);
    }
    (bar.lit(&t), bar.take_offset())
}

/// After the body: the current find, drawn at `hit`, followed into view.
pub(super) fn after_body(st: &mut UiState, in_window: bool, key: &MessageKey, hit: Option<Rect>, scrolled: &egui::scroll_area::ScrollAreaOutput<()>) {
    let bar = st.find.bar(in_window, key);
    if bar.open {
        bar.follow_up(hit, scrolled.inner_rect, scrolled.state.clamped_offset().y, scrolled.content_size.y);
    }
}
