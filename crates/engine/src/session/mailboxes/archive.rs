// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The session: IMAP mail moved into the Archives account on this computer (a local account made
//! the first time), and put back. The network layer copies each message, reads the copy back and
//! only then deletes it on the server; the session asks for it, keeps the messages out of the list
//! while that runs, and remembers how to undo it.

use quatrelle_model::{AccountConfig, AccountRole, MAX_ARCHIVE_BATCH};

use super::*;

mod age;
mod policy;
mod read_only;
pub(crate) use age::AgeState;
pub use policy::MessagePolicy;
pub(crate) use policy::PolicyState;
pub use read_only::ARCHIVE_READ_ONLY;

/// *Archive* on an IMAP account not set to archive here, whose server has no Archive folder.
const NO_ARCHIVE_FOLDER: &str =
    "this account has no Archive folder on its server: to archive its mail on this computer, tick “Archive on this computer” in File › Info";
/// *Archive* on mail already in the Archives account.
const ALREADY_ARCHIVED: &str = "this mail is already archived on this computer: drag it to another folder of Archives to move it";

/// A message on its way into or out of the Archives account: kept out of the list until the
/// network layer says how it went.
#[derive(Clone, Debug)]
pub(crate) struct Leaving {
    /// Its Message-ID, to find it again where it lands.
    pub id: String,
    /// Putting back: the folder of the server it goes to.
    pub to: Option<String>,
}

/// The reply to a move between accounts that isn't archiving or putting back.
const NOT_YET: &str = "not between these accounts yet";

impl Session {
    /// The Archives account, when there is one: found by its mark, whatever it is called.
    pub fn archives_id(&self) -> Option<String> {
        self.accounts.iter().find(|a| a.config.role == Some(AccountRole::Archives)).map(|a| a.config.id.clone())
    }

    /// Whether this account's mail is on an IMAP server (not on this computer, POP, or a web API).
    pub fn is_imap(&self, account: &str) -> bool {
        self.account(account).is_some_and(|a| !a.is_local() && a.config.pop.is_none() && !a.config.is_api())
    }

    /// Whether *Archive* files this account's mail on this computer: an IMAP account that says so.
    pub(crate) fn archives_here(&self, account: &str) -> bool {
        self.is_imap(account) && self.account(account).is_some_and(|a| a.config.archive_local)
    }

    /// *Archive* (the button, Backspace): each message goes to its account's Archive folder, or,
    /// for an IMAP account set to archive on this computer, into the Archives account.
    pub fn archive_selection(&mut self) -> CmdResult {
        self.refuse_archive_selection()?;
        let sel = self.selected();
        let (here, rest): (Vec<Arc<Summary>>, Vec<Arc<Summary>>) = sel.iter().cloned().partition(|m| self.archives_here(&m.key.account));
        if here.is_empty() {
            if let Some(why) = self.nowhere_to_archive(&sel) {
                return Err(CmdError::Failed(why.into()));
            }
            return self.move_selection(MoveTarget::Role(FolderRole::Archive));
        }
        let mut moved = 0;
        if !rest.is_empty() {
            moved = self.move_messages(&rest, &MoveTarget::Role(FolderRole::Archive))?;
        }
        let r = self.archive_messages(&here)?;
        Ok(serde_json::json!({ "moved": moved, "archiving": r["archiving"] }))
    }

