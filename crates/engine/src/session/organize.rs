//! Organising mail: categories, pinning, snoozing, inbox rules and the Focused/Other split.

use quatrelle_model::organize::{Category, Rule, RuleAction, Snooze, actions_for};
use quatrelle_model::{Flags, PINNED, is_keyword};

use super::*;
use quatrelle_model::organize::ServerKind;

pub mod junk;
mod outlook_rules;
pub mod quick_steps;
pub mod search_folders;

/// The folder snoozed mail waits in.
const SNOOZED: &str = "Snoozed";

/// What a server would be told for these rules (Quatrelle's ids left out): comparing it says
/// whether a change is worth a round trip to the server.
fn server_shape(rules: &[Rule], kind: &impl Fn(&str) -> Option<ServerKind>) -> Vec<String> {
    rules
        .iter()
        .filter(|r| r.server && r.account.as_deref().and_then(kind).is_some_and(|k| r.server_ready_for(k)))
        .map(|r| format!("{:?}|{}|{}|{:?}|{:?}|{}", r.account, r.name, r.enabled, r.conditions, r.actions, r.stop))
        .collect()
}

/// Where "run the rules now" looks: the list on screen, the Inbox of each account the rules
/// apply to, or one folder (`account`: the rule's, or the current one, when not given).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RulesScope {
    List,
    Inbox,
    Folder { account: Option<String>, folder: String },
}

impl Session {
    // ----- categories and pins -----

    /// Add or remove a keyword on the selection (`on: None` toggles, following the first message);
    /// a category's carries its name (Exchange keeps categories by name).
    fn keyword_selection(&mut self, keyword: &str, category: Option<Category>, on: Option<bool>) -> CmdResult {
        if !is_keyword(keyword) {
            return Err(CmdError::Params(format!("`{keyword}` can't be used as a keyword")));
        }
        let carries = |f: &Flags| category.as_ref().map_or_else(|| f.has(keyword), |c| c.marks(f));
        let name = category.as_ref().map(|c| c.name.clone());
        let sel = self.selected();
        let first = sel.first().ok_or_else(|| CmdError::Disabled("no message selected".into()))?;
        let on = on.unwrap_or(!carries(&first.flags));
        let mut by_account: HashMap<String, Vec<MessageKey>> = HashMap::new();
        let mut changed: HashMap<String, Vec<MessageKey>> = HashMap::new();
        for m in &sel {
            by_account.entry(m.key.account.clone()).or_default().push(m.key.clone());
            if carries(&m.flags) != on {
                changed.entry(m.key.account.clone()).or_default().push(m.key.clone());
            }
        }
        for (a, keys) in changed {
            let undo = Op::SetKeyword { keys, keyword: keyword.to_string(), on: !on, name: name.clone() };
            self.remember(Undo::Ops { account: a, ops: vec![undo] });
        }
        for (a, keys) in by_account {
            self.apply_op(&a, Op::SetKeyword { keys, keyword: keyword.to_string(), on, name: name.clone() });
        }
        Ok(serde_json::json!({ "on": on, "count": sel.len() }))
    }

    /// The category called `name` (any case).
    pub fn category(&self, name: &str) -> Option<&Category> {
        self.prefs.categories.iter().find(|c| c.name.eq_ignore_ascii_case(name.trim()))
    }

    /// The categories a message carries, in the order the user keeps them.
    pub fn categories_of(&self, m: &Summary) -> Vec<&Category> {
        self.prefs.categories.iter().filter(|c| c.marks(&m.flags)).collect()
    }

    pub fn categorize_selection(&mut self, name: &str, on: Option<bool>) -> CmdResult {
        let c = self.category(name).cloned().ok_or_else(|| CmdError::Params(format!("no category called {name}")))?;
        self.keyword_selection(&c.keyword.clone(), Some(c), on)
    }

    pub fn pin_selection(&mut self, on: Option<bool>) -> CmdResult {
        self.keyword_selection(PINNED, None, on)
    }

