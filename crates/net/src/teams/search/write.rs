// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The index's writes: a chat's tranches staged to be written with the pages ([`Staged`]), put
//! back in the index once they are, and the index held to its bounds.

use super::*;

/// A chat's tranches once messages went in, waiting for the pages' write.
pub(crate) struct Staged {
    hash: String,
    chat: Tranches,
    /// The files to write with the pages.
    files: Vec<(String, Vec<u8>)>,
    /// The numbers written empty, to be removed once they are.
    gone: Vec<u32>,
    /// The numbers written with messages: spared by the bound that follows.
    touched: Vec<u32>,
    /// The chat's oldest tranches left it to keep it within [`TRANCHES_MAX`] and [`SEGMENT_MAX`].
    trimmed: bool,
}

impl Staged {
    /// Its files, to be written with the pages.
    pub(crate) fn take_files(&mut self) -> Vec<(String, Vec<u8>)> {
        std::mem::take(&mut self.files)
    }
}

/// A draft cut into tranches of at most `cap` messages ([`Draft::cut`]), a tranche of more than
/// one message past [`TRANCHE_BYTES`] cut in two again (its size taken with the widest number,
/// [`TRANCHES_MAX`]: no number written has more digits).
pub(super) fn fitted(d: Draft, side: Side, cap: usize) -> Result<Vec<Draft>> {
    let mut out = Vec::new();
    let mut todo: Vec<Draft> = d.cut(side, cap).into_iter().rev().collect();
    while let Some(piece) = todo.pop() {
        let held = piece.held();
        if held > 1 && piece.to_bytes(TRANCHES_MAX)?.len() > TRANCHE_BYTES {
            todo.extend(piece.cut(Side::Halves, held.div_ceil(2)).into_iter().rev());
            continue;
        }
        out.push(piece);
    }
    Ok(out)
}

/// Each tranche as the file keeps it, numbered 0, 1, … in order.
fn encode(pieces: &[Draft]) -> Result<Vec<Vec<u8>>> {
    pieces.iter().zip(0u32..).map(|(piece, n)| piece.to_bytes(n)).collect()
}

impl Copy {
    /// The words of messages [`Copy::upsert`] changed put in their chat's tranches (an edit's
    /// replacing the old ones, a deletion's taken out), the files to be written with the pages.
    /// [`Copy::settle_index`] takes them back once they are. `Err` when the chat's words can't be
    /// read from the file: the pages aren't written either.
    pub(crate) fn stage_index(&self, chat: &str, changed: &[ChatMessage]) -> Result<Option<Staged>> {
        if changed.is_empty() || !id_ok(chat) {
            return Ok(None);
        }
        let hash = chat_hash(chat);
        let mut ix = self.index.borrow_mut();
        let had = match ix.take_chat(&hash) {
            Some(t) => t,
            None => self.chat_index(&mut ix, &hash)?.unwrap_or_else(|| Tranches::empty(chat)),
        };
        let cap = ix.tranche_cap();
        match staged_whole(chat, &hash, had, changed, cap) {
            Ok(s) => Ok(Some(s)),
            Err(e) => {
                ix.reload(hash);
                Err(e)
            }
        }
    }

    /// Staged tranches back in the index once the pages' write is over: kept when they were
    /// written (the numbers left empty removed, and the index held to its bound), else read
    /// again from the file.
    pub(crate) fn settle_index(&self, staged: Option<Staged>, written: bool) {
        let Some(Staged { hash, mut chat, gone, touched, trimmed, .. }) = staged else { return };
        let mut ix = self.index.borrow_mut();
        if !written {
            ix.reload(hash);
            return;
        }
        // A number that can't be removed stays an orphan: written empty again at the chat's next
        // write.
        for n in gone {
            if self.store.remove_account_files(TEAMS_ACCOUNT, &tranche_key(&hash, n)).is_err() {
                log::warn!("Teams search: an emptied tranche couldn't leave the file");
                chat.orphans.push(n);
            }
        }
        ix.put_chat(hash.clone(), chat);
        if trimmed {
            self.mark_partial(&mut ix);
        }
        self.bound(&mut ix, Some((&hash, &touched)));
    }

