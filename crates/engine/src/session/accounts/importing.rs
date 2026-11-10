// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The session: a data file's import followed (where it is, how much of it), stopped and gone on
//! with. An import that isn't finished is read from its journal when the session opens, and is
//! never started again by itself: only `account.resumeImport` does.

use super::*;
use quatrelle_model::{IMPORT_DOC, IMPORT_FILE_CHANGED, IMPORT_NO_JOURNAL, ImportJournal};

/// An import running now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportState {
    /// The folder being read, by its readable name; empty before the first.
    pub folder: String,
    /// Messages in so far (those of an earlier run included, when it goes on).
    pub done: usize,
    /// The messages to come in all, when the file's folders could be counted.
    pub total: Option<usize>,
    /// Stop was asked: the import ends at its next item.
    pub stopping: bool,
    /// It goes on with an earlier one (its account is never taken away when it fails).
    pub resumed: bool,
}

/// An import stopped, or cut short by a quit or a crash, that can be gone on with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnfinishedImport {
    /// The data file, where it was.
    pub path: String,
    pub done: usize,
    pub total: Option<usize>,
}

/// What the pool tells of an import ([`Event::Imported`]).
pub(in crate::session) struct ImportReport {
    pub account: String,
    pub messages: usize,
    pub skipped: usize,
    pub total: Option<usize>,
    pub folder: String,
    pub stopped: bool,
    pub done: bool,
    pub error: Option<String>,
}

/// A folder's readable leaf name, from its path (modified UTF-7, `/` between levels).
fn leaf(path: &str) -> String {
    let decoded = quatrelle_model::utf7::decode(path);
    decoded.rsplit('/').next().unwrap_or(&decoded).to_string()
}

impl Session {
    /// The account's import running now.
    pub fn import_of(&self, account: &str) -> Option<&ImportState> {
        self.importing.get(account)
    }

    /// The account's import that isn't finished and isn't running.
    pub fn unfinished_import(&self, account: &str) -> Option<&UnfinishedImport> {
        self.unfinished_imports.get(account).filter(|_| !self.importing.contains_key(account))
    }

    /// Note that an import of the account begins.
    pub(super) fn begin_import(&mut self, account: &str, resumed: bool, done: usize, total: Option<usize>) {
        self.importing.insert(account.to_string(), ImportState { done, total, resumed, ..Default::default() });
    }

    /// Read the account's journal again: whether its import is left unfinished.
    pub(in crate::session) fn read_unfinished(&mut self, account: &str) {
        self.unfinished_imports.remove(account);
        let Some(store) = &self.store else { return };
        // A damaged journal reads as none: the import can only be started over.
        let journal: ImportJournal = store.load_account_doc(account, IMPORT_DOC).unwrap_or_default();
        if journal.unfinished() {
            self.unfinished_imports
                .insert(account.to_string(), UnfinishedImport { path: journal.path, done: journal.messages, total: journal.total });
        }
    }

    /// Stop the account's import: it ends at its next item, keeping what came in.
    pub fn stop_import(&mut self, account: &str) -> CmdResult {
        if self.account(account).is_none() {
            return Err(CmdError::Params(format!("no account `{account}`")));
        }
        let Some(state) = self.importing.get_mut(account) else {
            return Err(CmdError::Disabled("no import is running in this account".into()));
        };
        state.stopping = true;
        self.jobs.push(Job::StopImport { account: account.to_string() });
        Ok(serde_json::json!({ "account": account, "stopping": true }))
    }

    /// Go on with the account's unfinished import, from where it stopped. `path`: the data file
    /// when it isn't where it was; `allow_changed`: bring in what is new in it when it changed.
    pub fn resume_import(&mut self, account: &str, path: Option<&str>, allow_changed: bool) -> CmdResult {
        if self.account(account).is_none() {
            return Err(CmdError::Params(format!("no account `{account}`")));
        }
        if self.importing.contains_key(account) {
            return Err(CmdError::Disabled("this account's import is running".into()));
        }
        let Some(left) = self.unfinished_imports.get(account).cloned() else {
            return Err(CmdError::Disabled("this account has no import to go on with".into()));
        };
        let path = path.map(str::trim).filter(|p| !p.is_empty()).map_or(left.path, str::to_string);
        if let Some(a) = self.account_mut(account) {
            a.status = SyncStatus::Syncing { folder: String::new() };
        }
        self.begin_import(account, true, left.done, left.total);
        self.jobs.push(Job::ResumeImport { account: account.to_string(), path, allow_changed });
        Ok(serde_json::json!({ "account": account, "importing": true }))
    }

    /// Whether `account.stopImport` applies to some account.
    pub(crate) fn can_stop_import(&self) -> bool {
        self.importing.values().any(|i| !i.stopping)
    }

    /// Whether `account.resumeImport` applies to some account.
    pub(crate) fn can_resume_import(&self) -> bool {
        self.unfinished_imports.keys().any(|a| !self.importing.contains_key(a))
    }

    /// The account to stop or go on with: the one named, else the only one that fits.
    pub(crate) fn import_account(&self, named: Option<&str>, running: bool) -> Result<String, CmdError> {
        if let Some(a) = named {
            return if self.account(a).is_some() { Ok(a.to_string()) } else { Err(CmdError::Params(format!("no account `{a}`"))) };
        }
        let mut fit = self.accounts.iter().map(|a| &a.config.id).filter(|a| {
            if running { self.importing.contains_key(*a) } else { self.unfinished_imports.contains_key(*a) && !self.importing.contains_key(*a) }
        });
        match (fit.next(), fit.next()) {
            (Some(a), None) => Ok(a.clone()),
            (None, _) => Err(CmdError::Disabled("no import fits".into())),
            _ => Err(CmdError::Params("`account` is required: several imports fit".into())),
        }
    }

