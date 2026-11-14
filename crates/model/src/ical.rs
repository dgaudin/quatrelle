// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! iCalendar (RFC 5545) events: reading, writing and expanding recurrences.
//!
//! Reading is lenient (mail and servers produce all sorts): lines are unfolded, unknown
//! properties are kept verbatim so an edited event loses nothing. A time in a named zone is read
//! in it (the IANA database's when it knows the name, Windows' names included, else the file's
//! own `VTIMEZONE`), and the event keeps its zone (`TZID`, and the definition when only the file
//! had one); floating times are the viewer's. Writing puts a timed event in its zone, with the
//! zone's `VTIMEZONE`, else in UTC, and `VALUE=DATE` for all-day ones, folding lines at 75 octets.
//!
//! Recurrence (`RRULE`) covers what calendars use day to day: daily, weekly, monthly and yearly
//! with `INTERVAL`, `COUNT`, `UNTIL`, `BYDAY` (with ordinals for monthly and yearly rules),
//! `BYMONTHDAY`, `BYMONTH`, `BYSETPOS`, and `BYHOUR`, `BYMINUTE` and `BYSECOND` (the times of
//! each day, or of each hour or minute); hourly, minutely and secondly ones every so long from the
//! start, on the days `BYDAY`, `BYMONTHDAY` and `BYMONTH` allow; and `RDATE`'s added dates. A
//! series repeats at its wall-clock time in its zone
//! (each occurrence turned into UTC on its own date, so summer time doesn't move it), on that
//! zone's days; `EXDATE` removes occurrences and `RECURRENCE-ID` instances replace them, both
//! matched as instants. Expansion is capped.

use serde::{Deserialize, Serialize};

use crate::dates::{civil_from_days, days_from_civil};

mod alarms;
mod conference;
mod find;
mod meeting;
mod notes;
mod recurrence;
mod text;
mod todo;
mod whole;
mod write;
mod zones;
pub use alarms::*;
pub use conference::{CONFERENCE_LABEL_MAX, Conference};
pub use find::*;
pub use meeting::{OPTIONAL, PrivateKind, ShowAs, intended_show_as, show_as_answered, valid_webex_id};
use notes::*;
pub use notes::*;
use recurrence::*;
pub use recurrence::*;
pub use text::*;
pub use todo::write_task;
use whole::*;
use write::{day, fold, stamp, trigger};
pub use write::{write, write_event};
use zones::*;
pub use zones::{ZoneChange, ZoneRule, wall_clock, zone_rule};

/// The most occurrences produced for one event in one expansion.
const MAX_OCCURRENCES: usize = 2000;
/// The most rule iterations tried (rules that rarely match).
const MAX_STEPS: usize = 50_000;
/// The longest input read: as much as a calendar file opened is (one written with its events'
/// files, as a `.pst` import keeps them, reads back whole).
const MAX_INPUT: usize = 16 << 20;

/// Someone taking part in an event.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attendee {
    pub email: String,
    #[serde(default)]
    pub name: String,
    /// `NEEDS-ACTION`, `ACCEPTED`, `DECLINED`, `TENTATIVE`…
    #[serde(default)]
    pub partstat: String,
    /// Asked to reply.
    #[serde(default)]
    pub rsvp: bool,
    /// What kind of participant (RFC 5545 `CUTYPE`): empty for a person; `ROOM` for a room
    /// booked for the meeting, `RESOURCE` for other equipment (Exchange's resource attendees,
    /// which book themselves).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cutype: String,
    /// Its role (RFC 5545 `ROLE`): empty for a required guest, the default; [`OPTIONAL`] for an
    /// optional one (Outlook's Optional line).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub role: String,
}

impl Attendee {
    /// A room or other resource booked for the meeting rather than someone invited to it.
    pub fn is_resource(&self) -> bool {
        matches!(self.cutype.as_str(), "ROOM" | "RESOURCE")
    }
}

