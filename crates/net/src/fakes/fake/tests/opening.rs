// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Opening a message over IMAP (issue #24): its text first, and never behind a sync.

use std::sync::{Arc, Mutex};

use super::*;

fn account(id: &str, port: u16) -> AccountConfig {
    AccountConfig {
        id: id.into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port, security: Security::None },
        smtp: ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None },
        ..Default::default()
    }
}

/// A message with a big file: text, an HTML version with a picture, and a file of `bytes`.
fn with_a_big_file(bytes: usize) -> Vec<u8> {
    let file = crate::imap::base64(&vec![7u8; bytes]);
    format!(
        "From: Sam <sam@studio.example>\r\nTo: me@studio.example\r\nSubject: Rushes\r\nMessage-ID: <r1@studio.example>\r\n\
         Importance: high\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"B\"\r\n\r\n\
         --B\r\nContent-Type: multipart/alternative; boundary=\"A\"\r\n\r\n\
         --A\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nThe notes, to read n=C3=B6w.\r\n\
         --A\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>The notes, to read <b>now</b>.</p><img src=\"cid:still@x\">\r\n--A--\r\n\
         --B\r\nContent-Type: image/png\r\nContent-ID: <still@x>\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n\
         --B\r\nContent-Type: video/quicktime; name=\"reel.mov\"\r\nContent-Disposition: attachment; filename=\"reel.mov\"\r\n\
         Content-Transfer-Encoding: base64\r\n\r\n{file}\r\n--B--\r\n"
    )
    .into_bytes()
}

