// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! An account's worker: the server's own mail rules (Exchange's inbox rules, ManageSieve's
//! script), put there, read back and changed one at a time.

use super::*;

impl Worker {
    /// Put the account's server-marked rules on its server (Exchange's inbox rules), so mail is
    /// filed while Quatrelle is closed, and remove the ones turned off or deleted here.
    pub(crate) fn set_server_rules(&mut self, rules: Vec<quatrelle_model::organize::Rule>, delete: Vec<String>, categories: Vec<(String, String)>) {
        use quatrelle_model::organize::RuleAction;
        let r = (|| -> Result<(Vec<(String, String)>, Vec<quatrelle_model::organize::ForeignRule>)> {
            if rules.is_empty() && delete.is_empty() {
                return Ok((Vec::new(), Vec::new()));
            }
            if self.conn.is_none() {
                self.connect()?;
            }
            // An IMAP account keeps them in its Sieve script, through ManageSieve: the script
            // holds the whole set, so it is written again (nothing to remove by id).
            if matches!(self.conn, Some(Conn::Imap(_))) {
                let (wanted, ids) = self.sieve_rules(&rules);
                let secret = self.secret()?;
                let user = if self.acc.username.is_empty() { self.acc.email.clone() } else { self.acc.username.clone() };
                let mut s = crate::sieve::Client::connect(&self.acc.imap, self.sieve_port).map_err(|e| match e {
                    NetError::Io(_) => NetError::No("this server doesn't take inbox rules from Quatrelle (no ManageSieve)".into()),
                    e => e,
                })?;
                if self.acc.oauth.is_some() {
                    s.login_xoauth2(&user, &secret)?;
                } else {
                    s.login(&user, &secret)?;
                }
                let written = s.set_rules(wanted, categories);
                let listed = s.list();
                s.logout();
                written?;
                let (names, _) = listed?;
                let others = names
                    .iter()
                    .filter(|n| n.as_str() != crate::sieve::SCRIPT)
                    .map(|n| quatrelle_model::organize::ForeignRule {
                        account: self.acc.id.clone(),
                        id: format!("sieve:{n}"),
                        name: n.clone(),
                        enabled: true,
                        runnable: false,
                        rule: None,
                        summary: "a Sieve script".into(),
                    })
                    .collect();
                return Ok((ids, others));
            }
            // The folder each rule files mail into, by the id the server knows it by.
            let mut wanted: Vec<(quatrelle_model::organize::Rule, Option<String>)> = Vec::new();
            for rule in rules.iter().filter(|r| r.server && r.server_ready()) {
                let folder: Option<&str> = rule.actions.iter().find_map(|a| match a {
                    RuleAction::MoveTo(path) => Some(path.as_str()),
                    RuleAction::MoveToRole(role) => self.folders.iter().find(|f| f.role == *role && f.mailbox.is_none()).map(|f| f.path.as_str()),
                    _ => None,
                });
                let moves = rule.actions.iter().any(|a| matches!(a, RuleAction::MoveTo(_) | RuleAction::MoveToRole(_)));
                let id = match folder.map(|path| self.jstate.mailbox(path).map(str::to_string)) {
                    Some(Ok(id)) => Some(id),
                    // Its folder isn't on the server (any more), or the mailbox has none of that
                    // kind (no Archive): keeping the rule here is better than filing mail nowhere —
                    // sent without its move, it reached the server doing nothing.
                    Some(Err(_)) | None if moves => {
                        self.notice(format!("the rule \"{}\" waits: its folder isn't on the server", rule.name));
                        continue;
                    }
                    Some(Err(_)) | None => None,
                };
                wanted.push((rule.clone(), id));
            }
            let ids = match self.conn.as_mut() {
                Some(Conn::Ews(c)) => c.set_inbox_rules(&wanted, &delete),
                Some(Conn::Graph(c)) => c.set_message_rules(&wanted, &delete),
                _ => Err(NetError::No("this server's rules can't be changed from Quatrelle".into())),
            }?;
            // What else the server runs: made in Outlook, or on its own web page.
            let all = match self.conn.as_ref() {
                Some(Conn::Ews(c)) => c.inbox_rules()?,
                Some(Conn::Graph(c)) => c.message_rules()?,
                _ => Vec::new(),
            };
            let mine: Vec<String> = ids.iter().map(|(_, id)| id.clone()).collect();
            let (_, mut others) = split_rules(all, &mine);
            for r in &mut others {
                r.account.clone_from(&self.acc.id);
            }
            self.resolve_rule_folders(&mut others);
            Ok((ids, others))
        })();
        match r {
            Ok((ids, others)) => self.sink.emit(Event::ServerRules { account: self.acc.id.clone(), ids, others, cleared: delete, error: None }),
            Err(e) => {
                // What the server said, in the log too: a notice alone left a refused rule
                // unexplained once it was gone from the screen.
                log::warn!("{}: the server's rules weren't changed: {e}", self.acc.id);
                self.sink.emit(Event::ServerRules {
                    account: self.acc.id.clone(),
                    ids: Vec::new(),
                    others: Vec::new(),
                    cleared: Vec::new(),
                    error: Some(e.to_string()),
                });
            }
        }
    }

