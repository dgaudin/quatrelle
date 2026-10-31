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

/// The fields that ask for suggestions: the message's recipients and From, the address book's
/// (`book-*`), a meeting's (`required`, `optional`, `rooms`, and their `att-*` twins), the command line's,
/// and those of the dialogs that open a place (`calendar`, `mailbox`) or match people (`rule-one`,
/// `rule-any`, `search-people`).
pub const FIELD_NAMES: &[&str] = &[
    "to",
    "cc",
    "bcc",
    "replyTo",
    "book-to",
    "book-cc",
    "book-bcc",
    "cli",
    "from",
    "required",
    "optional",
    "rooms",
    "att-required",
    "att-optional",
    "att-rooms",
    "calendar",
    "mailbox",
    "rule-one",
    "rule-any",
    "search-people",
];

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

    /// What the field named `name` takes: the one place that says it (a name not among
    /// [`FIELD_NAMES`]' places of their own: To's, everyone).
    pub fn accepts_of(name: &str) -> &'static Accepts {
        match name {
            "required" | "optional" | "att-required" | "att-optional" => &Accepts::ATTENDEES,
            "rooms" | "att-rooms" => &Accepts::ROOMS,
            "from" => &Accepts::ONE_SENDER,
            "calendar" => &Accepts::CALENDARS,
            "mailbox" => &Accepts::MAILBOX,
            "rule-one" => &Accepts::MATCHED_ONE,
            "rule-any" | "search-people" => &Accepts::MATCHED,
            _ => &Accepts::RECIPIENTS,
        }
    }

    /// What this field takes ([`FieldId::accepts_of`]).
    pub fn accepts(&self) -> &'static Accepts {
        Self::accepts_of(self.name)
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

/// A field's last answer, what it was worked out from, and when.
pub(crate) struct Cached {
    text: String,
    account: Option<String>,
    kinds: Vec<AddresseeKind>,
    revision: u64,
    at: i64,
    list: Vec<Suggestion>,
}

/// While the book is being read (a slice per frame, each changing it), the same text is worked
/// out again at most this often (seconds); once it's read, at once.
const READING_EVERY: i64 = 1;

/// A candidate, not yet cloned: how well it fits, how much it's used, its address in lower case
/// (or a contact group's reference), and where it comes from.
struct Candidate<'a> {
    fit: u8,
    score: f64,
    key: &'a str,
    from: From<'a>,
}

