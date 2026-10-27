//! What an account keeps of its calendars and address books: the server's (CalDAV, CardDAV)
//! with what's needed to sync them, or ones kept on this computer only.

use serde::{Deserialize, Serialize};

use crate::ical::CalEvent;
use crate::vcard::Card;

pub mod scheduling;
pub mod tasks;

pub use tasks::{DueGroup, StoredTask, Task, TaskStatus};

/// The id of a calendar or address book that lives on this computer only.
pub const LOCAL: &str = "local";

/// Where an account's calendars and address books are kept in its cache.
pub const PIM_DOC: &str = "pim.json";

/// Where what an account's next calendar sync reads is kept ([`PimReading`]).
pub const PIM_READING_DOC: &str = "pim-reading.json";

/// What an account's next calendar sync reads ([`Pim::own_only`], [`Pim::not_read`]), kept in a
/// document of its own: it changes each time the calendar is shown or left, and the whole
/// calendar written again for it (25,000 other people's appointments, compressed) held the window
/// up a second at each change (27/10). It's newer than what the calendar's document says.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PimReading {
    #[serde(default)]
    pub own_only: bool,
    #[serde(default)]
    pub not_read: Vec<String>,
}

/// A calendar.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarInfo {
    /// The collection's address on the server, or [`LOCAL`].
    pub id: String,
    pub name: String,
    /// An index into the UI's palette.
    #[serde(default)]
    pub color: u8,
    #[serde(default)]
    pub read_only: bool,
    /// The collection's version at the last sync (`getctag` / sync token).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ctag: String,
    /// The group it's listed in, as the server keeps the user's calendar groups (Outlook's My
    /// Calendars, a team's, People's Calendars); empty: the account's own.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub group: String,
}

/// Whether two UIDs name the same meeting: as an invitation names it, or as Exchange's single
/// appointments are kept here (`UID#start`, a series sent as separate items keeping its UID).
pub fn same_meeting(a: &str, b: &str) -> bool {
    let base = |u: &str| u.split('#').next().unwrap_or("").to_string();
    !a.is_empty() && base(a) == base(b)
}

/// An event with where it lives.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredEvent {
    /// The calendar's id.
    pub calendar: String,
    /// The resource's address on the server (empty until stored there).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub href: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub etag: String,
    pub event: CalEvent,
    /// Changed here and not yet on the server.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dirty: bool,
    /// The server is to tell the guests of this change itself (Exchange sends the meeting
    /// update, as Outlook's "Send update" has it): nothing is mailed from here then. Cleared
    /// once the change is on the server.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub notify: bool,
    /// The user's answer to this invitation (`ACCEPTED`, `TENTATIVE`, `DECLINED`), for the
    /// server to give (Exchange's AcceptItem…): it updates the meeting it placed in the
    /// calendar and tells the organizer itself, as Outlook has it. One not yet in the calendar
    /// here (no address) is answered once the calendar is read and the meeting found there.
    /// Cleared once given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub respond: Option<String>,
}

/// An address book.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ctag: String,
}

/// A card with where it lives.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredCard {
    pub book: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub href: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub etag: String,
    pub card: Card,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dirty: bool,
}

/// The id of someone else's calendar opened by their address.
pub fn shared_calendar_id(email: &str) -> String {
    format!("shared:{}", email.trim().to_ascii_lowercase())
}

/// Whose calendar a shared calendar's id is (`shared:<address>`).
pub fn shared_calendar_owner(id: &str) -> Option<&str> {
    id.strip_prefix("shared:").filter(|e| !e.is_empty())
}

/// A note, as Outlook's Notes keeps them: text, and a colour.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub uid: String,
    pub text: String,
    /// Outlook's note colours: 0 blue, 1 green, 2 pink, 3 yellow (the usual), 4 white.
    #[serde(default = "yellow")]
    pub color: u8,
    /// When it was last changed (Unix seconds).
    #[serde(default)]
    pub modified: i64,
}

fn yellow() -> u8 {
    NOTE_YELLOW
}

