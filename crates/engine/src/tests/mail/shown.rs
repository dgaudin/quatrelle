// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The message shown, in the reading pane and in windows of its own: what each frame copies of it.

use std::sync::Arc;

use super::*;

/// A message waiting in the reading pane and in a window of its own is parsed once when it
/// comes, both showing that one parse: the pane and each window copy what they show every frame,
/// which must cost nothing however big the message is. A reply quotes a copy of its own.
#[test]
fn a_message_shown_twice_is_parsed_once() {
    let mut s = sample_session("parsed-once");
    let k = s.list.first().map(|m| m.key.clone()).unwrap();
    s.store.as_ref().unwrap().remove_body(&k);
    s.run("list.open", &json!({"key": k.to_id()})).unwrap();
    s.run("message.popOut", &json!({"key": k.to_id()})).unwrap();
    assert!(s.reading.as_ref().unwrap().parsed.is_none() && s.windows[0].parsed.is_none());
    // 5 MB of text.
    let raw = format!("From: Ana <ana@posteo.example>\r\nSubject: Twice\r\n\r\n{}", "a line of the message\r\n".repeat(220_000));
    s.apply(Event::Body { key: k.clone(), raw: raw.into_bytes() });
    // A second of frames: the pane and the window copy what they show.
    let began = std::time::Instant::now();
    for _ in 0..60 {
        let shown = (s.reading.clone(), s.windows.clone());
        drop(shown);
    }
    let copies = began.elapsed();
    assert!(copies < std::time::Duration::from_millis(5), "a second of frames copied the message for {copies:?}");
    let pane = s.reading.as_ref().unwrap().parsed.clone().unwrap();
    let window = s.windows[0].parsed.clone().unwrap();
    assert!(Arc::ptr_eq(&pane, &window), "one parse");
    assert_eq!(pane.subject, "Twice");
    // A reply quotes a copy of its own, the one shown left as it was.
    s.run("mail.reply", &json!({})).unwrap();
    assert!(s.compose.as_ref().is_some_and(|c| c.subject.contains("Twice")));
    assert!(Arc::ptr_eq(s.reading.as_ref().unwrap().parsed.as_ref().unwrap(), &pane), "the one shown is left as it was");
    assert_eq!(*pane, *window);
    done(s);
}
