//! The contacts field's glue with the session. [`PeopleFields`] keeps every field's state by
//! [`FieldId`] (the message's To, Cc, Bcc and Replies to; the Select Names dialog's rows), turns a
//! field's text into pieces and the widget's events back into text and actions, asks the engine for
//! suggestions, and holds the waits Send makes for the names still being resolved (one per
//! message).
//!
//! The work is bounded as the engine's is (the search bug of 2026-10-30): a field's text is parsed
//! when it changes (fed in, or the engine writing a name found), never on a frame where nothing
//! happened; the suggestions come from the engine's cache on an unchanged frame; the chips' kinds
//! are read again only when the address book changed, once it's read. The widget never sees the
//! `Session`, and nothing here draws beyond a line under the field.

use std::collections::{HashMap, HashSet};

use egui::{RichText, Ui};
use quatrelle_engine::model::Addressee;
use quatrelle_engine::model::addr::Accepts;
use quatrelle_engine::{FieldId, Session};
use serde_json::{Value, json};

use crate::Action;
use crate::look::people_field::{self, FieldInput, PeopleEvent, Piece, PieceError, text_of};
use crate::reading::check_names::Answer;
use crate::theme::Tokens;

mod field;
mod send;

use field::{Field, same_list};
use send::SendWait;

/// How long Send waits for the directory: a request's own timeout (`IO_TIMEOUT`, `net/src/stream.rs`).
pub const SEND_WAIT_SECS: f64 = 60.0;

/// The message's own fields, in the order Check Names goes through them.
pub(crate) const COMPOSE_FIELDS: [&str; 4] = ["to", "cc", "bcc", "replyTo"];

/// The Select Names dialog's rows (in the message's window, with their own names).
pub(crate) const BOOK_FIELDS: [&str; 3] = ["book-to", "book-cc", "book-bcc"];

/// The egui id a field is drawn under (its input, list, card, scroll and held click are keyed by
/// it): one per field, two message windows apart.
pub(crate) fn egui_id(field: FieldId) -> egui::Id {
    egui::Id::unique(("people-field", field.window, field.name))
}

/// What drawing a field came to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Drawn {
    /// The book button was pressed.
    pub book: bool,
    /// The text was written back with no event from the user: the field spelling what it knows
    /// (a name filled in, a comma name quoted), not a change of the owner's.
    pub normalised: bool,
}

/// Every contacts field on screen, by [`FieldId`].
#[derive(Default)]
pub(crate) struct PeopleFields {
    fields: HashMap<FieldId, Field>,
    /// The field whose input has the keyboard.
    open: Option<FieldId>,
    /// The sends held, by message.
    send_wait: HashMap<u64, SendWait>,
    /// The names whose choice the user cancelled (the field and the name as typed): a send that
    /// finds them unsettled stops quietly.
    cancelled: HashSet<(FieldId, String)>,
    /// The messages being written as of the last frame (one gone takes its fields with it).
    windows: HashSet<u64>,
}

/// The text of a message's own field (To, Cc, Bcc, Reply-To of the message whose id is the window).
fn compose_text(s: &Session, field: FieldId) -> Option<&String> {
    let c = s.compose.iter().chain(s.composes.iter()).find(|c| c.id == field.window)?;
    match field.name {
        "to" => Some(&c.to),
        "cc" => Some(&c.cc),
        "bcc" => Some(&c.bcc),
        "replyTo" => Some(&c.reply_to),
        _ => None,
    }
}

fn compose_text_mut(s: &mut Session, field: FieldId) -> Option<&mut String> {
    let c = s.compose.iter_mut().chain(s.composes.iter_mut()).find(|c| c.id == field.window)?;
    match field.name {
        "to" => Some(&mut c.to),
        "cc" => Some(&mut c.cc),
        "bcc" => Some(&mut c.bcc),
        "replyTo" => Some(&mut c.reply_to),
        _ => None,
    }
}

/// The parameters of `people.resolve` for a field: a message's own field is checked in the
/// message, any other hands its text in.
fn resolve_params(field: FieldId, account: Option<&str>, text: &str) -> Value {
    let mut p = json!({"window": field.window, "field": field.name});
    if let Some(a) = account {
        p["account"] = json!(a);
    }
    if !COMPOSE_FIELDS.contains(&field.name) {
        p["text"] = json!(text);
    }
    p
}

