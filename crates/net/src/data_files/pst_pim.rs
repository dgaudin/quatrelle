// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Contacts, contact groups, calendar items, meeting requests, tasks and journal entries in an
//! Outlook data file (MS-OXOCNTC, MS-OXOCAL, MS-OXOTASK, MS-OXOJRNL).
//!
//! Most of what makes a contact or an appointment lives in *named* properties: their ids differ
//! from file to file and are found through the file's name-to-id map (MS-PST 2.4.7). The
//! recurrence of a repeating appointment is a binary pattern (MS-OXOCAL 2.2.1.44), read into an
//! RFC 5545 rule, its deleted occurrences and its changed ones (`recur`).

use std::collections::HashMap;

use outlook_pst::ltp::prop_context::{GuidValue, PropertyValue};
use outlook_pst::messaging::named_prop::{NamedPropertyGuid, NamedPropertyId, PS_MAPI, PS_PUBLIC_STRINGS};
use outlook_pst::messaging::store::Store as PstStore;
use quatrelle_model::EmailAddress;
use quatrelle_model::dates::civil_from_days;
use quatrelle_model::ical::{Attendee, CalEvent, PrivateKind, ShowAs};
use quatrelle_model::vcard::{Card, Photo, PostalAddress, Typed, base64_encode};

use crate::pst::Val;

mod files;
mod groups;
mod meetings;
mod recur;
mod rest;
mod sharing;
mod tasks;
pub(crate) use files::{card_file, event_file};
pub(crate) use groups::{group_of, wrapped_entry_id};
pub(crate) use meetings::invitation;
pub(crate) use meetings::uid_of;
use meetings::{known_zone, zone_key_name};
pub(crate) use recur::{Own, changed_events};
pub(crate) use recur::{recurrence, recurrence_in};
pub(crate) use rest::{split_rest, with_rest};
pub(crate) use sharing::{put_before, sharing_lines};
pub(crate) use tasks::task_of;

pub(crate) type GuidKey = (u32, u16, u16, [u8; 8]);
const OLE: [u8; 8] = [0xC0, 0, 0, 0, 0, 0, 0, 0x46];
const PSETID_ADDRESS: GuidKey = (0x0006_2004, 0, 0, OLE);
const PSETID_APPOINTMENT: GuidKey = (0x0006_2002, 0, 0, OLE);
const PSETID_TASK: GuidKey = (0x0006_2003, 0, 0, OLE);
const PSETID_NOTE: GuidKey = (0x0006_200E, 0, 0, OLE);
const PSETID_COMMON: GuidKey = (0x0006_2008, 0, 0, OLE);
const PSETID_LOG: GuidKey = (0x0006_200A, 0, 0, OLE);
const PSETID_MEETING: GuidKey = (0x6ED8_DA90, 0x450B, 0x101B, [0x98, 0xDA, 0x00, 0xAA, 0x00, 0x3F, 0x13, 0x05]);
/// Name-to-id entries read at most.
const MAX_NAMES: usize = 200_000;
/// Deleted (and changed) occurrences read at most.
const MAX_EXCEPTIONS: usize = 5_000;
/// A repeating appointment's pattern read at most: one with as many changed occurrences as are
/// read, each with a long subject and location, is far less. What's past it is as if cut short.
const MAX_PATTERN: usize = 1 << 20;
/// An event's categories kept at most.
const MAX_CATEGORIES: usize = 64;
/// A reminder further ahead than a year (in minutes) isn't one; nor a journal entry longer.
const MAX_MINUTES: i64 = 366 * 24 * 60;
/// The last moment a reminder may come at (9999-12-31 23:59:59 UTC): past it, a time is damage.
const LAST_TIME: i64 = 253_402_300_799;
/// Minutes from 1601-01-01 to 1970-01-01.
const EPOCH_MINUTES: i64 = 11_644_473_600 / 60;

fn key(g: &GuidValue) -> GuidKey {
    (g.data1(), g.data2(), g.data3(), *g.data4())
}

