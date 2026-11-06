// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The session: the folders' kept indexes (`search.idx`) read and taken back on a lane of their
//! own ([`IndexWork`]), a few folders ahead of the search that joins them. On the window's thread,
//! each folder's was a call of its own: its file read and made sense of, its list read from the
//! disk to see what changed since, the bodies come since read. The first search after a start
//! took 1.6 s of calls for 50,000 messages, the worst 90 ms (about 17 s at the owner's 550,000).
//! Where the window has no lane (the browser, the command line, the tests), it's done there, as
//! before.

use std::collections::VecDeque;
use std::sync::Mutex;

use quatrelle_store::Store;

use super::*;

/// Folders taken back ahead of the one the search is at, at most: each waits with its index in
/// memory until it's joined.
const AHEAD: usize = 3;
/// Seconds the search waits for the lane at most before reading the folder itself (a lane that
/// stopped).
const LANE_WAIT: i64 = 20;

/// A folder's kept index as the lane took it back, and the version of the folder's list it was
/// taken against ([`Store::folder_version`]): a list changed since is taken back again.
pub(crate) struct Prepared {
    version: u64,
    kept: Kept,
}

/// What the session and the lane's work share.
#[derive(Default)]
pub(crate) struct Ahead {
    /// Folders to take back, in the order the search wants them.
    asked: VecDeque<(String, String)>,
    /// The one being taken back.
    working: Option<(String, String)>,
    /// Taken back, waiting for the search.
    ready: HashMap<(String, String), Prepared>,
    /// Whether a piece of work is out (on the lane, or on its way there).
    out: bool,
    /// When the search began waiting for the folder it's at (the window's clock), 0: it isn't.
    since: i64,
    /// The lane didn't answer in time: the rest is taken back on the window's thread.
    given_up: bool,
}

fn lock(shared: &Mutex<Ahead>) -> std::sync::MutexGuard<'_, Ahead> {
    shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Work for the window's search lane: the folders asked for, taken back one after the other,
/// until none is asked (or the search that asked is gone).
pub struct IndexWork {
    store: Store,
    shared: Arc<Mutex<Ahead>>,
}

impl IndexWork {
    pub fn run(self) {
        loop {
            // Nobody waits for it any more (the index was let go).
            let wanted = Arc::strong_count(&self.shared) > 1;
            let next = {
                let mut a = lock(&self.shared);
                let n = if wanted { a.asked.pop_front() } else { None };
                a.working.clone_from(&n);
                if n.is_none() {
                    a.out = false;
                }
                n
            };
            let Some((account, folder)) = next else { return };
            let version = self.store.folder_version(&account, &folder);
            let kept = prepare(&self.store, &account, &folder);
            let mut a = lock(&self.shared);
            a.working = None;
            a.ready.insert((account, folder), Prepared { version, kept });
        }
    }
}

/// A folder's kept index taken back as [`Session::kept_folder`] takes it, from the cache alone
/// (the folder's list as the disk has it, its bodies read there).
fn prepare(store: &Store, account: &str, folder: &str) -> Kept {
    let Some(bytes) = store.load_search_index(account, folder) else { return Kept::No };
    let Some(kept) = FolderIndex::from_bytes(account, folder, &bytes) else {
        log::warn!("search index of {account}/{folder}: unreadable, reading the folder again");
        return Kept::No;
    };
    drop(bytes);
    let listed = match store.load_folder(account, folder) {
        Ok(st) => kept::listed(&st),
        Err(e) => {
            log::warn!("search index of {account}/{folder}: its list couldn't be read: {e}");
            return Kept::No;
        }
    };
    kept::take_back(store, account, folder, kept, listed, &mut |key| store.read_body(key).map(|raw| Arc::from(mime::search_text(&raw))))
}

impl Session {
    /// From now on, the folders' kept indexes are taken back on the window's search lane
    /// ([`Session::take_index_work`]), where it has one.
    pub fn defer_index_reading(&mut self, on: bool) {
        self.search_pace.lane = on;
    }

    /// Whether the folders' kept indexes are taken back on the window's search lane.
    pub fn index_reading_deferred(&self) -> bool {
        self.search_pace.lane
    }

    /// The kept index of the folder the search is at, for it to join: taken back on the lane
    /// when the window has one (the next few folders asked too), here otherwise. `None`: not
    /// ready yet, the search waits (the window calls again next frame).
    pub(super) fn kept_or_wait(&mut self, ix: &mut SearchIndex, account: &str, folder: &str) -> Option<Kept> {
        if !self.search_pace.lane || self.store.is_none() {
            return Some(self.kept_folder(account, folder));
        }
        let shared = ix.ahead.get_or_insert_with(Arc::default).clone();
        let k = (account.to_string(), folder.to_string());
        let now = self.clock.now;
        let got = {
            let mut a = lock(&shared);
            if a.given_up {
                drop(a);
                return Some(self.kept_folder(account, folder));
            }
            let known = |a: &Ahead, f: &(String, String)| a.ready.contains_key(f) || a.working.as_ref() == Some(f) || a.asked.contains(f);
            if !known(&a, &k) {
                a.asked.push_front(k.clone());
            }
            for f in ix.scan.iter().skip(ix.folder.saturating_add(1)).take(AHEAD) {
                if !known(&a, f) && a.ready.len().saturating_add(a.asked.len()) <= AHEAD {
                    a.asked.push_back(f.clone());
                }
            }
            match a.ready.remove(&k) {
                Some(p) => {
                    a.since = 0;
                    Some(p)
                }
                None if a.since == 0 => {
                    a.since = now;
                    None
                }
                None if now.saturating_sub(a.since) > LANE_WAIT => {
                    log::warn!("search index of {account}/{folder}: the lane didn't take it back, the rest read here");
                    a.given_up = true;
                    a.asked.clear();
                    drop(a);
                    return Some(self.kept_folder(account, folder));
                }
                None => None,
            }
        };
        let p = got?;
        // Its list changed since the lane took it back: taken back here, against the list now.
        if self.store.as_ref().map_or(0, |s| s.folder_version(account, folder)) != p.version {
            return Some(self.kept_folder(account, folder));
        }
        Some(p.kept)
    }

    /// The work for the search lane, when the search asked for folders to be taken back and none
    /// is out: the window sends it to its lane each frame there's one.
    pub fn take_index_work(&mut self) -> Option<IndexWork> {
        let shared = self.index.as_ref().and_then(|ix| ix.ahead.clone())?;
        let store = self.store.clone()?;
        {
            let mut a = lock(&shared);
            if a.out || a.asked.is_empty() {
                return None;
            }
            a.out = true;
        }
        Some(IndexWork { store, shared })
    }
}
