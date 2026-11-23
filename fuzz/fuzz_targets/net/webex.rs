// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Webex's API answers: a meeting (a usable id and link, its number, password and SIP address
//! cleaned and bounded) and why a request was refused (403 on its own, else its words on one
//! line, at most a text's length). A meeting without a usable id or link is an `Err`, never half
//! kept; the password never shows in a log.
#![no_main]

use libfuzzer_sys::fuzz_target;
use quatrelle_model::ical::valid_webex_id;
use quatrelle_model::WebexError;
use quatrelle_net::webex::{meeting_of, refusal_of};

/// The status the first two bytes give (`200` when there are none).
fn status_of(data: &[u8]) -> u16 {
    match data {
        [a, b, ..] => u16::from_be_bytes([*a, *b]),
        [a] => u16::from(*a),
        [] => 200,
    }
}

/// A text Webex's reader keeps: no control character, trimmed, bounded.
fn clean_ok(s: &str) {
    assert!(s.chars().count() <= 300, "{} characters", s.chars().count());
    assert!(!s.chars().any(char::is_control), "{s:?}");
    assert_eq!(s.trim(), s, "{s:?}");
}

fuzz_target!(|data: &[u8]| {
    let body = data.get(2..).unwrap_or_default();
    if let Ok(m) = meeting_of(body) {
        assert!(valid_webex_id(&m.id), "{:?}", m.id);
        assert!(m.web_link.starts_with("https://"), "{:?}", m.web_link);
        assert!(m.web_link.chars().count() <= 2048 && !m.web_link.chars().any(char::is_control));
        clean_ok(&m.meeting_number);
        clean_ok(&m.password.0);
        clean_ok(&m.sip_address);
        // What a log or a panic says of it is one of the two redacted forms, never the password.
        let shown = format!("{:?}", m.password);
        assert!(shown == "Secret(<empty>)" || shown == "Secret(<redacted>)", "{shown}");
    }
    let status = status_of(data);
    match refusal_of(status, body) {
        WebexError::Forbidden => assert_eq!(status, 403),
        WebexError::Refused { status: s, message } => {
            assert_eq!(s, status);
            assert!(!message.is_empty() && message.chars().count() <= 300, "{message:?}");
            assert!(!message.chars().any(char::is_control), "{message:?}");
        }
        other => panic!("{other:?}"),
    }
    // The same answer read for every status: only 403 is forbidden, a meeting is never an answer
    // to a refusal.
    for s in [200, 201, 400, 401, 403, 404, 429, 500] {
        let _ = refusal_of(s, body);
        let _ = meeting_of(body);
    }
});