/// One event (or one changed instance of a recurring event, when `recurrence_id` is set).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalEvent {
    pub uid: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub location: String,
    /// Unix seconds. All-day events start at UTC midnight of their first day.
    pub start: i64,
    /// Exclusive end.
    pub end: i64,
    #[serde(default)]
    pub all_day: bool,
    /// The time zone its times were written in (`TZID`, as written; empty: UTC, or an all-day
    /// event). A recurring event repeats at the same wall-clock time there, summer or winter.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tzid: String,
    /// That zone as the file defined it (its `VTIMEZONE` lines), written back with the event.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vtimezone: Vec<String>,
    /// The recurrence rule, as written (`FREQ=WEEKLY;BYDAY=MO,WE`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rrule: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exdates: Vec<i64>,
    /// Occurrences added to the rule's (`RDATE`; a period's start), or, with no rule, to the
    /// event's own start.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rdates: Vec<i64>,
    /// The occurrence this instance replaces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recurrence_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organizer: Option<Attendee>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attendees: Vec<Attendee>,
    /// `CONFIRMED`, `TENTATIVE` or `CANCELLED` (empty: not said).
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub sequence: u32,
    /// When the organizer wrote this version (`DTSTAMP`, Unix seconds; 0: not known): of two
    /// invitations with the same `sequence`, the later one counts.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub dtstamp: i64,
    /// Its categories (`CATEGORIES`), which colour it in the calendar as Outlook colours them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    /// The reminder's distance from the start, in minutes (15: a quarter of an hour before; a
    /// negative one sounds after it started). `None` when there's none. Read from the event's
    /// `VALARM`, as Outlook's reminder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reminder: Option<i64>,
    /// When that reminder was last dismissed in a calendar program, as its alarm says
    /// (`ACKNOWLEDGED`, RFC 9074; Unix seconds): an occurrence due at or before then is answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acknowledged: Option<i64>,
    /// Properties Quatrelle doesn't read, kept as written (unfolded content lines).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra: Vec<String>,
    /// The address to join it online, when the server gives one apart from its notes (Microsoft
    /// Graph's `onlineMeeting.joinUrl`); otherwise found in its notes ([`CalEvent::join_link`]).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub join_url: String,
    /// How its time shows to others (Outlook's Show As; busy by default).
    #[serde(default, skip_serializing_if = "ShowAs::is_busy")]
    pub show_as: ShowAs,
    /// Private: others who can see the calendar see only that the time is taken.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub private: bool,
    /// Which kind of private it is, while it's private.
    #[serde(default, skip_serializing_if = "PrivateKind::is_plain")]
    pub private_kind: PrivateKind,
    /// A meeting whose guests aren't asked to answer (Outlook's Request Responses off), and one
    /// for which they can't propose another time (Allow New Time Proposals off).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_responses: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_new_times: bool,
    /// The user's own answer to it as the server keeps it (Exchange's `MyResponseType`:
    /// `ACCEPTED`, `TENTATIVE`, `DECLINED`, `NEEDS-ACTION`; empty when not said, or the user's
    /// own meeting): an invitation sent to a group the user is in names the group among its
    /// guests, not them.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub my_answer: String,
    /// The id of the Webex meeting made for it (`X-QUATRELLE-WEBEX-ID`), kept in the event's own
    /// file and never sent in an invitation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webex_id: Option<String>,
    /// The id Graph gave the Teams meeting made for it (`X-QUATRELLE-TEAMS-ID`), kept in the
    /// event's own file and never sent in an invitation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub teams_meeting_id: Option<String>,
    /// Where to join it online, as its calendar says (RFC 7986 `CONFERENCE`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conference: Option<Conference>,
    /// A Microsoft 365 appointment that is, or is to be made, a Teams meeting by Exchange Online
    /// itself (Graph's `isOnlineMeeting` with the `teamsForBusiness` provider). Kept with the
    /// event and never written in iCalendar.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub teams_online: bool,
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

