// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The Teams worker against the fake Graph server and a sign-in service of the tests' own:
//! signing in and whose organisation the account is of, the tokens (kept in the keychain only,
//! renewed once when stale or refused), starting again from a kept sign-in, switching off,
//! signing out, a panic in a step, and what the log says.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use quatrelle_model::pim::teams::{self as model, ChatMessage, Job, Status, TeamsSettings};
use quatrelle_model::{AccountConfig, Event};
use quatrelle_store::Store;
use serde_json::json;

use super::{CHAT, ME, MINUTE, T0, TENANT, chat, hits, me, serve_teams, teams_with};
use crate::fake_graph::{FakeChat, FakeMessage, FakeTeams, FakeUser, Server, TOKEN, rfc3339};
use crate::oauth::{TEAMS_SCOPE, Tokens, now_unix};
use crate::teams::chat_of;
use crate::teams::store::{ChatRecord, Copy, Identity, Opened};
use crate::teams::worker::{Authority, Emit, Handle, Hooks, REFRESH_KEY, Worker, is_guid, keyring, missing_scope, start_with, with_ids};
use crate::worker::{Credentials, MemoryCredentials};
use crate::{NetError, Result};

/// How long a test waits for the worker to say something.
const WAIT: Duration = Duration::from_secs(10);
const CLIENT: &str = "11111111-2222-3333-4444-555555555555";
const REFRESH: &str = "kept-in-the-keychain-only-0001";
const SIGN_IN_PAGE: &str = "https://login.contoso.example/authorize";
const NO_ORGANISATION: &str = "Teams couldn't tell which organisation this account belongs to";

/// A session: tokens, starting again, stopping, signing out, the log.
mod session;
/// Signing in, and whose account it is.
mod sign_in;

/// The sign-in service of the tests: what a sign-in in the browser hands out, and what it was
/// asked.
struct Desk {
    grant: Mutex<Tokens>,
    /// Renewals are refused (the sign-in was revoked).
    refuse: AtomicBool,
    /// Nothing answers (no network).
    offline: AtomicBool,
    /// Renewed tokens have run out as they come.
    short: AtomicBool,
    authorized: AtomicU32,
    renewed: AtomicU32,
    /// The refresh tokens presented for a renewal.
    presented: Mutex<Vec<String>>,
}

struct AtDesk(Arc<Desk>);

impl Authority for AtDesk {
    fn authorize(&self, _client_id: &str, _tenant: &str, open: &mut dyn FnMut(&str) -> std::result::Result<(), String>) -> Result<Tokens> {
        self.0.authorized.fetch_add(1, Ordering::SeqCst);
        open(SIGN_IN_PAGE).map_err(NetError::Refused)?;
        Ok(self.0.grant.lock().unwrap().clone())
    }

    fn refresh(&self, _client_id: &str, _tenant: &str, refresh: &str) -> Result<Tokens> {
        if self.0.offline.load(Ordering::SeqCst) {
            return Err(NetError::Io(std::io::Error::from(std::io::ErrorKind::ConnectionRefused)));
        }
        self.0.renewed.fetch_add(1, Ordering::SeqCst);
        self.0.presented.lock().unwrap().push(refresh.to_string());
        if self.0.refuse.load(Ordering::SeqCst) {
            return Err(NetError::Auth("Microsoft Teams: the sign-in was revoked".into()));
        }
        let lasts = if self.0.short.load(Ordering::SeqCst) { 0 } else { 3600 };
        Ok(Tokens { refresh: Some(refresh.to_string()), expires_at: now_unix() + lasts, ..grant(None) })
    }
}

/// What a sign-in hands out: the fake Graph's token, a refresh token, good for an hour, the
/// permissions not said, and the organisation the ID token names (`tid`).
fn grant(tenant: Option<&str>) -> Tokens {
    Tokens {
        access: TOKEN.into(),
        refresh: Some(REFRESH.into()),
        expires_at: now_unix() + 3600,
        granted: String::new(),
        user: None,
        tenant: tenant.map(str::to_string),
    }
}

fn desk(tokens: Tokens) -> Arc<Desk> {
    Arc::new(Desk {
        grant: Mutex::new(tokens),
        refuse: AtomicBool::new(false),
        offline: AtomicBool::new(false),
        short: AtomicBool::new(false),
        authorized: AtomicU32::new(0),
        renewed: AtomicU32::new(0),
        presented: Mutex::new(Vec::new()),
    })
}

type Seen = Arc<Mutex<Vec<Event>>>;

fn emit_into(seen: &Seen) -> Emit {
    let seen = seen.clone();
    Arc::new(move |e| seen.lock().unwrap().push(e))
}

