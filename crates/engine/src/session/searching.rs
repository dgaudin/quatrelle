//! The session: search, here and on the servers.

use super::*;

mod answer;
mod indexing;
use answer::Answer;
pub(crate) use indexing::SearchIndex;

/// Sent at least this long ago with nothing back, a message is waiting for a reply
/// (`is:awaiting`); sent longer ago than [`AWAITING_WITHIN_DAYS`], it no longer is.
pub const AWAITING_AFTER_DAYS: i64 = 2;
pub const AWAITING_WITHIN_DAYS: i64 = 30;

impl Session {
    /// The folders a search looks in: the one on view, it and the folders inside it, its
    /// account's (every account's from a unified view), or every account's.
    pub(crate) fn scope_folders(&self, scope: SearchScope) -> Vec<(String, String)> {
        let every = |a: &AccountState| a.folders.iter().filter(|f| f.selectable).map(|f| (a.config.id.clone(), f.path.clone())).collect::<Vec<_>>();
        match (scope, &self.view) {
            (SearchScope::Current, _) => self.view_folders(),
            (SearchScope::Subfolders, View::Folder { account, folder }) => {
                let Some(a) = self.account(account) else { return Vec::new() };
                let inside = |p: &str| {
                    a.folders
                        .iter()
                        .find(|f| f.path == *folder)
                        .and_then(|f| f.delimiter)
                        .is_some_and(|d| p.strip_prefix(folder.as_str()).is_some_and(|rest| rest.starts_with(d)))
                };
                a.folders
                    .iter()
                    .filter(|f| f.selectable && (f.path == *folder || inside(&f.path)))
                    .map(|f| (account.clone(), f.path.clone()))
                    .collect()
            }
            (SearchScope::Subfolders, _) => self.view_folders(),
            (SearchScope::Mailbox, View::Folder { account, .. }) => self.account(account).map(every).unwrap_or_default(),
            (SearchScope::Mailbox | SearchScope::All, _) => self.accounts.iter().flat_map(every).collect(),
        }
    }

    /// The user's messages still waiting for a reply: in a Sent folder, sent between
    /// [`AWAITING_AFTER_DAYS`] and [`AWAITING_WITHIN_DAYS`] ago to someone else (not in bulk),
    /// and named by no message from anyone else, in any folder, as the one it answers
    /// (`In-Reply-To`) or follows (`References`).
    pub fn awaiting_replies(&mut self) -> HashSet<MessageKey> {
        const DAY: i64 = 86_400;
        let mine: HashSet<String> = self
            .accounts
            .iter()
            .flat_map(|a| [a.config.email.trim().to_lowercase(), a.config.username.trim().to_lowercase()])
            .filter(|e| e.contains('@'))
            .collect();
        let folders: Vec<(String, String, bool)> = self
            .accounts
            .iter()
            .flat_map(|a| a.folders.iter().filter(|f| f.selectable).map(|f| (a.config.id.clone(), f.path.clone(), f.role == FolderRole::Sent)))
            .collect();
        let (newest, oldest) = (self.clock.now.saturating_sub(AWAITING_AFTER_DAYS * DAY), self.clock.now.saturating_sub(AWAITING_WITHIN_DAYS * DAY));
        let mut answered: HashSet<String> = HashSet::new();
        let mut sent: Vec<(MessageKey, String)> = Vec::new();
        for (account, folder, is_sent) in folders {
            for m in &self.folder_state(&account, &folder).messages {
                if !mine.contains(&m.from.email.trim().to_lowercase()) {
                    answered.extend(std::iter::once(&m.in_reply_to).chain(&m.references).filter(|id| !id.is_empty()).cloned());
                } else if is_sent
                    && !m.bulk
                    && !m.message_id.is_empty()
                    && (oldest..=newest).contains(&m.date)
                    && m.to.iter().chain(&m.cc).any(|r| !mine.contains(&r.email.trim().to_lowercase()))
                {
                    sent.push((m.key.clone(), m.message_id.clone()));
                }
            }
        }
        sent.into_iter().filter(|(_, id)| !answered.contains(id)).map(|(k, _)| k).collect()
    }

    /// The messages waiting for a reply, worked out once for a search call and its list.
    pub(super) fn awaiting_now(&mut self) -> HashSet<MessageKey> {
        if let Some(memo) = &self.awaiting_memo {
            return memo.clone();
        }
        let set = self.awaiting_replies();
        self.awaiting_memo = Some(set.clone());
        set
    }

    /// The servers' search is no longer wanted (the search box cleared or changed, issue #4):
    /// the accounts still searching stop; what they found stays in the folders' lists.
    pub(crate) fn stop_server_search(&mut self) {
        if let Some(ss) = self.server_search.take()
            && ss.pending > 0
        {
            self.jobs.push(Job::StopSearch);
        }
    }