    /// What the account's server keeps: the ids Quatrelle's own rules have there (`ours` is what
    /// it holds), the names of the ones it didn't make, and its own the server no longer has.
    pub(crate) fn read_server_rules(
        &mut self,
        ours: Vec<String>,
    ) -> Result<(Vec<(String, String)>, Vec<quatrelle_model::organize::ForeignRule>, Vec<String>)> {
        if self.conn.is_none() {
            self.connect()?;
        }
        if matches!(self.conn, Some(Conn::Pop)) {
            return Err(NetError::No("POP accounts keep no rules on their server".into()));
        }
        if matches!(self.conn, Some(Conn::Imap(_))) {
            let secret = self.secret()?;
            let user = if self.acc.username.is_empty() { self.acc.email.clone() } else { self.acc.username.clone() };
            let mut s = crate::sieve::Client::connect(&self.acc.imap, self.sieve_port).map_err(|e| match e {
                NetError::Io(_) => NetError::No("this server keeps no rules for Quatrelle (no ManageSieve)".into()),
                e => e,
            })?;
            if self.acc.oauth.is_some() {
                s.login_xoauth2(&user, &secret)?;
            } else {
                s.login(&user, &secret)?;
            }
            let listed = s.list();
            s.logout();
            let (names, _) = listed?;
            // Our script holds our rules; without it, they're gone from the server.
            let own = names.iter().any(|n| n == crate::sieve::SCRIPT);
            let ids =
                if own { ours.iter().map(|id| (id.strip_prefix("sieve:").unwrap_or(id).to_string(), id.clone())).collect() } else { Vec::new() };
            let cleared = if own { Vec::new() } else { ours };
            let others = names
                .iter()
                .filter(|n| n.as_str() != crate::sieve::SCRIPT)
                .map(|n| quatrelle_model::organize::ForeignRule {
                    account: self.acc.id.clone(),
                    id: format!("sieve:{n}"),
                    name: n.clone(),
                    enabled: true,
                    runnable: false,
                    rule: None,
                    summary: "a Sieve script".into(),
                })
                .collect();
            return Ok((ids, others, cleared));
        }
        let all = match self.conn.as_ref() {
            Some(Conn::Ews(c)) => c.inbox_rules()?,
            Some(Conn::Graph(c)) => c.message_rules()?,
            _ => return Err(NetError::No("this server keeps no inbox rules".into())),
        };
        let on_server: Vec<&str> = all.iter().map(|r| r.id.as_str()).collect();
        let cleared = ours.iter().filter(|id| !on_server.contains(&id.as_str())).cloned().collect();
        let (ids, mut others) = split_rules(all, &ours);
        for r in &mut others {
            r.account.clone_from(&self.acc.id);
        }
        self.resolve_rule_folders(&mut others);
        Ok((ids, others, cleared))
    }

    /// The folder path behind the id a server's rule files mail into (Exchange names folders by
    /// id, Quatrelle by path).
    pub(crate) fn path_of_folder_id(&self, id: &str) -> Option<String> {
        self.folders
            .iter()
            .filter(|f| f.mailbox.is_none())
            .find(|f| self.jstate.mailboxes.get(&f.path).is_some_and(|x| x == id))
            .map(|f| f.path.clone())
    }

    /// A server rule's `MoveTo` names a folder by the id the server knows; in Quatrelle's terms
    /// it's the folder's path, so the rule can be shown and changed here.
    pub(crate) fn resolve_rule_folders(&self, rules: &mut [quatrelle_model::organize::ForeignRule]) {
        for f in rules.iter_mut() {
            let Some(rule) = &mut f.rule else { continue };
            for a in &mut rule.actions {
                if let quatrelle_model::organize::RuleAction::MoveTo(path) = a
                    && let Some(id) = path.strip_prefix("id:")
                    && let Some(found) = self.path_of_folder_id(id)
                {
                    *path = found;
                }
            }
        }
    }

