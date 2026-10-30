//! The session: the search index, built a slice at a time and kept per folder between runs.

use std::ops::Range;

use super::*;

pub(crate) struct SearchIndex {
    pub(super) index: Index,
    pub(super) keys: Vec<MessageKey>,
    /// Whether each message's body words are in the index (the words themselves aren't kept: a
    /// big mailbox's would fill the memory; they're read again for the messages a search finds).
    pub(super) has_body: Vec<bool>,
    pub(super) scope: SearchScope,
    pub(super) view: View,
    /// The folders this index covers and how far the scan has got: the build goes on a slice at a
    /// time (see [`Session::index_step`]), so searching a big mailbox never freezes the window.
    pub(super) scan: Vec<(String, String)>,
    pub(super) folder: usize,
    pub(super) pos: usize,
    pub(super) done: bool,
    /// The folder being read, numbered from zero: it is kept on disk once read, so a later run
    /// takes it as it is, and it answers searches while it is being read too.
    pub(super) building: Option<FolderIndex>,
    /// Folders read (or being read) whose list changed since: what came in is added to the index
    /// ([`Session::top_up`]) before the scan goes on. A busy mailbox's sync tells of changes every
    /// few seconds; when each threw the whole index away, the scan started over and never ended.
    pub(super) stale: Vec<(String, String)>,
    /// Folders read that are no longer searched (deleted, renamed, out of the scope now): their
    /// messages stay in the index but no longer answer.
    pub(super) gone: HashSet<(String, String)>,
    /// Where each folder read is in the index: the first range is the folder as it was read, in
    /// its list's order then; a folder topped up has more.
    pub(super) ranges: HashMap<(String, String), Vec<Range<u32>>>,
    /// The search's answer, carried on as the index grows (see `answer.rs`).
    pub(super) answer: Option<Answer>,
}

impl SearchIndex {
    /// Whether the folder at `n` in the scan is read, or being read.
    fn read(&self, n: usize) -> bool {
        n < self.folder || (n == self.folder && self.building.is_some())
    }

    /// Whether a message's folder is no longer searched.
    pub(super) fn is_gone(&self, key: &MessageKey) -> bool {
        !self.gone.is_empty() && self.gone.iter().any(|(a, f)| *a == key.account && *f == key.folder)
    }

    /// Whether more is left to do: folders to read, folders to top up, or messages the answer
    /// hasn't looked at yet.
    pub(super) fn pending(&self) -> bool {
        !self.done
            || !self.stale.is_empty()
            || self.answer.as_ref().is_some_and(|a| a.checked < self.total() || !a.again.is_empty() || !a.todo.is_empty())
    }

    /// The candidates for a query numbered within `docs`: the index's, then the folder being
    /// read's after them.
    pub(super) fn candidates_in(&self, query: &Query, docs: Range<u32>) -> Vec<u32> {
        let offset = u32::try_from(self.keys.len()).unwrap_or(u32::MAX);
        let mut out: Vec<u32> = self.index.candidates_in(query, docs.start..docs.end.min(offset)).into_iter().collect();
        if let Some(b) = &self.building
            && docs.end > offset
        {
            let inside = docs.start.saturating_sub(offset)..docs.end - offset;
            out.extend(b.index.candidates_in(query, inside).into_iter().map(|d| d.saturating_add(offset)));
        }
        out
    }

    /// Where a folder's messages are in the index (the folder being read's too).
    fn docs_of(&self, k: &(String, String)) -> Vec<Range<u32>> {
        let mut out: Vec<Range<u32>> = self.ranges.get(k).cloned().unwrap_or_default();
        if self.building.is_some() && self.scan.get(self.folder) == Some(k) {
            let offset = u32::try_from(self.keys.len()).unwrap_or(u32::MAX);
            out.push(offset..self.total());
        }
        out
    }

    /// The message an index number stands for (the folder being read numbered after the index),
    /// and whether its body's words are in.
    pub(super) fn doc(&self, d: u32) -> Option<(&MessageKey, bool)> {
        let at = d as usize;
        if at < self.keys.len() {
            return Some((self.keys.get(at)?, self.has_body.get(at) == Some(&true)));
        }
        let b = self.building.as_ref()?;
        let at = at - self.keys.len();
        Some((b.keys.get(at)?, b.has_body.get(at) == Some(&true)))
    }

    /// How many messages are in: the index's, then the folder being read's.
    pub(super) fn total(&self) -> u32 {
        u32::try_from(self.keys.len() + self.building.as_ref().map_or(0, |b| b.keys.len())).unwrap_or(u32::MAX)
    }

