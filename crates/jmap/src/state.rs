//! What JMAP sync keeps between runs (states, ids), and messages' summaries from their properties.

use std::sync::Arc;

use super::*;

mod ids;
pub(crate) use ids::Ids;

/// What an account keeps between syncs: the numbers given to message ids, and the mailbox id
/// behind each folder path.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub(super) next: u32,
    /// Both ways, each id kept small ([`Ids`]): an Exchange id is some 150 characters, and a
    /// 530,000-message mailbox's ids were in memory twice, three times during a full sync —
    /// some 500 MB.
    #[serde(default)]
    pub(super) ids: Ids,
    #[serde(default)]
    pub mailboxes: BTreeMap<String, String>,
    /// The folders of other mailboxes opened in the account: folder path → the mailbox's
    /// address (Microsoft Graph reaches them under `/users/<address>`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub owners: BTreeMap<String, String>,
    /// Messages a search of the server found in each folder (issue #4), kept there past the
    /// newest window while the server has them there.
    #[serde(default)]
    pub found: BTreeMap<String, Vec<u32>>,
    /// Where each folder was when last read in full (Exchange): a folder found as it was is
    /// left alone — one look at it rather than a listing of up to its whole window, folder after
    /// folder, every pass.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub marks: BTreeMap<String, FolderMark>,
    /// What was last written ([`State::fingerprint`]): the same isn't written again.
    #[serde(skip)]
    saved: std::cell::Cell<u64>,
    /// A reader's ([`State::reserve`]): the numbering it started from, looked up, never changed.
    #[serde(skip)]
    base: Option<Arc<Numbering>>,
    /// A reader's: the numbers it gave since it last told ([`State::take_given`]).
    #[serde(skip)]
    given: Vec<(String, u32)>,
}

/// An account's numbering as it was when a full sync handed folders to readers of their own:
/// one copy shared by them all.
#[derive(Debug, Default)]
pub struct Numbering {
    ids: Ids,
}

/// A folder as it was when last read in full: its last change as the server stamps it, its
/// counts, the window it was read with, and when (seconds) — a mark this old isn't trusted.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderMark {
    pub stamp: String,
    pub total: u32,
    pub unread: u32,
    pub window: usize,
    pub at: i64,
    /// How many of its newest were to be kept downloaded then (more asked: it's read again).
    #[serde(default)]
    pub ahead: usize,
    /// Exchange's state of the folder as read (`SyncFolderItems`): the next read asks only what
    /// changed since. Empty: unknown, the folder is listed.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sync: String,
    /// When the folder was found too big to follow by its changes (Exchange: more than a state
    /// is read for): it's listed without asking again until a week later. Asked at every pass,
    /// two folders of 100,000 reports each took a quarter of an hour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub too_big: Option<i64>,
    /// The age kept then ([`SyncOptions::since_days`]): another one lists the folder again.
    #[serde(default)]
    pub days: u32,
    /// Exchange: its messages' categories were read (they weren't before Quatrelle read
    /// categories: those Outlook put on earlier are asked for once).
    #[serde(default)]
    pub categories: bool,
}

/// Messages a folder's search of the server brings in, at most.
pub const FOUND_MAX: usize = 500;

pub(super) const STATE_DOC: &str = "jmap.json";

impl State {
    /// Whose mailbox a folder is in, when it's another one opened in the account.
    pub fn owner(&self, path: &str) -> Option<&str> {
        self.owners.get(path).map(String::as_str)
    }

    pub fn load(store: &Store, account: &str) -> State {
        let mut s: State = store.load_account_doc(account, STATE_DOC).unwrap_or_default();
        s.next = s.next.max(s.ids.numbers().max().unwrap_or(0));
        s.saved.set(s.fingerprint());
        s
    }