    /// Change one of the server's own rules (one Quatrelle didn't make): only that rule is sent,
    /// with the place it already has there, so the other rules and their order stay as they are.
    pub(crate) fn edit_server_rule(&mut self, id: &str, rule: &quatrelle_model::organize::Rule, ours: Vec<String>) {
        use quatrelle_model::organize::RuleAction;
        let r = (|| -> Result<()> {
            if self.conn.is_none() {
                self.connect()?;
            }
            let mut rule = rule.clone();
            rule.remote_id = Some(id.to_string());
            rule.server = true;
            let folder: Option<String> = match rule.actions.iter().find_map(|a| match a {
                RuleAction::MoveTo(path) => Some(path.clone()),
                RuleAction::MoveToRole(role) => self.folders.iter().find(|f| f.role == *role && f.mailbox.is_none()).map(|f| f.path.clone()),
                _ => None,
            }) {
                Some(path) => Some(match path.strip_prefix("id:") {
                    Some(id) => id.to_string(),
                    None => self.jstate.mailbox(&path)?.to_string(),
                }),
                None => None,
            };
            let wanted = [(rule, folder)];
            match self.conn.as_mut() {
                Some(Conn::Ews(c)) => c.set_inbox_rules(&wanted, &[]).map(|_| ()),
                Some(Conn::Graph(c)) => c.set_message_rules(&wanted, &[]).map(|_| ()),
                _ => Err(NetError::No("this server's rules can't be changed from Quatrelle".into())),
            }
        })();
        match r {
            Ok(()) => match self.read_server_rules(ours) {
                Ok((ids, others, cleared)) => self.sink.emit(Event::ServerRules { account: self.id(), ids, others, cleared, error: None }),
                Err(e) => self.sink.emit(Event::ServerRules {
                    account: self.id(),
                    ids: Vec::new(),
                    others: Vec::new(),
                    cleared: Vec::new(),
                    error: Some(e.to_string()),
                }),
            },
            Err(e) => self.sink.emit(Event::ServerRules {
                account: self.id(),
                ids: Vec::new(),
                others: Vec::new(),
                cleared: Vec::new(),
                error: Some(e.to_string()),
            }),
        }
    }

    /// Turn one of the server's own rules on or off, or remove it; then read them all back, so
    /// the dialog shows what the server now keeps.
    pub(crate) fn change_server_rule(&mut self, id: &str, enabled: Option<bool>, delete: bool, ours: Vec<String>) {
        let r = (|| -> Result<()> {
            if self.conn.is_none() {
                self.connect()?;
            }
            match self.conn.as_ref() {
                Some(Conn::Ews(c)) => {
                    if delete {
                        c.delete_rule(id)
                    } else {
                        c.set_rule_enabled(id, enabled.unwrap_or(true))
                    }
                }
                Some(Conn::Graph(c)) => {
                    if delete {
                        c.delete_rule(id)
                    } else {
                        c.set_rule_enabled(id, enabled.unwrap_or(true))
                    }
                }
                _ => Err(NetError::No("this server's rules can't be changed from Quatrelle".into())),
            }
        })();
        match r {
            Ok(()) => match self.read_server_rules(ours) {
                Ok((ids, others, cleared)) => self.sink.emit(Event::ServerRules { account: self.id(), ids, others, cleared, error: None }),
                Err(e) => self.sink.emit(Event::ServerRules {
                    account: self.id(),
                    ids: Vec::new(),
                    others: Vec::new(),
                    cleared: Vec::new(),
                    error: Some(e.to_string()),
                }),
            },
            Err(e) => self.sink.emit(Event::ServerRules {
                account: self.id(),
                ids: Vec::new(),
                others: Vec::new(),
                cleared: Vec::new(),
                error: Some(e.to_string()),
            }),
        }
    }

    /// The rules an IMAP account's Sieve script gets, their destinations resolved to the
    /// server's folder names, and the id kept for each (the script holds the rules by name).
    pub(crate) fn sieve_rules(&self, rules: &[quatrelle_model::organize::Rule]) -> (Vec<quatrelle_model::organize::Rule>, Vec<(String, String)>) {
        use quatrelle_model::organize::RuleAction;
        let mut wanted = Vec::new();
        let mut ids = Vec::new();
        for rule in rules.iter().filter(|r| r.server && r.sieve_ready()) {
            if !rule.enabled {
                // Off: it isn't in the script. Its id still comes back, so it isn't sent again
                // until it changes.
                ids.push((rule.name.clone(), format!("sieve:{}", rule.name)));
                continue;
            }
            let mut kept = rule.clone();
            let mut actions = Vec::new();
            let mut missing = false;
            for a in &rule.actions {
                match a {
                    RuleAction::MoveTo(path) => {
                        if self.folders.iter().any(|f| &f.path == path) {
                            actions.push(a.clone());
                        } else {
                            missing = true;
                        }
                    }
                    RuleAction::MoveToRole(role) => match self.folders.iter().find(|f| f.role == *role && f.mailbox.is_none()) {
                        Some(f) => actions.push(RuleAction::MoveTo(f.path.clone())),
                        None => missing = true,
                    },
                    // Deleting is filing in Deleted Items, as Quatrelle does here.
                    RuleAction::Delete => match self.folders.iter().find(|f| f.role == quatrelle_model::FolderRole::Trash && f.mailbox.is_none()) {
                        Some(f) => actions.push(RuleAction::MoveTo(f.path.clone())),
                        None => missing = true,
                    },
                    other => actions.push(other.clone()),
                }
            }
            // Its folder isn't on the server (any more): keeping the rule here is better than
            // filing mail nowhere.
            if missing {
                self.notice(format!("the rule \"{}\" waits: its folder isn't on the server", rule.name));
                continue;
            }
            kept.actions = actions;
            ids.push((rule.name.clone(), format!("sieve:{}", rule.name)));
            wanted.push(kept);
        }
        (wanted, ids)
    }
}
