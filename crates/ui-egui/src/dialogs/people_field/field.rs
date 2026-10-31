//! One contacts field's state beside the widget's ([`Field`]): its text parsed into pieces, what
//! the engine settled taken, the lists expanded, the losses kept (entries past the cap, a list
//! expanded in part).

use quatrelle_engine::model::addr::{Accepts, MAX_ENTRIES, group_ref, parse_one, split_entries};
use quatrelle_engine::model::{Addressee, AddresseeKind};
use quatrelle_engine::{Fetched, FieldId, Outcome, Session, Suggestion, group_entry};

use crate::look::people_field::{FieldState, Piece, PieceError};

/// One field: the widget's state, and what the glue keeps beside it.
#[derive(Default)]
pub(super) struct Field {
    pub(super) st: FieldState,
    /// The suggestions for the input (the engine's cache on an unchanged frame).
    pub(super) suggestions: Vec<Suggestion>,
    /// Names waiting on the directory, as of the last frame (the widget's spinner).
    pub(super) pending: usize,
    /// The lists whose members were asked for: each chip is replaced by them once they come.
    pub(super) expanding: Vec<Addressee>,
    /// A word under the field (how many members more, a list that couldn't be expanded), until
    /// the pieces next change.
    pub(super) note: Option<String>,
    /// Text the widget closed hasn't been parsed yet: the field's text is parsed at the next frame.
    pub(super) reparse: bool,
    /// The pieces changed without the widget (an expansion, a name found): the text is written
    /// back at the next draw.
    pub(super) dirty: bool,
    /// The book's revision the chips' kinds were read at, and whether it was still being read.
    pub(super) kinds_at: Option<(u64, bool)>,
    /// A list expanded here had more members than the field shows: what's shown isn't everyone,
    /// and Send stops until the field is emptied or its text set anew.
    pub(super) partial: bool,
    /// The owner asked for what the next write brings (Expand, a choice): that write is theirs,
    /// not the field spelling what it knows.
    pub(super) user_changed: bool,
    /// The rooms and equipment entered among a meeting's guests, taken out of the pieces for its
    /// Rooms to book (`PeopleFields::take_moved`).
    pub(super) moved: Vec<Addressee>,
}

/// Whether two entries are the same list: by item id for a contact group (no address), by address
/// otherwise.
pub(super) fn same_list(a: &Addressee, b: &Addressee) -> bool {
    if a.address.is_empty() || b.address.is_empty() {
        !a.item_id.is_empty() && a.item_id == b.item_id
    } else {
        a.address.eq_ignore_ascii_case(&b.address)
    }
}

/// A contact group's chip: it has no address, only its members have.
pub(super) fn is_group(who: &Addressee) -> bool {
    who.kind == AddresseeKind::ContactGroup && who.address.is_empty()
}

