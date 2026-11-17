// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The box at the foot of a conversation: a draft per chat (in memory, at most [`DRAFTS_MAX`]
//! chats), bold and italic over the text's [`Styles`] as the mail editor keeps them, the emoji
//! picker of the mail editor, and Enter to send (`teams.send`, with the byte ranges of the bold
//! and italic runs, and of the people @mentioned when there are); Shift+Enter makes a new line.
//! "@" offers the chat's people, mentioned by name (`mentions.rs`); ↑ in an empty box edits the
//! user's last message, Esc lets an edit go (`editing.rs`). The paperclip beside the emoji button
//! asks the window for files to attach, Ctrl+V with no text pasted for the clipboard's picture:
//! they go with the message as chips over the box, sent with the text or alone (`attach.rs`).
//! The params sent, and read back from a refusal, are `params.rs`'. These keys act where they
//! come among the frame's events, on the text as it stood when they were pressed (what's typed
//! after them in the same frame waits for the next), and are the box's as it stood at the
//! frame's start: the offer's while people are offered (Esc closes it), an edit's while one is
//! under way (Esc lets it go). An Esc the box doesn't take is the window's, as before (the
//! keyboard let go, the search emptied: `app/keys.rs` asks [`takes_escape`]). In Conversations
//! the window's keys never send the mail being written (`app/keys.rs`), so neither does a key
//! pressed in the box.
//!
//! The text is never lost. A message too long for Teams (its HTML past [`SEND_MAX`] bytes), or
//! with too many waiting already (the command's own caps, checked first), stays in the box, and
//! the box says why. One the engine refuses all the same goes back in it with its styles and
//! mentions, an edit as an edit of its message again ([`refused`]), the caret where it was; why
//! is said under the box in the interface's language: never in the engine's English words.
//!
//! What is hostile here: the text is the user's, sent as text (the writer escapes it, so what
//! looks like markup goes as it was typed); the chat's members' names (Graph's) are put in as
//! mentions on one line only; a message's words are loaded for an edit only as the box holds
//! them. What it costs is bounded: the drafts kept, the ranges sent and read back at the
//! command's caps, a text weighed again only when it changes.
//!
//! [`SEND_MAX`]: quatrelle_engine::model::pim::teams::SEND_MAX

use std::ops::Range;

use egui::text::{LayoutJob, TextFormat};
use egui::{Key, KeyboardShortcut, Modifiers, RichText, Ui};
use quatrelle_engine::Session;
use quatrelle_engine::mime::rich::{self, Style, Styles};
use quatrelle_engine::mime::{Typed, teams_compose_typed, teams_write_mentions};
use quatrelle_engine::model::pim::teams::{MAX_ID, MAX_MENTIONS, OUTBOX_CHAT_MAX, OUTBOX_MAX};
use serde_json::{Value, json};

use super::{ConversationsUi, TeamsAct};
use crate::icons::Icon;
use crate::rich_edit::{self, Picker};
use crate::theme::{self, Tokens};
use crate::{Action, widgets};

mod attach;
mod editing;
mod mentions;
mod params;

pub(crate) use attach::{add_files, add_picture};

use editing::{Editing, Words};
pub(crate) use mentions::BoxMention;
use mentions::Offer;
pub(crate) use params::refused;
use params::{Refusal, mention_pairs, mentions_of, pairs, ranges_of, styled};

/// The most chats whose drafts are kept: past it, the one drawn longest ago goes (an empty one
/// first).
const DRAFTS_MAX: usize = 20;
/// The text's size.
const SIZE: f32 = 14.0;
/// The tallest the box grows; past it, it scrolls.
const TALLEST: f32 = 160.0;
/// The most events kept for the next frame (what came after one of the box's keys): past them,
/// the rest go to the box at once.
const PENDING_MAX: usize = 1_000;

