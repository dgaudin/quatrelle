// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The session: mailbox settings on the server (automatic replies), emptying folders, the data
//! file kept here (its size, Compact Now), and each folder's size here and on the server.

use quatrelle_model::{FolderSize, MailboxQuota};

use super::*;

/// How far Compact Now has come ([`Session::compacting`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CompactPhase {
    /// Asked for, nothing heard yet.
    #[default]
    Starting,
    /// Making smaller the attachments an earlier version kept uncompressed: `done` of `total` files
    /// looked at. The window goes on as ever.
    Recompressing { done: u64, total: u64 },
    /// Compacting in place: the data file is had to itself, and the window waits under a veil.
    InPlace,
}

/// The last data file compacted here ([`Session::compacted`]): its size before and after, and
/// what making its attachments smaller saved of that (bytes as kept).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Compacted {
    pub account: String,
    pub before: u64,
    pub after: u64,
    pub recompressed: u64,
}

/// Outlook's Folder Size for one account: each folder's size on this computer (Local Data) and
/// on its server (Server Data), as each answer comes in ([`Session::folder_sizes`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FolderSizes {
    pub account: String,
    pub local: Sizes,
    pub server: Sizes,
    /// The mailbox's limits, when the server told them.
    pub quota: Option<MailboxQuota>,
}

/// One side of [`FolderSizes`].
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Sizes {
    /// Asked, not answered yet.
    #[default]
    Loading,
    /// From the top down.
    Loaded(Vec<FolderSize>),
    /// Nothing of the account is kept on this computer (online only).
    NotKept,
    /// The account has no server (the sample mailbox, a data file).
    NoServer,
    /// The server doesn't tell each folder's size (only Exchange does).
    NotTold,
    Failed(String),
}

impl Sizes {
    fn json(&self) -> serde_json::Value {
        match self {
            Sizes::Loading => serde_json::json!({"loading": true}),
            Sizes::Loaded(f) => serde_json::json!({"folders": f}),
            Sizes::NotKept => serde_json::json!({"error": "nothing of this account is kept on this computer"}),
            Sizes::NoServer => serde_json::json!({"error": "this account has no server"}),
            Sizes::NotTold => serde_json::json!({"error": "this server doesn't tell each folder's size"}),
            Sizes::Failed(e) => serde_json::json!({"error": e}),
        }
    }
}

impl Session {
    /// Delete everything in a folder for good (only Deleted Items and Junk Email).
    pub fn empty_folder(&mut self, account: &str, folder: &str) -> CmdResult {
        let f = self.folder(account, folder).ok_or_else(|| CmdError::Params(format!("no folder `{folder}`")))?;
        if !matches!(f.role, FolderRole::Trash | FolderRole::Junk) {
            // Any other folder: everything goes to Deleted Items, as Outlook's Delete All.
            let trash = self
                .account(account)
                .and_then(|a| a.folder(FolderRole::Trash))
                .map(|t| t.path.clone())
                .ok_or_else(|| CmdError::Params("the account has no Deleted Items folder".into()))?;
            let keys: Vec<MessageKey> = self.folder_state(account, folder).messages.iter().map(|m| m.key.clone()).collect();
            let n = keys.len();
            if n > 0 {
                self.apply_op(account, Op::Move { keys, to: trash });
            }
            return Ok(serde_json::json!({ "moved": n, "to": "trash" }));
        }
        self.apply_op(account, Op::EmptyFolder { path: folder.to_string() });
        Ok(serde_json::json!({ "emptied": folder }))
    }

    /// Ask the server for an account's automatic replies.
    pub fn load_vacation(&mut self, account: &str) -> CmdResult {
        let a = self.account_mut(account).ok_or_else(|| CmdError::Params(format!("no account `{account}`")))?;
        if a.is_local() {
            a.vacation = VacationState::Failed("the sample mailbox has no server".into());
        } else {
            a.vacation = VacationState::Loading;
            self.jobs.push(Job::GetVacation { account: account.to_string() });
        }
        Ok(self.vacation_json(account))
    }

    /// Change an account's automatic replies on the server.
    pub fn set_vacation(&mut self, account: &str, v: Vacation) -> CmdResult {
        for d in [&v.from, &v.until].into_iter().flatten() {
            if !quatrelle_model::job::is_day(d) {
                return Err(CmdError::Params(format!("`{d}` is not a date (YYYY-MM-DD)")));
            }
        }
        if let (Some(a), Some(b)) = (&v.from, &v.until)
            && a > b
        {
            return Err(CmdError::Params("the last day is before the first".into()));
        }
        if v.enabled && v.text.trim().is_empty() {
            return Err(CmdError::Params("write the reply first".into()));
        }
        let a = self.account_mut(account).ok_or_else(|| CmdError::Params(format!("no account `{account}`")))?;
        if a.is_local() {
            return Err(CmdError::Params("the sample mailbox has no server".into()));
        }
        a.vacation = VacationState::Loading;
        self.jobs.push(Job::SetVacation { account: account.to_string(), vacation: v });
        Ok(self.vacation_json(account))
    }

