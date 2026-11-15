// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The box at the foot of a conversation: a draft per chat (in memory, at most [`DRAFTS_MAX`]
//! chats), bold and italic over the text's [`Styles`] as the mail editor keeps them, the emoji
//! picker of the mail editor, and Enter to send (`teams.send`, with the byte ranges of the bold
//! and italic runs); Shift+Enter makes a new line.
//!
//! The text is never lost. A message too long for Teams (its HTML past [`SEND_MAX`] bytes), or
//! with too many waiting already (the command's own caps, checked first), stays in the box, and
//! the box says why. One the engine refuses all the same goes back in it with its styles
//! ([`refused`]), and why is said under the box in the interface's language: never in the
//! engine's English words.
//!
//! What is hostile here: nothing from outside: the text is the user's, sent as text (the writer
//! escapes it, so what looks like markup goes as it was typed). What it costs is bounded: the
//! drafts kept, the ranges sent at the command's cap, a text weighed again only when it changes.
//!
//! [`SEND_MAX`]: quatrelle_engine::model::pim::teams::SEND_MAX

use std::ops::Range;

use egui::text::{LayoutJob, TextFormat};
use egui::{Key, KeyboardShortcut, Modifiers, RichText, Ui};
use quatrelle_engine::mime::rich::{self, Style, Styles};
use quatrelle_engine::mime::{teams_compose, teams_write};
use quatrelle_engine::model::pim::teams::{MAX_ID, OUTBOX_CHAT_MAX, OUTBOX_MAX};
use quatrelle_engine::{CmdError, Session};
use serde_json::{Value, json};

use super::ConversationsUi;
use crate::icons::Icon;
use crate::rich_edit::{self, Picker};
use crate::theme::{self, Tokens};
use crate::{Action, widgets};

/// The most chats whose drafts are kept: past it, the one drawn longest ago goes (an empty one
/// first).
const DRAFTS_MAX: usize = 20;
/// The most byte ranges of each style `teams.send` takes.
const RANGES_MAX: usize = 1_000;
/// The text's size.
const SIZE: f32 = 14.0;
/// The tallest the box grows; past it, it scrolls.
const TALLEST: f32 = 160.0;

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
    /// Why the last message (or a Retry, a Delete) wasn't taken: said under the box until the
    /// text changes.
    said: Option<Refusal>,
    /// When it was last drawn ([`ConversationsUi::drafted`]).
    used: u64,
}

impl ChatDraft {
    /// Whether its HTML is too long for Teams, worked out again (after each change).
    fn weigh(&mut self) {
        let (bold, italic) = runs(&self.text, &self.styles);
        self.too_long = teams_write(&teams_compose(&self.text, &bold, &italic)).is_err();
    }

    /// `text` put in place of the selection, in the look typing has there; the cursor after it.
    fn insert(&mut self, text: &str) {
        self.styles.fit(&self.text);
        let n = self.text.chars().count();
        let sel = self.sel.start.min(n)..self.sel.end.min(n);
        let byte = |at: usize| self.text.char_indices().nth(at).map_or(self.text.len(), |(b, _)| b);
        let (Some(head), Some(tail)) = (self.text.get(..byte(sel.start)), self.text.get(byte(sel.end)..)) else { return };
        let new = format!("{head}{text}{tail}");
        let style = self.typing.clone().unwrap_or_else(|| rich_edit::typing_style(&self.text, &self.styles, sel.start));
        let at = sel.start + text.chars().count();
        self.styles.edit_at(&self.text, &new, &style, Some(at));
        self.text = new;
        self.sel = at..at;
        self.changed();
    }

    /// What follows a change of the text or of its looks.
    fn changed(&mut self) {
        self.typing = None;
        self.said = None;
        self.weigh();
    }
}

/// Why something the box asked for wasn't taken, as the window says it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Refusal {
    TooLong,
    TooMany,
    Left,
    MayBeInTeams,
    /// Anything else (the chat gone from the list, say).
    NotSent,
}

impl Refusal {
    /// The engine's refusal of `id` as one of the window's (its English words are never drawn);
    /// none for a Retry or a Delete refused for a reason there's nothing to say of.
    fn of(id: &str, e: &CmdError) -> Option<Refusal> {
        let words = match e {
            CmdError::Params(m) | CmdError::Disabled(m) | CmdError::Failed(m) => m.as_str(),
            CmdError::Unknown(_) => "",
        };
        match words {
            "This message is too long for Teams" => Some(Refusal::TooLong),
            "Too many messages are waiting" => Some(Refusal::TooMany),
            "This message may already be in Teams" => Some(Refusal::MayBeInTeams),
            "you're no longer in this conversation" => Some(Refusal::Left),
            _ if id == "teams.send" => Some(Refusal::NotSent),
            _ => None,
        }
    }

