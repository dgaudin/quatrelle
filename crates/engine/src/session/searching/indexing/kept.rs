// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The session: each folder's index kept between runs (`search.idx`), taken back by a later run
//! with the folder's changes since: the messages gone or changed taken out, those come in read
//! alone, never the whole folder again for a message.

use quatrelle_store::Store;

use super::*;

/// What a folder's kept index gave the index being built.
pub(crate) enum Kept {
    /// The folder as it is (with what went or changed taken out): whole, to join.
    Whole(FolderIndex),
    /// The folder less what went or changed; the messages it has (by validity and number), the
    /// others (if any) to read as the folder is read ([`SearchIndex::have`]), and the whole kept.
    Partly(FolderIndex, HashSet<(u32, u32)>),
    /// None to use: the folder is read whole.
    No,
}

/// A folder's index stamp is a hash kept on disk, so it is the same from one run to the next (the
/// standard library's hasher makes no such promise).
fn fnv(h: &mut u64, bytes: &[u8]) {
    for &b in bytes {
        *h ^= u64::from(b);
        *h = h.wrapping_mul(0x100000001b3);
    }
}

/// A folder's list as a kept index is matched against it: its stamp ([`Session::folder_stamp`]),
/// and each message's number and [`summary_sum`], in the list's order.
pub(crate) struct Listed {
    stamp: u64,
    keys: Vec<(u32, u32)>,
    sums: Vec<u64>,
}

/// A folder's list as a kept index is matched against it, in one pass.
pub(crate) fn listed<M: std::borrow::Borrow<Summary>>(st: &FolderState<M>) -> Listed {
    let mut h = 0xcbf29ce484222325u64;
    let mut sums = Vec::with_capacity(st.messages.len());
    let mut keys = Vec::with_capacity(st.messages.len());
    fnv(&mut h, &st.validity.to_le_bytes());
    fnv(&mut h, &(st.messages.len() as u64).to_le_bytes());
    for m in st.messages.iter().map(M::borrow) {
        fnv(&mut h, &m.key.uid.to_le_bytes());
        for s in [&m.subject, &m.from.name, &m.from.email, &m.preview] {
            fnv(&mut h, s.as_bytes());
            fnv(&mut h, &[0]);
        }
        for a in m.to.iter().chain(m.cc.iter()) {
            fnv(&mut h, a.email.as_bytes());
            fnv(&mut h, &[0]);
        }
        sums.push(summary_sum(m));
        keys.push((m.key.validity, m.key.uid));
    }
    Listed { stamp: h, keys, sums }
}

/// A folder's index kept by an earlier run (`kept`, read from its file), taken with the folder's
/// changes since (`now`, its list): as it is when the folder didn't change (the bodies that came
/// in since read, by `words`), else less the messages gone or changed, those come in left to
/// read (`Partly`). One kept by the version before (no sums) is taken only for a folder that
/// didn't change. Written back when it changed. Works from the cache alone: on the window's
/// thread ([`Session::kept_folder`]) or on its search lane (`ahead.rs`).
pub(crate) fn take_back(
    store: &Store,
    account: &str,
    folder: &str,
    mut kept: FolderIndex,
    now: Listed,
    words: &mut dyn FnMut(&MessageKey) -> Option<Arc<str>>,
) -> Kept {
    let Listed { stamp, keys, sums } = now;
    if kept.stamp == stamp {
        // Kept by the version before: its messages are the folder's, in its order.
        let summed = kept.sums.is_empty() && sums.len() == kept.keys.len();
        if summed {
            kept.sums = sums;
        }
        if bodies_came(store, &mut kept, words) || summed {
            keep_folder(store, account, folder, &kept);
        }
        return Kept::Whole(kept);
    }
    if kept.sums.len() != kept.keys.len() {
        return Kept::No;
    }
    // Each message as the folder has it now, by its number: one gone, or whose summary says
    // something else, goes (read again if it's still there); each kept once.
    let now: HashMap<(u32, u32), u64> = keys.into_iter().zip(sums).collect();
    let mut have: HashSet<(u32, u32)> = HashSet::with_capacity(now.len());
    let keep: Vec<bool> =
        kept.keys.iter().zip(&kept.sums).map(|(m, s)| now.get(&(m.validity, m.uid)) == Some(s) && have.insert((m.validity, m.uid))).collect();
    let gone = kept.retain(&keep);
    kept.stamp = stamp;
    bodies_came(store, &mut kept, words);
    let to_read = now.len().saturating_sub(have.len());
    log::info!("search index of {account}/{folder}: kept; {gone} messages out, {to_read} to read");
    // Kept on disk as the folder ends, a step of its own (with nothing to read too: a big
    // folder's file read back and written again was a call of a sixth of a second).
    Kept::Partly(kept, have)
}

/// The messages of a kept index without their body's words whose body is here now: their
/// words added. Whether any were.
fn bodies_came(store: &Store, kept: &mut FolderIndex, words: &mut dyn FnMut(&MessageKey) -> Option<Arc<str>>) -> bool {
    let mut changed = false;
    for i in 0..kept.keys.len() {
        let Some(key) = kept.keys.get(i).filter(|_| kept.has_body.get(i) == Some(&false)) else { continue };
        if !store.has_body(key) {
            continue;
        }
        let key = key.clone();
        let Some(words) = words(&key) else { continue };
        kept.index.add(u32::try_from(i).unwrap_or(u32::MAX), &words);
        if let Some(b) = kept.has_body.get_mut(i) {
            *b = true;
        }
        changed = true;
    }
    changed
}

/// Keep a folder's index for the next run (a cache that can't be written is no reason to fail
/// a search: the folder is read again next time).
fn keep_folder(store: &Store, account: &str, folder: &str, b: &FolderIndex) {
    if b.keys.is_empty() {
        return;
    }
    if let Err(e) = store.save_search_index(account, folder, &b.to_bytes()) {
        log::warn!("search index of {account}/{folder} not kept: {e}");
    }
}

impl Session {
    /// A folder's index kept by an earlier run, taken with the folder's changes since, on the
    /// window's thread (see [`take_back`]).
    pub(crate) fn kept_folder(&mut self, account: &str, folder: &str) -> Kept {
        let Some(store) = self.store.clone() else { return Kept::No };
        let Some(bytes) = store.load_search_index(account, folder) else { return Kept::No };
        let Some(kept) = FolderIndex::from_bytes(account, folder, &bytes) else {
            // A damaged file is no use: the folder is read again, and the file written over.
            log::warn!("search index of {account}/{folder}: unreadable, reading the folder again");
            return Kept::No;
        };
        drop(bytes);
        let now = listed(self.scan_state(account, folder));
        take_back(&store, account, folder, kept, now, &mut |key| self.words_of(key))
    }

    /// Keep a folder's index for the next run.
    pub(crate) fn keep_folder(&self, account: &str, folder: &str, b: &FolderIndex) {
        if let Some(store) = &self.store {
            keep_folder(store, account, folder, b);
        }
    }

    /// A fingerprint of a folder's messages as search sees them — their text, not their flags
    /// (marking mail read keeps the index): the one kept for the folder is used again only when it
    /// matches. It costs a pass over the summaries, far less than reading their bodies again.
    pub(crate) fn folder_stamp(&mut self, account: &str, folder: &str) -> u64 {
        listed(self.scan_state(account, folder)).stamp
    }
}