/// The colour a new note takes, as in Outlook.
pub const NOTE_YELLOW: u8 = 3;
/// How many note colours there are.
pub const NOTE_COLORS: u8 = 5;

impl Note {
    /// Its first line: what a list of notes shows as its title.
    pub fn title(&self) -> &str {
        self.text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("")
    }
}

/// A note with where it lives: `href` is its item on the server (Exchange's Notes folder),
/// empty for a note kept on this computer only.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredNote {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub href: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub etag: String,
    pub note: Note,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dirty: bool,
}

/// Something deleted here that is still to be deleted on the server.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tombstone {
    pub href: String,
    #[serde(default)]
    pub etag: String,
    /// The calendar it was in, when that decides where it's deleted (someone else's calendar).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub calendar: String,
}

/// An account's calendars and address books.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pim {
    #[serde(default)]
    pub calendars: Vec<CalendarInfo>,
    #[serde(default)]
    pub events: Vec<StoredEvent>,
    #[serde(default)]
    pub books: Vec<BookInfo>,
    #[serde(default)]
    pub cards: Vec<StoredCard>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deleted: Vec<Tombstone>,
    /// Other people's calendars opened from this account (their addresses, lower-case), as
    /// Outlook's Open Calendar › From Address Book: read from Exchange with the rest.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shared: Vec<String>,
    /// Notes (Outlook's Notes): the server's (Exchange) and those kept here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<StoredNote>,
    /// Tasks (Outlook's Tasks): the server's (Exchange) and those kept here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<StoredTask>,
    /// Where the server keeps the user's calendars and address books (found once); empty when
    /// it has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caldav_home: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carddav_home: Option<String>,
    /// When the server was last asked where they are.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dav_checked: Option<i64>,
    /// Read this account's own calendar and contacts only, not other people's or public
    /// folders' (the calendar isn't shown: they're read once it is).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub own_only: bool,
    /// Calendars left unticked: listed, not read (what this computer has of them stays).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not_read: Vec<String>,
}

/// What a CalDAV / CardDAV sync found, for the engine to merge into its [`Pim`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DavSync {
    /// The homes found (`None`: the server offers none).
    pub caldav_home: Option<String>,
    pub carddav_home: Option<String>,
    pub checked: i64,
    /// The server's calendars (`None`: no CalDAV).
    pub calendars: Option<Vec<CalendarInfo>>,
    /// The events of the calendars read in full (`synced_calendars`), except those unchanged
    /// (`kept`, by href).
    pub events: Vec<StoredEvent>,
    pub synced_calendars: Vec<String>,
    /// Calendars that didn't change: what the engine has stays.
    pub unchanged_calendars: Vec<String>,
    pub books: Option<Vec<BookInfo>>,
    pub cards: Vec<StoredCard>,
    pub synced_books: Vec<String>,
    pub unchanged_books: Vec<String>,
    /// Items read again unchanged (their etag matched).
    pub kept: Vec<String>,
    /// Changes stored on the server, with their new addresses and etags.
    pub pushed_events: Vec<StoredEvent>,
    pub pushed_cards: Vec<StoredCard>,
    /// The server's notes, read in full (`None`: it keeps none, so notes stay on this
    /// computer), and the notes stored there this time.
    #[serde(default)]
    pub notes: Option<Vec<StoredNote>>,
    #[serde(default)]
    pub pushed_notes: Vec<StoredNote>,
    /// The server's tasks, read in full (`None`: it keeps none, so tasks stay on this
    /// computer), and the tasks stored there this time.
    #[serde(default)]
    pub tasks: Option<Vec<StoredTask>>,
    #[serde(default)]
    pub pushed_tasks: Vec<StoredTask>,
    /// Deletions done (by href).
    pub deleted: Vec<String>,
    /// Invitations the server answered ([`StoredEvent::respond`]): their UID and the answer.
    #[serde(default)]
    pub responded: Vec<(String, String)>,
    /// What couldn't be stored or deleted (tried again next time).
    pub errors: Vec<String>,
    /// Only some calendars and address books were looked at ([`Pim::own_only`]): those not
    /// listed stay as they are.
    #[serde(default)]
    pub partial: bool,
}