    /// A digest of what's kept: what was written is written again only once it has changed.
    /// The numbering of a big mailbox is tens of megabytes, and it was rewritten after every
    /// folder read, changed or not.
    fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (self.next, self.ids.len()).hash(&mut h);
        self.ids.numbers().fold(0u64, |a, n| a.wrapping_add(u64::from(n).wrapping_mul(0x9e37_79b9_7f4a_7c15))).hash(&mut h);
        self.mailboxes.hash(&mut h);
        self.owners.hash(&mut h);
        self.found.hash(&mut h);
        for (k, m) in &self.marks {
            (k, &m.stamp, m.total, m.unread, m.window, m.at, m.ahead, &m.sync).hash(&mut h);
        }
        h.finish()
    }

    /// Keep the numbering on disk — never an empty one over the one there: a state that lost
    /// its ids (a mistake, a connection refused mid-way) would take every cached message's key
    /// with it, and the next sync would wipe the folder states and download everything again.
    pub fn save(&self, store: &Store, account: &str) -> Result<()> {
        if self.ids.is_empty() {
            return Ok(());
        }
        let now = self.fingerprint();
        if now == self.saved.get() {
            return Ok(());
        }
        store.save_account_doc(account, STATE_DOC, self)?;
        self.saved.set(now);
        Ok(())
    }

    /// The number standing for a message id (given on first sight).
    pub fn number(&mut self, id: &str) -> u32 {
        if let Some(n) = self.known(id) {
            return n;
        }
        self.next = self.next.saturating_add(1);
        let n = self.next;
        self.set_number(id, n);
        n
    }

    /// Give an id the number it had: a message found again after the numbering lost it.
    pub fn set_number(&mut self, id: &str, n: u32) {
        self.ids.insert(id, n);
        self.next = self.next.max(n);
        if self.base.is_some() {
            self.given.push((id.to_string(), n));
        }
    }

    /// How many message ids the numbering knows (none: it numbered nothing yet).
    pub fn len(&self) -> usize {
        self.ids.len() + self.base.as_ref().map_or(0, |b| b.ids.len())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The message id a number stands for.
    pub fn id(&self, n: u32) -> Option<String> {
        self.ids.id(n).or_else(|| self.base.as_ref().and_then(|b| b.ids.id(n)))
    }

    /// The number a message id has, if it has one (none is given).
    pub fn known(&self, id: &str) -> Option<u32> {
        self.ids.get(id).or_else(|| self.base.as_ref().and_then(|b| b.ids.get(id)))
    }

    /// The mailbox id behind a folder path.
    pub fn mailbox(&self, path: &str) -> Result<&str> {
        self.mailboxes.get(path).map(String::as_str).ok_or_else(|| Error::No(format!("there is no folder {} on the server", utf7::decode(path))))
    }

    /// A copy of this state for a connection syncing on its own, with `count` numbers kept for it
    /// so that two connections never give the same number to different messages. The numbers it
    /// took are given back by [`State::absorb`], and [`State::compact`] returns the counter to
    /// the numbers really in use once every such connection has finished.
    ///
    /// It knows the numbers given before (`numbering`, [`State::numbering`]): one that didn't
    /// numbered every message of a folder it listed again (their downloads thrown away, edits
    /// made meanwhile on the old numbers lost) and ignored the read marks and deletions of the
    /// messages it had.
    pub fn reserve(&mut self, count: u32, numbering: &Arc<Numbering>) -> State {
        let base = self.next;
        self.next = self.next.saturating_add(count);
        State {
            next: base,
            ids: Ids::default(),
            mailboxes: self.mailboxes.clone(),
            owners: self.owners.clone(),
            found: BTreeMap::new(),
            marks: self.marks.clone(),
            saved: std::cell::Cell::new(0),
            base: Some(numbering.clone()),
            given: Vec::new(),
        }
    }

    /// The numbering as it is, for readers of their own ([`State::reserve`]).
    pub fn numbering(&self) -> Arc<Numbering> {
        Arc::new(Numbering { ids: self.ids.clone() })
    }

    /// A reader's: the numbers it gave since it last told, for the main connection to know them
    /// at once ([`State::learn`]) — not only once the reader is done, a whole first sync later,
    /// while a message it brought in is read and the mark found no number to go to the server.
    pub fn take_given(&mut self) -> Vec<(String, u32)> {
        std::mem::take(&mut self.given)
    }

    /// The numbers a reader gave ([`State::take_given`]).
    pub fn learn(&mut self, given: Vec<(String, u32)>) {
        for (id, n) in given {
            self.ids.insert(&id, n);
        }
    }

    /// Take what a connection of its own numbered while it read folders.
    pub fn absorb(&mut self, other: State) {
        self.ids.absorb(&other.ids);
        // A folder it listed for the first time, or another mailbox it met.
        for (path, id) in other.mailboxes {
            self.mailboxes.entry(path).or_insert(id);
        }
        self.owners.extend(other.owners);
        // The folders it read, as they were: the newer mark of each (it started from a copy of
        // these, so its other marks are older than any this connection set meanwhile).
        for (folder, m) in other.marks {
            if self.marks.get(&folder).is_none_or(|cur| cur.at < m.at) {
                self.marks.insert(folder, m);
            }
        }
    }

    /// Put the counter back after the numbers kept by [`State::reserve`], so a long run of
    /// parallel syncs doesn't walk the whole range.
    pub fn compact(&mut self) {
        self.next = self.ids.numbers().max().unwrap_or(0).saturating_add(1);
    }

    /// Forget numbers no cached message uses any more.
    pub fn prune(&mut self, keep: &std::collections::HashSet<u32>) {
        self.ids.retain(|n| keep.contains(&n));
        for v in self.found.values_mut() {
            v.retain(|n| keep.contains(n));
        }
        self.found.retain(|_, v| !v.is_empty());
    }

    /// Whether a search of the server found this message in this folder (kept past the window).
    pub fn was_found(&self, folder: &str, n: u32) -> bool {
        self.found.get(folder).is_some_and(|v| v.contains(&n))
    }

    /// What a folder's search of the server found, added to what earlier ones did (the newest
    /// finds first, [`FOUND_MAX`] at most), as IMAP keeps what it fetches.
    pub fn add_found(&mut self, folder: &str, found: &[u32]) {
        if found.is_empty() || (self.found.len() >= 1000 && !self.found.contains_key(folder)) {
            return;
        }
        let v = self.found.entry(folder.to_string()).or_default();
        let mut next: Vec<u32> = found.to_vec();
        next.extend(v.iter().filter(|n| !found.contains(n)));
        next.truncate(FOUND_MAX);
        *v = next;
    }

    /// Keep, of a folder's finds, those its server still has there.
    pub fn keep_found(&mut self, folder: &str, still: impl Fn(u32) -> bool) {
        if let Some(v) = self.found.get_mut(folder) {
            v.retain(|n| still(*n));
            if v.is_empty() {
                self.found.remove(folder);
            }
        }
    }

    /// The server ids of a folder's cached messages that are there only because a search found
    /// them (out of the newest window `wanted`): to check they're still there, and their flags.
    pub fn found_outside(&self, folder: &str, cached: &[Summary], wanted: &HashMap<u32, Flags>) -> Vec<String> {
        cached.iter().filter(|m| !wanted.contains_key(&m.key.uid) && self.was_found(folder, m.key.uid)).filter_map(|m| self.id(m.key.uid)).collect()
    }
}

