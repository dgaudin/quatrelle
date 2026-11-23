// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The session's documents, read as it opens: the rules and the snoozed mail, the contacts, the
//! marks still due, the list of accounts and each account's folders.

use super::*;

/// What an account whose folder list is damaged says (the file is left as it is): its server's
/// next listing makes it again.
pub(crate) const FOLDERS_DAMAGED: &str = "this account's list of folders is damaged and is read again from the server";
/// What an account with no server (POP, on this computer) whose folder list is damaged says before
/// why: nothing makes it again, and nothing is written over it.
pub(crate) const FOLDERS_DAMAGED_HERE: &str = "this account's list of folders is damaged, so nothing is filed in it for now";

impl Session {
    /// Reads what the data folder keeps for the session, as it opens.
    pub(in crate::session) fn load_documents(&mut self) {
        let mut why = None;
        // The accounts whose folder list is damaged: who, what was said, and whether a server lists.
        let mut damaged: Vec<(String, String, bool)> = Vec::new();
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
                // Never taken for none: the account opens with no folders shown, its file left as
                // it is, and it's said.
                let mut folders = match store.load_folders(&config.id) {
                    Ok(folders) => folders,
                    Err(e) => {
                        log::warn!("{}: the folder list can't be read: {}", config.id, kind_of(&e));
                        if matches!(e, quatrelle_store::StoreError::Damaged { .. }) {
                            let who = if config.email.is_empty() { config.id.clone() } else { config.email.clone() };
                            damaged.push((who, e.to_string(), config.is_local() || config.pop.is_some()));
                        }
                        Vec::new()
                    }
                };
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
        for (who, e, here) in damaged {
            let said = if here { format!("{FOLDERS_DAMAGED_HERE}: {e}") } else { FOLDERS_DAMAGED.to_string() };
            self.notice_with("{who}: {error}", &[("who", &who), ("error", &said)], true);
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
