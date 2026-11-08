// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The session: accounts' folder lists and what's kept of each folder; with them, making and
//! changing folders, a mailbox's settings on the server, other mailboxes opened in an account,
//! and the address book built from the mail kept.

use super::*;

mod contacts;
pub(crate) use contacts::Contacts;
mod archive;
pub(crate) use archive::Leaving;
mod echo;
pub(crate) use echo::Echoes;
mod directory;
pub(crate) use directory::{Directory, bare_name, directory_key};
pub use directory::{DirectoryAnswer, directory_people, fill_name};
mod folder_edits;
mod follow_up;
pub(crate) use follow_up::MAX_FOLLOW_UP;
mod mailbox_settings;
mod nav;
pub(crate) use nav::ListNav;
mod patch;
pub(super) mod people;
mod rows;
pub use mailbox_settings::{CompactPhase, Compacted, FolderSizes, Sizes};
mod recovering;
pub(crate) use people::People;
pub use people::{Expansion, FIELD_NAMES, FieldId, INDEX_MAX, MAX_SHOWN_MEMBERS, MAX_SUGGESTIONS, MAX_TOTAL, Source, Suggestion};
pub use recovering::{RecoverBlock, Recovered, Recovering};
mod envelope;
mod shared_mailboxes;
mod synced;
mod trust;
pub(crate) use envelope::Envelope;

impl Session {
    pub fn account(&self, id: &str) -> Option<&AccountState> {
        self.accounts.iter().find(|a| a.config.id == id)
    }

    pub(super) fn account_mut(&mut self, id: &str) -> Option<&mut AccountState> {
        self.accounts.iter_mut().find(|a| a.config.id == id)
    }

    pub fn folder(&self, account: &str, path: &str) -> Option<&Folder> {
        self.account(account).and_then(|a| a.folders.iter().find(|f| f.path == path))
    }

    /// What opens at the start: the first account's Inbox, as Outlook opens on its first mailbox
    /// (All Inboxes is a click away in Favorites; the accounts can be put in any order).
    pub(super) fn default_view(&self) -> View {
        match self.accounts.first() {
            Some(a) => {
                let folder = a.folder(FolderRole::Inbox).map(|f| f.path.clone()).unwrap_or_else(|| "INBOX".to_string());
                View::Folder { account: a.config.id.clone(), folder }
            }
            None => View::Empty,
        }
    }

    /// The account the view belongs to (or the first one, for unified views).
    pub fn current_account(&self) -> Option<&AccountState> {
        match &self.view {
            View::Folder { account, .. } => self.account(account),
            _ => self.accounts.first(),
        }
    }

    /// The role of the folder on screen.
    pub fn view_role(&self) -> Option<FolderRole> {
        match &self.view {
            View::Folder { account, folder } => self.folder(account, folder).map(|f| f.role),
            View::Unified { role } => Some(*role),
            View::Empty | View::Outbox => None,
        }
    }

    /// The view's title for the list header.
    pub fn view_title(&self) -> String {
        if let Some((q, _)) = &self.search {
            return format!("Results for “{q}”");
        }
        match &self.view {
            View::Folder { account, folder } => {
                self.folder(account, folder).map(Folder::display_name).unwrap_or_else(|| quatrelle_model::utf7::decode(folder))
            }
            View::Unified { role } => match role {
                FolderRole::Inbox => "All Inboxes".to_string(),
                r => format!("All {}", r.label().unwrap_or("folders")),
            },
            View::Outbox => "Outbox".to_string(),
            View::Empty => "Quatrelle".to_string(),
        }
    }

    /// The cached state of a folder, loaded from disk on first use; empty while it can't be read.
    pub fn folder_state(&mut self, account: &str, folder: &str) -> &FolderState<Arc<Summary>> {
        self.folder_state_as(account, folder, false)
    }

    /// [`Session::folder_state`] for a look through every folder (a search, its index): a list
    /// read for it from the disk is the first to go when the cache is full, not the lists kept
    /// before. Put last like any other, the lists a search of a mailbox bigger than the cache
    /// read went in turn, each pushing out the one the next search would want first: every
    /// search of everything read every folder's list again, a list or two a call.
    pub(crate) fn scan_state(&mut self, account: &str, folder: &str) -> &FolderState<Arc<Summary>> {
        self.folder_state_as(account, folder, true)
    }

