// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The store: changes waiting for the server (RFC 4549), a file each. They were all in one
//! `ops.json`, read and written again whole at each change added or taken out: the thousandth
//! change of a burst made offline rewrote the 999 before it. Each is now `ops/<number>.json`
//! (a key of its own in a data file, written in one sealed commit), numbered in the order made;
//! an `ops.json` from before is read first, as the head of the queue, and empties as its changes
//! go out (nothing is moved, so nothing is ever there twice). A change that can't be read is never
//! passed over: the queue reads as an error up to it ([`StoreError::Damaged`]), and it goes aside
//! with every change made after it ([`Store::set_ops_aside_from`]).

use crate::*;

/// The changes waiting, before this journal: read first, taken out of as they go.
const OLD: &str = "ops.json";
/// The folder of the journal, in the account's.
const DIR: &str = "ops";
/// Where changes that can't be sent are kept aside, in the account's folder: never read as changes.
const ASIDE: &str = "ops-aside";
/// The most times an account's changes are kept aside (`ops-aside/`, then `ops-aside/2`…).
const ASIDE_MAX: u32 = 1000;

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

    /// A change of the journal; `None` when it's gone. A damaged one is an error that names it
    /// ([`StoreError::Damaged`]), left where it is: passed over, the changes made after it were
    /// sent as if it had never been made.
    fn read_op(&self, path: &Path) -> Result<Option<Op>> {
        match self.files.read(path) {
            // Not what serde says: it quotes the file.
            Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|_| StoreError::Damaged { path: path.display().to_string() }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io_err(path)(e)),
        }
    }

    /// The changes waiting, oldest first; the first that is damaged makes it an error.
    pub fn load_ops(&self, account: &str) -> Result<Vec<Op>> {
        let mut ops: Vec<Op> = self.read_json_strict(&self.account_dir(account).join(OLD))?;
        for (_, path) in self.op_files(account) {
            ops.extend(self.read_op(&path)?);
        }
        Ok(ops)
    }

    /// The oldest change waiting (what goes to the server next), the others not read; an error
    /// when it is damaged.
    pub fn first_op(&self, account: &str) -> Result<Option<Op>> {
        let old: Vec<Op> = self.read_json_strict(&self.account_dir(account).join(OLD))?;
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
    /// this journal that can't be read stops it: written over, the changes in it were lost; and so
    /// does a damaged one before `op`.
    pub fn remove_op(&self, account: &str, op: &Op) -> Result<()> {
        let _held = self.changes.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let old_path = self.account_dir(account).join(OLD);
        let mut old: Vec<Op> = self.read_json_strict(&old_path)?;
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

    /// `path` (one of the journal's files, or the queue from before it) and every change made after
    /// it, moved as they are into the account's `ops-aside/`: never read as changes again. How many.
    /// A later time goes in a folder of its own (`ops-aside/2`…): nothing kept aside before is
    /// written over, nor removed. None moves when `path` isn't one of the queue's, or reads (read
    /// again here): a change that reads is never kept from the server.
    pub fn set_ops_aside_from(&self, account: &str, path: &Path) -> Result<usize> {
        let _held = self.changes.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let old = self.account_dir(account).join(OLD);
        let damaged = if path == old {
            matches!(self.read_json_strict::<Vec<Op>>(&old), Err(StoreError::Damaged { .. }))
        } else {
            matches!(self.read_op(path), Err(StoreError::Damaged { .. }))
        };
        if !damaged {
            return Ok(0);
        }
        let journal = self.op_files(account).into_iter().map(|(_, p)| p);
        // The queue from before is the head: everything is after it.
        let going: Vec<PathBuf> =
            if path == old { std::iter::once(old.clone()).chain(journal).collect() } else { journal.skip_while(|p| p != path).collect() };
        if going.is_empty() {
            return Ok(0);
        }
        let dir = self.aside_dir(account)?;
        // Copied whole first, in one go: one that can't be leaves the queue as it was.
        let mut copies = Vec::with_capacity(going.len());
        for from in &going {
            let bytes = self.files.read(from).map_err(io_err(from))?;
            copies.push((dir.join(from.file_name().unwrap_or_default()), bytes));
        }
        self.write_files(&copies)?;
        // Then out of the queue, the last first: cut short, what stays still ends with `path`, and
        // nothing made after it is ever sent before it.
        for from in going.iter().rev() {
            match self.files.remove_file(from) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(io_err(from)(e)),
                _ => {}
            }
        }
        log::warn!(
            "{}: can't be read; it and the {} changes made after it are kept aside in {}",
            path.display(),
            going.len().saturating_sub(1),
            dir.display()
        );
        Ok(going.len())
    }

    /// Where the account's changes go aside this time: `ops-aside/` while it holds none, else the
    /// first of `ops-aside/2`, `ops-aside/3`… not there.
    fn aside_dir(&self, account: &str) -> Result<PathBuf> {
        let first = self.account_dir(account).join(ASIDE);
        if self.files.try_list(&first).map_err(io_err(&first))?.is_empty() {
            return Ok(first);
        }
        (2..=ASIDE_MAX).map(|n| first.join(n.to_string())).find(|d| !self.files.exists(d)).ok_or_else(|| StoreError::Io {
            path: first.display().to_string(),
            source: io::Error::other("changes were kept aside too many times already"),
        })
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
