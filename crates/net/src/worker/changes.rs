//! An account's worker: the jobs it's handed, mailbox settings, and changes made offline replayed.

use super::*;

impl Worker {
    pub(super) fn handle(&mut self, job: Job) {
        match job {
            Job::SyncAll { .. } => {
                if self.conn.is_none() {
                    self.auth_failed = false;
                    self.next_attempt = Instant::now();
                    self.connect_and_sync();
                }
                self.last_full = None;
            }
            Job::SyncFolder { folder, .. } => {
                self.read_followed();
                if self.conn.is_none() {
                    self.next_attempt = Instant::now();
                    self.connect_and_sync();
                }
                if self.conn.is_some() {
                    self.sync_one(&folder);
                }
            }
            Job::MoreMail { folder, more, .. } => {
                let kept = self.store.load_folder(&self.acc.id, &folder).map(|f| f.messages.len()).unwrap_or(0);
                self.read_followed();
                let now = self.options_for(&folder).window.max(kept);
                self.windows.insert(folder.clone(), now.saturating_add(more as usize).min(1_000_000));
                if let Err(e) = self.store.save_account_doc(&self.acc.id, WINDOWS, &self.windows) {
                    log::warn!("{}: {WINDOWS}: {e}", self.acc.id);
                }
                if self.conn.is_none() {
                    self.next_attempt = Instant::now();
                    self.connect_and_sync();
                }
                if self.conn.is_some() {
                    self.sync_one(&folder);
                }
            }
            Job::SearchServer { folders, query, search, .. } => self.search_server(&folders, query, search),
            Job::FetchBody { key } => self.fetch(key),
            Job::Older { folder, offset, .. } => self.older(&folder, offset),
            Job::ReadOnline { key } => self.read_online_body(key),
            Job::Search { query, folder, .. } => self.search(query, folder),
            // Already in the cache and ops.json (the pool wrote them): push the queue.
            Job::Apply { op, .. } if self.acc.pop.is_none() => {
                if op.is_folder_op() {
                    self.folders = self.store.load_folders(&self.acc.id).unwrap_or_default();
                }
                if self.conn.is_some() {
                    self.replay();
                }
            }
            Job::Vote { key, choice } => self.vote(key, choice),
            Job::Apply { op, .. } => {
                // POP: a move is carried out here (there's no server copy to come back); what's
                // deleted for good is noted, for removing the server's copy when the account says.
                {
                    let gone: Vec<quatrelle_model::MessageKey> = match &op {
                        Op::Expunge { keys } => keys.clone(),
                        Op::EmptyFolder { path } => {
                            self.store.load_folder(&self.acc.id, path).map(|f| f.messages.into_iter().map(|m| m.key).collect()).unwrap_or_default()
                        }
                        _ => Vec::new(),
                    };
                    if let Err(e) = crate::pop::deleted_for_good(&self.store, &self.acc.id, &gone) {
                        self.notice(e.to_string());
                    }
                }
                let applied = match &op {
                    Op::Move { keys, to } if self.acc.pop.is_some() => crate::pop::move_locally(&self.store, &self.acc.id, keys, to),
                    _ => self.store.apply_op(&self.acc.id, &op).map_err(NetError::from),
                };
                match applied {
                    Ok(changed) => {
                        if op.is_folder_op() {
                            self.folders = self.store.load_folders(&self.acc.id).unwrap_or_default();
                            self.publish_folders();
                        }
                        for folder in changed {
                            self.sink.emit(Event::FolderChanged { account: self.id(), folder });
                        }
                    }
                    Err(e) => self.notice(e.to_string()),
                }
                if let Err(e) = self.store.add_op(&self.acc.id, op) {
                    self.notice(e.to_string());
                }
                if self.conn.is_some() {
                    self.replay();
                }
            }
            Job::Send { id, raw, from, recipients, sent_folder, .. } => self.send(id, &raw, &from, &recipients, sent_folder),
            // The pool keeps these itself; one reaching a worker is kept the same way.
            Job::KeepDraft { raw, folder, replaces, .. } => {
                if let Err(e) = keep_draft(&self.store, &self.acc.id, &folder, replaces, raw) {
                    log::warn!("{}: the draft couldn't be kept: {e}", self.acc.id);
                }
            }
            Job::SaveDraft { raw, folder, replaces, .. } => {
                let (raw_kept, replaces_kept) = (raw.clone(), replaces.clone());
                let r = (|| -> Result<()> {
                    if self.conn.is_none() {
                        self.connect()?;
                    }
                    if let Some(Conn::Pop) = self.conn.as_ref() {
                        let flags = quatrelle_model::Flags { draft: true, seen: true, ..Default::default() };
                        crate::pop::file_locally(&self.store, &self.acc.id, &folder, &raw, flags, now())?;
                        if let Some(old) = replaces {
                            self.store.apply_op(&self.acc.id, &Op::Expunge { keys: vec![old] })?;
                        }
                        return Ok(());
                    }
                    if let Some(Conn::Jmap(c)) = self.conn.as_mut() {
                        jmap::append(c, &self.jstate, &folder, &raw, &["$draft", "$seen"])?;
                        if let Some(old) = replaces {
                            jmap::apply_remote(c, &self.jstate, &Op::Expunge { keys: vec![old] })?;
                        }
                        return Ok(());
                    }
                    if let Some(Conn::Graph(c)) = self.conn.as_mut() {
                        crate::graph::append(c, &self.jstate, &folder, &raw)?;
                        if let Some(old) = replaces {
                            crate::graph::apply_remote(c, &self.jstate, &Op::Expunge { keys: vec![old] })?;
                        }
                        return Ok(());
                    }
                    if let Some(Conn::Ews(c)) = self.conn.as_mut() {
                        crate::ews::append(c, &self.jstate, &folder, &raw)?;
                        if let Some(old) = replaces {
                            crate::ews::apply_remote(c, &self.jstate, &Op::Expunge { keys: vec![old] })?;
                        }
                        return Ok(());
                    }
                    let trash = self.folders.iter().find(|f| f.role == FolderRole::Trash).map(|f| f.path.clone());
                    let c = self.client()?;
                    c.append(&folder, "\\Draft \\Seen", &raw)?;
                    if let Some(old) = replaces {
                        expunge_keys(c, &[old], trash.as_deref())?;
                    }
                    Ok(())
                })();
                match r {
                    Ok(()) => self.sync_one(&folder),
                    Err(e) => {
                        // Not lost: kept on this computer and saved when the account connects.
                        match keep_draft(&self.store, &self.acc.id, &folder, replaces_kept, raw_kept) {
                            Ok(()) => self.notice(
                                "the draft couldn't be saved to the server; it's kept on this computer and saved there when the account connects",
                            ),
                            Err(k) => self.notice(format!("the draft couldn't be saved: {k}")),
                        }
                        self.failed(e);
                    }
                }
            }
            Job::FlushOutbox { .. } => self.flush_outbox(),
            Job::SyncDav { .. } => self.sync_dav(),
            Job::GetVacation { .. } => {
                let r = self.vacation(None);
                self.report_vacation(r, None);
            }
            Job::SetVacation { vacation, .. } => {
                let r = self.vacation(Some(&vacation));
                match r {
                    Ok(v) => self.report_vacation(Ok(v), None),
                    // Show what the server still has, with why the change failed.
                    Err(e) => {
                        let back = self.vacation(None);
                        self.report_vacation(back, Some(e.to_string()));
                    }
                }
            }
            Job::SetServerRules { rules, delete, categories, .. } => self.set_server_rules(rules, delete, categories),
            Job::GetServerRules { ours, .. } => match self.read_server_rules(ours) {
                Ok((ids, others, cleared)) => {
                    log::info!("{}: the server keeps {} rules ({} of ours, {} others)", self.id(), ids.len() + others.len(), ids.len(), others.len());
                    self.sink.emit(Event::ServerRules { account: self.id(), ids, others, cleared, error: None });
                }
                // Read by itself on connecting: one that fails (the server busy, a connection
                // dropped) is only logged — the list kept is still right, and a red "the server's
                // rules weren't changed" for a read nobody asked for was one more false alarm.
                Err(e) => {
                    log::warn!("{}: the server's rules couldn't be read: {e}", self.id());
                    if e.is_fatal() || matches!(e, NetError::Auth(_)) {
                        self.failed(e);
                    }
                }
            },
            Job::SetServerRuleEnabled { id, enabled, ours, .. } => self.change_server_rule(&id, Some(enabled), false, ours),
            Job::SetServerRule { id, rule, ours, .. } => self.edit_server_rule(&id, &rule, ours),
            Job::DeleteServerRule { id, ours, .. } => self.change_server_rule(&id, None, true, ours),
            Job::FetchPhoto { email, contact, .. } => self.fetch_photo(&email, contact.as_deref()),
            Job::GetRooms { .. } => self.rooms(),
            Job::GetAvailability { emails, from, to, .. } => self.availability(&emails, from, to),
            Job::ServerFolderSizes { .. } => self.folder_sizes(),
            Job::FetchUrl { .. }
            | Job::Discover { .. }
            | Job::SignIn { .. }
            | Job::ImportPst { .. }
            | Job::CompactDataFile { .. }
            | Job::LocalFolderSizes { .. }
            | Job::ExportPst { .. }
            | Job::ImportIdentity { .. }
            | Job::ForgetIdentity { .. }
            | Job::Protect { .. }
            | Job::Decrypt { .. }
            | Job::SealDraft { .. }
            | Job::CheckRevocation { .. }
            | Job::FindHeldKeys { .. }
            | Job::UseHeldKey { .. }
            | Job::CheckUpdates
            | Job::StopSearch => {}
            Job::Start { .. } | Job::SmtpPassword { .. } | Job::Remove { .. } | Job::WorkOffline { .. } | Job::SetSync { .. } => {}
        }
    }