    fn folder_state_as(&mut self, account: &str, folder: &str, scan: bool) -> &FolderState<Arc<Summary>> {
        if let Err(e) = self.load_cached_as(account, folder, scan) {
            log::warn!("a folder's list couldn't be read: {e}");
        }
        static EMPTY: std::sync::OnceLock<FolderState<Arc<Summary>>> = std::sync::OnceLock::new();
        self.cache.get(&(account.to_string(), folder.to_string())).unwrap_or_else(|| EMPTY.get_or_init(FolderState::default))
    }

    /// Have a folder's list in the cache. One that can't be read is left out, not kept as empty:
    /// a flag changed in it then saved the empty list over the folder's mail.
    pub(super) fn load_cached(&mut self, account: &str, folder: &str) -> Result<(), quatrelle_store::StoreError> {
        self.load_cached_as(account, folder, false)
    }

    /// [`Session::load_cached`]; `scan`: read for a look through every folder, a list read from
    /// the disk goes first in the order the cache lets lists go (see [`Session::scan_state`]).
    fn load_cached_as(&mut self, account: &str, folder: &str, scan: bool) -> Result<(), quatrelle_store::StoreError> {
        let k = (account.to_string(), folder.to_string());
        if !self.cache.contains_key(&k) {
            // A local account's marks still being written elsewhere are in it first.
            if self.account(account).is_none_or(AccountState::is_local) {
                self.finish_local_marks();
            }
            let st = match &self.store {
                Some(s) => s.load_folder(account, folder)?.shared(),
                None => FolderState::default(),
            };
            let (lists, messages, pages) = self.lists_read;
            self.lists_read = (lists.wrapping_add(1), messages.wrapping_add(st.messages.len() as u64), pages);
            self.cache.insert(k.clone(), st);
            self.cache_order.retain(|o| *o != k);
            if scan {
                self.cache_order.insert(0, k.clone());
            } else {
                self.cache_order.push(k.clone());
            }
            self.trim_cache(&k);
        } else if !scan && self.cache_order.last() != Some(&k) {
            // (One a look through every folder finds stays where it is: moved last, the lists it
            // hadn't come to yet went first.)
            self.cache_order.retain(|o| *o != k);
            self.cache_order.push(k);
        }
        Ok(())
    }

    /// How many folders' lists were read from the cache on disk (not found in memory), for the
    /// tests and benches of what a look through the mailbox costs.
    pub fn lists_read(&self) -> u64 {
        self.lists_read.0
    }

    /// How many pages of folders' lists were read alone (a search's candidates looked at in the
    /// page they're in, their folder not in memory).
    pub fn pages_read(&self) -> u64 {
        self.lists_read.2
    }

    /// A page of `n` messages read alone.
    pub(super) fn note_page_read(&mut self, n: usize) {
        let (lists, messages, pages) = self.lists_read;
        self.lists_read = (lists, messages.wrapping_add(n as u64), pages.wrapping_add(1));
    }

    /// A message as its folder's list has it: in memory, or read from the page of the list it's
    /// in (not the whole list read for it, nor kept), its folder not in memory.
    pub(super) fn summary_where(&mut self, key: &MessageKey) -> Option<Arc<Summary>> {
        let local = self.account(&key.account).is_none_or(AccountState::is_local);
        if local || self.cache.contains_key(&(key.account.clone(), key.folder.clone())) {
            return self.folder_state(&key.account, &key.folder).messages.iter().find(|m| m.key.uid == key.uid).cloned();
        }
        let page = self.store.as_ref()?.load_page_of(&key.account, &key.folder, key.uid).ok()?;
        self.note_page_read(page.len());
        page.into_iter().find(|m| m.key.uid == key.uid).map(Arc::new)
    }

    /// How many messages the lists [`Session::lists_read`] counts held, and the pages
    /// [`Session::pages_read`] counts: what was read again.
    pub fn messages_read(&self) -> u64 {
        self.lists_read.1
    }

    /// The folders whose lists are in memory, the least recently used first (what a look
    /// through the mailbox has read so far, for the tests of what one costs).
    pub fn cached_folders(&self) -> Vec<(String, String)> {
        self.cache_order.clone()
    }

