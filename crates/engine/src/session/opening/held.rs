// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The source of an open message that isn't kept on this computer: older mail read on the
//! server for showing, or a message of a folder read on the server whose copy isn't in the cache.
//! It's held in memory with the message while the reading pane or a window shows it, for what
//! needs the whole of it (its headers, its files, Save As, forwarding it as a file), and let go
//! when the last of them closes. Nothing of it is written to disk, and a source past
//! [`HELD_SOURCE_MAX`] isn't held.

use super::*;

/// The largest source held with an open message not kept here (mail of the usual sizes, once
/// encoded, stays well under it); a bigger one isn't, and what needs it says the message isn't
/// downloaded, as before. The reading pane and [`MAX_WINDOWS`] windows hold at most one each.
pub const HELD_SOURCE_MAX: usize = 64 << 20;

/// What needs the open message's source says when it isn't here (said in the interface's
/// language: the catalogs have it).
pub const NOT_DOWNLOADED: &str = "the message isn't downloaded";

/// What needs the open message says while it's on its way (in the catalogs too).
pub const STILL_DOWNLOADING: &str = "the message is still downloading";

/// A message's source as it came, shared by the reading pane and the windows that show it.
#[derive(Clone)]
pub struct HeldSource(Arc<[u8]>);

impl HeldSource {
    pub fn bytes(&self) -> &[u8] {
        &self.0
    }

    pub(super) fn shared(&self) -> Arc<[u8]> {
        self.0.clone()
    }
}

impl PartialEq for HeldSource {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0 == other.0
    }
}

/// Its size only: a message's text never goes into what's printed of the session.
impl std::fmt::Debug for HeldSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HeldSource({} bytes)", self.0.len())
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
        if !self.reading.iter().chain(&self.windows).any(|r| &r.key == key) || self.store.as_ref().is_some_and(|s| s.has_body(key)) {
            return;
        }
        if raw.len() > HELD_SOURCE_MAX {
            log::info!("a message not kept here is too big to hold while it's open ({} MB)", raw.len() >> 20);
            return;
        }
        let held = HeldSource(Arc::from(raw));
        for r in self.reading.iter_mut().chain(&mut self.windows).filter(|r| &r.key == key) {
            r.held = Some(held.clone());
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