/// The parameters of `people.expand` for a list or a contact group.
fn expand_params(account: Option<&str>, who: &Addressee) -> Value {
    let mut p = json!({"name": who.name, "kind": who.kind.code()});
    if let Some(a) = account {
        p["account"] = json!(a);
    }
    if !who.address.is_empty() {
        p["email"] = json!(who.address);
    }
    if !who.item_id.is_empty() {
        p["itemId"] = json!(who.item_id);
    }
    p
}

impl PeopleFields {
    /// Make a field ready to draw: its text parsed into pieces when it changed, the names the
    /// engine settled taken, the lists expanded, and, while the field has the keyboard, the
    /// suggestions for what's typed (the engine's cache makes that free on an unchanged frame).
    pub(crate) fn prepare(&mut self, s: &mut Session, field: FieldId, account: Option<&str>, accepts: &Accepts, text: &str) {
        self.prepare_with(s, field, account, accepts, Some(text));
    }

    /// The message's four fields made ready, their text cloned only when it changed.
    pub(crate) fn prepare_compose(&mut self, s: &mut Session, id: u64, account: &str) {
        for name in COMPOSE_FIELDS {
            let field = FieldId { window: id, name };
            let Some(text) = compose_text(s, field) else { continue };
            let stale = self.fields.get(&field).is_none_or(|f| f.reparse || *text != f.st.synced);
            let text = stale.then(|| text.clone());
            self.prepare_with(s, field, Some(account), &Accepts::RECIPIENTS, text.as_deref());
        }
    }

    fn prepare_with(&mut self, s: &mut Session, field: FieldId, account: Option<&str>, accepts: &Accepts, text: Option<&str>) {
        let f = self.fields.entry(field).or_default();
        if let Some(text) = text
            && (f.reparse || text != f.st.synced)
        {
            f.parse(s, account, text);
        }
        f.refresh(s, field, account);
        if self.open == Some(field) && !f.st.input.trim().is_empty() {
            f.suggestions = s.people_suggest(field, account, accepts, &f.st.input);
        } else {
            f.suggestions.clear();
        }
    }

