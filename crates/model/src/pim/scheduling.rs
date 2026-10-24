//! Scheduling a meeting: the rooms a server offers (Outlook's Room Finder) and when people and
//! rooms are free (its Scheduling Assistant). Pure data: the network layer fills it from
//! Exchange (EWS `GetRoomLists`/`GetRooms`/`GetUserAvailability`) or Microsoft Graph
//! (`findRoomLists`/`findRooms`/`getSchedule`), the engine from the calendars it has here.

use serde::{Deserialize, Serialize};

/// A room (or other bookable resource) with a mailbox of its own.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Room {
    pub email: String,
    pub name: String,
    /// The room list it came from (its address or name as the server gives it); empty for a
    /// room known from the calendar rather than from a list.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub list: String,
}

/// A room list (a group of rooms the organisation set up: a building, a floor).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomList {
    pub email: String,
    pub name: String,
}

/// How a stretch of someone's time is taken, as Exchange tells it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BusyKind {
    Free,
    Tentative,
    #[default]
    Busy,
    OutOfOffice,
    WorkingElsewhere,
    /// The server has no information for that time (or wouldn't say).
    NoData,
}

impl BusyKind {
    /// As Exchange's `BusyType` and Graph's `status` name it (also the merged view's digit).
    pub fn parse(s: &str) -> BusyKind {
        match s.trim() {
            "Free" | "free" | "0" => BusyKind::Free,
            "Tentative" | "tentative" | "1" => BusyKind::Tentative,
            "Busy" | "busy" | "2" => BusyKind::Busy,
            "OOF" | "oof" | "3" => BusyKind::OutOfOffice,
            "WorkingElsewhere" | "workingElsewhere" => BusyKind::WorkingElsewhere,
            _ => BusyKind::NoData,
        }
    }

    /// Its name in commands (`busy`, `outOfOffice`…).
    pub fn code(self) -> &'static str {
        match self {
            BusyKind::Free => "free",
            BusyKind::Tentative => "tentative",
            BusyKind::Busy => "busy",
            BusyKind::OutOfOffice => "outOfOffice",
            BusyKind::WorkingElsewhere => "workingElsewhere",
            BusyKind::NoData => "noData",
        }
    }
}

/// A stretch of someone's time, as their calendar has it taken.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusySlot {
    /// Unix seconds.
    pub start: i64,
    pub end: i64,
    pub kind: BusyKind,
    /// What takes it, when the server says (a detailed view the person allows).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub subject: String,
}

/// When one person or room is busy over a window of time.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Availability {
    pub email: String,
    pub slots: Vec<BusySlot>,
    /// Why nothing could be read for them (no rights to their free/busy, unknown address…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Rooms and room lists read at most.
pub const MAX_ROOMS: usize = 2000;
/// Busy stretches kept for one person over one window at most.
pub const MAX_SLOTS: usize = 2000;
/// People and rooms asked about in one request at most.
pub const MAX_PEOPLE: usize = 100;
/// The longest window availability is asked for (Exchange answers 62 days at most).
pub const MAX_WINDOW: i64 = 62 * 86_400;

/// Whether a calendar group's name is the one Outlook gives the group it keeps room calendars
/// in ("Rooms", or its name in the mailbox's language), as the owner's "Salles". A server with
/// no room lists set up offers these calendars' mailboxes as rooms.
pub fn is_rooms_group(name: &str) -> bool {
    const NAMES: [&str; 20] = [
        "rooms",
        "room",
        "salles",
        "salle",
        "salas",
        "sala",
        "räume",
        "raeume",
        "raum",
        "sale",
        "stanze",
        "zalen",
        "ruimtes",
        "комнаты",
        "переговорные",
        "会議室",
        "회의실",
        "会议室",
        "會議室",
        "غرف",
    ];
    let n = name.trim().to_lowercase();
    !n.is_empty() && NAMES.contains(&n.as_str())
}

/// Merge the stretches of time that touch or overlap, by kind, so a day of back-to-back
/// meetings is drawn as one block (and the list stays small); sorted by start.
pub fn merge_slots(mut slots: Vec<BusySlot>) -> Vec<BusySlot> {
    slots.retain(|s| s.end > s.start);
    // Kind by kind, so a tentative hour inside a busy day stays its own block.
    slots.sort_by_key(|s| (s.kind.code(), s.start, s.end));
    let mut out: Vec<BusySlot> = Vec::new();
    for s in slots {
        match out.last_mut() {
            Some(last) if last.kind == s.kind && s.start <= last.end && (last.subject.is_empty() || last.subject == s.subject) => {
                last.end = last.end.max(s.end);
                if last.subject.is_empty() {
                    last.subject = s.subject;
                }
            }
            _ => out.push(s),
        }
    }
    out.sort_by_key(|s| (s.start, s.end));
    out.truncate(MAX_SLOTS);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_parse_both_ways() {
        for (s, k) in [("Busy", BusyKind::Busy), ("tentative", BusyKind::Tentative), ("OOF", BusyKind::OutOfOffice), ("3", BusyKind::OutOfOffice)] {
            assert_eq!(BusyKind::parse(s), k);
        }
        assert_eq!(BusyKind::parse("nonsense"), BusyKind::NoData);
        assert_eq!(BusyKind::parse(BusyKind::WorkingElsewhere.code()), BusyKind::WorkingElsewhere);
    }

    #[test]
    fn rooms_groups_by_name() {
        assert!(is_rooms_group("Salles") && is_rooms_group(" rooms ") && is_rooms_group("Räume"));
        assert!(!is_rooms_group("Équipe : Studio") && !is_rooms_group(""));
    }

    #[test]
    fn slots_merge_when_they_touch() {
        let s = |a: i64, b: i64, kind: BusyKind| BusySlot { start: a, end: b, kind, subject: String::new() };
        let merged = merge_slots(vec![
            s(10, 20, BusyKind::Busy),
            s(0, 10, BusyKind::Busy),
            s(25, 30, BusyKind::Busy),
            s(5, 8, BusyKind::Tentative),
            s(9, 3, BusyKind::Busy),
        ]);
        assert_eq!(merged.len(), 3);
        assert_eq!((merged[0].start, merged[0].end, merged[0].kind), (0, 20, BusyKind::Busy));
        assert_eq!((merged[1].start, merged[1].end, merged[1].kind), (5, 8, BusyKind::Tentative));
        assert_eq!((merged[2].start, merged[2].end), (25, 30));
    }
}
