// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Time zones: the IANA database (built in through `jiff-tzdb`, so it reads no system files and
//! works the same everywhere, wasm included; its rules read by `jiff`) and fixed offsets. A zone
//! turns a local wall-clock time into Unix seconds and back, with the offset in force on that date
//! (summer time included).

use std::cell::RefCell;

use jiff::Timestamp;
use jiff::tz::{AmbiguousOffset, Offset, TimeZone};

/// A time zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    /// Seconds east of UTC, all year.
    Fixed(i32),
    /// A zone of the IANA database, by its name there (`Europe/Berlin`).
    Named(&'static str),
}

/// Zones whose rules are kept read, per thread, at most (a calendar shows a handful).
const RULES_KEPT: usize = 64;

/// The rules of zone `name` (a name of the database built in), read once per thread and kept: a
/// calendar asks for an offset for every event it draws.
fn rules(name: &'static str) -> Option<TimeZone> {
    thread_local! {
        static READ: RefCell<Vec<(&'static str, TimeZone)>> = const { RefCell::new(Vec::new()) };
    }
    let kept = READ.with(|r| r.try_borrow().ok().and_then(|r| r.iter().find(|(n, _)| *n == name).map(|(_, tz)| tz.clone())));
    if kept.is_some() {
        return kept;
    }
    let (canonical, data) = jiff_tzdb::get(name)?;
    let tz = TimeZone::tzif(canonical, data).ok()?;
    READ.with(|r| {
        if let Ok(mut r) = r.try_borrow_mut()
            && r.len() < RULES_KEPT
        {
            r.push((name, tz.clone()));
        }
    });
    Some(tz)
}

/// A zone of the database built in, by any spelling of its name (`america/new_york` too).
fn in_database(name: &str) -> Option<Zone> {
    jiff_tzdb::get(name).map(|(canonical, _)| Zone::Named(canonical))
}

impl Default for Zone {
    fn default() -> Zone {
        Zone::UTC
    }
}

/// Outlook and Exchange name zones the Windows way in `TZID` (`W. Europe Standard Time`).
/// Each Windows name maps to the IANA zone Unicode CLDR picks for it (`windowsZones.xml`,
/// territory "001"); the facts are CLDR's. Each entry is (IANA, Windows), sorted by the Windows name.
const WINDOWS: &[(&str, &str)] = &[
    ("America/Anchorage", "Alaskan Standard Time"),
    ("Asia/Riyadh", "Arab Standard Time"),
    ("Asia/Dubai", "Arabian Standard Time"),
    ("America/Argentina/Buenos_Aires", "Argentina Standard Time"),
    ("America/Halifax", "Atlantic Standard Time"),
    ("Australia/Darwin", "AUS Central Standard Time"),
    ("Australia/Sydney", "AUS Eastern Standard Time"),
    ("Asia/Dhaka", "Bangladesh Standard Time"),
    ("America/Regina", "Canada Central Standard Time"),
    ("Australia/Adelaide", "Cen. Australia Standard Time"),
    ("America/Guatemala", "Central America Standard Time"),
    ("Europe/Budapest", "Central Europe Standard Time"),
    ("Europe/Warsaw", "Central European Standard Time"),
    ("America/Chicago", "Central Standard Time"),
    ("America/Mexico_City", "Central Standard Time (Mexico)"),
    ("Asia/Shanghai", "China Standard Time"),
    ("Etc/GMT+12", "Dateline Standard Time"),
    ("Africa/Nairobi", "E. Africa Standard Time"),
    ("Australia/Brisbane", "E. Australia Standard Time"),
    ("Europe/Chisinau", "E. Europe Standard Time"),
    ("America/Sao_Paulo", "E. South America Standard Time"),
    ("America/New_York", "Eastern Standard Time"),
    ("Africa/Cairo", "Egypt Standard Time"),
    ("Europe/Kyiv", "FLE Standard Time"),
    ("Europe/London", "GMT Standard Time"),
    ("Atlantic/Reykjavik", "Greenwich Standard Time"),
    ("Europe/Bucharest", "GTB Standard Time"),
    ("Pacific/Honolulu", "Hawaiian Standard Time"),
    ("Asia/Kolkata", "India Standard Time"),
    ("Asia/Tehran", "Iran Standard Time"),
    ("Asia/Jerusalem", "Israel Standard Time"),
    ("Asia/Seoul", "Korea Standard Time"),
    ("America/Denver", "Mountain Standard Time"),
    ("Asia/Kathmandu", "Nepal Standard Time"),
    ("Pacific/Auckland", "New Zealand Standard Time"),
    ("America/St_Johns", "Newfoundland Standard Time"),
    ("America/Los_Angeles", "Pacific Standard Time"),
    ("America/Tijuana", "Pacific Standard Time (Mexico)"),
    ("Asia/Karachi", "Pakistan Standard Time"),
    ("Europe/Paris", "Romance Standard Time"),
    ("Europe/Moscow", "Russian Standard Time"),
    ("America/Cayenne", "SA Eastern Standard Time"),
    ("America/Bogota", "SA Pacific Standard Time"),
    ("Asia/Bangkok", "SE Asia Standard Time"),
    ("Asia/Singapore", "Singapore Standard Time"),
    ("Africa/Johannesburg", "South Africa Standard Time"),
    ("Asia/Taipei", "Taipei Standard Time"),
    ("Asia/Tokyo", "Tokyo Standard Time"),
    ("Europe/Istanbul", "Turkey Standard Time"),
    ("America/Indiana/Indianapolis", "US Eastern Standard Time"),
    ("America/Phoenix", "US Mountain Standard Time"),
    ("Etc/UTC", "UTC"),
    ("Etc/GMT+11", "UTC-11"),
    ("Australia/Perth", "W. Australia Standard Time"),
    ("Africa/Lagos", "W. Central Africa Standard Time"),
    ("Europe/Berlin", "W. Europe Standard Time"),
];

impl Zone {
    pub const UTC: Zone = Zone::Fixed(0);

    /// A zone by name: an IANA name (`Europe/Berlin`), a Windows name (`W. Europe Standard
    /// Time`), a path ending in an IANA name (`/mozilla.org/20070129_1/Europe/Berlin`), or a
    /// fixed offset (`+0530`, `-05:00`, as [`Zone::id`] writes one).
    pub fn named(id: &str) -> Option<Zone> {
        let id = id.trim().trim_matches('"').trim();
        if id.is_empty() || id.len() > 200 {
            return None;
        }
        if let Some(o) = fixed_offset(id) {
            return Some(Zone::Fixed(o));
        }
        if ["UTC", "GMT", "Z", "UT", "Etc/UTC", "Etc/GMT"].iter().any(|u| id.eq_ignore_ascii_case(u)) {
            return Some(Zone::UTC);
        }
        if let Some(zone) = in_database(id) {
            return Some(zone);
        }
        if let Some((iana, _)) = WINDOWS.iter().find(|(_, w)| w.eq_ignore_ascii_case(id)) {
            return in_database(iana);
        }
        // A prefixed name: the longest tail that is a zone.
        let mut rest = id;
        for _ in 0..8 {
            let Some((_, tail)) = rest.split_once('/') else { break };
            if let Some(zone) = in_database(tail) {
                return Some(zone);
            }
            rest = tail;
        }
        None
    }

    /// Its name as [`Zone::named`] reads it back: the IANA name, or the offset (`+0530`).
    pub fn id(&self) -> String {
        match self {
            Zone::Named(name) => (*name).to_string(),
            Zone::Fixed(o) => {
                let a = o.unsigned_abs();
                format!("{}{:02}{:02}", if *o < 0 { '-' } else { '+' }, a / 3600, a % 3600 / 60)
            }
        }
    }

    /// Its IANA name, if it has one.
    pub fn name(&self) -> Option<&'static str> {
        match self {
            Zone::Named(name) => Some(name),
            Zone::Fixed(_) => None,
        }
    }

    /// Its Windows name (`Romance Standard Time`), as Exchange names zones, when it's the zone
    /// CLDR gives a Windows name.
    pub fn windows_name(&self) -> Option<&'static str> {
        let name = self.name()?;
        WINDOWS.iter().find(|(iana, _)| *iana == name).map(|(_, w)| *w)
    }

    /// The offset (seconds east of UTC) in force at the instant `utc` (Unix seconds).
    pub fn offset_at(&self, utc: i64) -> i64 {
        match self {
            Zone::Fixed(o) => i64::from(*o),
            Zone::Named(name) => match (rules(name), Timestamp::from_second(utc)) {
                (Some(tz), Ok(at)) => i64::from(tz.to_offset(at).seconds()),
                _ => 0,
            },
        }
    }

    /// Whether summer (daylight saving) time is in force at the instant `utc`.
    pub fn is_dst(&self, utc: i64) -> bool {
        match self {
            Zone::Fixed(_) => false,
            Zone::Named(name) => match (rules(name), Timestamp::from_second(utc)) {
                (Some(tz), Ok(at)) => tz.to_offset_info(at).dst().is_dst(),
                _ => false,
            },
        }
    }

    /// The local wall-clock time (seconds since the epoch, as if it were UTC) of the instant `utc`.
    pub fn local(&self, utc: i64) -> i64 {
        utc.saturating_add(self.offset_at(utc))
    }

    /// The instant (Unix seconds) of a local wall-clock time. A time that happens twice (when the
    /// clocks go back) is the first; one that doesn't happen (when they go forward) is read with
    /// the offset before the change, as RFC 5545 §3.3.5 says (02:30 becomes 03:30).
    pub fn to_utc(&self, local: i64) -> i64 {
        match self {
            Zone::Fixed(o) => local.saturating_sub(i64::from(*o)),
            Zone::Named(name) => {
                let (Some(tz), Ok(wall)) = (rules(name), Timestamp::from_second(local)) else { return local };
                let offset = match tz.to_ambiguous_timestamp(Offset::UTC.to_datetime(wall)).offset() {
                    AmbiguousOffset::Unambiguous { offset } => offset,
                    // Twice (the clocks go back): the first, in the offset before the change.
                    AmbiguousOffset::Fold { before, .. } => before,
                    // Never (they go forward): read with the offset before the change.
                    AmbiguousOffset::Gap { before, .. } => before,
                };
                local.saturating_sub(i64::from(offset.seconds()))
            }
        }
    }
}