/// What a chat's box holds between frames.
#[derive(Clone, Debug, Default)]
pub(crate) struct ChatDraft {
    pub(crate) text: String,
    /// Its bold and italic, by character.
    pub(crate) styles: Styles,
    /// Its HTML would be longer than Teams takes: Enter does nothing, and the box says so.
    pub(crate) too_long: bool,
    /// The emoji picker's search and tab, once it was opened.
    pub(crate) emoji: Option<Picker>,
    /// The look of what's typed next, set by Ctrl+B or Ctrl+I with nothing selected.
    typing: Option<Style>,
    /// The selection (characters) at the end of the last frame.
    sel: Range<usize>,
    /// Why the last message (or an edit, a Retry, a Delete, ↑) wasn't taken: said under the box
    /// until the text changes.
    said: Option<Refusal>,
    /// When it was last drawn ([`ConversationsUi::drafted`]).
    used: u64,
    /// What came after one of the box's keys in a frame (typed, pasted, another key), for the
    /// next frame: so the key acts on the text as it stood when it was pressed.
    pending: Vec<egui::Event>,
    /// The people @mentioned in the text, by character, in its order (at most [`MAX_MENTIONS`]).
    pub(crate) mentions: Vec<BoxMention>,
    /// The chat's people offered after an "@" typed.
    offer: Option<Offer>,
    /// The user's own message being edited, and what the box held before.
    editing: Option<Editing>,
    /// Where the caret goes at the next frame (the keyboard given to the box): words put in the
    /// box from elsewhere than its keys (an edit begun or let go, a refusal, a member picked).
    caret_to: Option<Range<usize>>,
    /// The edit handed to the engine this frame (its message, and the words the box had loaded),
    /// for a refusal of it to be an edit judged as before; let go at the next frame.
    handed: Option<(String, Typed)>,
    /// The files and pictures going with the message, in the order added (`attach.rs`).
    pub(crate) parts: Vec<attach::BoxPart>,
    /// What of the files and pictures added wasn't taken, and why: said under the box until the
    /// text or the parts change.
    not_taken: Vec<String>,
}

impl ChatDraft {
    /// What the box holds, as `teams.send` and `teams.edit` take it: the text, the byte ranges of
    /// its bold and italic, and of each mention with its member's id.
    fn typed(&self) -> Typed {
        let (bold, italic) = runs(&self.text, &self.styles);
        let mut mentions = Vec::new();
        if !self.mentions.is_empty() {
            let at = Chars::of(&self.text);
            mentions = self.mentions.iter().filter_map(|m| Some((at.byte(m.chars.start)?..at.byte(m.chars.end)?, m.user.clone()))).collect();
        }
        Typed { text: self.text.clone(), bold, italic, mentions }
    }

    /// Whether its HTML is too long for Teams, worked out again (after each change).
    fn weigh(&mut self) {
        let (body, mentions) = teams_compose_typed(&self.typed());
        self.too_long = teams_write_mentions(&body, &mentions).is_err();
    }

    /// `text` put in place of the selection, in the look typing has there; the cursor after it.
    fn insert(&mut self, text: &str) {
        self.styles.fit(&self.text);
        let n = self.text.chars().count();
        let sel = self.sel.start.min(n)..self.sel.end.min(n);
        let (Some(head), Some(tail)) = (self.text.get(..byte_at(&self.text, sel.start)), self.text.get(byte_at(&self.text, sel.end)..)) else {
            return;
        };
        let new = format!("{head}{text}{tail}");
        let style = self.typing.clone().unwrap_or_else(|| rich_edit::typing_style(&self.text, &self.styles, sel.start));
        let at = sel.start + text.chars().count();
        self.styles.edit_at(&self.text, &new, &style, Some(at));
        mentions::follow(&mut self.mentions, &self.text, &new);
        self.text = new;
        self.sel = at..at;
        self.changed();
    }

    /// What's said under the box of the last refusal.
    #[cfg(test)]
    pub(crate) fn said(&self) -> Option<&'static str> {
        self.said.map(Refusal::sentence)
    }

    /// The message being edited in the box.
    #[cfg(test)]
    pub(crate) fn editing(&self) -> Option<&str> {
        self.editing.as_ref().map(|e| e.message.as_str())
    }

    /// What follows a change of the text or of its looks.
    fn changed(&mut self) {
        self.typing = None;
        self.said = None;
        self.not_taken.clear();
        self.weigh();
    }
}

/// Where each character of a text starts, to go between characters and bytes.
struct Chars {
    starts: Vec<usize>,
    len: usize,
}

impl Chars {
    fn of(text: &str) -> Chars {
        Chars { starts: text.char_indices().map(|(b, _)| b).collect(), len: text.len() }
    }

