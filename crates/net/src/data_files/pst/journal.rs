// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! An import that is followed, can be stopped and gone on with: its journal (`import.json`, an
//! account document) notes the data file and, folder by folder, the items dealt with. It is
//! written after each page of mail (and each folder, when enough came in since), after the lists
//! and the contacts and calendar, so a quit or a crash leaves at most items written but not
//! noted; going on, those are written again over themselves (their folder's next number is the
//! journal's), so nothing comes in twice.

use std::time::{Duration, Instant};

use quatrelle_model::{IMPORT_DOC, IMPORT_FILE_CHANGED, IMPORT_NO_JOURNAL, ImportJournal, ImportedFolder, MAX_IMPORT_FOLDERS, MAX_IMPORT_IDS};

use super::out::Open;
use super::*;

/// A folder's end writes the journal when this many items were dealt with since it was written…
const FOLDER_ITEMS: usize = 512;
/// …or this long has passed.
const FOLDER_WAIT: Duration = Duration::from_secs(2);

/// How an import is run.
#[derive(Clone, Copy, Default)]
pub struct Run<'a> {
    /// Asked before each item: true ends the import there, what came in kept.
    pub stop: Option<&'a (dyn Fn() -> bool + 'a)>,
    /// Go on with the account's import, from its journal, instead of starting one.
    pub resume: bool,
    /// Going on with a file that isn't the one the import began with (another size or date):
    /// bring in what the journal doesn't have, instead of ending with [`IMPORT_FILE_CHANGED`].
    pub allow_changed: bool,
}

/// How far an import has come.
#[derive(Clone, Copy, Debug)]
pub struct Progress<'a> {
    pub messages: usize,
    /// The messages of the folders that are imported; `None` when their counts can't be read.
    pub total: Option<usize>,
    /// The folder being read (its path in the account).
    pub folder: &'a str,
}

/// Who is told how far the import has come.
pub(super) enum Tell<'a> {
    /// The messages in so far, every so often.
    Count(&'a mut dyn FnMut(usize)),
    /// The same with the total and the folder, and at each folder begun.
    Full(&'a mut dyn FnMut(&Progress<'_>)),
}

/// An import's journal as it is kept while the import runs.
#[derive(Default)]
pub(super) struct Log {
    /// The journal; none (no path) for what is written without one (a test's own `Out`).
    journal: ImportJournal,
    /// Where each folder of the data file is in the journal, by its node.
    at: HashMap<u32, usize>,
    /// The items an earlier run dealt with, by folder node (sorted).
    before: HashMap<u32, Vec<u32>>,
    /// Folders an earlier run began, met again: their node and next number. Their list is
    /// opened again only when a message comes for it.
    pub(super) resumed: HashMap<String, (u32, u32)>,
    /// Folders the account has that the journal doesn't: made by the user, or by a run after
    /// the journal was last written. They are out of the way while the file's folders are made
    /// (one the file has is made again, its list from the start), and back at the end.
    aside: Vec<Folder>,
    /// Calendar items the account has past those the journal counts: written after it (they
    /// come in again, and are dropped here then), or made by the user since (they come back at
    /// the end). And the ids of those it counts: one that comes in again takes its place.
    events_aside: Vec<StoredEvent>,
    events_before: std::collections::HashSet<String>,
    /// Item ids in the journal.
    held: usize,
    /// Items dealt with since the journal was written, and when it was.
    since: usize,
    written: Option<Instant>,
    /// No longer written: it would be past its caps, or a batch of mail couldn't be written (its
    /// items are noted, so the journal as it was last written is the one to go on from).
    pub(super) off: bool,
}

/// A file's size and when it was last changed (milliseconds since 1970).
fn stamp(path: &Path) -> io::Result<(u64, i64)> {
    let meta = std::fs::metadata(path)?;
    let modified = meta.modified()?.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    Ok((meta.len(), modified))
}

/// The number in the id an import gave a contact, calendar item, note or task of `account`.
fn pim_number(uid: &str, account: &str) -> Option<usize> {
    uid.strip_prefix("pst-")?.strip_prefix(account)?.strip_prefix('-')?.strip_suffix("@quatrelle.invalid")?.parse().ok()
}

impl Log {
    fn on(&self) -> bool {
        self.journal.is_some()
    }

