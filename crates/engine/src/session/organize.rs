// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Organising mail: categories, pinning, snoozing, inbox rules and the Focused/Other split.

use quatrelle_model::organize::{Category, Rule, RuleAction, Snooze, actions_for};
use quatrelle_model::{Flags, PINNED, is_keyword};

use super::*;
use quatrelle_model::organize::ServerKind;

pub mod junk;
mod outlook_rules;
pub mod quick_steps;
mod rules;
pub mod search_folders;

/// The folder snoozed mail waits in.
const SNOOZED: &str = "Snoozed";
/// The mail rules, and the snoozed mail to bring back.
const RULES_DOC: &str = "rules.json";
const SNOOZES_DOC: &str = "snoozes.json";

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

    /// The rules and the snoozed mail, as the session opens. One that can't be read is never
    /// written over with what's shown in its place (nothing): a notice says so.
    pub(super) fn load_rules_and_snoozes(&mut self) {
        let Some(store) = self.store.clone() else { return };
        match store.load_doc(RULES_DOC) {
            Ok(r) => self.rules = r,
            Err(e) => {
                log::warn!("the rules couldn't be read: {e}");
                self.docs_unread.insert(RULES_DOC);
                self.notice("Your rules couldn't be read: changes to them won't be saved", true);
            }
        }
        match store.load_doc(SNOOZES_DOC) {
            Ok(z) => self.snoozes = z,
            Err(e) => {
                log::warn!("snoozed mail couldn't be read: {e}");
                self.docs_unread.insert(SNOOZES_DOC);
                self.notice("Snoozed mail couldn't be read: changes to it won't be saved", true);
            }
        }
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
        // Its record wouldn't be kept: the mail would stay in Snoozed after a restart.
        if self.docs_unread.contains(SNOOZES_DOC) {
            return Err(CmdError::Failed("Snoozed mail couldn't be read, so no more can be snoozed".into()));
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

    /// Bring back snoozed mail that is due (and what else waits for its time: the list's date
    /// headings on a new day). Called often (each frame, each CLI run): cheap when nothing is due.
    pub fn tick(&mut self) {
        self.flush_due_outbox();
        self.read_when_due();
        self.web_calendars_due();
        self.refresh_rooms_due();
        self.scam_tick();
        // A new day moves the list's date headings (before the snoozes' early return).
        self.follow_the_day();
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
        // What couldn't be read isn't written over (the mail snoozed before wouldn't come back).
        if self.docs_unread.contains(SNOOZES_DOC) {
            return;
        }
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
            && let Err(e) = store.save_doc(SNOOZES_DOC, &kept)
        {
            self.notice_with("snoozed mail couldn't be saved: {error}", &[("error", &e.to_string())], true);
        }
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