    /// Draw a field and apply what happened in it; `text` is written when the entries changed:
    /// by the owner, or by the field spelling what it knows ([`Drawn::normalised`]).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw(
        &mut self,
        ui: &mut Ui,
        t: &Tokens,
        field: FieldId,
        account: Option<&str>,
        accepts: &Accepts,
        label: &str,
        hint: &str,
        text: &mut String,
        focus: bool,
        can_expand: bool,
        book: bool,
        acts: &mut Vec<Action>,
    ) -> Drawn {
        let f = self.fields.entry(field).or_default();
        let id = egui_id(field);
        let input = FieldInput { id, label, hint, book, accepts, suggestions: &f.suggestions, can_expand, focus, pending: f.pending };
        let events = people_field::people_field(ui, t, &mut f.st, &input);
        let focused = ui.memory(|m| m.has_focus(people_field::input_id(id)));
        if focused {
            self.open = Some(field);
        } else if self.open == Some(field) {
            self.open = None;
        }
        let dirty = std::mem::take(&mut f.dirty);
        // A write from `dirty` with no event this frame is the field spelling what it knows (a
        // name filled in, the kinds read again) — unless the owner asked for what came (Expand,
        // a choice): that write is theirs.
        let normalised = dirty && events.is_empty() && !f.user_changed;
        if dirty {
            f.user_changed = false;
        }
        let mut changed = dirty;
        let (mut opened, mut resolve, mut closed_any) = (false, false, false);
        for e in events {
            match e {
                PeopleEvent::Changed => changed = true,
                PeopleEvent::Closed(closed) => {
                    f.close(&closed);
                    closed_any = true;
                    changed = true;
                }
                PeopleEvent::Take(k) => {
                    if let Some(who) = f.suggestions.get(k).map(|s| s.who.clone()) {
                        f.add_chip(who);
                        changed = true;
                    }
                }
                PeopleEvent::Forget(address) => acts.push(Action::Run("contacts.forget", json!({"email": address}))),
                PeopleEvent::RemoveChip(i) => {
                    if i < f.st.pieces.len() {
                        f.st.pieces.remove(i);
                        if f.st.pieces.is_empty() {
                            f.forget_losses();
                        } else if !f.partial {
                            f.note = None;
                        }
                        changed = true;
                    }
                }
                PeopleEvent::Expand(i) => {
                    if let Some(Piece::Chip { who, .. }) = f.st.pieces.get(i)
                        && !f.expanding.iter().any(|w| same_list(w, who))
                    {
                        acts.push(Action::Run("people.expand", expand_params(account, who)));
                        f.expanding.push(who.clone());
                        f.user_changed = true;
                    }
                }
                PeopleEvent::Resolve => resolve = true,
                PeopleEvent::OpenBook => opened = true,
            }
        }
        if resolve {
            f.close_input();
            changed = true;
        }
        // Text closed with the input left empty (the field left, a name closed by `;`, a paste)
        // is checked as Ctrl+K checks it; with something still being typed, the check waits for
        // it (typing never asks the server). Ctrl+K checks whatever is there.
        let ask = resolve || (closed_any && f.st.input.trim().is_empty() && f.needs_resolve());
        if ask {
            f.clear_errors(&[PieceError::Unreached]);
        }
        if changed {
            *text = text_of(&f.st.pieces, &f.st.input);
            f.st.synced.clone_from(text);
        }
        if ask {
            acts.push(Action::Run("people.resolve", resolve_params(field, account, &f.unresolved_text())));
        }
        if let Some(note) = &f.note {
            ui.horizontal(|ui| {
                ui.add_space(crate::widgets::FIELD_LABEL_W);
                ui.label(RichText::new(note).color(t.dim).small());
            });
        }
        Drawn { book: opened, normalised }
    }

    /// The choice dialog for a name several people fit in one of `names` of a window
    /// (`check_names::choice_dialog`), when one waits. A cancel there is the user's: a send held
    /// for that name stops quietly. A pick is the user's change to the field.
    pub(crate) fn choice_dialog(
        &mut self,
        ctx: &egui::Context,
        t: &Tokens,
        s: &Session,
        window: u64,
        names: &[&'static str],
        acts: &mut Vec<Action>,
    ) {
        if let Some(choice) = names.iter().find_map(|name| s.name_choice_for(FieldId { window, name })) {
            match crate::reading::check_names::choice_dialog(ctx, t, choice, acts) {
                Answer::Picked => {
                    if let Some(f) = self.fields.get_mut(&choice.field) {
                        f.user_changed = true;
                    }
                }
                Answer::Cancelled => {
                    self.cancelled.insert((choice.field, choice.token.clone()));
                }
                Answer::Pending => {}
            }
        }
    }

    /// The field whose input has the keyboard.
    pub(crate) fn open_field(&self) -> Option<FieldId> {
        self.open
    }

    pub(crate) fn pieces(&self, field: FieldId) -> &[Piece] {
        self.fields.get(&field).map_or(&[], |f| f.st.pieces.as_slice())
    }

    #[cfg(test)]
    pub(crate) fn suggestions(&self, field: FieldId) -> &[quatrelle_engine::Suggestion] {
        self.fields.get(&field).map_or(&[], |f| f.suggestions.as_slice())
    }

    /// How many times the field's text was parsed.
    #[cfg(test)]
    pub(crate) fn syncs(&self, field: FieldId) -> u32 {
        self.fields.get(&field).map_or(0, |f| f.st.syncs)
    }

    /// How many entries a paste or a text fed in left out, past the field's cap.
    #[cfg(test)]
    pub(crate) fn left_out(&self, field: FieldId) -> Option<usize> {
        self.fields.get(&field).and_then(|f| f.st.left_out)
    }

    /// Housekeeping each frame: the fields of messages gone, and a closed dialog's rows, go
    /// (here and in the engine); a send held for a message gone is off.
    pub(crate) fn settle(&mut self, s: &mut Session, live: &HashSet<u64>, book_open_for: Option<u64>) {
        for window in self.windows.difference(live) {
            s.forget_people_window(*window);
        }
        self.windows.clone_from(live);
        let closed: Vec<FieldId> = self
            .fields
            .keys()
            .copied()
            .filter(|f| live.contains(&f.window) && BOOK_FIELDS.contains(&f.name) && book_open_for != Some(f.window))
            .collect();
        for f in closed {
            s.forget_people_field(f);
        }
        self.fields.retain(|f, _| live.contains(&f.window) && (!BOOK_FIELDS.contains(&f.name) || book_open_for == Some(f.window)));
        if self.open.is_some_and(|f| !self.fields.contains_key(&f)) {
            self.open = None;
        }
        self.send_wait.retain(|id, _| live.contains(id));
        self.cancelled.retain(|(f, _)| live.contains(&f.window));
    }
}
