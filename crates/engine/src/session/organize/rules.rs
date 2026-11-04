// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Inbox rules: the list kept on this computer, the ones a server runs too (Sieve, Exchange),
//! and a rules file brought in or taken out.

use super::*;

/// What a server would be told for these rules (Quatrelle's ids left out): comparing it says
/// whether a change is worth a round trip to the server.
fn server_shape(rules: &[Rule], kind: &impl Fn(&str) -> Option<ServerKind>) -> Vec<String> {
    rules
        .iter()
        .filter(|r| r.server && r.account.as_deref().and_then(kind).is_some_and(|k| r.server_ready_for(k)))
        .map(|r| format!("{:?}|{}|{}|{:?}|{:?}|{}", r.account, r.name, r.enabled, r.conditions, r.actions, r.stop))
        .collect()
}

impl Session {
    pub fn set_rules(&mut self, rules: Vec<Rule>) -> CmdResult {
        if rules.len() > 500 {
            return Err(CmdError::Params("at most 500 rules".into()));
        }
        for r in &rules {
            if r.name.trim().is_empty() {
                return Err(CmdError::Params("every rule needs a name".into()));
            }
            for a in &r.actions {
                if let RuleAction::Categorize(c) = a
                    && self.category(c).is_none()
                {
                    return Err(CmdError::Params(format!("rule {}: no category called {c}", r.name)));
                }
            }
        }
        let mut rules = rules;
        // What goes to the server has to be something it can run (Exchange's rules, or an IMAP
        // account's Sieve): what it can't stays here (as Outlook's client-only rules do), and
        // the user is told.
        let mut kept_home: Vec<String> = Vec::new();
        for r in &mut rules {
            let ready = r.account.as_deref().and_then(|a| self.server_kind(a)).is_some_and(|kind| r.server_ready_for(kind));
            if r.server && !ready {
                r.server = false;
                kept_home.push(r.name.clone());
            }
        }
        match kept_home.as_slice() {
            [] => {}
            [one] => self.notice_with("{rule} stays on this computer: the server can't run it", &[("rule", one)], false),
            more => self.notice_with("{rules} stay on this computer: the server can't run them", &[("rules", &more.join(", "))], false),
        }
        // The server copies that must go: rules removed here, turned off, or no longer sent.
        let mut delete: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for old in &self.rules {
            let Some(id) = old.remote_id.as_deref().filter(|id| !id.is_empty()) else { continue };
            if rules.iter().any(|r| r.server && r.remote_id.as_deref() == Some(id)) {
                continue;
            }
            if let Some(account) = old.account.as_deref() {
                delete.entry(account.to_string()).or_default().push(id.to_string());
            }
        }
        for r in &mut rules {
            if !r.server {
                r.remote_id = None;
                // Kept here only: nothing to be refused (turned on again, it's tried afresh).
                r.refused = None;
            }
        }
        let before = server_shape(&self.rules, &|a| self.server_kind(a));
        self.rules = rules;
        if let Some(store) = &self.store
            && let Err(e) = store.save_doc("rules.json", &self.rules)
        {
            return Err(CmdError::Failed(format!("the rules couldn't be saved: {e}")));
        }
        // Tell each server, when there is something to say: a change to what runs there, or a
        // rule to remove there.
        if before != server_shape(&self.rules, &|a| self.server_kind(a)) || !delete.is_empty() {
            let mut accounts: BTreeSet<String> = delete.keys().cloned().collect();
            for r in &self.rules {
                if r.server
                    && let Some(a) = &r.account
                {
                    accounts.insert(a.clone());
                }
            }
            for account in accounts {
                let wanted: Vec<Rule> = self.rules.iter().filter(|r| r.server && r.account.as_deref() == Some(&account)).cloned().collect();
                let remove = delete.remove(&account).unwrap_or_default();
                if !wanted.is_empty() || !remove.is_empty() {
                    self.jobs.push(Job::SetServerRules { account, rules: wanted, delete: remove, categories: self.rule_keywords() });
                }
            }
        }
        Ok(serde_json::json!({ "rules": self.rules.len() }))
    }