    /// The character a byte starts (the text's end too).
    fn at(&self, b: usize) -> Option<usize> {
        if b == self.len { Some(self.starts.len()) } else { self.starts.binary_search(&b).ok() }
    }

    /// A byte range's characters, both ends on characters' starts.
    fn range(&self, r: &Range<usize>) -> Option<Range<usize>> {
        let (from, to) = (self.at(r.start)?, self.at(r.end)?);
        (from <= to).then_some(from..to)
    }

    /// The first byte of character `c` (the text's end too).
    fn byte(&self, c: usize) -> Option<usize> {
        if c == self.starts.len() { Some(self.len) } else { self.starts.get(c).copied() }
    }
}

/// The byte character `c` of `text` starts at; past the text, its end.
fn byte_at(text: &str, c: usize) -> usize {
    text.char_indices().nth(c).map_or(text.len(), |(b, _)| b)
}

/// The text box's id, for `chat`.
fn box_id(chat: &str) -> egui::Id {
    egui::Id::unique(("teams-box", chat))
}

/// Whether Esc is the open chat's box's: it has the keyboard, and people are offered (Esc closes
/// the offer) or a message is edited (Esc lets the edit go). The window's own Esc (emptying the
/// search) waits then.
pub(crate) fn takes_escape(ctx: &egui::Context, c: &ConversationsUi, open: Option<&str>) -> bool {
    open.is_some_and(|chat| ctx.memory(|m| m.has_focus(box_id(chat))) && c.drafts.get(chat).is_some_and(|d| d.offering() || d.editing.is_some()))
}

/// One of the box's own keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BoxKey {
    /// Enter alone (Shift+Enter is the text box's new line): the message sent, or the edit.
    Send,
    Bold,
    Italic,
    /// Enter or Tab alone, people offered: the one chosen picked.
    Pick,
    /// ↑ or ↓ alone, people offered: the one above, or below, chosen.
    Previous,
    Next,
    /// Esc alone, people offered: the offer closed.
    Close,
    /// Esc alone, a message edited: the edit let go.
    Cancel,
    /// ↑ alone in an empty box: the user's last message edited.
    EditLast,
    /// Esc alone the box doesn't take, come after another of its keys in a frame (replayed):
    /// egui never saw it, so the box lets go of the keyboard as an Esc does.
    LetGo,
}

/// Which keys are the box's, as it stands at the frame's start.
struct Mode {
    offering: bool,
    editing: bool,
    /// Empty, not editing, and the window has a last message of the user's: ↑ edits it.
    up_edits: bool,
}

impl Mode {
    fn of(s: &Session, d: &ChatDraft) -> Mode {
        let (offering, editing) = (d.offering(), d.editing.is_some());
        let up_edits = !offering && !editing && d.text.is_empty() && s.teams.last_own(&s.prefs.teams.object_id).is_some();
        Mode { offering, editing, up_edits }
    }

    /// `e` as one of the box's keys (`replayed`: left from the last frame, after one of them).
    fn key(&self, e: &egui::Event, replayed: bool) -> Option<BoxKey> {
        let egui::Event::Key { key, pressed: true, modifiers, .. } = e else { return None };
        let alone = modifiers.is_none();
        match key {
            Key::Enter | Key::Tab if alone && self.offering => Some(BoxKey::Pick),
            Key::Enter if alone => Some(BoxKey::Send),
            Key::ArrowUp if alone && self.offering => Some(BoxKey::Previous),
            Key::ArrowDown if alone && self.offering => Some(BoxKey::Next),
            Key::ArrowUp if alone && self.up_edits => Some(BoxKey::EditLast),
            Key::Escape if alone && self.offering => Some(BoxKey::Close),
            Key::Escape if alone && self.editing => Some(BoxKey::Cancel),
            Key::Escape if alone && replayed => Some(BoxKey::LetGo),
            Key::B if modifiers.matches_logically(Modifiers::COMMAND) => Some(BoxKey::Bold),
            Key::I if modifiers.matches_logically(Modifiers::COMMAND) => Some(BoxKey::Italic),
            _ => None,
        }
    }
}

