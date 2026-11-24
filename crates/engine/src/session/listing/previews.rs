// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The previews and kinds of list rows that came without them, read from their sources: a stranger's
//! message parsed in full, so off the window's thread. [`Session::take_preview_work`] hands the
//! work out, [`PreviewWork::run`] does it on any thread, [`Session::finish_previews`] takes its
//! result in.

use std::panic::{AssertUnwindSafe, catch_unwind};

use quatrelle_store::Store;

use super::*;

/// Rows looked at in one piece of work, at most (each looked up in the list).
const ITEMS_MAX: usize = 200;

/// Sources to read for the rows' previews and kinds. Runs on any thread.
pub struct PreviewWork {
    store: Store,
    /// Each row's key, whether its list entry has a preview already, and the kind it says.
    items: Vec<(MessageKey, bool, MessageKind)>,
    now: i64,
}

/// What [`PreviewWork::run`] read.
pub struct PreviewsRead {
    read: Vec<(MessageKey, Preview)>,
    /// Every key the work was given, read or not.
    given: Vec<MessageKey>,
    /// Some were left for another go (a few at a time).
    pub(super) left: bool,
}

impl PreviewWork {
    /// Read a few sources (at most [`PREVIEW_STEP`], and [`PREVIEW_STEP_BYTES`] of them).
    pub fn run(self) -> PreviewsRead {
        let given: Vec<MessageKey> = self.items.iter().map(|(k, _, _)| k.clone()).collect();
        let (mut read, mut left) = (Vec::new(), false);
        let (mut done, mut bytes) = (0usize, 0usize);
        for (key, has_preview, said) in &self.items {
            if done >= PREVIEW_STEP || bytes >= PREVIEW_STEP_BYTES {
                left = true;
                break;
            }
            let Some(raw) = self.store.read_body(key) else {
                read.push((key.clone(), Preview::Missing(self.now)));
                continue;
            };
            done += 1;
            bytes = bytes.saturating_add(raw.len());
            // The row as listed: with a preview it keeps it, with a kind it keeps it. A parser of
            // strangers' bytes: a panic in it is a plain message, not a lost thread.
            let one = catch_unwind(AssertUnwindSafe(|| {
                // As its summary would have it, had it come whole: not a full parse (laid out).
                let text = if *has_preview { String::new() } else { mime::preview_from_source(&raw) };
                let kind = if !said.is_unknown() {
                    *said
                } else if raw.len() > KIND_SOURCE_MAX {
                    MessageKind::Plain
                } else {
                    mime::message_kind(&raw)
                };
                (text, kind)
            }));
            let (text, kind) = one.unwrap_or((String::new(), MessageKind::Plain));
            read.push((key.clone(), Preview::Read { text, kind }));
        }
        PreviewsRead { read, given, left }
    }
}

impl Session {
    /// The work of reading the sources of `keys`, rows shown without a preview or a kind: none
    /// while another is out, with no cache, or for rows of an account Compact Now has to itself.
    /// The rows are not asked for again until [`Session::finish_previews`].
    pub fn take_preview_work(&mut self, keys: &[MessageKey]) -> Option<PreviewWork> {
        let store = self.store.clone()?;
        if !self.previews_pending.is_empty() {
            return None;
        }
        if self.previews.len() >= PREVIEWS_KEPT {
            self.previews.clear();
        }
        let compacting = self.compacting_in_place().map(str::to_string);
        let mut items = Vec::new();
        for key in keys.iter().take(ITEMS_MAX) {
            // Its data file had to itself by Compact Now: worked out once it's done.
            if compacting.as_deref() == Some(key.account.as_str()) {
                continue;
            }
            let (has_preview, said) = match self.list.iter().find(|m| m.key.uid == key.uid && m.key == *key) {
                Some(m) => (!m.preview.is_empty(), m.kind),
                None => (false, MessageKind::Unknown),
            };
            items.push((key.clone(), has_preview, said));
        }
        if items.is_empty() {
            return None;
        }
        self.previews_pending.extend(items.iter().map(|(k, _, _)| k.clone()));
        Some(PreviewWork { store, items, now: self.clock.now })
    }

    /// What the work read is kept (in memory only); its rows may be asked for again.
    pub fn finish_previews(&mut self, read: PreviewsRead) {
        for key in &read.given {
            self.previews_pending.remove(key);
        }
        self.previews.extend(read.read);
    }
}