impl Pim {
    /// What the next sync reads.
    pub fn reading(&self) -> PimReading {
        PimReading { own_only: self.own_only, not_read: self.not_read.clone() }
    }

    /// Reading as `r` says, when there's one ([`PIM_READING_DOC`]): it's newer than what the
    /// calendar's document says.
    pub fn read_as(&mut self, r: Option<PimReading>) {
        if let Some(r) = r {
            self.own_only = r.own_only;
            self.not_read = r.not_read;
        }
    }

    /// Make sure there is somewhere to put events and people.
    pub fn ensure_defaults(&mut self) {
        if self.calendars.is_empty() {
            self.calendars.push(CalendarInfo {
                id: LOCAL.into(),
                name: "Calendar".into(),
                color: 0,
                read_only: false,
                ctag: String::new(),
                group: String::new(),
            });
        }
        if self.books.is_empty() {
            self.books.push(BookInfo { id: LOCAL.into(), name: "Contacts".into(), read_only: false, ctag: String::new() });
        }
    }

    /// The first calendar events can be added to.
    pub fn default_calendar(&self) -> Option<&CalendarInfo> {
        self.calendars.iter().find(|c| !c.read_only)
    }

    pub fn default_book(&self) -> Option<&BookInfo> {
        self.books.iter().find(|b| !b.read_only)
    }

    /// Every event and instance with this uid.
    pub fn event_indices(&self, uid: &str) -> Vec<usize> {
        self.events.iter().enumerate().filter(|(_, e)| e.event.uid == uid).map(|(i, _)| i).collect()
    }
}

/// Make UIDs unique within each calendar. A calendar can hold several plain (non-recurring)
/// appointments under one UID, e.g. invitations a scheduling tool sends per date under its
/// series' UID. The engine stores events by UID, so only one survived. Such appointments get
/// `<uid>/<start>` here. Recurring masters and exceptions are untouched, and the new UID never
/// goes back to the server.
pub fn separate_shared_uids(events: &mut [StoredEvent]) {
    let mut seen: std::collections::HashMap<(String, String), usize> = std::collections::HashMap::new();
    for e in events.iter().filter(|e| e.event.recurrence_id.is_none()) {
        *seen.entry((e.calendar.clone(), e.event.uid.clone())).or_default() += 1;
    }
    let mut taken: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    for e in events.iter_mut() {
        let shared = seen.get(&(e.calendar.clone(), e.event.uid.clone())).is_some_and(|n| *n > 1);
        if !shared || e.event.recurrence_id.is_some() || e.event.rrule.is_some() {
            continue;
        }
        let mut uid = format!("{}/{}", e.event.uid, e.event.start);
        let mut n = 1;
        while !taken.insert((e.calendar.clone(), uid.clone())) && n < 1000 {
            n += 1;
            uid = format!("{}/{}-{n}", e.event.uid, e.event.start);
        }
        e.event.uid = uid;
    }
}

#[cfg(test)]
mod shared_uid_tests {
    use super::*;

    /// Duplicates get their start appended (a counter when the start repeats too); a lone event
    /// and a recurring one in another calendar keep their UID.
    #[test]
    fn repeated_uids_get_their_start_appended() {
        let ev = |cal: &str, start: i64, rrule: Option<&str>| StoredEvent {
            calendar: cal.into(),
            event: CalEvent { uid: "series-1".into(), start, rrule: rrule.map(str::to_string), ..Default::default() },
            ..Default::default()
        };
        let mut list = vec![ev("cal", 100, None), ev("cal", 200, None), ev("cal", 100, None), ev("other", 100, Some("FREQ=WEEKLY"))];
        separate_shared_uids(&mut list);
        let uids: Vec<&str> = list.iter().map(|e| e.event.uid.as_str()).collect();
        assert_eq!(uids, ["series-1/100", "series-1/200", "series-1/100-2", "series-1"]);
        // Nothing to tell apart: unchanged.
        let mut one = vec![ev("cal", 100, None)];
        separate_shared_uids(&mut one);
        assert_eq!(one[0].event.uid, "series-1");
    }
}
