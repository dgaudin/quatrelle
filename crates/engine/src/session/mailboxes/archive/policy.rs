// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The session: Exchange's archive policy. The tags the server offers are told once a session
//! ([`Event::ArchiveTags`]); the tag a message carries is asked when it is opened
//! ([`Job::ReadArchiveTag`]); a folder's comes with the folder list. Setting one is a change
//! through the journal ([`Op::SetArchiveTag`]): made at once here, sent when the server can be
//! reached, put back with a notice when the server refuses it or no longer has the message.

use quatrelle_model::{ArchiveTag, Op};
use serde_json::json;

use super::*;

/// Most messages whose tag is remembered (the last ones opened).
const MAX_REMEMBERED: usize = 256;
/// Most changes waiting to be told their outcome.
const MAX_PENDING: usize = 256;

/// What a tag was before a change, to put it back.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Target {
    Message(MessageKey),
    Folder(String, String),
}

/// The archive policy as the session knows it.
#[derive(Debug, Default)]
pub(crate) struct PolicyState {
    /// The tags each account's server offers.
    tags: HashMap<String, Vec<ArchiveTag>>,
    /// The tag (an id) and planned day (`YYYY-MM-DD`) of the messages opened lately.
    messages: HashMap<MessageKey, (Option<String>, Option<String>)>,
    /// The tag each target had before a change that isn't known to have reached the server.
    before: Vec<(Target, Option<String>)>,
}

/// What a message's header says of its archive policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessagePolicy {
    /// The tag's name, as the server gives it.
    pub name: String,
    /// The day the server plans to move it, when it says.
    pub day: Option<String>,
}

impl Session {
    /// The archive policy tags the account's server offers (none for a server without).
    pub fn archive_tags(&self, account: &str) -> &[ArchiveTag] {
        self.policy.tags.get(account).map_or(&[], Vec::as_slice)
    }

    /// The tags were told (`Event::ArchiveTags`).
    pub(crate) fn tags_told(&mut self, account: String, tags: Vec<ArchiveTag>) {
        if self.account(&account).is_some() {
            self.policy.tags.insert(account, tags);
        }
    }

    /// The archive policy a message carries of its own, with the tag's name.
    pub fn message_policy(&self, key: &MessageKey) -> Option<MessagePolicy> {
        let (tag, day) = self.policy.messages.get(key)?;
        let id = tag.as_deref()?;
        let name = self.archive_tags(&key.account).iter().find(|t| t.id == id)?.name.clone();
        Some(MessagePolicy { name, day: day.clone() })
    }

    /// The id of the archive tag a folder carries of its own.
    pub fn folder_archive_tag(&self, account: &str, folder: &str) -> Option<&str> {
        self.account(account)?.folders.iter().find(|f| f.path == folder)?.archive_tag.as_deref()
    }

    /// Whether the archive policy can be set on mail of this folder: an Exchange account whose
    /// server offers tags, outside the online archive.
    pub fn offers_archive_policy(&self, account: &str, folder: &str) -> bool {
        folder != OUTBOX
            && !self.archive_tags(account).is_empty()
            && !self.read_only_folder(account, folder)
            && self.account(account).is_some_and(|a| a.config.ews.is_some() && !a.is_local())
    }

    /// A message was opened: ask the server for its tag when the account has some to offer.
    pub(crate) fn ask_archive_tag(&mut self, key: &MessageKey) {
        if self.offers_archive_policy(&key.account, &key.folder) {
            self.jobs.push(Job::ReadArchiveTag { key: key.clone() });
        }
    }

    /// A message's tag came from the server: that is the truth, whatever was waiting.
    pub(crate) fn tag_read(&mut self, key: MessageKey, tag: Option<String>, date: Option<String>) {
        if self.policy.messages.len() >= MAX_REMEMBERED {
            self.policy.messages.clear();
        }
        self.policy.before.retain(|(t, _)| *t != Target::Message(key.clone()));
        self.policy.messages.insert(key, (tag, date));
    }

    /// The tag to set, checked against what the server offers (its id as the server gave it).
    fn checked_tag(&self, account: &str, tag: Option<&str>) -> Result<Option<String>, CmdError> {
        let Some(t) = tag else { return Ok(None) };
        self.archive_tags(account)
            .iter()
            .find(|o| o.id.eq_ignore_ascii_case(t))
            .map(|o| Some(o.id.clone()))
            .ok_or_else(|| CmdError::Params("`tag` isn't one of the server's archive policy tags".into()))
    }

