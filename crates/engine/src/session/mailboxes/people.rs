//! The session: what an address is. Exchange's directory cannot tell a room from a person in
//! its answers, so the room lists (already loaded for the Room Finder) are the source of truth
//! for "this address is a room": they are asked as soon as an account is online, and again
//! once a day, quietly, without disturbing a Room Finder that is open.

use quatrelle_model::AddresseeKind;
use quatrelle_model::scheduling::Room;

use super::*;

/// The room lists are asked again after this long (seconds).
pub const ROOMS_EVERY: i64 = 86_400;

/// A failed ask is tried again after this long (seconds).
pub const ROOMS_RETRY: i64 = 600;

/// What the session knows of the people and places addresses stand for.
#[derive(Default)]
pub(crate) struct People {
    /// Each account's rooms: the address (lower case) → the name.
    pub(crate) rooms: HashMap<String, HashMap<String, String>>,
    /// When each account's room lists were last asked or answered, and whether that failed.
    pub(crate) rooms_asked: HashMap<String, (i64, bool)>,
}

impl Session {
    /// Replace an account's room set with these rooms; what the book answers changed.
    pub(crate) fn set_room_set(&mut self, account: &str, rooms: &[Room]) {
        let set: HashMap<String, String> =
            rooms.iter().map(|r| (r.email.trim().to_ascii_lowercase(), r.name.clone())).filter(|(e, _)| !e.is_empty()).collect();
        self.people.rooms.insert(account.to_string(), set);
        self.contacts.changed();
    }

    /// The name of a room, when the address is one of an account's rooms.
    pub fn room_name(&self, account: &str, email: &str) -> Option<&str> {
        let email = email.trim().to_ascii_lowercase();
        self.people.rooms.get(account)?.get(&email).map(String::as_str)
    }

    /// What an address stands for: a room (the room lists), a shared mailbox opened by an
    /// account, a person written to or heard from, or nothing known. `account` narrows the
    /// rooms to those of one account.
    pub fn addressee_kind(&self, account: Option<&str>, email: &str) -> AddresseeKind {
        let email = email.trim().to_ascii_lowercase();
        let is_room = match account {
            Some(a) => self.people.rooms.get(a).is_some_and(|r| r.contains_key(&email)),
            None => self.people.rooms.values().any(|r| r.contains_key(&email)),
        };
        if is_room {
            AddresseeKind::Room
        } else if self.accounts.iter().any(|a| a.config.shared_mailboxes.iter().any(|m| m.eq_ignore_ascii_case(&email))) {
            AddresseeKind::SharedMailbox
        } else if self.known_contact(&email).is_some() {
            AddresseeKind::Person
        } else {
            AddresseeKind::Unknown
        }
    }
}
