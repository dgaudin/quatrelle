//! The session: pictures from the web, loaded only when asked.

use super::*;

impl Session {
    /// Web pictures of the open message (none for plain-text mail).
    pub fn remote_pictures(&self) -> Vec<String> {
        // Encrypted mail never reaches the web: a picture address could carry what it says out
        // (EFAIL).
        if self.reading_encrypted() {
            return Vec::new();
        }
        self.reading
            .as_ref()
            .and_then(|r| r.parsed.as_ref())
            .map(|p| {
                // Tracking pictures stay unloaded even when the others are shown.
                p.images
                    .iter()
                    .filter(|u| (u.starts_with("https://") || u.starts_with("http://")) && !p.trackers.pictures.contains(u))
                    .take(MAX_PICTURES)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
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
        if self.pictures.len() > 400 {
            self.pictures.retain(|_, p| matches!(p, Picture::Loading));
        }
        for url in self.remote_pictures() {
            if !self.pictures.contains_key(&url) {
                self.pictures.insert(url.clone(), Picture::Loading);
                self.jobs.push(Job::FetchUrl { url });
            }
        }
    }

    /// Show the open message's pictures (`always`: and everything from its sender from now on).
    pub fn show_pictures(&mut self, always: bool) -> CmdResult {
        let r = self.reading.clone().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
        self.pictures_for.insert(r.key.clone());
        let sender = r.parsed.as_ref().map(|p| p.from.email.trim().to_ascii_lowercase()).unwrap_or_default();
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
