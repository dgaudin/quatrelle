// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The local copy of the user's chats, as Outlook's Cached Exchange Mode keeps mail: the data
//! file `Teams.quatrelle`, through the store, under the pseudo-account [`TEAMS_ACCOUNT`]. Its keys:
//!
//! * `identity.json`: whose copy it is (organisation and user ids). A file of another identity is
//!   set aside (`Teams-<short hash>.quatrelle.old`), none of its records read;
//! * `listed.json`: the copy's first listing is over (a chat new after it is news);
//! * `chats/<hash>.json`: a chat and how far its copy has come ([`ChatRecord`]);
//! * `msgs/<hash>/<n>.json`: its messages, in pages of at most [`PAGE`] in creation order
//!   (`pages.rs`);
//! * `img/<sha256>`: a picture, by its bytes' hash, and `imguse/<sha256>` its size and last use;
//!   `imgref/<hash>`: which picture a message's hosted image is; `photos/<hash>.json`: someone's
//!   photo, or that they have none (`images.rs`);
//! * `index/<hash>.json`: the chat's words, for the search, kept with its pages; `index/partial`:
//!   the oldest chats' words left the index (`search.rs`).
//!
//! `<hash>` is the first 32 hex digits of the SHA-256 of the chat's id. The file is hostile input
//! as much as a server is: every record carries the id it's for (one under another's key is
//! passed over), and what is read back is held to the model's caps.

use std::path::PathBuf;

use quatrelle_model::pim::teams::{Attachment, Chat, ChatMessage, MAX_ID, MAX_MEMBERS, MAX_PICTURE, Sender, TEAMS_ACCOUNT, one_line};
use quatrelle_store::{DATA_FILE_EXTENSION, Result, Store, StoreError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

mod images;
mod pages;

pub(crate) use pages::PAGE;

/// The most bytes of a record read back (a page, a chat): more is a damaged or hostile file. Above
/// the most a record within the caps takes (5,000 members, or one message's reactions).
const MAX_RECORD: usize = 64 << 20;
/// The longest kept `nextLink` read back.
const MAX_LINK: usize = 8 << 10;
/// The highest page number read back: a page past it is a damaged record.
const MAX_PAGE_KEY: u32 = 1 << 24;
/// The most unread messages kept of a chat (the list shows "99+" past 99 in all).
pub(crate) const MAX_UNREAD: usize = 1_000;
/// The record that says the copy's first listing is over.
const LISTED: &str = "listed.json";

/// Whose copy it is: the organisation's id and the user's object id, as Graph gives them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Identity {
    pub(crate) tenant_id: String,
    pub(crate) object_id: String,
}

impl Identity {
    fn same(&self, other: &Identity) -> bool {
        self.tenant_id.eq_ignore_ascii_case(&other.tenant_id) && self.object_id.eq_ignore_ascii_case(&other.object_id)
    }
}

/// What [`Copy::open`] found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Opened {
    /// No copy yet: an empty one is begun.
    New,
    /// The copy made for this identity.
    Same,
    /// Another identity's copy (or one that doesn't say whose it is) was set aside there, unread;
    /// an empty one is begun.
    SetAside(PathBuf),
}