fn online(st: &Arc<Mutex<State>>, name: &str) -> (crate::worker::Pool, Store) {
    let store = store(name);
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    pool.opts.prefetch = 0;
    pool.submit(Job::Start { account: account("a1", serve_tcp(st)), password: Some(Secret("pw".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    (pool, store)
}

fn text_of(seen: &[Event], key: &MessageKey) -> Option<Vec<u8>> {
    seen.iter().find_map(|e| match e {
        Event::TextFirst { key: k, raw } if k == key => Some(raw.clone()),
        _ => None,
    })
}

/// A big message's header and text parts come first, its files listed with their sizes but no
/// bytes; then the whole of it, kept.
#[test]
fn a_big_message_comes_as_its_text_first() {
    let st = State::new();
    let raw = with_a_big_file(900_000);
    st.lock().unwrap().add_raw("INBOX", raw.clone());
    let (mut pool, store) = online(&st, "imap-text-first");
    let key = store.load_folder("a1", "INBOX").unwrap().messages[0].key.clone();
    assert!(!store.has_body(&key), "not downloaded ahead");
    pool.submit(Job::FetchTextFirst { key: key.clone() });
    let seen = until(&pool, "the whole of it", |e| matches!(e, Event::Body { key: k, .. } if *k == key));
    let first = text_of(&seen, &key).expect("its text first");
    assert!(first.len() < 4096, "no bytes of the files: {}", first.len());
    let p = quatrelle_mime::parse_text_first(&first).unwrap();
    assert_eq!((p.subject.as_str(), p.from.email.as_str(), p.message_id.as_str()), ("Rushes", "sam@studio.example", "r1@studio.example"));
    assert!(p.text.contains("The notes, to read") && p.text.contains("now"), "the HTML version: {}", p.text);
    let files: Vec<(&str, usize, bool)> = p.attachments.iter().map(|a| (a.name.as_str(), a.size, a.inline)).collect();
    assert!(files.contains(&("reel.mov", 900_000, false)), "{files:?}");
    assert!(files.iter().any(|f| f.2), "the picture, inline: {files:?}");
    assert!(matches!(seen.last(), Some(Event::Body { raw: whole, .. }) if *whole == raw), "then the whole of it, as it is");
    assert_eq!(store.read_body(&key).unwrap(), raw, "what's kept is the source, not the text built");
    let log = st.lock().unwrap().log.clone();
    assert!(log.iter().any(|l| l.contains("BODYSTRUCTURE")) && log.iter().any(|l| l.contains("BODY.PEEK[1.2]")), "{log:?}");

    // A message of one part comes whole at once.
    let uid = st.lock().unwrap().add("INBOX", "Small", &[]);
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() });
    until(&pool, "the new one", |e| matches!(e, Event::FolderChanged { .. } | Event::FolderSynced { .. }));
    let key = store.load_folder("a1", "INBOX").unwrap().messages.iter().find(|m| m.key.uid == uid).unwrap().key.clone();
    pool.submit(Job::FetchTextFirst { key: key.clone() });
    let seen = until(&pool, "the small one", |e| matches!(e, Event::Body { key: k, .. } if *k == key));
    assert!(text_of(&seen, &key).is_none(), "{seen:?}");
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

/// The probe: a message with a 20 MB file, opened on a line of 10 MB a second. What comes before
/// its text shows, and when.
#[test]
fn a_twenty_megabyte_file_doesnt_hold_its_text() {
    let st = State::new();
    let raw = with_a_big_file(20 << 20);
    st.lock().unwrap().add_raw("INBOX", raw.clone());
    let (mut pool, store) = online(&st, "imap-text-first-20mb");
    st.lock().unwrap().rate = 10 << 20;
    let key = store.load_folder("a1", "INBOX").unwrap().messages[0].key.clone();
    let began = Instant::now();
    pool.submit(Job::FetchTextFirst { key: key.clone() });
    let seen = until(&pool, "the message shown", |e| matches!(e, Event::Body { key: k, .. } | Event::TextFirst { key: k, .. } if *k == key));
    let shown = began.elapsed();
    let first = match seen.last() {
        Some(Event::TextFirst { raw, .. } | Event::Body { raw, .. }) => raw.len(),
        _ => 0,
    };
    eprintln!("a message of {} bytes: shown after {shown:?}, from {first} bytes", raw.len());
    assert!(matches!(seen.last(), Some(Event::TextFirst { .. })), "its text first");
    assert!(first < 4096, "{first} bytes");
    let seen = until(&pool, "the whole of it", |e| matches!(e, Event::Body { key: k, .. } if *k == key));
    eprintln!("the whole of it after {:?}", began.elapsed());
    assert!(matches!(seen.last(), Some(Event::Body { raw: whole, .. }) if *whole == raw));
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

/// A message opened while the main connection is in the middle of a slow folder comes at once,
/// on a connection of its own, not when that folder is done.
#[test]
fn a_message_opened_during_a_sync_doesnt_wait_for_it() {
    let st = State::new();
    st.lock().unwrap().add("INBOX", "Opened during the sync", &[]);
    st.lock().unwrap().add("Archive", "Old", &[]);
    let store = store("imap-busy-open");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    pool.opts.prefetch = 0;
    pool.submit(Job::Start { account: account("a1", serve_tcp(&st)), password: Some(Secret("pw".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let key = store.load_folder("a1", "INBOX").unwrap().messages[0].key.clone();
    assert!(!store.has_body(&key));
    // The next full sync spends 8 seconds in the Archive.
    st.lock().unwrap().stall_on = Some(("SELECT \"Archive\"".into(), Duration::from_secs(8)));
    pool.submit(Job::SyncAll { account: "a1".into() });
    until(&pool, "the Archive's sync", |e| matches!(e, Event::Status { status: SyncStatus::Syncing { folder }, .. } if folder == "Archive"));
    let began = Instant::now();
    pool.submit(Job::FetchBody { key: key.clone() });
    until(&pool, "the opened message", |e| matches!(e, Event::Body { key: k, .. } if *k == key));
    eprintln!("opened during the Archive's sync in {:?}", began.elapsed());
    assert!(began.elapsed() < Duration::from_secs(5), "it waited for the Archive: {:?}", began.elapsed());
    st.lock().unwrap().stall_on = None;
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

/// A big message opened while the main connection reads a slow folder: its text at once, on a
/// connection of its own, then the whole of it, kept.
#[test]
fn a_big_message_opened_during_a_sync_shows_its_text_at_once() {
    let st = State::new();
    let raw = with_a_big_file(900_000);
    st.lock().unwrap().add_raw("INBOX", raw.clone());
    st.lock().unwrap().add("Archive", "Old", &[]);
    let (mut pool, store) = online(&st, "imap-busy-text-first");
    let key = store.load_folder("a1", "INBOX").unwrap().messages[0].key.clone();
    st.lock().unwrap().stall_on = Some(("SELECT \"Archive\"".into(), Duration::from_secs(8)));
    pool.submit(Job::SyncAll { account: "a1".into() });
    until(&pool, "the Archive's sync", |e| matches!(e, Event::Status { status: SyncStatus::Syncing { folder }, .. } if folder == "Archive"));
    let began = Instant::now();
    pool.submit(Job::FetchTextFirst { key: key.clone() });
    let seen = until(&pool, "its text", |e| matches!(e, Event::TextFirst { key: k, .. } | Event::Body { key: k, .. } if *k == key));
    eprintln!("a big message's text during the Archive's sync in {:?}", began.elapsed());
    assert!(text_of(&seen, &key).is_some(), "its text first: {seen:?}");
    assert!(began.elapsed() < Duration::from_secs(5), "it waited for the Archive: {:?}", began.elapsed());
    let seen = until(&pool, "the whole of it", |e| matches!(e, Event::Body { key: k, .. } if *k == key));
    assert!(began.elapsed() < Duration::from_secs(7), "the whole of it waited for the Archive: {:?}", began.elapsed());
    assert!(matches!(seen.last(), Some(Event::Body { raw: whole, .. }) if *whole == raw));
    assert_eq!(store.read_body(&key).unwrap(), raw);
    st.lock().unwrap().stall_on = None;
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}
