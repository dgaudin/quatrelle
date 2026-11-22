// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Microsoft Graph's answers read as hostile JSON: folder pages (ids, names, parents, counts), a
//! listing's `@odata.nextLink` (followed only on the same server), a message's headers and marks,
//! a calendar event with its recurrence (written back and read again), a contact, calendars and
//! `calendarGroups`, and MailTips. Every id, name, list and count stays within its cap; a
//! recurrence Graph can't say is `None`, never half an event.
#![no_main]

use libfuzzer_sys::fuzz_target;
use quatrelle_model::MessageKey;
use quatrelle_net::graph;
use quatrelle_net::graph_pim::{calendars_listed, card_of, event_json, event_of, mailtips::mail_tips_read, recurrence_of, rrule_of};
use serde_json::{json, Value};

/// The Graph endpoint the answers are for.
const BASE: &str = "https://graph.microsoft.com/v1.0";

fn check_folders(v: &Value, limit: usize) {
    let folders = graph::mailboxes_listed(v, limit);
    assert!(folders.len() <= limit, "{} folders", folders.len());
    for f in &folders {
        assert!(f.name.chars().count() <= 255, "{:?}", f.name);
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(v) = serde_json::from_slice::<Value>(data) else { return };
    // A folder page, the value itself and its `value` list.
    check_folders(&v, 5000);
    for item in v.get("value").and_then(Value::as_array).into_iter().flatten().take(256) {
        check_folders(item, 5000);
    }
    // A next page is followed only on the same server; another site's is no link at all.
    if let Some(next) = graph::next_link(BASE, &v) {
        let there = quatrelle_net::http::Url::parse(&next).expect("a next page the reader took");
        let base = quatrelle_net::http::Url::parse(BASE).expect("the base address");
        assert_eq!(there.origin(), base.origin(), "{next}");
    }
    assert!(graph::next_link(BASE, &json!({"@odata.nextLink": "https://evil.example/v1.0/me/messages"})).is_none());
    assert!(graph::next_link("not a url", &v).is_none());

    // A message's marks, its header block and its list entry.
    let _flags = graph::flags_of(&v);
    let headers = graph::header_block_of(&v);
    assert!(headers.len() <= 1 << 20, "{} bytes of headers", headers.len());
    if let Some(parsed) = quatrelle_mime::parse(&headers) {
        assert!(parsed.subject.chars().count() <= 64 * 1024);
    }
    let _summary = graph::summary_of(&v, MessageKey::default());
    let _ = graph::mailboxes_listed(&v, 0);

    // A mailbox's calendars and calendar groups.
    for key in ["calendars", "calendarGroups"] {
        for cal in calendars_listed(&v[key]) {
            assert!(!cal.0.is_empty() && cal.1.chars().count() <= 64 * 1024);
        }
    }

    // An event, its recurrence as an RRULE and back, and the event written as Graph has it.
    if let Some(e) = event_of(&v) {
        assert!(!e.uid.is_empty(), "an event with no uid");
        assert!(e.attendees.len() <= 500, "{} guests", e.attendees.len());
        assert!(e.end >= e.start, "{e:?}");
        assert!(e.categories.len() <= 20, "{} categories", e.categories.len());
        if let Some(rule) = &e.rrule {
            let back = recurrence_of(rule, e.start);
            assert!(back.is_some(), "{rule}");
            if let Some(back) = back {
                assert!(rrule_of(&back).is_some(), "{back}");
            }
        }
        let written = event_json(&e);
        if let Some(again) = event_of(&written) {
            assert_eq!(again.uid, e.uid);
            assert_eq!(again.summary, e.summary);
        }
    }
    // A contact as a card.
    let _ = card_of(&v);
    // MailTips, each for an address and within their texts.
    let tips = mail_tips_read(&v);
    assert!(tips.len() <= quatrelle_model::scheduling::MAX_MAIL_TIPS);
    for t in &tips {
        assert!(t.email.contains('@') && t.email.len() <= 320);
    }
    if let Some(list) = v.get("mailTips") {
        let tips = mail_tips_read(&json!({"value": list}));
        assert!(tips.len() <= quatrelle_model::scheduling::MAX_MAIL_TIPS);
    }
});
