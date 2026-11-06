// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The store: folders' lists of messages, and changes applied to them.

use std::borrow::Borrow;
use std::collections::BTreeMap;

use quatrelle_model::Flags;

use crate::*;

mod marks;
pub(crate) use marks::MARKS;
pub use marks::MARKS_MAX;
mod noted;
pub(crate) use noted::Notings;

pub use noted::{NOTED_MAX, NoteGuard, Noted};

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
        // A big folder's list in pages (by number): put together, newest first again, with the
        // flags marked since its pages were written.
        if !file.pages.is_empty() {
            for i in 0..file.pages.len() {
                let page: Vec<Summary> = self.read_json(&dir.join(format!("state-{i}.json")))?;
                st.messages.extend(page);
            }
            sort_newest_first(&mut st.messages);
            let marks: Vec<(u32, Flags)> = self.read_json(&dir.join(MARKS))?;
            if !marks.is_empty() {
                let marks: HashMap<u32, Flags> = marks.into_iter().collect();
                for m in &mut st.messages {
                    if let Some(f) = marks.get(&m.key.uid) {
                        m.flags.clone_from(f);
                    }
                }
            }
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
    /// from a list can be trusted while its version stays the same. Not its flags: a big
    /// folder's marks are kept beside its pages ([`MARKS`]) and leave the head as it was, so
    /// what's worked out from the version mustn't depend on a message's flags (what's still to
    /// download doesn't).
    pub fn folder_version(&self, account: &str, folder: &str) -> u64 {
        self.files.read(&self.folder_dir(account, folder).join("state.json")).map(|b| page_hash(&b)).unwrap_or(0)
    }

    /// The messages of a folder's list kept in the same page as message `uid` (marked as the
    /// list is, keys rebuilt as [`Store::load_folder`] makes them): a search that finds a few
    /// messages in a big folder looks at them there, not in a list of tens of thousands read
    /// whole. A small list is its own page; a list with no first numbers in its head (written
    /// before them) is read whole. Not in any order.
    pub fn load_page_of(&self, account: &str, folder: &str, uid: u32) -> Result<Vec<Summary>> {
        let dir = self.folder_dir(account, folder);
        let file: StateFile = self.read_json(&dir.join("state.json"))?;
        if !file.pages.is_empty() && file.firsts.len() != file.pages.len() {
            return self.load_folder(account, folder).map(|st| st.messages);
        }
        let mut messages = if file.pages.is_empty() {
            file.messages
        } else {
            // The last page whose first number isn't past it (the first, for one below them all).
            let i = file.firsts.partition_point(|&f| f <= uid).saturating_sub(1);
            let mut page: Vec<Summary> = self.read_json(&dir.join(format!("state-{i}.json")))?;
            let marks: Vec<(u32, Flags)> = self.read_json(&dir.join(MARKS))?;
            if !marks.is_empty() {
                let marks: HashMap<u32, Flags> = marks.into_iter().collect();
                for m in &mut page {
                    if let Some(f) = marks.get(&m.key.uid) {
                        m.flags.clone_from(f);
                    }
                }
            }
            page
        };
        for m in &mut messages {
            m.key.account = account.to_string();
            m.key.folder = folder.to_string();
            m.key.validity = file.validity;
        }
        Ok(messages)
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
        self.save_list(account, folder, state, changed, true, &mut Vec::new())
    }

    /// [`Store::save_folder_counted`]; `waiting`: the edits waiting in the journal made again on
    /// the list first (not for a list just read from here, which has them); `extra`: other files
    /// written in the same transaction (taken from it).
    fn save_list<M: Borrow<Summary>>(
        &self,
        account: &str,
        folder: &str,
        state: &FolderState<M>,
        changed: Option<&std::collections::HashSet<u32>>,
        waiting: bool,
        extra: &mut Vec<(PathBuf, Vec<u8>)>,
    ) -> Result<usize> {
        // The marks beside the pages read and written with nothing in between (a sync's save
        // puts them into the pages while the edit writer keeps one more).
        let _held = self.changes.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        // Edits still on their way to the server win over what this list was made from: a sync
        // that read the folder before a message was marked read (moved, deleted) and saves after
        // put the server's old state back — bold, normal, bold again until the edit got there.
        let pending = if waiting { self.pending_edits(account, folder) } else { Vec::new() };
        let reapplied;
        let widened;
        let (messages, changed): (Vec<&Summary>, _) = if pending.is_empty() {
            (state.messages.iter().map(M::borrow).collect(), changed)
        } else {
            let (list, touched) = reapply(state.messages.iter().map(M::borrow), account, folder, state.validity, &pending);
            reapplied = list;
            widened = changed.map(|c| c.union(&touched).copied().collect::<std::collections::HashSet<u32>>());
            (reapplied.iter().map(|m| &**m).collect(), widened.as_ref())
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
            firsts: Vec::new(),
            count: 0,
        };
        let mut writes: Vec<(PathBuf, Vec<u8>)> = Vec::new();
        let marks_path = dir.join(MARKS);
        // What was saved, for what changed to be noted ([`Store::note_list`]): the list, and
        // for a list in pages the flags changed alone and what was written.
        let listed: Vec<&Summary>;
        let mut noted_same: Option<std::collections::HashSet<u32>> = None;
        let paged: bool;
        if messages.len() <= PAGE_MESSAGES {
            file.messages = messages.iter().map(|m| Summary::clone(m)).collect();
            listed = messages;
            paged = false;
        } else {
            let n = messages.len();
            // The same messages as last time, some flags changed: the other pages stand.
            let same = changed.filter(|_| old.count == n && old.pages.len() == n.div_ceil(PAGE_MESSAGES));
            // Few marked since the pages were written: those kept beside them, nothing else.
            let mut folded: Option<std::collections::HashSet<u32>> = None;
            if let Some(changed) = same {
                let kept: Vec<(u32, Flags)> = self.read_json(&marks_path)?;
                let mut marks: BTreeMap<u32, &Flags> = kept.iter().map(|(uid, f)| (*uid, f)).collect();
                // Each one marked as the list has it now (one pass over it).
                for m in &messages {
                    if changed.contains(&m.key.uid) || marks.contains_key(&m.key.uid) {
                        marks.insert(m.key.uid, &m.flags);
                    }
                }
                if marks.len() <= MARKS_MAX {
                    let list: Vec<(u32, &Flags)> = marks.into_iter().collect();
                    let bytes = serde_json::to_vec(&list).map_err(json_err(&marks_path))?;
                    let mut writes = vec![(marks_path, bytes)];
                    writes.append(extra);
                    let saved = self.note_list((account, folder, &dir), &old, &messages, state.validity, Some(changed), None);
                    self.write_files(&writes)?;
                    self.note_saved(account, folder, saved);
                    return Ok(writes.len());
                }
                // Too many: their pages are written again (as the list has them now), and the
                // marks emptied.
                folded = Some(marks.into_keys().collect());
            }
            let same = folded.as_ref().or(same);
            let mut by_number = messages;
            by_number.sort_by_key(|m| m.key.uid);
            file.count = n;
            for (i, page) in by_number.chunks(PAGE_MESSAGES).enumerate() {
                file.firsts.push(page.first().map_or(0, |m| m.key.uid));
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
            noted_same = same.cloned();
            listed = by_number;
            paged = true;
        }
        // The pages have every flag now: the marks beside them emptied, with them.
        if self.has_marks(&marks_path) {
            writes.push((marks_path, b"[]".to_vec()));
        }
        writes.push((head.clone(), serde_json::to_vec(&file).map_err(json_err(&head))?));
        writes.append(extra);
        // What it changes, worked out while the pages before are there.
        let pages = paged.then_some((file.firsts.as_slice(), file.pages.as_slice()));
        let saved = self.note_list((account, folder, &dir), &old, &listed, state.validity, noted_same.as_ref(), pages);
        // The list and its pages together (one transaction in a data file).
        self.write_files(&writes)?;
        self.note_saved(account, folder, saved);
        // Pages the list no longer has.
        for i in file.pages.len()..old.pages.len() {
            let _ = self.files.remove_file(&dir.join(format!("state-{i}.json")));
        }
        Ok(writes.len())
    }

    /// The edits waiting in the journal that touch messages of this folder.
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
        ListWriter {
            account: account.to_string(),
            folder: folder.to_string(),
            pages: Vec::new(),
            firsts: Vec::new(),
            page: Vec::new(),
            total: 0,
            unread: 0,
        }
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
        self.note_unknown(account, folder);
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
        self.apply_op_with(account, op, &mut Vec::new())
    }

    /// [`Store::apply_op`], `extra` written in the same transaction as the first folder's list
    /// or marks (taken from it; left there when no folder was written).
    pub(super) fn apply_op_with(&self, account: &str, op: &Op, extra: &mut Vec<(PathBuf, Vec<u8>)>) -> Result<Vec<String>> {
        if op.is_folder_op() {
            return self.apply_folder_op(account, op);
        }
        if let Op::EmptyFolder { path } = op {
            let mut st = self.load_folder(account, path)?;
            // The list first: one that can't be saved still has its messages' sources.
            let gone = std::mem::take(&mut st.messages);
            self.save_folder(account, path, &st)?;
            for m in &gone {
                self.remove_body(&m.key);
            }
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
            // A flag or category in a big folder: the pages the messages are in alone read, and
            // their marks written beside them.
            if matches!(op, Op::SetFlag { .. } | Op::SetKeyword { .. }) && self.mark_in_pages(account, folder, op, extra)? {
                continue;
            }
            let mut st = self.load_folder(account, folder)?;
            let mut sources_gone: Vec<&MessageKey> = Vec::new();
            // A flag or category: those messages' marks alone are kept (beside a big folder's
            // pages), the list just read having every edit waiting already.
            let mut marked: Option<HashSet<u32>> = None;
            let in_folder = |k: &MessageKey| k.account == account && &k.folder == folder && k.validity == st.validity;
            match op {
                Op::SetFlag { keys, flag, on } => {
                    // A set: every message of a folder may change at once (looked up in a list, that was quadratic).
                    let uids: HashSet<u32> = keys.iter().filter(|k| in_folder(k)).map(|k| k.uid).collect();
                    for m in st.messages.iter_mut().filter(|m| uids.contains(&m.key.uid)) {
                        flag.set(&mut m.flags, *on);
                    }
                    marked = Some(uids);
                }
                Op::SetKeyword { keys, keyword, on, name } => {
                    // A set: every message of a folder may change at once (looked up in a list, that was quadratic).
                    let uids: HashSet<u32> = keys.iter().filter(|k| in_folder(k)).map(|k| k.uid).collect();
                    for m in st.messages.iter_mut().filter(|m| uids.contains(&m.key.uid)) {
                        m.flags.set_category(keyword, name.as_deref(), *on);
                    }
                    marked = Some(uids);
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
                    sources_gone = keys.iter().filter(|k| in_folder(k)).collect();
                    let uids: HashSet<u32> = sources_gone.iter().map(|k| k.uid).collect();
                    st.messages.retain(|m| !uids.contains(&m.key.uid));
                }
                Op::CreateFolder { .. } | Op::RenameFolder { .. } | Op::DeleteFolder { .. } | Op::EmptyFolder { .. } => {}
            }
            // The list just read has every edit waiting already: none is made again on it (a
            // burst of moves made offline made them all again at each one, over the whole list).
            self.save_list(account, folder, &st, marked.as_ref(), false, extra)?;
            // Once the list no longer has them: one that can't be saved keeps their sources.
            for k in sources_gone {
                self.remove_body(k);
            }
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

/// What an edit waiting does to one message.
enum Effect<'o> {
    Flag(&'o quatrelle_model::Flag, bool),
    Category(&'o str, Option<&'o str>, bool),
    /// Moved away or deleted.
    Gone,
}

/// Edits not yet on the server, made again on a folder's list (they're what the user sees until
/// the server has them): flags and keywords set, messages moved away or deleted taken out. One
/// pass over the list, each message's edits in the order made (a table number -> effects):
/// each edit went over a copy of the whole list, O(edits x messages) at every save of a sync.
/// Messages no edit changes are borrowed, not copied. The list, and the numbers of the messages
/// an edit changed (one later taken out or changed back included, as each edit saw it).
pub(crate) fn reapply<'a>(
    messages: impl Iterator<Item = &'a Summary>,
    account: &str,
    folder: &str,
    validity: u32,
    ops: &[Op],
) -> (Vec<std::borrow::Cow<'a, Summary>>, HashSet<u32>) {
    let mut effects: HashMap<u32, Vec<Effect<'_>>> = HashMap::new();
    for op in ops {
        let effect = match op {
            Op::SetFlag { flag, on, .. } => Effect::Flag(flag, *on),
            Op::SetKeyword { keyword, on, name, .. } => Effect::Category(keyword, name.as_deref(), *on),
            Op::Move { .. } | Op::Expunge { .. } => Effect::Gone,
            _ => continue,
        };
        // Each message once an edit (a key named twice is changed once).
        let uids: HashSet<u32> =
            op.keys().iter().filter(|k| k.account == account && k.folder == folder && k.validity == validity).map(|k| k.uid).collect();
        for uid in uids {
            let list = effects.entry(uid).or_default();
            list.push(match &effect {
                Effect::Flag(f, on) => Effect::Flag(f, *on),
                Effect::Category(k, n, on) => Effect::Category(k, *n, *on),
                Effect::Gone => Effect::Gone,
            });
        }
    }
    let mut touched = HashSet::new();
    let mut out = Vec::new();
    for m in messages {
        let Some(list) = effects.get(&m.key.uid) else {
            out.push(std::borrow::Cow::Borrowed(m));
            continue;
        };
        let mut flags = m.flags.clone();
        let mut gone = false;
        for e in list {
            let before = flags.clone();
            match e {
                Effect::Flag(f, on) => f.set(&mut flags, *on),
                Effect::Category(k, n, on) => flags.set_category(k, *n, *on),
                Effect::Gone => {
                    gone = true;
                    break;
                }
            }
            if flags != before {
                touched.insert(m.key.uid);
            }
        }
        if gone {
            continue;
        }
        if flags == m.flags {
            out.push(std::borrow::Cow::Borrowed(m));
        } else {
            let mut changed = m.clone();
            changed.flags = flags;
            out.push(std::borrow::Cow::Owned(changed));
        }
    }
    (out, touched)
}