    fn remember_before(&mut self, target: Target, before: Option<String>) {
        if self.policy.before.len() >= MAX_PENDING {
            self.policy.before.remove(0);
        }
        // The first value stands while the change before it is still waiting.
        if !self.policy.before.iter().any(|(t, _)| *t == target) {
            self.policy.before.push((target, before));
        }
    }

    /// Give the selected messages an archive policy tag, or take theirs off (`None`).
    pub fn archive_policy_selection(&mut self, tag: Option<&str>) -> CmdResult {
        self.refuse_archive_selection()?;
        let keys: Vec<MessageKey> = self.selection.iter().filter(|k| k.folder != OUTBOX).cloned().collect();
        if keys.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        let mut accounts: Vec<String> = keys.iter().map(|k| k.account.clone()).collect();
        accounts.sort();
        accounts.dedup();
        if let Some(a) = accounts.iter().find(|a| !self.offers_archive_policy(a, "INBOX")) {
            return Err(CmdError::Disabled(format!("{a}: this account's server offers no archive policy")));
        }
        // Every account's tag is checked before anything is changed.
        let mut chosen = Vec::new();
        for a in &accounts {
            chosen.push(self.checked_tag(a, tag)?);
        }
        for (a, chosen) in accounts.iter().zip(chosen) {
            let mine: Vec<MessageKey> = keys.iter().filter(|k| &k.account == a).cloned().collect();
            for k in &mine {
                let before = self.policy.messages.get(k).and_then(|(t, _)| t.clone());
                self.remember_before(Target::Message(k.clone()), before);
                if self.policy.messages.len() >= MAX_REMEMBERED {
                    self.policy.messages.clear();
                }
                self.policy.messages.insert(k.clone(), (chosen.clone(), None));
            }
            self.jobs.push(Job::Apply { account: a.clone(), op: Op::SetArchiveTag { keys: mine, folder: None, tag: chosen } });
        }
        Ok(json!({"messages": keys.len(), "tag": tag}))
    }

    /// Give a folder's mail an archive policy tag, or take the folder's off (`None`).
    pub fn archive_policy_folder(&mut self, account: &str, folder: &str, tag: Option<&str>) -> CmdResult {
        self.refuse_archive_folder(account, folder)?;
        if self.account(account).is_none_or(|a| !a.folders.iter().any(|f| f.path == folder)) {
            return Err(CmdError::Params(format!("no folder `{folder}`")));
        }
        if !self.offers_archive_policy(account, folder) {
            return Err(CmdError::Disabled("this account's server offers no archive policy".into()));
        }
        let chosen = self.checked_tag(account, tag)?;
        let before = self.folder_archive_tag(account, folder).map(str::to_string);
        self.remember_before(Target::Folder(account.into(), folder.into()), before);
        self.set_folder_tag(account, folder, chosen.clone());
        self.jobs.push(Job::Apply {
            account: account.to_string(),
            op: Op::SetArchiveTag { keys: Vec::new(), folder: Some(folder.to_string()), tag: chosen },
        });
        Ok(json!({"folder": folder, "tag": tag}))
    }

    fn set_folder_tag(&mut self, account: &str, folder: &str, tag: Option<String>) {
        if let Some(f) = self.account_mut(account).and_then(|a| a.folders.iter_mut().find(|f| f.path == folder)) {
            f.archive_tag = tag;
        }
    }

    /// The server didn't take a tag change: the tags go back to what they were.
    pub(crate) fn policy_failed(&mut self, account: &str, op: &Op) {
        let Op::SetArchiveTag { keys, folder, .. } = op else { return };
        let mut targets: Vec<Target> = keys.iter().map(|k| Target::Message(k.clone())).collect();
        targets.extend(folder.iter().map(|f| Target::Folder(account.to_string(), f.clone())));
        for t in targets {
            let Some(at) = self.policy.before.iter().position(|(x, _)| *x == t) else { continue };
            let (_, was) = self.policy.before.remove(at);
            match t {
                Target::Message(k) => {
                    self.policy.messages.insert(k, (was, None));
                }
                Target::Folder(a, f) => self.set_folder_tag(&a, &f, was),
            }
        }
    }
}
