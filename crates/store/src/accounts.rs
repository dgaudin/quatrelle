// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The store: accounts' data, data files (kept or online only), and removing them.

use std::sync::atomic::AtomicBool;

use crate::*;

impl Store {
    /// The data file an account is kept in, when the cache keeps them so.
    pub fn data_file(&self, account: &str) -> Option<PathBuf> {
        if self.is_online_only(account) {
            return None;
        }
        match &self.files.main {
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => x.file(&safe_segment(account)),
            _ => None,
        }
    }

    /// The folder the data files are in, when the cache keeps them so.
    pub fn data_files_dir(&self) -> Option<PathBuf> {
        match &self.files.main {
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => Some(x.dir().to_path_buf()),
            _ => None,
        }
    }

    /// Keep an account online only (issue #4): its mail, folder lists, changes waiting for the
    /// server, Outbox, calendars and contacts are held in memory and never written anywhere, so
    /// nothing of it is left on this computer once Quatrelle closes. Turned on, what was kept on
    /// disk comes into memory (nothing is lost while Quatrelle runs) and its data file goes;
    /// turned off, what's held is written out to disk again.
    pub fn set_online_only(&self, account: &str, on: bool) -> Result<()> {
        if self.is_online_only(account) == on {
            return Ok(());
        }
        let dir = self.account_dir(account);
        let (from, to) = if on { (&self.files.main, &self.files.held) } else { (&self.files.held, &self.files.main) };
        let mut items = Vec::new();
        for p in from.walk(&dir) {
            if let Ok(bytes) = from.read(&p) {
                items.push((p, bytes));
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        to.write_many(&items)?;
        #[cfg(target_arch = "wasm32")]
        for (p, bytes) in &items {
            to.write(p, bytes)?;
        }
        {
            let mut online = self.files.online();
            if on {
                online.insert(safe_segment(account));
            } else {
                online.remove(&safe_segment(account));
            }
        }
        // Only once it's all on the other side: a failure above leaves it where it was. A data
        // file the system won't let go of yet (a sync writing to it) is switched all the same;
        // the next start, holding the account in memory from the outset, removes it.
        match from.remove_dir_all(&dir) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => log::warn!("{}: left for the next start: {e}", dir.display()),
            _ => {}
        }
        Ok(())
    }

    /// Whether an account is kept online only.
    pub fn is_online_only(&self, account: &str) -> bool {
        self.files.online().contains(&safe_segment(account))
    }

    /// Drop an online-only account's held files; `false` when it isn't one.
    pub(super) fn forget_online(&self, account: &str) -> bool {
        if !self.files.online().remove(&safe_segment(account)) {
            return false;
        }
        let _ = self.files.held.remove_dir_all(&self.account_dir(account));
        true
    }

    /// Where an account's mail is kept (to show the user): its data file, or its folder. Before
    /// anything is written, the folder its data file goes in (its own folder never exists then).
    pub fn account_path(&self, account: &str) -> PathBuf {
        self.data_file(account).or_else(|| self.data_files_dir()).unwrap_or_else(|| self.account_dir(account))
    }

    pub(super) fn account_dir(&self, account: &str) -> PathBuf {
        self.root.join(safe_segment(account))
    }

    /// Fill an account from an existing Quatrelle data file (`.quatrelle`): its folders, mail,
    /// calendar and contacts, as they were. The file is only read (from a copy). Messages are
    /// renumbered as this account's; returns how many there are.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn import_data_file(&self, account: &str, path: &Path) -> Result<usize> {
        let dir = self.account_dir(account);
        let tmp = self.root.join(format!(".import-{}.tmp", safe_segment(account)));
        packed::read_data_file(path, &tmp, &mut |batch| {
            // Paths come from a file: kept only when they stay inside the account. The old
            // account's changes still to make on its server, and its unsent mail, stay behind.
            let items: Vec<(PathBuf, Vec<u8>)> = batch
                .into_iter()
                .filter(|(k, _)| k != "ops.json" && !k.starts_with("outbox/"))
                .filter_map(|(k, v)| inner_path(&k).map(|rel| (dir.join(rel), v)))
                .collect();
            self.files.write_many(&items)
        })?;
        // Folders and messages name the account they belong to: this one now.
        let mut folders = self.load_folders(account)?;
        if folders.iter().any(|f| f.account != account) {
            for f in &mut folders {
                f.account = account.to_string();
            }
            self.save_folders(account, &folders)?;
        }
        let mut messages = 0;
        for f in folders {
            let mut st = self.load_folder(account, &f.path)?;
            messages += st.messages.len();
            if st.messages.iter().any(|m| m.key.account != account) {
                for m in &mut st.messages {
                    m.key.account = account.to_string();
                }
                self.save_folder(account, &f.path, &st)?;
            }
        }
        Ok(messages)
    }