    /// Where a message probably is in its folder's list: where it was when its folder was read
    /// (the list is looked through only when it isn't there any more).
    pub(super) fn hint(&self, d: u32, key: &MessageKey) -> usize {
        let offset = u32::try_from(self.keys.len()).unwrap_or(u32::MAX);
        if d >= offset {
            return (d - offset) as usize;
        }
        let first = self.ranges.get(&(key.account.clone(), key.folder.clone())).and_then(|r| r.first());
        first.filter(|r| r.contains(&d)).map_or(usize::MAX, |r| (d - r.start) as usize)
    }

    /// A folder's list changed: one read already is topped up (`words`: messages came or went;
    /// otherwise only their flags did), and the answer looks again at what it found there and at
    /// the messages that changed (`keys`, when known; otherwise all the folder's): one marked
    /// unread, flagged or filed in a category answers `is:unread` or `category:` now with no word
    /// of it changed.
    ///
    /// A message whose words change under the same key keeps its old words in the index until
    /// its folder is read again (the next run: its stamp no longer matches). Exchange and IMAP
    /// never change a summary's text in place (previews are worked out apart, in
    /// `Session::previews`); a draft edited and synced is the case that waits.
    pub(crate) fn changed(&mut self, account: &str, folder: &str, words: bool, keys: Option<&[MessageKey]>) {
        let k = (account.to_string(), folder.to_string());
        if words
            && let Some(n) = self.scan.iter().position(|f| *f == k)
            && self.read(n)
            && !self.gone.contains(&k)
            && !self.stale.contains(&k)
        {
            self.stale.push(k.clone());
        }
        if self.answer.is_none() {
            return;
        }
        let ranges = self.docs_of(&k);
        let again: Vec<Range<u32>> = match keys {
            None => ranges,
            Some(keys) => {
                let wanted: HashSet<(u32, u32)> =
                    keys.iter().filter(|m| m.account == account && m.folder == folder).map(|m| (m.validity, m.uid)).collect();
                if wanted.is_empty() {
                    Vec::new()
                } else {
                    ranges
                        .into_iter()
                        .flatten()
                        .filter(|&d| self.doc(d).is_some_and(|(m, _)| wanted.contains(&(m.validity, m.uid))))
                        .map(|d| d..d.saturating_add(1))
                        .collect()
                }
            }
        };
        if let Some(a) = self.answer.as_mut() {
            a.recheck.insert(k);
            a.again.extend(again);
        }
    }

    /// The folders searched are now `wanted`: the index follows without starting over. A folder
    /// new to it is read after the others; one read that isn't searched any more stays in the
    /// index but no longer answers, and answers again (topped up) if it comes back. Whether
    /// anything changed.
    fn rescope(&mut self, wanted: Vec<(String, String)>) -> bool {
        let set: HashSet<&(String, String)> = wanted.iter().collect();
        let mut changed = false;
        // The folder being read and no longer searched: what's read of it joins the index (the
        // answer has looked at it), and it counts as read.
        if self.building.is_some()
            && let Some(k) = self.scan.get(self.folder).filter(|k| !set.contains(k)).cloned()
            && let Some(b) = self.building.take()
        {
            join_folder(self, k, b);
            self.folder += 1;
            self.pos = 0;
        }
        let mut kept = Vec::with_capacity(self.scan.len());
        for (n, k) in std::mem::take(&mut self.scan).into_iter().enumerate() {
            if set.contains(&k) {
                kept.push(k);
                continue;
            }
            changed = true;
            // A folder still to read is simply left out; one read stays where it is (the scan's
            // place counts the folders before it).
            if n < self.folder {
                self.stale.retain(|s| *s != k);
                if let Some(a) = self.answer.as_mut() {
                    a.recheck.insert(k.clone());
                }
                self.gone.insert(k.clone());
                kept.push(k);
            }
        }
        self.scan = kept;
        for k in wanted {
            if self.gone.remove(&k) {
                if let Some(a) = self.answer.as_mut() {
                    a.again.extend(self.ranges.get(&k).into_iter().flatten().cloned());
                }
                self.stale.push(k);
                changed = true;
            } else if !self.scan.contains(&k) {
                self.scan.push(k);
                changed = true;
            }
        }
        self.done = self.folder >= self.scan.len();
        changed
    }
}

/// A folder's messages take their place in the mailbox's index after the ones before them.
fn join_folder(ix: &mut SearchIndex, k: (String, String), folder: FolderIndex) {
    let offset = u32::try_from(ix.keys.len()).unwrap_or(u32::MAX);
    let n = u32::try_from(folder.keys.len()).unwrap_or(u32::MAX);
    ix.index.absorb(folder.index, offset);
    ix.keys.extend(folder.keys);
    ix.has_body.extend(folder.has_body);
    ix.ranges.entry(k).or_default().push(offset..offset.saturating_add(n));
}

