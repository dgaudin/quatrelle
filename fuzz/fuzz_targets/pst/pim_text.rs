// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! What an export to an Outlook data file keeps in an item's text and the import reads back
//! (`pst_pim::rest`): a contact's text, a card of what Outlook has no field for after it, and a
//! task's, its rule after it; and a task's rule as the recurrence pattern the export writes.
//! What comes back is held to what a card and a task keep, and a pattern written is one the
//! import reads.
//!
//! The input is the task's first day (eight bytes, little-endian Unix seconds), then the text,
//! whose first line is also the rule.
#![no_main]

use libfuzzer_sys::fuzz_target;
use quatrelle_net::pst_export;

fuzz_target!(|input: &[u8]| {
    let Some((from, text)) = input.split_first_chunk::<8>() else { return };
    let from = i64::from_le_bytes(*from);
    let text = String::from_utf8_lossy(text);
    let (card, task) = pst_export::read_back_text(&text);
    assert!(card.emails.len() <= 50 && card.phones.len() <= 50 && card.addresses.len() <= 20 && card.extra.len() <= 200);
    assert!(task.rrule.as_ref().is_none_or(|r| !r.contains(['\r', '\n'])));
    let rule = text.lines().next().unwrap_or_default();
    if let Some((pattern, _)) = pst_export::task_pattern(rule, from) {
        // The pattern counts from the task's first day: a task starting then reads it.
        assert!(pst_export::task_rule(&pattern, from.div_euclid(86_400).saturating_mul(86_400)).is_some(), "{rule}");
    }
});
