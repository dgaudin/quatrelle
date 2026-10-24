//! The store: folders' lists of messages, and changes applied to them.

use std::borrow::Borrow;

use crate::*;

impl Store {
    pub fn load_folder(&self, account: &str, folder: &str) -> Result<FolderState> {
        let dir = self.folder_dir(account, folder);
        let file: StateFile = self.read_json(&dir.join("state.json"))?;
        let mut st = FolderState {
            validity: file.validity,
            uid_next: file.uid_next,
            highest_modseq: file.highest_modseq,
            messages: file.messages,
            window: file.window,
        };
        // A big folder's list in pages (by number): put together, newest first again.
        if !file.pages.is_empty() {
            for i in 0..file.pages.len() {
                let page: Vec<Summary> = self.read_json(&dir.join(format!("state-{i}.json")))?;
                st.messages.extend(page);
            }
            sort_newest_first(&mut st.messages);
        }
        // Keys are rebuilt rather than trusted: a renamed or copied cache stays consistent.
        for m in &mut st.messages {
            m.key.account = account.to_string();
            m.key.folder = folder.to_string();
            m.key.validity = st.validity;
        }
        Ok(st)
    }

    /// A number that changes whenever a folder's list does (read from its head alone: a big
    /// folder's pages are named there by their hash), 0 when it has none: what was worked out
    /// from a list can be trusted while its version stays the same.
    pub fn folder_version(&self, account: &str, folder: &str) -> u64 {
        self.files.read(&self.folder_dir(account, folder).join("state.json")).map(|b| page_hash(&b)).unwrap_or(0)
    }

    /// Whether a folder's list is kept here, numbered under `validity`: read from its head alone
    /// (a big folder's pages aren't loaded to know).
    pub fn folder_kept(&self, account: &str, folder: &str, validity: u32) -> bool {
        self.read_json::<StateFile>(&self.folder_dir(account, folder).join("state.json")).is_ok_and(|head| head.validity == validity)
    }

    /// Keep a folder's list. A big one goes in pages of [`PAGE_MESSAGES`] by number, and only
    /// the pages that changed are written: one message marked read in a folder of hundreds of
    /// thousands rewrote all of it (hundreds of megabytes, held in memory twice over).
    pub fn save_folder<M: Borrow<Summary>>(&self, account: &str, folder: &str, state: &FolderState<M>) -> Result<()> {
        self.save_folder_counted(account, folder, state, None).map(drop)
    }

    /// [`Store::save_folder`] after only these messages' flags changed (the same messages, in
    /// number): only their pages are made again (a big folder's other pages aren't even turned
    /// into text to see they're the same).
    pub fn save_folder_changed<M: Borrow<Summary>>(
        &self,
        account: &str,
        folder: &str,
        state: &FolderState<M>,
        changed: &std::collections::HashSet<u32>,
    ) -> Result<()> {
        self.save_folder_counted(account, folder, state, Some(changed)).map(drop)
    }

