// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The session: a folder changed on the disk. Told what a sync changed in it
//! ([`Event::FolderSynced`]), the folder in memory and the list on screen take that as it is;
//! told only that it changed ([`Event::FolderChanged`]: a first sync, a folder numbered anew),
//! the folder is read again. Every sync that brought a message did that: the folder let go,
//! read again from the disk whole and the list built, 110 to 170 ms on the window's thread for a
//! folder of 50,000 messages on screen.

use std::cmp::Ordering;

use quatrelle_model::ListChange;

use super::*;

impl Session {
    /// A folder's list changed on the disk in a way that isn't told: read again, with what
    /// depends on it.
    pub(crate) fn folder_changed(&mut self, account: &str, folder: &str) {
        if self.showing_more.as_ref().is_some_and(|(a, f)| a == account && f == folder) {
            self.showing_more = None;
        }
        self.reload_folder(account, folder);
        self.after_folder_change(account, folder, None);
    }

    /// What a sync changed in a folder's list: the folder in memory takes it, and the list on
    /// screen, where it shows it. Read again when what's in memory, with the change, isn't the
    /// list on the disk (its numbers, how many), as when it isn't told.
    pub(crate) fn folder_synced(&mut self, account: &str, folder: &str, change: ListChange) {
        let ListChange { validity, upserts, removed, count, print } = change;
        let upserts: Vec<Arc<Summary>> = upserts
            .into_iter()
            .map(|mut m| {
                m.key = MessageKey { account: account.to_string(), folder: folder.to_string(), validity, uid: m.key.uid };
                // A flag or category sent from here and not yet on the server's side of the
                // disk stays as sent: the sync may have read the message before it.
                self.echoes.redo(&mut m);
                Arc::new(m)
            })
            .collect();
        let removed: HashSet<u32> = removed.into_iter().collect();
        let k = (account.to_string(), folder.to_string());
        let held = match self.cache.get_mut(&k) {
            // Not in memory: read with the change when it's wanted.
            None => true,
            Some(st) if st.validity != validity && !st.messages.is_empty() => false,
            Some(st) => {
                st.validity = validity;
                take_change(&mut st.messages, &upserts, &removed);
                st.messages.len() == count && ListChange::print(st.messages.iter().map(|m| m.key.uid)) == print
            }
        };
        if !held {
            log::info!("{account}: {folder}: what's in memory isn't the list with the sync's change: read again");
            return self.folder_changed(account, folder);
        }
        if self.showing_more.as_ref().is_some_and(|(a, f)| a == account && f == folder) {
            self.showing_more = None;
        }
        // The search indexes look again at these messages alone (the folder's other words stand).
        let keys: Vec<MessageKey> = upserts
            .iter()
            .map(|m| m.key.clone())
            .chain(removed.iter().map(|&uid| MessageKey { account: account.to_string(), folder: folder.to_string(), validity, uid }))
            .collect();
        for ix in self.index.iter_mut().chain(self.summary_index.iter_mut()) {
            ix.changed(account, folder, true, Some(&keys));
        }
        self.contacts_folder_changed(account, folder);
        self.after_folder_change(account, folder, Some((validity, &upserts, &removed)));
    }

    /// What follows a folder's list changing: new mail noted, Drafts' marks, the list on screen
    /// (taking `change` as it is when there's one, built again otherwise), the conversation of
    /// the message read.
    fn after_folder_change(&mut self, account: &str, folder: &str, change: Option<(u32, &[Arc<Summary>], &HashSet<u32>)>) {
        self.note_new_mail(account, folder);
        // A reply saved in Drafts (or gone from there): the list's marks follow.
        if self.folder(account, folder).is_some_and(|f| f.role == FolderRole::Drafts) {
            self.note_drafts();
        }
        let visible = self.search.is_some() || self.view_folders().iter().any(|(a, f)| a == account && f == folder);
        if visible && !change.is_some_and(|(validity, upserts, removed)| self.patch_synced(account, folder, validity, upserts, removed)) {
            self.refresh_list();
        }
        if let Some(r) = &self.reading
            && r.key.account == account
            && r.key.folder == folder
        {
            let key = r.key.clone();
            let thread = self.thread_of(&key);
            if let Some(r) = self.reading.as_mut() {
                r.thread = thread;
            }
        }
    }