/// The property (in PS_PUBLIC_STRINGS, a PtypBoolean) an export gives a pinned message: MS-OXPROPS
/// has no pin (Outlook's files keep none), so it is Quatrelle's own, which Outlook ignores and
/// the import reads back as `$Pinned` (`pst_export::item`)…
pub(crate) const PINNED: &str = "X-Quatrelle-Pinned";
/// …as it reads MailCraft's, which a file MailCraft exported keeps.
const PINNED_UPSTREAM: &str = "X-MailCraft-Pinned";
/// The property (in PS_PUBLIC_STRINGS) an export marks each item of the account's calendar,
/// contacts, notes and tasks with: the item's uid, the account's (a digest of its id) and a digest
/// of what was written, so a file added to again tells the items it has from Quatrelle from
/// changed ones, from its own and from another account's (`pst_export::pim`).
pub(crate) const EXPORTED_ITEM: &str = "X-Quatrelle-Item";

/// The data file's named properties: (property set, number) → property id; and those named by a
/// string that Quatrelle reads, the categories (`Keywords` in PS_PUBLIC_STRINGS), a message's pin
/// ([`PINNED`]) and an exported item's mark ([`EXPORTED_ITEM`]).
#[derive(Default)]
pub(crate) struct Named {
    ids: HashMap<(GuidKey, u32), u16>,
    keywords: Option<u16>,
    pinned: Vec<u16>,
    exported: Option<u16>,
}

impl Named {
    pub(crate) fn of(store: &dyn PstStore) -> Named {
        let mut named = Named::default();
        let Ok(map) = store.named_property_map() else { return named };
        let props = map.properties();
        let guids = props.stream_guid().unwrap_or_default();
        let strings = match props.get(0x0004) {
            Some(PropertyValue::Binary(b)) => b.buffer(),
            _ => &[],
        };
        for e in props.stream_entry().unwrap_or_default().into_iter().take(MAX_NAMES) {
            let set = match e.guid() {
                NamedPropertyGuid::None => continue,
                NamedPropertyGuid::Mapi => key(&PS_MAPI),
                NamedPropertyGuid::PublicStrings => key(&PS_PUBLIC_STRINGS),
                NamedPropertyGuid::GuidIndex(i) => match guids.get(usize::from(i)) {
                    Some(g) => key(g),
                    None => continue,
                },
            };
            match e.id() {
                NamedPropertyId::Number(lid) => {
                    named.ids.insert((set, lid), e.prop_id());
                }
                NamedPropertyId::StringOffset(at) if set == key(&PS_PUBLIC_STRINGS) => match name_at(strings, at) {
                    Some(n) if n.eq_ignore_ascii_case("Keywords") => named.keywords = Some(e.prop_id()),
                    Some(n) if (n.eq_ignore_ascii_case(PINNED) || n.eq_ignore_ascii_case(PINNED_UPSTREAM)) && named.pinned.len() < 2 => {
                        named.pinned.push(e.prop_id());
                    }
                    Some(n) if n.eq_ignore_ascii_case(EXPORTED_ITEM) => named.exported = Some(e.prop_id()),
                    _ => {}
                },
                NamedPropertyId::StringOffset(_) => {}
            }
        }
        named
    }

    /// The named properties of an Outlook message file (`.msg`, MS-OXMSG §2.2.3): each its set
    /// (`None`: PS_MAPI; the empty GUID: PS_PUBLIC_STRINGS), its number or name, and its id.
    pub(crate) fn of_names(entries: impl Iterator<Item = (Option<GuidKey>, Result<u32, String>, u16)>) -> Named {
        let mut named = Named::default();
        for (set, name, id) in entries.take(MAX_NAMES) {
            let set = match set {
                None => key(&PS_MAPI),
                Some(g) if g == (0, 0, 0, [0; 8]) => key(&PS_PUBLIC_STRINGS),
                Some(g) => g,
            };
            match name {
                Ok(lid) => {
                    named.ids.insert((set, lid), id);
                }
                Err(n) if set == key(&PS_PUBLIC_STRINGS) => match n {
                    n if n.eq_ignore_ascii_case("Keywords") => named.keywords = Some(id),
                    n if (n.eq_ignore_ascii_case(PINNED) || n.eq_ignore_ascii_case(PINNED_UPSTREAM)) && named.pinned.len() < 2 => {
                        named.pinned.push(id);
                    }
                    n if n.eq_ignore_ascii_case(EXPORTED_ITEM) => named.exported = Some(id),
                    _ => {}
                },
                Err(_) => {}
            }
        }
        named
    }

