// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A note's window: a window of its own on the system, as a message's, in the note's colour,
//! moved and resized as any other, several open at once; in the browser, a window inside the
//! page. Its size is the note's: it opens at it (Outlook's usual 200 × 166 for a note never
//! sized) and goes back to it once a resize ends, not at each frame. Its place on the screen is
//! this computer's, kept with the other windows' places. Closing it or Escape saves its text, as
//! does its text losing the keyboard; ⋯ has the colours, Copy and Delete.

use egui::{Key, Margin, Modifiers, RichText, Sense, Stroke, Ui, vec2};
use quatrelle_engine::Session;
use quatrelle_engine::model::pim::Note;
use serde_json::{Map, Value, json};

use super::editor::{self, Editor, Face};
use super::{colors_menu, paper};
use crate::icons::Icon;
use crate::theme::Tokens;
use crate::{Action, UiState, WindowPlace, widgets};

/// The size a note never sized opens at (Outlook's), the smallest, and the largest side.
pub(crate) const USUAL: (u16, u16) = (200, 166);
pub(crate) const MIN: (u16, u16) = (160, 120);
pub(crate) const MAX: u16 = 4000;
/// How long a window's size stays put before it's taken as the end of a resize (seconds).
pub(crate) const STEADY: f64 = 0.5;
/// Most notes open in windows at once.
const MAX_OPEN: usize = 64;
/// How many frames a window opening (or brought forward) asks for the keyboard.
const RAISE: u8 = 8;
/// The room round the window's bar and text, and its edge.
const MARGIN: i8 = 6;
const EDGE: f32 = 1.0;

/// One note's window between frames.
pub(crate) struct NoteWindow {
    pub uid: String,
    pub editor: Editor,
    /// The text as last saved (the note's, unless it changed elsewhere since).
    pub saved: String,
    pub size: Sizer,
    /// Bring it to the front, its text given the keyboard: asked again for a few frames until
    /// the text has it (a window's first frame only measures it).
    pub raise: u8,
}

/// A window's size, followed so it is sent once a resize ends.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Sizer {
    /// The note's size (the window's at its opening), as last sent.
    pub sent: (u16, u16),
    /// The size seen last, and since when.
    pub seen: Option<(u16, u16)>,
    since: f64,
}

impl Sizer {
    pub(crate) fn new(at: (u16, u16)) -> Sizer {
        Sizer { sent: at, seen: None, since: 0.0 }
    }

    /// The window seen at `size` at `now` (`held`: a button is down, a resize may be going
    /// on): the size to send, once it has stayed put for [`STEADY`], once only.
    pub(crate) fn step(&mut self, size: egui::Vec2, now: f64, held: bool) -> Option<(u16, u16)> {
        let s = bounded(size)?;
        if self.seen != Some(s) {
            self.seen = Some(s);
            self.since = now;
            return None;
        }
        if s == self.sent || held || now - self.since < STEADY {
            return None;
        }
        self.sent = s;
        Some(s)
    }

    /// A size seen and not sent yet (the window repaints to send it).
    pub(crate) fn pending(&self) -> bool {
        self.seen.is_some_and(|s| s != self.sent)
    }

    /// At the window's closing: the size seen, when it wasn't sent.
    pub(crate) fn left(&mut self) -> Option<(u16, u16)> {
        let s = self.seen.filter(|s| *s != self.sent)?;
        self.sent = s;
        Some(s)
    }
}

/// A size in whole points held to what a note takes; none when it isn't a size.
fn bounded(v: egui::Vec2) -> Option<(u16, u16)> {
    if !(v.x.is_finite() && v.y.is_finite()) {
        return None;
    }
    let side = |x: f32, min: u16| x.round().clamp(f32::from(min), f32::from(MAX)) as u16;
    Some((side(v.x, MIN.0), side(v.y, MIN.1)))
}

/// Open note `note`'s window (selecting it), or bring it to the front when it's open.
pub(crate) fn open(st: &mut UiState, note: &Note) {
    st.note_selected = Some(note.uid.clone());
    if let Some(w) = st.note_windows.iter_mut().find(|w| w.uid == note.uid) {
        w.raise = RAISE;
        return;
    }
    if st.note_windows.len() >= MAX_OPEN {
        return;
    }
    let at = note.size().map_or(USUAL, |(x, y)| (x.clamp(MIN.0, MAX), y.clamp(MIN.1, MAX)));
    // The caret at the end, to go on writing.
    let end = note.text.chars().count();
    st.note_windows.push(NoteWindow {
        uid: note.uid.clone(),
        editor: Editor::new(note.text.clone(), end..end),
        saved: note.text.clone(),
        size: Sizer::new(at),
        raise: RAISE,
    });
}