/// What the worker reported of Teams, in order.
fn teams_of(seen: &Seen) -> Vec<model::Event> {
    seen.lock().unwrap().iter().filter_map(|e| if let Event::Teams { event } = e { Some(event.clone()) } else { None }).collect()
}

/// A worker on its thread, the fake Graph it talks to, and what it reported.
struct Rig {
    server: Server,
    store: Store,
    creds: Arc<dyn Credentials>,
    desk: Arc<Desk>,
    seen: Seen,
    handle: Handle,
}

fn rig(teams: FakeTeams, tokens: Tokens) -> Rig {
    rig_with(teams, tokens, Store::memory(), Arc::new(MemoryCredentials::default()), Hooks::default())
}

fn rig_with(teams: FakeTeams, tokens: Tokens, store: Store, creds: Arc<dyn Credentials>, hooks: Hooks) -> Rig {
    let (server, desk, seen) = (serve_teams(teams), desk(tokens), Seen::default());
    let handle = start_with(store.clone(), creds.clone(), emit_into(&seen), server.base.clone(), Box::new(AtDesk(desk.clone())), hooks).unwrap();
    Rig { server, store, creds, desk, seen, handle }
}

impl Rig {
    fn sign_in(&self, tenant: &str) {
        self.handle.submit(Job::SignIn { client_id: CLIENT.into(), tenant: tenant.into() });
    }

    /// Wait until what the worker reported of Teams satisfies `done`; all of it.
    fn wait(&self, what: &str, done: impl Fn(&[model::Event]) -> bool) -> Vec<model::Event> {
        let until = Instant::now() + WAIT;
        loop {
            let seen = teams_of(&self.seen);
            if done(&seen) {
                return seen;
            }
            assert!(Instant::now() < until, "{what} never came: {seen:?}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// The refresh token the keychain holds.
    fn kept(&self) -> Option<String> {
        self.creds.get_secret(&keyring(), REFRESH_KEY)
    }

    fn hits(&self, route: &str) -> u32 {
        hits(&self.server, route)
    }

    /// Every Teams request the fake Graph has had.
    fn all_hits(&self) -> u32 {
        self.server.data.lock().unwrap().teams.as_ref().unwrap().hits.values().sum()
    }

    fn opened_the_browser(&self) -> bool {
        self.seen.lock().unwrap().iter().any(|e| matches!(e, Event::OpenUrl { .. }))
    }
}

fn status(s: Status) -> model::Event {
    model::Event::Status { status: s }
}

fn said(seen: &[model::Event], s: &Status) -> bool {
    seen.iter().any(|e| matches!(e, model::Event::Status { status } if status == s))
}

fn failed(seen: &[model::Event]) -> bool {
    seen.iter().any(|e| matches!(e, model::Event::Status { status: Status::Failed { .. } }))
}

fn account(seen: &[model::Event]) -> Option<&model::Event> {
    seen.iter().find(|e| matches!(e, model::Event::Account { .. }))
}

fn tenant_of(seen: &[model::Event]) -> Option<&str> {
    seen.iter().find_map(|e| if let model::Event::Account { tenant_id, .. } = e { Some(tenant_id.as_str()) } else { None })
}

fn one_chat() -> FakeTeams {
    teams_with(vec![chat(CHAT, T0, 2)])
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// What the Options hold once `object` has signed in.
fn settings(object: &str) -> TeamsSettings {
    TeamsSettings {
        on: true,
        client_id: CLIENT.into(),
        tenant: "organizations".into(),
        signed_in: true,
        name: "Morgan Lee".into(),
        address: "morgan@contoso.example".into(),
        object_id: object.into(),
        tenant_id: TENANT.into(),
        ..TeamsSettings::default()
    }
}

/// A worker driven by hand, a turn at a time, at the times the test makes.
fn by_hand(server: &Server, tokens: Tokens) -> (Worker, Arc<Desk>, Seen, Arc<MemoryCredentials>) {
    let (desk, seen, creds) = (desk(tokens), Seen::default(), Arc::new(MemoryCredentials::default()));
    let w = Worker::new(Store::memory(), creds.clone(), emit_into(&seen), server.base.clone(), Box::new(AtDesk(desk.clone()))).unwrap();
    (w, desk, seen, creds)
}

/// Turn a worker driven by hand, at `now`, until `done` (the browser's answer comes from a thread
/// of its own, so the first turns may find nothing yet).
fn turn_until(w: &mut Worker, now: Instant, what: &str, done: impl Fn(&Worker) -> bool) {
    let until = Instant::now() + WAIT;
    while !done(w) {
        assert!(Instant::now() < until, "{what} never came");
        w.turn(now);
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A folder of the test's own, empty.
fn temp(name: &str) -> PathBuf {
    let t = std::env::temp_dir().join(format!("quatrelle-teams-worker-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&t);
    t
}
