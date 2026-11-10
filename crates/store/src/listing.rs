// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! A folder's list written a page at a time (an import's: the whole list needn't be held), kept
//! as it stands whenever asked ([`ListWriter::save`]), and gone on with after a stop
//! ([`Store::list_writer_from`]).

use crate::*;
use quatrelle_model::Flags;

/// A folder's list being written a page at a time (see [`Store::list_writer`]).
pub struct ListWriter {
    account: String,
    folder: String,
    /// Hashes of the pages written.
    pages: Vec<u64>,
    /// The first number in each page written.
    firsts: Vec<u32>,
    /// Messages not yet in a page.
    page: Vec<Summary>,
    total: usize,
    unread: u32,
    /// Going on with a list kept before: the marks beside its pages still hold.
    resumed: bool,
}

impl ListWriter {
    pub(crate) fn new(account: &str, folder: &str) -> ListWriter {
        ListWriter {
            account: account.to_string(),
            folder: folder.to_string(),
            pages: Vec::new(),
            firsts: Vec::new(),
            page: Vec::new(),
            total: 0,
            unread: 0,
            resumed: false,
        }
    }

    /// Add messages (in rising number order); full pages are written as they fill.
    pub fn push(&mut self, store: &Store, messages: Vec<Summary>) -> Result<()> {
        for m in messages {
            self.count(&m.flags);
            self.page.push(m);
            if self.page.len() == PAGE_MESSAGES {
                let hash = self.put_page(store)?;
                self.pages.push(hash);
                self.firsts.push(self.page.first().map_or(0, |m| m.key.uid));
                self.page.clear();
            }
        }
        Ok(())
    }

    fn count(&mut self, flags: &Flags) {
        self.total = self.total.saturating_add(1);
        if !flags.seen && !flags.deleted {
            self.unread = self.unread.saturating_add(1);
        }
    }

    /// Write the messages not yet in a page as the next one: its hash.
    fn put_page(&self, store: &Store) -> Result<u64> {
        let path = store.folder_dir(&self.account, &self.folder).join(format!("state-{}.json", self.pages.len()));
        let p = path.display().to_string();
        let bytes = serde_json::to_vec(&self.page).map_err(|source| StoreError::Json { path: p, source })?;
        store.files.write(&path, &bytes)?;
        Ok(page_hash(&bytes))
    }

    /// Keep the list as it stands (with `head`'s sync state), every message handed over so far
    /// in it, and go on: its (unread, total). An import does so as it goes, so a quit or a crash
    /// leaves a list that can be read.
    pub fn save(&self, store: &Store, head: &FolderState) -> Result<(u32, u32)> {
        let counts = (self.unread, u32::try_from(self.total).unwrap_or(u32::MAX));
        if self.pages.is_empty() {
            // Small: kept whole, as save_folder keeps it.
            let mut st = FolderState { messages: self.page.clone(), ..head.clone() };
            sort_newest_first(&mut st.messages);
            store.save_folder(&self.account, &self.folder, &st)?;
            return Ok(counts);
        }
        let (mut pages, mut firsts) = (self.pages.clone(), self.firsts.clone());
        if !self.page.is_empty() {
            // Written where the page will be once it's full.
            pages.push(self.put_page(store)?);
            firsts.push(self.page.first().map_or(0, |m| m.key.uid));
        }
        let dir = store.folder_dir(&self.account, &self.folder);
        let path = dir.join("state.json");
        let old: StateFile = store.read_json(&path).unwrap_or_default();
        let file = StateFile {
            validity: head.validity,
            uid_next: head.uid_next,
            highest_modseq: head.highest_modseq,
            messages: Vec::new(),
            window: head.window,
            count: self.total,
            pages,
            firsts,
        };
        let p = path.display().to_string();
        let bytes = serde_json::to_vec(&file).map_err(|source| StoreError::Json { path: p, source })?;
        // A new list: no marks from an old one put on it.
        let marks = dir.join(folders::MARKS);
        let mut writes = vec![(path, bytes)];
        if !self.resumed && store.has_marks(&marks) {
            writes.push((marks, b"[]".to_vec()));
        }
        store.write_files(&writes)?;
        for i in file.pages.len()..old.pages.len() {
            let _ = store.files.remove_file(&dir.join(format!("state-{i}.json")));
        }
        Ok(counts)
    }

    /// Write what's left and the list (with `head`'s sync state): its (unread, total).
    pub fn finish(self, store: &Store, head: &FolderState) -> Result<(u32, u32)> {
        self.save(store, head)
    }
}

impl Store {
    /// Write a folder's list a page at a time (importing a data file, the whole list needn't be
    /// held): messages are handed over in rising number order, then [`ListWriter::finish`]. The
    /// pages come out as [`Store::save_folder`] makes them.
    pub fn list_writer(&self, account: &str, folder: &str) -> ListWriter {
        ListWriter::new(account, folder)
    }

    /// Go on with the list a folder has (an import stopped part-way): the messages it keeps
    /// numbered under `uid_next` stay, those from there on are written again by what is handed
    /// over next. A full page that stays isn't written again, nor held: only read.
    pub fn list_writer_from(&self, account: &str, folder: &str, uid_next: u32) -> Result<ListWriter> {
        let dir = self.folder_dir(account, folder);
        let head: StateFile = self.read_json(&dir.join("state.json"))?;
        let mut w = ListWriter::new(account, folder);
        w.resumed = true;
        if head.pages.is_empty() {
            let mut kept: Vec<Summary> = head.messages.into_iter().filter(|m| m.key.uid < uid_next).collect();
            kept.sort_by_key(|m| m.key.uid);
            w.push(self, kept)?;
            return Ok(w);
        }
        // The flags marked since the pages were written, which stay beside them.
        let marks: HashMap<u32, Flags> = self.read_json::<Vec<(u32, Flags)>>(&dir.join(folders::MARKS))?.into_iter().collect();
        for i in 0..head.pages.len() {
            let path = dir.join(format!("state-{i}.json"));
            let bytes = match self.files.read(&path) {
                Ok(b) => b,
                Err(e) if e.kind() == io::ErrorKind::NotFound => break,
                Err(e) => return Err(io_err(&path)(e)),
            };
            // A damaged page ends the list there, like a missing one.
            let Ok(page) = serde_json::from_slice::<Vec<Summary>>(&bytes) else { break };
            if page.len() == PAGE_MESSAGES && page.iter().all(|m| m.key.uid < uid_next) {
                for m in &page {
                    w.count(marks.get(&m.key.uid).unwrap_or(&m.flags));
                }
                w.pages.push(page_hash(&bytes));
                w.firsts.push(page.first().map_or(0, |m| m.key.uid));
                continue;
            }
            // The page the list goes on in: the pages after it hold higher numbers only.
            let mut kept: Vec<Summary> = page.into_iter().filter(|m| m.key.uid < uid_next).collect();
            for m in &mut kept {
                if let Some(f) = marks.get(&m.key.uid) {
                    m.flags.clone_from(f);
                }
            }
            kept.sort_by_key(|m| m.key.uid);
            w.push(self, kept)?;
            break;
        }
        Ok(w)
    }
}
