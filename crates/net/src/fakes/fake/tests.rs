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