/// Why a field's place doesn't take an entry, or `None` when it does: a kind it takes, or one it
/// takes only typed in full when the entry was (`by_name`: picked by name — a suggestion, a book,
/// a name settled — or read back from a chip picked so). A place with no words of its own yet is
/// refused in the attendees' words: an entry is never taken for want of a sentence.
fn refused_by(accepts: &Accepts, who: &Addressee, by_name: bool) -> Option<String> {
    if accepts.takes(who.kind) || (!by_name && accepts.typed.contains(&who.kind)) {
        return None;
    }
    let why = if *accepts == Accepts::ROOMS {
        tl!("This isn't a room")
    } else if *accepts == Accepts::ONE_SENDER {
        tl!("Send from a person, a shared mailbox or a list")
    } else {
        tl!("Only people, lists and groups can be invited")
    };
    Some(why.to_string())
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

impl Field {
    /// Parse the field's text into pieces, once (`contacts_step(0)` first, so that the book names
    /// the people known here). What Check Names said of a piece stays said while the piece stays;
    /// what is being typed stays typed when the text ends in it (the engine wrote the text back
    /// while a key was typed). The text is written back when the pieces spell it otherwise. A
    /// chip the place refuses is red; one refused stays so while a chip of its address stays (a
    /// person picked by name in Rooms reads back from the text as if typed), with what it was
    /// known to be while nothing here knows it (unknown is no reason to take it).
    pub(super) fn parse(&mut self, s: &mut Session, account: Option<&str>, accepts: &Accepts, text: &str) {
        s.contacts_step(0);
        let was_reparse = std::mem::take(&mut self.reparse);
        let entries = split_entries(text).len();
        let mut pieces = pieces_of(s, account, text);
        let refused_before: Vec<&Addressee> = self
            .st
            .pieces
            .iter()
            .filter_map(|p| match p {
                Piece::Chip { who, refused: Some(_) } => Some(who),
                _ => None,
            })
            .collect();
        for p in &mut pieces {
            match p {
                Piece::Text { text, error: error @ None } => {
                    *error = self.st.pieces.iter().find_map(|old| match old {
                        Piece::Text { text: t, error: Some(e) } if t == text => Some(e.clone()),
                        _ => None,
                    });
                }
                Piece::Chip { who, refused } => {
                    let before = refused_before.iter().find(|w| same_list(w, who));
                    if let Some(old) = before
                        && who.kind == AddresseeKind::Unknown
                    {
                        who.kind = old.kind;
                    }
                    *refused = refused_by(accepts, who, before.is_some());
                }
                Piece::Text { .. } => {}
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
        self.move_rooms(accepts);
    }

    /// What the engine has for the field since the last frame: the names settled (a chip, or why
    /// not), the lists expanded, the chips' kinds from the book once it's read, the names waiting.
    pub(super) fn refresh(&mut self, s: &mut Session, field: FieldId, account: Option<&str>, accepts: &Accepts) {
        for (token, outcome) in s.take_people_outcomes(field) {
            let error = match outcome {
                Outcome::Found(who) => {
                    self.found(&token, who, accepts);
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
            self.settle_expansions(s, &dir, accepts);
        }
        let (revision, reading) = (s.contacts_revision(), s.contacts_pending());
        if !reading && self.kinds_at != Some((revision, reading)) {
            self.refresh_kinds(s, account, accepts);
            self.kinds_at = Some((revision, reading));
        }
        self.pending = s.people_pending(field);
    }

    /// The lists whose members came: each chip replaced by them (the field's cap kept), with how
    /// many more the list has; an empty list, or one that couldn't be expanded, is said and its
    /// chip stays. A member is an entry like any other: refused when the place doesn't take its
    /// kind, a room among a meeting's guests moved.
    fn settle_expansions(&mut self, s: &Session, dir: &str, accepts: &Accepts) {
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
                // The question is gone (its account removed, the answers kept trimmed): said, not
                // taken for a list with no members.
                None => {
                    self.note = Some(tl!("This list is no longer available").to_string());
                    None
                }
            };
            let who = self.expanding.remove(i);
            let Some(x) = ready else { continue };
            if x.members.is_empty() {
                self.note = Some(tl!("This list has no members").to_string());
                continue;
            }
            if let Some(at) = self.st.pieces.iter().position(|p| matches!(p, Piece::Chip { who: w, .. } if same_list(w, &who))) {
                let members = x.members.iter().cloned().map(|who| {
                    let refused = refused_by(accepts, &who, false);
                    Piece::Chip { who, refused }
                });
                self.st.pieces.splice(at..=at, members);
                let over = self.st.pieces.len().saturating_sub(MAX_ENTRIES);
                if over > 0 {
                    self.st.pieces.truncate(MAX_ENTRIES);
                    self.st.left_out = Some(over);
                }
                self.st.selected = None;
                self.st.card = None;
                self.dirty = true;
                self.move_rooms(accepts);
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

    /// The chips' kinds and names from what is known now (the book read, a room list come), and
    /// what the place makes of them: a chip refused, or a room among a meeting's guests moved.
    fn refresh_kinds(&mut self, s: &Session, account: Option<&str>, accepts: &Accepts) {
        for p in &mut self.st.pieces {
            if let Piece::Chip { who, refused } = p
                && !who.address.is_empty()
            {
                let known = s.known_addressee(account, who.email());
                self.dirty |= known.name != who.name;
                who.name = known.name;
                // Refused, it was picked by name: it stays so while its place doesn't take what it
                // is, and what it was known to be holds while nothing here knows it.
                if refused.is_none() || known.kind != AddresseeKind::Unknown {
                    who.kind = known.kind;
                }
                *refused = refused_by(accepts, who, refused.is_some());
            }
        }
        self.move_rooms(accepts);
    }

    /// A name settled (`token` as typed): its text becomes the chip, picked by name unless the
    /// token was an address; in a one-entry field it takes the place of what was there.
    fn found(&mut self, token: &str, who: Addressee, accepts: &Accepts) {
        if !self.st.pieces.iter().any(|p| matches!(p, Piece::Text { text, .. } if text == token)) {
            return;
        }
        let refused = refused_by(accepts, &who, !token.contains('@'));
        if accepts.one {
            self.clear_for_one();
            self.st.pieces.push(Piece::Chip { who, refused });
        } else {
            for p in &mut self.st.pieces {
                if matches!(p, Piece::Text { text, .. } if text == token) {
                    *p = Piece::Chip { who: who.clone(), refused: refused.clone() };
                }
            }
        }
        self.dirty = true;
        self.move_rooms(accepts);
    }

    /// A one-entry field's new entry takes the place of everything it had: those go, and aren't
    /// left out.
    fn clear_for_one(&mut self) {
        self.st.pieces.clear();
        self.st.selected = None;
        self.st.card = None;
        self.st.left_out = None;
        self.partial = false;
        self.expanding.clear();
    }

    /// Among a meeting's guests (a field taking [`Accepts::ATTENDEES`]), a room or equipment is no
    /// guest: taken out of the pieces for its Rooms to book, the text written back without it.
    fn move_rooms(&mut self, accepts: &Accepts) {
        let is_room = |who: &Addressee| matches!(who.kind, AddresseeKind::Room | AddresseeKind::Equipment);
        if *accepts != Accepts::ATTENDEES || !self.st.pieces.iter().any(|p| matches!(p, Piece::Chip { who, .. } if is_room(who))) {
            return;
        }
        let moved = &mut self.moved;
        self.st.pieces.retain(|p| match p {
            Piece::Chip { who, .. } if is_room(who) => {
                if moved.len() < MAX_ENTRIES && !moved.iter().any(|m| same_list(m, who)) {
                    moved.push(who.clone());
                }
                false
            }
            _ => true,
        });
        self.st.selected = None;
        self.st.card = None;
        self.dirty = true;
    }

    /// Text the widget closed (`;`, a paste, the input left): entries as typed, up to the field's
    /// cap, parsed at the next frame. A one-entry field keeps the last one closed, in place of
    /// what it had.
    pub(super) fn close(&mut self, closed: &str, accepts: &Accepts) {
        let entries = split_entries(closed);
        if accepts.one {
            if let Some(last) = entries.last() {
                self.clear_for_one();
                self.st.pieces.push(Piece::Text { text: (*last).to_string(), error: None });
            }
        } else {
            let room = MAX_ENTRIES.saturating_sub(self.st.pieces.len());
            let cut = entries.len().saturating_sub(room);
            self.st.pieces.extend(entries.iter().take(room).map(|e| Piece::Text { text: (*e).to_string(), error: None }));
            if cut > 0 {
                self.st.left_out = Some(self.st.left_out.unwrap_or(0).saturating_add(cut));
            }
        }
        if !self.partial {
            self.note = None;
        }
        self.reparse = true;
    }

    /// The field lost recipients (entries past its cap, a list expanded in part): it stays
    /// stopped until the loss is undone — the field emptied, or its text set anew.
    pub(super) fn forget_losses(&mut self) {
        self.st.left_out = None;
        self.partial = false;
        self.note = None;
    }

    /// Whether the field lost recipients: entries past its cap, or a list expanded in part.
    pub(super) fn has_losses(&self) -> bool {
        self.st.left_out.is_some() || self.partial
    }

    /// What's being typed, closed as an entry (Ctrl+K, Send).
    pub(super) fn close_input(&mut self, accepts: &Accepts) -> bool {
        let typed = self.st.input.trim().to_string();
        if typed.is_empty() {
            return false;
        }
        self.close(&typed, accepts);
        self.st.input.clear();
        self.st.open = false;
        true
    }

    /// Whether a piece is left to check: text with no error, or one the directory couldn't be
    /// asked for (asked again, as the directory may be back).
    pub(super) fn needs_resolve(&self) -> bool {
        self.st.pieces.iter().any(|p| matches!(p, Piece::Text { error: None | Some(PieceError::Unreached), .. }))
    }

    /// The pieces left to check, as a dialog's row hands them in (what's settled isn't asked again).
    pub(super) fn unresolved_text(&self) -> String {
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
    pub(super) fn clear_errors(&mut self, kinds: &[PieceError]) {
        for p in &mut self.st.pieces {
            if let Piece::Text { error: error @ Some(_), .. } = p
                && error.as_ref().is_some_and(|e| kinds.contains(e))
            {
                *error = None;
            }
        }
    }

    /// A chip picked by name (a suggestion taken, a book's button): red when the place doesn't
    /// take it so, in place of everything in a one-entry field.
    pub(super) fn add_chip(&mut self, who: Addressee, accepts: &Accepts) {
        let refused = refused_by(accepts, &who, true);
        if accepts.one {
            self.clear_for_one();
        }
        if self.st.pieces.len() < MAX_ENTRIES {
            self.st.pieces.push(Piece::Chip { who, refused });
        } else {
            self.st.left_out = Some(self.st.left_out.unwrap_or(0).saturating_add(1));
        }
        if !self.partial {
            self.note = None;
        }
        self.move_rooms(accepts);
    }

    /// A contact group's members asked for (once).
    pub(super) fn expand(&mut self, s: &mut Session, account: Option<&str>, who: &Addressee) {
        if self.expanding.iter().any(|w| same_list(w, who)) {
            return;
        }
        match s.people_expand(account, who) {
            Ok(_) => self.expanding.push(who.clone()),
            Err(e) => self.note = Some(tf!("This list couldn't be expanded: {error}", error = e.to_string())),
        }
    }
}
