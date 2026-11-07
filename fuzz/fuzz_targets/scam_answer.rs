// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A language model's answer to the scam check, read as hostile text, and the request built from
//! the same bytes (split at the first two line breaks: subject, sender, text).
#![no_main]

use libfuzzer_sys::fuzz_target;
use quatrelle_model::lookalike::signs::{self, Sign};
use quatrelle_model::scam::{self, Facts, ScamDoc, Verdict};

fuzz_target!(|data: &[u8]| {
    let all = String::from_utf8_lossy(data);
    if let Some((score, reasons)) = scam::read_answer(&all) {
        assert!(score <= 100 && reasons.len() <= scam::MAX_REASONS);
        assert!(reasons.iter().all(|r| r.chars().count() <= scam::MAX_REASON_CHARS));
    }
    let (subject, rest) = all.split_once('\n').unwrap_or((&all, ""));
    let (from, text) = rest.split_once('\n').unwrap_or((rest, ""));
    let links: Vec<String> = text.split_whitespace().map(String::from).collect();
    let pairs: Vec<(String, String)> = links.iter().map(|l| (subject.to_string(), l.clone())).collect();
    let mut found = signs::misleading_links(&pairs);
    found.extend(signs::risky_attachments(&links));
    found.extend(signs::auth_failures(&all));
    found.push(Sign::SenderWarning);
    let facts = Facts {
        from_name: subject,
        from,
        reply_to: Some(from),
        subject,
        date: data.len() as i64 * 1_000_003,
        known: data.len() % 2 == 0,
        signs: &found,
        link_targets: &links,
        attachment_names: &links,
        text,
    };
    let _ = scam::request(&facts, subject);
    let mut doc = ScamDoc::default();
    doc.keep(from, Verdict { score: 50, reasons: vec![text.to_string()], model: String::new(), at: 0 });
    doc.trust(from);
});