    /// Keep the cached lists under `cache_limit` summaries: the least recently used folders
    /// go first (read again when wanted), never the ones on screen nor `keep`. Looking through a
    /// big mailbox (the address book, a search of everything) no longer leaves all of it in memory.
    pub(super) fn trim_cache(&mut self, keep: &(String, String)) {
        let mut total: usize = self.cache.values().map(|st| st.messages.len()).sum();
        if total <= self.cache_limit {
            return;
        }
        let shown = self.view_folders();
        let mut i = 0;
        while total > self.cache_limit && i < self.cache_order.len() {
            let k = self.cache_order.get(i).cloned().unwrap_or_default();
            if k == *keep || shown.contains(&k) {
                i += 1;
                continue;
            }
            if let Some(st) = self.cache.remove(&k) {
                total = total.saturating_sub(st.messages.len());
            }
            self.cache_order.remove(i);
        }
    }

    /// Change how many summaries the cache keeps, letting go of what's past it now.
    #[cfg(test)]
    pub(crate) fn set_cache_limit(&mut self, n: usize) {
        self.cache_limit = n;
        self.trim_cache(&(String::new(), String::new()));
    }

    pub(super) fn reload_folder(&mut self, account: &str, folder: &str) {
        let k = (account.to_string(), folder.to_string());
        self.cache.remove(&k);
        // The search index adds what came in, if it read the folder already, and its answer looks
        // again at what it found there. (Dropping the index, as this once did, made the search of
        // a busy mailbox start over at each change a sync told of: it never ended.)
        for ix in self.index.iter_mut().chain(self.summary_index.iter_mut()) {
            ix.changed(account, folder, true, None);
        }
        // The address book reads the folder's new mail (a slice at a time, by the window).
        self.contacts_folder_changed(account, folder);
    }

    /// Remember what an Inbox holds; unread mail above the last high-water mark is new. The first
    /// look at a folder only sets the mark, so a first sync doesn't announce the backlog.
    pub(super) fn note_new_mail(&mut self, account: &str, folder: &str) {
        let role = self.folder(account, folder).map(|f| f.role);
        if !matches!(role, Some(FolderRole::Inbox | FolderRole::Junk)) {
            return;
        }
        let st = self.folder_state(account, folder);
        let (validity, max) = (st.validity, st.messages.iter().map(|m| m.key.uid).max().unwrap_or(0));
        let k = (account.to_string(), folder.to_string());
        let arrived: Vec<Summary> = match self.seen_uid.get(&k) {
            Some(&seen) => self
                .folder_state(account, folder)
                .messages
                .iter()
                .filter(|m| m.key.uid > seen && m.key.validity == validity)
                .map(|m| Summary::clone(m))
                .collect(),
            None => Vec::new(),
        };
        self.seen_uid.insert(k, max);
        // Junk from safe senders goes back to the Inbox (and is seen arriving there).
        if role == Some(FolderRole::Junk) {
            self.rescue_arrived(&arrived);
            return;
        }
        // Blocked senders' mail goes to Junk, replies in ignored conversations to Deleted Items,
        // then rules; what they file away or mark read isn't announced.
        let mut quiet = self.block_arrived(&arrived);
        let rest: Vec<Summary> = arrived.iter().filter(|m| !quiet.contains(&m.key)).cloned().collect();
        quiet.extend(self.ignore_arrived(&rest));
        let rest: Vec<Summary> = arrived.iter().filter(|m| !quiet.contains(&m.key)).cloned().collect();
        // Mail older than a day (a resync after a long time away) isn't looked at by the scam
        // check either; one that a rule files away is (a rule doesn't make mail safe).
        let cutoff = self.clock.now - 86_400;
        let recent: Vec<Summary> = rest.iter().filter(|m| m.received_at() >= cutoff).cloned().collect();
        self.scam_arrived(account, &recent);
        quiet.extend(self.run_rules(&rest));
        // Mail older than a day (a resync after a long time away) isn't "new" any more.
        let cutoff = self.clock.now - 86_400;
        let fresh: Vec<Summary> = arrived.into_iter().filter(|m| !m.flags.seen && m.received_at() >= cutoff && !quiet.contains(&m.key)).collect();
        self.add_to_envelope(&fresh);
        if self.prefs.notify {
            self.new_mail.extend(fresh.into_iter().rev());
            // A burst (a first sync, a long time offline) is summed up by the UI; keep it bounded.
            let excess = self.new_mail.len().saturating_sub(50);
            self.new_mail.drain(..excess);
        }
    }

    /// Junk's high-water mark, from what's kept, as soon as the folder is known: what the server
    /// files there after is seen arriving (a safe sender's goes back to the Inbox), even when the
    /// folder was empty at the first sync and so never changed until then.
    pub(super) fn mark_junk(&mut self, account: &str) {
        let Some(junk) = self.account(account).and_then(|a| a.folder(FolderRole::Junk)).map(|f| f.path.clone()) else { return };
        if !self.seen_uid.contains_key(&(account.to_string(), junk.clone())) {
            self.note_new_mail(account, &junk);
        }
    }