/// A chat in the copy, and how far its copy has come. Times are milliseconds since 1970.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ChatRecord {
    pub(crate) chat: Chat,
    /// The newest `lastModifiedDateTime` read: the incremental reads go on from there.
    #[serde(default)]
    pub(crate) watermark: Option<i64>,
    /// Where the history read stopped (Graph's `@odata.nextLink`): checked as any link before use.
    #[serde(default)]
    pub(crate) next_link: Option<String>,
    /// The server's time when the chat's copy began (`S`): history is read from before it.
    #[serde(default)]
    pub(crate) history_from: Option<i64>,
    /// The oldest `lastModifiedDateTime` the history has copied: a read started again without its
    /// link goes on from there.
    #[serde(default)]
    pub(crate) history_oldest: Option<i64>,
    /// The first creation time of each page of messages, oldest page first. Only
    /// [`Copy::upsert`] changes them (and `page_keys`): [`Copy::put_chat`] keeps those stored.
    #[serde(default)]
    pub(crate) pages: Vec<i64>,
    /// The number `<n>` each page is kept under (`msgs/<hash>/<n>.json`), in the same order: a
    /// page split in two doesn't renumber the ones after it.
    #[serde(default)]
    pub(crate) page_keys: Vec<u32>,
    /// Its whole history is copied.
    #[serde(default)]
    pub(crate) done: bool,
    /// Reads of it that panicked, one after the other.
    #[serde(default)]
    pub(crate) panics: u8,
    /// Set aside after panicking (not read again until then).
    #[serde(default)]
    pub(crate) set_aside_until: Option<i64>,
    /// A photo Graph said isn't there: not asked again until then.
    #[serde(default)]
    pub(crate) photo_missing_until: Option<i64>,
    /// Its unread messages in the copy (id, creation time): from someone else, after the user
    /// last read it; kept as messages come and as the read state moves, never by reading the
    /// pages again. At most [`MAX_UNREAD`].
    #[serde(default)]
    pub(crate) unread: Vec<(String, i64)>,
    /// The server's time when its messages were last read (an incremental read that ended).
    #[serde(default)]
    pub(crate) read_at: Option<i64>,
}

impl ChatRecord {
    /// A chat new to the copy: nothing of it copied yet.
    pub(crate) fn new(chat: Chat) -> ChatRecord {
        ChatRecord {
            chat,
            watermark: None,
            next_link: None,
            history_from: None,
            history_oldest: None,
            pages: Vec::new(),
            page_keys: Vec::new(),
            done: false,
            panics: 0,
            set_aside_until: None,
            photo_missing_until: None,
            unread: Vec::new(),
            read_at: None,
        }
    }

    /// Held to the caps, as read back from the file. Pages that don't hold together (not as many
    /// numbers as times, out of order, a number twice or past [`MAX_PAGE_KEY`]) can't be found
    /// again: the chat's copy starts again from the server.
    fn held(mut self) -> ChatRecord {
        // As Graph's reader keeps them: ids usable, names and topic on one line.
        let c = &mut self.chat;
        c.topic = c.topic.take().map(|t| one_line(&t)).filter(|t| !t.is_empty());
        c.members.retain(|m| id_ok(&m.user_id));
        c.members.truncate(MAX_MEMBERS);
        for m in &mut c.members {
            m.name = one_line(&m.name);
            m.tenant_id = m.tenant_id.take().filter(|t| id_ok(t));
        }
        if let Some(p) = c.preview.as_mut() {
            hold_sender(&mut p.sender);
            p.text = one_line(&p.text);
        }
        c.tenant_id = c.tenant_id.take().filter(|t| id_ok(t));
        if self.next_link.as_ref().is_some_and(|l| l.len() > MAX_LINK) {
            self.next_link = None;
        }
        self.unread.retain(|(id, _)| id_ok(id));
        self.unread.truncate(MAX_UNREAD);
        let mut keys = self.page_keys.clone();
        keys.sort_unstable();
        let sound = self.pages.len() == self.page_keys.len()
            && self.pages.windows(2).all(|w| w.first() <= w.get(1))
            && keys.windows(2).all(|w| w.first() != w.get(1))
            && keys.last().is_none_or(|k| *k < MAX_PAGE_KEY);
        if !sound {
            log::warn!("Teams copy: a chat's pages don't hold together; its copy starts again");
            self.pages.clear();
            self.page_keys.clear();
            self.watermark = None;
            self.next_link = None;
            self.history_from = None;
            self.history_oldest = None;
            self.done = false;
            self.unread.clear();
        }
        self
    }
}