    /// The categories by name, with the keyword each is kept as (a rule run by a server adds
    /// the keyword of the category it files mail in).
    pub(super) fn rule_keywords(&self) -> Vec<(String, String)> {
        self.prefs.categories.iter().map(|c| (c.name.clone(), c.keyword.clone())).collect()
    }

    /// Where an account's rules can also run beside this computer (`None`: its server keeps no
    /// rules at all — JMAP, POP). An IMAP account's are its Sieve script, through ManageSieve.
    pub fn server_kind(&self, account: &str) -> Option<ServerKind> {
        let a = self.account(account)?;
        if a.config.graph.is_some() || a.config.ews.is_some() {
            Some(ServerKind::Exchange)
        } else if a.config.jmap.is_some() || a.config.pop.is_some() {
            None
        } else {
            Some(ServerKind::Sieve)
        }
    }

    /// Keep the rules on disk, without telling any server (the server's answer brings the ids
    /// back through `Event::ServerRules`, which lands here).
    pub(crate) fn save_rules(&mut self) {
        if let Some(store) = &self.store
            && let Err(e) = store.save_doc("rules.json", &self.rules)
        {
            self.notice_with("the rules couldn't be saved: {error}", &[("error", &e.to_string())], true);
        }
    }

    /// Read the rules the account's server keeps (`GetServerRules`): the ids Quatrelle's own have
    /// there, and the ones made elsewhere. Accounts whose server keeps none are left alone.
    pub(crate) fn refresh_server_rules(&mut self, account: &str) {
        if self.server_kind(account).is_none() {
            return;
        }
        let ours: Vec<String> = self.rules.iter().filter(|r| r.account.as_deref() == Some(account)).filter_map(|r| r.remote_id.clone()).collect();
        self.jobs.push(Job::GetServerRules { account: account.to_string(), ours });
    }

    /// The server wouldn't change its rules: said once per account and reason a session (not
    /// on every try: the rules are sent again at each connection), and the rules sent there
    /// that it refused outright are marked with why — Exchange refuses every change while
    /// Outlook keeps its own copy of the rules, and a rule sent again on every connection
    /// forever was refused forever. A refusal that may pass (the network, a busy server) marks
    /// nothing: the rule goes again next time.
    pub(crate) fn server_rules_refused(&mut self, account: &str, why: String) {
        if why.starts_with("the server refused") {
            let mut changed = false;
            for r in self.rules.iter_mut().filter(|r| r.server && r.account.as_deref() == Some(account) && r.remote_id.is_none()) {
                if r.refused.as_deref() != Some(why.as_str()) {
                    r.refused = Some(why.clone());
                    changed = true;
                }
            }
            if changed {
                self.save_rules();
            }
        }
        if self.rules_refused_told.get(account) != Some(&why) {
            self.rules_refused_told.insert(account.to_string(), why.clone());
            self.notice_with("the server's rules weren't changed: {error}", &[("error", &why)], true);
        }
    }

    /// Rules marked for the server that never got there (the account was offline when they were
    /// made): send them now that it's connected. One the server refused outright waits to be
    /// changed here, or sent again by hand (`rules.set`): sent on every connection, it was
    /// refused on every connection, with the same notice each time.
    pub(crate) fn resend_server_rules(&mut self, account: &str) {
        let pending: Vec<Rule> = self
            .rules
            .iter()
            .filter(|r| r.server && r.account.as_deref() == Some(account) && r.remote_id.is_none() && r.refused.is_none())
            .cloned()
            .collect();
        if pending.is_empty() {
            return;
        }
        self.jobs.push(Job::SetServerRules { account: account.to_string(), rules: pending, delete: Vec::new(), categories: self.rule_keywords() });
    }

