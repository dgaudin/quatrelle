// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Writing events as iCalendar: folded content lines, one `VEVENT`, a whole calendar file.

use super::*;

/// Fold a content line at 75 octets (RFC 5545 §3.1), on character boundaries.
pub(super) fn fold(line: &str, out: &mut String) {
    let mut width = 0;
    for c in line.chars() {
        let n = c.len_utf8();
        if width + n > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(c);
        width += n;
    }
    out.push_str("\r\n");
}

pub(super) fn stamp(t: i64) -> String {
    let (y, m, d) = civil_from_days(t.div_euclid(86_400));
    let s = t.rem_euclid(86_400);
    format!("{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z", s / 3600, s % 3600 / 60, s % 60)
}

pub(super) fn day(t: i64) -> String {
    let (y, m, d) = civil_from_days(t.div_euclid(86_400));
    format!("{y:04}{m:02}{d:02}")
}

fn mailbox(prop: &str, a: &Attendee) -> String {
    let mut s = prop.to_string();
    if !a.name.is_empty() {
        s.push_str(&format!(";CN=\"{}\"", a.name.replace(['"', '\r', '\n'], "")));
    }
    if !a.partstat.is_empty() {
        s.push_str(&format!(";PARTSTAT={}", a.partstat.replace(|c: char| !c.is_ascii_alphanumeric() && c != '-', "")));
    }
    if a.rsvp {
        s.push_str(";RSVP=TRUE");
    }
    if !a.role.is_empty() {
        s.push_str(&format!(";ROLE={}", a.role.replace(|c: char| !c.is_ascii_alphanumeric() && c != '-', "")));
    }
    if a.is_resource() {
        s.push_str(&format!(";CUTYPE={}", a.cutype));
    }
    s.push_str(&format!(":mailto:{}", a.email.replace(['\r', '\n'], "")));
    s
}

/// A trigger for `minutes` from the start, as a `VALARM` writes it (`-PT15M`, `PT0S`).
pub(super) fn trigger(minutes: i64) -> String {
    match minutes {
        0 => "PT0S".to_string(),
        m if m > 0 => format!("-PT{m}M"),
        m => format!("PT{}M", -m),
    }
}

/// One `VEVENT`, folded; `now` is its `DTSTAMP`.
pub fn write_event(e: &CalEvent, now: i64, out: &mut String) {
    let mut lines: Vec<String> = vec!["BEGIN:VEVENT".into(), format!("UID:{}", e.uid.replace(['\r', '\n'], "")), format!("DTSTAMP:{}", stamp(now))];
    // Timed events in a zone are written in it (repeating at its wall-clock time); others in UTC.
    let zone = zone_of(e);
    let when = |prop: &str, t: i64| match &zone {
        Some(z) => format!("{prop};TZID={}:{}", tzid_param(&e.tzid), local_stamp(z.local(t))),
        None => format!("{prop}:{}", stamp(t)),
    };
    if e.all_day {
        lines.push(format!("DTSTART;VALUE=DATE:{}", day(e.start)));
        lines.push(format!("DTEND;VALUE=DATE:{}", day(e.end.max(e.start + 86_400))));
    } else {
        lines.push(when("DTSTART", e.start));
        lines.push(when("DTEND", e.end.max(e.start)));
    }
    if let Some(r) = e.recurrence_id {
        lines.push(if e.all_day { format!("RECURRENCE-ID;VALUE=DATE:{}", day(r)) } else { when("RECURRENCE-ID", r) });
    }
    if let Some(r) = &e.rrule {
        lines.push(format!("RRULE:{}", r.replace(['\r', '\n'], "")));
    }
    for x in &e.exdates {
        lines.push(if e.all_day { format!("EXDATE;VALUE=DATE:{}", day(*x)) } else { when("EXDATE", *x) });
    }
    // A changed occurrence adds none.
    for x in e.rdates.iter().filter(|_| e.recurrence_id.is_none()) {
        lines.push(if e.all_day { format!("RDATE;VALUE=DATE:{}", day(*x)) } else { when("RDATE", *x) });
    }
    if !e.summary.is_empty() {
        lines.push(format!("SUMMARY:{}", escape(&e.summary)));
    }
    if !e.location.is_empty() {
        lines.push(format!("LOCATION:{}", escape(&e.location)));
    }
    if !e.description.is_empty() {
        lines.push(format!("DESCRIPTION:{}", escape(&e.description)));
    }
    if !e.categories.is_empty() {
        lines.push(format!("CATEGORIES:{}", e.categories.iter().map(|c| escape(c)).collect::<Vec<_>>().join(",")));
    }
    if !e.status.is_empty() {
        lines.push(format!("STATUS:{}", e.status.replace(|c: char| !c.is_ascii_alphanumeric() && c != '-', "")));
    }
    if e.sequence > 0 {
        lines.push(format!("SEQUENCE:{}", e.sequence));
    }
    lines.extend(meeting::lines(e));
    lines.extend(e.conference.as_ref().and_then(conference::line));
    if let Some(o) = &e.organizer {
        lines.push(mailbox("ORGANIZER", o));
    }
    for a in &e.attendees {
        // Answers asked for, unless the meeting asks for none.
        let a = Attendee { rsvp: a.rsvp && !e.no_responses, ..a.clone() };
        lines.push(mailbox("ATTENDEE", &a));
    }
    lines.extend(alarms::alarm_lines(e));
    // A kept line with the conference's link isn't written again.
    let repeated = conference::repeats(e);
    lines.extend(whole(e.extra.iter().enumerate().filter(|(i, _)| !repeated.contains(i)).map(|(_, x)| x), &[]));
    lines.push("END:VEVENT".into());
    for l in lines {
        fold(&l, out);
    }
}

