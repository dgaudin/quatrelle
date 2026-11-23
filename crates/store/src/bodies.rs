// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The store: message bodies, kept search indexes, and the Outbox.

use crate::*;

#[cfg(test)]
mod tests;

/// The end of the mark a message taken out of the Outbox has beside its files while any of them
/// is left ([`Store::outbox_remove`]).
const GONE: &str = ".gone";

impl Store {
    pub fn read_body(&self, key: &MessageKey) -> Option<Vec<u8>> {
        self.reads.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.files.read(&self.body_path(key)).ok()
    }

    /// How many message sources were read since the cache opened (clones count together): a
    /// search that needs none can be shown to read none.
    pub fn reads(&self) -> u64 {
        self.reads.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// A message's source, `None` when it isn't kept here (never downloaded, or damaged past
    /// reading: like a damaged cache file, never worth stopping over); an error when it is but
    /// couldn't be read now, so that what goes on to rewrite it stops rather than lose it.
    pub fn load_body(&self, key: &MessageKey) -> Result<Option<Vec<u8>>> {
        self.reads.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = self.body_path(key);
        match self.files.read(&path) {
            Ok(raw) => Ok(Some(raw)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                log::warn!("{}: damaged, taken as missing: {e}", path.display());
                Ok(None)
            }
            Err(e) => Err(io_err(&path)(e)),
        }
    }

    pub fn has_body(&self, key: &MessageKey) -> bool {
        self.files.is_file(&self.body_path(key))
    }

    pub fn write_body(&self, key: &MessageKey, raw: &[u8]) -> Result<()> {
        self.files.write(&self.body_path(key), raw)
    }

    /// Many message sources at once: one transaction for all of them in a data file (importing
    /// hundreds of thousands, a commit each spent most of the time waiting for the disk).
    pub fn write_bodies(&self, bodies: &[(MessageKey, Vec<u8>)]) -> Result<()> {
        let items: Vec<(PathBuf, Vec<u8>)> = bodies.iter().map(|(k, raw)| (self.body_path(k), raw.clone())).collect();
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.files.write_many(&items)
        }
        #[cfg(target_arch = "wasm32")]
        {
            for (p, raw) in &items {
                self.files.write(p, raw)?;
            }
            Ok(())
        }
    }

    pub fn remove_body(&self, key: &MessageKey) {
        let _ = self.files.remove_file(&self.body_path(key));
    }

    /// Where a folder's kept search index lives, beside what it caches of its messages.
    pub(super) fn search_index_path(&self, account: &str, folder: &str) -> PathBuf {
        self.folder_dir(account, folder).join("search.idx")
    }

    /// Keep a folder's search index (its bytes are `quatrelle_model::search::FolderIndex`'s): the
    /// next run reads it back instead of reading the folder's messages again.
    pub fn save_search_index(&self, account: &str, folder: &str, bytes: &[u8]) -> Result<()> {
        self.files.write(&self.search_index_path(account, folder), bytes)
    }

    /// The index kept for a folder, if any (a damaged one is refused by its reader).
    pub fn load_search_index(&self, account: &str, folder: &str) -> Option<Vec<u8>> {
        self.files.read(&self.search_index_path(account, folder)).ok()
    }

    pub fn remove_search_index(&self, account: &str, folder: &str) {
        let _ = self.files.remove_file(&self.search_index_path(account, folder));
    }

    pub(super) fn outbox_dir(&self, account: &str) -> PathBuf {
        self.account_dir(account).join("outbox")
    }

    /// Queue a message to send when the account is back online. A mark an earlier message of the
    /// id left ([`Store::outbox_remove`]) goes first: left, it would hide this one, which would
    /// then never go and never be seen; one that can't go is an error, and nothing is queued.
    pub fn outbox_add(&self, account: &str, item: &OutboxItem, raw: &[u8]) -> Result<()> {
        let dir = self.outbox_dir(account);
        let mark = dir.join(format!("{}{GONE}", item.id));
        if let Some(e) = self.left_after_removal(&mark) {
            return Err(io_err(&mark)(e));
        }
        self.files.write(&dir.join(format!("{}.eml", item.id)), raw)?;
        self.write_json(&dir.join(format!("{}.json", item.id)), item)
    }

    /// Update a queued message's bookkeeping (attempts, last error).
    pub fn outbox_update(&self, account: &str, item: &OutboxItem) -> Result<()> {
        self.write_json(&self.outbox_dir(account).join(format!("{}.json", item.id)), item)
    }