    /// Bytes an account's cache takes on disk (message sources, summaries, the Outbox).
    pub fn account_size(&self, account: &str) -> u64 {
        self.files.size(&self.account_dir(account))
    }

    /// The bytes each of an account's folders takes here (its list and the messages kept), as
    /// Outlook's Folder Size shows a data file's (Local Data): each folder named by its place.
    pub fn folder_sizes(&self, account: &str) -> Result<Vec<quatrelle_model::FolderSize>> {
        let folders = self.load_folders(account)?;
        Ok(folders
            .iter()
            .map(|f| {
                let decoded = quatrelle_model::utf7::decode(&f.path);
                let path = match f.delimiter {
                    Some(d) => decoded.split(d).map(str::to_string).collect(),
                    None => vec![decoded],
                };
                let size = self.files.size(&self.folder_dir(account, &f.path));
                quatrelle_model::FolderSize { path, size, items: 0, folder: Some(f.path.clone()) }
            })
            .collect())
    }

    /// An account removed, as Outlook removes one: its data file stays where it is, closed and
    /// no one's (returned, when there is one), and what's kept outside data files goes.
    pub fn remove_account(&self, account: &str) -> Result<Option<PathBuf>> {
        let _gone = self.removing(account);
        if self.forget_online(account) {
            return Ok(None);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Files::Packed(x) = &self.files.main {
            let kept = x.let_go(&safe_segment(account));
            // A folder from before data files, if one is still there.
            let _ = fs::remove_dir_all(self.account_dir(account));
            return Ok(kept);
        }
        self.delete_files(account).map(|()| None)
    }

    /// Compact an account's data file, as Outlook's Compact Now: the space freed inside it goes
    /// back to the disk. Its size before and after; `None` when it has none (online only, or a
    /// cache that keeps no data files). Whatever else reads or writes the account meanwhile
    /// waits for it.
    pub fn compact_data_file(&self, account: &str) -> Result<Option<(u64, u64)>> {
        if self.is_online_only(account) {
            return Ok(None);
        }
        match &self.files.main {
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => {
                let file = x.file(&safe_segment(account)).unwrap_or_default();
                x.compact(&safe_segment(account)).map_err(io_err(&file))
            }
            _ => Ok(None),
        }
    }

    /// What an account's data file keeps raw that compresses (a build before R3 kept base64
    /// attachments so) compressed again, a few files at a time: each batch is compressed away from
    /// the file, then written in a short sealed transaction of its own, so a write from the window
    /// waits at most for one. `cancel` stops it between batches; the next call goes on from where
    /// it got to, as after a sleep or a kill. `progress` hears how far it has come as it starts
    /// and after each batch. The file gets smaller only once compacted (the space freed is used
    /// again meanwhile). `None` when the account has no data file, or is removed while it runs
    /// (its file isn't made again).
    pub fn recompress_data_file(&self, account: &str, cancel: &AtomicBool, progress: &mut dyn FnMut(Recompressing)) -> Result<Option<Recompressed>> {
        if self.is_online_only(account) {
            return Ok(None);
        }
        match &self.files.main {
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => {
                let a = safe_segment(account);
                // A removal waits for the batch being written, and the run stops after it.
                let seen = self.removals(account);
                let kept = |write: &mut dyn FnMut() -> io::Result<()>| self.while_kept(account, seen, write);
                match x.recompress(&a, cancel, progress, &kept) {
                    Err(_) if self.removals(account) != seen => Ok(None),
                    done => done.map_err(|source| StoreError::Io { path: a, source }),
                }
            }
            _ => {
                let _ = (cancel, progress);
                Ok(None)
            }
        }
    }

    /// Compact Now, whole: what an earlier build kept raw compressed again first
    /// ([`Store::recompress_data_file`]: the account is read and written as ever meanwhile), then
    /// the file compacted in place ([`Store::compact_data_file`]: what wants it waits, but a read
    /// from the window's thread is refused at once, [`crate::no_waiting`]). Without
    /// `recompress` (the drive hasn't the room for the file to grow while it's compressed again),
    /// only compacted. `progress` hears each step. Stopped by `cancel` while compressing, it isn't
    /// compacted (the next one goes on from there). Compressing again that fails (a damaged
    /// `META`, the disk) doesn't keep the file from being compacted. `None` when the account has
    /// no data file, or is removed meanwhile.
    pub fn compact_now(
        &self,
        account: &str,
        recompress: bool,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(CompactStep),
    ) -> Result<Option<Compacted>> {
        let Some(file) = self.data_file(account) else { return Ok(None) };
        // The room it takes on the disk, as File › Info says it.
        let size = || fs::metadata(&file).map_or(0, |m| crate::on_disk(&m));
        let before = size();
        let recompressed = match recompress {
            true => self.recompress_data_file(account, cancel, &mut |w| progress(CompactStep::Recompressing(w))),
            false => Ok(Some(Recompressed::default())),
        };
        let recompressed = match recompressed {
            Ok(None) => return Ok(None),
            Ok(Some(r)) => r,
            Err(e) => {
                log::warn!("{}: couldn't compress it again, compacted all the same: {e}", file.display());
                Recompressed::default()
            }
        };
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(Some(Compacted { before, after: size(), recompressed }));
        }
        progress(CompactStep::Compacting);
        let Some((_, after)) = self.compact_data_file(account)? else { return Ok(None) };
        Ok(Some(Compacted { before, after, recompressed }))
    }

