// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The local copy of the user's chats, as Outlook's Cached Exchange Mode keeps mail: the data
//! file `Teams.quatrelle`, through the store, under the pseudo-account [`TEAMS_ACCOUNT`]. Its keys:
//!
//! * `identity.json`: whose copy it is (organisation and user ids). A file of another identity is
//!   set aside (`Teams-<short hash>.quatrelle.old`), none of its records read;
//! * `chats/<hash>.json`: a chat and how far its copy has come ([`ChatRecord`]);
//! * `msgs/<hash>/<n>.json`: its messages, in pages of at most [`PAGE`] in creation order
//!   (`pages.rs`);
//! * `img/<sha256>`: a picture, by its bytes' hash; `imgref/<hash>`: which picture a message's
//!   hosted image is;
//! * `index/<hash>.json`: the chat's words, for the search.
//!
//! `<hash>` is the first 32 hex digits of the SHA-256 of the chat's id. The file is hostile input
//! as much as a server is: every record carries the id it's for (one under another's key is
//! passed over), and what is read back is held to the model's caps.

use std::path::PathBuf;

use quatrelle_model::pim::teams::{Attachment, Chat, ChatMessage, MAX_ID, MAX_MEMBERS, MAX_PICTURE, TEAMS_ACCOUNT};
use quatrelle_store::{DATA_FILE_EXTENSION, Result, Store, StoreError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

mod pages;

pub(crate) use pages::PAGE;

/// The most bytes of a record read back (a page, a chat): more is a damaged or hostile file. Above
/// the most a record within the caps takes (5,000 members, or one message's reactions).
const MAX_RECORD: usize = 64 << 20;
/// The longest kept `nextLink` read back.
const MAX_LINK: usize = 8 << 10;
/// The highest page number read back: a page past it is a damaged record.
const MAX_PAGE_KEY: u32 = 1 << 24;

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
}

impl ChatRecord {
    /// A chat new to the copy: nothing of it copied yet.
    pub(crate) fn new(chat: Chat) -> ChatRecord {
        ChatRecord {
            chat,
            watermark: None,
            next_link: None,
            history_from: None,
            pages: Vec::new(),
            page_keys: Vec::new(),
            done: false,
            panics: 0,
            set_aside_until: None,
            photo_missing_until: None,
        }
    }

    /// Held to the caps, as read back from the file. Pages that don't hold together (not as many
    /// numbers as times, out of order, a number twice or past [`MAX_PAGE_KEY`]) can't be found
    /// again: the chat's copy starts again from the server.
    fn held(mut self) -> ChatRecord {
        self.chat.members.truncate(MAX_MEMBERS);
        if self.next_link.as_ref().is_some_and(|l| l.len() > MAX_LINK) {
            self.next_link = None;
        }
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
            self.done = false;
        }
        self
    }
}