/// An event for the text box: what's typed, pasted, cut or copied, and keys.
fn for_the_box(e: &egui::Event) -> bool {
    matches!(e, egui::Event::Text(_) | egui::Event::Paste(_) | egui::Event::Key { .. } | egui::Event::Ime(_) | egui::Event::Copy | egui::Event::Cut)
}

/// `chat`'s draft, made when it has none: past [`DRAFTS_MAX`] chats, the one drawn longest ago
/// goes, an empty one first (one with an edit under way, or files, isn't empty).
pub(crate) fn draft<'a>(c: &'a mut ConversationsUi, chat: &str) -> &'a mut ChatDraft {
    c.drafted = c.drafted.saturating_add(1);
    if !c.drafts.contains_key(chat) && c.drafts.len() >= DRAFTS_MAX {
        let written = |d: &ChatDraft| !d.text.is_empty() || d.editing.is_some() || !d.parts.is_empty();
        let oldest = c.drafts.iter().min_by_key(|(_, d)| (written(d), d.used)).map(|(k, _)| k.clone());
        if let Some(k) = oldest {
            c.drafts.remove(&k);
        }
    }
    let d = c.drafts.entry(chat.to_string()).or_default();
    d.used = c.drafted;
    d
}

/// The byte ranges of `text` in bold and in italic, as `teams.send` takes them.
fn runs(text: &str, styles: &Styles) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let looks: Vec<(bool, bool)> = styles.palette.iter().map(|s| (s.bold, s.italic)).collect();
    let (mut bold, mut italic) = (Vec::new(), Vec::new());
    let (mut b, mut i) = (None, None);
    for (k, (at, _)) in text.char_indices().enumerate() {
        let (is_b, is_i) = styles.chars.get(k).and_then(|&p| looks.get(usize::from(p))).copied().unwrap_or_default();
        span(&mut bold, &mut b, is_b, at);
        span(&mut italic, &mut i, is_i, at);
    }
    span(&mut bold, &mut b, false, text.len());
    span(&mut italic, &mut i, false, text.len());
    (bold, italic)
}

/// A run of a style opened at `at` (`on`), or closed there.
fn span(out: &mut Vec<Range<usize>>, open: &mut Option<usize>, on: bool, at: usize) {
    match (*open, on) {
        (None, true) => *open = Some(at),
        (Some(from), false) => {
            out.push(from..at);
            *open = None;
        }
        _ => {}
    }
}

