//! One contacts field's state beside the widget's ([`Field`]): its text parsed into pieces, what
//! the engine settled taken, the lists expanded, the losses kept (entries past the cap, a list
//! expanded in part).

use quatrelle_engine::model::addr::{MAX_ENTRIES, group_ref, parse_one, split_entries};
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
    /// while a key was typed). The text is written back when the pieces spell it otherwise.
    pub(super) fn parse(&mut self, s: &mut Session, account: Option<&str>, text: &str) {
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
    pub(super) fn refresh(&mut self, s: &mut Session, field: FieldId, account: Option<&str>) {
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
    pub(super) fn close(&mut self, closed: &str) {
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
    pub(super) fn forget_losses(&mut self) {
        self.st.left_out = None;
        self.partial = false;
        self.note = None;
    }

    /// What's being typed, closed as an entry (Ctrl+K, Send).
    pub(super) fn close_input(&mut self) -> bool {
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

    pub(super) fn add_chip(&mut self, who: Addressee) {
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
