// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The source of an open message that isn't kept on this computer: older mail read on the
//! server for showing, or a message of a folder read on the server whose copy isn't in the cache.
//! It's held in memory with the message while the reading pane or a window shows it, for what
//! needs the whole of it (its headers, its files, Save As, forwarding it as a file), and let go
//! when the last of them closes. It isn't written to the cache; only a video attachment is written
//! to the temporary folder to be played, as one of mail kept here is (the user's alone, cleared
//! with it). A source past [`HELD_SOURCE_MAX`] isn't held, and all of them together stay within
//! [`HELD_SOURCES_MAX`].

use std::sync::atomic::{AtomicU64, Ordering};

use super::*;

/// The largest source held with an open message not kept here (mail of the usual sizes, once
/// encoded, stays well under it); a bigger one isn't, and what needs it says the message isn't
/// downloaded, as before. The reading pane and [`MAX_WINDOWS`] windows hold at most one each.
pub const HELD_SOURCE_MAX: usize = 64 << 20;

/// The most held at once, every open message's source together (the reading pane's and the
/// windows'): past it, the copy held longest is let go, and what needs that message then says it
/// isn't downloaded.
pub const HELD_SOURCES_MAX: usize = 256 << 20;

/// The order sources were held in: the lowest is the one held longest.
static HELD_ORDER: AtomicU64 = AtomicU64::new(0);

/// What needs the open message's source says when it isn't here (said in the interface's
/// language: the catalogs have it).
pub const NOT_DOWNLOADED: &str = "the message isn't downloaded";

/// What needs the open message says while it's on its way (in the catalogs too).
pub const STILL_DOWNLOADING: &str = "the message is still downloading";

/// A message's source as it came, shared by the reading pane and the windows that show it, with
/// when it was held ([`HELD_ORDER`]).
#[derive(Clone)]
pub struct HeldSource {
    bytes: Arc<[u8]>,
    order: u64,
}

impl HeldSource {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(super) fn shared(&self) -> Arc<[u8]> {
        self.bytes.clone()
    }
}

impl PartialEq for HeldSource {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.bytes, &other.bytes) || self.bytes == other.bytes
    }
}

/// Its size only: a message's text never goes into what's printed of the session.
impl std::fmt::Debug for HeldSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HeldSource({} bytes)", self.bytes.len())
    }
}

impl Session {
    /// The source held with an open message (by the reading pane or a window), when it isn't
    /// kept here.
    pub(crate) fn held_source(&self, key: &MessageKey) -> Option<&HeldSource> {
        self.reading.iter().chain(&self.windows).filter(|r| &r.key == key).find_map(|r| r.held.as_ref())
    }

    /// A message's source came: held with it where it's open, when the cache hasn't kept it
    /// and it's within [`HELD_SOURCE_MAX`]. One kept here, or not open, holds nothing.
    pub(crate) fn hold_if_not_kept(&mut self, key: &MessageKey, raw: &[u8]) {
        self.hold_within(key, raw, HELD_SOURCE_MAX, HELD_SOURCES_MAX);
    }

    /// [`Session::hold_if_not_kept`], each source within `per_copy` and all of them within
    /// `total`: past it, the copies held longest are let go until they fit.
    pub(crate) fn hold_within(&mut self, key: &MessageKey, raw: &[u8], per_copy: usize, total: usize) {
        if !self.reading.iter().chain(&self.windows).any(|r| &r.key == key) || self.store.as_ref().is_some_and(|s| s.has_body(key)) {
            return;
        }
        if raw.len() > per_copy {
            log::info!("a message not kept here is too big to hold while it's open ({} MB)", raw.len() >> 20);
            return;
        }
        let held = HeldSource { bytes: Arc::from(raw), order: HELD_ORDER.fetch_add(1, Ordering::Relaxed) };
        for r in self.reading.iter_mut().chain(&mut self.windows).filter(|r| &r.key == key) {
            r.held = Some(held.clone());
        }
        self.let_go_past(total, &held);
    }

    /// Let the copies held longest go (all but `newest`) until every one held fits in `total`. A
    /// copy the pane and a window share counts once. The pane and [`MAX_WINDOWS`] windows hold
    /// one each at most: a few to weigh.
    fn let_go_past(&mut self, total: usize, newest: &HeldSource) {
        loop {
            let mut copies: Vec<&HeldSource> = Vec::new();
            for h in self.reading.iter().chain(&self.windows).filter_map(|r| r.held.as_ref()) {
                if !copies.iter().any(|c| Arc::ptr_eq(&c.bytes, &h.bytes)) {
                    copies.push(h);
                }
            }
            let size = copies.iter().fold(0usize, |n, c| n.saturating_add(c.bytes.len()));
            let Some(oldest) = copies.iter().filter(|c| !Arc::ptr_eq(&c.bytes, &newest.bytes)).min_by_key(|c| c.order).map(|c| c.bytes.clone())
            else {
                return;
            };
            if size <= total {
                return;
            }
            log::info!("the sources held of open messages not kept here are past {} MB: the one held longest is let go", total >> 20);
            for r in self.reading.iter_mut().chain(&mut self.windows) {
                if r.held.as_ref().is_some_and(|h| Arc::ptr_eq(&h.bytes, &oldest)) {
                    r.held = None;
                }
            }
        }
    }

    /// A message's source as it's here: kept in the cache, else held with it while it's open.
    /// The one lookup of what needs a message whole (its headers, files, Save As, the source
    /// window, a protection checked again, the pictures and previews the window draws).
    pub fn kept_or_held(&self, key: &MessageKey) -> Option<Vec<u8>> {
        self.store.as_ref().and_then(|s| s.read_body(key)).or_else(|| self.held_source(key).map(|h| h.bytes().to_vec()))
    }

    /// The source held with an open message not kept here, shared: what the window's work on
    /// another thread (its pictures, a video's file) reads when the cache has nothing of it.
    pub fn held_raw(&self, key: &MessageKey) -> Option<Arc<[u8]>> {
        self.held_source(key).map(HeldSource::shared)
    }
}