    /// [`Store::save_folder`], saying how many files it wrote (the list and the pages that
    /// changed).
    pub(crate) fn save_folder_counted<M: Borrow<Summary>>(
        &self,
        account: &str,
        folder: &str,
        state: &FolderState<M>,
        changed: Option<&std::collections::HashSet<u32>>,
    ) -> Result<usize> {
        // Edits still on their way to the server win over what this list was made from: a sync
        // that read the folder before a message was marked read (moved, deleted) and saves after
        // put the server's old state back — bold, normal, bold again until the edit got there.
        let pending = self.pending_edits(account, folder);
        let mutated;
        let widened;
        let (messages, changed): (Vec<&Summary>, _) = if pending.is_empty() {
            (state.messages.iter().map(M::borrow).collect(), changed)
        } else {
            let mut st = state.with(state.messages.iter().map(|m| m.borrow().clone()).collect());
            let touched = reapply(&mut st, account, folder, &pending);
            mutated = st;
            widened = changed.map(|c| c.union(&touched).copied().collect::<std::collections::HashSet<u32>>());
            (mutated.messages.iter().collect(), widened.as_ref())
        };
        let dir = self.folder_dir(account, folder);
        let head = dir.join("state.json");
        let json_err = |p: &Path| {
            let p = p.display().to_string();
            move |source| StoreError::Json { path: p, source }
        };
        let old: StateFile = self.read_json(&head).unwrap_or_default();
        let mut file = StateFile {
            validity: state.validity,
            uid_next: state.uid_next,
            highest_modseq: state.highest_modseq,
            messages: Vec::new(),
            window: state.window,
            pages: Vec::new(),
            count: 0,
        };
        let mut writes: Vec<(PathBuf, Vec<u8>)> = Vec::new();
        if messages.len() <= PAGE_MESSAGES {
            file.messages = messages.into_iter().cloned().collect();
        } else {
            let mut by_number = messages;
            by_number.sort_by_key(|m| m.key.uid);
            file.count = by_number.len();
            // The same messages as last time, some flags changed: the other pages stand.
            let same = changed.filter(|_| old.count == by_number.len() && old.pages.len() == by_number.len().div_ceil(PAGE_MESSAGES));
            for (i, page) in by_number.chunks(PAGE_MESSAGES).enumerate() {
                if let Some(changed) = same
                    && let Some(&hash) = old.pages.get(i)
                    && !page.iter().any(|m| changed.contains(&m.key.uid))
                {
                    file.pages.push(hash);
                    continue;
                }
                let path = dir.join(format!("state-{i}.json"));
                let bytes = serde_json::to_vec(page).map_err(json_err(&path))?;
                let hash = page_hash(&bytes);
                if old.pages.get(i) != Some(&hash) || !self.files.is_file(&path) {
                    writes.push((path, bytes));
                }
                file.pages.push(hash);
            }
        }
        writes.push((head.clone(), serde_json::to_vec(&file).map_err(json_err(&head))?));
        // The list and its pages together (one transaction in a data file).
        self.write_files(&writes)?;
        // Pages the list no longer has.
        for i in file.pages.len()..old.pages.len() {
            let _ = self.files.remove_file(&dir.join(format!("state-{i}.json")));
        }
        Ok(writes.len())
    }

    /// The edits waiting in ops.json that touch messages of this folder.
    fn pending_edits(&self, account: &str, folder: &str) -> Vec<Op> {
        let touches = |keys: &[MessageKey]| keys.iter().any(|k| k.account == account && k.folder == folder);
        self.load_ops(account)
            .unwrap_or_default()
            .into_iter()
            .filter(|op| match op {
                Op::SetFlag { keys, .. } | Op::SetKeyword { keys, .. } | Op::Expunge { keys } => touches(keys),
                Op::Move { keys, to } => to != folder && touches(keys),
                _ => false,
            })
            .collect()
    }

    /// Write a folder's list a page at a time (importing a data file, the whole list needn't be
    /// held): messages are handed over in rising number order, then [`ListWriter::finish`]. The
    /// pages come out as [`Store::save_folder`] makes them.
    pub fn list_writer(&self, account: &str, folder: &str) -> ListWriter {
        ListWriter { account: account.to_string(), folder: folder.to_string(), pages: Vec::new(), page: Vec::new(), total: 0, unread: 0 }
    }