impl CalEvent {
    /// The address that joins it online: the server's own, else its conference's (any web link:
    /// the calendar says it joins), else the first meeting link in its location or notes (as
    /// Outlook's Join finds a Teams or Zoom meeting).
    pub fn join_link(&self) -> Option<String> {
        if is_meeting_link(&self.join_url) {
            return Some(self.join_url.clone());
        }
        if let Some(c) = self.conference.as_ref().filter(|c| launchable_link(&c.url)) {
            return Some(c.url.clone());
        }
        links_in(&self.location).into_iter().chain(links_in(&self.description)).find(|l| is_meeting_link(l))
    }

    /// Every web address in its location and notes (and its join address, then its
    /// conference's), once each.
    pub fn links(&self) -> Vec<String> {
        let mut out = links_in(&self.location);
        for l in links_in(&self.description) {
            if !out.contains(&l) {
                out.push(l);
            }
        }
        if !self.join_url.is_empty() && !out.contains(&self.join_url) && self.join_url.starts_with("http") {
            out.insert(0, self.join_url.clone());
        }
        if let Some(c) = self.conference.as_ref().filter(|c| launchable_link(&c.url) && !out.contains(&c.url)) {
            let at = out.iter().position(|l| *l == self.join_url).map_or(0, |i| i + 1);
            out.insert(at, c.url.clone());
        }
        out.truncate(MAX_LINKS);
        out
    }
}

/// A calendar file: what it asks (`METHOD`, for invitations), its events, and, for a calendar
/// published to subscribe to, its name and how often to read it again.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ICalendar {
    pub method: Option<String>,
    pub events: Vec<CalEvent>,
    /// `NAME` (RFC 7986) or `X-WR-CALNAME`.
    pub name: Option<String>,
    /// `REFRESH-INTERVAL` (RFC 7986) or `X-PUBLISHED-TTL`, in seconds.
    pub refresh: Option<i64>,
}

/// One occurrence of an event: which event (index into the list given) and when.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Occurrence {
    pub event: usize,
    pub start: i64,
    pub end: i64,
}

// ----- reading -----

/// A content line: name, parameters (upper-case names), value.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Line {
    name: String,
    params: Vec<(String, String)>,
    value: String,
    raw: String,
}

impl Line {
    fn param(&self, n: &str) -> Option<&str> {
        self.params.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str())
    }
}

/// Unfold (RFC 5545 §3.1) and split into content lines.
fn lines(text: &str) -> Vec<Line> {
    // Cut at a character's start: a cap falling inside one would otherwise read the whole text.
    let mut end = MAX_INPUT.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let text = text.get(..end).unwrap_or(text);
    let mut unfolded: Vec<String> = Vec::new();
    for l in text.split('\n') {
        // A CR inside a line is taken out, as writing takes it out: what's read is what's written.
        let l = l.replace('\r', "");
        match (l.strip_prefix(' ').or_else(|| l.strip_prefix('\t')), unfolded.last_mut()) {
            (Some(rest), Some(prev)) => prev.push_str(rest),
            _ => unfolded.push(l),
        }
    }
    unfolded
        .into_iter()
        .filter_map(|raw| {
            parse_line(&raw).map(|mut l| {
                l.raw = raw;
                l
            })
        })
        .collect()
}

