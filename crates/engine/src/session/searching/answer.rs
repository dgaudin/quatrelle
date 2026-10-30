//! The session: a search's answer, carried on while its index grows.
//!
//! The window asks for a slice of the index a frame. Looking through the whole index after each
//! slice made every frame dearer than the last, quadratic over a big mailbox's scan; the answer
//! is kept with the index instead, and a slice looks only at the messages indexed since. What it
//! found is looked up again where something changed: a folder told of changes, a body that came
//! in, a folder that came back.

use std::collections::VecDeque;
use std::ops::Range;

use super::*;

/// What a search found so far.
#[derive(Default)]
pub(crate) struct Answer {
    /// The query it answers, as the index reads it.
    pub(super) query: Query,
    /// The messages waiting for a reply then, when the query asks about them: they change with
    /// the clock and with mail anywhere else, and the answer starts over when they do.
    pub(super) awaiting: HashSet<MessageKey>,
    /// The messages looked at so far, by number: the index's, then the folder being read's.
    pub(super) checked: u32,
    /// What it found, in the order found.
    pub(super) found: Vec<Found>,
    pub(super) have: Keys,
    /// Folders whose messages found are looked up again: their list changed, or they went.
    pub(super) recheck: HashSet<(String, String)>,
    /// Messages to look at again, by number: their folder changed (one marked unread may answer
    /// `is:unread` now), a body came in, or their folder came back.
    pub(super) again: Vec<Range<u32>>,
    /// The messages the index found for the query and not looked at yet: a slice's worth is
    /// looked at a call (a new query over a big index, or a big folder that changed, doesn't
    /// load every folder in one frame).
    pub(super) todo: VecDeque<u32>,
    /// How many were found when the list last showed them.
    pub(super) shown: usize,
    /// Something found went or changed since the list last showed them.
    pub(super) changed: bool,
}

/// A message found.
pub(super) struct Found {
    /// As its folder's list had it when last looked at.
    pub(super) summary: Arc<Summary>,
    /// Where it was in that list (looked at there first).
    pub(super) pos: usize,
    /// Its number in the index.
    pub(super) doc: u32,
}

/// Messages by folder and number: a set of keys without a copy of their folder's name each
/// (a search finding most of a big mailbox kept as many).
#[derive(Default)]
pub(super) struct Keys(HashMap<String, HashMap<String, HashSet<(u32, u32)>>>);

impl Keys {
    fn contains(&self, k: &MessageKey) -> bool {
        self.0.get(&k.account).and_then(|f| f.get(&k.folder)).is_some_and(|s| s.contains(&(k.validity, k.uid)))
    }

    fn insert(&mut self, k: &MessageKey) {
        self.0.entry(k.account.clone()).or_default().entry(k.folder.clone()).or_default().insert((k.validity, k.uid));
    }

    fn remove(&mut self, k: &MessageKey) {
        if let Some(s) = self.0.get_mut(&k.account).and_then(|f| f.get_mut(&k.folder)) {
            s.remove(&(k.validity, k.uid));
        }
    }
}

/// While few are found, the list shows each slice's finds; past this, each time a quarter more
/// are found (showing it sorts all of them: each slice's would grow with what's found).
const SHOW_EACH_UP_TO: usize = 500;

/// Messages looked at for one indexed, in a slice: looking at one (found where it was in its
/// list, checked against the query) costs a small part of indexing one.
const LOOKS_PER_MESSAGE_INDEXED: usize = 8;

/// What a message is checked against: the query, and what's waiting for a reply.
struct Looking<'a> {
    query: &'a Query,
    awaiting: &'a HashSet<MessageKey>,
}

/// Each folder's messages by number, made once a call when a message isn't where it was.
type Lists = HashMap<(String, String), HashMap<u32, usize>>;

impl Session {
    /// The query as the index reads it: `category:Work` means the Work category's keyword
    /// (`$label2`), or its name's as Exchange keeps it (`work`); `is:categorized` means one of the
    /// categories.
    pub(super) fn search_query(&self, q: &str) -> Query {
        let mut query = Query::parse(q);
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
        query
    }

