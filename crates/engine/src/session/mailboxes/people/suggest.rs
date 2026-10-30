//! The session: suggestions for a contacts field, from what is already on this computer (the
//! address book, the directory's answers kept, the room lists and the shared mailboxes opened).
//! The server is never asked while a name is typed.
//!
//! A suggestion pass looks at every entry of the book, the index and the rooms once, so it runs
//! only when a field's text (or what the book answers) changed: the window asks every frame, and
//! a frame where nothing was typed gets the field's last answer back, scanning nothing. A pass per
//! frame over a big mailbox is what froze the search on 2026-10-30.

use quatrelle_model::addr::Accepts;
use quatrelle_model::{Addressee, AddresseeKind};
use serde::Serialize;

use super::*;

/// Most suggestions offered.
pub const MAX_SUGGESTIONS: usize = 8;

/// The fields that ask for suggestions: the message's recipients, the address book's
/// (`book-*`), and the command line's.
pub const FIELD_NAMES: &[&str] = &["to", "cc", "bcc", "replyTo", "book-to", "book-cc", "book-bcc", "cli"];

/// The fields whose last answer is kept; past them, all are forgotten (a window closed takes
/// its fields with it, and nothing says so).
const CACHED_FIELDS: usize = 64;

/// The longest text looked for (characters).
const MAX_TEXT: usize = 256;

/// A contacts field: the window it is in and which field it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct FieldId {
    pub window: u64,
    pub name: &'static str,
}

impl FieldId {
    /// The field named `name` (one of [`FIELD_NAMES`]) in a window.
    pub fn parse(window: u64, name: &str) -> Option<FieldId> {
        FIELD_NAMES.iter().find(|n| **n == name).map(|&name| FieldId { window, name })
    }
}

/// Where a suggestion comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    /// Written to or heard from (and the shared mailboxes opened).
    Recent,
    /// Saved in Contacts, and no mail exchanged.
    Contacts,
    /// The directory's answers kept.
    Directory,
    /// The room lists.
    RoomList,
}

impl Source {
    /// The stable text of the source (the serde name).
    pub fn code(self) -> &'static str {
        match self {
            Source::Recent => "recent",
            Source::Contacts => "contacts",
            Source::Directory => "directory",
            Source::RoomList => "roomList",
        }
    }
}

/// One suggestion: who, and where they come from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Suggestion {
    pub who: Addressee,
    pub source: Source,
}

/// A field's last answer, and what it was worked out from.
pub(crate) struct Cached {
    text: String,
    account: Option<String>,
    kinds: Vec<AddresseeKind>,
    revision: u64,
    list: Vec<Suggestion>,
}

/// A candidate and what orders it: how well it fits, how much it's used, where its kind comes
/// in what the field takes, its address.
struct Hit {
    s: Suggestion,
    fit: u8,
    score: f64,
    place: usize,
    address: String,
}

impl Hit {
    fn new(who: Addressee, source: Source, score: f64, q: &str, kinds: &[AddresseeKind]) -> Hit {
        let place = kinds.iter().position(|k| *k == who.kind).unwrap_or(kinds.len());
        Hit { fit: fit(&who, q), score, place, address: who.address.to_lowercase(), s: Suggestion { who, source } }
    }
}

/// Whether a name or an address starts with `q` (lower case): the address, the name, or any word
/// of the name.
fn starts_with(name: &str, address: &str, q: &str) -> bool {
    let name = name.to_lowercase();
    address.to_lowercase().starts_with(q) || name.starts_with(q) || words(&name).any(|w| w.starts_with(q))
}

fn words(name: &str) -> impl Iterator<Item = &str> {
    name.split(|c: char| c.is_whitespace() || matches!(c, '-' | '.' | ',')).filter(|w| !w.is_empty())
}

/// How well a suggestion fits `q`: 0 an exact name or address, 1 the start of the first or the
/// last name or of the address, 2 anything else that matched.
fn fit(who: &Addressee, q: &str) -> u8 {
    let (name, address) = (who.name.trim().to_lowercase(), who.address.trim().to_lowercase());
    if name == q || address == q {
        return 0;
    }
    let mut w = words(&name);
    let first = w.next();
    let last = w.last().or(first);
    if address.starts_with(q) || first.is_some_and(|f| f.starts_with(q)) || last.is_some_and(|l| l.starts_with(q)) { 1 } else { 2 }
}

impl Session {
    /// Up to [`MAX_SUGGESTIONS`] people for a contacts field, best first, from what is on this
    /// computer: nothing for a blank text; the field's last answer when neither its text nor the
    /// book changed since; else one pass ([`Session::people_suggest_passes`]).
    pub fn people_suggest(&mut self, field: FieldId, account: Option<&str>, accepts: &Accepts, text: &str) -> Vec<Suggestion> {
        self.people_suggest_kinds(field, account, accepts.kinds, text)
    }