/// A folder's index stamp is a hash kept on disk, so it is the same from one run to the next (the
/// standard library's hasher makes no such promise).
fn fnv(h: &mut u64, bytes: &[u8]) {
    for &b in bytes {
        *h ^= u64::from(b);
        *h = h.wrapping_mul(0x100000001b3);
    }
}

impl Session {
    /// Make sure an index covers this scope, starting one when there's none for it. An index
    /// already under way for the same scope (or a finished one) is kept as it is: the build goes
    /// on a slice at a time through [`Session::index_step`].
    pub(crate) fn ensure_index(&mut self, scope: SearchScope) {
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
            stale: Vec::new(),
            gone: HashSet::new(),
            ranges: HashMap::new(),
            answer: None,
        });
    }

    /// Index at most `budget` more messages (`usize::MAX`: whatever is left), reading a folder in
    /// slices so a big one isn't copied whole. Whether the scan is done: a big mailbox's first
    /// search is spread over several calls, one slice each, and the window keeps drawing between
    /// them. Between folders, what came into the folders read already is added first (usually a
    /// few messages).
    pub(crate) fn index_step(&mut self, budget: usize) -> bool {
        let Some(mut ix) = self.index.take() else { return true };
        let mut added = 0usize;
        while added < budget {
            if ix.building.is_none()
                && ix.pos == 0
                && let Some(k) = ix.stale.first().cloned()
            {
                added += self.top_up(&mut ix, &k, budget - added);
                continue;
            }
            let Some((a, f)) = ix.scan.get(ix.folder).cloned() else { break };
            if ix.building.is_none() && ix.pos == 0 {
                // A folder read by an earlier run is taken as it is (a folder that changed
                // since is read again on its own; the others stay).
                let before = ix.keys.len();
                if self.kept_folder(&mut ix, &a, &f) {
                    ix.folder += 1;
                    // A folder kept on disk is read in one go: a big one takes the whole
                    // slice, so a first search after a restart doesn't load them all at once.
                    added += ix.keys.len().saturating_sub(before).max(1);
                    continue;
                }
                ix.building = Some(FolderIndex::new(self.folder_stamp(&a, &f)));
            }
            // A slice of the folder at once (not the folder looked up again for each message).
            let slice: Vec<Arc<Summary>> = self.folder_state(&a, &f).messages.iter().skip(ix.pos).take(budget - added).cloned().collect();
            let Some(mut b) = ix.building.take() else { break };
            for m in &slice {
                self.index_message(&mut b, m);
            }
            ix.pos += slice.len();
            added += slice.len();
            if ix.pos < self.folder_state(&a, &f).messages.len() {
                ix.building = Some(b);
                continue;
            }
            // The folder is read: what it holds joins the mailbox's index, and is kept for the
            // next run (unless it changed while it was read: a mix of before and after is read
            // again by the next run, and topped up now).
            let k = (a, f);
            if !ix.stale.contains(&k) {
                self.keep_folder(&k.0, &k.1, &b);
            }
            join_folder(&mut ix, k, b);
            ix.folder += 1;
            ix.pos = 0;
            added += 1;
        }
        ix.done = ix.folder >= ix.scan.len();
        let done = !ix.pending();
        self.index = Some(ix);
        done
    }

    /// One message into a folder's index: its summary's words and its body's, when downloaded.
    fn index_message(&mut self, b: &mut FolderIndex, m: &Summary) {
        let doc = u32::try_from(b.keys.len()).unwrap_or(u32::MAX);
        b.index.add(doc, &summary_text(m));
        let words = self.words_of(&m.key);
        if let Some(words) = &words {
            b.index.add(doc, words);
        }
        b.has_body.push(words.is_some());
        b.keys.push(m.key.clone());
    }

    /// What came into a folder read already: its messages the index hasn't got, at most
    /// `budget`, join the index (the folder isn't read again). What it cost (at least one); the
    /// folder is up to date once nothing is left.
    pub(crate) fn top_up(&mut self, ix: &mut SearchIndex, k: &(String, String), budget: usize) -> usize {
        if ix.gone.contains(k) {
            ix.stale.retain(|s| s != k);
            return 1;
        }
        let have: HashSet<(u32, u32)> = ix
            .ranges
            .get(k)
            .into_iter()
            .flatten()
            .flat_map(|r| r.clone())
            .filter_map(|d| ix.keys.get(d as usize))
            .map(|key| (key.validity, key.uid))
            .collect();
        let budget = budget.max(1);
        let came: Vec<Arc<Summary>> = self
            .folder_state(&k.0, &k.1)
            .messages
            .iter()
            .filter(|m| !have.contains(&(m.key.validity, m.key.uid)))
            .take(budget.saturating_add(1))
            .cloned()
            .collect();
        let all = came.len() <= budget;
        let mut b = FolderIndex::new(0);
        for m in came.iter().take(budget) {
            self.index_message(&mut b, m);
        }
        let n = b.keys.len();
        if n > 0 {
            join_folder(ix, k.clone(), b);
        }
        if all {
            ix.stale.retain(|s| s != k);
        }
        // Looking through what the index has of the folder counts too (a few of its messages
        // cost what one indexed does), so a slice doesn't top up many big folders at once.
        n + have.len() / 8 + 1
    }

    /// The folders a search covers changed (the folder list came again, a folder was made,
    /// renamed or deleted, an account removed): the index follows them rather than starting
    /// over (see [`SearchIndex::rescope`]). Whether anything changed.
    pub(crate) fn rescope_index(&mut self) -> bool {
        let Some(scope) = self.index.as_ref().map(|ix| ix.scope) else { return false };
        let wanted = self.scope_folders(scope);
        self.index.as_mut().is_some_and(|ix| ix.rescope(wanted))
    }

    /// A folder's index kept by an earlier run, when it still fits it: its messages are joined to
    /// the index, and the bodies that came in since are read (the folder itself isn't).
    pub(crate) fn kept_folder(&mut self, ix: &mut SearchIndex, account: &str, folder: &str) -> bool {
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
        join_folder(ix, (account.to_string(), folder.to_string()), kept);
        true
    }

    /// Keep a folder's index for the next run (a cache that can't be written is no reason to
    /// fail a search: the folder is read again next time).
    pub(crate) fn keep_folder(&self, account: &str, folder: &str, b: &FolderIndex) {
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
    pub(crate) fn folder_stamp(&mut self, account: &str, folder: &str) -> u64 {
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
    pub(crate) fn words_of(&mut self, key: &MessageKey) -> Option<Arc<str>> {
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
    pub(crate) fn keep_words(&mut self, key: &MessageKey, words: Arc<str>) {
        if self.body_words_bytes + words.len() > BODY_WORDS_BYTES {
            self.body_words.clear();
            self.body_words_bytes = 0;
        }
        self.body_words_bytes += words.len();
        if let Some(old) = self.body_words.insert(key.clone(), words) {
            self.body_words_bytes = self.body_words_bytes.saturating_sub(old.len());
        }
    }

    /// A body that came in joins the search index there is (rather than throwing it away), and
    /// the search's answer looks at its message again.
    pub(crate) fn index_body(&mut self, key: &MessageKey, raw: &[u8]) {
        let words: Arc<str> = Arc::from(mime::search_text(raw));
        // Kept even without an index: a search made later needs no reading again.
        self.keep_words(key, words.clone());
        let Some(ix) = self.index.as_mut() else { return };
        let offset = u32::try_from(ix.keys.len()).unwrap_or(u32::MAX);
        // The folder being read numbers its messages from zero, the whole index after it.
        let doc = if let Some(b) = ix.building.as_mut()
            && let Some(doc) = b.keys.iter().position(|k| k == key)
        {
            if b.has_body.get(doc) == Some(&true) {
                return;
            }
            b.index.add(u32::try_from(doc).unwrap_or(u32::MAX), &words);
            if let Some(seen) = b.has_body.get_mut(doc) {
                *seen = true;
            }
            offset.saturating_add(u32::try_from(doc).unwrap_or(u32::MAX))
        } else {
            // Looked for among its folder's messages only.
            let ranges = ix.ranges.get(&(key.account.clone(), key.folder.clone())).cloned().unwrap_or_default();
            let Some(doc) = ranges.into_iter().flatten().find(|&d| ix.keys.get(d as usize) == Some(key)) else { return };
            if ix.has_body.get(doc as usize) != Some(&false) {
                return;
            }
            ix.index.add(doc, &words);
            if let Some(seen) = ix.has_body.get_mut(doc as usize) {
                *seen = true;
            }
            doc
        };
        if let Some(a) = ix.answer.as_mut() {
            a.again.push(doc..doc.saturating_add(1));
            // The window takes a slice more, and the answer looks at the message then (nothing
            // else would ask, a search done).
            if self.search.is_some() {
                self.search_pending = true;
            }
        }
    }
}