    /// The search box's recent searches, newest first: kept with the settings, or, while an
    /// account is kept online only, in memory only.
    pub fn recent_searches(&self) -> &[String] {
        if self.searches_in_memory() { &self.recent_in_memory } else { &self.prefs.recent_searches }
    }

    /// Nothing of an online-only account reaches the disk (issue #4), its searches neither.
    fn searches_in_memory(&self) -> bool {
        self.accounts.iter().any(|a| a.config.keeps_online())
    }

    /// Forget the recent searches.
    pub fn forget_recent_searches(&mut self) {
        self.prefs.recent_searches.clear();
        self.recent_in_memory.clear();
    }

    /// Keep a search among the recent ones (the search box's list): newest first, once, ten.
    pub fn remember_search(&mut self, query: &str) {
        let q: String = query.trim().chars().take(500).collect();
        if q.is_empty() {
            return;
        }
        let recent = if self.searches_in_memory() {
            // Those kept on disk before go too.
            if self.recent_in_memory.is_empty() {
                self.recent_in_memory = std::mem::take(&mut self.prefs.recent_searches);
            }
            self.prefs.recent_searches.clear();
            &mut self.recent_in_memory
        } else {
            &mut self.prefs.recent_searches
        };
        recent.retain(|r| *r != q);
        recent.insert(0, q);
        recent.truncate(RECENT_SEARCHES);
    }

    /// Search every folder or the folder open now.
    pub fn run_search(&mut self, query: &str, scope: SearchScope) -> usize {
        self.search_now(query, scope, usize::MAX, None)
    }

    /// Search, adding at most `budget` messages to the index this call (`0`: no limit). A caller
    /// that draws a frame between calls (the window) keeps the interface responsive on a big
    /// mailbox; [`Session::search_pending`] says whether more is left to index.
    pub fn run_search_budgeted(&mut self, query: &str, scope: SearchScope, budget: usize) -> usize {
        // A whole search asked for at once is a search of its own, never a Search Folder's (the
        // window's slices of one search carry it on: they keep it).
        if budget == 0 {
            self.search_folder = None;
        }
        self.search_now(query, scope, if budget == 0 { usize::MAX } else { budget }, None)
    }

    /// A search, or (`folder`) a Search Folder's: a search typed in is never one, whatever it
    /// asks.
    pub(crate) fn search_as(&mut self, query: &str, scope: SearchScope, folder: Option<String>) -> usize {
        self.search_now(query, scope, usize::MAX, folder)
    }

    pub(super) fn search_now(&mut self, query: &str, scope: SearchScope, budget: usize, folder: Option<String>) -> usize {
        let q = query.trim();
        // Carrying the same search on (a slice more of the index) leaves the list's selection,
        // focus and the open message as they are; a new query starts them over.
        let same = self.search.as_ref().is_some_and(|(t, s)| t == q && *s == scope);
        self.search = if q.is_empty() { None } else { Some((q.to_string(), scope)) };
        // Carrying a Search Folder's search on (a slice more of the index) keeps it its own; a
        // new search, or one typed in, is never one.
        if folder.is_some() || !same {
            self.search_folder = folder;
        }
        self.search_budget = budget;
        if q.is_empty() {
            self.search_pending = false;
            // The index stays (the next search is quick), not the answer: what it would have
            // to look at again piles up while no search is shown, and a later one starts its
            // answer from the index.
            if let Some(ix) = self.index.as_mut() {
                ix.answer = None;
            }
        }
        if self.server_search.as_ref().is_some_and(|ss| ss.query != q) {
            self.stop_server_search();
        }
        if !same {
            self.selection.clear();
            self.focus = None;
            self.reading = None;
            self.found = None;
        }
        // A slice more of the index; carrying the same search on, the list shows the answer
        // again only when the slice says so (a big mailbox's list sorted at every frame made
        // each slice dearer than the last).
        let show = self.search_step(q, scope, budget);
        if show || !same {
            self.refresh_list();
        }
        self.awaiting_memo = None;
        self.ask_server(q, scope);
        self.list.len()
    }

    /// Ask the server's own index about the query, once: what it finds (bodies included) joins
    /// the local answer when it arrives.
    pub(super) fn ask_server(&mut self, query: &str, scope: SearchScope) {
        if query.is_empty() || self.store.is_none() {
            return;
        }
        if self.server_asked.as_ref().is_some_and(|(q, s)| q == query && *s == scope) {
            return;
        }
        self.server_asked = Some((query.to_string(), scope));
        for (account, folder) in self.search_accounts(scope) {
            self.jobs.push(Job::Search { account, query: query.to_string(), folder });
        }
    }

