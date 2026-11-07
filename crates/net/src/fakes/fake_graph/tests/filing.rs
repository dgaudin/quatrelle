// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A message filed in a folder over the fake Graph server arrives as mail received and read, at
//! its own date, not as a draft.

use super::*;

#[test]
fn a_filed_message_is_received_and_read_not_a_draft() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"));
    d.folder("Drafts", None, Some("drafts"));
    let server = serve(d);
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let mut st = crate::jmap::State::default();
    crate::graph::list_folders(&c, &mut st, "a1", &[]).unwrap();
    let raw = b"From: Zoe <zoe@x.example>\r\nTo: me@x.example\r\nSubject: Filed\r\nDate: Wed, 17 Jul 1996 02:44:25 -0700\r\nMessage-ID: <f@x>\r\n\r\nHello.\r\n";
    let id = crate::graph::append_received(&c, &st, "INBOX", raw).unwrap();
    let data = server.data.lock().unwrap();
    let m = data.messages.iter().find(|m| m.id == id).unwrap();
    assert!(m.folder == inbox && m.read && !m.draft, "in the folder, read, not a draft");
    assert!(m.received.starts_with("1996-07-17T09:44:25"), "at its own date: {}", m.received);
}
