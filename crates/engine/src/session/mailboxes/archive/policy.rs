// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The session: Exchange's archive and retention policies. The tags the server offers, of both
//! kinds, are told after each connection ([`Event::ArchiveTags`]); the tags a message carries
//! are asked when it is opened, or when the « Policy » submenu shows a message whose tags aren't
//! known ([`Job::ReadArchiveTag`]); a folder's come with the folder list. Setting one is a change
//! through the journal ([`Op::SetArchiveTag`], with its [`PolicyKind`]): made at once here, sent
//! when the server can be reached, put back with a notice when the server refuses it or no
//! longer has the message. While it waits, a read that doesn't carry it (one answered before the
//! change reached the server: the read goes ahead of the journal) leaves it shown.

use quatrelle_model::{ArchiveTag, Op, PolicyKind};
use serde_json::json;

use super::*;

/// Most messages whose tags are remembered (the last ones read), and most asked at once.
const MAX_REMEMBERED: usize = 256;
/// Most changes waiting to be told their outcome.
const MAX_PENDING: usize = 256;

/// What a tag was before a change, to put it back.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Target {
    Message(MessageKey, PolicyKind),
    Folder(String, String, PolicyKind),
}

/// A message's own tag of one kind (an id) and the day the server plans for it (`YYYY-MM-DD`).
type Carried = (Option<String>, Option<String>);

/// The policies as the session knows them.
#[derive(Debug, Default)]
pub(crate) struct PolicyState {
    /// The tags each account's server offers, of both kinds.
    tags: HashMap<String, Vec<ArchiveTag>>,
    /// The archive and the retention tag of the messages read lately, with their days.
    messages: HashMap<MessageKey, [Carried; 2]>,
    /// The messages whose tags were asked of the server and haven't come.
    asked: HashSet<MessageKey>,
    /// The tag each target had before a change that isn't known to have reached the server.
    before: Vec<(Target, Option<String>)>,
}

/// Where a kind's tag sits in a message's pair.
fn slot(kind: PolicyKind) -> usize {
    match kind {
        PolicyKind::Archive => 0,
        PolicyKind::Retention => 1,
    }
}

/// Why a policy of this kind can't be set: the server offers none.
pub(crate) fn none_offered(kind: PolicyKind) -> &'static str {
    match kind {
        PolicyKind::Archive => "this account's server offers no archive policy",
        PolicyKind::Retention => "this account's server offers no retention policy",
    }
}

/// What a message's header says of one of its policies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessagePolicy {
    /// The tag's id (a GUID): how the header finds it among those offered.
    pub id: String,
    /// The tag's name, as the server gives it.
    pub name: String,
    /// The day the server plans to move it (archive) or to delete it (retention), when it says.
    pub day: Option<String>,
}

impl Session {
    /// The policy tags of this kind the account's server offers (none for a server without).
    pub fn policy_tags(&self, account: &str, kind: PolicyKind) -> impl Iterator<Item = &ArchiveTag> {
        self.policy.tags.get(account).into_iter().flatten().filter(move |t| t.kind == kind)
    }

    /// The tags were told (`Event::ArchiveTags`, at each pass over the account's folders: after
    /// each connection). A message's tags asked and never answered (asked offline, or the read
    /// lost to an error: the network says nothing then) may be asked again.
    pub(crate) fn tags_told(&mut self, account: String, tags: Vec<ArchiveTag>) {
        self.policy.asked.retain(|k| k.account != account);
        if self.account(&account).is_some() {
            self.policy.tags.insert(account, tags);
        }
    }

    /// The policy of this kind a message carries of its own, with the tag's name.
    pub fn message_policy(&self, key: &MessageKey, kind: PolicyKind) -> Option<MessagePolicy> {
        let (tag, day) = self.policy.messages.get(key)?.get(slot(kind))?;
        let id = tag.as_deref()?;
        let name = self.policy_tags(&key.account, kind).find(|t| t.id == id)?.name.clone();
        Some(MessagePolicy { id: id.to_string(), name, day: day.clone() })
    }

    /// The id of the tag of this kind a message carries of its own, as last read.
    pub fn message_tag(&self, key: &MessageKey, kind: PolicyKind) -> Option<&str> {
        self.policy.messages.get(key)?.get(slot(kind))?.0.as_deref()
    }

    /// Whether a message's tags are known, or asked of the server already: the submenu asks for
    /// those of a message that isn't ([`Session::read_policy_of`]).
    pub fn message_tags_known(&self, key: &MessageKey) -> bool {
        self.policy.messages.contains_key(key) || self.policy.asked.contains(key)
    }