    /// The pool tells how an import goes, or how it ended.
    pub(in crate::session) fn imported(&mut self, r: ImportReport) {
        let ImportReport { account, messages, skipped, total, folder, stopped, done, error } = r;
        let account = account.as_str();
        let Some(name) = self.account(account).map(|a| a.config.email.clone()) else {
            self.importing.remove(account);
            self.unfinished_imports.remove(account);
            return;
        };
        if !done {
            // Followed in the status bar, not by a notice for each step.
            let state = self.importing.entry(account.to_string()).or_default();
            state.done = messages;
            state.total = total.or(state.total);
            if !folder.is_empty() {
                state.folder = leaf(&folder);
            }
            return;
        }
        let state = self.importing.remove(account);
        self.read_unfinished(account);
        // An import stopped or gone on with, or one that left its journal unfinished, never
        // takes its account away: only a first import that fails with nothing noted does.
        let keep = stopped || state.is_some_and(|s| s.resumed) || self.unfinished_imports.contains_key(account);
        if let Some(a) = self.account_mut(account) {
            a.status = SyncStatus::Offline;
        }
        // The file's contacts, calendar, notes and tasks, written next to its mail.
        let pim =
            self.store.clone().and_then(|store| store.load_account_doc::<quatrelle_model::pim::Pim>(account, quatrelle_model::pim::PIM_DOC).ok());
        // An error after mail or anything else came in is the last save failing: what came in
        // stays (the import used to end there, and the account went with all it had written).
        let came_in =
            messages > 0 || pim.as_ref().is_some_and(|p| !(p.cards.is_empty() && p.events.is_empty() && p.notes.is_empty() && p.tasks.is_empty()));
        // An account that was waiting for its data file connects now.
        let held = self.held.remove(account);
        let has_held = held.is_some();
        if let Some(start) = held {
            self.jobs.push(start);
        }
        // An import that can't go on is said, and the account stays as it is.
        if let Some(e) = error.as_deref().filter(|e| *e == IMPORT_FILE_CHANGED || *e == IMPORT_NO_JOURNAL) {
            if e == IMPORT_FILE_CHANGED {
                if !self.ask_import.iter().any(|a| a == account) {
                    self.ask_import.push(account.to_string());
                }
            } else {
                self.notice_with(
                    "{name}: what this import had brought in was not noted, so it can't go on. Open the file again to bring it in whole.",
                    &[("name", &name)],
                    true,
                );
            }
            return;
        }
        if has_held
            && let Some(e) = &error
            && !came_in
        {
            self.notice_with("{name}: the data file could not be opened: {error}", &[("name", &name), ("error", e)], true);
            return;
        }
        match error {
            Some(e) if !came_in && !keep => {
                // Nothing came in: the account goes again, and the file it was starting.
                let _ = self.drop_account(account, false);
                self.notice_with("{name} could not be opened: {error}", &[("name", &name), ("error", &e)], true);
            }
            Some(e) if !came_in => {
                self.notice_with("{name}: the import could not go on: {error}", &[("name", &name), ("error", &e)], true);
            }
            error => {
                // The file's contacts and calendar came in too.
                if let Some(mut p) = pim {
                    p.ensure_defaults();
                    p.events.iter_mut().for_each(|e| e.event.adopt_kept_conference());
                    self.pim.insert(account.to_string(), p);
                    self.pim_unread.remove(account);
                    self.forget_contacts();
                    self.pim_revision = self.pim_revision.wrapping_add(1);
                }
                let counted = messages.to_string();
                if let Some(e) = error {
                    // The folder list came with the end and is held here: saved again, or the
                    // next start would find none and the mail out of reach.
                    self.finish_local_marks();
                    let kept = match (&self.store, self.account(account)) {
                        (Some(store), Some(a)) if !a.folders.is_empty() => {
                            store.save_folders(account, &a.folders).inspect_err(|e| log::warn!("{account}: folders not saved: {e}")).is_ok()
                        }
                        _ => false,
                    };
                    let values = [("name", name.as_str()), ("messages", counted.as_str()), ("error", e.as_str())];
                    if kept {
                        self.notice_with("{name}: {messages} messages imported, but the last of it could not be saved: {error}", &values, true);
                    } else {
                        self.notice_with(
                            "{name}: {messages} messages imported, but the last of it could not be saved: {error}. Import the file again to keep it.",
                            &values,
                            true,
                        );
                    }
                } else if stopped {
                    match total {
                        Some(total) => {
                            let total = total.to_string();
                            self.notice_with(
                                "{name}: import stopped, {done} of {total} messages are here. Right-click the account to go on with it.",
                                &[("name", &name), ("done", &counted), ("total", &total)],
                                false,
                            );
                        }
                        None => self.notice_with(
                            "{name}: import stopped, {done} messages are here. Right-click the account to go on with it.",
                            &[("name", &name), ("done", &counted)],
                            false,
                        ),
                    }
                } else if skipped > 0 {
                    let skipped = skipped.to_string();
                    self.notice_with(
                        "{name}: {messages} messages imported ({skipped} could not be read)",
                        &[("name", &name), ("messages", &counted), ("skipped", &skipped)],
                        false,
                    );
                } else {
                    self.notice_with("{name}: {messages} messages imported", &[("name", &name), ("messages", &counted)], false);
                }
                if !stopped && let Some(inbox) = self.account(account).and_then(|a| a.folder(FolderRole::Inbox)).map(|f| f.path.clone()) {
                    self.open_view(View::Folder { account: account.to_string(), folder: inbox });
                }
            }
        }
    }
}
