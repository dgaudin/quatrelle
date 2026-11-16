// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The index read back from the file at start, in slices: the chats listed, then one chat's
//! name and words a slice.

use super::*;

impl Copy {
    /// One slice of the index read back: the chats listed, or one chat's name and words. `false`
    /// when it's read whole.
    pub(crate) fn index_step(&self) -> bool {
        let mut ix = self.index.borrow_mut();
        let next = ix.todo.as_mut().map(Vec::pop);
        match next {
            None => {
                let mut hashes = match self.store.account_files(TEAMS_ACCOUNT, "chats") {
                    Ok(names) => names,
                    Err(StoreError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                    Err(_) => {
                        log::warn!("Teams search: the copy's chats couldn't be listed; nothing is searched");
                        Vec::new()
                    }
                };
                hashes
                    .retain(|n| n.strip_suffix(".json").is_some_and(|h| h.len() == 32 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))));
                hashes.sort_unstable();
                hashes.truncate(MAX_CHATS);
                // Popped from the end: read in the listing's order.
                let todo: Vec<String> = hashes.iter().rev().filter_map(|n| n.strip_suffix(".json").map(str::to_string)).collect();
                ix.partial = self.read(PARTIAL).is_some();
                ix.todo = Some(todo);
                true
            }
            Some(None) => false,
            Some(Some(hash)) => {
                if let Some(r) = self.record_by_hash(&hash) {
                    ix.name(&r.chat);
                }
                if !ix.segments.contains_key(&hash)
                    && let Some(seg) = self.segment_from_file(&hash)
                {
                    ix.put_segment(hash, seg);
                }
                // Held to the bounds as it's read, not once it's whole: a planted file can't
                // make it hold more meanwhile.
                self.bound(&mut ix, None);
                true
            }
        }
    }

    /// A chat's record by its hash, when it's that chat's.
    fn record_by_hash(&self, hash: &str) -> Option<ChatRecord> {
        let r: ChatRecord = serde_json::from_slice(&self.read(&format!("chats/{hash}.json"))?).ok()?;
        (id_ok(&r.chat.id) && chat_hash(&r.chat.id) == hash).then_some(r)
    }

    fn segment_from_file(&self, hash: &str) -> Option<Segment> {
        let bytes = self.read(&segment_key(hash))?;
        Some(Draft::read(&bytes, hash)?.settle(bytes.len()))
    }
}
