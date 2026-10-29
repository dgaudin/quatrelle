//! The session: search, here and on the servers.

use super::*;

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

    /// Make sure an index covers this scope, starting one when there's none for it. An index
    /// already under way for the same scope (or a finished one) is kept as it is: the build goes
    /// on a slice at a time through [`Session::index_step`].
    pub(super) fn ensure_index(&mut self, scope: SearchScope) {
        if self.index.as_ref().is_some_and(|ix| ix.scope == scope && (scope == SearchScope::All || ix.view == self.view)) {
            return;
        }
        let scan: Vec<(String, String)> = self.scope_folders(scope);
        self.index = Some(SearchIndex {
            index: Index::new(),
            keys: Vec::new(),
            has_body: Vec::new(),
            scope,
            view: self.view.clone(),
            scan,
            folder: 0,
            pos: 0,
            done: false,
            building: None,
        });
    }

    /// Index at most `budget` more messages (`usize::MAX`: whatever is left), reading a folder in
    /// slices so a big one isn't copied whole. Whether the scan is done: a big mailbox's first
    /// search is spread over several calls, one slice each, and the window keeps drawing between
    /// them.
    pub(super) fn index_step(&mut self, budget: usize) -> bool {
        let Some(mut ix) = self.index.take() else { return true };
        if !ix.done {
            let mut added = 0usize;
            'scan: while ix.folder < ix.scan.len() {
                let (a, f) = ix.scan[ix.folder].clone();
                if ix.building.is_none() && ix.pos == 0 {
                    // A folder read by an earlier run is taken as it is (a folder that changed
                    // since is read again on its own; the others stay).
                    let before = ix.keys.len();
                    if self.kept_folder(&mut ix, &a, &f) {
                        ix.folder += 1;
                        // A folder kept on disk is read in one go: a big one takes the whole
                        // slice, so a first search after a restart doesn't load them all at once.
                        added += ix.keys.len().saturating_sub(before).max(1);
                        if added >= budget {
                            break 'scan;
                        }
                        continue;
                    }
                    ix.building = Some(FolderIndex::new(self.folder_stamp(&a, &f)));
                }
                let n = self.folder_state(&a, &f).messages.len();
                while ix.pos < n {
                    if added >= budget {
                        break 'scan;
                    }
                    let Some(m) = self.folder_state(&a, &f).messages.get(ix.pos).cloned() else { break };
                    let Some(b) = ix.building.as_mut() else { break };
                    let doc = u32::try_from(b.keys.len()).unwrap_or(u32::MAX);
                    b.index.add(doc, &summary_text(&m));
                    let words = self.words_of(&m.key);
                    if let Some(words) = &words {
                        b.index.add(doc, words);
                    }
                    b.has_body.push(words.is_some());
                    b.keys.push(m.key.clone());
                    ix.pos += 1;
                    added += 1;
                }
                // The folder is read: what it holds joins the mailbox's index, and is kept for
                // the next run.
                if let Some(b) = ix.building.take() {
                    self.keep_folder(&a, &f, &b);
                    join_folder(&mut ix, b);
                }
                ix.folder += 1;
                ix.pos = 0;
                if added >= budget {
                    break 'scan;
                }
            }
            if ix.folder >= ix.scan.len() {
                ix.done = true;
            }
        }
        let done = ix.done;
        self.index = Some(ix);
        done
    }

    /// A folder's index kept by an earlier run, when it still fits it: its messages are joined to
    /// the index, and the bodies that came in since are read (the folder itself isn't).
    pub(super) fn kept_folder(&mut self, ix: &mut SearchIndex, account: &str, folder: &str) -> bool {
        let Some(bytes) = self.store.as_ref().and_then(|s| s.load_search_index(account, folder)) else { return false };
        let Some(mut kept) = FolderIndex::from_bytes(account, folder, &bytes) else {
            // A damaged file is no use: the folder is read again, and the file written over.
            log::warn!("search index of {account}/{folder}: unreadable, reading the folder again");
            return false;
        };
        if kept.stamp != self.folder_stamp(account, folder) {
            return false;
        }
        let mut changed = false;
        for i in 0..kept.keys.len() {
            if kept.has_body.get(i) == Some(&true) || !self.store.as_ref().is_some_and(|s| s.has_body(&kept.keys[i])) {
                continue;
            }
            let Some(words) = self.words_of(&kept.keys[i]) else { continue };
            kept.index.add(u32::try_from(i).unwrap_or(u32::MAX), &words);
            if let Some(b) = kept.has_body.get_mut(i) {
                *b = true;
            }
            changed = true;
        }
        if changed {
            self.keep_folder(account, folder, &kept);
        }
        join_folder(ix, kept);
        true
    }

    /// Keep a folder's index for the next run (a cache that can't be written is no reason to
    /// fail a search: the folder is read again next time).
    pub(super) fn keep_folder(&self, account: &str, folder: &str, b: &FolderIndex) {
        if b.keys.is_empty() {
            return;
        }
        if let Some(store) = &self.store
            && let Err(e) = store.save_search_index(account, folder, &b.to_bytes())
        {
            log::warn!("search index of {account}/{folder} not kept: {e}");
        }
    }

    /// A fingerprint of a folder's messages as search sees them — their text, not their flags
    /// (marking mail read keeps the index): the one kept for the folder is used again only when it
    /// matches. It costs a pass over the summaries, far less than reading their bodies again.
    pub(super) fn folder_stamp(&mut self, account: &str, folder: &str) -> u64 {
        let st = self.folder_state(account, folder);
        let mut h = 0xcbf29ce484222325u64;
        fnv(&mut h, &st.validity.to_le_bytes());
        fnv(&mut h, &(st.messages.len() as u64).to_le_bytes());
        for m in &st.messages {
            fnv(&mut h, &m.key.uid.to_le_bytes());
            for s in [&m.subject, &m.from.name, &m.from.email, &m.preview] {
                fnv(&mut h, s.as_bytes());
                fnv(&mut h, &[0]);
            }
            for a in m.to.iter().chain(m.cc.iter()) {
                fnv(&mut h, a.email.as_bytes());
                fnv(&mut h, &[0]);
            }
        }
        h
    }

    /// A downloaded message's words for search, read once (`None`: not downloaded).
    pub(super) fn words_of(&mut self, key: &MessageKey) -> Option<Arc<str>> {
        if let Some(w) = self.body_words.get(key) {
            return Some(w.clone());
        }
        let raw = self.store.as_ref().and_then(|s| s.read_body(key))?;
        let words: Arc<str> = Arc::from(mime::search_text(&raw));
        self.keep_words(key, words.clone());
        Some(words)
    }

    /// Keep a message's body words, the oldest kept going first past [`BODY_WORDS_BYTES`] (all
    /// at once: they're cheap to read again).
    pub(super) fn keep_words(&mut self, key: &MessageKey, words: Arc<str>) {
        if self.body_words_bytes + words.len() > BODY_WORDS_BYTES {
            self.body_words.clear();
            self.body_words_bytes = 0;
        }
        self.body_words_bytes += words.len();
        if let Some(old) = self.body_words.insert(key.clone(), words) {
            self.body_words_bytes = self.body_words_bytes.saturating_sub(old.len());
        }
    }

    pub(super) fn search_results(&mut self, q: &str, scope: SearchScope) -> Vec<Arc<Summary>> {
        let mut query = Query::parse(q);
        // `category:Work` means the Work category's keyword (`$label2`), or its name's as Exchange
        // keeps it (`work`). `is:categorized` means one of the categories.
        use quatrelle_model::search::Term;
        let both = |c: &quatrelle_model::organize::Category| [c.keyword.clone(), quatrelle_model::keyword_for(&c.name)];
        for c in &mut query.clauses {
            if let Term::Keyword(k) = &c.term
                && let Some(cat) = self.category(k)
            {
                c.term = Term::Categorized(both(cat).to_vec());
            }
            if let Term::Categorized(ks) = &mut c.term
                && ks.is_empty()
            {
                *ks = self.prefs.categories.iter().flat_map(both).collect();
            }
        }
        if query.is_empty() {
            self.search_pending = false;
            return Vec::new();
        }
        let awaiting = if query.clauses.iter().any(|c| asks_awaiting(&c.term)) { self.awaiting_replies() } else { HashSet::new() };
        self.ensure_index(scope);
        let budget = self.search_budget;
        self.search_pending = !self.index_step(budget);
        let Some(ix) = self.index.take() else { return Vec::new() };
        let mut out = Vec::new();
        // Each folder's messages by number, made once: looked up one by one in the list, a search
        // finding most of a big folder was quadratic.
        let mut by_uid: HashMap<(String, String), HashMap<u32, usize>> = HashMap::new();
        let needs_body = query.needs_body();
        // The folder being read isn't part of the whole index yet; it answers too, so results
        // come as it goes rather than when the last folder is done.
        let mut candidates = ix.index.candidates(&query);
        let offset = u32::try_from(ix.keys.len()).unwrap_or(u32::MAX);
        if let Some(b) = &ix.building {
            for d in b.index.candidates(&query) {
                candidates.insert(d.saturating_add(offset));
            }
        }
        for doc in candidates {
            let (key, has_body) = if doc >= offset {
                let at = (doc - offset) as usize;
                match ix.building.as_ref().and_then(|b| Some((b.keys.get(at)?, b.has_body.get(at).copied().unwrap_or(false)))) {
                    Some((k, h)) => (k.clone(), h),
                    None => continue,
                }
            } else {
                let Some(key) = ix.keys.get(doc as usize) else { continue };
                (key.clone(), ix.has_body.get(doc as usize) == Some(&true))
            };
            let folder = (key.account.clone(), key.folder.clone());
            if !by_uid.contains_key(&folder) {
                // Where each message is in its folder's list (not a copy of every summary).
                let map = self.folder_state(&key.account, &key.folder).messages.iter().enumerate().map(|(i, m)| (m.key.uid, i)).collect();
                by_uid.insert(folder.clone(), map);
            }
            let Some(&at) = by_uid.get(&folder).and_then(|f| f.get(&key.uid)) else { continue };
            let Some(m) = self.folder_state(&key.account, &key.folder).messages.get(at).filter(|m| m.key.uid == key.uid).cloned() else { continue };
            // The body read again only when the index can't settle the query.
            let body = if needs_body && has_body { self.words_of(&key) } else { None };
            if query.matches(&Doc { summary: &m, body: body.as_deref(), indexed: true, awaiting: awaiting.contains(&m.key) }) {
                out.push(m);
            }
        }
        self.index = Some(ix);
        out
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
        self.refresh_list();
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