/// `+0530`, `-05:00`, `+05` → seconds east.
fn fixed_offset(s: &str) -> Option<i32> {
    let sign = match s.get(..1)? {
        "+" => 1,
        "-" => -1,
        _ => return None,
    };
    let digits: String = s.get(1..)?.chars().filter(|c| *c != ':').collect();
    if !digits.bytes().all(|b| b.is_ascii_digit()) || !matches!(digits.len(), 2 | 4) {
        return None;
    }
    let h: i32 = digits.get(..2)?.parse().ok()?;
    let m: i32 = digits.get(2..).filter(|m| !m.is_empty()).map_or(Some(0), |m| m.parse().ok())?;
    (h <= 14 && m < 60).then_some(sign * (h * 3600 + m * 60))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dates::days_from_civil;

    fn at(y: i64, m: u32, d: u32, h: i64, mi: i64) -> i64 {
        days_from_civil(y, m, d) * 86_400 + h * 3600 + mi * 60
    }

    #[test]
    fn names_and_offsets() {
        let ny = Zone::named("America/New_York").unwrap();
        assert_eq!(ny.name(), Some("America/New_York"));
        assert_eq!(Zone::named("Eastern Standard Time"), Some(ny), "Windows' name");
        assert_eq!(Zone::named("/mozilla.org/20070129_1/America/New_York"), Some(ny));
        assert_eq!(Zone::named("\"America/New_York\""), Some(ny));
        assert_eq!(Zone::named("utc"), Some(Zone::UTC));
        assert_eq!(Zone::named("+0530"), Some(Zone::Fixed(19_800)));
        assert_eq!(Zone::named(&Zone::Fixed(-16_200).id()), Some(Zone::Fixed(-16_200)));
        assert_eq!(Zone::named(&ny.id()), Some(ny));
        for junk in ["", "Mars/Olympus", "+99", "+", "/", "//////////", &"a/".repeat(500), "-0x:00", "+é"] {
            assert_eq!(Zone::named(junk), None, "{junk}");
        }
        // Every Windows name stands for a zone of the database.
        for (iana, w) in WINDOWS {
            assert!(Zone::named(iana).is_some() && Zone::named(w).is_some(), "{w} → {iana}");
        }
    }

    #[test]
    fn summer_time() {
        let ny = Zone::named("America/New_York").unwrap();
        assert_eq!(ny.offset_at(at(2026, 1, 5, 14, 0)), -5 * 3600);
        assert_eq!(ny.offset_at(at(2026, 7, 6, 13, 0)), -4 * 3600);
        assert!(ny.is_dst(at(2026, 7, 6, 13, 0)) && !ny.is_dst(at(2026, 1, 5, 14, 0)));
        assert_eq!(ny.to_utc(at(2026, 7, 6, 9, 0)), at(2026, 7, 6, 13, 0));
        assert_eq!(ny.local(at(2026, 7, 6, 13, 0)), at(2026, 7, 6, 9, 0));
        // 02:30 on 8 March 2026 doesn't happen in New York: it's 03:30 summer time.
        assert_eq!(ny.to_utc(at(2026, 3, 8, 2, 30)), at(2026, 3, 8, 7, 30));
        // 01:30 on 1 November 2026 happens twice: the first.
        assert_eq!(ny.to_utc(at(2026, 11, 1, 1, 30)), at(2026, 11, 1, 5, 30));
        // Paris: 02:30 on 29 March 2026 doesn't happen (03:30 summer time); 02:30 on 25 October
        // happens twice (the first, in summer time).
        let paris = Zone::named("europe/paris").unwrap();
        assert_eq!(paris.name(), Some("Europe/Paris"), "any spelling, kept as the database spells it");
        assert_eq!(paris.to_utc(at(2026, 3, 29, 2, 30)), at(2026, 3, 29, 1, 30));
        assert_eq!(paris.to_utc(at(2026, 10, 25, 2, 30)), at(2026, 10, 25, 0, 30));
        // The southern summer, a half-hour zone without one, and a fixed offset.
        let sydney = Zone::named("Australia/Sydney").unwrap();
        assert_eq!((sydney.offset_at(at(2026, 1, 15, 0, 0)), sydney.offset_at(at(2026, 7, 15, 0, 0))), (11 * 3600, 10 * 3600));
        assert!(sydney.is_dst(at(2026, 1, 15, 0, 0)) && !sydney.is_dst(at(2026, 7, 15, 0, 0)));
        let kolkata = Zone::named("India Standard Time").unwrap();
        assert_eq!((kolkata.offset_at(at(2026, 7, 1, 0, 0)), kolkata.is_dst(at(2026, 7, 1, 0, 0))), (19_800, false));
        assert_eq!(Zone::Fixed(-16_200).to_utc(at(2026, 7, 1, 0, 0)), at(2026, 7, 1, 4, 30));
        // Hostile values never panic.
        for t in [i64::MIN, i64::MAX, 0, -1] {
            let _ = (ny.offset_at(t), ny.to_utc(t), ny.local(t), ny.is_dst(t), Zone::Fixed(i32::MIN).to_utc(t));
        }
    }
}