    /// Several files at once (one transaction in a data file).
    pub(super) fn write_files(&self, items: &[(PathBuf, Vec<u8>)]) -> Result<()> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.files.write_many(items)
        }
        #[cfg(target_arch = "wasm32")]
        {
            for (p, bytes) in items {
                self.files.write(p, bytes)?;
            }
            Ok(())
        }
    }

    /// A small document kept with a folder's cache (`name` a plain file name): it moves with the
    /// folder and goes with it ([`Store::clear_folder`]). Empty when missing or damaged.
    pub fn load_folder_doc<T: DeserializeOwned + Default>(&self, account: &str, folder: &str, name: &str) -> Result<T> {
        self.read_json(&self.folder_dir(account, folder).join(safe_doc(name)))
    }

    pub fn save_folder_doc<T: Serialize + ?Sized>(&self, account: &str, folder: &str, name: &str, value: &T) -> Result<()> {
        self.write_json(&self.folder_dir(account, folder).join(safe_doc(name)), value)
    }

    /// Forget a folder's cache (UIDVALIDITY changed, or the folder was deleted on the server).
    pub fn clear_folder(&self, account: &str, folder: &str) -> Result<()> {
        let dir = self.folder_dir(account, folder);
        match self.files.remove_dir_all(&dir) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(io_err(&dir)(e)),
            _ => Ok(()),
        }
    }

    /// Apply a change to the cached folders at once (optimistic update). Returns the folders that
    /// changed. Moves only remove from the source: the destination learns the new UID at its
    /// next sync.
    pub fn apply_op(&self, account: &str, op: &Op) -> Result<Vec<String>> {
        if op.is_folder_op() {
            return self.apply_folder_op(account, op);
        }
        if let Op::EmptyFolder { path } = op {
            let mut st = self.load_folder(account, path)?;
            for m in &st.messages {
                self.remove_body(&m.key);
            }
            st.messages.clear();
            self.save_folder(account, path, &st)?;
            let mut list = self.load_folders(account)?;
            for f in list.iter_mut().filter(|f| &f.path == path) {
                f.total = 0;
                f.unread = 0;
            }
            self.save_folders(account, &list)?;
            return Ok(vec![path.clone()]);
        }
        let mut folders: Vec<String> = op.keys().iter().filter(|k| k.account == account).map(|k| k.folder.clone()).collect();
        folders.sort();
        folders.dedup();
        for folder in &folders {
            let mut st = self.load_folder(account, folder)?;
            let in_folder = |k: &MessageKey| k.account == account && &k.folder == folder && k.validity == st.validity;
            match op {
                Op::SetFlag { keys, flag, on } => {
                    // A set: every message of a folder may change at once (looked up in a list, that was quadratic).
                    let uids: HashSet<u32> = keys.iter().filter(|k| in_folder(k)).map(|k| k.uid).collect();
                    for m in st.messages.iter_mut().filter(|m| uids.contains(&m.key.uid)) {
                        flag.set(&mut m.flags, *on);
                    }
                }
                Op::SetKeyword { keys, keyword, on, name } => {
                    // A set: every message of a folder may change at once (looked up in a list, that was quadratic).
                    let uids: HashSet<u32> = keys.iter().filter(|k| in_folder(k)).map(|k| k.uid).collect();
                    for m in st.messages.iter_mut().filter(|m| uids.contains(&m.key.uid)) {
                        m.flags.set_category(keyword, name.as_deref(), *on);
                    }
                }
                Op::Move { keys, to } => {
                    if to == folder {
                        continue;
                    }
                    // A set: every message of a folder may change at once (looked up in a list, that was quadratic).
                    let uids: HashSet<u32> = keys.iter().filter(|k| in_folder(k)).map(|k| k.uid).collect();
                    st.messages.retain(|m| !uids.contains(&m.key.uid));
                }
                Op::Expunge { keys } => {
                    let gone: Vec<&MessageKey> = keys.iter().filter(|k| in_folder(k)).collect();
                    for k in &gone {
                        self.remove_body(k);
                    }
                    let uids: HashSet<u32> = gone.iter().map(|k| k.uid).collect();
                    st.messages.retain(|m| !uids.contains(&m.key.uid));
                }
                Op::CreateFolder { .. } | Op::RenameFolder { .. } | Op::DeleteFolder { .. } | Op::EmptyFolder { .. } => {}
            }
            self.save_folder(account, folder, &st)?;
        }
        Ok(folders)
    }

    /// Folder-list changes: `folders.json` is updated and each folder's cache moves with it (or
    /// goes, for a deleted folder). Returns the folders whose contents should be shown afresh.
    pub(super) fn apply_folder_op(&self, account: &str, op: &Op) -> Result<Vec<String>> {
        let mut list = self.load_folders(account)?;
        let delimiter = list.iter().find_map(|f| f.delimiter);
        let changed = match op {
            Op::CreateFolder { path } => {
                if !list.iter().any(|f| &f.path == path) {
                    // Made in another mailbox (a shared one), it's in that mailbox too.
                    let parent = delimiter.and_then(|d| path.rsplit_once(d)).map(|(p, _)| p);
                    let mailbox = list.iter().find(|f| Some(f.path.as_str()) == parent).and_then(|f| f.mailbox.clone());
                    list.push(Folder {
                        account: account.to_string(),
                        path: path.clone(),
                        delimiter,
                        role: quatrelle_model::FolderRole::Other,
                        selectable: true,
                        mailbox,
                        ..Default::default()
                    });
                }
                vec![path.clone()]
            }
            Op::RenameFolder { from, to } => {
                let mut moved = Vec::new();
                for f in list.iter_mut() {
                    if let Some(new) = quatrelle_model::rebase_path(&f.path, from, to, f.delimiter.or(delimiter)) {
                        let (src, dst) = (self.folder_dir(account, &f.path), self.folder_dir(account, &new));
                        if self.files.exists(&src) {
                            let _ = self.files.remove_dir_all(&dst);
                            self.files.rename(&src, &dst).map_err(io_err(&src))?;
                        }
                        f.path = new.clone();
                        moved.push(new);
                    }
                }
                moved
            }
            Op::DeleteFolder { path } => {
                let gone: Vec<String> =
                    list.iter().filter(|f| quatrelle_model::is_within(&f.path, path, f.delimiter.or(delimiter))).map(|f| f.path.clone()).collect();
                for g in &gone {
                    self.clear_folder(account, g)?;
                }
                list.retain(|f| !gone.contains(&f.path));
                Vec::new()
            }
            Op::SetFlag { .. } | Op::SetKeyword { .. } | Op::Move { .. } | Op::Expunge { .. } | Op::EmptyFolder { .. } => Vec::new(),
        };
        quatrelle_model::sort_folders(&mut list);
        self.save_folders(account, &list)?;
        Ok(changed)
    }
}

