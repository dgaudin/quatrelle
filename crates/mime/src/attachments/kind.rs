// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What a message's source says it is, for the list's icon: a delivery report (RFC 3464), a read
//! receipt (RFC 8098), a meeting invitation or answer (iCalendar over mail, RFC 6047) or an
//! automatic reply (RFC 3834). Only the message's own parts are read, never those of a message
//! attached to it, and no report or calendar longer than [`MAX_KIND_PART`].

use quatrelle_model::MessageKind;

use super::*;

/// The longest report status or calendar read for a message's kind.
pub const MAX_KIND_PART: usize = 256 * 1024;

/// What a report's status says happened to the message it is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Report {
    Failed,
    Delayed,
    Delivered,
    Displayed,
    Deleted,
}

/// What a calendar sent by mail says about a meeting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeetingMail {
    Request,
    Cancel,
    Accepted,
    Declined,
    Tentative,
}

/// The unfolded values of the first field called `name` in each block of a report's status (one
/// block for the message, one for each recipient), in lower case.
fn values(status: &str, name: &str) -> Vec<String> {
    let status = status.replace("\r\n", "\n");
    let mut out = Vec::new();
    for block in status.split("\n\n") {
        let mut found: Option<String> = None;
        for line in block.lines() {
            if let Some(value) = found.as_mut() {
                if line.starts_with([' ', '\t']) {
                    value.push(' ');
                    value.push_str(line.trim());
                    continue;
                }
                break;
            }
            if let Some((n, v)) = line.split_once(':')
                && n.trim().eq_ignore_ascii_case(name)
            {
                found = Some(v.trim().to_string());
            }
        }
        if let Some(v) = found {
            out.push(v.to_ascii_lowercase());
        }
    }
    out
}

/// A report's meaning from its `report-type` (lower case) and its status's text: for a delivery
/// status the worst action of any recipient (failed, else delayed, else delivered, relayed or
/// expanded), for a disposition notification the `Disposition` field's type (RFC 8098 §3.2.6).
pub fn report_of_status(kind: &str, status: &str) -> Option<Report> {
    if kind == "delivery-status" {
        let actions = values(status, "Action");
        let any = |a: &[&str]| actions.iter().any(|x| a.contains(&x.split_whitespace().next().unwrap_or_default()));
        return if any(&["failed"]) {
            Some(Report::Failed)
        } else if any(&["delayed"]) {
            Some(Report::Delayed)
        } else if any(&["delivered", "relayed", "expanded"]) {
            Some(Report::Delivered)
        } else {
            None
        };
    }
    if kind != "disposition-notification" {
        return None;
    }
    let disposition = values(status, "Disposition").into_iter().next()?;
    let (_, kind) = disposition.split_once(';')?;
    match kind.split('/').next().unwrap_or_default().trim() {
        "displayed" => Some(Report::Displayed),
        "deleted" => Some(Report::Deleted),
        _ => None,
    }
}

/// The meeting mail a calendar's `method` makes, and for an answer the `partstat` it carries.
pub fn meeting_mail(method: &str, partstat: &str) -> Option<MeetingMail> {
    match (method.trim().to_ascii_uppercase().as_str(), partstat.trim().to_ascii_uppercase().as_str()) {
        ("REQUEST", _) => Some(MeetingMail::Request),
        ("CANCEL", _) => Some(MeetingMail::Cancel),
        ("REPLY", "ACCEPTED") => Some(MeetingMail::Accepted),
        ("REPLY", "DECLINED") => Some(MeetingMail::Declined),
        ("REPLY", "TENTATIVE") => Some(MeetingMail::Tentative),
        _ => None,
    }
}

/// Whether the first field `Auto-Submitted` of the message says `auto-replied` (RFC 3834 §5).
fn auto_replied(m: &Message<'_>) -> bool {
    let Some(h) = m.root().headers().all("Auto-Submitted").next() else {
        return false;
    };
    let raw = String::from_utf8_lossy(h.raw_value());
    raw.trim().split(|c: char| c.is_whitespace() || c == ';').next().is_some_and(|w| w.eq_ignore_ascii_case("auto-replied"))
}

