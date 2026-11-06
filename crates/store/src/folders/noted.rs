// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The store: what saves of a folder's list change in it, noted while a sync asks
//! ([`Store::note_changes`]), so the window can take the change as it is. Told only that the
//! folder changed, the window read all of it again from the disk and built its list: 110 to 170
//! ms on a folder of 50,000 messages, at every sync that brought a message.
//!
//! Worked out where the list is saved, from what's on the disk before: whatever a sync does
//! (IMAP, JMAP, Graph, EWS, POP, and the saves they make on the way), what changed is what the
//! list on the disk says, not what each of them would say it did. A small list is in its head;
//! a big one's pages are read only where they differ: a page kept holds the numbers between its
//! first and the next page's first, and the messages the new list has there turned into text
//! are the page again, by its hash, when nothing changed in it.

use quatrelle_model::ListChange;

use super::*;

/// The most messages noted changed in a folder: past it, what changed isn't told (the window
/// reads the folder again, as for a first sync).
pub const NOTED_MAX: usize = 4_096;

/// The folders whose changes are noted, by account and folder.
pub(crate) type Notings = HashMap<(String, String), Noting>;

/// What saves of one folder's list changed since it was last told.
#[derive(Debug, Default)]
pub(crate) struct Noting {
    /// How many note it (a sync, and another of the same folder at the same time).
    noters: usize,
    /// Each message changed, by number: as the list has it now, or `None` when it went.
    changed: BTreeMap<u32, Option<Summary>>,
    /// The list after the last save: its numbering, how many, the print of their numbers.
    validity: u32,
    count: usize,
    print: u64,
    /// What changed can't be told: a list numbered anew or emptied away, a head from before
    /// (no first numbers), too much changed, a page that couldn't be read.
    unknown: bool,
}

/// What a save changes in a list ([`Store::note_list`]): each message come, changed (`Some`) or
/// gone (`None`), or `None` when it can't be told; and the list after.
pub(super) struct Saved {
    diff: Option<Vec<(u32, Option<Summary>)>>,
    validity: u32,
    count: usize,
    print: u64,
}

/// What was noted of a folder's list.
#[derive(Debug, PartialEq, Eq)]
pub enum Noted {
    /// Nothing in it changed.
    Nothing,
    /// What changed.
    Changed(Box<ListChange>),
    /// It changed, in a way that isn't told (read it again).
    Unknown,
}

/// A folder's changes noted while this is held ([`Store::note_changes`]); dropped, noting stops.
#[must_use]
#[derive(Debug)]
pub struct NoteGuard {
    store: Store,
    key: (String, String),
    done: bool,
}

impl NoteGuard {
    /// What changed until now, told once: what's noted starts again from here.
    pub fn so_far(&self) -> Noted {
        self.store.drain_noted(&self.key, false)
    }

    /// What changed since it was last told; noting stops.
    pub fn take(mut self) -> Noted {
        self.done = true;
        self.store.drain_noted(&self.key, true)
    }
}

impl Drop for NoteGuard {
    fn drop(&mut self) {
        if !self.done {
            let _ = self.store.drain_noted(&self.key, true);
        }
    }
}

impl Store {
    /// Note what the saves of a folder's list change in it, until the guard is dropped or taken.
    pub fn note_changes(&self, account: &str, folder: &str) -> NoteGuard {
        let key = (account.to_string(), folder.to_string());
        let mut all = self.noting.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let n = all.entry(key.clone()).or_default();
        n.noters = n.noters.saturating_add(1);
        NoteGuard { store: self.clone(), key, done: false }
    }

    fn drain_noted(&self, key: &(String, String), stop: bool) -> Noted {
        let mut all = self.noting.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(n) = all.get_mut(key) else { return Noted::Nothing };
        let noted = if n.unknown {
            Noted::Unknown
        } else if n.changed.is_empty() {
            Noted::Nothing
        } else {
            let mut change = ListChange { validity: n.validity, count: n.count, print: n.print, ..ListChange::default() };
            for (uid, m) in std::mem::take(&mut n.changed) {
                match m {
                    Some(m) => change.upserts.push(m),
                    None => change.removed.push(uid),
                }
            }
            Noted::Changed(Box::new(change))
        };
        n.changed.clear();
        n.unknown = false;
        if stop {
            n.noters = n.noters.saturating_sub(1);
            if n.noters == 0 {
                all.remove(key);
            }
        }
        noted
    }

    /// Whether a folder's changes are noted.
    pub(crate) fn noting(&self, account: &str, folder: &str) -> bool {
        let all = self.noting.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        !all.is_empty() && all.contains_key(&(account.to_string(), folder.to_string()))
    }

    /// A folder's list changed in a way that isn't told (emptied, renamed, numbered anew).
    pub(super) fn note_unknown(&self, account: &str, folder: &str) {
        let mut all = self.noting.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(n) = all.get_mut(&(account.to_string(), folder.to_string())) {
            n.changed.clear();
            n.unknown = true;
        }
    }

