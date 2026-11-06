// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The session: the search index, built a slice at a time and kept per folder between runs.

use std::ops::Range;

use super::*;

mod ahead;
pub use ahead::IndexWork;
mod kept;
use kept::Kept;

pub(crate) struct SearchIndex {
    /// Whether it reads the messages' words (summary and body); without, only the messages are
    /// listed in it, for the queries their summaries answer (nothing is read, nor kept on disk).
    pub(super) words: bool,
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
    /// The messages the folder being read has already (by validity and number): a kept index
    /// taken back less what changed ([`Kept::Partly`]); passed over as the folder is read.
    pub(super) have: HashSet<(u32, u32)>,
    /// The folder being read is whole and kept on disk: it joins the index at the next step, not
    /// in the same one (a big folder's file written then joined in one go was a call of a tenth
    /// of a second and more).
    pub(super) joining: bool,
    /// Folders read (or being read) whose list changed since: what came in is added to the index
    /// ([`Session::top_up`]) before the scan goes on. A busy mailbox's sync tells of changes every
    /// few seconds; when each threw the whole index away, the scan started over and never ended.
    pub(super) stale: Vec<(String, String)>,
    /// Messages indexed without their source whose source came since without an `Event::Body`
    /// (downloaded ahead in the background, as Cached Exchange Mode keeps its copy, or by a sync's
    /// prefetch), found when their folder was told of: their words are read a few a call.
    pub(super) came: BTreeSet<u32>,
    /// Folders read that are no longer searched (deleted, renamed, out of the scope now): their
    /// messages stay in the index but no longer answer.
    pub(super) gone: HashSet<(String, String)>,
    /// Where each folder read is in the index: the first range is the folder as it was read, in
    /// its list's order then; a folder topped up has more.
    pub(super) ranges: HashMap<(String, String), Vec<Range<u32>>>,
    /// The search's answer, carried on as the index grows (see `answer.rs`).
    pub(super) answer: Option<Answer>,
    /// The folders' kept indexes taken back on the window's search lane (`ahead.rs`).
    pub(super) ahead: Option<Arc<std::sync::Mutex<ahead::Ahead>>>,
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
            || !self.came.is_empty()
            || self
                .answer
                .as_ref()
                .is_some_and(|a| a.checked < self.total() || !a.dirty.is_empty() || !a.again.is_empty() || !a.todo.is_empty() || !a.next.is_empty())
    }

    /// The candidates for a query numbered within `docs`: the index's, then the folder being
    /// read's after them. The summaries' index has no words: every message is one.
    pub(super) fn candidates_in(&self, query: &Query, docs: Range<u32>) -> Vec<u32> {
        let every = Query::default();
        let query = if self.words { query } else { &every };
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
    pub(super) fn docs_of(&self, k: &(String, String)) -> Vec<Range<u32>> {
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
        // The messages changed here, by number (a sync marks the whole folder instead).
        let again: Vec<u32> = match keys {
            None => Vec::new(),
            Some(keys) => {
                let wanted: HashSet<(u32, u32)> =
                    keys.iter().filter(|m| m.account == account && m.folder == folder).map(|m| (m.validity, m.uid)).collect();
                if wanted.is_empty() {
                    Vec::new()
                } else {
                    self.docs_of(&k)
                        .into_iter()
                        .flatten()
                        .filter(|&d| self.doc(d).is_some_and(|(m, _)| wanted.contains(&(m.validity, m.uid))))
                        .collect()
                }
            }
        };
        if let Some(a) = self.answer.as_mut() {
            if keys.is_none() {
                a.dirty.insert(k.clone());
            }
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
            self.have.clear();
            self.joining = false;
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
                    a.dirty.insert(k.clone());
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

/// Most messages taken from a folder's list at once (a call that runs out of time early doesn't
/// copy a big folder's list for nothing).
const SLICE_MAX: usize = 512;

/// A folder's messages take their place in the mailbox's index after the ones before them.
fn join_folder(ix: &mut SearchIndex, k: (String, String), folder: FolderIndex) {
    let offset = u32::try_from(ix.keys.len()).unwrap_or(u32::MAX);
    let n = u32::try_from(folder.keys.len()).unwrap_or(u32::MAX);
    ix.index.absorb(folder.index, offset);
    ix.keys.extend(folder.keys);
    ix.has_body.extend(folder.has_body);
    ix.ranges.entry(k).or_default().push(offset..offset.saturating_add(n));
}

impl Session {
    /// Make sure an index covers this scope, starting one when there's none for it. An index
    /// already under way for the same scope (or a finished one) is kept as it is: the build goes
    /// on a slice at a time through [`Session::index_step`].
    pub(crate) fn ensure_index(&mut self, scope: SearchScope, words: bool) {
        if self.index.as_ref().is_some_and(|ix| ix.words == words && ix.scope == scope && (scope == SearchScope::All || ix.view == self.view)) {
            return;
        }
        // The folders on view are read first (what's on screen is found first), the others in
        // the account's order after them.
        let mut scan: Vec<(String, String)> = self.scope_folders(scope);
        let shown = self.view_folders();
        scan.sort_by_key(|k| !shown.contains(k));
        self.index = Some(SearchIndex {
            words,
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
            have: HashSet::new(),
            joining: false,
            stale: Vec::new(),
            came: BTreeSet::new(),
            gone: HashSet::new(),
            ranges: HashMap::new(),
            answer: None,
            ahead: None,
        });
    }

    /// Index what the call's allowance lets ([`Pace`]: messages at most, and the window's share
    /// of a frame), reading a folder in slices so a big one isn't copied whole. Whether the scan
    /// is done: a big mailbox's first search is spread over several calls, and the window keeps
    /// drawing between them. Between folders, what came into the folders read already is added
    /// first (usually a few messages).
    pub(crate) fn index_step(&mut self) -> bool {
        let Some(mut ix) = self.index.take() else { return true };
        while self.search_pace.index_more() {
            if let Some(d) = ix.came.pop_first() {
                self.read_came(&mut ix, d);
                self.search_pace.spend(1);
                continue;
            }
            // A folder read whole joins the index.
            if ix.joining {
                ix.joining = false;
                if let (Some(b), Some(k)) = (ix.building.take(), ix.scan.get(ix.folder).cloned()) {
                    ix.have.clear();
                    join_folder(&mut ix, k, b);
                }
                ix.folder += 1;
                ix.pos = 0;
                self.search_pace.spend(1);
                continue;
            }
            if ix.building.is_none()
                && ix.pos == 0
                && let Some(k) = ix.stale.first().cloned()
            {
                let cost = self.top_up(&mut ix, &k);
                self.search_pace.spend(cost);
                continue;
            }
            let Some((a, f)) = ix.scan.get(ix.folder).cloned() else { break };
            if ix.building.is_none() && ix.pos == 0 {
                // A folder read by an earlier run is taken as it is; one that changed since,
                // less what went or changed, what came in read as the folder is.
                let kept = if ix.words { self.kept_or_wait(&mut ix, &a, &f) } else { Some(Kept::No) };
                // Being taken back on the lane: the search waits for it (the next call).
                let Some(kept) = kept else { break };
                match kept {
                    Kept::Whole(b) => {
                        // A folder kept on disk is read in one go: a big one takes the whole
                        // slice, so a first search after a restart doesn't load them all at once.
                        // Joined at once (read back, it's half the cost of one written).
                        self.search_pace.spend(b.keys.len().max(1));
                        join_folder(&mut ix, (a, f), b);
                        ix.folder += 1;
                        continue;
                    }
                    Kept::Partly(b, have) => {
                        self.search_pace.spend(b.keys.len().max(1));
                        ix.building = Some(b);
                        ix.have = have;
                        continue;
                    }
                    Kept::No => {}
                }
                // (The summaries' index is never kept: it costs no more than its list.)
                let stamp = if ix.words { self.folder_stamp(&a, &f) } else { 0 };
                ix.building = Some(FolderIndex::new(stamp));
            }
            // A slice of the folder at once (not the folder looked up again for each message);
            // a message is begun only while the call's time allows one more.
            let take = self.search_pace.left().min(SLICE_MAX);
            let slice: Vec<Arc<Summary>> = self.scan_state(&a, &f).messages.iter().skip(ix.pos).take(take).cloned().collect();
            let Some(mut b) = ix.building.take() else { break };
            let (mut n, mut indexed) = (0, 0);
            for m in &slice {
                if indexed > 0 && !self.search_pace.index_more() {
                    break;
                }
                n += 1;
                // Already in (a kept index taken back): passed over.
                if !ix.have.is_empty() && ix.have.remove(&(m.key.validity, m.key.uid)) {
                    continue;
                }
                self.index_message(&mut b, m, ix.words);
                indexed += 1;
            }
            ix.pos += n;
            // Passing a message over costs a little of what reading one does.
            self.search_pace.spend(indexed + (n - indexed) / 64);
            if ix.pos < self.scan_state(&a, &f).messages.len() {
                ix.building = Some(b);
                continue;
            }
            // The folder is read: it's kept for the next run (unless it changed while it was
            // read: a mix of before and after is read again by the next run, and topped up now),
            // and joins the mailbox's index at the next step.
            let k = (a, f);
            if ix.words && !ix.stale.contains(&k) {
                self.keep_folder(&k.0, &k.1, &b);
            }
            ix.building = Some(b);
            ix.joining = true;
            self.search_pace.spend(1);
        }
        ix.done = ix.folder >= ix.scan.len();
        let done = !ix.pending();
        self.index = Some(ix);
        done
    }

    /// One message into a folder's index: its summary's words and its body's, when downloaded
    /// (`words`; without, the message alone).
    fn index_message(&mut self, b: &mut FolderIndex, m: &Summary, words: bool) {
        let doc = u32::try_from(b.keys.len()).unwrap_or(u32::MAX);
        if !words {
            b.index.add(doc, "");
            b.has_body.push(false);
            b.keys.push(m.key.clone());
            return;
        }
        b.sums.push(summary_sum(m));
        b.index.add(doc, &summary_text(m));
        let words = self.words_of(&m.key);
        if let Some(words) = &words {
            b.index.add(doc, words);
        }
        b.has_body.push(words.is_some());
        b.keys.push(m.key.clone());
    }

    /// What came into a folder read already: its messages the index hasn't got join the index
    /// (the folder isn't read again), as many as the call's allowance lets. What it cost (at
    /// least one); the folder is up to date once nothing is left.
    pub(crate) fn top_up(&mut self, ix: &mut SearchIndex, k: &(String, String)) -> usize {
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
        let budget = self.search_pace.left().max(1);
        let came: Vec<Arc<Summary>> = self
            .scan_state(&k.0, &k.1)
            .messages
            .iter()
            .filter(|m| !have.contains(&(m.key.validity, m.key.uid)))
            .take(budget.saturating_add(1))
            .cloned()
            .collect();
        let mut b = FolderIndex::new(0);
        for m in came.iter().take(budget) {
            if !b.keys.is_empty() && !self.search_pace.index_more() {
                break;
            }
            self.index_message(&mut b, m, ix.words);
        }
        let n = b.keys.len();
        let all = n == came.len();
        if n > 0 {
            join_folder(ix, k.clone(), b);
        }
        if all {
            ix.stale.retain(|s| s != k);
            if ix.words {
                self.sources_came(ix, k);
            }
        }
        // Looking through what the index has of the folder counts too (a few of its messages
        // cost what one indexed does), so a slice doesn't top up many big folders at once.
        n + have.len() / 8 + 1
    }

    /// The messages of a folder read already, indexed without their source, whose source is in
    /// the cache now: queued ([`SearchIndex::came`]). One listing of the folder's sources, when
    /// it's told of (a sync, a download ahead), only for a folder with messages still without.
    fn sources_came(&mut self, ix: &mut SearchIndex, k: &(String, String)) {
        let missing: Vec<u32> =
            ix.ranges.get(k).into_iter().flatten().flat_map(|r| r.clone()).filter(|&d| ix.has_body.get(d as usize) == Some(&false)).collect();
        if missing.is_empty() {
            return;
        }
        let validity = self.scan_state(&k.0, &k.1).validity;
        let Some(store) = self.store.as_ref() else { return };
        let cached: HashSet<u32> = store.cached_bodies(&k.0, &k.1, validity).into_iter().collect();
        for d in missing {
            if ix.keys.get(d as usize).is_some_and(|key| key.validity == validity && cached.contains(&key.uid)) {
                ix.came.insert(d);
            }
        }
    }

    /// A message's source that came since it was indexed: its words join the index, and the
    /// search's answer looks at it again.
    fn read_came(&mut self, ix: &mut SearchIndex, d: u32) {
        let Some(key) = ix.keys.get(d as usize).cloned() else { return };
        if ix.has_body.get(d as usize) != Some(&false) {
            return;
        }
        let Some(words) = self.words_of(&key) else { return };
        ix.index.add(d, &words);
        if let Some(seen) = ix.has_body.get_mut(d as usize) {
            *seen = true;
        }
        if let Some(a) = ix.answer.as_mut() {
            a.again.insert(d);
        }
    }

    /// The folders a search covers changed (the folder list came again, a folder was made,
    /// renamed or deleted, an account removed): the index follows them rather than starting
    /// over (see [`SearchIndex::rescope`]). Whether anything changed.
    pub(crate) fn rescope_index(&mut self) -> bool {
        let mut changed = false;
        for summaries in [false, true] {
            let ix = if summaries { &self.summary_index } else { &self.index };
            let Some(scope) = ix.as_ref().map(|ix| ix.scope) else { continue };
            let wanted = self.scope_folders(scope);
            let ix = if summaries { &mut self.summary_index } else { &mut self.index };
            changed |= ix.as_mut().is_some_and(|ix| ix.rescope(wanted));
        }
        changed
    }

    /// What the word index holds, for the benchmarks: its messages, words, documents counted once
    /// per word, the words' bytes; and the bytes of message text kept for the next search.
    #[cfg(test)]
    pub(crate) fn search_index_sizes(&self) -> Option<(usize, usize, usize, usize, usize)> {
        let ix = self.index.as_ref()?;
        let (words, postings, bytes) = ix.index.sizes();
        Some((ix.keys.len(), words, postings, bytes, self.body_words_bytes))
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
            a.again.insert(doc);
            // The window takes a slice more, and the answer looks at the message then (nothing
            // else would ask, a search done).
            if self.search.is_some() {
                self.search_pending = true;
            }
        }
    }
}