/// The `report-type` (lower case) of a message whose own `Content-Type` is `multipart/report`.
fn report_type(m: &Message<'_>) -> Option<String> {
    let c = m.root_part().content_type()?;
    if !(c.ctype().eq_ignore_ascii_case("multipart") && c.subtype().is_some_and(|s| s.eq_ignore_ascii_case("report"))) {
        return None;
    }
    Some(c.attribute("report-type").unwrap_or_default().trim().to_ascii_lowercase())
}

/// What the header alone establishes: an automatic reply, unless the message is a report (which
/// the source settles), else [`MessageKind::Unknown`].
pub(crate) fn from_header(m: &Message<'_>) -> MessageKind {
    if report_type(m).is_none() && auto_replied(m) { MessageKind::AutoReply } else { MessageKind::Unknown }
}

/// The text of the first part of the message itself of type `mime` and at most [`MAX_KIND_PART`].
fn first_part(m: &Message<'_>, mime: &str) -> Option<String> {
    let p = m.root().parts().find(|p| part_mime(*p) == mime)?;
    let data = contents(p);
    (data.len() <= MAX_KIND_PART).then(|| String::from_utf8_lossy(&data).into_owned())
}

fn of_report(m: &Message<'_>, kind: &str) -> MessageKind {
    let status_type = match kind {
        "delivery-status" => "message/delivery-status",
        "disposition-notification" => "message/disposition-notification",
        _ => return MessageKind::Plain,
    };
    let Some(status) = first_part(m, status_type) else {
        return MessageKind::Plain;
    };
    match report_of_status(kind, &status) {
        Some(Report::Failed) => MessageKind::NotDelivered,
        Some(Report::Delayed) => MessageKind::Delayed,
        Some(Report::Delivered) => MessageKind::Delivered,
        Some(Report::Displayed) => MessageKind::ReadReceipt,
        Some(Report::Deleted) => MessageKind::NotRead,
        None => MessageKind::Plain,
    }
}

/// The meeting mail the message's calendar makes, when it has one; an answer counts only from
/// the guest it names (the `From`'s address), so no one can answer for another.
fn of_calendar(m: &Message<'_>) -> Option<MessageKind> {
    let text = first_part(m, "text/calendar")?;
    let cal = quatrelle_model::ical::parse(&text, 0);
    let method = cal.method.as_deref()?;
    let master = cal.events.iter().find(|e| e.recurrence_id.is_none()).or_else(|| cal.events.first())?;
    let partstat = if method.trim().eq_ignore_ascii_case("REPLY") {
        let from = addrs(m.from()).into_iter().next().map(|a| a.email).unwrap_or_default();
        if from.is_empty() {
            return Some(MessageKind::Plain);
        }
        match master.attendees.iter().find(|a| a.email.trim().eq_ignore_ascii_case(from.trim())) {
            Some(a) => a.partstat.clone(),
            None => return Some(MessageKind::Plain),
        }
    } else {
        String::new()
    };
    Some(match meeting_mail(method, &partstat) {
        Some(MeetingMail::Request) => MessageKind::MeetingRequest,
        Some(MeetingMail::Cancel) => MessageKind::MeetingCanceled,
        Some(MeetingMail::Accepted) => MessageKind::MeetingAccepted,
        Some(MeetingMail::Declined) => MessageKind::MeetingDeclined,
        Some(MeetingMail::Tentative) => MessageKind::MeetingTentative,
        None => MessageKind::Plain,
    })
}

