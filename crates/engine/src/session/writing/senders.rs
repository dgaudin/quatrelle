//! Who a message is from, as Outlook's From button offers it: the account's own address, a
//! mailbox it opens (a shared one), or another address it may send from. Exchange sends as that
//! address, or on its behalf, as the account is allowed to; other servers decide for themselves.

use super::*;

/// Other addresses an account remembers having sent from.
const KEPT_SENDERS: usize = 10;

/// A choice of the From button.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sender {
    pub account: String,
    /// `Name <address>`, or empty for the account's own address.
    pub from: String,
    /// The address alone.
    pub email: String,
    /// As the menu shows it.
    pub label: String,
}

/// The one address `text` holds, if it holds just one.
fn one_address(text: &str) -> Option<EmailAddress> {
    addr::parse_list(text).ok().filter(|l| l.len() == 1).and_then(|l| l.into_iter().next())
}

impl Session {
    /// What the From button offers: each account's own address, then the mailboxes it opens and
    /// the other addresses it sent from.
    pub fn senders(&self) -> Vec<Sender> {
        let mut out = Vec::new();
        for a in &self.accounts {
            let account = a.config.id.clone();
            out.push(Sender {
                account: account.clone(),
                from: String::new(),
                email: a.config.email.clone(),
                label: EmailAddress::new(a.config.name.clone(), a.config.email.clone()).to_string(),
            });
            let mut seen = vec![a.config.email.to_ascii_lowercase()];
            let opened = a.config.shared_mailboxes.iter().cloned().chain(a.folders.iter().filter_map(|f| f.mailbox.clone()));
            for from in opened.chain(a.config.send_from.iter().cloned()) {
                let Some(email) = one_address(&from).map(|e| e.email) else { continue };
                if seen.contains(&email.to_ascii_lowercase()) {
                    continue;
                }
                seen.push(email.to_ascii_lowercase());
                out.push(Sender { account: account.clone(), label: from.clone(), from, email });
            }
        }
        out
    }

    /// The address `c` goes from: the one chosen, else its account's.
    pub fn compose_from(&self, c: &Compose) -> EmailAddress {
        one_address(&c.from)
            .or_else(|| self.account(&c.account).map(|a| EmailAddress::new(a.config.name.clone(), a.config.email.clone())))
            .unwrap_or_default()
    }

    /// Send the message being written from `from` (`Name <address>`; empty: the account's own
    /// address) through `account` (default: its own).
    pub fn set_compose_from(&mut self, account: Option<&str>, from: &str) -> CmdResult {
        if let Some(a) = account
            && self.account(a).is_none()
        {
            return Err(CmdError::Params(format!("no account `{a}`")));
        }
        let from = from.trim().trim_end_matches([';', ',']).trim();
        let address =
            if from.is_empty() { None } else { Some(one_address(from).ok_or_else(|| CmdError::Params(format!("not an email address: {from}")))?) };
        let c = self.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
        if c.sending {
            return Err(CmdError::Disabled("the message is being sent".into()));
        }
        if let Some(a) = account {
            c.account = a.to_string();
        }
        let own = self.accounts.iter().find(|a| a.config.id == c.account).map(|a| a.config.email.clone()).unwrap_or_default();
        c.from = match address {
            Some(a) if !a.email.eq_ignore_ascii_case(&own) => a.to_string(),
            _ => String::new(),
        };
        Ok(serde_json::json!({ "compose": c.id, "account": c.account, "from": c.from }))
    }

    /// The mailbox a reply to a message in `key`'s folder goes from: that mailbox (a shared
    /// one), named as the message names it, when the folder is another mailbox's.
    pub(super) fn reply_from(&self, key: &MessageKey, p: &mime::Parsed) -> String {
        let Some(mailbox) = self.account(&key.account).and_then(|a| a.folder_mailbox(&key.folder)) else { return String::new() };
        p.to.iter()
            .chain(&p.cc)
            .find(|a| a.email.trim().eq_ignore_ascii_case(&mailbox))
            .map_or_else(|| mailbox.clone(), |a| EmailAddress::new(a.name.clone(), mailbox.clone()).to_string())
    }

    /// `c` went from another address than its account's own: remembered for the From button,
    /// unless it's a mailbox the account opens (offered anyway).
    pub(super) fn remember_sender(&mut self, c: &Compose) {
        let Some(email) = one_address(&c.from).map(|a| a.email) else { return };
        let Some(a) = self.account_mut(&c.account) else { return };
        let opened =
            a.config.shared_mailboxes.iter().chain(a.folders.iter().filter_map(|f| f.mailbox.as_ref())).any(|m| m.eq_ignore_ascii_case(&email));
        if opened || a.config.send_from.first().is_some_and(|f| *f == c.from) {
            return;
        }
        a.config.send_from.retain(|f| one_address(f).is_none_or(|x| !x.email.eq_ignore_ascii_case(&email)));
        a.config.send_from.insert(0, c.from.clone());
        a.config.send_from.truncate(KEPT_SENDERS);
        self.save_accounts();
    }
}