enum From<'a> {
    Book(&'a quatrelle_model::contacts::Contact),
    Index(&'a Addressee),
    Room(&'a String),
}

/// A suggestion kept, and what orders it among the others.
struct Hit {
    s: Suggestion,
    fit: u8,
    score: f64,
    place: usize,
    address: String,
}

/// Whether a name (in lower case) or an address (in lower case) starts with `q`: the address,
/// the name, or any word of the name.
fn starts_with(name: &str, address: &str, q: &str) -> bool {
    address.starts_with(q) || name.starts_with(q) || words(name).any(|w| w.starts_with(q))
}

fn words(name: &str) -> impl Iterator<Item = &str> {
    name.split(|c: char| c.is_whitespace() || matches!(c, '-' | '.' | ',')).filter(|w| !w.is_empty())
}

/// How well a name and an address (in lower case) fit `q`: 0 one of them exactly, 1 the start of
/// the first or the last name or of the address, 2 anything else that matched.
fn fit(name: &str, address: &str, q: &str) -> u8 {
    let name = name.trim();
    if name == q || address == q {
        return 0;
    }
    let mut w = words(name);
    let first = w.next();
    let last = w.last().or(first);
    if address.starts_with(q) || first.is_some_and(|f| f.starts_with(q)) || last.is_some_and(|l| l.starts_with(q)) { 1 } else { 2 }
}

impl Session {
    /// Up to [`MAX_SUGGESTIONS`] people for a contacts field, best first, from what is on this
    /// computer: nothing for a blank text; the field's last answer when neither its text nor the
    /// book changed since (or the book is being read, and the last pass is under a second old);
    /// else one pass ([`Session::people_suggest_passes`]).
    pub fn people_suggest(&mut self, field: FieldId, account: Option<&str>, accepts: &Accepts, text: &str) -> Vec<Suggestion> {
        self.people_suggest_kinds(field, account, accepts.kinds, text)
    }

    /// [`Session::people_suggest`] for the kinds a field takes, as a command gives them.
    pub(crate) fn people_suggest_kinds(&mut self, field: FieldId, account: Option<&str>, kinds: &[AddresseeKind], text: &str) -> Vec<Suggestion> {
        let q: String = text.trim().chars().take(MAX_TEXT).collect::<String>().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let (revision, now) = (self.contacts_revision(), self.clock.now);
        let reading = self.contacts_pending();
        if let Some(c) = self.people.suggest_cache.get(&field)
            && c.text == q
            && c.account.as_deref() == account
            && c.kinds == kinds
            && (c.revision == revision || (reading && now.saturating_sub(c.at) < READING_EVERY))
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
        let cached = Cached { text: q, account: account.map(str::to_string), kinds: kinds.to_vec(), revision, at: now, list: list.clone() };
        self.people.suggest_cache.insert(field, cached);
        list
    }

    /// The suggestion passes run, and the entries they looked at.
    pub fn people_suggest_passes(&self) -> (u64, u64) {
        (self.people.passes, self.people.scanned)
    }

    /// One pass: the book's people, the directory's answers, the rooms and the shared mailboxes
    /// that `q` (lower case) matches, of the kinds taken, each address once, ordered, cut. Every
    /// match is ranked (how well it fits, then how much it's used) before anything is cut, and
    /// only what is kept is cloned.
    fn suggestion_pass(&mut self, account: Option<&str>, kinds: &[AddresseeKind], q: &str) -> Vec<Suggestion> {
        self.contacts_step(0);
        let now = self.clock.now;
        let book: Vec<Candidate> = self
            .contacts_book()
            .map(|b| {
                b.matches(q)
                    .map(|c| Candidate {
                        fit: fit(c.name_lower(), &c.address.email, q),
                        score: c.score(now),
                        key: &c.address.email,
                        from: From::Book(c),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let index: Vec<Candidate> = self
            .people
            .index
            .iter()
            .filter_map(|(key, e)| {
                // A contact group's key is its reference, not an address.
                let address = if e.who.address.is_empty() { "" } else { key.as_str() };
                starts_with(&e.name_lower, address, q).then(|| Candidate {
                    fit: fit(&e.name_lower, address, q),
                    score: 0.0,
                    key,
                    from: From::Index(&e.who),
                })
            })
            .collect();
        let mut rooms_scanned = 0usize;
        let mut rooms: Vec<Candidate> = Vec::new();
        for (_, set) in self.people.rooms.iter().filter(|(a, _)| account.is_none_or(|x| x == a.as_str())) {
            rooms_scanned = rooms_scanned.saturating_add(set.len());
            if Accepts::taken_by(kinds, AddresseeKind::Room) {
                for (address, name) in set {
                    let lower = name.to_lowercase();
                    if starts_with(&lower, address, q) {
                        rooms.push(Candidate { fit: fit(&lower, address, q), score: 0.0, key: address, from: From::Room(name) });
                    }
                }
            }
        }
        let mut seen: HashSet<String> = HashSet::new();
        let mut hits: Vec<Hit> = Vec::new();
        for candidates in [book, index, rooms] {
            hits.extend(self.best_of(candidates, account, kinds, &mut seen));
        }
        if Accepts::taken_by(kinds, AddresseeKind::SharedMailbox) {
            for m in self.accounts.iter().flat_map(|a| &a.config.shared_mailboxes) {
                let address = m.trim().to_ascii_lowercase();
                if starts_with("", &address, q) && seen.insert(address.clone()) {
                    // Ranked on the address in lower case, as typed text is: configured in any
                    // case, the mailbox typed exactly is an exact match.
                    let how_well = fit("", &address, q);
                    let who = Addressee::new("", address, AddresseeKind::SharedMailbox);
                    hits.push(hit(who, Source::Recent, how_well, 0.0, kinds));
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

    /// The best [`MAX_SUGGESTIONS`] of one source's candidates whose kind the field takes, best
    /// first (how well they fit, how much they're used, the address), not already offered. A
    /// kind is looked up only for the candidates walked (ties of fit and use across kinds are
    /// told apart by the address here, by the kind among the sources).
    fn best_of(&self, mut candidates: Vec<Candidate>, account: Option<&str>, kinds: &[AddresseeKind], seen: &mut HashSet<String>) -> Vec<Hit> {
        candidates.sort_by(|a, b| a.fit.cmp(&b.fit).then_with(|| b.score.total_cmp(&a.score)).then_with(|| a.key.cmp(b.key)));
        let mut out = Vec::new();
        for c in candidates {
            if out.len() >= MAX_SUGGESTIONS {
                break;
            }
            if seen.contains(c.key) {
                continue;
            }
            let (who, source) = match c.from {
                From::Book(b) => {
                    // Mail exchanged counts first: Contacts is for someone known from there alone.
                    let s = if b.saved && b.sent == 0 && b.received == 0 { Source::Contacts } else { Source::Recent };
                    let kind = self.kind_lower(account, c.key);
                    (Addressee::new(b.address.name.clone(), b.address.email.clone(), kind), s)
                }
                // The room lists, and the mailboxes opened, know better than the directory.
                From::Index(a) => {
                    let kind = if a.address.is_empty() { a.kind } else { self.kind_lower(account, c.key) };
                    (Addressee { kind, ..a.clone() }, Source::Directory)
                }
                From::Room(name) => (Addressee::new(name.as_str(), c.key, AddresseeKind::Room), Source::RoomList),
            };
            if Accepts::taken_by(kinds, who.kind) {
                seen.insert(c.key.to_string());
                out.push(hit(who, source, c.fit, c.score, kinds));
            }
        }
        out
    }
}

fn hit(who: Addressee, source: Source, fit: u8, score: f64, kinds: &[AddresseeKind]) -> Hit {
    let place = kinds.iter().position(|k| *k == who.kind).unwrap_or(kinds.len());
    Hit { fit, score, place, address: who.address.to_lowercase(), s: Suggestion { who, source } }
}
