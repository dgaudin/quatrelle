//! Send and the contacts fields: a send held while its fields' names are resolved and its
//! contact groups expanded — a message's ([`PeopleFields::gate_send`]) or a meeting's
//! ([`PeopleFields::gate_meeting`]) — and what it comes to each frame
//! ([`PeopleFields::send_when_resolved`]).

use quatrelle_engine::model::Addressee;
use quatrelle_engine::{FieldId, Session};
use serde_json::Value;

use super::field::{is_group, same_list};
use super::{
    COMPOSE_FIELDS, MEETING_FIELDS, MEETING_WINDOWS, PeopleFields, SEND_WAIT_SECS, accepts_of, compose_text, compose_text_mut, is_meeting_window,
};
use crate::look::people_field::{Piece, PieceError, error_text, text_of};

/// A send held for the fields' names: the command's parameters (none for a meeting's: its caller
/// builds the command from the form once it can go), since when (the clock restarts while a
/// choice dialog is open), and the window's fields it waits on.
pub(super) struct SendWait {
    params: Value,
    since: f64,
    fields: &'static [&'static str],
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

/// The stop for a field that lost recipients (entries past its cap, a list expanded in part), and
/// what to do about it: the loss stays until the field is emptied or set anew.
pub(crate) fn left_out_stop() -> String {
    tl!("Some recipients were left out. Empty the field, then add fewer at once or the list itself").to_string()
}

