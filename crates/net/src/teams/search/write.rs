// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The index's writes: a chat's segment staged to be written with the pages ([`Staged`]), put
//! back in the index once they are, and the index held to its bounds.

use super::*;

/// A chat's segment once messages went in, waiting for the pages' write.
pub(crate) struct Staged {
    hash: String,
    segment: Segment,
    file: Option<(String, Vec<u8>)>,
    /// The chat's oldest messages left it to keep it within [`SEGMENT_MAX`].
    trimmed: bool,
}

impl Staged {
    /// Its file, to be written with the pages.
    pub(crate) fn take_file(&mut self) -> Option<(String, Vec<u8>)> {
        self.file.take()
    }
}

impl Copy {
    /// The words of messages [`Copy::upsert`] changed put in their chat's segment (an edit's
    /// replacing the old ones, a deletion's taken out), the segment's file to be written with
    /// the pages. [`Copy::settle_index`] takes it back once they are.
    pub(crate) fn stage_index(&self, chat: &str, changed: &[ChatMessage]) -> Result<Option<Staged>> {
        if changed.is_empty() || !id_ok(chat) {
            return Ok(None);
        }
        let hash = chat_hash(chat);
        let mut ix = self.index.borrow_mut();
        // The chat's words in the form a change works on, for this change only.
        let mut seg = match ix.take_segment(&hash) {
            Some(seg) => Draft::of(&seg),
            None => self.read(&segment_key(&hash)).and_then(|b| Draft::read(&b, &hash)).unwrap_or_else(|| Draft::empty(chat)),
        };
        let ids: HashSet<&str> = changed.iter().map(|m| m.id.as_str()).collect();
        seg.remove(&ids);
        for m in changed.iter().filter(|m| m.chat == chat && m.deleted.is_none() && id_ok(&m.id)) {
            seg.add(&m.id, m.created, words_of(m));
        }
        let mut trimmed = false;
        let mut bytes = match seg.to_bytes() {
            Ok(b) => b,
            Err(e) => {
                ix.reload(hash);
                return Err(e);
            }
        };
        for _ in 0..16 {
            if bytes.len() <= SEGMENT_MAX || seg.places.is_empty() {
                break;
            }
            seg.trim_oldest();
            trimmed = true;
            bytes = match seg.to_bytes() {
                Ok(b) => b,
                Err(e) => {
                    ix.reload(hash);
                    return Err(e);
                }
            };
        }
        let segment = seg.settle(bytes.len());
        let key = segment_key(&hash);
        Ok(Some(Staged { hash, segment, file: Some((key, bytes)), trimmed }))
    }

    /// A staged segment back in the index once the pages' write is over: kept when it was
    /// written (and the index held to its bound), else read again from the file.
    pub(crate) fn settle_index(&self, staged: Option<Staged>, written: bool) {
        let Some(s) = staged else { return };
        let mut ix = self.index.borrow_mut();
        if !written {
            ix.reload(s.hash);
            return;
        }
        let hash = s.hash.clone();
        ix.put_segment(s.hash, s.segment);
        if s.trimmed {
            self.mark_partial(&mut ix);
        }
        self.bound(&mut ix, Some(&hash));
    }

    /// The index held to its bounds, in the file ([`Copy::index_cap`]) and in memory
    /// ([`RESIDENT_MAX`]): the chats with the oldest messages leave it first (`keep` apart, the
    /// chat just written).
    pub(super) fn bound(&self, ix: &mut Index, keep: Option<&str>) {
        let cap = self.index_cap(ix);
        let held = ix.resident_cap();
        let mut total: u64 = ix.segments.values().map(|s| s.bytes).fold(0, u64::saturating_add);
        let mut resident: u64 = ix.segments.values().map(|s| s.resident).fold(0, u64::saturating_add);
        if total <= cap && resident <= held {
            return;
        }
        let oldest: Vec<String> = ix.by_newest.iter().rev().map(|(_, h)| h.clone()).filter(|h| Some(h.as_str()) != keep).collect();
        for hash in oldest {
            if total <= cap && resident <= held {
                break;
            }
            if let Some(seg) = ix.take_segment(&hash) {
                total = total.saturating_sub(seg.bytes);
                resident = resident.saturating_sub(seg.resident);
                if self.store.remove_account_files(TEAMS_ACCOUNT, &segment_key(&hash)).is_err() {
                    log::warn!("Teams search: an old chat's words couldn't leave the file");
                }
            }
        }
        log::info!("Teams search: the oldest chats' words left the index ({total} bytes kept of {cap}, about {resident} in memory)");
        self.mark_partial(ix);
    }
}