    /// The folder the data file's folder `node` became in an earlier run, and its next number.
    fn folder_before(&self, node: u32) -> Option<(String, u32)> {
        let f = self.journal.folders.get(*self.at.get(&node)?)?;
        (self.before.contains_key(&node) && !f.path.is_empty()).then(|| (f.path.clone(), f.uid_next.max(1)))
    }

    /// The journal's entry for the data file's folder `node`, made when it has none.
    fn entry(&mut self, node: u32, path: &str) -> Option<&mut ImportedFolder> {
        let at = match self.at.get(&node) {
            Some(at) => *at,
            None => {
                if self.journal.folders.len() >= MAX_IMPORT_FOLDERS {
                    self.off = true;
                    return None;
                }
                self.journal.folders.push(ImportedFolder { node, path: path.to_string(), uid_next: 1, ids: Vec::new() });
                let at = self.journal.folders.len().saturating_sub(1);
                self.at.insert(node, at);
                at
            }
        };
        self.journal.folders.get_mut(at)
    }

    /// Item `id` of the folder `node` is dealt with.
    fn note(&mut self, node: u32, id: u32) {
        if !self.on() || self.off {
            return;
        }
        if self.held >= MAX_IMPORT_IDS {
            log::warn!("import: more items than its journal notes ({MAX_IMPORT_IDS}); stopped from here, it starts over from there");
            self.off = true;
            return;
        }
        if let Some(f) = self.entry(node, "") {
            f.ids.push(id);
            self.held = self.held.saturating_add(1);
            self.since = self.since.saturating_add(1);
        }
    }

    fn set_next(&mut self, node: Option<u32>, uid_next: u32) {
        if let Some(f) = node.and_then(|n| self.at.get(&n)).and_then(|at| self.journal.folders.get_mut(*at)) {
            f.uid_next = uid_next;
        }
    }
}