/// What a window has to save: its text when it changed; at its closing, its size too when a
/// resize wasn't sent.
fn save(w: &mut NoteWindow, closing: bool) -> Option<Action> {
    let mut p = Map::new();
    if w.editor.text != w.saved {
        p.insert("text".into(), json!(w.editor.text));
        w.saved = w.editor.text.clone();
    }
    if closing && let Some((x, y)) = w.size.left() {
        p.insert("width".into(), json!(x));
        p.insert("height".into(), json!(y));
    }
    if p.is_empty() {
        return None;
    }
    p.insert("uid".into(), json!(w.uid));
    Some(Action::Run("note.update", Value::Object(p)))
}

fn viewport(uid: &str) -> egui::ViewportId {
    egui::ViewportId::from_hash_of(("note-window", uid))
}

/// The text box's id in note `uid`'s window.
pub(crate) fn text_id(uid: &str) -> egui::Id {
    egui::Id::unique(("note-text", uid))
}

/// The id of note `uid`'s window drawn inside the page.
pub(crate) fn in_page_id(uid: &str) -> egui::Id {
    egui::Id::unique(("note-window", uid))
}

/// Every note's window, once a frame. One whose note went (deleted here, elsewhere or on the
/// server) closes with nothing to save.
pub(crate) fn windows(ctx: &egui::Context, s: &Session, st: &mut UiState, acts: &mut Vec<Action>) {
    let open = std::mem::take(&mut st.note_windows);
    let embedded = ctx.embed_viewports();
    let mut keep = Vec::with_capacity(open.len());
    for (n, mut w) in open.into_iter().enumerate() {
        let Some(note) = s.note(&w.uid) else {
            if !embedded {
                forget(ctx, &w.uid);
            }
            continue;
        };
        // Changed elsewhere while nothing was typed here: what it says now.
        if note.text != w.saved && w.editor.text == w.saved {
            w.editor.text = note.text.clone();
            w.saved = note.text.clone();
        }
        let closed = if embedded { in_page(ctx, note, &mut w, st, acts) } else { of_its_own(ctx, note, &mut w, n, st, acts) };
        if closed {
            acts.extend(save(&mut w, true));
            if !embedded {
                forget(ctx, &w.uid);
            }
        } else {
            keep.push(w);
        }
    }
    keep.append(&mut st.note_windows);
    st.note_windows = keep;
}

/// A window of its own closed: it opens afresh, at its note's size, next time.
fn forget(ctx: &egui::Context, uid: &str) {
    crate::app::forget_open(ctx, viewport(uid));
    crate::os_windows::forget_first_frame(ctx, viewport(uid));
}

/// What a note's window holds: its bar (with ⋯, and Close inside the page) and its text. True
/// when it's to close.
fn body(ui: &mut Ui, note: &Note, w: &mut NoteWindow, embedded: bool, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    // Dark ink on light paper in either theme: the text box's caret and selection too.
    ui.style_mut().visuals = egui::Visuals::light();
    ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
    let id = text_id(&w.uid);
    let ctx = ui.ctx().clone();
    // As the frame began: egui lets go of the keyboard on Escape before anything is drawn.
    let typing = w.editor.focused;
    if embedded {
        let title = window_title(&w.editor.text);
        let r = ui.interact(ui.max_rect(), id.with("window"), Sense::hover());
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Window, true, &title));
    }
    // Escape closes it: from anywhere in a window of its own, from its text inside the page.
    let mut close = (!embedded || typing) && !ctx.any_popup_open() && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
    // The bar: on a second row when the window is narrow, nothing in it wider than the window.
    let width = ui.available_width();
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(width);
        editor::bar(ui, &mut w.editor);
        let more = editor::button(ui, Face::More, tl!("More actions"));
        egui::Popup::menu(&more).show(|ui| {
            colors_menu(ui, note, t, acts);
            ui.separator();
            if ui.button((widgets::menu_icon(Some(Icon::Copy)), tl!("Copy"))).clicked() {
                ui.ctx().copy_text(w.editor.text.clone());
                ui.close();
            }
            if ui.button((widgets::menu_icon(Some(Icon::Delete)), RichText::new(tl!("Delete")).color(t.danger))).clicked() {
                acts.push(Action::Run("note.delete", json!({"uid": note.uid})));
                ui.close();
            }
        });
        if embedded && editor::button(ui, Face::Icon(Icon::Close), tl!("Close")).clicked() {
            close = true;
        }
    });
    ui.add_space(2.0);
    if w.raise > 0 {
        w.editor.refocus.get_or_insert(w.editor.sel.clone());
    }
    let focused = editor::show(ui, &mut w.editor, id, acts);
    w.raise = if focused { 0 } else { w.raise.saturating_sub(1) };
    // The keyboard gone (a click elsewhere, Tab out, another window in front): what was typed
    // is kept. Not while a button is held: a click on the bar gives it back.
    let away = ui.input(|i| i.viewport().focused == Some(false));
    let held = ui.input(|i| i.pointer.any_down());
    if (!w.editor.focused || away) && w.editor.refocus.is_none() && !held {
        acts.extend(save(w, false));
    }
    close
}