    /// A page of a folder's older mail read on the server ([`Job::Older`]): added to what's
    /// shown of it after the folder's own list, held in memory only.
    pub(super) fn older_from_server(&mut self, account: &str, folder: &str, items: Vec<Summary>, more: bool, error: Option<String>) {
        if self.showing_more.as_ref().is_some_and(|(a, f)| a == account && f == folder) {
            self.showing_more = None;
        }
        if let Some(e) = error {
            self.notice_with("older mail couldn't be read on the server: {error}", &[("error", &e)], true);
            return;
        }
        if !self.older.as_ref().is_some_and(|(a, f, ..)| a == account && f == folder) {
            self.older = Some((account.to_string(), folder.to_string(), Vec::new(), true));
        }
        if let Some((_, _, list, has_more)) = self.older.as_mut() {
            for m in items {
                if !list.iter().any(|x| x.key == m.key) {
                    list.push(m);
                }
            }
            *has_more = more;
        }
        if matches!(&self.view, View::Folder { account: a, folder: f } if a == account && f == folder) {
            self.refresh_list();
        }
    }

    /// What the server found for a search: kept for the query on screen, and shown with the
    /// messages the local index found (they answer at once; the server takes a moment).
    pub(super) fn found_by_server(&mut self, account: &str, query: &str, items: Vec<Summary>, error: Option<String>) {
        let on_screen = self.search.as_ref().is_some_and(|(q, _)| q == query);
        if on_screen {
            let entry = match self.found.take() {
                Some((q, v)) if q == query => (q, v),
                _ => (query.to_string(), Vec::new()),
            };
            let mut v = entry.1;
            for m in items {
                if !v.iter().any(|x| x.key == m.key || (!x.message_id.is_empty() && x.message_id == m.message_id)) {
                    v.push(m);
                }
            }
            self.found = Some((query.to_string(), v));
            self.refresh_list();
        } else if !items.is_empty() {
            // The query changed while the server was answering: keep them anyway, so typing the
            // same search again shows them at once.
            self.found = Some((query.to_string(), items));
        }
        if let Some(e) = error
            && on_screen
            && self.list.is_empty()
        {
            self.notice_with("{account}: the server's search failed ({error})", &[("account", account), ("error", &e)], true);
        }
    }

    /// The accounts a search asks the server about, with the folder when it's the one open.
    /// Accounts whose server can be asked are the ones worth asking: Exchange's own index, or an
    /// IMAP server's `SEARCH`; the others (local, POP, JMAP) answer from this computer alone.
    pub(super) fn search_accounts(&self, scope: SearchScope) -> Vec<(String, Option<String>)> {
        let indexed = |a: &AccountState| {
            let c = &a.config;
            !a.is_local() && (c.ews.is_some() || (c.pop.is_none() && c.jmap.is_none() && c.graph.is_none()))
        };
        match scope {
            SearchScope::Mailbox => match &self.view {
                View::Folder { account, .. } => {
                    self.account(account).filter(|a| indexed(a)).map(|a| vec![(a.config.id.clone(), None)]).unwrap_or_default()
                }
                _ => self.accounts.iter().filter(|a| indexed(a)).map(|a| (a.config.id.clone(), None)).collect(),
            },
            SearchScope::Current | SearchScope::Subfolders => {
                let folders: Vec<(String, Option<String>)> = self.view_folders().into_iter().map(|(a, f)| (a, Some(f))).collect();
                if folders.is_empty() {
                    return self.accounts.iter().filter(|a| indexed(a)).map(|a| (a.config.id.clone(), None)).collect();
                }
                folders.into_iter().filter(|(a, _)| self.account(a).is_some_and(indexed)).collect()
            }
            SearchScope::All => self.accounts.iter().filter(|a| indexed(a)).map(|a| (a.config.id.clone(), None)).collect(),
        }
    }
}

/// How many searches the search box remembers.
pub const RECENT_SEARCHES: usize = 10;

/// Whether a search asks for mail waiting for a reply (`Some(true)`), or for mail that isn't
/// (`Some(false)`, `-is:awaiting`), as one of the terms that must all match.
pub(super) fn awaiting_wanted(q: &str) -> Option<bool> {
    Query::parse(q).clauses.iter().find(|c| c.term == quatrelle_model::search::Term::Awaiting).map(|c| !c.negated)
}

/// Whether a search term asks for mail waiting for a reply (on its own or among others).
fn asks_awaiting(t: &quatrelle_model::search::Term) -> bool {
    use quatrelle_model::search::Term;
    match t {
        Term::Awaiting => true,
        Term::AnyOf(any) => any.iter().any(asks_awaiting),
        _ => false,
    }
}