    /// A slice more of the search's index (whatever is left, for a search asked for at once), and
    /// its answer brought up to date. Whether the list should show the answer again now: always
    /// once the index is done; while it's being built, at a pace that keeps a slice's cost flat.
    pub(super) fn search_step(&mut self, q: &str, scope: SearchScope, budget: usize) -> bool {
        let query = self.search_query(q);
        if query.is_empty() {
            self.search_pending = false;
            return true;
        }
        self.ensure_index(scope);
        self.index_step(budget);
        self.update_answer(&query);
        let done = !self.index.as_ref().is_some_and(SearchIndex::pending);
        self.search_pending = !done;
        let Some(a) = self.index.as_ref().and_then(|ix| ix.answer.as_ref()) else { return true };
        let more = a.found.len().saturating_sub(a.shown);
        done || a.changed || (more > 0 && (a.found.len() <= SHOW_EACH_UP_TO || more >= a.shown / 4))
    }

    /// The search's answer for the list: the index brought up to date first when its scan is
    /// done (what came into the folders since, a slice at most), each message as its folder's
    /// list in memory has it now (marked read since, flagged…).
    pub(crate) fn search_results(&mut self, q: &str, scope: SearchScope) -> Vec<Arc<Summary>> {
        let query = self.search_query(q);
        if query.is_empty() {
            self.search_pending = false;
            return Vec::new();
        }
        self.ensure_index(scope);
        if self.index.as_ref().is_some_and(|ix| ix.done && !ix.stale.is_empty()) {
            let budget = self.search_budget;
            self.index_step(budget);
        }
        self.update_answer(&query);
        self.search_pending = self.index.as_ref().is_some_and(SearchIndex::pending);
        let Some(mut ix) = self.index.take() else { return Vec::new() };
        let Some(mut answer) = ix.answer.take() else {
            self.index = Some(ix);
            return Vec::new();
        };
        // Each one as its list in memory has it; a folder whose list moved is looked up again.
        let mut redo = Vec::new();
        let mut moved: HashSet<(String, String)> = HashSet::new();
        let mut list: Option<(&str, &str, Option<&FolderState<Arc<Summary>>>)> = None;
        for (i, f) in answer.found.iter().enumerate() {
            let key = &f.summary.key;
            if !list.is_some_and(|(a, fo, _)| a == key.account && fo == key.folder) {
                let st = self.cache.get(&(key.account.clone(), key.folder.clone()));
                list = Some((key.account.as_str(), key.folder.as_str(), st));
            }
            let Some((_, _, Some(st))) = list else { continue };
            match st.messages.get(f.pos) {
                Some(m) if m.key == *key => {
                    if !Arc::ptr_eq(m, &f.summary) {
                        redo.push((i, m.clone()));
                    }
                }
                _ => {
                    moved.insert((key.account.clone(), key.folder.clone()));
                }
            }
        }
        if !redo.is_empty() || !moved.is_empty() {
            let awaiting = answer.awaiting.clone();
            let looking = Looking { query: &query, awaiting: &awaiting };
            let mut drop = HashSet::new();
            for (i, m) in redo {
                let Some(f) = answer.found.get_mut(i) else { continue };
                let has_body = ix.doc(f.doc).is_some_and(|(_, b)| b);
                f.summary = m;
                if !self.answers(&looking, &f.summary, has_body) {
                    drop.insert(i);
                }
            }
            if !drop.is_empty() {
                let mut i = 0;
                answer.found.retain(|f| {
                    i += 1;
                    let keep = !drop.contains(&(i - 1));
                    if !keep {
                        answer.have.remove(&f.summary.key);
                    }
                    keep
                });
            }
            self.recheck_found(&mut ix, &mut answer, &looking, moved);
        }
        let out = answer.found.iter().map(|f| f.summary.clone()).collect();
        answer.shown = answer.found.len();
        answer.changed = false;
        ix.answer = Some(answer);
        self.index = Some(ix);
        out
    }

