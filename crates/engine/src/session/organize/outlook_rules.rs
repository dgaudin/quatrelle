//! The session: the copy of the rules Outlook keeps in an Exchange mailbox, which makes Exchange
//! refuse every change of them while it's there, replaced when the user says so.

use super::*;

/// What Exchange's refusal says, in the network layer's words.
const OUTLOOK_COPY: &str = "Outlook keeps its own copy";

impl Session {
    /// Whether Exchange refused to change this account's rules because of Outlook's copy of them.
    pub fn outlook_keeps_rules(&self, account: &str) -> bool {
        self.rules_refused_told.get(account).is_some_and(|w| w.contains(OUTLOOK_COPY))
    }

    /// Replace Outlook's copy of an Exchange account's rules (the user agreed to lose the rules
    /// Outlook runs only on its computer): every rule of the account meant for the server goes
    /// again, the first change removing the copy.
    pub fn replace_outlook_rules(&mut self, account: &str) -> CmdResult {
        if self.account(account).is_none_or(|a| a.config.ews.is_none()) {
            return Err(CmdError::Params("only an Exchange account's mailbox keeps Outlook's copy of the rules".into()));
        }
        let mut changed = false;
        for r in self.rules.iter_mut().filter(|r| r.account.as_deref() == Some(account)) {
            if r.refused.as_deref().is_some_and(|w| w.contains(OUTLOOK_COPY)) {
                r.refused = None;
                changed = true;
            }
        }
        if changed {
            self.save_rules();
        }
        self.rules_refused_told.remove(account);
        self.jobs.push(Job::ReplaceOutlookRules { account: account.to_string() });
        let rules: Vec<Rule> =
            self.rules.iter().filter(|r| r.server && r.account.as_deref() == Some(account) && r.refused.is_none()).cloned().collect();
        let n = rules.len();
        if !rules.is_empty() {
            self.jobs.push(Job::SetServerRules { account: account.to_string(), rules, delete: Vec::new(), categories: self.rule_keywords() });
        }
        Ok(serde_json::json!({"replacing": account, "rules": n}))
    }
}
