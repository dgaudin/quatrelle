// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The session: pictures from the web, loaded only when asked.

use super::*;

/// Most web pictures kept loaded before those no longer shown are let go.
const PICTURES_KEPT: usize = 400;

/// Whether a picture's address is one on the web (the only ones ever fetched).
fn on_web(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
}

/// The web pictures of a message that may load: tracking pictures stay unloaded even when the
/// others are shown.
fn web_pictures_of(p: &Parsed) -> impl Iterator<Item = &String> {
    p.images.iter().filter(|u| on_web(u) && !p.trackers.pictures.contains(u)).take(MAX_PICTURES)
}

impl Session {
    /// Web pictures of the open message (none for plain-text mail).
    pub fn remote_pictures(&self) -> Vec<String> {
        // Encrypted mail never reaches the web: a picture address could carry what it says out
        // (EFAIL).
        if self.reading_encrypted() {
            return Vec::new();
        }
        self.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| web_pictures_of(p).cloned().collect()).unwrap_or_default()
    }

    /// Whether the open message's web pictures may load: asked for this message; else, but for
    /// mail in Junk Email, as File › Options › Privacy says ([`PictureLoading`]): from a sender
    /// trusted or safe, also from the people known (saved in Contacts or written to), or always.
    pub fn pictures_allowed(&self) -> bool {
        let Some(r) = &self.reading else { return false };
        if self.pictures_for.contains(&r.key) {
            return true;
        }
        if self.folder(&r.key.account, &r.key.folder).is_some_and(|f| f.role == FolderRole::Junk) {
            return false;
        }
        let sender = r.parsed.as_ref().map(|p| p.from.email.trim().to_ascii_lowercase()).unwrap_or_default();
        let trusted = !sender.is_empty() && (self.prefs.trusted_senders.contains(&sender) || self.sender_safe(&sender));
        match self.prefs.pictures {
            PictureLoading::Ask => trusted,
            // From the address book as it's read (no search through every card each frame): one
            // who only ever wrote here is in it too, and isn't known for that.
            PictureLoading::Known => trusted || (!sender.is_empty() && self.known_contact(&sender).is_some_and(|c| c.saved || c.sent > 0)),
            PictureLoading::Always => true,
        }
    }

    /// When pictures from the web load by themselves; the open message's load now if they may.
    pub fn set_picture_loading(&mut self, to: PictureLoading) -> CmdResult {
        self.prefs.pictures = to;
        self.request_pictures();
        Ok(serde_json::json!({ "value": to.code(), "allowed": self.pictures_allowed() }))
    }

    /// Ask the network layer for the open message's pictures, when allowed.
    pub(super) fn request_pictures(&mut self) {
        if !self.pictures_allowed() {
            return;
        }
        let urls = self.remote_pictures();
        self.request_urls(&urls);
    }

    /// Ask the network layer for the pictures at `urls` not loaded or loading yet. Past
    /// [`PICTURES_KEPT`], those loaded are let go first, except the ones on screen: of the open
    /// message, a message's own window and the signatures written in HTML, and those just asked
    /// for. Nothing but `urls` is fetched.
    pub(crate) fn request_urls(&mut self, urls: &[String]) {
        if self.pictures.len() > PICTURES_KEPT {
            let mut shown: HashSet<String> = urls.iter().cloned().collect();
            for r in self.reading.iter().chain(&self.windows) {
                if let Some(p) = &r.parsed {
                    shown.extend(web_pictures_of(p).cloned());
                }
            }
            shown.extend(self.signature_web_pictures());
            self.pictures.retain(|url, p| matches!(p, Picture::Loading) || shown.contains(url));
        }
        for url in urls {
            if !self.pictures.contains_key(url) {
                self.pictures.insert(url.clone(), Picture::Loading);
                self.jobs.push(Job::FetchUrl { url: url.clone() });
            }
        }
    }

    /// Show the open message's pictures (`always`: and everything from its sender from now on).
    pub fn show_pictures(&mut self, always: bool) -> CmdResult {
        let r = self.reading.clone().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
        let sender = r.parsed.as_ref().map(|p| p.from.email.trim().to_ascii_lowercase()).unwrap_or_default();
        if always && sender.is_empty() {
            self.whole_here()?;
        }
        self.pictures_for.insert(r.key.clone());
        if always && !sender.is_empty() && self.prefs.trusted_senders.len() < 10_000 {
            self.prefs.trusted_senders.insert(sender.clone());
        }
        self.request_pictures();
        Ok(serde_json::json!({ "pictures": self.remote_pictures().len(), "trusted": always.then_some(sender) }))
    }

    /// Stop loading pictures from the open message's sender automatically.
    pub fn block_pictures(&mut self) -> CmdResult {
        let r = self.reading.clone().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
        self.pictures_for.remove(&r.key);
        let sender = r.parsed.as_ref().map(|p| p.from.email.trim().to_ascii_lowercase()).unwrap_or_default();
        let was = self.prefs.trusted_senders.remove(&sender);
        Ok(serde_json::json!({ "untrusted": was }))
    }
}