/// The copy of the user's chats.
pub(crate) struct Copy {
    pub(super) store: Store,
    /// Its image cache's account (`images.rs`).
    images: std::cell::RefCell<images::Images>,
    /// The search's index of it (`search.rs`).
    pub(super) index: std::cell::RefCell<super::search::Index>,
}

/// The first 32 hex digits of the SHA-256 of `bytes`.
fn hash32(bytes: &[u8]) -> String {
    let mut out = hex(&Sha256::digest(bytes));
    out.truncate(32);
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The hash a chat's records are kept under.
pub(crate) fn chat_hash(chat: &str) -> String {
    hash32(chat.as_bytes())
}

fn chat_key(chat: &str) -> String {
    format!("chats/{}.json", chat_hash(chat))
}

/// Where a message's hosted image is said to be: its ids can't hold a NUL (they're refused with
/// control characters), so no two triples give the same input.
pub(crate) fn imgref_key(chat: &str, message: &str, hosted: &str) -> String {
    format!("imgref/{}", hash32(format!("{chat}\0{message}\0{hosted}").as_bytes()))
}

/// An id as the copy keeps one: not empty, at most [`MAX_ID`] characters, no control character.
pub(crate) fn id_ok(id: &str) -> bool {
    !id.is_empty() && id.chars().nth(MAX_ID).is_none() && !id.chars().any(char::is_control)
}

fn refused(what: &str) -> StoreError {
    StoreError::Io { path: TEAMS_ACCOUNT.to_string(), source: std::io::Error::new(std::io::ErrorKind::InvalidInput, what.to_string()) }
}

/// A sender as Graph's reader keeps one: a usable id (else no one), the name on one line.
fn hold_sender(s: &mut Sender) {
    match s {
        Sender::User { id, name, identity_type, tenant_id } if id_ok(id) => {
            *name = one_line(name);
            *identity_type = identity_type.take().filter(|t| id_ok(t));
            *tenant_id = tenant_id.take().filter(|t| id_ok(t));
        }
        Sender::App { id, name } if id_ok(id) => *name = one_line(name),
        Sender::Nobody => {}
        _ => *s = Sender::Nobody,
    }
}

/// Whether a sender is as [`hold_sender`] keeps one.
fn sender_ok(s: &Sender) -> bool {
    match s {
        Sender::User { id, name, .. } | Sender::App { id, name } => id_ok(id) && one_line(name) == *name,
        Sender::Nobody => true,
    }
}

/// The hosted images a message shows or carries.
fn hosted_ids(m: &ChatMessage) -> Vec<String> {
    let mut ids: Vec<String> = m.attachments.iter().filter_map(|a| if let Attachment::Hosted { id } = a { Some(id.clone()) } else { None }).collect();
    ids.extend(m.body.blocks.iter().flat_map(|b| &b.runs).filter_map(|r| r.hosted.clone()));
    ids.sort();
    ids.dedup();
    ids
}

impl Copy {
    /// The copy in `store` for `who`. A copy made for another identity, or one that doesn't say
    /// whose it is but holds chats, is set aside unread (`Teams-<short hash>.quatrelle.old`, beside
    /// it) — or deleted, in a cache that keeps no data files — and an empty one begun.
    pub(crate) fn open(store: Store, who: &Identity) -> Result<(Copy, Opened)> {
        if !id_ok(&who.tenant_id) || !id_ok(&who.object_id) {
            return Err(refused("no identity to keep the copy for"));
        }
        let copy = Copy { store, images: images::images(), index: std::cell::RefCell::new(super::search::Index::new(&who.object_id)) };
        let kept = copy.store.account_file(TEAMS_ACCOUNT, "identity.json")?;
        let whose = match &kept {
            Some(bytes) => match serde_json::from_slice::<Identity>(bytes) {
                Ok(id) if id.same(who) => return Ok((copy, Opened::Same)),
                Ok(id) => Some(hash32(format!("{}/{}", id.tenant_id, id.object_id).as_bytes())),
                // Damaged: whose it is can't be told.
                Err(_) => Some(hash32(bytes)),
            },
            // Nothing says whose it is: kept aside only when it holds chats.
            None if !copy.store.account_files(TEAMS_ACCOUNT, "chats")?.is_empty() => Some(hash32(b"")),
            None => None,
        };
        let opened = match whose {
            Some(h) => {
                let name = format!("Teams-{}.{DATA_FILE_EXTENSION}.old", h.get(..8).unwrap_or(&h));
                match copy.store.set_aside_data_file(TEAMS_ACCOUNT, &name)? {
                    Some(to) => Opened::SetAside(to),
                    None => {
                        copy.store.delete_account(TEAMS_ACCOUNT)?;
                        Opened::New
                    }
                }
            }
            None => {
                // An empty file (one made just now while setting a damaged one aside) or leftovers:
                // nothing worth keeping.
                copy.store.delete_account(TEAMS_ACCOUNT)?;
                Opened::New
            }
        };
        let bytes = serde_json::to_vec(who).map_err(|source| StoreError::Json { path: "identity.json".into(), source })?;
        copy.store.write_account_files(TEAMS_ACCOUNT, &[("identity.json".to_string(), bytes)])?;
        Ok((copy, opened))
    }

    /// Whether the copy's first listing is over: a chat that comes after it is new, its first
    /// message news.
    pub(crate) fn listed(&self) -> bool {
        self.read(LISTED).is_some_and(|b| b == b"true")
    }

    /// The copy's first listing is over.
    pub(crate) fn set_listed(&self) -> Result<()> {
        self.store.write_account_files(TEAMS_ACCOUNT, &[(LISTED.to_string(), b"true".to_vec())])
    }

    /// A record's bytes, when there are some and not too many.
    pub(super) fn read(&self, key: &str) -> Option<Vec<u8>> {
        match self.store.account_file(TEAMS_ACCOUNT, key) {
            Ok(Some(bytes)) if bytes.len() <= MAX_RECORD => Some(bytes),
            Ok(Some(_)) => {
                log::warn!("Teams copy: a record past {MAX_RECORD} bytes passed over");
                None
            }
            Ok(None) => None,
            Err(e) => {
                // Not the path: it names the chat's hash.
                let why = match &e {
                    StoreError::Io { source, .. } => format!("{:?}", source.kind()),
                    StoreError::Json { .. } => "damaged".to_string(),
                };
                log::warn!("Teams copy: a record couldn't be read: {why}");
                None
            }
        }
    }

    /// A chat's record, when the copy has it.
    pub(crate) fn chat(&self, id: &str) -> Option<ChatRecord> {
        if !id_ok(id) {
            return None;
        }
        self.record_at(&chat_key(id), id)
    }

    /// The record at `key`, when it's `id`'s.
    fn record_at(&self, key: &str, id: &str) -> Option<ChatRecord> {
        let r: ChatRecord = serde_json::from_slice(&self.read(key)?).ok()?;
        (r.chat.id == id).then(|| r.held())
    }

    /// A chat's record written. Its pages are the copy's own: those stored are kept, whatever
    /// `r` holds (a record read before [`Copy::upsert`] wrote doesn't take them back).
    pub(crate) fn put_chat(&self, r: &ChatRecord) -> Result<()> {
        self.put_chats(std::slice::from_ref(r))
    }

    /// Chats' records written as [`Copy::put_chat`] does, in one go (a page of the list).
    pub(crate) fn put_chats(&self, records: &[ChatRecord]) -> Result<()> {
        let mut files = Vec::with_capacity(records.len());
        for r in records {
            if !id_ok(&r.chat.id) {
                return Err(refused("a chat without a usable id"));
            }
            let mut r = r.clone();
            r.chat.members.truncate(MAX_MEMBERS);
            let (pages, keys) = self.chat(&r.chat.id).map(|old| (old.pages, old.page_keys)).unwrap_or_default();
            r.pages = pages;
            r.page_keys = keys;
            files.push(self.chat_file(&r)?);
        }
        if files.is_empty() {
            return Ok(());
        }
        self.store.write_account_files(TEAMS_ACCOUNT, &files)?;
        // Their names, as the search finds them.
        let mut index = self.index.borrow_mut();
        records.iter().for_each(|r| index.name(&r.chat));
        Ok(())
    }

    fn chat_file(&self, r: &ChatRecord) -> Result<(String, Vec<u8>)> {
        let bytes = serde_json::to_vec(r).map_err(|source| StoreError::Json { path: "chats".into(), source })?;
        Ok((chat_key(&r.chat.id), bytes))
    }

    /// Every chat in the copy, at most [`quatrelle_model::pim::teams::MAX_CHATS`].
    pub(crate) fn chats(&self) -> Vec<ChatRecord> {
        let mut names = match self.store.account_files(TEAMS_ACCOUNT, "chats") {
            Ok(names) => names,
            Err(_) => {
                log::warn!("Teams copy: its chats couldn't be listed");
                return Vec::new();
            }
        };
        names.sort();
        names
            .iter()
            .filter_map(|n| n.strip_suffix(".json").filter(|h| h.len() == 32 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))))
            .filter_map(|h| {
                let r: ChatRecord = serde_json::from_slice(&self.read(&format!("chats/{h}.json"))?).ok()?;
                (id_ok(&r.chat.id) && chat_hash(&r.chat.id) == h).then(|| r.held())
            })
            .take(quatrelle_model::pim::teams::MAX_CHATS)
            .collect()
    }

    /// The chats in the copy whose hash ([`chat_hash`]) isn't among `listed`: those the server's
    /// list no longer has. At most [`quatrelle_model::pim::teams::MAX_CHATS`] are looked at.
    pub(crate) fn absent(&self, listed: &std::collections::HashSet<String>) -> Vec<ChatRecord> {
        let names = match self.store.account_files(TEAMS_ACCOUNT, "chats") {
            Ok(names) => names,
            Err(_) => {
                log::warn!("Teams copy: its chats couldn't be listed");
                return Vec::new();
            }
        };
        names
            .iter()
            .take(quatrelle_model::pim::teams::MAX_CHATS)
            .filter_map(|n| n.strip_suffix(".json").filter(|h| h.len() == 32 && !listed.contains(*h)))
            .filter_map(|h| {
                let r: ChatRecord = serde_json::from_slice(&self.read(&format!("chats/{h}.json"))?).ok()?;
                (id_ok(&r.chat.id) && chat_hash(&r.chat.id) == h).then(|| r.held())
            })
            .collect()
    }

    /// The copy deleted: its data file, and every copy of it set aside as damaged.
    #[cfg(test)]
    pub(crate) fn delete(self) -> Result<()> {
        Copy::delete_in(&self.store)
    }

    /// The copy in `store` deleted, open or not (signing out before anything was
    /// read): the data file, then every copy of it the store set aside as damaged, whichever
    /// name the file had then. A copy set aside for the user (`.old`) stays.
    pub(crate) fn delete_in(store: &Store) -> Result<()> {
        let beside = store.data_file(TEAMS_ACCOUNT).map(|f| f.with_extension(format!("{DATA_FILE_EXTENSION}.damaged")));
        store.delete_account(TEAMS_ACCOUNT)?;
        let mut failed = None;
        for d in beside.into_iter().chain(damaged_copies(store)) {
            match std::fs::remove_file(&d) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => failed = Some(StoreError::Io { path: d.display().to_string(), source: e }),
                _ => {}
            }
        }
        failed.map_or(Ok(()), Err)
    }

    /// [`Job::DeleteSetAside`](quatrelle_model::pim::teams::Job::DeleteSetAside): the copy set
    /// aside for the user at `path` deleted, as they chose. Only a file of the data files' folder
    /// named as [`Copy::open`] names one ([`set_aside_name`]): any other path (another folder, a
    /// `..`, another name) is refused. `false`: refused, or already gone.
    pub(crate) fn delete_set_aside(store: &Store, path: &str) -> Result<bool> {
        let path = std::path::Path::new(path);
        let Some(dir) = store.data_files_dir() else { return Ok(false) };
        let named = path.file_name().and_then(|n| n.to_str()).is_some_and(set_aside_name);
        if !named || path.parent() != Some(dir.as_path()) {
            return Ok(false);
        }
        match std::fs::remove_file(path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(StoreError::Io { path: path.display().to_string(), source: e }),
        }
    }
}