    pub fn vacation_json(&self, account: &str) -> serde_json::Value {
        match self.account(account).map(|a| &a.vacation) {
            Some(VacationState::Loaded(v)) => serde_json::json!({"vacation": v}),
            Some(VacationState::Loading) => serde_json::json!({"pending": true}),
            Some(VacationState::Failed(e)) => serde_json::json!({"error": e}),
            _ => serde_json::json!({}),
        }
    }

    /// Bytes of mail cached on this computer for an account.
    pub fn cache_size(&self, account: &str) -> u64 {
        self.store.as_ref().map_or(0, |s| s.account_size(account))
    }

    /// Compact an account's data file, as Outlook's Compact Now (the current account's when none
    /// is named): attachments an earlier version kept uncompressed are made smaller first, the
    /// window going on as ever ([`CompactPhase::Recompressing`]), then the space freed inside it
    /// goes back to the disk, the rest waiting for it ([`CompactPhase::InPlace`]).
    pub fn compact_data_file(&mut self, account: Option<&str>) -> CmdResult {
        let id = match account {
            Some(a) => a.to_string(),
            None => self.current_account().map(|a| a.config.id.clone()).ok_or_else(|| CmdError::Disabled("add an account first".into()))?,
        };
        if self.account(&id).is_none() {
            return Err(CmdError::Params(format!("no account `{id}`")));
        }
        if self.compacting.is_some() {
            return Err(CmdError::Disabled("a data file is being compacted already".into()));
        }
        if self.store.as_ref().and_then(|s| s.data_file(&id)).is_none() {
            return Err(CmdError::Disabled("this account keeps no data file on this computer".into()));
        }
        self.compacting = Some(id.clone());
        self.compact_phase = CompactPhase::Starting;
        self.jobs.push(Job::CompactDataFile { account: id.clone() });
        Ok(serde_json::json!({"compacting": id}))
    }

    /// Read how much space each of an account's folders takes (the current account's when none
    /// is named), here and on its server, as Outlook's Folder Size: the answers come in as
    /// events, into [`Session::sizes`].
    pub fn folder_sizes(&mut self, account: Option<&str>) -> CmdResult {
        let id = match account {
            Some(a) => a.to_string(),
            None => self.current_account().map(|a| a.config.id.clone()).ok_or_else(|| CmdError::Disabled("add an account first".into()))?,
        };
        let a = self.account(&id).ok_or_else(|| CmdError::Params(format!("no account `{id}`")))?;
        let (local, online, ews) = (a.is_local(), a.config.keeps_online(), a.config.ews.is_some());
        let mut v = FolderSizes { account: id.clone(), ..Default::default() };
        if online || self.store.is_none() {
            v.local = Sizes::NotKept;
        } else {
            self.jobs.push(Job::LocalFolderSizes { account: id.clone() });
        }
        if local {
            v.server = Sizes::NoServer;
        } else if ews {
            self.jobs.push(Job::ServerFolderSizes { account: id.clone() });
        } else {
            v.server = Sizes::NotTold;
        }
        let out = serde_json::json!({"account": id, "local": v.local.json(), "server": v.server.json()});
        self.sizes = Some(v);
        Ok(out)
    }

    /// An answer to [`Session::folder_sizes`], for the account still being looked at.
    pub(in crate::session) fn sizes_answered(
        &mut self,
        account: &str,
        server: bool,
        folders: Vec<FolderSize>,
        quota: Option<MailboxQuota>,
        error: Option<String>,
    ) {
        let Some(v) = self.sizes.as_mut().filter(|v| v.account == account) else { return };
        let side = match error {
            Some(e) => Sizes::Failed(e),
            None => Sizes::Loaded(folders),
        };
        if server {
            v.server = side;
            v.quota = quota.or(v.quota);
        } else {
            v.local = side;
        }
    }

    /// The account whose data file Compact Now has to itself now, when it does: the window
    /// doesn't read it meanwhile (its reads would be refused), and carries on once it's done.
    pub fn compacting_in_place(&self) -> Option<&str> {
        self.compacting.as_deref().filter(|_| self.compact_phase == CompactPhase::InPlace)
    }

    /// How far Compact Now has come, for the account it was asked for.
    pub(in crate::session) fn compact_progress(&mut self, account: &str, in_place: bool, done: u64, total: u64) {
        if self.compacting.as_deref() != Some(account) {
            return;
        }
        self.compact_phase = if in_place { CompactPhase::InPlace } else { CompactPhase::Recompressing { done: done.min(total), total } };
    }

    pub(in crate::session) fn compacted(&mut self, account: &str, done: Compacted, no_room: bool, error: Option<String>) {
        self.compacting = None;
        self.compact_phase = CompactPhase::Starting;
        match error {
            None => {
                self.compacted = Some(Compacted { account: account.to_string(), ..done });
                self.notice("The data file was compacted.", false);
                if no_room {
                    self.notice("There wasn't room on the drive to make attachments smaller first: the data file was only compacted.", false);
                }
            }
            Some(e) => self.notice_with("The data file could not be compacted: {error}", &[("error", &e)], true),
        }
    }
}