    pub fn add_category(&mut self, name: &str, color: Option<u8>) -> CmdResult {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 60 {
            return Err(CmdError::Params("a category needs a name (up to 60 characters)".into()));
        }
        if self.category(name).is_some() {
            return Err(CmdError::Params(format!("there is already a category called {name}")));
        }
        let color = color.unwrap_or_else(|| u8::try_from(self.prefs.categories.len() % 8).unwrap_or(0));
        let c = Category::new(name, color);
        let kw = c.keyword.clone();
        self.prefs.categories.push(c);
        Ok(serde_json::json!({ "created": name, "keyword": kw }))
    }

    /// Forget a category. Messages keep the keyword (another client may still use it).
    pub fn remove_category(&mut self, name: &str) -> CmdResult {
        let before = self.prefs.categories.len();
        self.prefs.categories.retain(|c| !c.name.eq_ignore_ascii_case(name.trim()));
        if self.prefs.categories.len() == before {
            return Err(CmdError::Params(format!("no category called {name}")));
        }
        Ok(serde_json::json!({ "removed": name }))
    }

    // ----- snooze -----

    /// The Snoozed folder of an account, created when missing.
    fn snoozed_folder(&mut self, account: &str) -> Result<String, CmdError> {
        let found = self.account(account).and_then(|a| {
            a.folders.iter().find(|f| f.selectable && f.role == FolderRole::Other && f.display_name().eq_ignore_ascii_case(SNOOZED) && f.depth() <= 1)
        });
        if let Some(f) = found {
            return Ok(f.path.clone());
        }
        let r = self.create_folder(account, None, SNOOZED)?;
        r.get("created").and_then(Value::as_str).map(str::to_string).ok_or_else(|| CmdError::Failed("the Snoozed folder couldn't be made".into()))
    }

    /// Move the selection out of sight until `until` (Unix seconds); it comes back unread to
    /// where it was.
    pub fn snooze_selection(&mut self, until: i64) -> CmdResult {
        if until <= self.clock.now {
            return Err(CmdError::Params("pick a time in the future".into()));
        }
        let sel = self.selected();
        if sel.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        if sel.iter().any(|m| m.message_id.is_empty()) {
            return Err(CmdError::Failed("a message without a Message-ID can't be snoozed".into()));
        }
        let next = self.successor(&self.selection.clone());
        let mut accounts: Vec<String> = sel.iter().map(|m| m.key.account.clone()).collect();
        accounts.sort();
        accounts.dedup();
        for a in accounts {
            let folder = self.snoozed_folder(&a)?;
            let mine: Vec<&Summary> = sel.iter().filter(|m| m.key.account == a && m.key.folder != folder).map(Arc::as_ref).collect();
            for m in &mine {
                self.snoozes.retain(|s| !(s.account == a && s.message_id == m.message_id));
                self.snoozes.push(Snooze {
                    account: a.clone(),
                    message_id: m.message_id.clone(),
                    folder: folder.clone(),
                    back_to: m.key.folder.clone(),
                    until,
                    subject: m.subject.clone(),
                });
            }
            let keys: Vec<MessageKey> = mine.iter().map(|m| m.key.clone()).collect();
            if !keys.is_empty() {
                self.apply_op(&a, Op::Move { keys, to: folder });
            }
        }
        self.save_snoozes();
        self.after_removal(next);
        Ok(serde_json::json!({ "snoozed": sel.len(), "until": until }))
    }

    /// Bring snoozed messages back now.
    pub fn unsnooze_selection(&mut self) -> CmdResult {
        let sel = self.selected();
        let mut n = 0;
        for m in &sel {
            if let Some(i) = self.snoozes.iter().position(|s| s.account == m.key.account && s.message_id == m.message_id) {
                let s = self.snoozes.remove(i);
                self.wake(&s, m.key.clone());
                n += 1;
            }
        }
        self.save_snoozes();
        Ok(serde_json::json!({ "unsnoozed": n }))
    }