/// Edits not yet on the server, made again on a folder's list (they're what the user sees until
/// the server has them): flags and keywords set, messages moved away or deleted taken out. The
/// numbers of the messages changed.
fn reapply(st: &mut FolderState, account: &str, folder: &str, ops: &[Op]) -> HashSet<u32> {
    let mut touched = HashSet::new();
    let validity = st.validity;
    let uids = |keys: &[MessageKey]| -> HashSet<u32> {
        keys.iter().filter(|k| k.account == account && k.folder == folder && k.validity == validity).map(|k| k.uid).collect()
    };
    for op in ops {
        match op {
            Op::SetFlag { keys, flag, on } => {
                let wanted = uids(keys);
                for m in st.messages.iter_mut().filter(|m| wanted.contains(&m.key.uid)) {
                    let before = m.flags.clone();
                    flag.set(&mut m.flags, *on);
                    if m.flags != before {
                        touched.insert(m.key.uid);
                    }
                }
            }
            Op::SetKeyword { keys, keyword, on, name } => {
                let wanted = uids(keys);
                for m in st.messages.iter_mut().filter(|m| wanted.contains(&m.key.uid)) {
                    let before = m.flags.clone();
                    m.flags.set_category(keyword, name.as_deref(), *on);
                    if m.flags != before {
                        touched.insert(m.key.uid);
                    }
                }
            }
            Op::Move { keys, .. } | Op::Expunge { keys } => {
                let gone = uids(keys);
                st.messages.retain(|m| !gone.contains(&m.key.uid));
            }
            _ => {}
        }
    }
    touched
}