    /// Only the categories' property known, at `id` (for tests and the fuzz target).
    pub(crate) fn with_keywords(id: u16) -> Named {
        Named { keywords: Some(id), ..Default::default() }
    }

    /// The property holding an item's categories.
    pub(crate) fn keywords(&self) -> Option<u16> {
        self.keywords
    }

    /// The properties holding a message's pin ([`PINNED`], and MailCraft's).
    pub(crate) fn pinned(&self) -> &[u16] {
        &self.pinned
    }

    /// The property holding an exported item's mark ([`EXPORTED_ITEM`]).
    pub(crate) fn exported(&self) -> Option<u16> {
        self.exported
    }

    fn id(&self, set: GuidKey, lid: u32) -> Option<u16> {
        self.ids.get(&(set, lid)).copied()
    }
}

/// A property's name in the name-to-id map's string stream (MS-PST 2.4.7.4): at byte `at`, a
/// little-endian length in bytes, then the name in UTF-16. Names longer than 512 bytes aren't
/// read.
fn name_at(strings: &[u8], at: u32) -> Option<String> {
    let at = usize::try_from(at).ok()?;
    let len = strings.get(at..at.checked_add(4)?)?;
    let len = usize::try_from(u32::from_le_bytes([*len.first()?, *len.get(1)?, *len.get(2)?, *len.get(3)?])).ok()?;
    if len > 512 {
        return None;
    }
    let start = at.checked_add(4)?;
    let bytes = strings.get(start..start.checked_add(len)?)?;
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    Some(String::from_utf16_lossy(&units))
}

