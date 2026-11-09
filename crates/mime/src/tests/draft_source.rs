// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
use super::*;

/// A draft of a forward names the message it forwards in its own copy, and its summary keeps
/// that, for the list to mark the message `[Draft]`: a folder's name in any script. A reply's
/// draft, other mail and a field that only looks like one keep nothing.
#[test]
fn a_forwards_draft_names_its_message_in_its_summary() {
    let from = EmailAddress::new("Riley", "riley@studio.example");
    let here = MessageKey { account: "a".into(), folder: "Drafts".into(), validity: 1, uid: 9 };
    for folder in ["INBOX", "Entw\u{fc}rfe/\u{c4}rger", "\u{412}\u{445}\u{43e}\u{434}\u{44f}\u{449}\u{438}\u{435}"] {
        let source = MessageKey { account: "acc-1".into(), folder: folder.into(), validity: 7, uid: 42 };
        let d = Draft { subject: "Fwd: Hi".into(), body: "x".into(), ..Default::default() };
        let raw = build(&d, &from, 1_700_000_000, "m@x").unwrap();
        let raw = draft_copy(raw, &[], Some((ReplyMode::Forward, &source.to_id())));
        let s = summary_from_full(&raw, here.clone(), Flags::default(), 1, 0);
        assert_eq!(s.forward_of, source.to_id(), "{}", String::from_utf8_lossy(&raw));
    }
    let d = Draft { subject: "Re: Hi".into(), body: "x".into(), ..Default::default() };
    let raw = build(&d, &from, 1_700_000_000, "m@x").unwrap();
    let reply = draft_copy(raw.clone(), &[], Some((ReplyMode::Reply, "a:1:2:INBOX")));
    assert_eq!(summary_from_full(&reply, here.clone(), Flags::default(), 1, 0).forward_of, "", "a reply is known by its In-Reply-To");
    assert_eq!(summary_from_full(&raw, here.clone(), Flags::default(), 1, 0).forward_of, "", "other mail");
    for field in [
        "X-Quatrelle-Source: forward nonsense".to_string(),
        "X-Quatrelle-Source: forward 613a783a323a494e424f58".to_string(),
        "X-Quatrelle-Source: forwarded 613a313a323a494e424f58".to_string(),
        format!("X-Quatrelle-Source: forward {}", "6f".repeat(9000)),
    ] {
        let raw = format!("From: a@b.example\r\nSubject: s\r\n{field}\r\n\r\nbody\r\n");
        assert_eq!(summary_from_full(raw.as_bytes(), here.clone(), Flags::default(), 1, 0).forward_of, "", "{}", field.get(..40).unwrap_or(&field));
    }
}