    /// The Inbox's high-water mark, from what's kept, once the account's mail has been synced
    /// in full: an Inbox empty at the first sync never changed, so it had none, and its first
    /// message was taken for the backlog (not announced, no rules, no invitation taken in).
    pub(super) fn mark_inbox(&mut self, account: &str) {
        let Some(inbox) = self.account(account).and_then(|a| a.folder(FolderRole::Inbox)).map(|f| f.path.clone()) else { return };
        if !self.seen_uid.contains_key(&(account.to_string(), inbox.clone())) {
            self.note_new_mail(account, &inbox);
        }
    }

    /// New mail to announce since the last call.
    pub fn take_new_mail(&mut self) -> Vec<Summary> {
        std::mem::take(&mut self.new_mail)
    }

    /// Unread mail in every Inbox (for the window title and badges).
    pub fn inbox_unread(&self) -> u32 {
        self.accounts.iter().filter_map(|a| a.folder(FolderRole::Inbox)).map(|f| f.unread).sum()
    }

    /// Whether anything was written in the message being written (more than its signature):
    /// closing it then asks whether to keep it.
    pub fn compose_written(&self) -> bool {
        self.compose.as_ref().is_some_and(|c| self.written(c))
    }

    /// Whether anything was written in `c` (beyond its signature).
    pub fn written(&self, c: &Compose) -> bool {
        let signature = self.signature(&c.account).trim().to_string();
        let body = c.body.trim();
        !(c.to.trim().is_empty()
            && c.cc.trim().is_empty()
            && c.bcc.trim().is_empty()
            && c.subject.trim().is_empty()
            && c.attachments.is_empty()
            && (body.is_empty() || body.trim_start_matches("--").trim() == signature))
    }

    /// The folders the view draws from.
    pub(super) fn view_folders(&self) -> Vec<(String, String)> {
        match &self.view {
            View::Folder { account, folder } => vec![(account.clone(), folder.clone())],
            View::Unified { role } => self.accounts.iter().filter_map(|a| a.folder(*role).map(|f| (a.config.id.clone(), f.path.clone()))).collect(),
            View::Empty | View::Outbox => Vec::new(),
        }
    }

    /// Rebuild the list from the view, search and filter, keeping the selection where possible.
    pub fn refresh_list(&mut self) {
        let mut list: Vec<Arc<Summary>> = if let Some((q, scope)) = self.search.clone() {
            let mut found = self.search_results(&q, scope);
            // What the servers found: its words may be in mail not downloaded here.
            let hits: Vec<MessageKey> = self.server_search.iter().filter(|ss| ss.query == q).flat_map(|ss| ss.found.iter().cloned()).collect();
            let have: HashSet<MessageKey> = found.iter().map(|m| m.key.clone()).collect();
            // The servers can't tell what's waiting for a reply: checked here.
            let awaiting = super::searching::awaiting_wanted(&q).map(|on| (on, self.awaiting_now()));
            let hits = hits.into_iter().filter(|k| awaiting.as_ref().is_none_or(|(on, set)| set.contains(k) == *on));
            let dated = self.search_query(&q);
            for key in hits.filter(|k| !have.contains(k)) {
                if let Some(m) = self.summary_where(&key)
                    && dated.dates_hold(&m)
                {
                    found.push(m);
                }
            }
            // A Search Folder leaves out Deleted Items and Junk, as Outlook's.
            if self.open_search_folder_name().is_some() {
                found.retain(|m| self.folder(&m.key.account, &m.key.folder).is_none_or(|f| !matches!(f.role, FolderRole::Trash | FolderRole::Junk)));
            }
            sort_newest_first(&mut found);
            found
        } else if self.view == View::Outbox {
            self.outbox_summaries().into_iter().map(Arc::new).collect()
        } else {
            let mut v = Vec::new();
            for (a, f) in self.view_folders() {
                v.extend(self.folder_state(&a, &f).messages.iter().filter(|m| !m.flags.deleted).cloned());
            }
            v
        };
        // Mail on its way to or from the Archives account is out of the list meanwhile.
        if !self.leaving.is_empty() {
            list.retain(|m| !self.leaving.contains_key(&m.key));
        }
        // The server's own findings, after the local ones (Outlook merges them so): a message
        // whose body is here shows once, from the index.
        if let Some((q, _)) = &self.search
            && let Some((found_for, found)) = &self.found
            && found_for == q
        {
            // The servers aren't asked the dates: what they found outside them isn't listed.
            let dated = self.search_query(q);
            for m in found.iter().filter(|m| dated.dates_hold(m)) {
                let known = list.iter().any(|x| x.key == m.key || (!x.message_id.is_empty() && x.message_id == m.message_id));
                if !known {
                    list.push(Arc::new(m.clone()));
                }
            }
        }
        // Older mail read on the server, after the folder's own (Cached Exchange Mode).
        if let (View::Folder { account, folder }, Some((oa, of, older, _))) = (&self.view, &self.older)
            && oa == account
            && of == folder
        {
            for m in older {
                if !list.iter().any(|x| x.key == m.key) {
                    list.push(Arc::new(m.clone()));
                }
            }
        }
        let filter = self.filter;
        list.retain(|m| self.filter_keeps(filter, m) || self.filter_keep.as_ref() == Some(&m.key));
        if self.focus_split() {
            // Told apart by the address book: the one built, meanwhile (a build only starts
            // here; the window takes it on, a slice per frame, and the list follows when done).
            self.contacts_step(0);
            let other = self.focus_other;
            list.retain(|m| self.is_focused(m) != other);
        }
        arrange_list(&mut list, self.prefs.arrange, self.prefs.reverse_sort);
        self.take_list(list);
    }

