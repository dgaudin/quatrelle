// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Recover deleted items: what an Exchange server keeps of the items deleted for good, listed in
//! a window, brought back into Deleted Items or purged. Read on the server each time the window
//! opens, a page at a time, and never kept in the cache.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};

use quatrelle_model::{RECOVERABLE_MAX, Recoverable};

use super::*;

/// Why an account can't be asked for its deleted items.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoverBlock {
    /// Its server doesn't keep deleted items for the user to bring back (anything but Exchange).
    NoServer,
    /// Working offline, or the account isn't connected: the server has to be asked.
    Offline,
}

/// What was done last in the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recovered {
    pub count: usize,
    pub purged: bool,
}

/// The window listing an account's recoverable items.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Recovering {
    pub account: String,
    /// Which asking the answers are for: a closed window's, or an earlier one's, are dropped.
    pub ask: u64,
    pub items: Vec<Recoverable>,
    /// The server has more past the last page.
    pub more: bool,
    /// A page is being read.
    pub loading: bool,
    /// Items are being brought back or purged.
    pub working: bool,
    /// Why the last reading or change failed.
    pub error: Option<String>,
    pub last: Option<Recovered>,
}

/// Numbers the askings, so a late answer to an earlier one is told from the current one's.
static ASKED: AtomicU64 = AtomicU64::new(1);

impl Session {
    /// Why this account can't list its deleted items, or `None` when it can be asked.
    pub fn recover_block(&self, account: &str) -> Option<RecoverBlock> {
        let a = self.account(account)?;
        if a.config.ews.is_none() && a.config.graph.is_none() {
            return Some(RecoverBlock::NoServer);
        }
        if self.prefs.work_offline || !a.status.is_online() {
            return Some(RecoverBlock::Offline);
        }
        None
    }

    /// Open the window on an account and read the server's first page.
    pub fn recover_deleted(&mut self, account: &str) -> CmdResult {
        if self.account(account).is_none() {
            return Err(CmdError::Params(format!("no account `{account}`")));
        }
        match self.recover_block(account) {
            Some(RecoverBlock::NoServer) => return Err(CmdError::Disabled("this server doesn't keep deleted items for you to bring back".into())),
            Some(RecoverBlock::Offline) => return Err(CmdError::Disabled("connect first: the server keeps these items".into())),
            None => {}
        }
        let ask = ASKED.fetch_add(1, Ordering::Relaxed);
        self.recovering = Some(Recovering { account: account.to_string(), ask, loading: true, ..Default::default() });
        self.jobs.push(Job::ListRecoverable { account: account.to_string(), ask, offset: 0 });
        Ok(serde_json::json!({"account": account}))
    }

    /// Read the next page.
    pub fn recover_more(&mut self) -> CmdResult {
        let r = self.recovering.as_mut().ok_or_else(|| CmdError::Disabled("the window is not open".into()))?;
        if !r.more || r.loading {
            return Ok(serde_json::json!({"asked": false}));
        }
        r.loading = true;
        let (account, ask, offset) = (r.account.clone(), r.ask, u32::try_from(r.items.len()).unwrap_or(u32::MAX));
        self.jobs.push(Job::ListRecoverable { account, ask, offset });
        Ok(serde_json::json!({"asked": true}))
    }

    /// Bring the items `ids` back into Deleted Items, or with `purge` delete them for good. Only
    /// what the window lists is touched.
    pub fn recover_items(&mut self, ids: &[String], purge: bool) -> CmdResult {
        let r = self.recovering.as_mut().ok_or_else(|| CmdError::Disabled("the window is not open".into()))?;
        if r.working {
            return Err(CmdError::Disabled("wait for the last change to finish".into()));
        }
        let listed: HashSet<&str> = r.items.iter().map(|i| i.id.as_str()).collect();
        let mut seen = HashSet::new();
        let ids: Vec<String> = ids.iter().filter(|i| listed.contains(i.as_str()) && seen.insert(i.as_str())).cloned().collect();
        if ids.is_empty() {
            return Err(CmdError::Params("none of those items is in the list".into()));
        }
        r.working = true;
        r.error = None;
        r.last = None;
        let (account, ask, n) = (r.account.clone(), r.ask, ids.len());
        self.jobs.push(Job::RestoreRecoverable { account, ask, ids, purge });
        Ok(serde_json::json!({"items": n, "purge": purge}))
    }

    pub fn recover_close(&mut self) -> CmdResult {
        self.recovering = None;
        Ok(serde_json::json!({}))
    }

    /// A page of the list came in.
    pub(in crate::session) fn recoverable_listed(
        &mut self,
        account: &str,
        ask: u64,
        offset: u32,
        items: Vec<Recoverable>,
        more: bool,
        error: Option<String>,
    ) {
        let Some(r) = self.recovering.as_mut().filter(|r| r.ask == ask && r.account == account) else { return };
        r.loading = false;
        if let Some(e) = error {
            r.error = Some(e);
            r.more = false;
            return;
        }
        r.error = None;
        if offset == 0 {
            r.items.clear();
        }
        let have: HashSet<String> = r.items.iter().map(|i| i.id.clone()).collect();
        r.items.extend(items.into_iter().filter(|i| !have.contains(&i.id)).take(RECOVERABLE_MAX.saturating_sub(r.items.len())));
        r.more = more && r.items.len() < RECOVERABLE_MAX;
    }

    /// Items were brought back or purged.
    pub(in crate::session) fn recoverable_done(&mut self, account: &str, ask: u64, done: Vec<String>, purge: bool, error: Option<String>) {
        let back = if purge { None } else { self.account(account).and_then(|a| a.folder(FolderRole::Trash)).map(|f| f.path.clone()) };
        if let (Some(folder), false) = (back, done.is_empty()) {
            // They come into Deleted Items with the sync that follows.
            self.jobs.push(Job::SyncFolder { account: account.to_string(), folder });
        }
        let Some(r) = self.recovering.as_mut().filter(|r| r.ask == ask && r.account == account) else { return };
        r.working = false;
        let gone: HashSet<&str> = done.iter().map(String::as_str).collect();
        r.items.retain(|i| !gone.contains(i.id.as_str()));
        r.error = error;
        r.last = Some(Recovered { count: done.len(), purged: purge });
    }
}
