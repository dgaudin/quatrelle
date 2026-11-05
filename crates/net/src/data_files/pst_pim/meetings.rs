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