    fn sentence(self) -> &'static str {
        match self {
            Refusal::TooLong => tl!("This message is too long for Teams"),
            Refusal::TooMany => tl!("Too many messages are waiting"),
            Refusal::Left => tl!("You're no longer in this conversation"),
            Refusal::MayBeInTeams => tl!("This message may already be in Teams"),
            Refusal::NotSent => tl!("This message couldn't be sent"),
        }
    }
}

/// The text box's id, for `chat`.
fn box_id(chat: &str) -> egui::Id {
    egui::Id::unique(("teams-box", chat))
}

/// `chat`'s draft, made when it has none: past [`DRAFTS_MAX`] chats, the one drawn longest ago
/// goes, an empty one first.
fn draft<'a>(c: &'a mut ConversationsUi, chat: &str) -> &'a mut ChatDraft {
    c.drafted = c.drafted.saturating_add(1);
    if !c.drafts.contains_key(chat) && c.drafts.len() >= DRAFTS_MAX {
        let oldest = c.drafts.iter().min_by_key(|(_, d)| (!d.text.is_empty(), d.used)).map(|(k, _)| k.clone());
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

/// Ranges as `teams.send` takes them: `[[from, to]]`.
fn pairs(ranges: &[Range<usize>]) -> Value {
    Value::Array(ranges.iter().map(|r| json!([r.start, r.end])).collect())
}

/// `text`'s styles from `teams.send`'s params: bold and italic on the byte ranges given (at most
/// [`RANGES_MAX`] of each; one off the text's characters is left out).
fn styled(text: &str, params: &Value) -> Styles {
    let mut st = Styles::plain(text);
    // Each character's first byte, to find where a byte range starts and ends in characters.
    let starts: Vec<usize> = text.char_indices().map(|(b, _)| b).collect();
    let char_at = |b: usize| if b == text.len() { Some(starts.len()) } else { starts.binary_search(&b).ok() };
    let looks: [(&str, fn(&mut Style)); 2] = [("bold", |s| s.bold = true), ("italic", |s| s.italic = true)];
    for (k, set) in looks {
        for r in params.get(k).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default().iter().take(RANGES_MAX) {
            let at = |i: usize| r.get(i).and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).and_then(char_at);
            if let (Some(from), Some(to)) = (at(0), at(1))
                && from < to
            {
                st.restyle(text, from..to, |s| {
                    let mut s = s.clone();
                    set(&mut s);
                    s
                });
            }
        }
    }
    st
}

/// What the engine refused of the box's (`teams.send`, `.retry` or `.discard`, with `params`;
/// `open` is the chat on screen, which Retry and Delete are of): a message sent goes back in its
/// chat's box with its bold and italic, before what was typed since; why is said under the box.
pub(crate) fn refused(c: &mut ConversationsUi, open: Option<&str>, id: &str, params: &Value, e: &CmdError) {
    let chat = if id == "teams.send" { params.get("chat").and_then(Value::as_str) } else { open };
    let Some(chat) = chat.filter(|c| !c.is_empty() && c.len() <= MAX_ID) else { return };
    let said = Refusal::of(id, e);
    let d = draft(c, chat);
    if id == "teams.send"
        && let Some(text) = params.get("text").and_then(Value::as_str)
    {
        let (mut back, mut styles) = (text.to_string(), styled(text, params));
        if !d.text.is_empty() {
            rich::append(&mut back, &mut styles, "\n", &Styles::plain("\n"));
            rich::append(&mut back, &mut styles, &d.text, &d.styles);
        }
        d.text = back;
        d.styles = styles;
        d.sel = 0..0;
        d.changed();
    }
    if said.is_some() {
        d.said = said;
    }
}

/// The box of `chat` (the open one), at the foot of the pane: none for a chat the user left or
/// one Teams won't let be read.
pub(crate) fn writing(s: &Session, c: &mut ConversationsUi, ui: &mut Ui, t: &Tokens, chat: &str, acts: &mut Vec<Action>) {
    if s.teams.row(chat).is_none_or(|r| r.left || r.unreadable) {
        return;
    }
    // The command's caps, the open chat's share then all of it: past them nothing is taken.
    let full = s.teams.outgoing.len() >= OUTBOX_CHAT_MAX || usize::try_from(s.teams.unsent).unwrap_or(usize::MAX) >= OUTBOX_MAX;
    let d = draft(c, chat);
    let frame = egui::Frame::new().fill(t.surface).inner_margin(egui::Margin { left: 16, right: 16, top: 6, bottom: 10 });
    egui::Panel::bottom("teams-box").resizable(false).show_separator_line(false).frame(frame).show(ui, |ui| {
        let id = box_id(chat);
        if ui.memory(|m| m.has_focus(id)) {
            keys(ui, d, id, chat, full, acts);
        }
        let focused = ui.memory(|m| m.has_focus(id));
        let border = egui::Frame::new()
            .fill(if t.dark { t.surface } else { egui::Color32::WHITE })
            .stroke(egui::Stroke::new(1.0, if focused { t.accent } else { t.border }))
            .corner_radius(6)
            .inner_margin(egui::Margin::symmetric(10, 6));
        border.show(ui, |ui| {
            ui.horizontal_top(|ui| {
                let room = (ui.available_width() - 36.0).max(80.0);
                egui::ScrollArea::vertical()
                    .id_salt(("teams-box-scroll", chat))
                    .max_height(TALLEST)
                    .max_width(room)
                    .auto_shrink([false, true])
                    .show(ui, |ui| edit(ui, t, d, id));
                emoji(ui, t, d, id, &s.prefs.recent_emoji, acts);
            });
        });
        let said = if d.too_long {
            Some(tl!("This message is too long for Teams"))
        } else if full {
            Some(tl!("Too many messages are waiting"))
        } else {
            d.said.map(Refusal::sentence)
        };
        if let Some(said) = said {
            ui.add_space(4.0);
            ui.label(RichText::new(said).size(12.5).color(t.danger));
        }
    });
}