/// The copy of the user's chats.
pub(crate) struct Copy {
    store: Store,
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
        let copy = Copy { store };
        let kept = copy.store.account_file(TEAMS_ACCOUNT, "identity.json")?;
        let whose = match &kept {
            Some(bytes) => match serde_json::from_slice::<Identity>(bytes) {
                Ok(id) if id.same(who) => return Ok((copy, Opened::Same)),
                Ok(id) => Some(hash32(format!("{}/{}", id.tenant_id, id.object_id).as_bytes())),
                // Damaged: whose it is can't be told.
                Err(_) => Some(hash32(bytes)),
            },
            // Nothing says whose it is: kept aside only when it holds chats.
            None if !copy.store.account_files(TEAMS_ACCOUNT, "chats").is_empty() => Some(hash32(b"")),
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

    /// A record's bytes, when there are some and not too many.
    fn read(&self, key: &str) -> Option<Vec<u8>> {
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
        if !id_ok(&r.chat.id) {
            return Err(refused("a chat without a usable id"));
        }
        let mut r = r.clone();
        r.chat.members.truncate(MAX_MEMBERS);
        let (pages, keys) = self.chat(&r.chat.id).map(|old| (old.pages, old.page_keys)).unwrap_or_default();
        r.pages = pages;
        r.page_keys = keys;
        self.write_chat(&r)
    }

    /// A chat's record written as it is.
    fn write_chat(&self, r: &ChatRecord) -> Result<()> {
        self.store.write_account_files(TEAMS_ACCOUNT, &[self.chat_file(r)?])
    }

    fn chat_file(&self, r: &ChatRecord) -> Result<(String, Vec<u8>)> {
        let bytes = serde_json::to_vec(r).map_err(|source| StoreError::Json { path: "chats".into(), source })?;
        Ok((chat_key(&r.chat.id), bytes))
    }

    /// Every chat in the copy, at most [`quatrelle_model::pim::teams::MAX_CHATS`].
    pub(crate) fn chats(&self) -> Vec<ChatRecord> {
        let mut names = self.store.account_files(TEAMS_ACCOUNT, "chats");
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

    /// A message's hosted image kept: the picture under its bytes' hash, and where to find it.
    pub(crate) fn put_hosted(&self, chat: &str, message: &str, hosted: &str, bytes: &[u8]) -> Result<()> {
        if bytes.len() > MAX_PICTURE || ![chat, message, hosted].into_iter().all(id_ok) {
            return Err(refused("a picture too big, or without usable ids"));
        }
        let sum = hex(&Sha256::digest(bytes));
        self.store
            .write_account_files(TEAMS_ACCOUNT, &[(format!("img/{sum}"), bytes.to_vec()), (imgref_key(chat, message, hosted), sum.into_bytes())])
    }

    /// A message's hosted image, when it's kept: only bytes that are the ones it was kept as.
    pub(crate) fn hosted(&self, chat: &str, message: &str, hosted: &str) -> Option<Vec<u8>> {
        if ![chat, message, hosted].into_iter().all(id_ok) {
            return None;
        }
        let sum = self.store.account_file(TEAMS_ACCOUNT, &imgref_key(chat, message, hosted)).ok()??;
        let sum = String::from_utf8(sum).ok().filter(|s| s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))?;
        let bytes = self.store.account_file(TEAMS_ACCOUNT, &format!("img/{sum}")).ok()??;
        (bytes.len() <= MAX_PICTURE && hex(&Sha256::digest(&bytes)) == sum).then_some(bytes)
    }

    /// A message's hosted image gone from the copy: where it was said to be, and the picture. One
    /// kept for another message with the same bytes goes too (it's fetched again if wanted): what
    /// a deleted message showed doesn't stay.
    fn forget_hosted(&self, chat: &str, message: &str, hosted: &str) -> Result<()> {
        let at = imgref_key(chat, message, hosted);
        if let Some(sum) = self.store.account_file(TEAMS_ACCOUNT, &at)?
            && let Ok(sum) = String::from_utf8(sum)
            && sum.len() == 64
            && sum.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        {
            self.store.remove_account_files(TEAMS_ACCOUNT, &format!("img/{sum}"))?;
        }
        self.store.remove_account_files(TEAMS_ACCOUNT, &at)
    }

    /// The copy deleted: its data file, and one set aside as damaged.
    pub(crate) fn delete(self) -> Result<()> {
        let damaged = self.store.data_file(TEAMS_ACCOUNT).map(|f| f.with_extension(format!("{DATA_FILE_EXTENSION}.damaged")));
        self.store.delete_account(TEAMS_ACCOUNT)?;
        if let Some(d) = damaged {
            match std::fs::remove_file(&d) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    return Err(StoreError::Io { path: d.display().to_string(), source: e });
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// A hostile copy (the `teams_store` fuzz target): a memory store filled from `data`, then opened
/// for `t-1`/`u-1`, listed, read in pages and written to. `data` is a chat id, then pairs of a
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
        let fits = id_ok(&r.chat.id) && r.chat.members.len() <= MAX_MEMBERS && r.pages.len() == r.page_keys.len();
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
    check("the last pages, once written", &copy.last_pages(&chat, 2), 2 * PAGE)?;
    Ok(())
}
