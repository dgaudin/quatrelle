//! The contacts field's glue with the session. [`PeopleFields`] keeps every field's state by
//! [`FieldId`] (the message's To, Cc, Bcc and Replies to; the Select Names dialog's rows), turns a
//! field's text into pieces and the widget's events back into text and actions, asks the engine for
//! suggestions.
//!
//! The work is bounded as the engine's is (the search bug of 2026-10-30): a field's text is parsed
//! when it changes (fed in, or the engine writing a name found), never on a frame where nothing
//! happened; the suggestions come from the engine's cache on an unchanged frame; the chips' kinds
//! are read again only when the address book changed, once it's read. The widget never sees the
//! `Session`, and nothing here draws beyond a line under the field.

use std::collections::{HashMap, HashSet};

use egui::{RichText, Ui};
use quatrelle_engine::model::addr::{Accepts, MAX_ENTRIES, group_ref, parse_one, split_entries};
use quatrelle_engine::model::{Addressee, AddresseeKind};
use quatrelle_engine::{Fetched, FieldId, Outcome, Session, Suggestion, group_entry};
use serde_json::{Value, json};

use crate::Action;
use crate::look::people_field::{self, FieldInput, FieldState, PeopleEvent, Piece, PieceError, text_of};
use crate::theme::Tokens;

/// The message's own fields, in the order Check Names goes through them.
pub(crate) const COMPOSE_FIELDS: [&str; 4] = ["to", "cc", "bcc", "replyTo"];

/// The Select Names dialog's rows (in the message's window, with their own names).
pub(crate) const BOOK_FIELDS: [&str; 3] = ["book-to", "book-cc", "book-bcc"];

/// The egui id a field is drawn under (its input, list, card, scroll and held click are keyed by
/// it): one per field, two message windows apart.
pub(crate) fn egui_id(field: FieldId) -> egui::Id {
    egui::Id::unique(("people-field", field.window, field.name))
}

/// One field: the widget's state, and what the glue keeps beside it.
#[derive(Default)]
struct Field {
    st: FieldState,
    /// The suggestions for the input (the engine's cache on an unchanged frame).
    suggestions: Vec<Suggestion>,
    /// Names waiting on the directory, as of the last frame (the widget's spinner).
    pending: usize,
    /// The lists whose members were asked for: each chip is replaced by them once they come.
    expanding: Vec<Addressee>,
    /// A word under the field (how many members more, a list that couldn't be expanded), until
    /// the pieces next change.
    note: Option<String>,
    /// Text the widget closed hasn't been parsed yet: the field's text is parsed at the next frame.
    reparse: bool,
    /// The pieces changed without the widget (an expansion, a name found): the text is written
    /// back at the next draw.
    dirty: bool,
    /// The book's revision the chips' kinds were read at, and whether it was still being read.
    kinds_at: Option<(u64, bool)>,
}

/// Every contacts field on screen, by [`FieldId`].
#[derive(Default)]
pub(crate) struct PeopleFields {
    fields: HashMap<FieldId, Field>,
    /// The field whose input has the keyboard.
    open: Option<FieldId>,
    /// The messages being written as of the last frame (one gone takes its fields with it).
    windows: HashSet<u64>,
}

/// Whether two entries are the same list: by item id for a contact group (no address), by address
/// otherwise.
fn same_list(a: &Addressee, b: &Addressee) -> bool {
    if a.address.is_empty() || b.address.is_empty() {
        !a.item_id.is_empty() && a.item_id == b.item_id
    } else {
        a.address.eq_ignore_ascii_case(&b.address)
    }
}

/// The pieces of a field's text, cut to [`MAX_ENTRIES`]: a chip for an address (its kind and, when
/// the entry gives no name, the name known here) and for a contact group's reference, text for a
/// name (red at once when it has an `@` and isn't an address).
pub(crate) fn pieces_of(s: &Session, account: Option<&str>, text: &str) -> Vec<Piece> {
    split_entries(text).into_iter().take(MAX_ENTRIES).map(|entry| piece_of(s, account, entry)).collect()
}

fn piece_of(s: &Session, account: Option<&str>, entry: &str) -> Piece {
    if let Some((name, id)) = group_entry(entry) {
        let name = if name.is_empty() { s.directory_known(&group_ref(id)).map(|g| g.name.clone()).unwrap_or_default() } else { name };
        return Piece::chip(Addressee { item_id: id.to_string(), ..Addressee::new(name, "", AddresseeKind::ContactGroup) });
    }
    if entry.contains('@') {
        return match parse_one(entry) {
            Some(typed) => Piece::chip(s.known_addressee(account, typed)),
            None => Piece::Text { text: entry.to_string(), error: Some(PieceError::NotAnAddress) },
        };
    }
    Piece::Text { text: entry.to_string(), error: None }
}

