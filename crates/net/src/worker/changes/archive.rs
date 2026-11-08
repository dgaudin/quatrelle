// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The local archive in an account's worker: IMAP mail copied into the Archives account on this
//! computer, checked, and only then deleted on the server. The deletion is the user's own delete
//! put in the account's journal, so a lost connection or a closed Quatrelle only delays it; a stop
//! between the copy and the deletion leaves the message in both places, never in neither.

use std::collections::{BTreeSet, HashMap};

use quatrelle_model::{AccountConfig, AccountRole, FolderRole, archive_folder, google_mailbox};
use quatrelle_store::FolderState;

use super::*;

mod copy;

pub(crate) use copy::{DELIMITER, copy_checked};

/// Most messages archived by one job.
pub(crate) const MAX_BATCH: usize = 500;

/// Room left on the disk after a batch.
const SPARE: u64 = 100 << 20;

/// Whether `available` bytes on the disk hold a batch of `needed` bytes and leave 100 MB.
pub(crate) fn has_room(available: u64, needed: u64) -> bool {
    available >= needed.saturating_add(SPARE)
}

/// Whether the disk the mail is kept on has room for a batch of `needed` bytes (a cache kept in
/// memory, or a disk that can't be asked, has).
pub(crate) fn room_for(store: &Store, needed: u64) -> bool {
    match store.data_dir().and_then(|dir| fs4::statvfs(dir).ok()) {
        Some(stats) => has_room(stats.available_space(), needed),
        None => true,
    }
}

/// What a batch did, before the deletions are journaled.
#[derive(Default)]
pub(crate) struct Batch {
    /// Each message's key, and its key in the Archives account.
    pub moved: Vec<(MessageKey, MessageKey)>,
    /// Messages left where they were, and why.
    pub kept: Vec<(MessageKey, String)>,
    /// The Archives folders written to.
    pub folders: BTreeSet<String>,
}

impl Batch {
    fn keep_all(keys: Vec<MessageKey>, why: &str) -> Batch {
        Batch { kept: keys.into_iter().map(|k| (k, why.to_string())).collect(), ..Default::default() }
    }
}

/// Copy `keys` (IMAP mail of `acc`, at most [`MAX_BATCH`]) into the Archives account `to_account`,
/// each checked by reading it back. `fetch` brings a message's source from the server when the
/// cache doesn't keep it whole. A copy that can't be made or doesn't read back the same stops the
/// batch: that message and the rest are kept, those copied before it stay copied. Nothing here
/// touches the server.
pub(crate) fn archive_batch(
    store: &Store,
    acc: &AccountConfig,
    keys: Vec<MessageKey>,
    to_account: &str,
    fetch: &mut dyn FnMut(&MessageKey) -> Result<Vec<u8>>,
) -> Batch {
    archive_batch_with(store, acc, keys, to_account, fetch, &|needed| room_for(store, needed))
}

