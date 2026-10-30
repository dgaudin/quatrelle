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
    /// The accounts whose room set comes from the server's lists (a failed ask leaves it as it
    /// is), not from the calendar alone.
    pub(crate) rooms_from_server: HashSet<String>,
    /// The accounts with an ask out, and when it went: one at a time, whoever asks (an ask
    /// unanswered for [`ROOMS_RETRY`] is given up).
    pub(crate) rooms_out: HashMap<String, i64>,
}

impl Session {
    /// Replace an account's room set with these rooms (`from_server`: the server's lists gave
    /// them); what the book answers changed.
    pub(crate) fn set_room_set(&mut self, account: &str, rooms: &[Room], from_server: bool) {
        let set: HashMap<String, String> =
            rooms.iter().map(|r| (r.email.trim().to_ascii_lowercase(), r.name.clone())).filter(|(e, _)| !e.is_empty()).collect();
        self.people.rooms.insert(account.to_string(), set);
        if from_server {
            self.people.rooms_from_server.insert(account.to_string());
        } else {
            self.people.rooms_from_server.remove(account);
        }
        self.contacts.changed();
    }

    /// An account is gone: so are its rooms.
    pub(crate) fn forget_rooms(&mut self, account: &str) {
        let had = self.people.rooms.remove(account).is_some();
        self.people.rooms_asked.remove(account);
        self.people.rooms_from_server.remove(account);
        self.people.rooms_out.remove(account);
        self.rooms.remove(account);
        if had {
            self.contacts.changed();
        }
    }

    /// Ask the server for an account's room lists, unless an ask is already out.
    pub(crate) fn ask_rooms(&mut self, account: &str) {
        let now = self.clock.now;
        if self.people.rooms_out.get(account).is_some_and(|&at| now.saturating_sub(at) < ROOMS_RETRY) {
            return;
        }
        self.people.rooms_out.insert(account.to_string(), now);
        self.jobs.push(Job::GetRooms { account: account.to_string() });
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