/// Whether `name` is one [`Copy::open`] sets a copy aside under: `Teams-<8 hex digits>.quatrelle.old`,
/// or with ` (2)` … ` (99)` after the digits when that name was taken.
fn set_aside_name(name: &str) -> bool {
    let tail = format!(".{DATA_FILE_EXTENSION}.old");
    let Some(rest) = name.strip_prefix(TEAMS_ACCOUNT).and_then(|n| n.strip_prefix('-')).and_then(|n| n.strip_suffix(tail.as_str())) else {
        return false;
    };
    let (Some(hash), Some(n)) = (rest.get(..8), rest.get(8..)) else { return false };
    hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        && (n.is_empty()
            || n.strip_prefix(" (")
                .and_then(|n| n.strip_suffix(')'))
                .is_some_and(|n| (1..=2).contains(&n.len()) && n.bytes().all(|b| b.is_ascii_digit())))
}

/// The most entries looked at in the folder of the data files.
const MAX_DIR_ENTRIES: usize = 10_000;

/// The copies of the Teams file set aside as damaged in the folder of the data files: the file
/// is `Teams.quatrelle`, or `Teams (2).quatrelle`… when that name was taken, and a damaged one
/// gets `.damaged` after its name. One whose data file is still there isn't the copy's (the copy's
/// own is deleted by now): it's of an account that has that name.
fn damaged_copies(store: &Store) -> Vec<PathBuf> {
    let Some(dir) = store.data_files_dir() else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
    let tail = format!(".{DATA_FILE_EXTENSION}.damaged");
    let numbered = |stem: &str| {
        stem.strip_prefix(TEAMS_ACCOUNT).is_some_and(|n| {
            n.is_empty()
                || n.strip_prefix(" (")
                    .and_then(|n| n.strip_suffix(')'))
                    .is_some_and(|n| !n.is_empty() && n.len() <= 9 && n.bytes().all(|b| b.is_ascii_digit()))
        })
    };
    entries
        .take(MAX_DIR_ENTRIES)
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|name| name.strip_suffix(&tail).is_some_and(numbered))
        .filter(|name| !dir.join(name.strip_suffix(".damaged").unwrap_or(name)).exists())
        .map(|name| dir.join(name))
        .collect()
}

