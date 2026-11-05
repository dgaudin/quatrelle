// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Opening a message over IMAP (issue #24): never behind a sync.

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