/// A calendar file with these events (`method` for invitations: `REQUEST`, `REPLY`, `CANCEL`).
/// An invitation is stamped `now`; a stored event keeps the stamp it came with, so the copy on a
/// server still tells which invitation it was.
pub fn write(events: &[CalEvent], method: Option<&str>, now: i64) -> String {
    let mut out = String::new();
    for l in ["BEGIN:VCALENDAR", "VERSION:2.0", "PRODID:-//Quatrelle//Quatrelle//EN", "CALSCALE:GREGORIAN"] {
        fold(l, &mut out);
    }
    if let Some(m) = method {
        fold(&format!("METHOD:{}", m.replace(|c: char| !c.is_ascii_alphabetic(), "")), &mut out);
    }
    // Each zone the events are in, once: as the file that brought it defined it, else as the
    // database has it in the year of its first event.
    let mut written: Vec<&str> = Vec::new();
    for e in events {
        if written.contains(&e.tzid.as_str()) || written.len() >= 50 {
            continue;
        }
        let definition = match zone_of(e) {
            Some(EventZone::Db(z)) if e.vtimezone.is_empty() => {
                let first = events.iter().filter(|x| x.tzid == e.tzid).map(|x| x.start).min().unwrap_or(e.start);
                zone_definition(&e.tzid, z, civil_from_days(first.div_euclid(86_400)).0)
            }
            Some(_) => e.vtimezone.clone(),
            None => continue,
        };
        written.push(&e.tzid);
        // Whole, and holding no event, task or calendar: a definition cut short, or with one of
        // those in it, would swallow what's written after it.
        for l in whole(&definition, &["VEVENT", "VTODO", "VCALENDAR"]) {
            fold(&l, &mut out);
        }
    }
    for e in events {
        // An invitation doesn't carry the id of a Webex or Teams meeting kept for the organizer's
        // own use, nor when they dismissed their reminder.
        let bare;
        let e = if method.is_some() && (e.webex_id.is_some() || e.teams_meeting_id.is_some() || e.acknowledged.is_some()) {
            bare = CalEvent { webex_id: None, teams_meeting_id: None, acknowledged: None, ..e.clone() };
            &bare
        } else {
            e
        };
        let stamp = if method.is_none() && e.dtstamp > 0 { e.dtstamp } else { now };
        write_event(e, stamp, &mut out);
    }
    fold("END:VCALENDAR", &mut out);
    out
}