    /// [`Session::people_suggest`] for the kinds a field takes, as a command gives them.
    pub(crate) fn people_suggest_kinds(&mut self, field: FieldId, account: Option<&str>, kinds: &[AddresseeKind], text: &str) -> Vec<Suggestion> {
        let q: String = text.trim().chars().take(MAX_TEXT).collect::<String>().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let revision = self.contacts_revision();
        if let Some(c) = self.people.suggest_cache.get(&field)
            && c.text == q
            && c.revision == revision
            && c.account.as_deref() == account
            && c.kinds == kinds
        {
            return c.list.clone();
        }
        let list = self.suggestion_pass(account, kinds, &q);
        let cache = &mut self.people.suggest_cache;
        if cache.len() >= CACHED_FIELDS && !cache.contains_key(&field) {
            cache.clear();
        }
        // The revision after the pass: starting the book may have changed it.
        let revision = self.contacts_revision();
        let cached = Cached { text: q, account: account.map(str::to_string), kinds: kinds.to_vec(), revision, list: list.clone() };
        self.people.suggest_cache.insert(field, cached);
        list
    }

    /// The suggestion passes run, and the entries they looked at.
    pub fn people_suggest_passes(&self) -> (u64, u64) {
        (self.people.passes, self.people.scanned)
    }

    /// One pass: the book's best, the directory's answers, the rooms and the shared mailboxes
    /// that start with `q` (lower case), of the kinds taken, each address once, ordered, cut.
    fn suggestion_pass(&mut self, account: Option<&str>, kinds: &[AddresseeKind], q: &str) -> Vec<Suggestion> {
        let takes = |k: AddresseeKind| k == AddresseeKind::Unknown || kinds.contains(&k);
        let book = self.suggest_contacts_now(q, MAX_SUGGESTIONS * 4);
        let mut seen: HashSet<String> = HashSet::new();
        let mut hits: Vec<Hit> = Vec::new();
        let now = self.clock.now;
        for c in book {
            let kind = self.addressee_kind(account, &c.address.email);
            if takes(kind) && seen.insert(c.address.email.trim().to_ascii_lowercase()) {
                // Mail exchanged counts first: Contacts is for someone known from there alone.
                let source = if c.saved && c.sent == 0 && c.received == 0 { Source::Contacts } else { Source::Recent };
                let who = Addressee::new(c.address.name.clone(), c.address.email.clone(), kind);
                hits.push(Hit::new(who, source, c.score(now), q, kinds));
            }
        }
        for (key, a) in &self.people.index {
            if !starts_with(&a.name, &a.address, q) || seen.contains(key) {
                continue;
            }
            // The room lists, and the mailboxes opened, know better than the directory.
            let kind = if a.address.is_empty() { a.kind } else { self.addressee_kind(account, &a.address) };
            if takes(kind) {
                seen.insert(key.clone());
                hits.push(Hit::new(Addressee { kind, ..a.clone() }, Source::Directory, 0.0, q, kinds));
            }
        }
        let mut rooms_scanned = 0usize;
        let room_sets = self.people.rooms.iter().filter(|(a, _)| account.is_none_or(|x| x == a.as_str()));
        for (_, set) in room_sets {
            rooms_scanned = rooms_scanned.saturating_add(set.len());
            if !takes(AddresseeKind::Room) {
                continue;
            }
            for (address, name) in set {
                if starts_with(name, address, q) && seen.insert(address.clone()) {
                    let who = Addressee::new(name.clone(), address.clone(), AddresseeKind::Room);
                    hits.push(Hit::new(who, Source::RoomList, 0.0, q, kinds));
                }
            }
        }
        if takes(AddresseeKind::SharedMailbox) {
            for m in self.accounts.iter().flat_map(|a| &a.config.shared_mailboxes) {
                let address = m.trim().to_ascii_lowercase();
                if starts_with("", &address, q) && seen.insert(address.clone()) {
                    let who = Addressee::new("", address, AddresseeKind::SharedMailbox);
                    hits.push(Hit::new(who, Source::Recent, 0.0, q, kinds));
                }
            }
        }
        hits.sort_by(|a, b| {
            a.fit.cmp(&b.fit).then_with(|| b.score.total_cmp(&a.score)).then_with(|| a.place.cmp(&b.place)).then_with(|| a.address.cmp(&b.address))
        });
        hits.truncate(MAX_SUGGESTIONS);
        let scanned = self.contacts_len().saturating_add(self.people.index.len()).saturating_add(rooms_scanned);
        let p = &mut self.people;
        p.passes = p.passes.saturating_add(1);
        p.scanned = p.scanned.saturating_add(u64::try_from(scanned).unwrap_or(u64::MAX));
        hits.into_iter().map(|h| h.s).collect()
    }
}