/// What a contact or appointment adds to a mail item.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Extra {
    pub display: String,
    pub given: String,
    pub family: String,
    pub company: String,
    pub title: String,
    /// A contact's department, office, profession, manager, assistant, nickname and spouse
    /// (PidTagDepartmentName, PidTagOfficeLocation, PidTagProfession, PidTagManagerName,
    /// PidTagAssistant, PidTagNickname, PidTagSpouseName).
    pub department: String,
    pub office: String,
    pub profession: String,
    pub manager: String,
    pub assistant: String,
    pub nickname: String,
    pub spouse: String,
    /// A contact's web page (PidLidHtml, else its business or personal home page) and instant
    /// messaging address (PidLidInstantMessagingAddress).
    pub web: String,
    pub im: String,
    /// A contact's wedding anniversary (PidTagWeddingAnniversary).
    pub anniversary: Option<i64>,
    pub emails: Vec<String>,
    pub phones: Vec<(String, &'static str)>,
    /// Business, home and other addresses.
    pub addresses: Vec<PostalAddress>,
    pub birthday: Option<i64>,
    pub start: Option<i64>,
    /// A task's due date, and whether it's done (PidLidTaskDueDate, PidLidTaskComplete); its
    /// status (PidLidTaskStatus: 0 not started, 1 in progress, 2 complete, 3 waiting on someone
    /// else, 4 deferred), how far along it is in percent (PidLidPercentComplete, a fraction of 1),
    /// its start (PidLidTaskStartDate), the day it was done (PidLidTaskDateCompleted), and how it
    /// repeats (PidLidTaskRecurrence, a recurrence pattern).
    pub due: Option<i64>,
    pub done: bool,
    pub task_status: Option<i64>,
    pub percent: Option<u8>,
    pub task_start: Option<i64>,
    pub completed: Option<i64>,
    pub task_recurrence: Vec<u8>,
    pub end: Option<i64>,
    pub location: String,
    pub all_day: bool,
    pub recurrence: Vec<u8>,
    pub uid: String,
    /// A note's colour (PidLidNoteColor: blue, green, pink, yellow, white).
    pub note_color: Option<u8>,
    /// A note's window size (PidLidNoteWidth, PidLidNoteHeight), held to its bounds; 0: none.
    pub note_width: u16,
    pub note_height: u16,
    /// When it last changed.
    pub modified: Option<i64>,
    /// The contact's picture (its media type, and its bytes), when the item holds one.
    pub photo: Option<(String, Vec<u8>)>,
    /// A meeting's version (PidLidAppointmentSequence).
    pub sequence: u32,
    /// An event's categories, how it shows its time (PidLidBusyStatus: 0 free, 1 tentative, 2
    /// busy, 3 away, 4 working elsewhere), its sensitivity (1 personal, 2 private, 3
    /// confidential) and its reminder, in minutes before it starts.
    pub categories: Vec<String>,
    pub busy: Option<i64>,
    pub sensitivity: u8,
    pub reminder: Option<i64>,
    /// When a task's reminder comes, while it reminds at all: the time it was put off to
    /// (PidLidReminderSignalTime), else the one it was set for (PidLidReminderTime). A task has no
    /// start for a reminder to count from.
    pub reminder_at: Option<i64>,
    /// A contact group's members that carry their address: (name, address).
    pub members: Vec<(String, String)>,
    /// A journal entry's start and end.
    pub log_start: Option<i64>,
    pub log_end: Option<i64>,
    /// A changed occurrence's item (attached to its series): the start of the occurrence it
    /// replaces (PidLidExceptionReplaceTime).
    pub replaces: Option<i64>,
    /// The zone an appointment's times were set in, when the time zone database knows it by the
    /// name the item gives (PidLidTimeZoneDescription, else its time zone definition's key name);
    /// never UTC, which is no zone.
    pub tzid: String,
    /// How each of a meeting's attendees answered (PidTagRecipientTrackStatus), by address in
    /// lower case; its organizer, when its recipients name one (recipOrganizer, which isn't among
    /// them then); whether it asks for answers (PidTagResponseRequested).
    pub answers: Vec<(String, i64)>,
    pub organizer: Option<EmailAddress>,
    pub response_requested: bool,
}

