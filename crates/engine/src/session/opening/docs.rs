// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The session's documents, read as it opens: the rules and the snoozed mail, the contacts, the
//! marks still due, and the list of accounts.

use super::*;

impl Session {
    /// Reads what the data folder keeps for the session, as it opens.
    pub(in crate::session) fn load_documents(&mut self) {
        let mut why = None;
        self.load_rules_and_snoozes();
        if let Some(store) = &self.store {
            self.contacts = Contacts::load(store);
            self.pending_marks = store
                .load_doc::<Vec<(u64, MessageKey, Flag)>>(PENDING_MARKS)
                .unwrap_or_default()
                .into_iter()
                .map(|(id, k, f)| (id, (k, f)))
                .collect();
            // Never taken for none: with none shown, the first change saved would write one
            // account over the others, whose data files would then be nobody's.
            let accounts = match store.load_accounts() {
                Ok(list) => list,
                Err(e) => {
                    log::warn!("the list of accounts can't be read: {}", kind_of(&e));
                    why = Some(e.to_string());
                    Vec::new()
                }
            };
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
        if let Some(why) = why {
            self.notice_with("the list of accounts couldn't be read, so it isn't saved: {error}", &[("error", &why)], true);
            self.accounts_unread = Some(why);
        }
    }
}

/// What kind of trouble a read had, for the log (never the file's words: they can name it).
fn kind_of(e: &quatrelle_store::StoreError) -> String {
    match e {
        quatrelle_store::StoreError::Io { source, .. } => source.kind().to_string(),
        quatrelle_store::StoreError::Damaged { .. } => "damaged".into(),
        _ => "other".into(),
    }
}