    fn wake(&mut self, s: &Snooze, key: MessageKey) {
        let back = if self.folder(&s.account, &s.back_to).is_some() {
            s.back_to.clone()
        } else {
            self.account(&s.account).and_then(|a| a.folder(FolderRole::Inbox)).map(|f| f.path.clone()).unwrap_or_else(|| "INBOX".into())
        };
        self.apply_op(&s.account, Op::SetFlag { keys: vec![key.clone()], flag: Flag::Seen, on: false });
        self.apply_op(&s.account, Op::Move { keys: vec![key], to: back });
    }

    /// Bring back snoozed mail that is due. Called often (each frame, each CLI run): cheap when
    /// nothing is due.
    pub fn tick(&mut self) {
        self.flush_due_outbox();
        self.read_when_due();
        self.web_calendars_due();
        self.refresh_rooms_due();
        let now = self.clock.now;
        if !self.snoozes.iter().any(|s| s.until <= now) {
            return;
        }
        let due: Vec<Snooze> = self.snoozes.iter().filter(|s| s.until <= now).cloned().collect();
        let mut changed = false;
        for s in due {
            if self.account(&s.account).is_none() {
                self.snoozes.retain(|x| x != &s);
                changed = true;
                continue;
            }
            let found = self.folder_state(&s.account, &s.folder).messages.iter().find(|m| m.message_id == s.message_id).map(|m| m.key.clone());
            match found {
                Some(key) => {
                    self.snoozes.retain(|x| x != &s);
                    self.wake(&s, key);
                    if let Some(m) = self.list.iter().find(|m| m.message_id == s.message_id).map(|m| Summary::clone(m)) {
                        self.new_mail.push(m);
                    } else {
                        self.notice_with("Back from snooze: {subject}", &[("subject", &s.subject)], false);
                    }
                    changed = true;
                }
                // Not synced into the Snoozed folder yet; give up after a week.
                None if now > s.until + 7 * 86_400 => {
                    self.snoozes.retain(|x| x != &s);
                    changed = true;
                }
                None => {}
            }
        }
        if changed {
            self.save_snoozes();
        }
    }

    pub(super) fn save_snoozes(&mut self) {
        // An online-only account's snoozed mail is kept without its subject (issue #4).
        let kept: Vec<_> = self
            .snoozes
            .iter()
            .map(|s| match self.account(&s.account) {
                Some(a) if a.config.keeps_online() => quatrelle_model::organize::Snooze { subject: String::new(), ..s.clone() },
                _ => s.clone(),
            })
            .collect();
        if let Some(store) = &self.store
            && let Err(e) = store.save_doc("snoozes.json", &kept)
        {
            self.notice_with("snoozed mail couldn't be saved: {error}", &[("error", &e.to_string())], true);
        }
    }