/// Read the contact and appointment properties of an item.
pub(crate) fn extract<'a>(p: &dyn Fn(u16) -> Option<Val<'a>>, named: &Named) -> Extra {
    let s = |id: u16| p(id).and_then(Val::into_text).unwrap_or_default();
    let n = |set, lid| named.id(set, lid).and_then(p);
    let ns = |set, lid| n(set, lid).and_then(Val::into_text).unwrap_or_default();
    let time = |set, lid, fallback: u16| n(set, lid).or_else(|| p(fallback)).and_then(|v| v.time());
    let mut x = Extra {
        display: s(0x3001),
        given: s(0x3A06),
        family: s(0x3A11),
        company: s(0x3A16),
        title: s(0x3A17),
        department: s(0x3A18),
        office: s(0x3A19),
        profession: s(0x3A46),
        manager: s(0x3A4E),
        assistant: s(0x3A30),
        nickname: s(0x3A4F),
        spouse: s(0x3A48),
        ..Default::default()
    };
    x.web = [ns(PSETID_ADDRESS, 0x802B), s(0x3A51), s(0x3A50)].into_iter().map(|w| w.trim().to_string()).find(|w| !w.is_empty()).unwrap_or_default();
    x.im = ns(PSETID_ADDRESS, 0x8062).trim().to_string();
    x.anniversary = p(0x3A41).and_then(|v| v.time());
    // Email1/2/3: the address, or (for an Exchange address) the display address.
    for (address, original) in [(0x8083, 0x8084), (0x8093, 0x8094), (0x80A3, 0x80A4)] {
        let e = [ns(PSETID_ADDRESS, address), ns(PSETID_ADDRESS, original)].into_iter().find(|e| e.contains('@'));
        if let Some(e) = e.filter(|e| !x.emails.contains(e)) {
            x.emails.push(e);
        }
    }
    // MS-OXOCNTC's telephone numbers: business (and its second), home (and its second),
    // mobile, fax, pager, other.
    for (id, kind) in
        [(0x3A08, "work"), (0x3A1B, "work"), (0x3A09, "home"), (0x3A2F, "home"), (0x3A1C, "cell"), (0x3A24, "fax"), (0x3A21, "pager"), (0x3A1F, "")]
    {
        let v = s(id);
        if !v.trim().is_empty() {
            x.phones.push((v.trim().to_string(), kind));
        }
    }
    // Its addresses: (street, city, state, postal code, country) of business, home and other.
    for (kind, [street, city, state, code, country]) in [
        ("work", [0x3A29, 0x3A27, 0x3A28, 0x3A2A, 0x3A26]),
        ("home", [0x3A5D, 0x3A59, 0x3A5C, 0x3A5B, 0x3A5A]),
        ("", [0x3A63, 0x3A5F, 0x3A62, 0x3A61, 0x3A60]),
    ] {
        let a = PostalAddress {
            kind: kind.into(),
            street: s(street).trim().replace("\r\n", "\n"),
            city: s(city).trim().to_string(),
            region: s(state).trim().to_string(),
            postcode: s(code).trim().to_string(),
            country: s(country).trim().to_string(),
        };
        if !a.is_empty() {
            x.addresses.push(a);
        }
    }
    x.birthday = p(0x3A42).and_then(|v| v.time());
    x.start = time(PSETID_APPOINTMENT, 0x820D, 0x0060);
    x.end = time(PSETID_APPOINTMENT, 0x820E, 0x0061);
    x.location = ns(PSETID_APPOINTMENT, 0x8208);
    x.due = n(PSETID_TASK, 0x8105).and_then(|v| v.time());
    x.done = matches!(n(PSETID_TASK, 0x811C), Some(Val::Bool(true)));
    x.task_status = match n(PSETID_TASK, 0x8101) {
        Some(Val::Int(s @ 0..=4)) => Some(s),
        _ => None,
    };
    // A fraction (0.4 is 40%); one out of range, or not a number, is none.
    x.percent = n(PSETID_TASK, 0x8102)
        .and_then(|v| v.float())
        .filter(|f| f.is_finite() && (0.0..=1.0).contains(f))
        .and_then(|f| u8::try_from((f * 100.0).round() as i64).ok());
    x.task_start = n(PSETID_TASK, 0x8104).and_then(|v| v.time()).filter(|t| (0..=LAST_TIME).contains(t));
    x.completed = n(PSETID_TASK, 0x810F).and_then(|v| v.time()).filter(|t| (0..=LAST_TIME).contains(t));
    if let Some(Val::Bin(b)) = n(PSETID_TASK, 0x8116) {
        x.task_recurrence = b.get(..MAX_PATTERN).unwrap_or(b).to_vec();
    }
    x.all_day = matches!(n(PSETID_APPOINTMENT, 0x8215), Some(Val::Bool(true)));
    if let Some(Val::Bin(b)) = n(PSETID_APPOINTMENT, 0x8216) {
        x.recurrence = b.get(..MAX_PATTERN).unwrap_or(b).to_vec();
    }
    if let Some(Val::Bin(b)) = n(PSETID_MEETING, 0x0003) {
        x.uid = uid_of(b);
    }
    x.tzid = known_zone(&ns(PSETID_APPOINTMENT, 0x8234))
        .or_else(|| match n(PSETID_APPOINTMENT, 0x825E) {
            Some(Val::Bin(b)) => zone_key_name(b).and_then(|k| known_zone(&k)),
            _ => None,
        })
        .unwrap_or_default();
    x.response_requested = matches!(p(0x0063), Some(Val::Bool(true)));
    if let Some(Val::Int(c)) = n(PSETID_NOTE, 0x8B00) {
        x.note_color = u8::try_from(c).ok().filter(|c| *c < quatrelle_model::pim::NOTE_COLORS);
    }
    let side = |lid, min| match n(PSETID_NOTE, lid) {
        Some(Val::Int(v)) => quatrelle_model::pim::note_side(v, min),
        _ => 0,
    };
    x.note_width = side(0x8B02, quatrelle_model::pim::NOTE_MIN_WIDTH);
    x.note_height = side(0x8B03, quatrelle_model::pim::NOTE_MIN_HEIGHT);
    x.modified = p(0x3008).and_then(|v| v.time());
    x.sequence = match n(PSETID_APPOINTMENT, 0x8201) {
        Some(Val::Int(s)) => u32::try_from(s).unwrap_or(0),
        _ => 0,
    };
    if let Some(Val::Texts(names)) = named.keywords().and_then(p) {
        for name in names.iter().map(|c| c.trim()).filter(|c| !c.is_empty()) {
            if x.categories.len() < MAX_CATEGORIES && !x.categories.iter().any(|c| c == name) {
                x.categories.push(name.to_string());
            }
        }
    }
    x.busy = match n(PSETID_APPOINTMENT, 0x8205) {
        Some(Val::Int(b)) => Some(b),
        _ => None,
    };
    x.sensitivity = match p(0x0036) {
        Some(Val::Int(s @ 1..=3)) => u8::try_from(s).unwrap_or(0),
        _ => 0,
    };
    // A reminder that's set: so many minutes ahead, or Outlook's 15 when it doesn't say.
    if matches!(n(PSETID_COMMON, 0x8503), Some(Val::Bool(true))) {
        x.reminder = match n(PSETID_COMMON, 0x8501) {
            Some(Val::Int(m)) if (0..=MAX_MINUTES).contains(&m) => Some(m),
            _ => Some(15),
        };
        x.reminder_at =
            [0x8560, 0x8502].into_iter().find_map(|lid| n(PSETID_COMMON, lid).and_then(|v| v.time())).filter(|t| (0..=LAST_TIME).contains(t));
    }
    x.replaces = n(PSETID_APPOINTMENT, 0x8228).and_then(|v| v.time());
    x.members = groups::members(n(PSETID_ADDRESS, 0x8054), n(PSETID_ADDRESS, 0x8055));
    x.log_start = n(PSETID_LOG, 0x8706).and_then(|v| v.time());
    x.log_end = n(PSETID_LOG, 0x8708).and_then(|v| v.time()).or_else(|| match (x.log_start, n(PSETID_LOG, 0x8707)) {
        (Some(start), Some(Val::Int(minutes))) if (0..=MAX_MINUTES).contains(&minutes) => Some(start.saturating_add(minutes * 60)),
        _ => None,
    });
    x
}