/// What the whole source says the message is; never [`MessageKind::Unknown`]: ordinary mail
/// ([`MessageKind::Plain`]) when it says nothing.
pub fn message_kind(raw: &[u8]) -> MessageKind {
    let Some(m) = MessageParser::default().parse(raw) else {
        return MessageKind::Plain;
    };
    if let Some(kind) = report_type(&m) {
        return of_report(&m, &kind);
    }
    if let Some(kind) = of_calendar(&m) {
        return kind;
    }
    if auto_replied(&m) { MessageKind::AutoReply } else { MessageKind::Plain }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quatrelle_model::{Flags, MessageKey};

    type K = MessageKind;

    fn dsn_padded(extra: usize) -> Vec<u8> {
        let pad = format!("X-Pad: {}\r\n", "x".repeat(extra));
        dsn_with(&["failed"], &pad)
    }

    fn dsn_with(actions: &[&str], pad: &str) -> Vec<u8> {
        let mut status = format!("Reporting-MTA: dns; mx.example\r\n{pad}");
        for a in actions {
            status.push_str(&format!("\r\nFinal-Recipient: rfc822; a@x.example\r\nAction: {a}\r\nStatus: 5.0.0\r\n"));
        }
        format!(
            "From: postmaster@y.example\r\nTo: a@x.example\r\nContent-Type: multipart/report; report-type=delivery-status; boundary=b\r\n\r\n\
             --b\r\nContent-Type: text/plain\r\n\r\nNot delivered.\r\n\
             --b\r\nContent-Type: message/delivery-status\r\n\r\n{status}\r\n--b--\r\n"
        )
        .into_bytes()
    }

    fn dsn(actions: &[&str]) -> Vec<u8> {
        dsn_with(actions, "")
    }

    fn mdn(disposition: &str) -> Vec<u8> {
        format!(
            "From: b@y.example\r\nTo: a@x.example\r\nContent-Type: multipart/report; report-type=disposition-notification; boundary=b\r\n\r\n\
             --b\r\nContent-Type: text/plain\r\n\r\nRead.\r\n\
             --b\r\nContent-Type: message/disposition-notification\r\n\r\nFinal-Recipient: rfc822; b@y.example\r\nDisposition: manual-action/MDN-sent-manually; {disposition}\r\n\r\n--b--\r\n"
        )
        .into_bytes()
    }

    /// A message from `from` carrying a calendar with `method`, whose one guest is `guest`.
    fn imip_from(from: &str, method: &str, partstat: &str, guest: &str) -> Vec<u8> {
        format!(
            "From: {from}\r\nTo: z@y.example\r\nContent-Type: text/calendar; method={method}\r\n\r\n\
             BEGIN:VCALENDAR\r\nMETHOD:{method}\r\nBEGIN:VEVENT\r\nUID:1@x.example\r\nDTSTART:20261201T100000Z\r\nDTEND:20261201T110000Z\r\n\
             ATTENDEE;PARTSTAT={partstat}:mailto:{guest}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
        )
        .into_bytes()
    }

    /// A message from `a@x.example` carrying a calendar naming `guest`.
    fn imip(method: &str, partstat: &str, guest: &str) -> Vec<u8> {
        imip_from("a@x.example", method, partstat, guest)
    }

    fn forwarding(inner: &[u8]) -> Vec<u8> {
        let mut raw = b"From: a@x.example\r\nTo: c@x.example\r\nContent-Type: multipart/mixed; boundary=o\r\n\r\n--o\r\nContent-Type: text/plain\r\n\r\nFYI\r\n--o\r\nContent-Type: message/rfc822\r\n\r\n".to_vec();
        raw.extend_from_slice(inner);
        raw.extend_from_slice(b"\r\n--o--\r\n");
        raw
    }

    fn plain(headers: &[&str]) -> Vec<u8> {
        let mut raw = String::from("From: a@x.example\r\nTo: b@y.example\r\n");
        for h in headers {
            raw.push_str(h);
            raw.push_str("\r\n");
        }
        raw.push_str("\r\nHello\r\n");
        raw.into_bytes()
    }

    fn with_header(raw: &[u8], header: &str) -> Vec<u8> {
        let mut out = format!("{header}\r\n").into_bytes();
        out.extend_from_slice(raw);
        out
    }

    #[test]
    fn reports_by_their_status() {
        let dsn = |actions: &[&str]| {
            let mut s = "Reporting-MTA: dns; mx.example\r\n".to_string();
            for a in actions {
                s.push_str(&format!("\r\nFinal-Recipient: rfc822; a@example.com\r\nAction: {a}\r\nStatus: 5.0.0\r\n"));
            }
            report_of_status("delivery-status", &s)
        };
        assert_eq!(dsn(&["delivered", "failed"]), Some(Report::Failed));
        assert_eq!(dsn(&["Delayed"]), Some(Report::Delayed));
        assert_eq!(dsn(&["relayed (gatewayed)"]), Some(Report::Delivered));
        assert_eq!(dsn(&["unknown"]), None);
        assert_eq!(dsn(&[]), None);
        let mdn = |d: &str| report_of_status("disposition-notification", &format!("Final-Recipient: rfc822; a@b\nDisposition: {d}\n"));
        assert_eq!(mdn("manual-action/MDN-sent-manually; displayed"), Some(Report::Displayed));
        assert_eq!(mdn("automatic-action/MDN-sent-automatically;\n deleted/error"), Some(Report::Deleted));
        assert_eq!(mdn("automatic-action/MDN-sent-automatically; processed"), None);
        assert_eq!(mdn("displayed"), None);
        assert_eq!(report_of_status("other", "Action: failed\n"), None);
    }

    #[test]
    fn what_a_source_says_it_is() {
        assert_eq!(message_kind(&dsn(&["failed"])), K::NotDelivered);
        assert_eq!(message_kind(&dsn(&["delivered", "failed"])), K::NotDelivered);
        assert_eq!(message_kind(&dsn(&["delayed"])), K::Delayed);
        assert_eq!(message_kind(&dsn(&["relayed"])), K::Delivered);
        assert_eq!(message_kind(&with_header(&dsn(&["failed"]), "Auto-Submitted: auto-replied")), K::NotDelivered);
        assert_eq!(message_kind(&mdn("displayed")), K::ReadReceipt);
        assert_eq!(message_kind(&mdn("deleted")), K::NotRead);
        assert_eq!(message_kind(&mdn("processed")), K::Plain);
        assert_eq!(message_kind(&imip("REQUEST", "NEEDS-ACTION", "b@y.example")), K::MeetingRequest);
        assert_eq!(message_kind(&imip("CANCEL", "ACCEPTED", "b@y.example")), K::MeetingCanceled);
        for (p, want) in
            [("ACCEPTED", K::MeetingAccepted), ("DECLINED", K::MeetingDeclined), ("TENTATIVE", K::MeetingTentative), ("DELEGATED", K::Plain)]
        {
            assert_eq!(message_kind(&imip("REPLY", p, "a@x.example")), want);
        }
        assert_eq!(message_kind(&imip("REPLY", "ACCEPTED", "b@y.example")), K::Plain, "not from the guest it names");
        assert_eq!(message_kind(&forwarding(&imip("REQUEST", "", "b@y.example"))), K::Plain, "an invitation in an attached message isn't one");
        for (h, want) in [
            ("auto-replied", K::AutoReply),
            ("AUTO-REPLIED; owner-email=\"a@x.example\"", K::AutoReply),
            ("auto-generated", K::Plain),
            ("no", K::Plain),
        ] {
            assert_eq!(message_kind(&plain(&[&format!("Auto-Submitted: {h}")])), want, "{h}");
        }
        assert_eq!(message_kind(&plain(&["Auto-Submitted: no", "Auto-Submitted: auto-replied"])), K::Plain, "the first field only");
        // The header alone: an automatic reply yes, a report no.
        let head = |raw: &[u8]| crate::summary_from_header(raw, MessageKey::default(), Flags::default(), 0, 0).kind;
        assert_eq!(head(b"From: a@x.example\r\nAuto-Submitted: auto-replied\r\n\r\n"), K::AutoReply);
        assert_eq!(
            head(b"From: a@x.example\r\nAuto-Submitted: auto-replied\r\nContent-Type: multipart/report; report-type=delivery-status; boundary=b\r\n\r\n"),
            K::Unknown
        );
        assert_eq!(head(b"From: a@x.example\r\nSubject: s\r\n\r\n"), K::Unknown);
        // Hostile: a status of 300 KiB, a huge header, any bytes: no icon, no panic.
        assert_eq!(message_kind(&dsn_padded(300 * 1024)), K::Plain);
        let _ = message_kind(&[0xFF; 4096]);
    }

    proptest::proptest! {
        #[test]
        fn message_kind_never_panics(raw in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..4096)) {
            let _ = message_kind(&raw);
        }
    }
}