    // ----- rules -----

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
    fn rule_keywords(&self) -> Vec<(String, String)> {
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
    pub(super) fn save_rules(&mut self) {
        if let Some(store) = &self.store
            && let Err(e) = store.save_doc("rules.json", &self.rules)
        {
            self.notice_with("the rules couldn't be saved: {error}", &[("error", &e.to_string())], true);
        }
    }

    /// Read the rules the account's server keeps (`GetServerRules`): the ids Quatrelle's own have
    /// there, and the ones made elsewhere. Accounts whose server keeps none are left alone.
    pub(super) fn refresh_server_rules(&mut self, account: &str) {
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
    pub(super) fn server_rules_refused(&mut self, account: &str, why: String) {
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
    pub(super) fn resend_server_rules(&mut self, account: &str) {
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
    fn our_server_ids(&self, account: &str) -> Vec<String> {
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

    // ----- ignoring conversations -----

    /// The Message-IDs a message names: its own, and those it replies to.
    fn conversation_ids(m: &Summary) -> Vec<String> {
        std::iter::once(&m.message_id)
            .chain(std::iter::once(&m.in_reply_to))
            .chain(&m.references)
            .map(|id| quatrelle_model::thread::norm_id(id).to_ascii_lowercase())
            .filter(|id| !id.is_empty())
            .collect()
    }

    /// Whether a message belongs to a conversation being ignored.
    pub fn is_ignored(&self, m: &Summary) -> bool {
        Self::conversation_ids(m).iter().any(|id| self.prefs.ignored.contains(id))
    }

    /// Ignore the selected conversations, as Outlook's Delete › Ignore: every message of them
    /// goes to Deleted Items, and so does whatever arrives in them later. On conversations already
    /// ignored, stop ignoring them.
    pub fn ignore_selection(&mut self) -> CmdResult {
        let sel = self.selected();
        if sel.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        if sel.iter().all(|m| self.is_ignored(m)) {
            for m in &sel {
                for id in Self::conversation_ids(m) {
                    self.prefs.ignored.remove(&id);
                }
            }
            return Ok(serde_json::json!({ "ignoring": false }));
        }
        // The conversations' messages, in the folders the selection is in.
        let mut members: Vec<Summary> = Vec::new();
        for m in &sel {
            let folder = self.folder_state(&m.key.account, &m.key.folder).messages.clone();
            for t in quatrelle_model::thread::threads(&folder) {
                let msgs: Vec<&Summary> = t.messages.iter().filter_map(|&i| folder.get(i).map(Arc::as_ref)).collect();
                if msgs.iter().any(|x| x.key == m.key) {
                    members.extend(msgs.into_iter().cloned());
                }
            }
            if !members.iter().any(|x| x.key == m.key) {
                members.push(Summary::clone(m));
            }
        }
        for m in &members {
            self.prefs.ignored.extend(Self::conversation_ids(m));
        }
        // Kept bounded: the oldest ids go first (a BTreeSet has no age; dropping any is safe).
        while self.prefs.ignored.len() > 5_000 {
            let Some(first) = self.prefs.ignored.iter().next().cloned() else { break };
            self.prefs.ignored.remove(&first);
        }
        let keys: BTreeSet<MessageKey> = members.iter().map(|m| m.key.clone()).collect();
        self.selection = keys.into_iter().collect();
        let n = self.selection.len();
        self.move_selection(MoveTarget::Role(FolderRole::Trash))?;
        Ok(serde_json::json!({ "ignoring": true, "moved": n }))
    }

    /// New mail in an ignored conversation goes to Deleted Items (and its own id joins the
    /// conversation's). Returns the keys moved away.
    pub(super) fn ignore_arrived(&mut self, msgs: &[Summary]) -> Vec<MessageKey> {
        if self.prefs.ignored.is_empty() {
            return Vec::new();
        }
        let mut gone = Vec::new();
        for m in msgs {
            if !self.is_ignored(m) {
                continue;
            }
            self.prefs.ignored.extend(Self::conversation_ids(m));
            let a = m.key.account.clone();
            if let Some(trash) = self.account(&a).and_then(|x| x.folder(FolderRole::Trash)).map(|f| f.path.clone())
                && trash != m.key.folder
            {
                self.apply_op(&a, Op::Move { keys: vec![m.key.clone()], to: trash });
                gone.push(m.key.clone());
            }
        }
        gone
    }

    /// Run the rules on messages: their actions become ordinary changes. Returns the keys the
    /// rules marked read or sent to Deleted Items or Junk (no need to announce those).
    pub(super) fn run_rules<M: std::borrow::Borrow<Summary>>(&mut self, msgs: &[M]) -> Vec<MessageKey> {
        let mut quiet = Vec::new();
        if self.rules.is_empty() {
            return quiet;
        }
        let rules = self.rules.clone();
        for m in msgs.iter().map(M::borrow) {
            let actions = actions_for(&rules, m);
            if actions.is_empty() {
                continue;
            }
            let a = m.key.account.clone();
            let keys = vec![m.key.clone()];
            let mut dest: Option<String> = None;
            for act in actions {
                match act {
                    RuleAction::MarkRead => {
                        self.apply_op(&a, Op::SetFlag { keys: keys.clone(), flag: Flag::Seen, on: true });
                        quiet.push(m.key.clone());
                    }
                    RuleAction::Flag => self.apply_op(&a, Op::SetFlag { keys: keys.clone(), flag: Flag::Flagged, on: true }),
                    RuleAction::Pin => self.apply_op(&a, Op::SetKeyword { keys: keys.clone(), keyword: PINNED.into(), on: true, name: None }),
                    RuleAction::Categorize(c) => {
                        if let Some(c) = self.category(&c).cloned() {
                            self.apply_op(&a, Op::SetKeyword { keys: keys.clone(), keyword: c.keyword, on: true, name: Some(c.name) });
                        }
                    }
                    RuleAction::MoveTo(f) => dest = self.folder(&a, &f).map(|f| f.path.clone()).or(dest),
                    RuleAction::MoveToRole(r) => dest = self.account(&a).and_then(|x| x.folder(r)).map(|f| f.path.clone()).or(dest),
                    RuleAction::Delete => dest = self.account(&a).and_then(|x| x.folder(FolderRole::Trash)).map(|f| f.path.clone()).or(dest),
                }
            }
            // Moves last, so flags and categories are set on the message before it leaves. Mail
            // filed in a folder is still new mail (announced, as Outlook does); mail a rule sends
            // to Deleted Items or Junk isn't.
            if let Some(to) = dest.filter(|to| *to != m.key.folder) {
                let role = self.folder(&a, &to).map(|f| f.role);
                if matches!(role, Some(FolderRole::Trash | FolderRole::Junk)) {
                    quiet.push(m.key.clone());
                }
                self.apply_op(&a, Op::Move { keys, to });
            }
        }
        quiet
    }

    /// Run the rules (or one) now, on the list on screen, on the mail already in the Inbox of
    /// the accounts they apply to, or on one folder's, as Outlook's "run this rule now".
    pub fn run_rules_now(&mut self, only: Option<usize>, scope: RulesScope) -> CmdResult {
        let saved = self.rules.clone();
        if let Some(i) = only {
            let r = saved.get(i).cloned().ok_or_else(|| CmdError::Params(format!("no rule {i}")))?;
            self.rules = vec![Rule { enabled: true, ..r }];
        }
        let only_account = only.and_then(|i| saved.get(i)).and_then(|r| r.account.clone());
        let folders: Vec<(String, String)> = match &scope {
            RulesScope::List => Vec::new(),
            RulesScope::Inbox => self
                .accounts
                .iter()
                .filter(|a| only_account.as_ref().is_none_or(|o| *o == a.config.id))
                .filter_map(|a| a.folder(FolderRole::Inbox).map(|f| (a.config.id.clone(), f.path.clone())))
                .collect(),
            // A rule made from a message in another folder runs on that folder's mail: the
            // rule's account's, else the one on screen.
            RulesScope::Folder { account, folder } => {
                let account = account
                    .clone()
                    .or_else(|| only_account.clone())
                    .or_else(|| self.current_account().map(|a| a.config.id.clone()))
                    .ok_or_else(|| CmdError::Params("`account` is required".into()))?;
                if self.folder(&account, folder).is_none() {
                    self.rules = saved;
                    return Err(CmdError::Params(format!("no folder `{folder}` in that account")));
                }
                vec![(account, folder.clone())]
            }
        };
        let msgs: Vec<Arc<Summary>> = if matches!(scope, RulesScope::List) {
            self.list.clone()
        } else {
            folders
                .iter()
                .flat_map(|(a, f)| self.folder_state(a, f).messages.iter().filter(|m| !m.flags.deleted).cloned().collect::<Vec<_>>())
                .collect()
        };
        let n = msgs.iter().filter(|m| !actions_for(&self.rules, m).is_empty()).count();
        self.run_rules(&msgs);
        self.rules = saved;
        if n == 1 {
            self.notice("Rules ran on the mail there now: 1 message matched", false);
        } else {
            self.notice_with("Rules ran on the mail there now: {n} messages matched", &[("n", &n.to_string())], false);
        }
        Ok(serde_json::json!({ "matched": n }))
    }

    // ----- Focused / Other -----

    /// Whether a message belongs in Focused: the user's choice for its sender, else anything not
    /// sent in bulk, and bulk mail from people the user writes to.
    pub fn is_focused(&self, m: &Summary) -> bool {
        let sender = m.from.email.trim().to_ascii_lowercase();
        if let Some(&f) = self.prefs.focus_overrides.get(&sender) {
            return f;
        }
        !m.bulk || self.known_contact(&sender).is_some_and(|c| c.sent > 0)
    }

    /// Whether the list shows the Focused/Other split (an Inbox, with the option on).
    pub fn focus_split(&self) -> bool {
        self.prefs.focused_inbox && self.search.is_none() && self.view_role() == Some(FolderRole::Inbox)
    }

    /// Always put the selection's senders in Focused (or Other).
    pub fn set_focus_for_selection(&mut self, focused: bool) -> CmdResult {
        let sel = self.selected();
        if sel.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        for m in &sel {
            let sender = m.from.email.trim().to_ascii_lowercase();
            if !sender.is_empty() && self.prefs.focus_overrides.len() < 10_000 {
                self.prefs.focus_overrides.insert(sender, focused);
            }
        }
        self.reading = None;
        self.selection.clear();
        self.refresh_list();
        Ok(serde_json::json!({ "focused": focused, "senders": sel.len() }))
    }
}

impl Session {
    // ----- clean up -----

    /// Clean Up, as Outlook's: messages of a conversation that a later reply quotes in full go to
    /// Deleted Items ([`quatrelle_model::cleanup`]). In these folders, every conversation, or
    /// (`only`) those holding these messages. Bodies not downloaded are left alone.
    pub fn clean_up(&mut self, folders: &[(String, String)], only: Option<&[MessageKey]>) -> CmdResult {
        let categories: Vec<String> = self.prefs.categories.iter().map(|c| c.keyword.clone()).collect();
        let mut gone: Vec<MessageKey> = Vec::new();
        let mut looked = 0usize;
        for (a, f) in folders {
            // Deleted Items and Junk aren't cleaned up into Deleted Items.
            if self.folder(a, f).is_none_or(|x| matches!(x.role, FolderRole::Trash | FolderRole::Junk)) {
                continue;
            }
            let msgs: Vec<Arc<Summary>> = self.folder_state(a, f).messages.iter().filter(|m| !m.flags.deleted).cloned().collect();
            for t in quatrelle_model::thread::threads(&msgs) {
                let members: Vec<&Summary> = t.messages.iter().filter_map(|&i| msgs.get(i).map(Arc::as_ref)).collect();
                if members.len() < 2 || only.is_some_and(|keys| !members.iter().any(|m| keys.contains(&m.key))) {
                    continue;
                }
                looked += 1;
                let texts: Vec<Option<String>> = members
                    .iter()
                    .map(|m| self.store.as_ref().and_then(|s| s.read_body(&m.key)).and_then(|raw| mime::parse(&raw)).map(|p| p.text))
                    .collect();
                gone.extend(
                    quatrelle_model::cleanup::redundant(&members, &texts, &categories)
                        .into_iter()
                        .filter_map(|i| members.get(i).map(|m| m.key.clone())),
                );
            }
        }
        if gone.is_empty() {
            if looked == 0 {
                self.notice("No conversations to clean up", false);
            } else {
                self.notice("Nothing to clean up: no message is quoted in full by a later reply", false);
            }
            return Ok(serde_json::json!({ "moved": 0, "conversations": looked }));
        }
        // Through the selection, so it can be undone like any move.
        let n = gone.len();
        self.selection = gone;
        self.move_selection(MoveTarget::Role(FolderRole::Trash))?;
        if n == 1 {
            self.notice("Clean Up moved 1 message to Deleted Items", false);
        } else {
            self.notice_with("Clean Up moved {n} messages to Deleted Items", &[("n", &n.to_string())], false);
        }
        Ok(serde_json::json!({ "moved": n, "conversations": looked }))
    }
}
