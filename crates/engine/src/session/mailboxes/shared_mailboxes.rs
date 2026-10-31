//! Other mailboxes opened in an Exchange account: a shared mailbox (support@…), or someone's who
//! made this account a delegate. Each lists under a folder named after its address, and its
//! calendar opens beside this account's, as Outlook adds them.

use super::*;

/// Mailboxes one account opens at most.
const MAX_SHARED: usize = 20;

impl Session {
    /// The Exchange account other mailboxes open in: the one asked for, else the first.
    fn exchange_account(&self, wanted: Option<&str>) -> Result<String, CmdError> {
        let a = match wanted {
            Some(id) => self.account(id).ok_or_else(|| CmdError::Params(format!("no account `{id}`")))?,
            None => self
                .accounts
                .iter()
                .find(|a| a.config.graph.is_some() || a.config.ews.is_some())
                .ok_or_else(|| CmdError::Disabled("other mailboxes open in an Exchange or Microsoft 365 account".into()))?,
        };
        if a.config.graph.is_none() && a.config.ews.is_none() {
            return Err(CmdError::Params("other mailboxes open in an Exchange or Microsoft 365 account".into()));
        }
        Ok(a.config.id.clone())
    }

    /// Open another mailbox in an Exchange account (and its calendar, with `calendar`).
    pub fn open_shared_mailbox(&mut self, account: Option<&str>, email: &str, calendar: bool) -> CmdResult {
        let account = self.exchange_account(account)?;
        let email = email.trim().to_ascii_lowercase();
        let ok =
            quatrelle_model::addr::parse_list(&email).is_ok_and(|l| l.len() == 1 && l.first().is_some_and(|a| a.email.eq_ignore_ascii_case(&email)));
        if !ok || email.contains('/') {
            return Err(CmdError::Params(format!("not an email address: {email}")));
        }
        // Best effort, by the field's own rule: a kind the session knows that Open another mailbox
        // doesn't take (a list, a folder, a group) is no mailbox. An address it doesn't know goes
        // through, the server has the last word.
        if !quatrelle_model::addr::Accepts::MAILBOX.takes(self.addressee_kind(Some(&account), &email)) {
            return Err(CmdError::Params(format!("not a mailbox to open: {email}")));
        }
        let Some(a) = self.account_mut(&account) else { return Err(CmdError::Params(format!("no account `{account}`"))) };
        if a.config.email.eq_ignore_ascii_case(&email) {
            return Err(CmdError::Params("that's this account's own mailbox".into()));
        }
        if !a.config.shared_mailboxes.iter().any(|m| m.eq_ignore_ascii_case(&email)) {
            if a.config.shared_mailboxes.len() >= MAX_SHARED {
                return Err(CmdError::Params(format!("at most {MAX_SHARED} other mailboxes can be open in an account")));
            }
            a.config.shared_mailboxes.push(email.clone());
        }
        let config = a.config.clone();
        self.save_accounts();
        // Suggested from now on.
        self.contacts.changed();
        // The worker starts again with the mailbox in its list, and lists its folders.
        if !self.prefs.work_offline {
            self.jobs.push(Job::Start { account: config, password: None });
        }
        if calendar && let Err(e) = self.open_shared_calendar(Some(&account), &email) {
            log::info!("{email}'s calendar: {e}");
        }
        let under = self.account(&account).map(|a| a.config.email.clone()).unwrap_or_default();
        self.notice_with("{email} opens under {account}", &[("email", &email), ("account", &under)], false);
        Ok(serde_json::json!({ "account": account, "folder": email }))
    }

    /// Close another mailbox opened in an account: its folders (and its calendar, when open)
    /// go from this computer.
    pub fn close_shared_mailbox(&mut self, account: Option<&str>, email: &str) -> CmdResult {
        let email = email.trim().to_ascii_lowercase();
        let account = match account {
            Some(a) => a.to_string(),
            None => self
                .accounts
                .iter()
                .find(|a| a.config.shared_mailboxes.iter().any(|m| m.eq_ignore_ascii_case(&email)))
                .map(|a| a.config.id.clone())
                .ok_or_else(|| CmdError::Params(format!("{email} isn't open")))?,
        };
        let Some(a) = self.account_mut(&account) else { return Err(CmdError::Params(format!("no account `{account}`"))) };
        if !a.config.shared_mailboxes.iter().any(|m| m.eq_ignore_ascii_case(&email)) {
            return Err(CmdError::Params(format!("{email} isn't open in this account")));
        }
        let gone: Vec<String> =
            a.folders.iter().filter(|f| a.folder_mailbox(&f.path).is_some_and(|m| m.eq_ignore_ascii_case(&email))).map(|f| f.path.clone()).collect();
        a.config.shared_mailboxes.retain(|m| !m.eq_ignore_ascii_case(&email));
        a.folders.retain(|f| !gone.contains(&f.path));
        let config = a.config.clone();
        self.contacts.changed();
        if let Some(store) = &self.store {
            for path in &gone {
                let _ = store.clear_folder(&account, path);
            }
            let _ = store.save_folders(&account, &self.account(&account).map(|a| a.folders.clone()).unwrap_or_default());
        }
        self.save_accounts();
        if matches!(&self.view, View::Folder { account: va, folder } if *va == account && gone.contains(folder)) {
            let inbox = self.account(&account).and_then(|a| a.folder(FolderRole::Inbox)).map(|f| f.path.clone()).unwrap_or_else(|| "INBOX".into());
            self.open_view(View::Folder { account: account.clone(), folder: inbox });
        }
        if !self.prefs.work_offline {
            self.jobs.push(Job::Start { account: config, password: None });
        }
        let _ = self.close_shared_calendar(&email);
        Ok(serde_json::json!({ "closed": email }))
    }
}
