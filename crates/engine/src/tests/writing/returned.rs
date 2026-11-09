// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A draft that nothing could keep comes back to the screen.

use super::*;

/// The server and the disk refused a draft: it opens again in a window of its own, as it was
/// written, and the notice says why.
#[test]
fn a_draft_nothing_could_keep_opens_again_with_the_cause() {
    let mut s = sample_session("returned");
    let account = "sample".to_string();
    let raw = b"From: me@studio.example\r\nTo: zoe@studio.example\r\nSubject: Plans\r\nMessage-ID: <d@studio.example>\r\n\r\nto finish\r\n".to_vec();
    let before = s.all_composes().count();
    s.apply(Event::DraftReturned { account, raw, error: "the disk is full".into() });
    assert_eq!(s.all_composes().count(), before + 1, "back on the screen");
    let back = s.all_composes().last().unwrap();
    assert_eq!(back.subject, "Plans");
    assert!(back.body.contains("to finish"));
    assert!(back.window, "in a window of its own");
    assert!(s.notices.iter().any(|n| n.text.contains("the disk is full")), "{:?}", s.notices);
}