/// The window's title: the note's first line, as the grid shows it.
fn window_title(text: &str) -> String {
    let title = editor::title(text);
    let title = if title.is_empty() { tl!("Untitled").to_string() } else { title };
    tf!("{title} – Note", title = title)
}

/// Its size seen this frame: sent once it stays put (and repainted until then).
fn follow(ctx: &egui::Context, w: &mut NoteWindow, size: egui::Vec2, acts: &mut Vec<Action>) {
    let (now, held) = ctx.input(|i| (i.time, i.pointer.any_down()));
    if let Some((x, y)) = w.size.step(size, now, held) {
        acts.push(Action::Run("note.update", json!({"uid": w.uid, "width": x, "height": y})));
    }
    if w.size.pending() {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(STEADY));
    }
}

/// A place to open at: where the last note's window was, its size aside (the note has its own).
fn spot(p: WindowPlace) -> Option<WindowPlace> {
    WindowPlace { w: p.w.max(200.0), h: p.h.max(150.0), maximized: false, ..p }.usable()
}

/// A window of its own on the system, opening `n` steps down and right of the last place.
fn of_its_own(ctx: &egui::Context, note: &Note, w: &mut NoteWindow, n: usize, st: &mut UiState, acts: &mut Vec<Action>) -> bool {
    let id = viewport(&w.uid);
    let min = [f32::from(MIN.0), f32::from(MIN.1)];
    let mut builder = crate::os_window().with_title(window_title(&w.editor.text)).with_min_inner_size(min);
    if crate::first_frame(ctx, id) {
        let (x, y) = w.size.sent;
        builder = builder.with_inner_size([f32::from(x), f32::from(y)]);
        if let Some(p) = st.places.get("note").copied().and_then(spot) {
            let step = 28.0 * (n % 8) as f32;
            builder = builder.with_position([p.x + step, p.y + step]);
        }
    }
    let t = st.tokens;
    let fill = paper(note.color, t.dark);
    let places = &mut st.places;
    ctx.show_viewport_immediate(id, builder, |ui, _| {
        let mut place = places.get("note").copied();
        if let Some(p) = place.and_then(spot) {
            crate::after_open(ui, Some(p));
        }
        crate::trace_window(ui, "note");
        WindowPlace::note(&mut place, ui);
        if let Some(p) = place {
            places.insert("note".into(), p);
        }
        let closing = ui.input(|i| i.viewport().close_requested());
        let raising = w.raise == RAISE;
        let mut close = false;
        egui::CentralPanel::default().frame(egui::Frame::new().fill(fill).inner_margin(Margin::same(MARGIN))).show(ui, |ui| {
            close = body(ui, note, w, false, &t, acts);
        });
        if raising {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        // Its size, but not while it's maximized, full screen or minimized: the note keeps its own.
        let v = ui.input(|i| i.viewport().clone());
        let own = v.maximized != Some(true) && v.fullscreen != Some(true) && v.minimized != Some(true);
        if own && let Some(r) = v.inner_rect {
            follow(ui.ctx(), w, r.size(), acts);
        }
        closing || close
    })
}

/// A window inside the page (the browser), of the same size.
fn in_page(ctx: &egui::Context, note: &Note, w: &mut NoteWindow, st: &mut UiState, acts: &mut Vec<Action>) -> bool {
    let t = st.tokens;
    let fill = paper(note.color, t.dark);
    // Its whole size is the note's (an egui window's size is its outside).
    let (x, y) = w.size.sent;
    let frame = egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(EDGE, fill.gamma_multiply(0.7)))
        .corner_radius(6)
        .inner_margin(Margin::same(MARGIN))
        .shadow(egui::epaint::Shadow { offset: [0, 3], blur: 12, spread: 0, color: egui::Color32::from_black_alpha(50) });
    let raising = w.raise > 0;
    let shown = egui::Window::new(window_title(&w.editor.text))
        .id(in_page_id(&w.uid))
        .title_bar(false)
        .collapsible(false)
        .resizable(true)
        .frame(frame)
        .default_size(vec2(f32::from(x), f32::from(y)))
        .min_size(vec2(f32::from(MIN.0), f32::from(MIN.1)))
        .max_size(vec2(f32::from(MAX), f32::from(MAX)))
        .show(ctx, |ui| body(ui, note, w, true, &t, acts));
    let Some(shown) = shown else { return false };
    if raising {
        ctx.move_to_top(shown.response.layer_id);
    }
    follow(ctx, w, shown.response.rect.size(), acts);
    shown.inner == Some(true)
}

impl crate::MailApp {
    /// Quatrelle closing: what the notes' windows hold is kept (their text, a size not sent).
    pub fn save_open_notes(&mut self) {
        for mut w in std::mem::take(&mut self.ui.note_windows) {
            if let Some(Action::Run(cmd, p)) = save(&mut w, true)
                && let Err(e) = self.session.run(cmd, &p)
            {
                log::warn!("a note's window couldn't keep its text: {e}");
            }
        }
    }
}