impl<'a> Out<'a> {
    /// Told in full how far the import has come.
    pub(super) fn watched(store: &'a Store, account: &'a str, tell: Tell<'a>, run: Run<'a>) -> Out<'a> {
        let mut out = Out::told(store, account, tell);
        out.stop = run.stop;
        out
    }

    /// Begin the journal of the import of the data file at `path`: a new one, or the account's,
    /// with what an earlier run wrote taken up again.
    pub(super) fn begin(&mut self, path: &Path, run: &Run<'_>) -> Result<(), String> {
        let now = stamp(path);
        if !run.resume {
            let (size, modified) = now.unwrap_or_default();
            self.log.journal = ImportJournal { path: path.to_string_lossy().into_owned(), size, modified, ..Default::default() };
            return Ok(());
        }
        // A damaged journal was set aside as it was read, and reads as none.
        let mut journal: ImportJournal = self.store.load_account_doc(self.account, IMPORT_DOC).map_err(|e| e.to_string())?;
        if !journal.is_some() {
            return Err(IMPORT_NO_JOURNAL.into());
        }
        let (size, modified) = now.map_err(|e| format!("the data file can't be read ({e})"))?;
        if (size, modified) != (journal.size, journal.modified) && !run.allow_changed {
            return Err(IMPORT_FILE_CHANGED.into());
        }
        let noted: std::collections::HashSet<&str> = journal.folders.iter().map(|f| f.path.as_str()).collect();
        let (folders, aside) =
            self.store.load_folders(self.account).map_err(|e| e.to_string())?.into_iter().partition(|f| noted.contains(f.path.as_str()));
        (self.folders, self.log.aside) = (folders, aside);
        // Contacts, calendar items, notes and tasks written after the journal come in again:
        // they go first (those the user made since have other ids, and stay).
        let mut pim: Pim = self.store.load_account_doc(self.account, PIM_DOC).map_err(|e| e.to_string())?;
        let (account, next) = (self.account, journal.pim_next);
        let late = |uid: &str| pim_number(uid, account).is_some_and(|n| n >= next);
        pim.cards.retain(|c| !late(&c.card.uid));
        pim.events.retain(|e| !late(&e.event.uid));
        pim.notes.retain(|n| !late(&n.note.uid));
        pim.tasks.retain(|t| !late(&t.task.uid));
        // A calendar item may keep the id it had in the file: those past the journal's count
        // are set aside until the file's have come in.
        self.log.events_aside = pim.events.split_off(journal.pim_events.min(pim.events.len()));
        self.log.events_before = pim.events.iter().map(|e| e.event.uid.clone()).collect();
        self.pim = pim;
        (self.pim_floor, self.pim_files) = (journal.pim_next, journal.pim_files);
        (self.imported, self.skipped) = (journal.messages, journal.skipped);
        for (at, f) in journal.folders.iter_mut().enumerate() {
            f.ids.sort_unstable();
            f.ids.dedup();
            if let std::collections::hash_map::Entry::Vacant(e) = self.log.at.entry(f.node) {
                e.insert(at);
                self.log.before.insert(f.node, f.ids.clone());
            }
        }
        self.log.held = journal.ids();
        journal.path = path.to_string_lossy().into_owned();
        (journal.size, journal.modified, journal.finished) = (size, modified, false);
        self.log.journal = journal;
        Ok(())
    }

    /// The file's folders are about to be read, `total` messages in those that are imported:
    /// the journal is written a first time (an import cut short before its first page can be
    /// gone on with), and the count told.
    pub(super) fn started(&mut self, total: Option<usize>) {
        self.total = total;
        if self.log.on() {
            self.log.journal.total = total;
            self.write_journal(false);
        }
        self.tell(true);
    }

    /// Whether the import was asked to stop (asked before each item).
    pub(super) fn stop(&mut self) -> bool {
        if !self.stopped && self.stop.is_some_and(|asked| asked()) {
            self.stopped = true;
        }
        self.stopped
    }

    /// Whether an earlier run dealt with item `id` of the data file's folder `node`.
    pub(super) fn seen(&self, node: u32, id: u32) -> bool {
        self.log.before.get(&node).is_some_and(|ids| ids.binary_search(&id).is_ok())
    }

    /// Item `id` of the data file's folder `node` is dealt with: noted, and the journal written
    /// when a page of mail went out with it.
    pub(super) fn item_done(&mut self, node: u32, id: u32) {
        self.log.note(node, id);
        if std::mem::take(&mut self.page_out) {
            self.checkpoint();
        }
    }

    /// A folder is read to its end: the journal is written when enough came in since it was.
    pub(super) fn folder_done(&mut self) {
        if self.log.since >= FOLDER_ITEMS || (self.log.since > 0 && self.log.written.is_none_or(|t| t.elapsed() >= FOLDER_WAIT)) {
            self.checkpoint();
        }
    }

    /// The folder for the data file's folder `node`, named `name` under `parent`: the one an
    /// earlier run made for it, or a new one.
    pub(super) fn folder_for(&mut self, node: u32, parent: Option<&str>, name: &str) -> String {
        let path = match self.log.folder_before(node) {
            Some((path, next)) => {
                if self.folders.iter().any(|f| f.path == path) {
                    self.log.resumed.insert(path.clone(), (node, next));
                } else {
                    // Its list was written, the folder list not yet: the folder is made again.
                    let role = self.role_for(parent, &self.clean(name));
                    self.push_folder(&path, role);
                    self.reopen(&path, node, next);
                }
                path
            }
            None => {
                let path = self.add_folder(parent, name);
                if let Some(st) = self.states.get_mut(&path) {
                    st.node = Some(node);
                }
                if self.log.on() {
                    self.log.entry(node, &path);
                }
                path
            }
        };
        self.folder_now.clone_from(&path);
        self.tell(true);
        path
    }

    /// Open again the list of a folder an earlier run began, to go on from number `next`.
    pub(super) fn reopen(&mut self, path: &str, node: u32, next: u32) {
        match self.store.list_writer_from(self.account, path, next) {
            Ok(list) => {
                self.states.insert(path.to_string(), Open { node: Some(node), uid_next: next, waiting: Vec::new(), list });
            }
            Err(e) => log::warn!("import: a folder's list could not be gone on with: {e}"),
        }
    }

    /// Say how far the import has come (`folder`: a folder was begun, told only to who follows
    /// them).
    pub(super) fn tell(&mut self, folder: bool) {
        match &mut self.tell {
            Tell::Count(f) if !folder => f(self.imported),
            Tell::Count(_) => {}
            Tell::Full(f) => f(&Progress { messages: self.imported, total: self.total, folder: &self.folder_now }),
        }
    }

    /// Everything read so far kept: the mail and the lists of the folders being read, the
    /// contacts and calendar, then the journal, then the folder list.
    pub(super) fn checkpoint(&mut self) {
        if !self.log.on() {
            return;
        }
        self.flush();
        let mut kept = true;
        let paths: Vec<String> = self.states.keys().cloned().collect();
        for path in paths {
            self.list_out(&path);
            let Some(st) = self.states.get(&path) else { continue };
            let head = FolderState { validity: 1, uid_next: st.uid_next, ..Default::default() };
            match st.list.save(self.store, &head) {
                Ok((unread, total)) => {
                    self.log.set_next(st.node, st.uid_next);
                    if let Some(f) = self.folders.iter_mut().find(|f| f.path == path) {
                        (f.unread, f.total) = (unread, total);
                    }
                }
                Err(e) => {
                    log::warn!("import: {e}");
                    kept = false;
                }
            }
        }
        if self.pim_changed {
            match self.save_pim() {
                Ok(()) => self.pim_changed = false,
                Err(e) => {
                    log::warn!("import: {e}");
                    kept = false;
                }
            }
        }
        // What couldn't be kept isn't noted as kept: the journal stays as it was.
        if kept {
            self.write_journal(false);
        }
        let mut all = self.folders.clone();
        all.extend(self.log.aside.iter().filter(|f| !self.folders.iter().any(|o| o.path == f.path)).cloned());
        if let Err(e) = self.store.save_folders(self.account, &all) {
            log::warn!("import: {e}");
        }
    }

    /// A calendar item came in (the first of those from `from`): one of the same id that an
    /// earlier run wrote after its journal, or that the journal counts, gives way to it.
    pub(super) fn event_again(&mut self, from: usize) {
        if self.log.events_aside.is_empty() && self.log.events_before.is_empty() {
            return;
        }
        let Some(uid) = self.pim.events.get(from).map(|e| e.event.uid.clone()) else { return };
        self.log.events_aside.retain(|e| e.event.uid != uid);
        if self.log.events_before.remove(&uid) {
            let mut at = 0;
            self.pim.events.retain(|e| {
                at += 1;
                at > from || e.event.uid != uid
            });
        }
    }

    /// The contacts and calendar as they are kept: with the calendar items set aside.
    fn save_pim(&mut self) -> Result<(), String> {
        self.pim.ensure_defaults();
        let saved = if self.log.events_aside.is_empty() {
            self.store.save_account_doc(self.account, PIM_DOC, &self.pim)
        } else {
            let mut all = self.pim.clone();
            all.events.extend(self.log.events_aside.iter().cloned());
            self.store.save_account_doc(self.account, PIM_DOC, &all)
        };
        saved.map_err(|e| e.to_string())
    }

    /// The folders set aside while the file's were made come back, those not made again; and
    /// the calendar items that didn't come in again.
    pub(super) fn folders_back(&mut self) {
        let aside = std::mem::take(&mut self.log.events_aside);
        self.pim.events.extend(aside);
        for f in std::mem::take(&mut self.log.aside) {
            if !self.folders.iter().any(|o| o.path == f.path) {
                self.folders.push(f);
            }
        }
    }

    /// A folder's list is finished: its next number, for the journal.
    pub(super) fn closed(&mut self, node: Option<u32>, uid_next: u32) {
        self.log.set_next(node, uid_next);
    }

    /// Write the journal as it stands (`finished`: the whole file was read).
    pub(super) fn write_journal(&mut self, finished: bool) {
        if !self.log.on() || self.log.off {
            return;
        }
        let j = &mut self.log.journal;
        (j.messages, j.skipped, j.pim_next, j.pim_files, j.finished) = (self.imported, self.skipped, self.pim_floor, self.pim_files, finished);
        j.pim_events = self.pim.events.len();
        for f in &mut j.folders {
            f.ids.sort_unstable();
        }
        if !j.fits() {
            self.log.off = true;
            return;
        }
        match self.store.save_account_doc(self.account, IMPORT_DOC, &self.log.journal) {
            Ok(()) => (self.log.since, self.log.written) = (0, Some(Instant::now())),
            Err(e) => log::warn!("import: its journal could not be written: {e}"),
        }
    }
}
