// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The copy's pictures: messages' hosted images and people's photos, each kept once under its
//! bytes' hash (`img/<sha256>`) and pointed at from where it's shown — `imgref/<hash>` for a
//! message's hosted image, `photos/<hash>.json` for someone's photo (which says too until when
//! Graph's word that they have none holds).
//!
//! An image cache of at most [`IMAGE_CACHE`] bytes and [`MAX_IMAGES`] pictures, counted on their
//! sizes: `imguse/<sha256>` keeps a picture's size and when it was last used (a number that only
//! grows). Past either bound, the least recently used go first; a place still pointing at one gone
//! finds nothing, and the picture is fetched again when it's next shown. The uses of pictures
//! shown are written with the next picture kept.
//!
//! What is read back is hostile as the rest of the copy: a picture is served only when its bytes
//! hash to its key and are within [`MAX_PICTURE`], a size is held to that too, and a photo's record
//! carries the id of whose it is.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use super::*;

/// The most bytes of pictures the cache keeps.
pub(crate) const IMAGE_CACHE: u64 = 1 << 30;
/// The most pictures it keeps, whatever their sizes.
pub(crate) const MAX_IMAGES: usize = 50_000;

/// A picture's place in the cache.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct Use {
    size: u64,
    /// When it was last kept or shown: the cache's clock then.
    used: u64,
}

/// The cache's account of its pictures, in memory: read from the copy at the first picture, kept
/// as pictures come and go.
#[derive(Debug)]
pub(super) struct Images {
    loaded: bool,
    entries: HashMap<String, Use>,
    total: u64,
    clock: u64,
    /// Pictures shown since the last write: their uses go with the next picture kept.
    touched: HashSet<String>,
    cap: u64,
    most: usize,
}

impl Default for Images {
    fn default() -> Images {
        Images { loaded: false, entries: HashMap::new(), total: 0, clock: 0, touched: HashSet::new(), cap: IMAGE_CACHE, most: MAX_IMAGES }
    }
}

impl Images {
    fn tick(&mut self) -> u64 {
        let now = self.clock;
        self.clock = self.clock.saturating_add(1);
        now
    }

    fn add(&mut self, sum: String, entry: Use) {
        if let Some(old) = self.entries.insert(sum, entry) {
            self.total = self.total.saturating_sub(old.size);
        }
        self.total = self.total.saturating_add(entry.size);
    }

    fn forget(&mut self, sum: &str) {
        if let Some(old) = self.entries.remove(sum) {
            self.total = self.total.saturating_sub(old.size);
        }
        self.touched.remove(sum);
    }

    /// A picture shown: now the most recently used (one found without its use is counted now).
    fn touch(&mut self, sum: &str, size: u64) {
        let used = self.tick();
        let size = self.entries.get(sum).map_or(size, |e| e.size);
        self.add(sum.to_string(), Use { size, used });
        self.touched.insert(sum.to_string());
    }

    /// The least recently used pictures that go for one of `size` bytes more (`new`: one picture
    /// more) to fit, `keep` apart.
    fn over(&self, keep: &str, size: u64, new: bool) -> Vec<String> {
        let mut total = self.total.saturating_add(size);
        let mut count = self.entries.len().saturating_add(usize::from(new));
        if total <= self.cap && count <= self.most {
            return Vec::new();
        }
        let mut by_use: Vec<(&String, &Use)> = self.entries.iter().filter(|(s, _)| s.as_str() != keep).collect();
        by_use.sort_unstable_by_key(|(_, u)| u.used);
        let mut gone = Vec::new();
        for (sum, u) in by_use {
            if total <= self.cap && count <= self.most {
                break;
            }
            total = total.saturating_sub(u.size);
            count = count.saturating_sub(1);
            gone.push(sum.clone());
        }
        gone
    }
}

/// Someone's photo, as the copy keeps it.
#[derive(Serialize, Deserialize)]
struct PhotoRecord {
    user: String,
    /// The picture's hash, when they have one.
    #[serde(default)]
    sum: Option<String>,
    /// Graph said they have none: not asked again before then (milliseconds since 1970, on
    /// Graph's clock).
    #[serde(default)]
    photo_missing_until: Option<i64>,
}

