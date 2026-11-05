// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Tests over the fake EWS server: what they share.

use std::time::{Duration, Instant};

use quatrelle_model::job::Secret;
use quatrelle_model::{AccountConfig, Event, Flag, Job, Op, SyncStatus};
use quatrelle_store::Store;

use super::server::serve_at;
use super::*;

mod account;
mod answers;
mod calendar_speed;
mod calendars;
mod directory;
mod gone;
mod guests;
mod listening;
mod mailbox;
mod opening;
mod pushing;
mod scheduling;
mod tasks;
mod text_first;
mod trust;

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

fn eventually(what: &str, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(20);
    while Instant::now() < end {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out waiting for {what}");
}

/// What riley@contoso.example's account trusts, with these other servers allowed. (The fake
/// answers on 127.0.0.1, which is another site than the address's.)
fn trusting(allowed: &[&str]) -> quatrelle_model::addr::trust::Trust {
    let acc = AccountConfig {
        email: "riley@contoso.example".into(),
        ews: Some(crate::ews::AUTODISCOVER.into()),
        trusted_domains: allowed.iter().map(|d| d.to_string()).collect(),
        ..Default::default()
    };
    quatrelle_model::addr::trust::Trust::of(&acc, &[])
}

fn mail(subject: &str, id: &str) -> Vec<u8> {
    format!("From: Zoe Park <zoe@contoso.example>\r\nTo: riley@contoso.example\r\nSubject: {subject}\r\nMessage-ID: <{id}@x>\r\n\r\nHello about {subject}.\r\n")
        .into_bytes()
}
