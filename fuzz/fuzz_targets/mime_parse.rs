// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
//! A message source as it comes from a server or a file: parsed for reading, for the message
//! list and for search, and its attachments taken out; told signed or encrypted from its header
//! alone, never wrongly told it isn't; and as an export to a data file takes it.
#![no_main]

use libfuzzer_sys::fuzz_target;
use quatrelle_mime as mime;
use quatrelle_model::{Flags, MessageKey};

fuzz_target!(|raw: &[u8]| {
    let key = MessageKey { account: "a".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
    let size = u32::try_from(raw.len()).unwrap_or(u32::MAX);
    let _ = mime::summary_from_header(raw, key.clone(), Flags::default(), size, 0);
    let _ = mime::summary_from_full(raw, key, Flags::default(), size, 0);
    let _ = mime::search_text(raw);
    let _ = mime::html_body(raw);
    // One its header says isn't protected is not, whatever its body.
    if !mime::smime::is_protected(raw) {
        assert_eq!(mime::smime::protection(raw), mime::smime::Protection::None);
    }
    if let Some(p) = mime::parse(raw) {
        for i in 0..p.attachments.len().min(8) {
            let _ = mime::attachment_data(raw, i);
        }
        // Several from one parse, as the reading pane takes a message's pictures.
        let _ = mime::attachments_data(raw, &[0, 1, 5, usize::MAX], 1 << 20, 4 << 20);
        let _ = mime::with_date(raw, 0);
    }
    // As an export to a data file reads it: S/MIME, a report's status, attached messages.
    let _ = quatrelle_net::pst_export::message_class(raw);
});