    /// Bring the answer up to date: what it found in folders that changed is looked up again;
    /// the messages to look at again (a folder that changed, a body that came in, a folder back)
    /// and those indexed since it last looked queue up as the index finds them; a slice's worth
    /// of the queue is looked at. A new query starts a new answer, and so does a change in what is
    /// waiting for a reply when it asks about that.
    fn update_answer(&mut self, query: &Query) {
        let awaiting = self.awaiting_for(query);
        let Some(mut ix) = self.index.take() else { return };
        let mut answer = match ix.answer.take() {
            Some(a) if a.query == *query && a.awaiting == awaiting => a,
            _ => Answer { query: query.clone(), awaiting: awaiting.clone(), changed: true, ..Default::default() },
        };
        let total = ix.total();
        if answer.recheck.is_empty() && answer.again.is_empty() && answer.todo.is_empty() && answer.checked >= total {
            ix.answer = Some(answer);
            self.index = Some(ix);
            return;
        }
        let looking = Looking { query, awaiting: &awaiting };
        let folders = std::mem::take(&mut answer.recheck);
        self.recheck_found(&mut ix, &mut answer, &looking, folders);
        // Finding the candidates costs their words, not their messages: they queue up whole (once
        // each), and the messages are looked at a slice's worth a call.
        for r in std::mem::take(&mut answer.again) {
            answer.todo.extend(ix.candidates_in(query, r));
        }
        if answer.checked < total {
            answer.todo.extend(ix.candidates_in(query, answer.checked..total));
            answer.checked = total;
        }
        let room = self.search_budget.max(1).saturating_mul(LOOKS_PER_MESSAGE_INDEXED);
        let mut looked = 0;
        let mut lists = Lists::new();
        while looked < room
            && let Some(d) = answer.todo.pop_front()
        {
            // Found already or gone from the search: nothing to look at (and nothing counted).
            let Some((key, has_body)) = ix.doc(d) else { continue };
            if answer.have.contains(key) || ix.is_gone(key) {
                continue;
            }
            looked += 1;
            let key = key.clone();
            let Some((m, pos)) = self.look_up(&mut lists, &key, ix.hint(d, &key)) else { continue };
            if self.answers(&looking, &m, has_body) {
                answer.have.insert(&key);
                answer.found.push(Found { summary: m, pos, doc: d });
            }
        }
        ix.answer = Some(answer);
        self.index = Some(ix);
    }

    /// The messages found in these folders, looked up again in their lists: gone from there (or
    /// their folder gone from the search), they go; changed, they're checked again.
    fn recheck_found(&mut self, ix: &mut SearchIndex, answer: &mut Answer, looking: &Looking<'_>, folders: HashSet<(String, String)>) {
        if folders.is_empty() {
            return;
        }
        let mut lists = Lists::new();
        let mut kept = Vec::with_capacity(answer.found.len());
        for mut f in std::mem::take(&mut answer.found) {
            let key = f.summary.key.clone();
            if !folders.iter().any(|(a, fo)| *a == key.account && *fo == key.folder) {
                kept.push(f);
                continue;
            }
            let gone = ix.is_gone(&key);
            let now = if gone { None } else { self.look_up(&mut lists, &key, f.pos) };
            match now {
                Some((m, _)) if Arc::ptr_eq(&m, &f.summary) => kept.push(f),
                Some((m, pos)) => {
                    let has_body = ix.doc(f.doc).is_some_and(|(_, b)| b);
                    answer.changed = true;
                    if self.answers(looking, &m, has_body) {
                        f.summary = m;
                        f.pos = pos;
                        kept.push(f);
                    } else {
                        answer.have.remove(&key);
                    }
                }
                None => {
                    answer.changed = true;
                    answer.have.remove(&key);
                }
            }
        }
        answer.found = kept;
    }

    /// Whether a message the index found for the query's words answers it, its body read again
    /// only when the index can't settle it.
    fn answers(&mut self, looking: &Looking<'_>, m: &Summary, has_body: bool) -> bool {
        let body = if has_body && looking.query.needs_body() { self.words_of(&m.key) } else { None };
        looking.query.matches(&Doc { summary: m, body: body.as_deref(), indexed: true, awaiting: looking.awaiting.contains(&m.key) })
    }

    /// A message as its folder's list has it now, and where: looked at where it was first, else
    /// found by number (the folder's numbers made once a call).
    fn look_up(&mut self, lists: &mut Lists, key: &MessageKey, hint: usize) -> Option<(Arc<Summary>, usize)> {
        let st = self.folder_state(&key.account, &key.folder);
        if let Some(m) = st.messages.get(hint).filter(|m| m.key == *key) {
            return Some((m.clone(), hint));
        }
        let at = lists
            .entry((key.account.clone(), key.folder.clone()))
            .or_insert_with(|| st.messages.iter().enumerate().map(|(i, m)| (m.key.uid, i)).collect())
            .get(&key.uid)
            .copied()?;
        st.messages.get(at).filter(|m| m.key == *key).map(|m| (m.clone(), at))
    }

    /// The messages waiting for a reply, when the query asks about them.
    fn awaiting_for(&mut self, query: &Query) -> HashSet<MessageKey> {
        if query.clauses.iter().any(|c| asks_awaiting(&c.term)) { self.awaiting_now() } else { HashSet::new() }
    }
}