    /// The id of the tag of this kind a folder carries of its own.
    pub fn folder_tag(&self, account: &str, folder: &str, kind: PolicyKind) -> Option<&str> {
        let f = self.account(account)?.folders.iter().find(|f| f.path == folder)?;
        match kind {
            PolicyKind::Archive => f.archive_tag.as_deref(),
            PolicyKind::Retention => f.retention_tag.as_deref(),
        }
    }

    /// Whether a policy can be set on mail of this folder: an Exchange account whose server
    /// offers tags (of either kind), outside the online archive.
    pub fn offers_policy(&self, account: &str, folder: &str) -> bool {
        folder != OUTBOX
            && self.policy.tags.get(account).is_some_and(|t| !t.is_empty())
            && !self.read_only_folder(account, folder)
            && self.account(account).is_some_and(|a| a.config.ews.is_some() && !a.is_local())
    }

    /// Whether a policy of this kind can be set there: [`Session::offers_policy`], with tags of
    /// that kind.
    pub fn offers_policy_of(&self, account: &str, folder: &str, kind: PolicyKind) -> bool {
        self.offers_policy(account, folder) && self.policy_tags(account, kind).next().is_some()
    }

    /// A message was opened, or the submenu wants its tags: ask the server for them when the
    /// account has some to offer.
    pub(crate) fn ask_archive_tag(&mut self, key: &MessageKey) {
        if self.offers_policy(&key.account, &key.folder) {
            if self.policy.asked.len() >= MAX_REMEMBERED {
                self.policy.asked.clear();
            }
            self.policy.asked.insert(key.clone());
            self.jobs.push(Job::ReadArchiveTag { key: key.clone() });
        }
    }

    /// The submenu shows a message's tags: they are asked of the server when they aren't known
    /// (a message only selected, or opened before the server told its tags), once a message
    /// however often it is drawn.
    pub fn read_policy_of(&mut self, key: &MessageKey) -> serde_json::Value {
        let ask = !self.message_tags_known(key) && self.offers_policy(&key.account, &key.folder);
        if ask {
            self.ask_archive_tag(key);
        }
        json!({"asked": ask})
    }

    /// A tag id read from the server as the server offered it (ids are GUIDs, whatever their
    /// case); one it doesn't offer, as it came.
    fn offered_id(&self, account: &str, kind: PolicyKind, id: String) -> String {
        self.policy_tags(account, kind).find(|t| t.id.eq_ignore_ascii_case(&id)).map_or(id, |t| t.id.clone())
    }

    /// Room for one more message's tags: once full, those with a change waiting are kept, the
    /// others forgotten (they are read again when wanted). Those kept are never more than the
    /// changes waiting (at most `MAX_PENDING`), so one more fits: a change waiting is never
    /// forgotten, its tag compared with each read.
    fn make_room(&mut self, key: &MessageKey) {
        let PolicyState { messages, before, .. } = &mut self.policy;
        if messages.len() < MAX_REMEMBERED || messages.contains_key(key) {
            return;
        }
        messages.retain(|k, _| before.iter().any(|(t, _)| matches!(t, Target::Message(m, _) if m == k)));
    }