/// How a folder sync goes.
#[derive(Clone, Copy, Debug)]
pub struct SyncOptions {
    /// How many of the newest messages the first sync of a folder fetches.
    pub window: usize,
    /// How many of the newest message sources to keep downloaded.
    pub prefetch: usize,
    /// Sources larger than this are only downloaded when opened.
    pub prefetch_max_bytes: u32,
    /// Folders read on connections of their own, beside the main one (0: one at a time). The
    /// main connection then stays free for what someone is waiting on.
    pub parallel: usize,
    /// The newest `window` listed, nothing more: no state of the folder is asked for to follow
    /// its changes (a public folder opened but not followed, read again each time it's opened).
    pub list_only: bool,
    /// Mail received in the last this many days is kept (0: all of it), as Outlook's "Mail to
    /// keep offline" in Cached Exchange Mode; older mail stays on the server, read there.
    pub since_days: u32,
    /// Every message kept is downloaded whole in the end, a few at a time while nothing else
    /// is asked (`prefetch` of the newest come with the folder's list, the rest after).
    pub full: bool,
}

impl Default for SyncOptions {
    fn default() -> Self {
        SyncOptions { window: 1000, prefetch: 40, prefetch_max_bytes: 2 << 20, parallel: 0, list_only: false, since_days: 0, full: false }
    }
}

impl SyncOptions {
    /// The oldest a message kept may be (Unix seconds, as of `now`), if the age is limited.
    pub fn since(&self, now: i64) -> Option<i64> {
        (self.since_days > 0).then(|| now.saturating_sub(i64::from(self.since_days).saturating_mul(86_400)))
    }
}

/// What a folder sync changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub new: usize,
    pub changed: usize,
    pub removed: usize,
    pub total: u32,
    pub unread: u32,
    /// The cache was emptied because UIDVALIDITY changed.
    pub reset: bool,
}

impl SyncReport {
    pub fn touched(&self) -> bool {
        self.new + self.changed + self.removed > 0 || self.reset
    }
}

/// Fill in what only the full source tells (preview, attachments, size).
pub fn fill_from_source(m: &mut Summary, raw: &[u8]) {
    let full = mime::summary_from_full(raw, m.key.clone(), m.flags.clone(), m.size, m.date);
    m.preview = full.preview;
    m.has_attachments = full.has_attachments;
    // Entries built from a Graph listing carry no References or List-Id: take them now.
    let threaded = !m.in_reply_to.is_empty() || !m.references.is_empty();
    if !threaded {
        (m.in_reply_to, m.references) = (full.in_reply_to, full.references);
    }
    m.bulk = m.bulk || full.bulk;
    m.importance = if m.importance != 0 { m.importance } else { full.importance };
    if m.size == 0 {
        m.size = u32::try_from(raw.len()).unwrap_or(u32::MAX);
    }
}

/// A message's list entry from its `Email/get` details.
pub(super) fn email_summary(e: &Value, key: MessageKey) -> Summary {
    let mut header = Vec::new();
    for h in list(Some(e), "headers").iter().take(500) {
        let (Some(name), Some(value)) = (h.get("name").and_then(Value::as_str), h.get("value").and_then(Value::as_str)) else { continue };
        if name.is_empty() || name.contains([':', '\r', '\n']) {
            continue;
        }
        header.extend_from_slice(format!("{name}:{value}\r\n").as_bytes());
    }
    header.extend_from_slice(b"\r\n");
    let size = e.get("size").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()).unwrap_or(0);
    let received = e.get("receivedAt").and_then(Value::as_str).and_then(utc_date).unwrap_or(0);
    let mut s = mime::summary_from_header(&header, key, keywords(e.get("keywords")), size, received);
    s.preview = e.get("preview").and_then(Value::as_str).map(mime::preview).unwrap_or_default();
    s.has_attachments = e.get("hasAttachment").and_then(Value::as_bool).unwrap_or(false);
    s
}