/// [`archive_batch`], `room` saying whether the disk holds a batch of that many bytes.
fn archive_batch_with(
    store: &Store,
    acc: &AccountConfig,
    mut keys: Vec<MessageKey>,
    to_account: &str,
    fetch: &mut dyn FnMut(&MessageKey) -> Result<Vec<u8>>,
    room: &dyn Fn(u64) -> bool,
) -> Batch {
    if acc.pop.is_some() || acc.is_api() || acc.is_local() {
        return Batch::keep_all(keys, "only IMAP mail can be archived on this computer");
    }
    let archives = store.load_accounts().ok().and_then(|l| l.into_iter().find(|a| a.id == to_account));
    if !archives.is_some_and(|a| a.role == Some(AccountRole::Archives) && a.is_local()) {
        return Batch::keep_all(keys, "there is no Archives account to archive into");
    }
    let rest = if keys.len() > MAX_BATCH { keys.split_off(MAX_BATCH) } else { Vec::new() };
    let mut out = Batch::keep_all(rest, "too many at once");
    // The cache's own record of each message: its marks, its arrival, its size.
    let mut lists: HashMap<String, FolderState> = HashMap::new();
    for k in &keys {
        if !lists.contains_key(&k.folder) {
            lists.insert(k.folder.clone(), store.load_folder(&acc.id, &k.folder).unwrap_or_default());
        }
    }
    let known = |k: &MessageKey| lists.get(&k.folder).filter(|st| st.validity == k.validity).and_then(|st| st.find(k.uid));
    let needed: u64 = keys.iter().filter_map(|k| known(k).map(|m| u64::from(m.size))).fold(0, u64::saturating_add);
    if !room(needed) {
        let mb = needed.div_ceil(1 << 20).saturating_add(SPARE >> 20);
        let batch = Batch::keep_all(keys, &format!("not enough room on the disk ({mb} MB needed)"));
        out.kept.extend(batch.kept);
        return out;
    }
    let delimiters: HashMap<String, Option<char>> =
        store.load_folders(&acc.id).unwrap_or_default().into_iter().map(|f| (f.path, f.delimiter)).collect();
    let name = if acc.name.trim().is_empty() { acc.email.as_str() } else { acc.name.as_str() };
    let mut stopped: Option<String> = None;
    for key in keys {
        if let Some(why) = &stopped {
            out.kept.push((key, why.clone()));
            continue;
        }
        let Some(m) = known(&key).filter(|_| key.account == acc.id) else {
            out.kept.push((key, "the message is no longer in its folder".into()));
            continue;
        };
        let (mut flags, received) = (m.flags.clone(), if m.received > 0 { m.received } else { m.date });
        flags.deleted = false;
        let raw = match store.load_body(&key) {
            Ok(Some(raw)) => raw,
            Ok(None) => match fetch(&key) {
                Ok(raw) => raw,
                // Gone from the server, or not to be had: this one stays; the rest go on.
                Err(NetError::No(why)) => {
                    out.kept.push((key, why));
                    continue;
                }
                // The connection, the sign-in: none of the rest would be had either.
                Err(e) => {
                    stopped = Some(e.to_string());
                    out.kept.push((key, e.to_string()));
                    continue;
                }
            },
            Err(e) => {
                stopped = Some(e.to_string());
                out.kept.push((key, e.to_string()));
                continue;
            }
        };
        let source_delimiter = delimiters.get(&key.folder).copied().flatten();
        let folder = archive_folder(name, &quatrelle_model::utf7::decode(&key.folder), source_delimiter, DELIMITER);
        match copy_checked(store, to_account, &folder, &raw, flags, received) {
            Ok(copy) => {
                out.folders.insert(folder);
                out.moved.push((key, copy));
            }
            Err(e) => {
                stopped = Some(e.to_string());
                out.kept.push((key, e.to_string()));
            }
        }
    }
    out
}

/// What the batch leaves to do on the server, and the event that tells of it. The deletions of
/// the messages copied go in the account's journal (and out of its cache) now; `Gmail` keeps
/// every message in All Mail unless it is first moved to its Trash, so without a Trash folder
/// nothing is deleted there: the messages are kept, with the reason.
pub(crate) fn settle(store: &Store, sink: &Sink, acc: &AccountConfig, to_account: &str, mut batch: Batch, no_trash_on_gmail: bool) -> Event {
    if !batch.folders.is_empty() {
        sink.emit(Event::Folders { account: to_account.to_string(), folders: store.load_folders(to_account).unwrap_or_default() });
        for folder in &batch.folders {
            sink.emit(Event::FolderChanged { account: to_account.to_string(), folder: folder.clone() });
        }
    }
    if no_trash_on_gmail && !batch.moved.is_empty() {
        let why =
            "this Gmail account has no Trash folder here, so its mail can't be deleted for good; it stays on the server and is also in Archives";
        batch.kept.extend(batch.moved.drain(..).map(|(k, _)| (k, why.to_string())));
    }
    if !batch.moved.is_empty() {
        let keys: Vec<MessageKey> = batch.moved.iter().map(|(k, _)| k.clone()).collect();
        make_edit(store, sink, &acc.id, &Op::Expunge { keys, tidy: false });
    }
    Event::Archived { account: acc.id.clone(), moved: batch.moved, kept: batch.kept }
}

/// Whether `folders` (or the cache's list of them) has a Trash.
fn has_trash(store: &Store, acc: &AccountConfig, folders: &[quatrelle_model::Folder]) -> bool {
    let listed;
    let folders = if folders.is_empty() {
        listed = store.load_folders(&acc.id).unwrap_or_default();
        &listed
    } else {
        folders
    };
    folders.iter().any(|f| f.role == FolderRole::Trash)
}

