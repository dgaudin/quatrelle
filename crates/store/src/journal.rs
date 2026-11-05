// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The store: changes waiting for the server (RFC 4549), a file each. They were all in one
//! `ops.json`, read and written again whole at each change added or taken out: the thousandth
//! change of a burst made offline rewrote the 999 before it. Each is now `ops/<number>.json`
//! (a key of its own in a data file, written in one sealed commit), numbered in the order made;
//! an `ops.json` from before is read first, as the head of the queue, and empties as its changes
//! go out (nothing is moved, so nothing is ever there twice).

use crate::*;

/// The changes waiting, before this journal: read first, taken out of as they go.
const OLD: &str = "ops.json";
/// The folder of the journal, in the account's.
const DIR: &str = "ops";

impl Store {
    fn ops_dir(&self, account: &str) -> PathBuf {
        self.account_dir(account).join(DIR)
    }

    /// The journal's files in the order made (by their number); others there are let be.
    fn op_files(&self, account: &str) -> Vec<(u64, PathBuf)> {
        let mut files: Vec<(u64, PathBuf)> = self
            .files
            .list(&self.ops_dir(account))
            .into_iter()
            .filter_map(|(name, p)| Some((name.strip_suffix(".json")?.parse().ok()?, p)))
            .collect();
        files.sort_unstable_by_key(|(n, _)| *n);
        files
    }

    /// A change of the journal; `None` when it's gone, or damaged (set aside, like any damaged
    /// cache file: one change lost, not the queue).
    fn read_op(&self, path: &Path) -> Result<Option<Op>> {
        match self.files.read(path) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(op) => Ok(Some(op)),
                Err(source) => {
                    log::warn!("{}: unreadable, set aside: {source}", path.display());
                    let _ = self.files.rename(path, &path.with_extension("json.bad"));
                    Ok(None)
                }
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io_err(path)(e)),
        }
    }

    /// The changes waiting, oldest first.
    pub fn load_ops(&self, account: &str) -> Result<Vec<Op>> {
        let mut ops: Vec<Op> = self.read_json(&self.account_dir(account).join(OLD))?;
        for (_, path) in self.op_files(account) {
            ops.extend(self.read_op(&path)?);
        }
        Ok(ops)
    }

    /// The oldest change waiting (what goes to the server next), the others not read.
    pub fn first_op(&self, account: &str) -> Result<Option<Op>> {
        let old: Vec<Op> = self.read_json(&self.account_dir(account).join(OLD))?;
        if let Some(op) = old.into_iter().next() {
            return Ok(Some(op));
        }
        for (_, path) in self.op_files(account) {
            if let Some(op) = self.read_op(&path)? {
                return Ok(Some(op));
            }
        }
        Ok(None)
    }

    /// Add `op` after the others: a file of its own, numbered after the last (the lock keeps
    /// two threads from taking the same number). Nothing else is read or written.
    pub fn add_op(&self, account: &str, op: Op) -> Result<()> {
        let _held = self.changes.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let next = self.op_files(account).last().map_or(1, |(n, _)| n.saturating_add(1));
        self.write_json(&self.ops_dir(account).join(format!("{next:020}.json")), &op)
    }

    /// Take the first change equal to `op` out of the queue, if there is one. One from before
    /// this journal that can't be read stops it: written over, the changes in it were lost.
    pub fn remove_op(&self, account: &str, op: &Op) -> Result<()> {
        let _held = self.changes.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let old_path = self.account_dir(account).join(OLD);
        let mut old: Vec<Op> = self.read_json(&old_path)?;
        if let Some(i) = old.iter().position(|o| o == op) {
            old.remove(i);
            return self.save_old_ops(&old_path, &old);
        }
        for (_, path) in self.op_files(account) {
            if self.read_op(&path)?.as_ref() == Some(op) {
                return match self.files.remove_file(&path) {
                    Err(e) if e.kind() != io::ErrorKind::NotFound => Err(io_err(&path)(e)),
                    _ => Ok(()),
                };
            }
        }
        Ok(())
    }

    /// Replace the whole queue with `ops` (in order).
    pub fn save_ops(&self, account: &str, ops: &[Op]) -> Result<()> {
        let _held = self.changes.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        self.save_old_ops(&self.account_dir(account).join(OLD), &[])?;
        for (_, path) in self.op_files(account) {
            match self.files.remove_file(&path) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(io_err(&path)(e)),
                _ => {}
            }
        }
        let dir = self.ops_dir(account);
        let mut writes = Vec::with_capacity(ops.len());
        for (i, op) in ops.iter().enumerate() {
            let path = dir.join(format!("{:020}.json", i.saturating_add(1)));
            let bytes = serde_json::to_vec(op).map_err(|source| StoreError::Json { path: path.display().to_string(), source })?;
            writes.push((path, bytes));
        }
        if writes.is_empty() { Ok(()) } else { self.write_files(&writes) }
    }

    /// The queue from before this journal, written again (gone once empty).
    fn save_old_ops(&self, path: &Path, ops: &[Op]) -> Result<()> {
        if ops.is_empty() {
            return match self.files.remove_file(path) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => Err(io_err(path)(e)),
                _ => Ok(()),
            };
        }
        self.write_json(path, ops)
    }
}