    /// Put the account's server-marked rules on its server (Exchange's inbox rules), so mail is
    /// filed while Quatrelle is closed, and remove the ones turned off or deleted here.
    pub(super) fn set_server_rules(&mut self, rules: Vec<quatrelle_model::organize::Rule>, delete: Vec<String>, categories: Vec<(String, String)>) {
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
    pub(super) fn read_server_rules(
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
    pub(super) fn path_of_folder_id(&self, id: &str) -> Option<String> {
        self.folders
            .iter()
            .filter(|f| f.mailbox.is_none())
            .find(|f| self.jstate.mailboxes.get(&f.path).is_some_and(|x| x == id))
            .map(|f| f.path.clone())
    }

    /// A server rule's `MoveTo` names a folder by the id the server knows; in Quatrelle's terms
    /// it's the folder's path, so the rule can be shown and changed here.
    pub(super) fn resolve_rule_folders(&self, rules: &mut [quatrelle_model::organize::ForeignRule]) {
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
    pub(super) fn edit_server_rule(&mut self, id: &str, rule: &quatrelle_model::organize::Rule, ours: Vec<String>) {
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
    pub(super) fn change_server_rule(&mut self, id: &str, enabled: Option<bool>, delete: bool, ours: Vec<String>) {
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

    /// The picture kept for a person: the one their contact item carries (`contact`), or else
    /// the one the directory keeps for their address. None is an answer too (the person simply
    /// has no picture, or the server doesn't keep them).
    pub(super) fn fetch_photo(&mut self, email: &str, contact: Option<&str>) {
        let r = (|| -> Result<(String, Vec<u8>)> {
            if self.conn.is_none() {
                self.connect()?;
            }
            if let Some(id) = contact {
                let own = match self.conn.as_mut() {
                    Some(Conn::Ews(c)) => c.contact_photo(id),
                    Some(Conn::Graph(c)) => c.contact_photo(id),
                    _ => Err(NetError::No("this server keeps no contact pictures".into())),
                };
                // A contact without a picture of its own may still have one in the directory.
                match own {
                    Ok(p) => return Ok(p),
                    Err(NetError::No(_)) => {}
                    Err(e) => return Err(e),
                }
            }
            match self.conn.as_mut() {
                Some(Conn::Ews(c)) => c.user_photo(email),
                Some(Conn::Graph(c)) => c.user_photo(email),
                _ => Err(NetError::No("this server keeps no pictures".into())),
            }
        })();
        let (media, data, error) = match r {
            Ok((media, data)) => (media, data, None),
            Err(e) => {
                let said = e.to_string();
                // The connection lost or the sign-in refused is the account's, as when a message
                // is opened: left unsaid, "Connecting…" stayed up for good.
                if e.is_fatal() || matches!(e, NetError::Auth(_)) {
                    self.failed(e);
                }
                (String::new(), Vec::new(), Some(said))
            }
        };
        // Said in the log either way: "why is there no picture?" is answered without a debugger.
        match &error {
            Some(e) => log::info!("picture of {email}: none ({e})"),
            None => log::info!("picture of {email}: {} bytes, {media}", data.len()),
        }
        self.sink.emit(Event::Photo { email: email.to_string(), media, data, error });
    }

    /// The rules an IMAP account's Sieve script gets, their destinations resolved to the
    /// server's folder names, and the id kept for each (the script holds the rules by name).
    pub(super) fn sieve_rules(&self, rules: &[quatrelle_model::organize::Rule]) -> (Vec<quatrelle_model::organize::Rule>, Vec<(String, String)>) {
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

    /// Read (and first change, when `set`) the automatic replies: JMAP's own, or a Sieve script
    /// through ManageSieve on the IMAP host.
    pub(super) fn vacation(&mut self, set: Option<&quatrelle_model::job::Vacation>) -> Result<quatrelle_model::job::Vacation> {
        // Not connected: one try, and a failure shows as the account's status (it said
        // "Connecting…" for good when the vacation page was opened on an account that can't).
        // (The second connection asks ManageSieve straight away: it needs no IMAP session.)
        if self.conn.is_none() && (self.main.is_none() || self.acc.is_api()) {
            if self.auth_failed {
                return Err(NetError::Refused("it shows once the account is connected".into()));
            }
            if let Err(e) = self.connect() {
                if self.main.is_some() {
                    return Err(e);
                }
                self.connect_failed(e);
                return Err(NetError::Refused("it shows once the account is connected".into()));
            }
        }
        if let Some(Conn::Pop) = &self.conn {
            return Err(NetError::No("POP accounts have no automatic replies on the server".into()));
        }
        if let Some(Conn::Jmap(c)) = &self.conn {
            if let Some(v) = set {
                c.set_vacation(v)?;
            }
            return c.vacation();
        }
        if let Some(Conn::Graph(c)) = &self.conn {
            if let Some(v) = set {
                c.set_vacation(v)?;
            }
            return c.vacation();
        }
        if let Some(Conn::Ews(c)) = &self.conn {
            if let Some(v) = set {
                c.set_vacation(v)?;
            }
            return c.vacation();
        }
        let secret = self.secret()?;
        let user = if self.acc.username.is_empty() { self.acc.email.clone() } else { self.acc.username.clone() };
        let mut s = crate::sieve::Client::connect(&self.acc.imap, self.sieve_port).map_err(|e| match e {
            NetError::Io(_) => NetError::No("this server doesn't offer automatic replies (no ManageSieve)".into()),
            e => e,
        })?;
        if self.acc.oauth.is_some() {
            s.login_xoauth2(&user, &secret)?;
        } else {
            s.login(&user, &secret)?;
        }
        if let Some(v) = set {
            s.set_vacation(v)?;
        }
        let v = s.vacation();
        s.logout();
        v
    }

    pub(super) fn report_vacation(&self, r: Result<quatrelle_model::job::Vacation>, failed: Option<String>) {
        let (vacation, error) = match r {
            Ok(v) => (Some(v), failed),
            Err(e) => (None, Some(failed.unwrap_or_else(|| e.to_string()))),
        };
        self.sink.emit(Event::Vacation { account: self.id(), vacation, error });
    }

    /// Re-apply the edits still waiting that touch `path` to its freshly saved cache; `true` when
    /// the cache changed as a result.
    pub(super) fn reapply_waiting(&mut self, path: &str) -> bool {
        let mut any = false;
        for op in self.store.load_ops(&self.acc.id).unwrap_or_default() {
            let touches = match &op {
                Op::EmptyFolder { path: p } => p == path,
                o if o.is_folder_op() => false,
                o => o.keys().iter().any(|k| k.folder == path),
            };
            if touches && self.store.apply_op(&self.acc.id, &op).is_ok_and(|c| !c.is_empty()) {
                any = true;
            }
        }
        any
    }

    /// Replay offline changes on the server, oldest first.
    pub(super) fn replay(&mut self) {
        let mut destinations: Vec<String> = Vec::new();
        let mut relist = false;
        let trash = self.folders.iter().find(|f| f.role == FolderRole::Trash).map(|f| f.path.clone());
        // The queue is ops.json's: bounded, in case taking an entry out keeps failing.
        for _ in 0..100_000 {
            let Some(op) = self.store.load_ops(&self.acc.id).unwrap_or_default().into_iter().next() else { break };
            let r = match self.conn.as_mut() {
                Some(Conn::Imap(c)) => apply_remote_with(c, &op, trash.as_deref()),
                Some(Conn::Jmap(c)) => jmap::apply_remote(c, &self.jstate, &op),
                Some(Conn::Graph(c)) => crate::graph::apply_remote(c, &self.jstate, &op),
                // Messages the numbering lost are looked for first: left out, the change was
                // made here only.
                Some(Conn::Ews(c)) => {
                    crate::ews::find_lost(c, &mut self.jstate, &self.store, op.keys()).and_then(|()| crate::ews::apply_remote(c, &self.jstate, &op))
                }
                // POP: changes are made here only (the cache has them already).
                Some(Conn::Pop) => Ok(()),
                None => Err(NetError::Closed("not connected".into())),
            };
            match r {
                Ok(()) => {
                    match &op {
                        Op::Move { to, .. } => destinations.push(to.clone()),
                        Op::RenameFolder { to, .. } => {
                            relist = true;
                            destinations.push(to.clone());
                        }
                        Op::CreateFolder { .. } | Op::DeleteFolder { .. } => relist = true,
                        Op::EmptyFolder { path } => destinations.push(path.clone()),
                        Op::SetFlag { .. } | Op::SetKeyword { .. } | Op::Expunge { .. } => {}
                    }
                    if let Err(e) = self.store.remove_op(&self.acc.id, &op) {
                        self.notice(e.to_string());
                        break;
                    }
                    // The next edit may name this folder (or its new name): refresh the ids now
                    // rather than after the pass.
                    if op.is_folder_op() && relist {
                        relist = !self.relist();
                    }
                }
                Err(e) if e.is_fatal() => {
                    self.failed(e);
                    return;
                }
                // Busy: the change stays queued for the next pass.
                Err(e @ NetError::Later(_)) => {
                    log::warn!("{}: a change waits: {e}", self.acc.id);
                    return;
                }
                Err(e) => {
                    // The server refused: drop the change and resync so the cache matches the
                    // server again (forcing a full flag check).
                    if let Err(e) = self.store.remove_op(&self.acc.id, &op) {
                        self.notice(e.to_string());
                        break;
                    }
                    let mut folders: Vec<String> = op.keys().iter().map(|k| k.folder.clone()).collect();
                    folders.dedup();
                    for f in &folders {
                        if let Ok(mut st) = self.store.load_folder(&self.acc.id, f) {
                            st.highest_modseq = 0;
                            st.uid_next = 0;
                            let _ = self.store.save_folder(&self.acc.id, f, &st);
                        }
                    }
                    relist |= op.is_folder_op();
                    self.sink.emit(Event::OpFailed { account: self.id(), op, error: e.to_string() });
                    destinations.extend(folders);
                }
            }
        }
        if relist {
            self.relist();
        }
        destinations.sort();
        destinations.dedup();
        for d in destinations {
            if self.conn.is_some() {
                self.sync_one(&d);
            }
        }
    }
}
