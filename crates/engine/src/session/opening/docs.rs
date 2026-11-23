// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The session's documents, read as it opens: the rules and the snoozed mail, the contacts, the
//! marks still due, and the list of accounts.

use super::*;

impl Session {
    /// Reads what the data folder keeps for the session, as it opens.
    pub(in crate::session) fn load_documents(&mut self) {
        self.load_rules_and_snoozes();
        if let Some(store) = &self.store {
            self.contacts = Contacts::load(store);
            self.pending_marks = store
                .load_doc::<Vec<(u64, MessageKey, Flag)>>(PENDING_MARKS)
                .unwrap_or_default()
                .into_iter()
                .map(|(id, k, f)| (id, (k, f)))
                .collect();
            let accounts = store.load_accounts().unwrap_or_else(|e| {
                log::warn!("accounts: {e}");
                Vec::new()
            });
            for config in accounts {
                // Online only: held in memory from the start (anything left on disk goes).
                if config.keeps_online()
                    && let Err(e) = store.set_online_only(&config.id, true)
                {
                    log::warn!("{}: couldn't be kept online only: {e}", config.id);
                }
                let mut folders = store.load_folders(&config.id).unwrap_or_default();
                // A folder belongs to the account it's kept with (a data file brought in from
                // another account once kept that one's name).
                for f in &mut folders {
                    if f.account != config.id {
                        f.account.clone_from(&config.id);
                    }
                }
                sort_folders(&mut folders);
                self.accounts.push(AccountState { config, folders, status: SyncStatus::Offline, quota: None, vacation: VacationState::Unknown });
            }
        }
    }
}
