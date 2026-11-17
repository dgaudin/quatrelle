// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The edit of one of the user's own messages in the box. ↑ in an empty box takes the window's
//! last message of the user ([`edit_last`]), the message's own Edit any of them
//! (`ConversationsUi::edit_asked`, [`asked`]), when the box can hold it (`TeamsState::editable`).
//! Its words are loaded in the box ([`begin`]), bold, italic and mentions kept, under "Editing
//! message" and Cancel ([`banner`]); what the box held is set aside ([`Words`]: a message the
//! engine refuses meanwhile goes back in there), and comes back once the edit is sent or let go
//! ([`cancel`], Esc or Cancel). Edit on the message being edited keeps what was typed; Edit on
//! another of the user's switches to it, deliberately: that one's words are loaded in the place
//! of the edit under way (its changes let go), and what was set aside before stays aside. Enter
//! sends it as `teams.edit` ([`send_edit`]),
//! unless it changes nothing — judged on what would be sent, the box's words and those loaded
//! composed alike: nothing is sent, the edit let go — or leaves nothing: the box says to delete
//! the message instead. One the engine refuses comes back in the box, still an edit of its
//! message ([`refused_edit`]).
//!
//! What is hostile here: the message's words (Graph's, read back by `teams_typed`, which gives
//! only what the box holds whole) are put in the box as text; the params of an edit refused (the
//! box's own, or a script's) are read as a refused send's are, their ranges checked against the
//! text, at the command's cap.

use super::*;

/// The user's own message being edited: its id, its words as the box loaded them (an edit that
/// changes nothing is judged against them), and what the box held before, set aside.
#[derive(Clone, Debug)]
pub(super) struct Editing {
    pub(super) message: String,
    loaded: Typed,
    pub(super) set_aside: Words,
}

/// The box's words: its text, its bold and italic, its mentions and its selection (what's set
/// aside during an edit).
#[derive(Clone, Debug, Default)]
pub(super) struct Words {
    text: String,
    styles: Styles,
    mentions: Vec<BoxMention>,
    sel: Range<usize>,
}

impl Words {
    /// The box's words, taken out of it.
    pub(super) fn take(d: &mut ChatDraft) -> Words {
        Words {
            text: std::mem::take(&mut d.text),
            styles: std::mem::take(&mut d.styles),
            mentions: std::mem::take(&mut d.mentions),
            sel: std::mem::take(&mut d.sel),
        }
    }

    /// These words in the box, the caret where they had it at the next frame.
    pub(super) fn into_box(self, d: &mut ChatDraft) {
        let n = self.text.chars().count();
        d.text = self.text;
        d.styles = self.styles;
        d.mentions = self.mentions;
        d.sel = self.sel.start.min(n)..self.sel.end.min(n);
        d.caret_to = Some(d.sel.clone());
        d.offer = None;
        d.changed();
    }

    /// A message that wasn't taken (`text`, its `styles` and `mentions`) put back before these
    /// words, on a line of its own; the caret where it was in them (at the message's end when
    /// they were empty). Mentions past [`MAX_MENTIONS`] are text.
    pub(super) fn put_back(&mut self, text: &str, styles: Styles, mentions: Vec<BoxMention>) {
        let n = text.chars().count();
        if self.text.is_empty() {
            *self = Words { text: text.to_string(), styles, mentions, sel: n..n };
            return;
        }
        let (mut back, mut st) = (text.to_string(), styles);
        rich::append(&mut back, &mut st, "\n", &Styles::plain("\n"));
        rich::append(&mut back, &mut st, &self.text, &self.styles);
        let shift = n.saturating_add(1);
        let moved = |r: &Range<usize>| r.start.saturating_add(shift)..r.end.saturating_add(shift);
        let mut all = mentions;
        all.extend(self.mentions.iter().map(|m| BoxMention { chars: moved(&m.chars), user: m.user.clone() }));
        all.truncate(MAX_MENTIONS);
        self.sel = moved(&self.sel);
        self.text = back;
        self.styles = st;
        self.mentions = all;
    }
}

/// The message `message` edited in the box, its words `typed` loaded.
pub(super) fn begin(d: &mut ChatDraft, message: &str, typed: Typed) {
    start(d, message, &typed.clone(), typed);
}