    /// The ids Quatrelle's own rules have on that account's server (what tells the server's
    /// rules apart from the ones it didn't make).
    pub(super) fn our_server_ids(&self, account: &str) -> Vec<String> {
        self.rules.iter().filter(|r| r.account.as_deref() == Some(account)).filter_map(|r| r.remote_id.clone()).collect()
    }

    /// Turn one of the server's own rules (made in Outlook, on a web page) on or off, there.
    pub fn set_server_rule_enabled(&mut self, account: &str, id: &str, enabled: bool) -> CmdResult {
        let found = self.foreign_rules.iter().any(|r| r.account == account && r.id == id);
        if !found {
            return Err(CmdError::Params(format!("no rule `{id}` on that server")));
        }
        let ours = self.our_server_ids(account);
        self.jobs.push(Job::SetServerRuleEnabled { account: account.to_string(), id: id.to_string(), enabled, ours });
        Ok(serde_json::json!({ "enabled": enabled }))
    }

    /// Remove one of the server's own rules.
    pub fn delete_server_rule(&mut self, account: &str, id: &str) -> CmdResult {
        let found = self.foreign_rules.iter().any(|r| r.account == account && r.id == id);
        if !found {
            return Err(CmdError::Params(format!("no rule `{id}` on that server")));
        }
        let ours = self.our_server_ids(account);
        self.jobs.push(Job::DeleteServerRule { account: account.to_string(), id: id.to_string(), ours });
        Ok(serde_json::json!({ "deleted": id }))
    }

    /// Change one of the server's own rules: its name, whether it runs, what it looks at and
    /// what it does — nothing else (its place among the server's rules stays).
    pub fn save_server_rule(&mut self, account: &str, id: &str, mut rule: quatrelle_model::organize::Rule) -> CmdResult {
        let Some(f) = self.foreign_rules.iter().find(|r| r.account == account && r.id == id) else {
            return Err(CmdError::Params(format!("no rule `{id}` on that server")));
        };
        if !f.runnable {
            return Err(CmdError::Failed(format!("Quatrelle doesn't know every part of \"{}\" yet", f.name)));
        }
        if rule.name.trim().is_empty() {
            return Err(CmdError::Params("a rule needs a name".into()));
        }
        if rule.conditions.is_empty() || rule.actions.is_empty() {
            return Err(CmdError::Params("a rule needs a condition and an action".into()));
        }
        // What the server keeps: its id, its place, and that it runs there.
        rule.remote_id = Some(id.to_string());
        rule.priority = self.foreign_rules.iter().find(|r| r.account == account && r.id == id).and_then(|r| r.rule.as_ref()).and_then(|r| r.priority);
        rule.server = true;
        rule.account = Some(account.to_string());
        let ours = self.our_server_ids(account);
        self.jobs.push(Job::SetServerRule { account: account.to_string(), id: id.to_string(), rule, ours });
        Ok(serde_json::json!({ "saved": id }))
    }

    /// Run one of the server's own rules on the mail already in the Inbox (Outlook's "Run this
    /// rule now"): only for a rule every part of which Quatrelle knows.
    pub fn run_server_rule(&mut self, account: &str, id: &str) -> CmdResult {
        let f = self
            .foreign_rules
            .iter()
            .find(|r| r.account == account && r.id == id)
            .ok_or_else(|| CmdError::Params(format!("no rule `{id}` on that server")))?;
        let rule = f.rule.clone().ok_or_else(|| {
            CmdError::Disabled("Quatrelle can't run this rule here: it uses a condition or an action Quatrelle doesn't know".into())
        })?;
        let inbox = self.account(account).and_then(|a| a.folder(FolderRole::Inbox)).map(|x| x.path.clone());
        let Some(inbox) = inbox else { return Err(CmdError::Disabled("that account has no Inbox yet".into())) };
        let msgs: Vec<Arc<Summary>> = self.folder_state(account, &inbox).messages.iter().filter(|m| !m.flags.deleted).cloned().collect();
        let saved = std::mem::replace(&mut self.rules, vec![Rule { enabled: true, account: Some(account.to_string()), ..rule }]);
        let n = msgs.iter().filter(|m| !actions_for(&self.rules, m).is_empty()).count();
        self.run_rules(&msgs);
        self.rules = saved;
        if n == 1 {
            self.notice("The rule ran on the mail there now: 1 message matched", false);
        } else {
            self.notice_with("The rule ran on the mail there now: {n} messages matched", &[("n", &n.to_string())], false);
        }
        Ok(serde_json::json!({ "matched": n }))
    }