    /// A message's tags came from the server. A kind with a change waiting keeps the tag shown
    /// while the read doesn't carry it (answered before the change reached the server), and that
    /// read is what a refusal puts back; one that carries it ends the wait. Otherwise what the
    /// server says is shown.
    pub(crate) fn tag_read(&mut self, key: MessageKey, archive: Carried, retention: Carried) {
        self.policy.asked.remove(&key);
        let mut carried = [archive, retention];
        for kind in PolicyKind::ALL {
            let Some(c) = carried.get_mut(slot(kind)) else { continue };
            c.0 = c.0.take().map(|id| self.offered_id(&key.account, kind, id));
            let waiting = Target::Message(key.clone(), kind);
            let Some(at) = self.policy.before.iter().position(|(t, _)| *t == waiting) else { continue };
            let shown = self.message_tag(&key, kind).map(str::to_string);
            let carries = match (shown.as_deref(), c.0.as_deref()) {
                (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
                (a, b) => a.is_none() && b.is_none(),
            };
            if carries {
                self.policy.before.remove(at);
            } else {
                // The server's tag before the change: a refusal shows it, not an older one.
                if let Some(b) = self.policy.before.get_mut(at) {
                    b.1 = c.0.clone();
                }
                *c = (shown, None);
            }
        }
        self.make_room(&key);
        self.policy.messages.insert(key, carried);
    }

    /// The tag to set, checked against what the server offers of that kind (its id as the
    /// server gave it).
    fn checked_tag(&self, account: &str, kind: PolicyKind, tag: Option<&str>) -> Result<Option<String>, CmdError> {
        let Some(t) = tag else { return Ok(None) };
        self.policy_tags(account, kind).find(|o| o.id.eq_ignore_ascii_case(t)).map(|o| Some(o.id.clone())).ok_or_else(|| {
            CmdError::Params(match kind {
                PolicyKind::Archive => "`tag` isn't one of the server's archive policy tags".into(),
                PolicyKind::Retention => "`tag` isn't one of the server's retention policy tags".into(),
            })
        })
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

    /// Set a message's tag of this kind as it is shown, its day unknown until read again.
    fn show_message_tag(&mut self, key: &MessageKey, kind: PolicyKind, tag: Option<String>) {
        self.make_room(key);
        let carried = self.policy.messages.entry(key.clone()).or_default();
        if let Some(c) = carried.get_mut(slot(kind)) {
            *c = (tag, None);
        }
    }

    /// Give the selected messages a policy tag of this kind, or take theirs off (`None`).
    pub fn policy_selection(&mut self, kind: PolicyKind, tag: Option<&str>) -> CmdResult {
        self.refuse_archive_selection()?;
        let keys: Vec<MessageKey> = self.selection.iter().filter(|k| k.folder != OUTBOX).cloned().collect();
        if keys.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        let mut accounts: Vec<String> = keys.iter().map(|k| k.account.clone()).collect();
        accounts.sort();
        accounts.dedup();
        if let Some(a) = accounts.iter().find(|a| !self.offers_policy_of(a, "INBOX", kind)) {
            return Err(CmdError::Disabled(format!("{a}: {}", none_offered(kind))));
        }
        // Every account's tag is checked before anything is changed.
        let mut chosen = Vec::new();
        for a in &accounts {
            chosen.push(self.checked_tag(a, kind, tag)?);
        }
        for (a, chosen) in accounts.iter().zip(chosen) {
            let mine: Vec<MessageKey> = keys.iter().filter(|k| &k.account == a).cloned().collect();
            for k in &mine {
                let before = self.message_tag(k, kind).map(str::to_string);
                self.remember_before(Target::Message(k.clone(), kind), before);
                self.show_message_tag(k, kind, chosen.clone());
            }
            self.jobs.push(Job::Apply { account: a.clone(), op: Op::SetArchiveTag { keys: mine, folder: None, tag: chosen, kind } });
        }
        Ok(json!({"messages": keys.len(), "tag": tag}))
    }

    /// Give a folder's mail a policy tag of this kind, or take the folder's off (`None`).
    pub fn policy_folder(&mut self, kind: PolicyKind, account: &str, folder: &str, tag: Option<&str>) -> CmdResult {
        self.refuse_archive_folder(account, folder)?;
        if self.account(account).is_none_or(|a| !a.folders.iter().any(|f| f.path == folder)) {
            return Err(CmdError::Params(format!("no folder `{folder}`")));
        }
        if !self.offers_policy_of(account, folder, kind) {
            return Err(CmdError::Disabled(none_offered(kind).into()));
        }
        let chosen = self.checked_tag(account, kind, tag)?;
        let before = self.folder_tag(account, folder, kind).map(str::to_string);
        self.remember_before(Target::Folder(account.into(), folder.into(), kind), before);
        self.set_folder_tag(account, folder, kind, chosen.clone());
        self.jobs.push(Job::Apply {
            account: account.to_string(),
            op: Op::SetArchiveTag { keys: Vec::new(), folder: Some(folder.to_string()), tag: chosen, kind },
        });
        Ok(json!({"folder": folder, "tag": tag}))
    }

    fn set_folder_tag(&mut self, account: &str, folder: &str, kind: PolicyKind, tag: Option<String>) {
        if let Some(f) = self.account_mut(account).and_then(|a| a.folders.iter_mut().find(|f| f.path == folder)) {
            match kind {
                PolicyKind::Archive => f.archive_tag = tag,
                PolicyKind::Retention => f.retention_tag = tag,
            }
        }
    }

    /// The server didn't take a tag change: the tags go back to what they were.
    pub(crate) fn policy_failed(&mut self, account: &str, op: &Op) {
        let Op::SetArchiveTag { keys, folder, kind, .. } = op else { return };
        let kind = *kind;
        let mut targets: Vec<Target> = keys.iter().map(|k| Target::Message(k.clone(), kind)).collect();
        targets.extend(folder.iter().map(|f| Target::Folder(account.to_string(), f.clone(), kind)));
        for t in targets {
            let Some(at) = self.policy.before.iter().position(|(x, _)| *x == t) else { continue };
            let (_, was) = self.policy.before.remove(at);
            match t {
                Target::Message(k, kind) => self.show_message_tag(&k, kind, was),
                Target::Folder(a, f, kind) => self.set_folder_tag(&a, &f, kind, was),
            }
        }
    }
}