    /// Show `list`, as filtered and arranged: its conversations, its index and its rows built
    /// with it, and the selection kept where it's still in it.
    pub(super) fn take_list(&mut self, list: Vec<Arc<Summary>>) {
        // Conversations are worked out once here when the list shows them (arranged by date);
        // arranged otherwise, the reading pane still shows the open message's, worked out once
        // with the first message opened (`thread_at`) rather than with every rebuild.
        let conversations = self.prefs.conversations;
        let shown = conversations && self.prefs.arrange == Arrange::Date;
        let all = if shown {
            self.nav.threaded();
            threads(&list)
        } else {
            Vec::new()
        };
        self.nav.build(&list, &all, conversations, !shown);
        self.threads = all;
        // What's still in the list, by the index just built (a set of every key made for it was
        // a tenth of what a sync's change to a big folder cost).
        let nav = &self.nav;
        let alive = |k: &MessageKey| nav.holds(&list, k);
        self.selection.retain(|k| alive(k));
        if self.focus.as_ref().is_some_and(|k| !alive(k)) {
            self.focus = None;
        }
        if self.anchor.as_ref().is_some_and(|k| !alive(k)) {
            self.anchor = None;
        }
        if let Some(r) = &self.reading
            && !alive(&r.key)
            && self.search.is_none()
        {
            self.reading = None;
        }
        self.list = list;
        self.rebuild_rows();
        self.awaiting_memo = None;
        self.note_drafts();
        self.revision = self.revision.wrapping_add(1);
    }

    /// Whether a message passes a filter.
    pub(super) fn filter_keeps(&self, filter: ListFilter, m: &Summary) -> bool {
        match filter {
            ListFilter::All => true,
            ListFilter::Unread => !m.flags.seen,
            ListFilter::Flagged => m.flags.flagged,
            ListFilter::Attachments => m.has_attachments,
            ListFilter::Categorized => self.prefs.categories.iter().any(|c| c.marks(&m.flags)),
            ListFilter::Important => m.importance > 0,
            ListFilter::ToMe => self.account(&m.key.account).is_some_and(|a| {
                let me = a.config.email.trim();
                !me.is_empty() && m.to.iter().chain(&m.cc).any(|r| r.email.trim().eq_ignore_ascii_case(me))
            }),
        }
    }

    /// Whether the message list is a table now (see [`ListLayout`]).
    pub fn list_as_table(&self) -> bool {
        match self.prefs.list_layout {
            ListLayout::Table => true,
            ListLayout::Cards => false,
            ListLayout::Auto => self.prefs.reading_pane != ReadingPane::Right,
        }
    }

    /// The open message's conversation again (conversations turned on or off).
    pub(crate) fn rethread_reading(&mut self) {
        let Some(key) = self.reading.as_ref().map(|r| r.key.clone()) else { return };
        let thread = self.thread_of(&key);
        if let Some(r) = self.reading.as_mut() {
            r.thread = thread;
        }
    }
}