impl PeopleFields {
    /// A message's own field brought up to date with the engine's text (parsed when it changed,
    /// the names settled taken, the lists expanded), and the text written back when the pieces
    /// spell it otherwise.
    fn sync_compose(&mut self, s: &mut Session, field: FieldId, account: &str) {
        let Some(text) = compose_text(s, field).cloned() else { return };
        self.prepare_with(s, field, Some(account), accepts_of(field.name), Some(&text));
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

    /// Each contact group of the field asked for its members (a send needs their addresses).
    fn expand_groups(&mut self, s: &mut Session, field: FieldId, account: &str) {
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
                f.expand(s, Some(account), who);
            }
        }
    }

    /// The send held for the window's `fields`, unless it can go now (`false`).
    fn hold(&mut self, s: &Session, window: u64, params: Value, fields: &'static [&'static str], now: f64) -> bool {
        match self.send_state(s, window, fields) {
            SendState::Ready => false,
            _ => {
                self.send_wait.insert(window, SendWait { params, since: now, fields });
                true
            }
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
            let accepts = accepts_of(name);
            if self.fields.get_mut(&field).is_some_and(|f| f.close_input(accepts)) {
                if let Some(f) = self.fields.get_mut(&field) {
                    f.dirty = true;
                }
                self.write_back(s, field);
            }
            self.sync_compose(s, field, &account);
            // Every name is asked again (an answer kept settles it at once): what the directory
            // said last time doesn't stop a name it is being asked for now.
            if let Some(f) = self.fields.get_mut(&field) {
                f.clear_errors(|e| matches!(e, PieceError::NotFound | PieceError::Unreached));
            }
            let _ = s.people_resolve(field, Some(&account), accepts, None);
            self.sync_compose(s, field, &account);
            self.expand_groups(s, field, &account);
        }
        self.hold(s, id, params.clone(), &COMPOSE_FIELDS, now)
    }

    /// Each frame while sends are held: for each window whose wait concluded, `Ok(params)` once
    /// every name is settled and every group expanded, `Err(why)` when an entry can't be sent to
    /// or the directory hasn't answered within [`SEND_WAIT_SECS`] (the clock doesn't run while a
    /// choice dialog is open). A wait cancelled by the user concludes with nothing to say.
    pub(crate) fn send_when_resolved(&mut self, s: &mut Session, now: f64) -> Vec<(u64, Result<Value, String>)> {
        let mut out = Vec::new();
        let windows: Vec<u64> = self.send_wait.keys().copied().collect();
        for window in windows {
            let Some(fields) = self.send_wait.get(&window).map(|w| w.fields) else { continue };
            // A message's fields are its text in the engine, brought up to date here; a meeting's
            // are its form's, made ready by its window each frame.
            if !is_meeting_window(window) {
                let Some(account) = s.all_composes().find(|c| c.id == window).map(|c| c.account.clone()) else {
                    self.send_wait.remove(&window);
                    continue;
                };
                for &name in fields {
                    self.sync_compose(s, FieldId { window, name }, &account);
                }
            }
            // A choice open: the user's time, not the directory's.
            let choosing = fields.iter().any(|&name| s.name_choice_for(FieldId { window, name }).is_some());
            let Some(w) = self.send_wait.get_mut(&window) else { continue };
            if choosing {
                w.since = now;
            }
            let since = w.since;
            let done = match self.send_state(s, window, fields) {
                SendState::Ready => self.send_wait.remove(&window).map(|w| Ok(w.params)),
                SendState::Waiting if now - since >= SEND_WAIT_SECS => {
                    self.send_wait.remove(&window);
                    self.mark_unreached(s, window, fields);
                    Some(Err(tl!("The directory can't be reached").to_string()))
                }
                SendState::Waiting => None,
                SendState::Stopped(why) => {
                    self.send_wait.remove(&window);
                    Some(Err(why))
                }
                SendState::Cancelled => {
                    self.send_wait.remove(&window);
                    None
                }
            };
            if !self.send_wait.contains_key(&window) {
                self.cancelled.retain(|(f, _)| f.window != window);
            }
            if let Some(r) = done {
                out.push((window, r));
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
    fn send_state(&self, s: &Session, window: u64, fields: &[&'static str]) -> SendState {
        let mut waiting = false;
        for &name in fields {
            let field = FieldId { window, name };
            let Some(f) = self.fields.get(&field) else { continue };
            let pending = s.people_pending(field) > 0 || s.name_choice_for(field).is_some();
            waiting |= pending || !f.expanding.is_empty();
            // The field shows at most 500 entries; nothing goes to part of what was meant.
            if f.has_losses() {
                return SendState::Stopped(left_out_stop());
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
    fn mark_unreached(&mut self, s: &Session, window: u64, fields: &[&'static str]) {
        for &name in fields {
            let field = FieldId { window, name };
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

/// A meeting's Send, held as a message's is: its lines are its form's text, handed in.
impl PeopleFields {
    /// A meeting's line written as its field's pieces spell it, when they changed without the
    /// widget (what's typed closed, a name settled, a room taken out or kept): the form's text
    /// then, the line drawn or not (the Scheduling Assistant draws none). The field knows it
    /// wrote it, so it isn't parsed again (a room kept as typed would be taken out again).
    pub(crate) fn write_line(&mut self, field: FieldId, line: &mut String) {
        let Some(f) = self.fields.get_mut(&field) else { return };
        if f.dirty {
            *line = text_of(&f.st.pieces, &f.st.input);
            f.st.synced.clone_from(line);
            f.dirty = false;
        }
    }

    /// A meeting's Send asked (meeting window `wid`, its three [`MEETING_FIELDS`] `lines` as its
    /// form holds them): each line's typing closed and written back into it at once, the names
    /// left in it checked (handed in: a meeting's text is its form's, not the engine's), and each
    /// contact group's members asked for. `false`: nothing waits and nothing is wrong, send now.
    /// `true`: held; [`PeopleFields::send_when_resolved`] gives `(MEETING_WINDOWS | wid,
    /// Ok(Value::Null))` once it can go — the caller builds the command from the form then — or
    /// why it can't.
    pub(crate) fn gate_meeting(&mut self, s: &mut Session, wid: u64, account: &str, lines: [(&'static str, &mut String); 3], now: f64) -> bool {
        let window = MEETING_WINDOWS | wid;
        self.send_wait.remove(&window);
        self.cancelled.retain(|(f, _)| f.window != window);
        for (name, line) in lines {
            let field = FieldId { window, name };
            let accepts = accepts_of(name);
            // The line as the form holds it (parsed only when it changed), what's typed closed.
            self.prepare_with(s, field, Some(account), accepts, Some(line.as_str()));
            if let Some(f) = self.fields.get_mut(&field)
                && f.close_input(accepts)
            {
                f.dirty = true;
                self.write_line(field, line);
                self.prepare_with(s, field, Some(account), accepts, Some(line.as_str()));
            }
            let Some(f) = self.fields.get_mut(&field) else { continue };
            // Asked again, as a message's names are: last time's answer doesn't stop them now, nor
            // a room the meeting couldn't book then (one may have been freed).
            f.clear_errors(|e| matches!(e, PieceError::NotFound | PieceError::Unreached | PieceError::RoomsFull(_)));
            let unresolved = f.unresolved_text();
            if !unresolved.is_empty() {
                let _ = s.people_resolve(field, Some(account), accepts, Some(&unresolved));
                // What the check settled at once (an address, a name answered before), taken now.
                self.prepare_with(s, field, Some(account), accepts, None);
            }
            self.write_line(field, line);
            self.expand_groups(s, field, account);
        }
        self.hold(s, window, Value::Null, &MEETING_FIELDS, now)
    }

    /// Meeting window `wid`'s Send held on its fields as they stand, nothing closed or asked
    /// again (its lines just written after [`PeopleFields::gate_meeting`] let it go: a room Rooms
    /// couldn't book is back among the guests, and the send stops on it). `false`: it can go.
    pub(crate) fn hold_meeting(&mut self, s: &Session, wid: u64, now: f64) -> bool {
        self.hold(s, MEETING_WINDOWS | wid, Value::Null, &MEETING_FIELDS, now)
    }
}