impl Worker {
    /// [`Job::Archive`]: copy, check, then delete on the server through the journal.
    pub(super) fn archive(&mut self, keys: Vec<MessageKey>, to_account: String) {
        let (store, acc) = (self.store.clone(), self.acc.clone());
        let mut fetch = |key: &MessageKey| -> Result<Vec<u8>> {
            if self.conn.is_none() {
                self.connect()?;
            }
            crate::sync::fetch_source(self.client()?, key)
        };
        let batch = archive_batch(&store, &acc, keys, &to_account, &mut fetch);
        let gmail = match &self.conn {
            Some(Conn::Imap(c)) => c.has("X-GM-EXT-1"),
            _ => google_mailbox(&acc.imap.host, &acc.smtp.host),
        };
        let no_trash = gmail && !has_trash(&store, &acc, &self.folders);
        let done = settle(&store, &self.sink, &acc, &to_account, batch, no_trash);
        if self.conn.is_some() {
            self.replay();
        }
        self.sink.emit(done);
        self.report_quota();
    }
}

/// [`Job::Archive`] for an account whose worker isn't running (working offline): the messages
/// the cache keeps whole are copied, the deletions wait in the journal for the next connection,
/// and the rest are kept. On the pool's edit-writing thread, after the changes before.
pub(crate) fn archive_here(store: &Store, sink: &Sink, acc: &AccountConfig, keys: Vec<MessageKey>, to_account: &str) {
    let mut fetch = |_: &MessageKey| -> Result<Vec<u8>> { Err(NetError::No("it isn't kept on this computer and the account is offline".into())) };
    let batch = archive_batch(store, acc, keys, to_account, &mut fetch);
    let no_trash = google_mailbox(&acc.imap.host, &acc.smtp.host) && !has_trash(store, acc, &[]);
    let done = settle(store, sink, acc, to_account, batch, no_trash);
    sink.emit(done);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_disk_needs_the_batch_and_a_hundred_megabytes() {
        assert!(has_room(300 << 20, 200 << 20));
        assert!(!has_room(300 << 20, 201 << 20));
        assert!(!has_room(0, 0));
        assert!(!has_room(1 << 40, u64::MAX), "no overflow");
        assert!(!has_room(1 << 40, u64::MAX / 2));
    }

    #[test]
    fn not_enough_disk_room_archives_nothing() {
        let dir = std::env::temp_dir().join(format!("quatrelle-archive-room-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::open(&dir).unwrap();
        assert!(room_for(&store, 1), "a few bytes fit");
        assert!(!room_for(&store, u64::MAX / 2), "no disk holds that");
        let archives = AccountConfig { id: "a9".into(), role: Some(AccountRole::Archives), ..Default::default() };
        store.save_accounts(std::slice::from_ref(&archives)).unwrap();
        let imap = quatrelle_model::ServerConfig { host: "imap.example".into(), ..Default::default() };
        let acc = AccountConfig { id: "a1".into(), imap, ..Default::default() };
        let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
        let mut st = FolderState { validity: 1, ..Default::default() };
        st.messages.push(quatrelle_model::Summary { key: key.clone(), size: 1000, ..Default::default() });
        store.save_folder("a1", "INBOX", &st).unwrap();
        let mut fetched = 0;
        let mut fetch = |_: &MessageKey| -> Result<Vec<u8>> {
            fetched += 1;
            Ok(Vec::new())
        };
        // A disk with no room: the whole batch is kept, with how much is needed; nothing is fetched or written.
        let batch = archive_batch_with(&store, &acc, vec![key.clone()], "a9", &mut fetch, &|n| {
            assert_eq!(n, 1000);
            false
        });
        assert!(batch.moved.is_empty() && batch.folders.is_empty());
        assert_eq!(batch.kept.len(), 1);
        assert!(batch.kept[0].1.contains("not enough room") && batch.kept[0].1.contains("101 MB"), "{}", batch.kept[0].1);
        assert_eq!(fetched, 0);
        assert!(store.load_folders("a9").unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