fn img_key(sum: &str) -> String {
    format!("img/{sum}")
}

fn use_key(sum: &str) -> String {
    format!("imguse/{sum}")
}

fn photo_key(user: &str) -> String {
    format!("photos/{}.json", hash32(user.as_bytes()))
}

/// Whether `s` is a picture's hash as the cache names them: 64 lowercase hex digits.
fn is_sum(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn size_of(bytes: &[u8]) -> u64 {
    u64::try_from(bytes.len()).unwrap_or(u64::MAX)
}

fn json<T: Serialize>(v: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(v).map_err(|source| StoreError::Json { path: "img".into(), source })
}

/// The most use records read when the cache is first looked at: past it, a damaged file.
const MAX_USES_READ: usize = 2 * MAX_IMAGES;

impl Copy {
    /// A message's hosted image kept: the picture under its bytes' hash, and where to find it.
    pub(crate) fn put_hosted(&self, chat: &str, message: &str, hosted: &str, bytes: &[u8]) -> Result<()> {
        if ![chat, message, hosted].into_iter().all(id_ok) {
            return Err(refused("a picture without usable ids"));
        }
        self.put_picture(imgref_key(chat, message, hosted), |sum| Ok(sum.as_bytes().to_vec()), bytes)
    }

    /// A message's hosted image, when it's kept: only bytes that are the ones it was kept as.
    pub(crate) fn hosted(&self, chat: &str, message: &str, hosted: &str) -> Option<Vec<u8>> {
        if ![chat, message, hosted].into_iter().all(id_ok) {
            return None;
        }
        let sum = self.store.account_file(TEAMS_ACCOUNT, &imgref_key(chat, message, hosted)).ok()??;
        let sum = String::from_utf8(sum).ok().filter(|s| is_sum(s))?;
        self.picture(&sum)
    }

    /// A message's hosted image gone from the copy: where it was said to be, and the picture. One
    /// kept for another message with the same bytes goes too (it's fetched again if wanted): what
    /// a deleted message showed doesn't stay.
    pub(super) fn forget_hosted(&self, chat: &str, message: &str, hosted: &str) -> Result<()> {
        let at = imgref_key(chat, message, hosted);
        if let Some(sum) = self.store.account_file(TEAMS_ACCOUNT, &at)?
            && let Ok(sum) = String::from_utf8(sum)
            && is_sum(&sum)
        {
            self.store.remove_account_files(TEAMS_ACCOUNT, &img_key(&sum))?;
            self.store.remove_account_files(TEAMS_ACCOUNT, &use_key(&sum))?;
            if let Ok(mut images) = self.images.try_borrow_mut() {
                images.forget(&sum);
            }
        }
        self.store.remove_account_files(TEAMS_ACCOUNT, &at)
    }

    /// Someone's photo as the copy has it: the picture, when it's kept; and until when Graph's
    /// word that they have none holds (milliseconds since 1970).
    pub(crate) fn photo(&self, user: &str) -> (Option<Vec<u8>>, Option<i64>) {
        if !id_ok(user) {
            return (None, None);
        }
        let record = self.read(&photo_key(user)).and_then(|b| serde_json::from_slice::<PhotoRecord>(&b).ok()).filter(|r| r.user == user);
        let Some(r) = record else { return (None, None) };
        let bytes = r.sum.as_deref().filter(|s| is_sum(s)).and_then(|s| self.picture(s));
        (bytes, r.photo_missing_until)
    }

    /// Someone's photo kept, as [`Copy::put_hosted`] keeps a picture.
    pub(crate) fn put_photo(&self, user: &str, bytes: &[u8]) -> Result<()> {
        if !id_ok(user) {
            return Err(refused("a photo without a usable id"));
        }
        let record = |sum: &str| json(&PhotoRecord { user: user.to_string(), sum: Some(sum.to_string()), photo_missing_until: None });
        self.put_picture(photo_key(user), record, bytes)
    }

    /// Graph said someone has no photo: not asked again before `until`.
    pub(crate) fn photo_missing(&self, user: &str, until: i64) -> Result<()> {
        if !id_ok(user) {
            return Err(refused("a photo without a usable id"));
        }
        let record = json(&PhotoRecord { user: user.to_string(), sum: None, photo_missing_until: Some(until) })?;
        self.store.write_account_files(TEAMS_ACCOUNT, &[(photo_key(user), record)])
    }

    /// The cache's bounds, for a test: `bytes` and `count` in place of [`IMAGE_CACHE`] and
    /// [`MAX_IMAGES`].
    #[cfg(test)]
    pub(crate) fn cap_images(&self, bytes: u64, count: usize) {
        if let Ok(mut images) = self.images.try_borrow_mut() {
            images.cap = bytes;
            images.most = count;
        }
    }

    /// A picture's bytes, when they're kept and are the ones its hash says: now the most recently
    /// used.
    fn picture(&self, sum: &str) -> Option<Vec<u8>> {
        let bytes = self.store.account_file(TEAMS_ACCOUNT, &img_key(sum)).ok()??;
        if bytes.len() > MAX_PICTURE || hex(&Sha256::digest(&bytes)) != sum {
            return None;
        }
        if let Ok(mut images) = self.images.try_borrow_mut() {
            self.load(&mut images);
            images.touch(sum, size_of(&bytes));
        }
        Some(bytes)
    }

    /// A picture kept under its bytes' hash, with `at` pointing at it (what `point` writes there,
    /// given the hash), in one write with its use and the uses of pictures shown since the last;
    /// the least recently used go first when the cache would be past its bounds.
    fn put_picture(&self, at: String, point: impl FnOnce(&str) -> Result<Vec<u8>>, bytes: &[u8]) -> Result<()> {
        if bytes.len() > MAX_PICTURE {
            return Err(refused("a picture too big"));
        }
        let sum = hex(&Sha256::digest(bytes));
        let pointer = point(&sum)?;
        let mut images = self.images.try_borrow_mut().map_err(|_| refused("the image cache is in use"))?;
        self.load(&mut images);
        let (size, known) = (size_of(bytes), images.entries.contains_key(&sum));
        for gone in images.over(&sum, if known { 0 } else { size }, !known) {
            self.store.remove_account_files(TEAMS_ACCOUNT, &img_key(&gone))?;
            self.store.remove_account_files(TEAMS_ACCOUNT, &use_key(&gone))?;
            images.forget(&gone);
        }
        let entry = Use { size, used: images.tick() };
        let mut files = vec![(img_key(&sum), bytes.to_vec()), (at, pointer), (use_key(&sum), json(&entry)?)];
        for t in images.touched.iter().filter(|t| **t != sum) {
            if let Some(u) = images.entries.get(t) {
                files.push((use_key(t), json(u)?));
            }
        }
        self.store.write_account_files(TEAMS_ACCOUNT, &files)?;
        images.touched.clear();
        images.add(sum, entry);
        Ok(())
    }

    /// The cache's account read from the copy, the first time it's needed: each picture's size
    /// (held to [`MAX_PICTURE`]) and last use.
    fn load(&self, images: &mut Images) {
        if images.loaded {
            return;
        }
        images.loaded = true;
        let names = match self.store.account_files(TEAMS_ACCOUNT, "imguse") {
            Ok(names) => names,
            Err(_) => return log::warn!("Teams copy: its pictures couldn't be listed"),
        };
        let max = u64::try_from(MAX_PICTURE).unwrap_or(u64::MAX);
        for sum in names.iter().filter(|n| is_sum(n)).take(MAX_USES_READ) {
            let Some(u) = self.read(&use_key(sum)).and_then(|b| serde_json::from_slice::<Use>(&b).ok()) else { continue };
            images.clock = images.clock.max(u.used.saturating_add(1));
            images.add(sum.clone(), Use { size: u.size.min(max), used: u.used });
        }
    }
}

/// The cache's account, for a copy just opened: nothing read yet.
pub(super) fn images() -> RefCell<Images> {
    RefCell::new(Images::default())
}