    /// The index held to its bounds, in the file ([`Copy::index_cap`]) and in memory
    /// ([`RESIDENT_MAX`]): the oldest tranches of every chat leave it first (`keep` apart, the
    /// tranches just written), a chat of the old format whole.
    pub(super) fn bound(&self, ix: &mut Index, keep: Option<(&str, &[u32])>) {
        let cap = self.index_cap(ix);
        let held = ix.resident_cap();
        let mut total: u64 = ix.tranches().map(|s| s.bytes).fold(0, u64::saturating_add);
        let mut resident: u64 = ix.tranches().map(|s| s.resident).fold(0, u64::saturating_add);
        if total <= cap && resident <= held {
            return;
        }
        let spared = |hash: &str, key: u32| keep.is_some_and(|(h, keys)| h == hash && keys.contains(&key));
        let oldest: Vec<(String, u32)> = ix.by_newest.iter().rev().filter(|(_, h, k)| !spared(h, *k)).map(|(_, h, k)| (h.clone(), *k)).collect();
        for (hash, key) in oldest {
            if total <= cap && resident <= held {
                break;
            }
            if ix.segments.get(&hash).is_some_and(|t| t.legacy) {
                // Its tranches are in memory only. Its folder goes first (tranches a copy rolled
                // back left there), then its head: while the folder stays, so does the head, and
                // the chat is read again as rolled back.
                let Some(t) = ix.take_chat(&hash) else { continue };
                for s in &t.list {
                    total = total.saturating_sub(s.bytes);
                    resident = resident.saturating_sub(s.resident);
                }
                if self.store.remove_account_files(TEAMS_ACCOUNT, &tranches_key(&hash)).is_err()
                    || self.store.remove_account_files(TEAMS_ACCOUNT, &segment_key(&hash)).is_err()
                {
                    log::warn!("Teams search: an old chat's words couldn't leave the file");
                }
            } else if let Some(seg) = ix.take_tranche(&hash, key) {
                total = total.saturating_sub(seg.bytes);
                resident = resident.saturating_sub(seg.resident);
                if self.store.remove_account_files(TEAMS_ACCOUNT, &tranche_key(&hash, key)).is_err() {
                    // An orphan of its chat: emptied at the chat's next write.
                    log::warn!("Teams search: an old tranche couldn't leave the file");
                    ix.segments.entry(hash).or_insert_with(|| Tranches::empty(&seg.chat)).orphans.push(key);
                }
            }
        }
        log::info!("Teams search: the oldest tranches left the index ({total} bytes kept of {cap}, about {resident} in memory)");
        self.mark_partial(ix);
    }
}

/// A chat's tranches with what changed, written whole: its words as one draft, the changed ids
/// taken out and the messages not deleted put in, then cut into tranches numbered 0, 1, … (the
/// oldest dropped past [`TRANCHES_MAX`] or [`SEGMENT_MAX`]). Every number the chat had and doesn't
/// write again is written empty, then removed; a chat of the old format gets its stub.
fn staged_whole(chat: &str, hash: &str, had: Tranches, changed: &[ChatMessage], cap: usize) -> Result<Staged> {
    // The numbers in the file: a chat of the old format's tranches are in memory only.
    let legacy = had.legacy;
    let numbers: BTreeSet<u32> = had.list.iter().filter(|_| !legacy).map(|s| s.key).chain(had.orphans.iter().copied()).collect();
    let mut d = Draft::merge(had.list, chat);
    let ids: HashSet<&str> = changed.iter().map(|m| m.id.as_str()).collect();
    d.remove(&ids);
    for m in changed.iter().filter(|m| m.chat == chat && m.deleted.is_none() && id_ok(&m.id)) {
        d.add(&m.id, m.created, words_of(m));
    }
    let mut pieces = fitted(d, Side::Front, cap)?;
    let over = pieces.len().saturating_sub(usize::try_from(TRANCHES_MAX).unwrap_or(usize::MAX));
    pieces.drain(..over);
    let mut bytes = encode(&pieces)?;
    let mut total: usize = bytes.iter().map(Vec::len).fold(0, usize::saturating_add);
    let mut dropped = 0;
    while total > SEGMENT_MAX && dropped < bytes.len() {
        total = total.saturating_sub(bytes.get(dropped).map_or(0, Vec::len));
        dropped += 1;
    }
    if dropped > 0 {
        // Numbered again from 0: no number grows, nor does any file.
        pieces.drain(..dropped);
        bytes = encode(&pieces)?;
    }
    let (mut files, mut list, mut touched) = (Vec::new(), Vec::new(), Vec::new());
    for ((piece, b), n) in pieces.into_iter().zip(bytes).zip(0u32..) {
        list.push(piece.settle(n, b.len()));
        files.push((tranche_key(hash, n), b));
        touched.push(n);
    }
    let written = u32::try_from(list.len()).unwrap_or(u32::MAX);
    let gone: Vec<u32> = numbers.into_iter().filter(|n| *n >= written).collect();
    let empty = Draft::empty(chat);
    for n in &gone {
        files.push((tranche_key(hash, *n), empty.to_bytes(*n)?));
    }
    if legacy {
        files.push((segment_key(hash), stub(chat)?));
    }
    by_first(&mut list);
    let chat = Tranches { chat: chat.to_string(), list, legacy: false, orphans: Vec::new() };
    Ok(Staged { hash: hash.to_string(), chat, files, gone, touched, trimmed: over > 0 || dropped > 0 })
}
