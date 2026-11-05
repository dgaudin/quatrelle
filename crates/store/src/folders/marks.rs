// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The store: a big folder's marks, the flags of its messages changed since its pages were
//! written, kept beside them (adapted from upstream 7ab836b, there for the window's own saves;
//! here for the edit writer's `Store::apply_op` too).

use std::collections::BTreeSet;

use super::*;

/// Beside a list kept in pages: the flags of the messages marked since its pages were written
/// (number, flags), so a message marked read writes a few bytes, not the page of thousands it's
/// in (about 2 MB, every page turned into text again to see which changed, on the edit writer's
/// thread at each Down that marks a message read). The next save of the whole list puts them
/// into the pages and empties it. Only flags: the head (`state.json`) stays as it was, so
/// [`Store::folder_version`], which hashes the head alone, doesn't change with a mark.
pub(crate) const MARKS: &str = "state-marks.json";

/// The most messages kept marked beside a list's pages (about 50 KB): one more puts them into
/// their pages.
pub const MARKS_MAX: usize = 512;

impl Store {
    /// Whether the marks beside a list's pages ([`MARKS`]) have any.
    pub(crate) fn has_marks(&self, path: &Path) -> bool {
        self.files.read(path).is_ok_and(|b| b.as_slice() != b"[]")
    }

    /// A flag or category changed on messages of a big folder (`Store::apply_op`, the edit
    /// writer's): their marks kept beside its pages, their flags read from the pages they're in
    /// alone (found by each page's first number, kept in the head) and from the marks before.
    /// It read every page of the folder and wrote again the one each message is in: 100 to 170
    /// ms a mark on a folder of 50,000. False: not a list in pages so numbered (a small one, a
    /// head from before), or more marked than the marks hold: the whole list is read then.
    /// `extra`: other files written in the same transaction (taken from it when written).
    pub(super) fn mark_in_pages(&self, account: &str, folder: &str, op: &Op, extra: &mut Vec<(PathBuf, Vec<u8>)>) -> Result<bool> {
        let _held = self.changes.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = self.folder_dir(account, folder);
        let head: StateFile = self.read_json(&dir.join("state.json"))?;
        if head.pages.is_empty() || head.firsts.len() != head.pages.len() {
            return Ok(false);
        }
        let uids: BTreeSet<u32> =
            op.keys().iter().filter(|k| k.account == account && k.folder == folder && k.validity == head.validity).map(|k| k.uid).collect();
        let marks_path = dir.join(MARKS);
        let kept: Vec<(u32, Flags)> = self.read_json(&marks_path)?;
        let mut marks: BTreeMap<u32, Flags> = kept.into_iter().collect();
        if marks.len().saturating_add(uids.len()) > MARKS_MAX {
            return Ok(false);
        }
        // The page each is in: the last whose first number isn't past it.
        let pages: BTreeSet<usize> = uids.iter().filter_map(|&uid| head.firsts.partition_point(|&f| f <= uid).checked_sub(1)).collect();
        for i in pages {
            let page: Vec<Summary> = self.read_json(&dir.join(format!("state-{i}.json")))?;
            for m in page.into_iter().filter(|m| uids.contains(&m.key.uid)) {
                // Marked before: as the marks have it, not the page.
                let flags = marks.entry(m.key.uid).or_insert(m.flags);
                match op {
                    Op::SetFlag { flag, on, .. } => flag.set(flags, *on),
                    Op::SetKeyword { keyword, on, name, .. } => flags.set_category(keyword, name.as_deref(), *on),
                    _ => {}
                }
            }
        }
        let list: Vec<(u32, Flags)> = marks.into_iter().collect();
        let bytes = serde_json::to_vec(&list).map_err(|source| StoreError::Json { path: marks_path.display().to_string(), source })?;
        let mut writes = vec![(marks_path, bytes)];
        writes.append(extra);
        self.write_files(&writes)?;
        Ok(true)
    }

    /// A flag or category changed on a local account's messages (no server to send it to, no
    /// journal), and the counts of the folders it touched as they are now (path, unread, total):
    /// the marks and the account's `folders.json`, when its counts differ, written in one
    /// transaction (the idea of upstream 7ab836b's `save_marks`). Two commits a message, on the
    /// window's thread, made holding Down cost 25 ms a step. Returns the folders changed.
    pub fn apply_marks(&self, account: &str, op: &Op, counts: &[(String, u32, u32)]) -> Result<Vec<String>> {
        let mut extra = Vec::new();
        let mut folders = self.load_folders(account)?;
        let mut differ = false;
        for (path, unread, total) in counts {
            for f in folders.iter_mut().filter(|f| &f.path == path && (f.unread != *unread || f.total != *total)) {
                f.unread = *unread;
                f.total = *total;
                differ = true;
            }
        }
        if differ {
            let path = self.account_dir(account).join("folders.json");
            let bytes = serde_json::to_vec(&folders).map_err(|source| StoreError::Json { path: path.display().to_string(), source })?;
            extra.push((path, bytes));
        }
        let changed = self.apply_op_with(account, op, &mut extra)?;
        // No folder written (none of its messages kept here): the counts alone.
        if !extra.is_empty() {
            self.write_files(&extra)?;
        }
        Ok(changed)
    }
}
