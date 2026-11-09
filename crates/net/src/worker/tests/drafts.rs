// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! A draft is never gone without being on the server, on this computer's disk, or on the screen
//! (adapted from the origin's 79af366e and 09b74691: the three leaks, not its rewrite).

use super::*;
use quatrelle_model::{Security, ServerConfig};

fn draft(id: &str, text: &str) -> Vec<u8> {
    format!("From: me@studio.example\r\nSubject: Plans\r\nMessage-ID: <{id}@studio.example>\r\n\r\n{text}\r\n").into_bytes()
}

/// A server with a Drafts folder.
fn server() -> (std::sync::Arc<std::sync::Mutex<crate::fake::State>>, u16) {
    let st = crate::fake::State::new();
    st.lock().unwrap().boxes.push(crate::fake::Mailbox {
        name: "Drafts".into(),
        attr: "\\Drafts",
        validity: 500,
        uid_next: 1,
        msgs: Vec::new(),
        expunged: Vec::new(),
    });
    let port = crate::fake::serve_tcp(&st);
    (st, port)
}

fn worker_on(port: u16, store: &Store, sink: &Sink) -> Worker {
    let acc = AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port, security: Security::None },
        ..Default::default()
    };
    Worker::new(acc, Auth::shared("pw"), store.clone(), sink.clone(), SyncOptions::default())
}

/// What the worker still has to do, done.
fn run_queue(w: &mut Worker) {
    while let Some(job) = w.queue.pop_front() {
        w.handle(job);
    }
}

fn kept(store: &Store) -> Vec<PendingDraft> {
    store.load_account_doc("a1", PENDING_DRAFTS).unwrap_or_default()
}

fn on_server(st: &std::sync::Arc<std::sync::Mutex<crate::fake::State>>) -> usize {
    st.lock().unwrap().mailbox("Drafts").map_or(0, |m| m.msgs.len())
}

/// The drafts kept are emptied from the disk only once filed: a worker given up between the
/// reconnection and the filing (Quatrelle closed, the connection lost) loses none, and the next
/// connection files each one, once.
#[test]
fn a_draft_kept_stays_on_disk_until_it_is_filed() {
    let (st, port) = server();
    let store = crate::fake::store("t3-kept-until-filed");
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    keep_draft(&store, "a1", "Drafts", None, draft("one", "to finish")).unwrap();

    let mut gone = worker_on(port, &store, &pool.sink);
    gone.connect_and_sync();
    assert!(!gone.queue.is_empty(), "the draft is queued");
    drop(gone);
    assert_eq!(kept(&store).len(), 1, "still on disk: nothing filed it");
    assert_eq!(on_server(&st), 0);

    let mut next = worker_on(port, &store, &pool.sink);
    next.connect_and_sync();
    run_queue(&mut next);
    assert_eq!(on_server(&st), 1, "filed once");
    assert!(kept(&store).is_empty(), "off the disk once filed");
    let _ = std::fs::remove_dir_all(store.root());
}

/// A connection made again before the first queue is done doesn't queue the same draft twice.
#[test]
fn a_draft_kept_is_queued_once_by_two_connections() {
    let (st, port) = server();
    let store = crate::fake::store("t3-queued-once");
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    keep_draft(&store, "a1", "Drafts", None, draft("one", "to finish")).unwrap();
    let mut w = worker_on(port, &store, &pool.sink);
    w.connect_and_sync();
    w.connect_and_sync();
    run_queue(&mut w);
    assert_eq!(on_server(&st), 1);
    let _ = std::fs::remove_dir_all(store.root());
}

/// The same draft closed twice offline (no copy known to replace) is one draft kept, and one
/// copy in Drafts after the connection.
#[test]
fn a_draft_closed_twice_offline_is_one_copy() {
    let (st, port) = server();
    let store = crate::fake::store("t3-closed-twice");
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let mut w = worker_on(port, &store, &pool.sink);
    for text in ["first", "second, longer"] {
        w.handle(Job::KeepDraft { account: "a1".into(), raw: draft("one", text), folder: "Drafts".into(), replaces: None });
    }
    assert_eq!(kept(&store).len(), 1, "the newer version replaces the one kept");
    assert!(String::from_utf8_lossy(&kept(&store)[0].raw).contains("second, longer"));
    w.connect_and_sync();
    run_queue(&mut w);
    assert_eq!(on_server(&st), 1);
    let _ = std::fs::remove_dir_all(store.root());
}
