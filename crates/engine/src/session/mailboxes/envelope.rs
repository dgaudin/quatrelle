//! The envelope in the notification area: Outlook shows one while mail that arrived in an Inbox
//! is unread, and clicking it opens the Inbox. The app draws it; the session says when.

use serde_json::json;

use super::*;

/// The most new messages followed: a burst (a first sync, a long time offline) keeps the newest.
const MAX_FOLLOWED: usize = 200;

/// The new mail the envelope stands for, until it's read.
#[derive(Debug, Default)]
pub(crate) struct Envelope {
    /// What arrived, oldest first.
    keys: Vec<MessageKey>,
}

impl Session {
    /// Mail just announced as new in an Inbox (`fresh`: unread, recent, not filed away by a rule).
    pub(super) fn add_to_envelope(&mut self, fresh: &[Summary]) {
        if !self.prefs.envelope {
            return;
        }
        self.envelope.keys.extend(fresh.iter().rev().map(|m| m.key.clone()));
        let excess = self.envelope.keys.len().saturating_sub(MAX_FOLLOWED);
        self.envelope.keys.drain(..excess);
    }

    /// How many of the messages that arrived are still unread: the envelope shows while there are
    /// any. A message read (here or elsewhere), deleted or moved away is forgotten; one whose
    /// folder isn't in memory is kept until it can be looked at. New mail is at the top of a
    /// folder's list, so the lookups stay short.
    pub fn new_unread(&mut self) -> usize {
        if !self.prefs.envelope {
            self.envelope.keys.clear();
        }
        if self.envelope.keys.is_empty() {
            return 0;
        }
        let mut keys = std::mem::take(&mut self.envelope.keys);
        keys.retain(|k| {
            if self.account(&k.account).is_none() {
                return false;
            }
            match self.cache.get(&(k.account.clone(), k.folder.clone())) {
                Some(st) => st.messages.iter().find(|m| &m.key == k).is_some_and(|m| !m.flags.seen),
                None => true,
            }
        });
        self.envelope.keys = keys;
        self.envelope.keys.len()
    }

    /// Clicking the envelope: the Inbox the new mail is in (All Inboxes when it came to several).
    pub(crate) fn show_new_mail(&mut self) -> CmdResult {
        self.module = crate::Module::Mail;
        let mut inboxes: Vec<(&str, &str)> = self.envelope.keys.iter().map(|k| (k.account.as_str(), k.folder.as_str())).collect();
        inboxes.sort_unstable();
        inboxes.dedup();
        let view = match inboxes.as_slice() {
            [(account, folder)] => View::Folder { account: (*account).to_string(), folder: (*folder).to_string() },
            [] => self.default_view(),
            _ => View::Unified { role: FolderRole::Inbox },
        };
        if self.view != view {
            self.open_view(view);
        }
        Ok(json!({"view": self.view, "new": self.envelope.keys.len()}))
    }
}