/// A journal entry as a note's text: its subject, when it was (in UTC), then its own text.
/// (Quatrelle has no journal.)
pub(crate) fn journal_text(x: &Extra, subject: &str, body: &str) -> String {
    let at = |t: i64| {
        let (y, m, d) = civil_from_days(t.div_euclid(86_400));
        let s = t.rem_euclid(86_400);
        (format!("{y:04}-{m:02}-{d:02}"), format!("{:02}:{:02}", s / 3600, s % 3600 / 60))
    };
    let when = x.log_start.map(|start| {
        let (day, time) = at(start);
        match x.log_end.filter(|e| *e > start).map(at) {
            Some((end_day, end)) if end_day == day => format!("{day} {time}–{end} UTC"),
            Some((end_day, end)) => format!("{day} {time} – {end_day} {end} UTC"),
            None => format!("{day} {time} UTC"),
        }
    });
    let body = body.replace("\r\n", "\n");
    [subject.trim().to_string(), when.unwrap_or_default(), body.trim().to_string()]
        .into_iter()
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// A day kept as the start of the day where it was set, in UTC, as `YYYY-MM-DD`.
fn day_of(t: i64) -> String {
    let (y, m, d) = civil_from_days((t + 12 * 3600 - 1).div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// The contact as a card, with what an export kept in its text (a card of what Outlook has no
/// place for, `rest`) back where it was.
pub(crate) fn card_of(x: &Extra, body: &str, uid: String) -> Card {
    let name = if x.display.trim().is_empty() { format!("{} {}", x.given, x.family).trim().to_string() } else { x.display.trim().to_string() };
    let (note, rest) = split_rest(&body.replace("\r\n", "\n"), "VCARD");
    let text = |s: &str| s.trim().to_string();
    let mut card = Card {
        uid,
        name,
        given: x.given.clone(),
        family: x.family.clone(),
        emails: x.emails.iter().map(|e| Typed { value: e.clone(), kind: String::new() }).collect(),
        phones: x.phones.iter().map(|(v, k)| Typed { value: v.clone(), kind: (*k).to_string() }).collect(),
        org: x.company.clone(),
        department: text(&x.department),
        title: x.title.clone(),
        profession: text(&x.profession),
        office: text(&x.office),
        manager: text(&x.manager),
        assistant: text(&x.assistant),
        web: x.web.replace(['\r', '\n'], ""),
        im: x.im.replace(['\r', '\n'], ""),
        nickname: text(&x.nickname),
        spouse: text(&x.spouse),
        anniversary: x.anniversary.map(day_of).unwrap_or_default(),
        note,
        addresses: x.addresses.clone(),
        birthday: x.birthday.map(day_of).unwrap_or_default(),
        // Outlook keeps it as a hidden attachment (MS-OXOCNTC 2.2.2.1.1), read by the caller.
        photo: x.photo.as_ref().map(|(media, bytes)| Photo { media: media.clone(), base64: base64_encode(bytes) }),
        extra: Vec::new(),
    };
    let more = if rest.is_empty() {
        None
    } else {
        quatrelle_model::vcard::parse(&format!("BEGIN:VCARD\r\nVERSION:4.0\r\n{}\r\nEND:VCARD\r\n", rest.join("\r\n"))).pop()
    };
    if let Some(more) = more {
        // An address's kind, which Outlook doesn't keep, beside the address it kept.
        for e in more.emails {
            match card.emails.iter_mut().find(|c| c.value == e.value && c.kind.is_empty()) {
                Some(c) => c.kind = e.kind,
                None => card.emails.push(e),
            }
        }
        card.phones.extend(more.phones);
        card.addresses.extend(more.addresses);
        if !more.birthday.is_empty() {
            card.birthday = more.birthday;
        }
        if !more.anniversary.is_empty() {
            card.anniversary = more.anniversary;
        }
        card.extra.extend(more.extra);
    }
    card
}

/// A contact's picture, from a hidden attachment: its media type (the item's own, or what the
/// bytes say) and its bytes — a picture, of a sane size, or nothing (a hidden attachment may be
/// a signature or an OLE object).
pub(crate) fn contact_photo(hidden: bool, mime: &str, data: Option<&[u8]>) -> Option<(String, Vec<u8>)> {
    if !hidden {
        return None;
    }
    let bytes = data.filter(|b| !b.is_empty() && b.len() <= quatrelle_model::vcard::MAX_PHOTO)?;
    let mime = mime.trim().to_ascii_lowercase();
    let media = if mime.starts_with("image/") {
        mime
    } else {
        match bytes.get(..4)? {
            [0xFF, 0xD8, 0xFF, _] => "image/jpeg",
            [0x89, b'P', b'N', b'G'] => "image/png",
            [b'G', b'I', b'F', b'8'] => "image/gif",
            [b'B', b'M', ..] => "image/bmp",
            _ => return None,
        }
        .to_string()
    };
    Some((media, bytes.to_vec()))
}

/// The appointment as an event (None without a start).
pub(crate) fn event_of(x: &Extra, subject: &str, body: &str, from: &EmailAddress, attendees: &[EmailAddress], uid: String) -> Option<CalEvent> {
    let start = x.start?;
    let mut end = x.end.filter(|e| *e >= start).unwrap_or(start);
    let mut start = start;
    if x.all_day {
        // Stored as local midnights: the nearest UTC midnight is the day.
        let day = |t: i64| (t + 43_200).div_euclid(86_400) * 86_400;
        start = day(start);
        end = day(end).max(start + 86_400);
    }
    let tzid = if x.all_day { String::new() } else { x.tzid.clone() };
    let (rrule, exdates) = match recurrence_in(&x.recurrence, x.start.unwrap_or(start), &tzid) {
        // An all-day series' deleted days at its local midnights: their days, as its start.
        Some((r, ex)) if x.all_day => (Some(r), ex.into_iter().map(|t| t.saturating_add(43_200).div_euclid(86_400).saturating_mul(86_400)).collect()),
        Some((r, ex)) => (Some(r), ex),
        None => (None, Vec::new()),
    };
    // A name that is only the address (what a recipient without one shows) is none.
    let person = |a: &EmailAddress| {
        let name = if a.name.trim().eq_ignore_ascii_case(a.email.trim()) { String::new() } else { a.name.clone() };
        Attendee { email: a.email.clone(), name, ..Default::default() }
    };
    // How each answered, as its recipient says; asked to when the meeting asks and it hasn't.
    let guest = |a: &EmailAddress| {
        let answer = x.answers.iter().find(|(e, _)| e.eq_ignore_ascii_case(&a.email)).map(|(_, t)| *t);
        let partstat = match answer {
            Some(2) => "TENTATIVE",
            Some(3) => "ACCEPTED",
            Some(4) => "DECLINED",
            Some(5) => "NEEDS-ACTION",
            _ => "",
        };
        Attendee { partstat: partstat.into(), rsvp: x.response_requested && matches!(partstat, "" | "NEEDS-ACTION"), ..person(a) }
    };
    let from = if from.email.is_empty() { x.organizer.as_ref().unwrap_or(from) } else { from };
    Some(CalEvent {
        uid: if x.uid.is_empty() { uid } else { x.uid.clone() },
        summary: subject.to_string(),
        description: body.replace("\r\n", "\n").trim().to_string(),
        location: x.location.clone(),
        start,
        end,
        all_day: x.all_day,
        tzid,
        rrule,
        exdates,
        organizer: (!from.email.is_empty() && !attendees.is_empty()).then(|| person(from)),
        attendees: attendees.iter().map(guest).collect(),
        sequence: x.sequence,
        categories: x.categories.clone(),
        // PidLidBusyStatus: 2 (busy) is the default.
        show_as: match x.busy {
            Some(0) => ShowAs::Free,
            Some(1) => ShowAs::Tentative,
            Some(3) => ShowAs::OutOfOffice,
            Some(4) => ShowAs::WorkingElsewhere,
            _ => ShowAs::Busy,
        },
        private: x.sensitivity != 0,
        private_kind: private_kind(x.sensitivity),
        reminder: x.reminder.map(|m| m.clamp(0, quatrelle_model::ical::REMINDER_MAX_MINUTES)),
        ..Default::default()
    })
}

/// Which private an item is, from its sensitivity, as Exchange's comes in: 1 personal, 2 private,
/// 3 confidential.
pub(crate) fn private_kind(sensitivity: u8) -> PrivateKind {
    match sensitivity {
        1 => PrivateKind::Personal,
        3 => PrivateKind::Confidential,
        _ => PrivateKind::Private,
    }
}

#[cfg(test)]
pub(crate) mod tests;