    /// Why *Archive* has nowhere to put `sel`, with what to do about it: mail already in Archives,
    /// or an IMAP account not set to archive here whose server has no Archive folder.
    fn nowhere_to_archive(&self, sel: &[Arc<Summary>]) -> Option<&'static str> {
        let archived = |m: &Summary| self.account(&m.key.account).is_some_and(|a| a.config.role == Some(AccountRole::Archives));
        if sel.iter().any(|m| archived(m)) {
            return Some(ALREADY_ARCHIVED);
        }
        let no_folder = |m: &Summary| {
            self.is_imap(&m.key.account)
                && self.account(&m.key.account).is_some_and(|a| a.folder_beside(&m.key.folder, FolderRole::Archive).is_none())
        };
        sel.iter().any(|m| no_folder(m)).then_some(NO_ARCHIVE_FOLDER)
    }

    /// `archive.settings`: whether *Archive* files into the Archives account, and the age (in
    /// months, 1 to 120, or none) after which mail is offered for archiving. IMAP accounts only.
    pub fn archive_settings(&mut self, account: &str, local: Option<bool>, after_months: Option<Option<u32>>) -> CmdResult {
        if self.account(account).is_none() {
            return Err(CmdError::Params(format!("no account `{account}`")));
        }
        if !self.is_imap(account) {
            return Err(CmdError::Disabled("archiving on this computer is for IMAP accounts".into()));
        }
        if matches!(after_months, Some(Some(m)) if !(1..=120).contains(&m)) {
            return Err(CmdError::Params("`afterMonths` is 1 to 120, or null".into()));
        }
        if let Some(a) = self.account_mut(account) {
            if let Some(l) = local {
                a.config.archive_local = l;
            }
            if let Some(m) = after_months {
                a.config.archive_after_months = m;
            }
        }
        self.save_accounts();
        if after_months.is_some() {
            self.age_settings_changed(account);
        }
        let c = self.account(account).map(|a| (a.config.archive_local, a.config.archive_after_months));
        Ok(serde_json::json!({ "account": account, "local": c.map(|c| c.0), "afterMonths": c.and_then(|c| c.1) }))
    }

    /// The Archives account, made when there is none: an account on this computer named
    /// "Archives", below the others.
    pub(crate) fn archives_account(&mut self) -> Result<String, CmdError> {
        if let Some(id) = self.archives_id() {
            return Ok(id);
        }
        let id = self.new_account_id();
        let config = AccountConfig {
            id: id.clone(),
            name: "Archives".into(),
            // Shown where an account's address would be; never an address, so never "me".
            email: "Archives".into(),
            color: u8::try_from(self.accounts.len() % usize::from(ACCOUNT_COLORS)).unwrap_or(0),
            role: Some(AccountRole::Archives),
            ..Default::default()
        };
        self.accounts.push(AccountState { config, folders: Vec::new(), status: SyncStatus::Offline, quota: None, vacation: VacationState::Unknown });
        self.save_accounts();
        Ok(id)
    }

    /// Move the selection to `folder` of another account (`mail.move` with an `account`): from an
    /// IMAP account into the Archives account is archiving, from the Archives account to a folder
    /// of an IMAP account is putting back, within one account is a move; the rest is refused.
    pub fn move_across(&mut self, account: &str, folder: &str) -> CmdResult {
        let sel = self.selected();
        if sel.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        self.refuse_archive_selection()?;
        self.refuse_archive_folder(account, folder)?;
        let wants_archives = account.eq_ignore_ascii_case("archives");
        if !wants_archives && self.account(account).is_none() {
            return Err(CmdError::Params(format!("no account `{account}`")));
        }
        if !wants_archives && sel.iter().all(|m| m.key.account == account) {
            return self.move_selection(MoveTarget::Folder(folder.to_string()));
        }
        let archives = self.archives_id();
        let into_archives = wants_archives || archives.as_deref() == Some(account);
        if into_archives && sel.iter().all(|m| self.is_imap(&m.key.account)) {
            return self.archive_messages(&sel);
        }
        if self.is_imap(account) && archives.is_some() && sel.iter().all(|m| Some(&m.key.account) == archives.as_ref()) {
            return self.put_back_messages(&sel, account, folder);
        }
        Err(CmdError::Disabled(NOT_YET.into()))
    }

    /// Ask the network layer to archive these messages (all IMAP mail), batch by batch, and take
    /// them out of the list.
    pub(crate) fn archive_messages(&mut self, sel: &[Arc<Summary>]) -> CmdResult {
        let next = self.successor(&self.selection.clone());
        self.queue_archive(sel)?;
        self.refresh_list();
        self.after_removal(next);
        Ok(serde_json::json!({ "archiving": sel.len() }))
    }

    /// The jobs that archive these messages, and the messages marked as on their way.
    pub(super) fn queue_archive(&mut self, sel: &[Arc<Summary>]) -> Result<(), CmdError> {
        let to = self.archives_account()?;
        let mut by_account: BTreeMap<String, Vec<MessageKey>> = BTreeMap::new();
        for m in sel {
            by_account.entry(m.key.account.clone()).or_default().push(m.key.clone());
            self.leaving.insert(m.key.clone(), Leaving { id: m.message_id.clone(), to: None });
        }
        for (account, keys) in by_account {
            for batch in keys.chunks(MAX_ARCHIVE_BATCH) {
                self.jobs.push(Job::Archive { account: account.clone(), keys: batch.to_vec(), to_account: to.clone() });
            }
        }
        Ok(())
    }

    /// Ask the network layer to put archived messages on the server, into `folder` of `account`.
    fn put_back_messages(&mut self, sel: &[Arc<Summary>], account: &str, folder: &str) -> CmdResult {
        if self.folder(account, folder).is_none_or(|f| !f.selectable) {
            return Err(CmdError::Params(format!("no folder `{folder}`")));
        }
        let next = self.successor(&self.selection.clone());
        let keys: Vec<MessageKey> = sel.iter().map(|m| m.key.clone()).collect();
        for m in sel {
            self.leaving.insert(m.key.clone(), Leaving { id: m.message_id.clone(), to: Some(folder.to_string()) });
        }
        for batch in keys.chunks(MAX_ARCHIVE_BATCH) {
            self.jobs.push(Job::PutBack { account: account.to_string(), keys: batch.to_vec(), to_folder: folder.to_string() });
        }
        self.refresh_list();
        self.after_removal(next);
        Ok(serde_json::json!({ "puttingBack": sel.len() }))
    }

    /// Say that some messages stayed where they were, and why (the first reason, in the
    /// network layer's words), when nothing else is said of this batch.
    fn say_kept(&mut self, kept: &[(MessageKey, String)]) {
        let Some((_, why)) = kept.first() else { return };
        let count = kept.len().to_string();
        self.notice_with("{count} messages stay where they were: {error}", &[("count", &count), ("error", why)], true);
    }

    /// The network layer archived some messages: `moved` pairs each with its copy in Archives,
    /// `kept` stayed where they were.
    pub(crate) fn archived(&mut self, account: &str, moved: Vec<(MessageKey, MessageKey)>, kept: Vec<(MessageKey, String)>) {
        self.age_progress(account, moved.len() + kept.len());
        for (from, _) in &moved {
            self.leaving.remove(from);
        }
        for (key, _) in &kept {
            self.leaving.remove(key);
        }
        if !moved.is_empty() {
            let back: Vec<(MessageKey, String)> = moved.iter().map(|(from, copy)| (copy.clone(), from.folder.clone())).collect();
            self.remember(Undo::Archive { account: account.to_string(), back });
            let count = moved.len().to_string();
            match kept.first() {
                Some((_, why)) => {
                    let left = kept.len().to_string();
                    self.notice_with(
                        "{count} messages archived on this computer, {kept} stayed where they were: {error}",
                        &[("count", &count), ("kept", &left), ("error", why)],
                        false,
                    );
                }
                None => self.notice_with("{count} messages archived on this computer", &[("count", &count)], false),
            }
            self.offer_undo();
        } else {
            self.say_kept(&kept);
        }
        self.refresh_list();
    }

    /// The network layer put some archived messages on the server: `back` pairs each with its UID
    /// there, `kept` stayed in Archives.
    pub(crate) fn put_back_done(&mut self, account: &str, back: Vec<(MessageKey, Option<u32>)>, kept: Vec<(MessageKey, String)>) {
        let mut to: Option<String> = None;
        let mut ids = Vec::new();
        for (key, _) in &back {
            if let Some(l) = self.leaving.remove(key) {
                to = to.or(l.to);
                if !l.id.is_empty() {
                    ids.push(l.id);
                }
            }
        }
        for (key, _) in &kept {
            self.leaving.remove(key);
        }
        if !back.is_empty() {
            // Undo archives them again (moved by the user; one put back by Undo has no entry here).
            let mut remembered = false;
            if let (Some(to), false) = (to, ids.is_empty()) {
                self.remember(Undo::PutBack { account: account.to_string(), to, ids });
                remembered = true;
            }
            let count = back.len().to_string();
            match kept.first() {
                Some((_, why)) => {
                    let left = kept.len().to_string();
                    self.notice_with(
                        "{count} messages put back on the server, {kept} stayed in Archives: {error}",
                        &[("count", &count), ("kept", &left), ("error", why)],
                        false,
                    );
                }
                None => self.notice_with("{count} messages put back on the server", &[("count", &count)], false),
            }
            if remembered {
                self.offer_undo();
            }
        } else {
            self.say_kept(&kept);
        }
        self.refresh_list();
    }

    /// Undo an archive: its messages go back to the folders of the server they came from.
    pub(super) fn undo_archive(&mut self, account: String, back: Vec<(MessageKey, String)>) -> CmdResult {
        let mut by_folder: BTreeMap<String, Vec<MessageKey>> = BTreeMap::new();
        for (copy, folder) in &back {
            by_folder.entry(folder.clone()).or_default().push(copy.clone());
        }
        let n = back.len();
        for (folder, keys) in by_folder {
            for batch in keys.chunks(MAX_ARCHIVE_BATCH) {
                self.jobs.push(Job::PutBack { account: account.clone(), keys: batch.to_vec(), to_folder: folder.clone() });
            }
        }
        Ok(serde_json::json!({ "undone": "archive", "count": n }))
    }

    /// Undo a put back: the messages, found again in `to` by Message-ID, are archived once more.
    pub(super) fn undo_put_back(&mut self, account: String, to: String, ids: Vec<String>) -> CmdResult {
        let here = self.folder_state(&account, &to).messages.clone();
        let mut used: BTreeSet<MessageKey> = BTreeSet::new();
        for id in &ids {
            if let Some(m) = here.iter().find(|m| &m.message_id == id && !used.contains(&m.key)) {
                used.insert(m.key.clone());
            }
        }
        if used.is_empty() {
            // Not there yet (the server is still bringing them): try again in a moment.
            self.undo.push(Undo::PutBack { account, to, ids });
            return Err(CmdError::Failed("still being put back; try Undo again in a moment".into()));
        }
        let archives = self.archives_account()?;
        let keys: Vec<MessageKey> = used.into_iter().collect();
        let n = keys.len();
        for batch in keys.chunks(MAX_ARCHIVE_BATCH) {
            self.jobs.push(Job::Archive { account: account.clone(), keys: batch.to_vec(), to_account: archives.clone() });
        }
        Ok(serde_json::json!({ "undone": "putBack", "count": n }))
    }
}