/// An edit of `message` in the box, `shown` put in it (bold, italic and mentions), the caret at
/// its end and the keyboard to the box; `loaded` what an edit that changes nothing is judged
/// against. What the box held is set aside (kept as it was when an edit was under way already).
fn start(d: &mut ChatDraft, message: &str, shown: &Typed, loaded: Typed) {
    let set_aside = match d.editing.take() {
        Some(e) => e.set_aside,
        None => Words::take(d),
    };
    let n = shown.text.chars().count();
    Words {
        text: shown.text.clone(),
        styles: styled(&shown.text, &shown.bold, &shown.italic),
        mentions: mentions::of_bytes(&shown.text, &shown.mentions),
        sel: n..n,
    }
    .into_box(d);
    d.editing = Some(Editing { message: message.to_string(), loaded, set_aside });
}

/// The edit let go: what the box held comes back, the caret where it was.
pub(super) fn cancel(d: &mut ChatDraft) {
    if let Some(e) = d.editing.take() {
        e.set_aside.into_box(d);
    }
}

/// Enter while editing: the box's words sent as the message's (`teams.edit`, the edit let go) —
/// unless they change nothing (nothing is sent, the edit let go), leave nothing (said: delete the
/// message instead), are too long for Teams or the outbox is `full` (said under the box).
pub(super) fn send_edit(d: &mut ChatDraft, chat: &str, full: bool, acts: &mut Vec<Action>) {
    let Some(e) = d.editing.as_ref() else { return };
    let typed = d.typed();
    let now = teams_compose_typed(&typed);
    if now == teams_compose_typed(&e.loaded) {
        cancel(d);
        return;
    }
    if now.0.blocks.is_empty() {
        d.said = Some(Refusal::NothingLeft);
        return;
    }
    if d.too_long || full {
        return;
    }
    let params = json!({
        "chat": chat,
        "message": e.message,
        "text": typed.text,
        "bold": pairs(&typed.bold),
        "italic": pairs(&typed.italic),
        "mentions": mention_pairs(&typed.mentions),
    });
    d.handed = Some((e.message.clone(), e.loaded.clone()));
    acts.push(Action::Run("teams.edit", params));
    cancel(d);
}

/// ↑ in an empty box: the window's last message of the user edited when the box can hold it;
/// else that's said, and nothing older is edited.
pub(super) fn edit_last(s: &Session, d: &mut ChatDraft) {
    if !d.text.is_empty() || d.editing.is_some() {
        return;
    }
    let me = s.prefs.teams.object_id.as_str();
    let Some(m) = s.teams.last_own(me) else { return };
    match s.teams.editable(m, me).then(|| s.teams.typed_for(m)).flatten() {
        Some(typed) => begin(d, &m.id, typed),
        None => d.said = Some(Refusal::LastNotEditable),
    }
}

/// The open chat's message `message` asked to be edited (from the message itself): edited when
/// it's in the window and the box can hold it; else nothing. The message being edited already:
/// what was typed stays, and the keyboard goes back to the box where it was.
pub(super) fn asked(s: &Session, d: &mut ChatDraft, message: &str) {
    if d.editing.as_ref().is_some_and(|e| e.message == message) {
        d.caret_to = Some(d.sel.clone());
        return;
    }
    let me = s.prefs.teams.object_id.as_str();
    let Some(m) = s.teams.window.iter().find(|m| m.id == message) else { return };
    if let Some(typed) = s.teams.editable(m, me).then(|| s.teams.typed_for(m)).flatten() {
        begin(d, &m.id, typed);
    }
}

/// A `teams.edit` the engine refused (`params`): its words back in the box, an edit of its
/// message again, the caret at their end. Judged against what the box had loaded when it was the
/// box's (else against nothing: sent again as it stands).
pub(super) fn refused_edit(d: &mut ChatDraft, params: &Value) {
    let message = params.get("message").and_then(Value::as_str).filter(|m| !m.is_empty() && m.len() <= MAX_ID);
    let (Some(message), Some(text)) = (message, params.get("text").and_then(Value::as_str)) else { return };
    let shown = Typed {
        text: text.to_string(),
        bold: ranges_of(text, params, "bold"),
        italic: ranges_of(text, params, "italic"),
        mentions: mentions_of(text, params),
    };
    let loaded = match d.handed.take() {
        Some((m, loaded)) if m == message => loaded,
        _ => Typed::default(),
    };
    start(d, message, &shown, loaded);
}

/// "Editing message" over the box, and Cancel: whether Cancel was clicked.
pub(super) fn banner(ui: &mut Ui, t: &Tokens) -> bool {
    let mut cancel = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(tl!("Editing message")).size(12.5).color(t.dim));
        cancel = ui.link(RichText::new(tl!("Cancel")).size(12.5).color(t.accent)).clicked();
    });
    ui.add_space(2.0);
    cancel
}