/// A hostile copy (the `teams_store` fuzz target): a memory store filled from `data`, then opened
/// for `t-1`/`u-1`, listed, read in pages and written to, its index read back and searched, its
/// pictures too (the chat id taken as a user's for a photo). `data` is a chat id, then pairs of a
/// key under `Teams/` and its bytes, then (when they're odd) a message's JSON, split at NUL bytes;
/// a `=` in a key is the chat's hash (`chats/=.json`, `msgs/=/0.json`). `Err` names a cap that
/// what was read back breaks.
#[doc(hidden)]
pub fn fuzz_teams_copy(data: &[u8]) -> std::result::Result<(), String> {
    let mut parts = data.split(|b| *b == 0);
    let chat = String::from_utf8_lossy(parts.next().unwrap_or_default()).into_owned();
    let rest: Vec<&[u8]> = parts.collect();
    let (pairs, message) = match rest.split_last() {
        Some((last, pairs)) if rest.len() % 2 == 1 => (pairs, Some(*last)),
        _ => (rest.as_slice(), None),
    };
    let hash = chat_hash(&chat);
    let files = pairs
        .chunks(2)
        .filter_map(|kv| Some((format!("{TEAMS_ACCOUNT}/{}", String::from_utf8_lossy(kv.first()?).replace('=', &hash)), kv.get(1)?.to_vec())))
        .collect();
    let who = Identity { tenant_id: "t-1".into(), object_id: "u-1".into() };
    let Ok((copy, _)) = Copy::open(Store::memory_from(files), &who) else { return Ok(()) };
    for r in copy.chats().iter().chain(copy.chat(&chat).as_ref()) {
        let c = &r.chat;
        let lines = c.topic.as_deref().is_none_or(|t| one_line(t) == t)
            && c.members.iter().all(|m| id_ok(&m.user_id) && one_line(&m.name) == m.name)
            && c.preview.as_ref().is_none_or(|p| sender_ok(&p.sender) && one_line(&p.text) == p.text);
        let unread = r.unread.len() <= MAX_UNREAD && r.unread.iter().all(|(id, _)| id_ok(id));
        let fits = id_ok(&c.id) && c.members.len() <= MAX_MEMBERS && r.pages.len() == r.page_keys.len() && lines && unread;
        if !fits {
            return Err(format!("a chat read back past its caps: {} members, {} pages", r.chat.members.len(), r.pages.len()));
        }
    }
    let check = |what: &str, msgs: &[ChatMessage], most: usize| -> std::result::Result<(), String> {
        if msgs.len() > most {
            return Err(format!("{what}: {} messages", msgs.len()));
        }
        msgs.iter().try_for_each(|m| pages::within_caps(m, &chat).map_err(|e| format!("{what}: {e}")))
    };
    let window = copy.last_pages(&chat, 2);
    check("the last pages", &window, 2 * PAGE)?;
    for at in window.first().map(|m| m.created).into_iter().chain([i64::MAX, i64::MIN]) {
        check("the page before", &copy.before(&chat, at), PAGE)?;
    }
    let mut m = message
        .and_then(|b| serde_json::from_slice::<ChatMessage>(b).ok())
        .unwrap_or_else(|| ChatMessage { id: "1".into(), ..ChatMessage::default() });
    m.chat = chat.clone();
    let changed = copy.upsert(&chat, std::slice::from_ref(&m)).unwrap_or_default();
    check("what changed", &changed, 1)?;
    // The search's index read back (a chat a slice) and searched: no hit past the bound, none
    // under an id the copy wouldn't keep.
    for _ in 0..=quatrelle_model::pim::teams::MAX_CHATS {
        if !copy.index_step() {
            break;
        }
    }
    let found = copy.found("message");
    if found.indexing || found.hits.len() > super::search::MAX_HITS || found.hits.iter().any(|(c, m)| !id_ok(c) || !(m.is_empty() || id_ok(m))) {
        return Err(format!("a search of the copy read back: {} hits, indexing {}", found.hits.len(), found.indexing));
    }
    check("the last pages, once written", &copy.last_pages(&chat, 2), 2 * PAGE)?;
    // The image cache read back (its uses, a photo's record), then a picture kept past them.
    let (photo, _) = copy.photo(&chat);
    if photo.is_some_and(|b| b.len() > MAX_PICTURE) {
        return Err("a photo read back past its cap".into());
    }
    let _ = copy.put_hosted(&chat, &m.id, "h", b"a picture");
    if copy.hosted(&chat, &m.id, "h").is_some_and(|b| b != b"a picture") {
        return Err("a picture read back isn't the one kept".into());
    }
    Ok(())
}
