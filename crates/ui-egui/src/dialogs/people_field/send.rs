//! Send and the contacts fields: a send held while its fields' names are resolved and its
//! contact groups expanded ([`PeopleFields::gate_send`]), and what it comes to each frame
//! ([`PeopleFields::send_when_resolved`]).

use quatrelle_engine::model::Addressee;
use quatrelle_engine::model::addr::Accepts;
use quatrelle_engine::{FieldId, Session};
use serde_json::Value;

use super::field::{Field, is_group, same_list};
use super::{COMPOSE_FIELDS, PeopleFields, SEND_WAIT_SECS, compose_text, compose_text_mut};
use crate::look::people_field::{Piece, PieceError, error_text, text_of};

/// A send held for the fields' names: the command's parameters, and since when (the clock
/// restarts while a choice dialog is open).
pub(super) struct SendWait {
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

impl PeopleFields {
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
