// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! A meeting request, cancellation or answer (`IPM.Schedule.Meeting.*`, MS-OXOCAL 2.2.6) as the
//! calendar it carried (RFC 6047), so that it can be answered or applied from the reading pane.

use super::*;

/// A meeting request, cancellation or answer (of class `class`) as the calendar it carried: its
/// method and its text, stamped when it was sent (`date`). A request and a cancellation go from
/// their organizer (`from`) to `guests`; an answer from its guest (`from`) to the organizer (the
/// first of `guests`). A request for a repeating meeting carries its changed occurrences too, as
/// its pattern has them. `None` for another kind (a forward's notice) or one with no time.
#[allow(clippy::too_many_arguments)]
pub(crate) fn invitation(
    x: &Extra,
    class: &str,
    subject: &str,
    body: &str,
    from: &EmailAddress,
    guests: &[EmailAddress],
    date: i64,
    uid: String,
) -> Option<(&'static str, String)> {
    let kind = class.to_ascii_lowercase();
    let kind = kind.strip_prefix("ipm.schedule.meeting.")?;
    let (method, answer) = if kind.starts_with("request") {
        ("REQUEST", None)
    } else if kind.starts_with("canceled") {
        ("CANCEL", None)
    } else if kind.starts_with("resp.pos") {
        ("REPLY", Some("ACCEPTED"))
    } else if kind.starts_with("resp.neg") {
        ("REPLY", Some("DECLINED"))
    } else if kind.starts_with("resp.tent") {
        ("REPLY", Some("TENTATIVE"))
    } else {
        return None;
    };
    let mut e = event_of(x, subject, body, from, guests, uid.clone())?;
    e.dtstamp = date;
    let mut changed: Vec<CalEvent> = match method {
        "REQUEST" => changed_events(x, subject, body, from, guests, &uid, &[]).into_iter().map(|(e, _)| e).collect(),
        _ => Vec::new(),
    };
    for c in &mut changed {
        c.dtstamp = date;
    }
    if let Some(partstat) = answer {
        let person = |a: &EmailAddress| Attendee { email: a.email.clone(), name: a.name.clone(), ..Default::default() };
        e.organizer = guests.first().map(person);
        e.attendees = vec![Attendee { partstat: partstat.into(), ..person(from) }];
    } else if method == "CANCEL" {
        e.status = "CANCELLED".into();
    }
    Some((method, quatrelle_model::ical::write(&[vec![e], changed].concat(), Some(method), date)))
}

/// PidLidGlobalObjectId's first bytes (MS-OXOCAL 2.2.1.27), and those of its Data for a meeting
/// made by a calendar other than Outlook, whose `UID` follows.
const GLOBAL_ID: [u8; 16] = [0x04, 0, 0, 0, 0x82, 0, 0xE0, 0, 0x74, 0xC5, 0xB7, 0x10, 0x1A, 0x82, 0xE0, 0x08];
const VCAL_UID: &[u8; 12] = b"vCal-Uid\x01\0\0\0";
/// The longest uid read from a global object id, in bytes.
const MAX_UID: usize = 2048;

/// A meeting's uid from its global object id: the `UID` a calendar other than Outlook gave it,
/// else the id in hexadecimal, as Outlook writes it in a calendar file (without the date of the
/// occurrence it may name, so a series and its occurrences share one).
pub(super) fn uid_of(id: &[u8]) -> String {
    let hex = |b: &[u8]| b.iter().map(|b| format!("{b:02X}")).collect::<String>();
    if id.len() < 40 || id.get(..16) != Some(&GLOBAL_ID[..]) {
        return hex(id);
    }
    let size = id.get(36..40).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes).and_then(|n| usize::try_from(n).ok()).unwrap_or(0);
    let data = id.get(40..).map(|d| d.get(..size).unwrap_or(d)).unwrap_or_default();
    if let Some(uid) = data.strip_prefix(&VCAL_UID[..]) {
        let uid = uid.get(..MAX_UID).unwrap_or(uid);
        let uid = String::from_utf8_lossy(uid).trim_end_matches('\0').to_string();
        if !uid.trim().is_empty() {
            return uid;
        }
    }
    let mut clean = id.to_vec();
    clean.iter_mut().skip(16).take(4).for_each(|b| *b = 0);
    hex(&clean)
}

/// The key name of a time zone definition (MS-OXOCAL 2.2.1.41: versions, the header's size, a
/// reserved word, then the name's length in characters and the name in UTF-16).
pub(super) fn zone_key_name(def: &[u8]) -> Option<String> {
    let n = usize::from(u16::from_le_bytes([*def.get(6)?, *def.get(7)?])).min(260);
    let units: Vec<u16> = def.get(8..8 + n * 2)?.as_chunks::<2>().0.iter().map(|u| u16::from_le_bytes(*u)).collect();
    Some(String::from_utf16_lossy(&units))
}

/// A zone's name as an appointment keeps it, when the time zone database knows it and it isn't
/// UTC.
pub(super) fn known_zone(name: &str) -> Option<String> {
    let name = name.trim();
    quatrelle_model::zone::Zone::named(name).filter(|z| *z != quatrelle_model::zone::Zone::UTC).map(|_| name.to_string())
}