/// The box's own keys, taken before it sees them: Enter alone sends, Ctrl+B and Ctrl+I make the
/// selection bold or italic (with nothing selected, what's typed next).
fn keys(ui: &mut Ui, d: &mut ChatDraft, id: egui::Id, chat: &str, full: bool, acts: &mut Vec<Action>) {
    let n = d.text.chars().count();
    if let Some((a, b)) = rich_edit::caret(ui.ctx(), id) {
        d.sel = a.min(b).min(n)..a.max(b).min(n);
    }
    let (mut enter, mut bold, mut italic) = (false, false, false);
    ui.input_mut(|i| {
        // Enter alone: Shift+Enter is the box's new line (`consume_key` would take it too).
        i.events.retain(|e| match e {
            egui::Event::Key { key: Key::Enter, pressed: true, modifiers, .. } if modifiers.is_none() => {
                enter = true;
                false
            }
            _ => true,
        });
        bold = i.consume_key(Modifiers::COMMAND, Key::B);
        italic = i.consume_key(Modifiers::COMMAND, Key::I);
    });
    if bold {
        toggle(d, |s| s.bold, |s, on| s.bold = on);
    }
    if italic {
        toggle(d, |s| s.italic, |s, on| s.italic = on);
    }
    if enter {
        send(ui, d, id, chat, full, acts);
    }
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

/// What's written sent, when there's something to send (judged on what would be sent: spaces,
/// blank lines or control characters alone are nothing), it isn't too long and the outbox has
/// room: the box emptied, the keyboard kept. Otherwise it stays.
fn send(ui: &Ui, d: &mut ChatDraft, id: egui::Id, chat: &str, full: bool, acts: &mut Vec<Action>) {
    if d.too_long || full {
        return;
    }
    let (bold, italic) = runs(&d.text, &d.styles);
    if teams_compose(&d.text, &bold, &italic).blocks.is_empty() {
        return;
    }
    acts.push(Action::Run("teams.send", json!({"chat": chat, "text": d.text, "bold": pairs(&bold), "italic": pairs(&italic)})));
    d.text.clear();
    d.styles = Styles::default();
    d.sel = 0..0;
    d.changed();
    rich_edit::set_caret(ui.ctx(), id, 0, 0);
}

/// The text box: its bold and italic drawn from the styles, which follow each change as the mail
/// editor's do.
fn edit(ui: &mut Ui, t: &Tokens, d: &mut ChatDraft, id: egui::Id) {
    d.styles.fit(&d.text);
    let before = d.text.clone();
    let styles = d.styles.clone();
    let typing = d.typing.clone().unwrap_or_else(|| rich_edit::typing_style(&before, &styles, d.sel.start));
    let sel = d.sel.clone();
    let color = t.text;
    let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let text = buf.as_str();
        ui.painter().layout_job(job(text, &rich_edit::following(&before, text, &styles, &typing, &sel), color, wrap))
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

/// `text` laid out in its bold and italic.
fn job(text: &str, styles: &Styles, color: egui::Color32, wrap: f32) -> LayoutJob {
    let looks: Vec<(bool, bool)> = styles.palette.iter().map(|s| (s.bold, s.italic)).collect();
    let format = |(bold, italics): (bool, bool)| TextFormat {
        font_id: if bold { theme::bold(SIZE) } else { theme::regular(SIZE) },
        color,
        italics,
        ..Default::default()
    };
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    let mut start = 0;
    let mut look = None;
    for (k, (at, _)) in text.char_indices().enumerate() {
        let here = styles.chars.get(k).and_then(|&p| looks.get(usize::from(p))).copied().unwrap_or_default();
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
        _ => job.append("", 0.0, format((false, false))),
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