    /// Take a sync's change to a folder shown into the list as it stands; false when it must be
    /// built again ([`Session::refresh_list`]), the same list either way. Messages whose place,
    /// row and filtering don't change (their read mark aside) take their entry; otherwise those
    /// changed or gone leave the list and those changed or come go in at their place, a merge
    /// rather than a sort, and its conversations, index and rows are made again from it.
    fn patch_synced(&mut self, account: &str, folder: &str, validity: u32, upserts: &[Arc<Summary>], removed: &HashSet<u32>) -> bool {
        if self.search.is_some() || self.view == View::Outbox || !self.nav_fresh() {
            return false;
        }
        let key = |uid: u32| MessageKey { account: account.to_string(), folder: folder.to_string(), validity, uid };
        // Older mail read on the server shows after the folder's own: a change to one of those
        // is the list's to work out.
        if let (View::Folder { account: va, folder: vf }, Some((oa, of, older, _))) = (&self.view, &self.older)
            && va == oa
            && vf == of
            && older.iter().any(|m| removed.contains(&m.key.uid) || upserts.iter().any(|u| u.key == m.key))
        {
            return false;
        }
        let split = self.focus_split();
        if split {
            self.contacts_step(0);
        }
        let (filter, other) = (self.filter, self.focus_other);
        let shows = |s: &Session, m: &Summary| {
            !m.flags.deleted && (s.filter_keeps(filter, m) || s.filter_keep.as_ref() == Some(&m.key)) && (!split || s.is_focused(m) != other)
        };
        // In place: nothing gone from the list, nothing new in it, each changed one where it was.
        let mut places: Vec<(usize, &Arc<Summary>)> = Vec::with_capacity(upserts.len());
        let in_place = removed.iter().all(|&uid| self.index_of(&key(uid)).is_none())
            && upserts.iter().all(|m| match self.index_of(&m.key) {
                Some(i) if self.list.get(i).is_some_and(|was| same_place(was, m)) && shows(self, m) => {
                    places.push((i, m));
                    true
                }
                Some(_) => false,
                None => !shows(self, m),
            });
        if in_place {
            let mut recount = Vec::new();
            for (i, m) in places {
                if let Some(slot) = self.list.get_mut(i) {
                    if slot.flags.seen != m.flags.seen {
                        recount.push(i);
                    }
                    *slot = m.clone();
                }
            }
            for i in recount {
                if !self.patch_unread(i) {
                    return false;
                }
            }
            self.revision = self.revision.wrapping_add(1);
            return true;
        }
        let touched: HashSet<u32> = removed.iter().copied().chain(upserts.iter().map(|m| m.key.uid)).collect();
        let mut list = std::mem::take(&mut self.list);
        list.retain(|m| !(m.key.account == account && m.key.folder == folder && touched.contains(&m.key.uid)));
        let order = arranged(self.prefs.arrange, self.prefs.reverse_sort);
        let mut come: Vec<Arc<Summary>> = upserts.iter().filter(|m| shows(self, m)).cloned().collect();
        come.sort_by(|a, b| order(a, b));
        // The two in order, one pass.
        let mut merged = Vec::with_capacity(list.len().saturating_add(come.len()));
        let mut come = come.into_iter().peekable();
        for m in list {
            while let Some(c) = come.next_if(|c| order(c, &m) == Ordering::Less) {
                merged.push(c);
            }
            merged.push(m);
        }
        merged.extend(come);
        self.take_list(merged);
        true
    }
}

/// `upserts` and `removed` taken into a folder's messages: one changed takes its place, one gone
/// goes, the new ones go in where the newest first has them (merged, not inserted one by one).
fn take_change(messages: &mut Vec<Arc<Summary>>, upserts: &[Arc<Summary>], removed: &HashSet<u32>) {
    let mut by_uid: HashMap<u32, &Arc<Summary>> = upserts.iter().map(|m| (m.key.uid, m)).collect();
    messages.retain_mut(|m| {
        if let Some(now) = by_uid.remove(&m.key.uid) {
            *m = now.clone();
            return true;
        }
        !removed.contains(&m.key.uid)
    });
    if by_uid.is_empty() {
        return;
    }
    let newest = |a: &Arc<Summary>, b: &Arc<Summary>| b.date.cmp(&a.date).then_with(|| b.key.cmp(&a.key));
    let mut come: Vec<Arc<Summary>> = by_uid.into_values().cloned().collect();
    come.sort_by(newest);
    let mut merged = Vec::with_capacity(messages.len().saturating_add(come.len()));
    let mut come = come.into_iter().peekable();
    for m in std::mem::take(messages) {
        while let Some(c) = come.next_if(|c| newest(c, &m) == Ordering::Less) {
            merged.push(c);
        }
        merged.push(m);
    }
    merged.extend(come);
    *messages = merged;
}

/// Whether a message changed only in what neither its place in the list, its row nor its
/// conversation depends on: its read mark (its row's count follows), replied, forwarded, its
/// preview, what the server numbers its changes by.
fn same_place(was: &Summary, now: &Summary) -> bool {
    let mut w = was.clone();
    w.flags.seen = now.flags.seen;
    w.flags.answered = now.flags.answered;
    w.flags.forwarded = now.flags.forwarded;
    w.preview.clone_from(&now.preview);
    w.modseq = now.modseq;
    w == *now
}