    /// What saving `messages` (in any order) over `old`, the list's head on the disk, changes,
    /// worked out before it's written (a big list's pages are read as they were), when the
    /// folder's changes are noted: [`Store::note_saved`] notes it once it's written. `same`:
    /// only these messages' flags changed (the same messages, in pages numbered as before).
    /// `pages`: the first numbers and hashes of the pages to write, when it's in pages (what's
    /// the same there isn't turned into text again).
    pub(super) fn note_list(
        &self,
        at: (&str, &str, &Path),
        old: &StateFile,
        messages: &[&Summary],
        validity: u32,
        same: Option<&HashSet<u32>>,
        pages: Option<(&[u32], &[u64])>,
    ) -> Option<Saved> {
        let (account, folder, _) = at;
        if !self.noting(account, folder) {
            return None;
        }
        let had = old.validity != 0 || !old.messages.is_empty() || !old.pages.is_empty();
        let diff = if had && old.validity != validity {
            None
        } else if let Some(same) = same {
            Some(messages.iter().filter(|m| same.contains(&m.key.uid)).map(|m| (m.key.uid, Some(Summary::clone(m)))).collect())
        } else if old.pages.is_empty() {
            let keyed = |mut m: Summary| {
                m.key = MessageKey { account: account.to_string(), folder: folder.to_string(), validity: old.validity, uid: m.key.uid };
                m
            };
            diff(old.messages.iter().cloned().map(keyed), messages)
        } else {
            let mut by_number = messages.to_vec();
            if !by_number.is_sorted_by_key(|m| m.key.uid) {
                by_number.sort_by_key(|m| m.key.uid);
            }
            self.diff_pages(at, old, &by_number, pages)
        };
        Some(Saved { diff, validity, count: messages.len(), print: ListChange::print(messages.iter().map(|m| m.key.uid)) })
    }

    /// What [`Store::note_list`] worked out, noted now that the list is written.
    pub(super) fn note_saved(&self, account: &str, folder: &str, saved: Option<Saved>) {
        let Some(saved) = saved else { return };
        let mut all = self.noting.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(n) = all.get_mut(&(account.to_string(), folder.to_string())) else { return };
        (n.validity, n.count, n.print) = (saved.validity, saved.count, saved.print);
        match saved.diff {
            Some(d) if !n.unknown => {
                n.changed.extend(d);
                if n.changed.len() > NOTED_MAX {
                    n.changed.clear();
                    n.unknown = true;
                }
            }
            _ => {
                n.changed.clear();
                n.unknown = true;
            }
        }
    }

    /// What changed between a list in pages on the disk and `by_number` (rising numbers): the
    /// pages read only where the new list differs from them. `None`: it can't be told (a head
    /// with no first numbers, a page that can't be read, too much changed).
    fn diff_pages(
        &self,
        (account, folder, dir): (&str, &str, &Path),
        old: &StateFile,
        by_number: &[&Summary],
        pages: Option<(&[u32], &[u64])>,
    ) -> Option<Vec<(u32, Option<Summary>)>> {
        if old.firsts.len() != old.pages.len() {
            return None;
        }
        // The flags marked since the pages were written: the list on the disk has those.
        let marks: HashMap<u32, Flags> = self.read_json::<Vec<(u32, Flags)>>(&dir.join(MARKS)).ok()?.into_iter().collect();
        let last = old.pages.len().saturating_sub(1);
        let mut out = Vec::new();
        for (i, &hash) in old.pages.iter().enumerate() {
            // The numbers this page holds: from its first (any below the first page's are new
            // in it) to the next page's.
            let lo = if i == 0 { 0 } else { old.firsts.get(i).copied().unwrap_or(0) };
            let start = by_number.partition_point(|m| m.key.uid < lo);
            let end = match old.firsts.get(i + 1) {
                Some(&hi) => by_number.partition_point(|m| m.key.uid < hi),
                None => by_number.len(),
            };
            let there = by_number.get(start..end).unwrap_or_default();
            let held = if i < last { PAGE_MESSAGES } else { old.count.saturating_sub(PAGE_MESSAGES.saturating_mul(last)) };
            // The same messages as the page, when as many and the same text.
            let same = there.len() == held && {
                let written = pages.filter(|_| start % PAGE_MESSAGES == 0 && (there.len() == PAGE_MESSAGES || end == by_number.len()));
                match written.and_then(|(_, hashes)| hashes.get(start / PAGE_MESSAGES)) {
                    Some(&h) => h == hash,
                    None => serde_json::to_vec(there).is_ok_and(|b| page_hash(&b) == hash),
                }
            };
            if same {
                // Marked since the page was written, and not so now.
                for m in there {
                    if marks.get(&m.key.uid).is_some_and(|f| *f != m.flags) {
                        out.push((m.key.uid, Some(Summary::clone(m))));
                    }
                }
            } else {
                let page: Vec<Summary> = self.read_json(&dir.join(format!("state-{i}.json"))).ok()?;
                let page = page.into_iter().map(|mut m| {
                    m.key = MessageKey { account: account.to_string(), folder: folder.to_string(), validity: old.validity, uid: m.key.uid };
                    if let Some(f) = marks.get(&m.key.uid) {
                        m.flags.clone_from(f);
                    }
                    m
                });
                out.extend(diff(page, there)?);
            }
            if out.len() > NOTED_MAX {
                return None;
            }
        }
        Some(out)
    }
}

/// What changed from `old` to `new`: each message come or changed, and each gone (`None`).
/// `None` when more than [`NOTED_MAX`] did.
fn diff(old: impl Iterator<Item = Summary>, new: &[&Summary]) -> Option<Vec<(u32, Option<Summary>)>> {
    let mut old: HashMap<u32, Summary> = old.map(|m| (m.key.uid, m)).collect();
    let mut out = Vec::new();
    for m in new {
        match old.remove(&m.key.uid) {
            Some(o) if o == **m => {}
            _ => out.push((m.key.uid, Some(Summary::clone(m)))),
        }
        if out.len() > NOTED_MAX {
            return None;
        }
    }
    out.extend(old.into_keys().map(|uid| (uid, None)));
    (out.len() <= NOTED_MAX).then_some(out)
}
