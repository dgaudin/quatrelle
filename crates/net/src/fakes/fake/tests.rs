// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Tests over the fake IMAP server: what they share.

use std::time::{Duration, Instant};

use quatrelle_model::job::Secret;
use quatrelle_model::{AccountConfig, Event, Flag, FolderRole, Job, MessageKey, Op, Security, ServerConfig, SyncStatus};

use super::*;
use crate::worker::apply_remote;

mod archive;
mod arrival;
mod counts;
mod folder_ops;
mod hostile;
mod idle;
mod opening;
mod pgp;
mod pool;
mod refusals;
mod search;
mod sent;
mod sync;

/// The ids a `ServerRules` event carries.
fn ids_of(e: &Event) -> Vec<(String, String)> {
    match e {
        Event::ServerRules { ids, .. } => ids.clone(),
        _ => Vec::new(),
    }
}

fn until(pool: &crate::worker::Pool, what: &str, mut f: impl FnMut(&Event) -> bool) -> Vec<Event> {
    let end = Instant::now() + Duration::from_secs(20);
    let mut seen = Vec::new();
    while Instant::now() < end {
        if let Some(e) = pool.wait(Duration::from_millis(100)) {
            let hit = f(&e);
            seen.push(e);
            if hit {
                return seen;
            }
        }
    }
    panic!("timed out waiting for {what}; saw {seen:?}");
}

/// Wait until `f` holds: a state the pool's work leaves (the server's, the cache's), looked at
/// every few milliseconds, at most twenty seconds.
fn eventually(what: &str, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(20);
    while !f() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// An IMAP account signed in with OAuth: the token endpoint gives t1, the server refuses it,
/// the worker renews once to t2 and syncs, without asking to sign in again or saying the server
/// keeps refusing. The token endpoint is asked exactly twice.
#[test]
fn a_refused_token_is_renewed_once_before_asking_to_sign_in() {
    let answers = [("200 OK", r#"{"access_token":"t1","expires_in":3600}"#), ("200 OK", r#"{"access_token":"t2","expires_in":3600}"#)]
        .iter()
        .map(|(status, json)| {
            let mut reply = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n", json.len()).into_bytes();
            reply.extend_from_slice(json.as_bytes());
            reply
        })
        .collect();
    let (oauth_base, token_requests) = crate::http::tests::serve(answers);
    let mut p = crate::oauth::tests::local(&format!("{oauth_base}/token"));
    p.id = "test-imap-renew".into();
    crate::oauth::TEST_PROVIDERS.lock().unwrap().push(p);

    let st = State::new();
    st.lock().unwrap().token = "t2".into();
    let port = serve_tcp(&st);
    let store = tests_store("imap-renew");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port, security: Security::None },
        oauth: Some("test-imap-renew".into()),
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret("r1".into())) });
    let seen =
        until(&pool, "online or asking", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. } | SyncStatus::NeedsPassword { .. }, .. }));
    assert!(seen.iter().any(|e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. })), "{seen:?}");
    assert!(!seen.iter().any(|e| matches!(e, Event::Status { status: SyncStatus::NeedsPassword { .. }, .. })), "{seen:?}");
    assert!(
        !seen.iter().any(|e| matches!(e, Event::Status { status: SyncStatus::Error { error }, .. } if error.contains("keeps refusing"))),
        "{seen:?}"
    );
    let asked = token_requests.join().unwrap();
    assert_eq!(asked.len(), 2, "the first sign-in and one renewal: {asked:?}");
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}