/// The box of `chat` (the open one), at the foot of the pane: none for a chat the user left or
/// one Teams won't let be read.
pub(crate) fn writing(s: &Session, c: &mut ConversationsUi, ui: &mut Ui, t: &Tokens, chat: &str, acts: &mut Vec<Action>) {
    let edit_asked = c.edit_asked.take();
    if s.teams.row(chat).is_none_or(|r| r.left || r.unreadable) {
        return;
    }
    // The command's caps, the open chat's share then all of it: past them nothing is taken.
    let full = s.teams.outgoing.len() >= OUTBOX_CHAT_MAX || usize::try_from(s.teams.unsent).unwrap_or(usize::MAX) >= OUTBOX_MAX;
    let d = draft(c, chat);
    d.handed = None;
    if let Some(message) = edit_asked {
        editing::asked(s, d, &message);
    }
    let frame = egui::Frame::new().fill(t.surface).inner_margin(egui::Margin { left: 16, right: 16, top: 6, bottom: 10 });
    egui::Panel::bottom("teams-box").resizable(false).show_separator_line(false).frame(frame).show(ui, |ui| {
        let id = box_id(chat);
        if let Some(r) = d.caret_to.take() {
            rich_edit::set_caret(ui.ctx(), id, r.start, r.end);
            ui.memory_mut(|m| m.request_focus(id));
            // egui holds the box's keys (Esc among them) only from the frame after the one that
            // gave it the keyboard: that frame is drawn at once, so an Esc right after is the box's.
            ui.ctx().request_repaint();
        }
        let key = if ui.memory(|m| m.has_focus(id)) {
            let mode = Mode::of(s, d);
            keys(ui, d, id, &mode)
        } else {
            // The keyboard went elsewhere: what was left for the box isn't given to another.
            d.pending.clear();
            None
        };
        let focused = ui.memory(|m| m.has_focus(id));
        // Ctrl+V with no text pasted: the window reads the clipboard's picture (an edit takes none).
        if focused && d.editing.is_none() && ui.input(|i| attach::pasting(&i.events)) {
            acts.push(Action::Teams(TeamsAct::Paste { chat: chat.to_string() }));
        }
        // The offer and the edit's banner over the box, as the frame found them; what's clicked
        // in them acts once the text box has taken this frame's typing. The files and pictures
        // going with the message (set aside with its words while one's own is edited).
        let picked = if focused { mentions::rows(ui, t, d) } else { None };
        let cancelled = d.editing.is_some() && editing::banner(ui, t);
        if d.editing.is_none() {
            attach::chips(ui, t, d);
        }
        let border = egui::Frame::new()
            .fill(if t.dark { t.surface } else { egui::Color32::WHITE })
            .stroke(egui::Stroke::new(1.0, if focused { t.accent } else { t.border }))
            .corner_radius(6)
            .inner_margin(egui::Margin::symmetric(10, 6));
        border.show(ui, |ui| {
            ui.horizontal_top(|ui| {
                // Files go with a message, not with an edit; the browser has none.
                let clip = d.editing.is_none() && !cfg!(target_arch = "wasm32");
                let room = (ui.available_width() - if clip { 66.0 } else { 36.0 }).max(80.0);
                egui::ScrollArea::vertical()
                    .id_salt(("teams-box-scroll", chat))
                    .max_height(TALLEST)
                    .max_width(room)
                    .auto_shrink([false, true])
                    .show(ui, |ui| edit(ui, t, d, id));
                emoji(ui, t, d, id, &s.prefs.recent_emoji, acts);
                if clip && widgets::bar_icon_button(ui, Icon::Attach, "Attach a file", t).clicked() {
                    acts.push(Action::Teams(TeamsAct::Attach { chat: chat.to_string() }));
                }
            });
        });
        // The key acts on the text as it stood when it was pressed: what came before it in the
        // frame was given to the text box just now.
        match key {
            Some(BoxKey::Send) if d.editing.is_some() => editing::send_edit(d, chat, full, acts),
            Some(BoxKey::Send) => send(ui, d, id, chat, full, acts),
            Some(BoxKey::Bold) => toggle(d, |s| s.bold, |s, on| s.bold = on),
            Some(BoxKey::Italic) => toggle(d, |s| s.italic, |s, on| s.italic = on),
            Some(BoxKey::Pick) => mentions::pick_chosen(d),
            Some(BoxKey::Previous) => mentions::choose(d, true),
            Some(BoxKey::Next) => mentions::choose(d, false),
            Some(BoxKey::Close) => mentions::close(d),
            Some(BoxKey::Cancel) => editing::cancel(d),
            Some(BoxKey::EditLast) => editing::edit_last(s, d),
            Some(BoxKey::LetGo) => ui.memory_mut(|m| m.surrender_focus(id)),
            None => {}
        }
        if let Some((user, name)) = picked {
            mentions::pick(d, &user, &name);
        }
        if cancelled {
            editing::cancel(d);
        }
        let focused = ui.memory(|m| m.has_focus(id));
        mentions::refresh(d, s.teams.members(), &s.prefs.teams.object_id, &s.prefs.teams.tenant_id, focused);
        // While people are offered or a message is edited, Tab and Esc are the box's (picking,
        // closing the offer, letting the edit go), not egui's (moving the keyboard, dropping it).
        let (offering, held) = (d.offering(), d.offering() || d.editing.is_some());
        let filter = egui::EventFilter { tab: offering, horizontal_arrows: true, vertical_arrows: true, escape: held };
        ui.memory_mut(|m| m.set_focus_lock_filter(id, filter));
        if key.is_some() || !d.pending.is_empty() || d.caret_to.is_some() {
            ui.ctx().request_repaint();
        }
        let said = if d.too_long {
            Some(tl!("This message is too long for Teams"))
        } else if full {
            Some(tl!("Too many messages are waiting"))
        } else {
            d.said.map(Refusal::sentence)
        };
        // Then what of the files wasn't taken (a sentence said already isn't said twice).
        let lines: Vec<&str> = said.into_iter().chain(d.not_taken.iter().map(String::as_str).filter(|l| Some(*l) != said)).collect();
        if !lines.is_empty() {
            ui.add_space(4.0);
        }
        for line in lines {
            ui.label(RichText::new(line).size(12.5).color(t.danger));
        }
    });
}

