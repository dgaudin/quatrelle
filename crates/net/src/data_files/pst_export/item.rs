// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! A message as it goes into the data file: its text, addresses and files from its source.

use super::*;

/// A message, ready to write.
pub(super) struct Owned {
    pub(super) subject: String,
    pub(super) from: (String, String),
    pub(super) recipients: Vec<(String, String, UnicodePstRecipientType)>,
    pub(super) body: String,
    pub(super) html: Option<String>,
    pub(super) message_id: String,
    pub(super) time: i64,
    pub(super) files: Vec<(String, String, Option<String>, Vec<u8>)>,
}

pub(super) fn owned(raw: &[u8]) -> Option<Owned> {
    let p = mime::parse(raw)?;
    let mut recipients = Vec::new();
    for (list, kind) in [(&p.to, UnicodePstRecipientType::To), (&p.cc, UnicodePstRecipientType::Cc)] {
        recipients.extend(list.iter().filter(|a| !a.email.is_empty()).map(|a| (a.name.clone(), a.email.clone(), kind)));
    }
    let files = p
        .attachments
        .iter()
        .filter(|a| a.size <= MAX_ATTACHMENT)
        .filter_map(|a| Some((a.name.clone(), a.mime.clone(), a.content_id.clone(), mime::attachment_data(raw, a.index)?)))
        .collect();
    Some(Owned {
        subject: p.subject,
        from: (p.from.name, p.from.email),
        recipients,
        body: p.text,
        html: mime::html_body(raw),
        message_id: if p.message_id.is_empty() { String::new() } else { format!("<{}>", p.message_id) },
        // Unix seconds → 100 ns since 1601.
        time: p.date.saturating_add(11_644_473_600).saturating_mul(10_000_000).max(0),
        files,
    })
}