    /// Forget everything about an account, its data file too (one that never held anyone's mail:
    /// a file that failed to open, an account starting again).
    pub fn delete_account(&self, account: &str) -> Result<()> {
        let _gone = self.removing(account);
        self.delete_files(account)
    }

    /// [`Store::delete_account`] once it's counted as removed.
    fn delete_files(&self, account: &str) -> Result<()> {
        if self.forget_online(account) {
            return Ok(());
        }
        let dir = self.account_dir(account);
        match self.files.remove_dir_all(&dir) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(io_err(&dir)(e)),
            _ => Ok(()),
        }
    }

    /// How many times `account` has been removed (or deleted): taken when a change for it is
    /// handed to another thread, to make it there with [`Store::while_kept`].
    pub fn removals(&self, account: &str) -> u64 {
        self.removals.read().unwrap_or_else(std::sync::PoisonError::into_inner).get(account).copied().unwrap_or(0)
    }

    /// `write` run unless `account` was removed since [`Store::removals`] said `seen`: a removal
    /// waits for one running, so what a change queued before it writes doesn't bring back the
    /// files of an account gone.
    pub fn while_kept<T>(&self, account: &str, seen: u64, write: impl FnOnce() -> T) -> Option<T> {
        let removals = self.removals.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        if removals.get(account).copied().unwrap_or(0) != seen {
            return None;
        }
        Some(write())
    }

    /// `account` counted as removed, held until its files are gone.
    fn removing(&self, account: &str) -> std::sync::RwLockWriteGuard<'_, HashMap<String, u64>> {
        let mut removals = self.removals.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        let n = removals.entry(account.to_string()).or_insert(0);
        *n = n.saturating_add(1);
        removals
    }
}

/// How far compressing a data file again has come ([`Store::recompress_data_file`]): files
/// looked at, of all those it has.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Recompressing {
    pub done: u64,
    pub total: u64,
}

/// What compressing a data file again did ([`Store::recompress_data_file`]), in this run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Recompressed {
    /// Files compressed again: how many, and their bytes as they were kept and as they are now.
    pub files: u64,
    pub before: u64,
    pub after: u64,
    /// Files tried that stayed as they were (they don't compress: pictures, archives), and their
    /// bytes.
    pub stayed_raw: u64,
    pub stayed_raw_bytes: u64,
    /// Files written again while they were being compressed: left as written.
    pub changed: u64,
    /// Files kept raw passed over, and their bytes as kept: damaged ones, and those bigger than a
    /// run takes (64 MiB). The data file says what every run passed over (`recompress.passed`).
    pub passed_over: u64,
    pub passed_over_bytes: u64,
    /// What every run of it has saved, as the data file records it.
    pub saved_in_all: u64,
    /// Everything was looked at: the data file says it's kept as this build keeps it (what's left
    /// raw doesn't compress, or was passed over). Otherwise it was stopped, and the next run goes
    /// on from where it got to.
    pub finished: bool,
}

/// A step of Compact Now ([`Store::compact_now`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompactStep {
    /// Compressing again what an earlier build kept raw: the window goes on as ever.
    Recompressing(Recompressing),
    /// Compacting in place: the file is had to itself.
    Compacting,
}

/// What Compact Now did ([`Store::compact_now`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Compacted {
    /// The data file's size before and after.
    pub before: u64,
    pub after: u64,
    /// What compressing again did first (nothing when the file had nothing kept raw).
    pub recompressed: Recompressed,
}