/// The box's own keys ([`Mode`]: as the box stood at the frame's start), taken where they come in
/// the frame's events, after what was left from the last frame: what comes before the first of
/// them is the text box's now; that key is returned, to act once the text box has taken it; what
/// follows it for the text box waits for the next frame (`pending`).
fn keys(ui: &mut Ui, d: &mut ChatDraft, id: egui::Id, mode: &Mode) -> Option<BoxKey> {
    let n = d.text.chars().count();
    if let Some((a, b)) = rich_edit::caret(ui.ctx(), id) {
        d.sel = a.min(b).min(n)..a.max(b).min(n);
    }
    let mut waiting = std::mem::take(&mut d.pending);
    let replayed = waiting.len();
    ui.input_mut(|i| {
        waiting.append(&mut i.events);
        let mut events = waiting;
        let Some(at) = events.iter().enumerate().position(|(k, e)| mode.key(e, k < replayed).is_some()) else {
            i.events = events;
            return None;
        };
        let mut after = events.split_off(at).into_iter();
        let key = after.next().and_then(|e| mode.key(&e, at < replayed));
        for e in after {
            if for_the_box(&e) && d.pending.len() < PENDING_MAX {
                d.pending.push(e);
            } else {
                events.push(e);
            }
        }
        i.events = events;
        key
    })
}

/// Bold or italic (`get`, `set`) on the selection, off when all of it has it already; with
/// nothing selected, for what's typed next.
fn toggle(d: &mut ChatDraft, get: fn(&Style) -> bool, set: fn(&mut Style, bool)) {
    d.styles.fit(&d.text);
    let sel = d.sel.clone();
    if sel.is_empty() {
        let mut next = d.typing.clone().unwrap_or_else(|| rich_edit::typing_style(&d.text, &d.styles, sel.start));
        let on = !get(&next);
        set(&mut next, on);
        d.typing = Some(next);
        return;
    }
    let on = !rich::all(&d.text, &d.styles, sel.clone(), get);
    d.styles.restyle(&d.text, sel, |s| {
        let mut s = s.clone();
        set(&mut s, on);
        s
    });
    d.said = None;
    d.weigh();
}

/// What's written sent with its files and pictures, when there's something to send (judged on
/// what would be sent: spaces, blank lines or control characters alone are nothing; files alone
/// are a message), it isn't too long and the outbox has room: the box emptied, the keyboard kept.
/// Otherwise it stays. The people @mentioned go as `mentions`, the files as `files` and the
/// pictures as `pictures` (their paths), when there are.
fn send(ui: &Ui, d: &mut ChatDraft, id: egui::Id, chat: &str, full: bool, acts: &mut Vec<Action>) {
    if d.too_long || full {
        return;
    }
    let typed = d.typed();
    if teams_compose_typed(&typed).0.blocks.is_empty() && d.parts.is_empty() {
        return;
    }
    let mut params = json!({"chat": chat, "text": typed.text, "bold": pairs(&typed.bold), "italic": pairs(&typed.italic)});
    let (files, pictures) = attach::paths(&d.parts);
    if let Some(p) = params.as_object_mut() {
        let lists = [("mentions", mention_pairs(&typed.mentions)), ("files", json!(files)), ("pictures", json!(pictures))];
        for (k, list) in lists.into_iter().filter(|(_, l)| l.as_array().is_some_and(|l| !l.is_empty())) {
            p.insert(k.into(), list);
        }
    }
    acts.push(Action::Run("teams.send", params));
    d.text.clear();
    d.styles = Styles::default();
    d.mentions.clear();
    d.parts.clear();
    d.offer = None;
    d.sel = 0..0;
    d.changed();
    rich_edit::set_caret(ui.ctx(), id, 0, 0);
}