fn parse_line(raw: &str) -> Option<Line> {
    // The value starts at the first ':' outside double quotes.
    let mut quoted = false;
    let mut colon = None;
    for (i, c) in raw.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ':' if !quoted => {
                colon = Some(i);
                break;
            }
            _ => {}
        }
    }
    let colon = colon?;
    let head = raw.get(..colon)?;
    let value = raw.get(colon + 1..)?.to_string();
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut q = false;
    for c in head.chars() {
        match c {
            '"' => q = !q,
            ';' if !q => parts.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    parts.push(cur);
    let mut it = parts.into_iter();
    let name = it.next()?.trim().to_ascii_uppercase();
    if name.is_empty() {
        return None;
    }
    let params = it.filter_map(|p| p.split_once('=').map(|(k, v)| (k.trim().to_ascii_uppercase(), v.to_string()))).collect();
    Some(Line { name, params, value, raw: String::new() })
}

/// `YYYYMMDD` → days since the epoch.
fn date_days(s: &str) -> Option<i64> {
    let y: i64 = s.get(0..4)?.parse().ok()?;
    let m: u32 = s.get(4..6)?.parse().ok()?;
    let d: u32 = s.get(6..8)?.parse().ok()?;
    ((1..=12).contains(&m) && (1..=31).contains(&d) && (1..=9999).contains(&y)).then(|| days_from_civil(y, m, d))
}

/// A DATE or DATE-TIME value: (local or UTC seconds, all day, UTC).
fn date_time(v: &str) -> Option<(i64, bool, bool)> {
    let v = v.trim();
    let days = date_days(v)?;
    let Some(t) = v.get(8..).filter(|r| !r.is_empty()) else { return Some((days * 86_400, true, false)) };
    let t = t.strip_prefix('T')?;
    let h: i64 = t.get(0..2)?.parse().ok()?;
    let mi: i64 = t.get(2..4)?.parse().ok()?;
    let s: i64 = t.get(4..6)?.parse().ok()?;
    if h > 24 || mi > 59 || s > 60 {
        return None;
    }
    Some((days * 86_400 + h * 3600 + mi * 60 + s, false, t.ends_with('Z') || t.ends_with('z')))
}

/// Read a mailbox (`mailto:a@b`) and its name.
fn person(l: &Line) -> Attendee {
    let v = l.value.trim();
    let email = v.get(..7).filter(|p| p.eq_ignore_ascii_case("mailto:")).map_or(v, |_| v.get(7..).unwrap_or("")).trim().to_string();
    Attendee {
        email,
        name: l.param("CN").map(|n| n.trim_matches('"').to_string()).unwrap_or_default(),
        partstat: l.param("PARTSTAT").map(str::to_ascii_uppercase).unwrap_or_default(),
        rsvp: l.param("RSVP").is_some_and(|r| r.eq_ignore_ascii_case("TRUE")),
        // Only the kinds that matter here are kept: a person is the default.
        cutype: l.param("CUTYPE").map(str::to_ascii_uppercase).filter(|c| matches!(c.as_str(), "ROOM" | "RESOURCE")).unwrap_or_default(),
        role: meeting::role_of(l.param("ROLE")),
    }
}

/// Read a calendar file. `local_offset` (seconds east of UTC) places floating times; better,
/// [`parse_in`] the viewer's zone.
pub fn parse(text: &str, local_offset: i64) -> ICalendar {
    let off = i32::try_from(local_offset.clamp(-14 * 3600, 14 * 3600)).unwrap_or(0);
    parse_in(text, crate::zone::Zone::Fixed(off))
}

/// Read a calendar file. Times in a named zone are read in it (the IANA database's when it knows
/// the name, else the file's `VTIMEZONE`), and the event keeps the zone; floating times, and
/// zones nobody defines, are in `local` (the viewer's).
pub fn parse_in(text: &str, local: crate::zone::Zone) -> ICalendar {
    let all = lines(text);
    let mut cal = ICalendar::default();
    // Zones first: events can come before the zones they use.
    let zones = FileZones::read(&all, local);
    // An exception's times wait for the event's zone (DTSTART may come after them).
    let mut pending: Vec<&Line> = Vec::new();
    // The event's Microsoft busy status was read (`ical/meeting.rs`).
    let mut busy_said = false;
    let mut ev: Option<(CalEvent, Option<i64>)> = None;
    let mut depth_in_event = 0usize;
    // The `VALARM` being read, as written: its trigger becomes the reminder, and an alarm
    // Quatrelle can't read is kept whole.
    let mut alarm: Option<Vec<String>> = None;
    for l in &all {
        let n = l.name.as_str();
        if n == "BEGIN" {
            let what = l.value.trim().to_ascii_uppercase();
            if what == "VEVENT" && ev.is_none() {
                ev = Some((CalEvent::default(), None));
                depth_in_event = 0;
                pending.clear();
                busy_said = false;
                continue;
            }
            if ev.is_some() {
                depth_in_event += 1;
                if what == "VALARM" && depth_in_event == 1 && alarm.is_none() {
                    alarm = Some(vec![l.raw.clone()]);
                } else if let Some(a) = alarm.as_mut() {
                    a.push(l.raw.clone());
                } else if let Some((e, _)) = ev.as_mut() {
                    e.extra.push(l.raw.clone());
                }
            }
            continue;
        }
        if n == "END" {
            if ev.is_some() && depth_in_event > 0 {
                depth_in_event -= 1;
                if let Some(lines) = alarm.as_mut() {
                    lines.push(l.raw.clone());
                    if depth_in_event == 0 {
                        let lines = alarm.take().unwrap_or_default();
                        match alarm_minutes(&lines) {
                            Some(m) => {
                                if let Some((e, _)) = ev.as_mut() {
                                    e.reminder = Some(m);
                                    e.acknowledged = e.acknowledged.max(alarms::acknowledged_in(&lines));
                                }
                            }
                            None => {
                                if let Some((e, _)) = ev.as_mut() {
                                    e.extra.extend(lines);
                                }
                            }
                        }
                    }
                } else if let Some((e, _)) = ev.as_mut() {
                    e.extra.push(l.raw.clone());
                }
                continue;
            }
            if l.value.trim().eq_ignore_ascii_case("VEVENT")
                && let Some((mut e, dur)) = ev.take()
            {
                for x in pending.drain(..) {
                    if x.name == "RECURRENCE-ID" {
                        e.recurrence_id = zones.time(x, &e.tzid).map(|(t, _, _)| t);
                        continue;
                    }
                    // At most 1,000 of each, from all their lines; a period (`RDATE;VALUE=PERIOD`)
                    // by its start.
                    let list = if x.name == "RDATE" { &mut e.rdates } else { &mut e.exdates };
                    let room = 1000usize.saturating_sub(list.len());
                    list.extend(zones.times(x, &e.tzid, room));
                }
                if e.end <= e.start {
                    e.end = match dur {
                        Some(d) => e.start.saturating_add(d.max(0)),
                        None if e.all_day => e.start + 86_400,
                        None => e.start,
                    };
                }
                if cal.events.len() < 20_000 {
                    cal.events.push(e);
                }
            }
            continue;
        }
        let Some((e, dur)) = ev.as_mut() else {
            match n {
                "METHOD" => cal.method = Some(l.value.trim().to_ascii_uppercase()),
                "NAME" | "X-WR-CALNAME" if cal.name.is_none() => {
                    let name: String = unescape(&l.value).chars().filter(|c| !c.is_control()).take(200).collect();
                    cal.name = Some(name.trim().to_string()).filter(|s| !s.is_empty());
                }
                "REFRESH-INTERVAL" | "X-PUBLISHED-TTL" if cal.refresh.is_none() => cal.refresh = duration(l.value.trim()).filter(|d| *d > 0),
                _ => {}
            }
            continue;
        };
        if depth_in_event > 0 {
            match alarm.as_mut() {
                Some(lines) => lines.push(l.raw.clone()),
                None => e.extra.push(l.raw.clone()),
            }
            continue;
        }
        match n {
            "UID" => e.uid = l.value.trim().to_string(),
            "SUMMARY" => e.summary = unescape(&l.value),
            "DESCRIPTION" => e.description = unescape(&l.value),
            "LOCATION" => e.location = unescape(&l.value),
            "CATEGORIES" => {
                for c in split_text_list(&l.value) {
                    if !c.is_empty() && !e.categories.contains(&c) && e.categories.len() < 20 {
                        e.categories.push(c);
                    }
                }
            }
            "STATUS" => e.status = l.value.trim().to_ascii_uppercase(),
            "SEQUENCE" => e.sequence = l.value.trim().parse().unwrap_or(0),
            "DTSTART" => {
                if let Some((t, all_day, tzid)) = zones.time(l, "") {
                    e.start = t;
                    e.all_day = all_day;
                    e.vtimezone = zones.definition(&tzid);
                    e.tzid = tzid;
                }
            }
            "DTEND" => {
                if let Some((t, _, _)) = zones.time(l, "") {
                    e.end = t;
                }
            }
            "DURATION" => *dur = duration(&l.value),
            "RRULE" => e.rrule = Some(l.value.trim().to_string()),
            "EXDATE" | "RDATE" | "RECURRENCE-ID" => {
                if pending.len() < 1000 {
                    pending.push(l);
                }
            }
            "ORGANIZER" => e.organizer = Some(person(l)),
            "ATTENDEE" => {
                if e.attendees.len() < 500 {
                    e.attendees.push(person(l));
                }
            }
            "DTSTAMP" => e.dtstamp = zones.time(l, "").map_or(0, |(t, _, _)| t),
            "CREATED" | "LAST-MODIFIED" => {}
            // A line that isn't the conference is kept as it came.
            "CONFERENCE" if conference::take(e, l) => {}
            _ if meeting::read(e, n, &l.value, &mut busy_said) => {}
            _ => {
                if e.extra.len() < 200 {
                    e.extra.push(l.raw.clone());
                }
            }
        }
    }
    // An invitation names no Webex or Teams meeting of the user's: whatever id it carries is a
    // stranger's.
    if cal.method.is_some() {
        for e in &mut cal.events {
            e.webex_id = None;
            e.teams_meeting_id = None;
        }
    }
    cal
}

/// The minutes a `VALARM`'s trigger is from its start, when it says so plainly (`-PT15M`,
/// `-P1D`…); `None` for an absolute time or one Quatrelle can't read, whose alarm is then kept
/// as written.
fn alarm_minutes(lines: &[String]) -> Option<i64> {
    for line in lines {
        let line = line.trim();
        if let Some(v) = line.get(..8).filter(|v| v.eq_ignore_ascii_case("TRIGGER;")) {
            // Parameters (RELATED=END, VALUE=DATE-TIME) aren't read: the alarm is kept whole.
            let _ = v;
            return None;
        }
        if let Some(v) = line.strip_prefix("TRIGGER:") {
            return duration_minutes(v);
        }
    }
    None
}

/// `-PT15M` and its like, in minutes; `None` when it isn't a duration Quatrelle reads.
fn duration_minutes(v: &str) -> Option<i64> {
    let mut s = v.trim();
    let sign = match s.chars().next() {
        Some('-') => {
            s = s.get(1..)?;
            -1
        }
        Some('+') => {
            s = s.get(1..)?;
            1
        }
        _ => 1,
    };
    s = s.strip_prefix('P')?;
    let (mut date, mut time) = match s.split_once('T') {
        Some((d, t)) => (d, Some(t)),
        None => (s, None),
    };
    let mut secs: i64 = 0;
    while !date.is_empty() {
        let n = take_unit(&mut date, 'D')?;
        secs = secs.checked_add(n.checked_mul(86_400)?)?;
    }
    if let Some(t) = time.as_mut() {
        while !t.is_empty() {
            let n = if let Some(h) = take_unit(t, 'H') {
                h.checked_mul(3_600)?
            } else if let Some(m) = take_unit(t, 'M') {
                m.checked_mul(60)?
            } else {
                take_unit(t, 'S')?
            };
            secs = secs.checked_add(n)?;
        }
    }
    // `-PT15M` is a quarter of an hour before the start: minutes before it are positive here.
    let minutes = -sign * (secs / 60);
    // A fortnight either way is as far as an alarm is ever set.
    (minutes.abs() <= 20_160).then_some(minutes)
}

/// The number before `unit` at the front of `s` (`30` in `30M`), `s` keeping what follows.
fn take_unit(s: &mut &str, unit: char) -> Option<i64> {
    let i = s.find(unit)?;
    let n: i64 = s.get(..i)?.parse().ok()?;
    *s = s.get(i + unit.len_utf8()..)?;
    Some(n)
}

// ----- recurrence -----

#[cfg(test)]
mod tests;