/// Whether `text` spells its entries as the pieces (and the input, as its last entry) do: then
/// it is left as it is (`a@x` or `a@x; `, as it came), else it's written back as the chips show
/// it (a name filled in, a comma-name quoted, entries past the cap gone).
fn spelled_as(text: &str, pieces: &[Piece], input: &str) -> bool {
    let typed = split_entries(text);
    let input = input.trim();
    let n = pieces.len() + usize::from(!input.is_empty());
    if typed.len() != n {
        return false;
    }
    let written = pieces
        .iter()
        .map(|p| match p {
            Piece::Chip { who, .. } => who.text(),
            Piece::Text { text, .. } => text.clone(),
        })
        .chain((!input.is_empty()).then(|| input.to_string()));
    typed.iter().zip(written).all(|(t, w)| *t == w)
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

impl Field {
    /// Parse the field's text into pieces, once (`contacts_step(0)` first, so that the book names
    /// the people known here). What Check Names said of a piece stays said while the piece stays;
    /// what is being typed stays typed when the text ends in it (the engine wrote the text back
    /// while a key was typed). The text is written back when the pieces spell it otherwise.
    fn parse(&mut self, s: &mut Session, account: Option<&str>, text: &str) {
        s.contacts_step(0);
        let was_reparse = std::mem::take(&mut self.reparse);
        let entries = split_entries(text).len();
        let mut pieces = pieces_of(s, account, text);
        for p in &mut pieces {
            if let Piece::Text { text, error: error @ None } = p {
                *error = self.st.pieces.iter().find_map(|old| match old {
                    Piece::Text { text: t, error: Some(e) } if t == text => Some(e.clone()),
                    _ => None,
                });
            }
        }
        let typed = self.st.input.trim();
        if !typed.is_empty() && matches!(pieces.last(), Some(Piece::Text { text, .. }) if text == typed) {
            pieces.pop();
        } else {
            self.st.input.clear();
            self.st.open = false;
        }
        let cut = entries.saturating_sub(MAX_ENTRIES);
        if cut > 0 {
            self.st.left_out = Some(cut);
        } else if !was_reparse {
            self.st.left_out = None;
        }
        self.st.selected = None;
        self.st.card = None;
        self.st.pieces = pieces;
        self.st.syncs = self.st.syncs.saturating_add(1);
        self.st.synced = text.to_string();
        self.dirty |= !spelled_as(text, &self.st.pieces, &self.st.input);
        self.kinds_at = Some((s.contacts_revision(), s.contacts_pending()));
        let pieces = &self.st.pieces;
        self.expanding.retain(|who| pieces.iter().any(|p| matches!(p, Piece::Chip { who: w, .. } if same_list(w, who))));
    }

    /// What the engine has for the field since the last frame: the names settled (a chip, or why
    /// not), the lists expanded, the chips' kinds from the book once it's read, the names waiting.
    fn refresh(&mut self, s: &mut Session, field: FieldId, account: Option<&str>) {
        for (token, outcome) in s.take_people_outcomes(field) {
            let error = match outcome {
                Outcome::Found(who) => {
                    for p in &mut self.st.pieces {
                        if matches!(p, Piece::Text { text, .. } if *text == token) {
                            *p = Piece::chip(who.clone());
                            self.dirty = true;
                        }
                    }
                    continue;
                }
                // The choice dialog shows it.
                Outcome::Ambiguous => continue,
                Outcome::NotFound => PieceError::NotFound,
                Outcome::NotAnAddress => PieceError::NotAnAddress,
                Outcome::Unreached(_) => PieceError::Unreached,
            };
            for p in &mut self.st.pieces {
                if let Piece::Text { text, error: e } = p
                    && *text == token
                {
                    *e = Some(error.clone());
                }
            }
        }
        if let Some(dir) = s.directory_account(account) {
            self.settle_expansions(s, &dir);
        }
        let (revision, reading) = (s.contacts_revision(), s.contacts_pending());
        if !reading && self.kinds_at != Some((revision, reading)) {
            self.refresh_kinds(s, account);
            self.kinds_at = Some((revision, reading));
        }
        self.pending = s.people_pending(field);
    }

    /// The lists whose members came: each chip replaced by them (the field's cap kept), with how
    /// many more the list has; an empty list, or one that couldn't be expanded, is said and its
    /// chip stays.
    fn settle_expansions(&mut self, s: &Session, dir: &str) {
        let mut i = 0;
        while let Some(who) = self.expanding.get(i) {
            let ready = match s.expansion_now(dir, who) {
                Some(Fetched::Loading) => {
                    i += 1;
                    continue;
                }
                Some(Fetched::Ready(x)) => Some(x.clone()),
                Some(Fetched::Failed(e)) => {
                    self.note = Some(tf!("This list couldn't be expanded: {error}", error = e));
                    None
                }
                None => None,
            };
            let who = self.expanding.remove(i);
            let Some(x) = ready else { continue };
            if x.members.is_empty() {
                self.note = Some(tl!("This list has no members").to_string());
                continue;
            }
            if let Some(at) = self.st.pieces.iter().position(|p| matches!(p, Piece::Chip { who: w, .. } if same_list(w, &who))) {
                self.st.pieces.splice(at..=at, x.members.iter().cloned().map(Piece::chip));
                let over = self.st.pieces.len().saturating_sub(MAX_ENTRIES);
                if over > 0 {
                    self.st.pieces.truncate(MAX_ENTRIES);
                    self.st.left_out = Some(over);
                }
                self.st.selected = None;
                self.st.card = None;
                self.dirty = true;
            }
            let more = x.total.saturating_sub(x.members.len());
            self.note = if more > 0 {
                Some(tf!("and {n} more", n = crate::widgets::count_text(more as u64)))
            } else if x.more {
                Some(tl!("and others").to_string())
            } else {
                None
            };
        }
    }

    /// The chips' kinds and names from what is known now (the book read, a room list come).
    fn refresh_kinds(&mut self, s: &Session, account: Option<&str>) {
        for p in &mut self.st.pieces {
            if let Piece::Chip { who, .. } = p
                && !who.address.is_empty()
            {
                let known = s.known_addressee(account, who.email());
                self.dirty |= known.name != who.name;
                who.name = known.name;
                who.kind = known.kind;
            }
        }
    }

    /// Text the widget closed (`;`, a paste, the input left): entries as typed, up to the field's
    /// cap, parsed at the next frame.
    fn close(&mut self, closed: &str) {
        let room = MAX_ENTRIES.saturating_sub(self.st.pieces.len());
        let entries = split_entries(closed);
        let cut = entries.len().saturating_sub(room);
        self.st.pieces.extend(entries.iter().take(room).map(|e| Piece::Text { text: (*e).to_string(), error: None }));
        self.st.left_out = (cut > 0).then_some(cut);
        self.note = None;
        self.reparse = true;
    }

    /// What's being typed, closed as an entry (Ctrl+K, Send).
    fn close_input(&mut self) -> bool {
        let typed = self.st.input.trim().to_string();
        if typed.is_empty() {
            return false;
        }
        self.close(&typed);
        self.st.input.clear();
        self.st.open = false;
        true
    }

    fn add_chip(&mut self, who: Addressee) {
        if self.st.pieces.len() < MAX_ENTRIES {
            self.st.pieces.push(Piece::chip(who));
            self.st.left_out = None;
        } else {
            self.st.left_out = Some(1);
        }
        self.note = None;
    }
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

    /// Draw a field and apply what happened in it; `text` is written when the entries changed.
    /// Whether its book button was pressed.
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
    ) -> bool {
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
        let mut changed = std::mem::take(&mut f.dirty);
        let (mut opened, mut resolve) = (false, false);
        for e in events {
            match e {
                PeopleEvent::Changed => changed = true,
                PeopleEvent::Closed(closed) => {
                    f.close(&closed);
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
                        f.note = None;
                        changed = true;
                    }
                }
                PeopleEvent::Expand(i) => {
                    if let Some(Piece::Chip { who, .. }) = f.st.pieces.get(i)
                        && !f.expanding.iter().any(|w| same_list(w, who))
                    {
                        acts.push(Action::Run("people.expand", expand_params(account, who)));
                        f.expanding.push(who.clone());
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
        if changed {
            *text = text_of(&f.st.pieces, &f.st.input);
            f.st.synced.clone_from(text);
        }
        if resolve {
            acts.push(Action::Run("people.resolve", resolve_params(field, account, text)));
        }
        if let Some(note) = &f.note {
            ui.horizontal(|ui| {
                ui.add_space(crate::widgets::FIELD_LABEL_W);
                ui.label(RichText::new(note).color(t.dim).small());
            });
        }
        opened
    }

    /// The choice dialog for a name several people fit in one of `names` of a window
    /// (`check_names::choice_dialog`), when one waits.
    pub(crate) fn choice_dialog(&self, ctx: &egui::Context, t: &Tokens, s: &Session, window: u64, names: &[&'static str], acts: &mut Vec<Action>) {
        if let Some(choice) = names.iter().find_map(|name| s.name_choice_for(FieldId { window, name })) {
            crate::reading::check_names::choice_dialog(ctx, t, choice, acts);
        }
    }

    /// The field whose input has the keyboard.
    pub(crate) fn open_field(&self) -> Option<FieldId> {
        self.open
    }

    /// (Read by the tests until Send reads it to see what it can send to.)
    #[cfg(test)]
    pub(crate) fn pieces(&self, field: FieldId) -> &[Piece] {
        self.fields.get(&field).map_or(&[], |f| f.st.pieces.as_slice())
    }

    #[cfg(test)]
    pub(crate) fn suggestions(&self, field: FieldId) -> &[Suggestion] {
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
    /// (here and in the engine).
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
    }
}
