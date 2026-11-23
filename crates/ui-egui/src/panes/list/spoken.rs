// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! What a list row says aloud to a screen reader.

use super::*;

/// To a screen reader a row is one item of the list: who, the subject, when, and what its
/// markers show (a sender to check, external, unread, flagged or completed, replied to or
/// forwarded, attachments).
pub(super) fn row_spoken(s: &Session, m: &Summary, marks: &quatrelle_engine::SenderMarks, unread: bool, sent_view: bool) -> String {
    let mut who = if sent_view { m.to.first().map(|a| a.display().to_string()).unwrap_or_default() } else { m.from.display().to_string() };
    if marks.address {
        who = format!("{who} <{}>", m.from.email.trim());
    }
    let subject = if m.subject.is_empty() { tl!("(no subject)") } else { m.subject.as_str() };
    let mut spoken = vec![who, subject.to_string(), crate::i18n::date(&dates::list_time(m.received_at(), &s.clock))];
    if let Some(score) = marks.scam {
        spoken.push(crate::reading::scam_headline(score).to_string());
    }
    for (on, word) in [
        (marks.warning, "Check who sent it"),
        (marks.external, "External"),
        (unread, "Unread"),
        (m.flags.flagged, "Flagged"),
        (m.flags.follow_up_complete(), "Completed"),
        (m.flags.answered, "Replied"),
        (!m.flags.answered && m.flags.forwarded, "Forwarded"),
        (m.has_attachments, "Has attachments"),
    ] {
        if on {
            spoken.push(tl!(word).to_string());
        }
    }
    spoken.join(", ")
}