    /// The rules as a file to keep or take to another computer (Outlook's Export Rules): each
    /// rule's account named by its address, which another computer knows it by.
    pub fn rules_file(&self) -> Result<Vec<u8>, CmdError> {
        let rules: Vec<serde_json::Value> = self
            .rules
            .iter()
            .map(|r| {
                let mut v = serde_json::to_value(r).unwrap_or_default();
                if let (Some(id), Some(o)) = (&r.account, v.as_object_mut()) {
                    let email = self.account(id).map(|a| a.config.email.clone()).unwrap_or_default();
                    o.insert("account".into(), serde_json::Value::Null);
                    o.insert("accountEmail".into(), serde_json::Value::String(email));
                }
                v
            })
            .collect();
        let doc = serde_json::json!({ "quatrelleRules": 1, "rules": rules });
        serde_json::to_vec_pretty(&doc).map_err(|e| CmdError::Failed(e.to_string()))
    }

    /// Add the rules in a file made by [`Session::rules_file`] (Outlook's Import Rules) after
    /// those here, or in place of them (`replace`). A rule for an account this computer doesn't
    /// have comes in turned off; a category one files mail under is made when it's missing; a
    /// rule already here (the same in every part) isn't added twice. What came in, and how many
    /// were turned off.
    pub fn import_rules(&mut self, bytes: &[u8], replace: bool) -> CmdResult {
        if bytes.len() > 4 << 20 {
            return Err(CmdError::Failed("that file is too big for a rules file".into()));
        }
        // A file saved again by an editor may start with a byte order mark.
        let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        let doc: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| CmdError::Failed("not a Quatrelle rules file".into()))?;
        let list = match (doc.get("quatrelleRules"), doc.get("rules").and_then(serde_json::Value::as_array)) {
            (Some(_), Some(list)) => list.clone(),
            _ => return Err(CmdError::Failed("not a Quatrelle rules file".into())),
        };
        let mut rules = if replace { Vec::new() } else { self.rules.clone() };
        let (mut added, mut off) = (0usize, 0usize);
        for (i, v) in list.into_iter().take(500).enumerate() {
            let email = v.get("accountEmail").and_then(serde_json::Value::as_str).map(str::to_string);
            let mut rule: Rule =
                serde_json::from_value(v).map_err(|e| CmdError::Failed(format!("not a Quatrelle rules file: rule {}: {e}", i + 1)))?;
            rule.account = None;
            if let Some(email) = email.filter(|e| !e.is_empty()) {
                match self.accounts.iter().find(|a| a.config.email.eq_ignore_ascii_case(&email)) {
                    Some(a) => rule.account = Some(a.config.id.clone()),
                    // Kept for that account only: off until it's chosen again.
                    None => {
                        rule.enabled = false;
                        off += 1;
                    }
                }
            }
            for a in &rule.actions {
                if let RuleAction::Categorize(c) = a
                    && self.category(c).is_none()
                {
                    self.add_category(c, None)?;
                }
            }
            if !rules.contains(&rule) {
                rules.push(rule);
                added += 1;
            }
        }
        self.set_rules(rules)?;
        Ok(serde_json::json!({ "added": added, "turnedOff": off, "rules": self.rules.len() }))
    }
}
