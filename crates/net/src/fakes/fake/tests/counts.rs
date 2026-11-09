// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The counts the pool says for a folder (adapted from the origin's 022e5770): mail read, then a
//! draft saved, never has the Inbox's unread count go back up in what the pool reports. The
//! engine keeps the count it made when a report is "as before" (`Event::Folders`), so a report
//! of the count from before the mark reached the server, after one of the count after it, would
//! be taken for news.

use super::*;

/// The unread counts the Inbox is reported with, in order, among `events`.
fn inbox_counts(events: &[Event]) -> Vec<u32> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Folders { folders, .. } => folders.iter().find(|f| f.path == "INBOX").map(|f| f.unread),
            _ => None,
        })
        .collect()
}

/// A read mark, then a draft saved: whatever is reported in between never goes back up.
#[test]
fn the_inbox_count_reported_after_a_read_mark_and_a_draft_never_goes_back_up() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        s.boxes.push(Mailbox { name: "Drafts".into(), attr: "\\Drafts", validity: 500, uid_next: 1, msgs: Vec::new(), expunged: Vec::new() });
        for n in 1..=3 {
            s.add("INBOX", &format!("mail {n}"), &[]);
        }
    }
    let port = serve_tcp(&st);
    let store = store("counts-after-read-and-draft");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port, security: Security::None },
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    let before = until(&pool, "synced", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    assert_eq!(inbox_counts(&before).last().copied(), Some(3), "{before:?}");
    // Read here (the cache first, as the engine's command does), then sent.
    let first = store.load_folder("a1", "INBOX").unwrap().messages[0].key.clone();
    let read = Op::SetFlag { keys: vec![first], flag: Flag::Seen, on: true };
    store.add_op("a1", read.clone()).unwrap();
    store.apply_op("a1", &read).unwrap();
    pool.submit(Job::Apply { account: "a1".into(), op: read });
    let draft = b"From: me@studio.example\r\nSubject: Plans\r\nMessage-ID: <d1@studio.example>\r\n\r\nhi\r\n".to_vec();
    pool.submit(Job::SaveDraft { account: "a1".into(), raw: draft, folder: "Drafts".into(), replaces: None });
    let mut seen = Vec::new();
    let end = Instant::now() + Duration::from_secs(20);
    while Instant::now() < end {
        if let Some(e) = pool.wait(Duration::from_millis(100)) {
            seen.push(e);
        }
        let done = st.lock().unwrap().mailbox("Drafts").is_some_and(|m| !m.msgs.is_empty());
        if done && st.lock().unwrap().mailbox("INBOX").is_some_and(|m| m.msgs.iter().any(|m| m.flags.iter().any(|f| f == "\\Seen"))) {
            break;
        }
    }
    // The pool goes on a moment: the folders listed again, the Inbox synced.
    let quiet = Instant::now() + Duration::from_secs(2);
    while Instant::now() < quiet {
        if let Some(e) = pool.wait(Duration::from_millis(100)) {
            seen.push(e);
        }
    }
    let counts = inbox_counts(&seen);
    assert!(counts.windows(2).all(|w| w[1] <= w[0]), "the count went back up: {counts:?}");
    // The folders are listed again after the draft, with the count the pool last had (3), which
    // is what the engine takes for "as before" and answers with the count it made (2).
    assert_eq!(counts.last().copied(), Some(3), "no folder list after the draft, or another count: {counts:?}");
}