    /// Queued messages, oldest first: each with its source, and not taken out
    /// ([`Store::outbox_remove`]).
    pub fn outbox(&self, account: &str) -> Vec<OutboxItem> {
        let files = self.files.list(&self.outbox_dir(account));
        let there: HashSet<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
        let mut items: Vec<OutboxItem> = files
            .iter()
            .filter(|(name, _)| name.ends_with(".json"))
            .filter_map(|(_, p)| self.files.read(p).ok())
            .filter_map(|b| serde_json::from_slice::<OutboxItem>(&b).ok())
            .filter(|i| there.contains(format!("{}.eml", i.id).as_str()) && !there.contains(format!("{}{GONE}", i.id).as_str()))
            .collect();
        items.sort_by_key(|i| (i.queued, i.id));
        items
    }

    pub fn outbox_raw(&self, account: &str, id: u64) -> Option<Vec<u8>> {
        self.files.read(&self.outbox_dir(account).join(format!("{id}.eml"))).ok()
    }

    /// A message taken out of the Outbox (sent, deleted, or taken back to be changed): never
    /// listed again, so never sent again. A mark (`<id>.gone`) is written beside it first, then its
    /// files are removed, then the mark once none is left: whatever stops on the way (a file
    /// another program holds, a failing disk, a removal that answers as if done and leaves the
    /// file), the mark keeps the message out of the list, and [`Store::outbox_sweep`] removes what
    /// is left later. Only [`Store::outbox_add`] of the same id lists one again. An error only when
    /// the message may still be listed: the mark couldn't be written, and neither file went.
    pub fn outbox_remove(&self, account: &str, id: u64) -> Result<()> {
        let dir = self.outbox_dir(account);
        let mark = dir.join(format!("{id}{GONE}"));
        let marked = self.files.write(&mark, b"");
        let mut left: Vec<(PathBuf, io::Error)> = Vec::new();
        for name in [format!("{id}.json"), format!("{id}.eml")] {
            let path = dir.join(name);
            if let Some(e) = self.left_after_removal(&path) {
                left.push((path, e));
            }
        }
        match (marked, left.len()) {
            // Nothing left to hide: the mark goes too (left, the sweep takes it).
            (Ok(()), 0) => {
                let _ = self.files.remove_file(&mark);
                Ok(())
            }
            (Ok(()), n) => {
                log::warn!("{account}: an Outbox message taken out left {n} of its files; its mark keeps it out");
                Ok(())
            }
            (Err(_), 0) => Ok(()),
            // Without one of its files it isn't listed: what's left is litter, not mail to send.
            (Err(_), 1) => {
                log::warn!("{account}: an Outbox message taken out left one of its files, and no mark");
                Ok(())
            }
            (Err(e), _) => {
                log::warn!("{account}: an Outbox message couldn't be taken out: neither its files nor a mark could be changed");
                Err(left.into_iter().next().map_or(e, |(path, why)| io_err(&path)(why)))
            }
        }
    }

    /// What [`Store::outbox_remove`] left of the messages taken out, removed now where it can be:
    /// a marked message's files, then its mark once neither of them is left. One pass over the
    /// Outbox's files, each mark tried once.
    pub fn outbox_sweep(&self, account: &str) {
        let dir = self.outbox_dir(account);
        let (mut swept, mut held) = (0usize, 0usize);
        for (name, mark) in self.files.list(&dir) {
            let Some(id) = name.strip_suffix(GONE).filter(|id| id.parse::<u64>().is_ok()) else { continue };
            // Both tried, each time.
            let left = [".json", ".eml"].iter().filter(|ext| self.left_after_removal(&dir.join(format!("{id}{ext}"))).is_some()).count();
            if left > 0 {
                held += 1;
            } else if self.left_after_removal(&mark).is_none() {
                swept += 1;
            }
        }
        if swept > 0 || held > 0 {
            log::info!("{account}: Outbox: {swept} messages taken out earlier cleared away, {held} still have files left");
        }
    }

    /// Removes a file of the Outbox, and says why it's still there when it is: its removal failed,
    /// or answered as if done and left it (a data file that couldn't look whether it was there).
    /// One that wasn't there is gone.
    fn left_after_removal(&self, path: &Path) -> Option<io::Error> {
        match self.files.remove_file(path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Some(e),
            _ if self.files.is_file(path) => Some(io::Error::other("still there after its removal")),
            _ => None,
        }
    }

    /// Cached sources across all folders of an account, as (key, path) pairs, for indexing.
    pub fn cached_bodies(&self, account: &str, folder: &str, validity: u32) -> Vec<u32> {
        let dir = self.folder_dir(account, folder).join(validity.to_string());
        self.files.list(&dir).into_iter().filter_map(|(n, _)| n.strip_suffix(".eml").and_then(|n| n.parse().ok())).collect()
    }
}