/// The text box: its bold and italic drawn from the styles, which follow each change as the mail
/// editor's do, and its mentions as a bubble draws them, which follow it too.
fn edit(ui: &mut Ui, t: &Tokens, d: &mut ChatDraft, id: egui::Id) {
    d.styles.fit(&d.text);
    let before = d.text.clone();
    let styles = d.styles.clone();
    let typing = d.typing.clone().unwrap_or_else(|| rich_edit::typing_style(&before, &styles, d.sel.start));
    let sel = d.sel.clone();
    let marks = d.mentions.clone();
    let t = *t;
    let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let text = buf.as_str();
        let followed;
        let marks = if text == before {
            &marks
        } else {
            let mut m = marks.clone();
            mentions::follow(&mut m, &before, text);
            followed = m;
            &followed
        };
        ui.painter().layout_job(job(text, &rich_edit::following(&before, text, &styles, &typing, &sel), marks, &t, wrap))
    };
    let out = egui::TextEdit::multiline(&mut d.text)
        .id(id)
        .frame(egui::Frame::NONE)
        .hint_text(tl!("Write a message"))
        .desired_width(f32::INFINITY)
        .desired_rows(1)
        .return_key(KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter))
        .layouter(&mut layouter)
        .show(ui);
    let cursor = out.cursor_range.map(|r| {
        let r = r.as_sorted_char_range();
        r.start.0..r.end.0
    });
    if d.text != before {
        d.styles.edit_at(&before, &d.text, &typing, cursor.as_ref().map(|r| r.end));
        mentions::follow(&mut d.mentions, &before, &d.text);
        d.changed();
        ui.ctx().request_repaint();
    } else if cursor.as_ref().is_some_and(|r| *r != d.sel) {
        // The cursor moved: what Ctrl+B or Ctrl+I set for typing no longer applies.
        d.typing = None;
    }
    if let Some(r) = cursor {
        d.sel = r;
    }
}

/// `text` laid out in its bold and italic, its `mentions` (in the text's order) in the accent on
/// its soft tint, as a bubble draws them.
fn job(text: &str, styles: &Styles, mentions: &[BoxMention], t: &Tokens, wrap: f32) -> LayoutJob {
    let looks: Vec<(bool, bool)> = styles.palette.iter().map(|s| (s.bold, s.italic)).collect();
    let format = |(bold, italics, mention): (bool, bool, bool)| TextFormat {
        font_id: if bold { theme::bold(SIZE) } else { theme::regular(SIZE) },
        color: if mention { t.accent } else { t.text },
        background: if mention { t.accent_soft } else { egui::Color32::TRANSPARENT },
        italics,
        ..Default::default()
    };
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    let mut marks = mentions.iter().map(|m| &m.chars).peekable();
    let mut start = 0;
    let mut look = None;
    for (k, (at, _)) in text.char_indices().enumerate() {
        while marks.peek().is_some_and(|r| r.end <= k) {
            marks.next();
        }
        let (bold, italic) = styles.chars.get(k).and_then(|&p| looks.get(usize::from(p))).copied().unwrap_or_default();
        let here = (bold, italic, marks.peek().is_some_and(|r| r.contains(&k)));
        if look != Some(here) {
            if let (Some(was), Some(part)) = (look, text.get(start..at)) {
                job.append(part, 0.0, format(was));
            }
            (start, look) = (at, Some(here));
        }
    }
    match (look, text.get(start..)) {
        (Some(was), Some(part)) => job.append(part, 0.0, format(was)),
        // Empty: the box still has a line's height.
        _ => job.append("", 0.0, format((false, false, false))),
    }
    job
}

/// The emoji button and its picker (the mail editor's): the emoji chosen goes in at the cursor,
/// and the box gets the keyboard back.
fn emoji(ui: &mut Ui, t: &Tokens, d: &mut ChatDraft, id: egui::Id, recent: &[String], acts: &mut Vec<Action>) {
    let r = widgets::bar_icon_button(ui, Icon::Emoji, "Emoji", t);
    let mut chosen = None;
    egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        chosen = rich_edit::picker(ui, t, recent, d.emoji.get_or_insert_with(Picker::default));
    });
    if let Some(e) = chosen {
        d.insert(&e);
        rich_edit::set_caret(ui.ctx(), id, d.sel.start, d.sel.end);
        ui.memory_mut(|m| m.request_focus(id));
        acts.push(Action::Run("emoji.used", json!({ "emoji": e })));
    }
}
