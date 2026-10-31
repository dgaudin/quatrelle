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
use quatrelle_engine::model::addr::{Accepts, MAX_ENTRIES, group_ref, parse_one, split_entries};
use quatrelle_engine::model::{Addressee, AddresseeKind};
use quatrelle_engine::{Fetched, FieldId, Outcome, Session, Suggestion, group_entry};
use serde_json::{Value, json};

use crate::Action;
use crate::look::people_field::{self, FieldInput, FieldState, PeopleEvent, Piece, PieceError, error_text, text_of};
use crate::reading::check_names::Answer;
use crate::theme::Tokens;

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

/// A send held for the fields' names: the command's parameters, and since when (the clock
/// restarts while a choice dialog is open).
struct SendWait {
    params: Value,
    since: f64,
}

/// What a send held comes to.
enum SendState {
    /// Every entry is a chip: it can go.
    Ready,
    /// Names wait on the directory, lists on their members, or a name on its choice.
    Waiting,
    /// An entry nothing can send to, and why.
    Stopped(String),
    /// An entry left as typed with nothing to wait for, by the user's own doing (the choice
    /// dialog cancelled): the send is off, with nothing to say.
    Cancelled,
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
    /// A list expanded here had more members than the field shows: what's shown isn't everyone,
    /// and Send stops until the field is emptied or its text set anew.
    partial: bool,
    /// The owner asked for what the next write brings (Expand, a choice): that write is theirs,
    /// not the field spelling what it knows.
    user_changed: bool,
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

/// Whether two entries are the same list: by item id for a contact group (no address), by address
/// otherwise.
fn same_list(a: &Addressee, b: &Addressee) -> bool {
    if a.address.is_empty() || b.address.is_empty() {
        !a.item_id.is_empty() && a.item_id == b.item_id
    } else {
        a.address.eq_ignore_ascii_case(&b.address)
    }
}

/// A contact group's chip: it has no address, only its members have.
fn is_group(who: &Addressee) -> bool {
    who.kind == AddresseeKind::ContactGroup && who.address.is_empty()
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
        // A text set from outside whole (a script, Select Names' OK) starts afresh, its losses
        // with it; the engine writing a name found keeps every entry, and the losses stay.
        let replaced = !was_reparse && pieces.len() != self.st.pieces.len();
        if replaced {
            self.forget_losses();
            self.st.left_out = (cut > 0).then_some(cut);
        } else if cut > 0 {
            self.st.left_out = Some(cut);
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
        if !self.expanding.is_empty()
            && let Some(dir) = s.directory_account(account)
        {
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
            self.partial |= x.more || more > 0;
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
        if cut > 0 {
            self.st.left_out = Some(self.st.left_out.unwrap_or(0).saturating_add(cut));
        }
        if !self.partial {
            self.note = None;
        }
        self.reparse = true;
    }

    /// The field lost recipients (entries past its cap, a list expanded in part): it stays
    /// stopped until the loss is undone — the field emptied, or its text set anew.
    fn forget_losses(&mut self) {
        self.st.left_out = None;
        self.partial = false;
        self.note = None;
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

    /// Whether a piece is left to check: text with no error, or one the directory couldn't be
    /// asked for (asked again, as the directory may be back).
    fn needs_resolve(&self) -> bool {
        self.st.pieces.iter().any(|p| matches!(p, Piece::Text { error: None | Some(PieceError::Unreached), .. }))
    }

    /// The pieces left to check, as a dialog's row hands them in (what's settled isn't asked again).
    fn unresolved_text(&self) -> String {
        let parts: Vec<&str> = self
            .st
            .pieces
            .iter()
            .filter_map(|p| match p {
                Piece::Text { text, error: None | Some(PieceError::Unreached) } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        parts.join("; ")
    }

    /// The errors of the pieces about to be asked again go: a name pending again isn't stopped on
    /// what the directory said last time (the answer puts them back if nothing changed).
    fn clear_errors(&mut self, kinds: &[PieceError]) {
        for p in &mut self.st.pieces {
            if let Piece::Text { error: error @ Some(_), .. } = p
                && error.as_ref().is_some_and(|e| kinds.contains(e))
            {
                *error = None;
            }
        }
    }

    fn add_chip(&mut self, who: Addressee) {
        if self.st.pieces.len() < MAX_ENTRIES {
            self.st.pieces.push(Piece::chip(who));
        } else {
            self.st.left_out = Some(self.st.left_out.unwrap_or(0).saturating_add(1));
        }
        if !self.partial {
            self.note = None;
        }
    }

    /// A contact group's members asked for (once).
    fn expand(&mut self, s: &mut Session, account: Option<&str>, who: &Addressee) {
        if self.expanding.iter().any(|w| same_list(w, who)) {
            return;
        }
        match s.people_expand(account, who) {
            Ok(_) => self.expanding.push(who.clone()),
            Err(e) => self.note = Some(tf!("This list couldn't be expanded: {error}", error = e.to_string())),
        }
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

    /// A message's own field brought up to date with the engine's text (parsed when it changed,
    /// the names settled taken, the lists expanded), and the text written back when the pieces
    /// spell it otherwise.
    fn sync_compose(&mut self, s: &mut Session, field: FieldId, account: &str) {
        let Some(text) = compose_text(s, field).cloned() else { return };
        self.prepare_with(s, field, Some(account), &Accepts::RECIPIENTS, Some(&text));
        self.write_back(s, field);
    }

    fn write_back(&mut self, s: &mut Session, field: FieldId) {
        let Some(f) = self.fields.get_mut(&field) else { return };
        if !f.dirty {
            return;
        }
        if let Some(t) = compose_text_mut(s, field) {
            *t = text_of(&f.st.pieces, &f.st.input);
            f.st.synced.clone_from(t);
            f.dirty = false;
        }
    }

    /// `compose.send` asked: what's typed is closed, every entry of the message's four fields is
    /// checked (an address here alone; a name in the directory, once), and a contact group's
    /// members are asked for. `false`: nothing waits and nothing is wrong, send now. `true`: the
    /// send is held ([`PeopleFields::send_when_resolved`] says when it goes, or why it can't).
    pub(crate) fn gate_send(&mut self, s: &mut Session, params: &Value, now: f64) -> bool {
        let Some((id, account)) = s.compose.as_ref().filter(|c| !c.sending).map(|c| (c.id, c.account.clone())) else { return false };
        self.send_wait.remove(&id);
        self.cancelled.retain(|(f, _)| f.window != id);
        for name in COMPOSE_FIELDS {
            let field = FieldId { window: id, name };
            if self.fields.get_mut(&field).is_some_and(Field::close_input) {
                if let Some(f) = self.fields.get_mut(&field) {
                    f.dirty = true;
                }
                self.write_back(s, field);
            }
            self.sync_compose(s, field, &account);
            // Every name is asked again (an answer kept settles it at once): what the directory
            // said last time doesn't stop a name it is being asked for now.
            if let Some(f) = self.fields.get_mut(&field) {
                f.clear_errors(&[PieceError::NotFound, PieceError::Unreached]);
            }
            let _ = s.people_resolve(field, Some(&account), &Accepts::RECIPIENTS, None);
            self.sync_compose(s, field, &account);
            let groups: Vec<Addressee> = self
                .pieces(field)
                .iter()
                .filter_map(|p| match p {
                    Piece::Chip { who, refused: None } if is_group(who) => Some(who.clone()),
                    _ => None,
                })
                .collect();
            if let Some(f) = self.fields.get_mut(&field) {
                for who in &groups {
                    f.expand(s, Some(&account), who);
                }
            }
        }
        match self.send_state(s, id) {
            SendState::Ready => false,
            _ => {
                self.send_wait.insert(id, SendWait { params: params.clone(), since: now });
                true
            }
        }
    }

    /// Each frame while sends are held: for each message whose wait concluded, `Ok(params)` once
    /// every name is settled and every group expanded, `Err(why)` when an entry can't be sent to
    /// or the directory hasn't answered within [`SEND_WAIT_SECS`] (the clock doesn't run while a
    /// choice dialog is open). A wait cancelled by the user concludes with nothing to say.
    pub(crate) fn send_when_resolved(&mut self, s: &mut Session, now: f64) -> Vec<(u64, Result<Value, String>)> {
        let mut out = Vec::new();
        let ids: Vec<u64> = self.send_wait.keys().copied().collect();
        for id in ids {
            let Some(account) = s.all_composes().find(|c| c.id == id).map(|c| c.account.clone()) else {
                self.send_wait.remove(&id);
                continue;
            };
            for name in COMPOSE_FIELDS {
                self.sync_compose(s, FieldId { window: id, name }, &account);
            }
            // A choice open: the user's time, not the directory's.
            let choosing = COMPOSE_FIELDS.iter().any(|name| s.name_choice_for(FieldId { window: id, name }).is_some());
            let Some(w) = self.send_wait.get_mut(&id) else { continue };
            if choosing {
                w.since = now;
            }
            let since = w.since;
            let done = match self.send_state(s, id) {
                SendState::Ready => self.send_wait.remove(&id).map(|w| Ok(w.params)),
                SendState::Waiting if now - since >= SEND_WAIT_SECS => {
                    self.send_wait.remove(&id);
                    self.mark_unreached(s, id);
                    Some(Err(tl!("The directory can't be reached").to_string()))
                }
                SendState::Waiting => None,
                SendState::Stopped(why) => {
                    self.send_wait.remove(&id);
                    Some(Err(why))
                }
                SendState::Cancelled => {
                    self.send_wait.remove(&id);
                    None
                }
            };
            if !self.send_wait.contains_key(&id) {
                self.cancelled.retain(|(f, _)| f.window != id);
            }
            if let Some(r) = done {
                out.push((id, r));
            }
        }
        out
    }

    /// How long the held send nearest its end has left before it gives up.
    pub(crate) fn send_wait_left(&self, now: f64) -> Option<f64> {
        self.send_wait.values().map(|w| w.since + SEND_WAIT_SECS - now).min_by(f64::total_cmp)
    }

    /// Where a held send stands. A piece left as typed is settled by nothing but the engine: it
    /// waits while a name of its field waits on the directory or on its choice, whatever the
    /// pending count alone would say; with nothing to wait for, the send is off: quietly when the
    /// user cancelled its choice, with a word otherwise (a choice evicted, a name a command left).
    fn send_state(&self, s: &Session, id: u64) -> SendState {
        let mut waiting = false;
        for name in COMPOSE_FIELDS {
            let field = FieldId { window: id, name };
            let Some(f) = self.fields.get(&field) else { continue };
            let pending = s.people_pending(field) > 0 || s.name_choice_for(field).is_some();
            waiting |= pending || !f.expanding.is_empty();
            // The field shows at most 500 entries; a message never goes to part of what was meant.
            if f.st.left_out.is_some() || f.partial {
                return SendState::Stopped(tl!("Some recipients were left out: send to fewer at once, or to the list itself").to_string());
            }
            for p in &f.st.pieces {
                match p {
                    Piece::Chip { refused: Some(why), .. } => return SendState::Stopped(why.clone()),
                    Piece::Chip { who, .. } if is_group(who) && !f.expanding.iter().any(|w| same_list(w, who)) => {
                        return SendState::Stopped(f.note.clone().unwrap_or_else(|| tl!("This list has no members").to_string()));
                    }
                    Piece::Chip { .. } => {}
                    Piece::Text { text, error: Some(e) } => return SendState::Stopped(error_text(text, e)),
                    Piece::Text { error: None, .. } if pending => {}
                    Piece::Text { text, error: None } => {
                        return if self.cancelled.iter().any(|(f, t)| *f == field && t == text) {
                            SendState::Cancelled
                        } else {
                            SendState::Stopped(tl!("Some names need checking before sending").to_string())
                        };
                    }
                }
            }
        }
        if waiting { SendState::Waiting } else { SendState::Ready }
    }

    /// The names still waiting when the send gave up: said unreached under the field.
    fn mark_unreached(&mut self, s: &Session, id: u64) {
        for name in COMPOSE_FIELDS {
            let field = FieldId { window: id, name };
            if s.people_pending(field) == 0 {
                continue;
            }
            if let Some(f) = self.fields.get_mut(&field) {
                for p in &mut f.st.pieces {
                    if let Piece::Text { error: error @ None, .. } = p {
                        *error = Some(PieceError::Unreached);
                    }
                }
            }
        }
    }
}
