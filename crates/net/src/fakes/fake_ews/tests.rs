//! Tests over the fake EWS server: what they share.

use std::time::{Duration, Instant};

use quatrelle_model::job::Secret;
use quatrelle_model::{AccountConfig, Event, Flag, Job, Op, SyncStatus};
use quatrelle_store::Store;

use super::server::serve_at;
use super::*;

mod account;
mod calendars;
mod listening;
mod mailbox;
mod scheduling;
mod tasks;

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

fn mail(subject: &str, id: &str) -> Vec<u8> {
    format!("From: Zoe Park <zoe@contoso.example>\r\nTo: riley@contoso.example\r\nSubject: {subject}\r\nMessage-ID: <{id}@x>\r\n\r\nHello about {subject}.\r\n")
        .into_bytes()
}
