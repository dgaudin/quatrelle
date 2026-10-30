//! The session: the search index, built a slice at a time and kept per folder between runs.

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
}

impl SearchIndex {
    /// Whether this index covers a folder (read, being read, or still to read).
    pub(crate) fn covers(&self, account: &str, folder: &str) -> bool {
        self.scan.iter().any(|(a, f)| a == account && f == folder)
    }
}

/// A folder's messages take their place in the mailbox's index after the ones before them.
fn join_folder(ix: &mut SearchIndex, folder: FolderIndex) {
    let offset = u32::try_from(ix.keys.len()).unwrap_or(u32::MAX);
    ix.index.absorb(folder.index, offset);
    ix.keys.extend(folder.keys);
    ix.has_body.extend(folder.has_body);
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
        });
    }

    /// Index at most `budget` more messages (`usize::MAX`: whatever is left), reading a folder in
    /// slices so a big one isn't copied whole. Whether the scan is done: a big mailbox's first
    /// search is spread over several calls, one slice each, and the window keeps drawing between
    /// them.
    pub(crate) fn index_step(&mut self, budget: usize) -> bool {
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
        join_folder(ix, kept);
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

    /// A body that came in joins the search index there is (rather than throwing it away).
    pub(crate) fn index_body(&mut self, key: &MessageKey, raw: &[u8]) {
        let words: Arc<str> = Arc::from(mime::search_text(raw));
        // Kept even without an index: a search made later needs no reading again.
        self.keep_words(key, words.clone());
        let Some(ix) = self.index.as_mut() else { return };
        // The folder being read numbers its messages from zero, the whole index after it.
        if let Some(b) = ix.building.as_mut()
            && let Some(doc) = b.keys.iter().position(|k| k == key)
        {
            if b.has_body.get(doc) != Some(&true) {
                b.index.add(u32::try_from(doc).unwrap_or(u32::MAX), &words);
                if let Some(seen) = b.has_body.get_mut(doc) {
                    *seen = true;
                }
            }
            return;
        }
        if let Some(doc) = ix.keys.iter().position(|k| k == key)
            && ix.has_body.get(doc) == Some(&false)
        {
            ix.index.add(u32::try_from(doc).unwrap_or(u32::MAX), &words);
            if let Some(seen) = ix.has_body.get_mut(doc) {
                *seen = true;
            }
        }
    }
}
