// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The session: making, renaming, moving and deleting folders.

use super::*;

impl Session {
    /// The account's hierarchy delimiter (from any folder that has one).
    pub(crate) fn delimiter(&self, account: &str) -> Option<char> {
        self.account(account).and_then(|a| a.folders.iter().find_map(|f| f.delimiter))
    }

    /// Where top-level folders go: the root, or `INBOX` on servers that keep every folder under
    /// it (`INBOX.Sent`, `INBOX.Projects`).
    pub(crate) fn top_parent(&self, account: &str) -> Option<String> {
        let a = self.account(account)?;
        let d = self.delimiter(account)?;
        let others: Vec<&Folder> = a.folders.iter().filter(|f| !f.path.eq_ignore_ascii_case("INBOX")).collect();
        let inbox = a.folders.iter().find(|f| f.path.eq_ignore_ascii_case("INBOX"))?;
        (!others.is_empty() && others.iter().all(|f| f.path.starts_with(&format!("{}{d}", inbox.path)))).then(|| inbox.path.clone())
    }

    pub(crate) fn parent_of(path: &str, delimiter: Option<char>) -> Option<&str> {
        delimiter.and_then(|d| path.rsplit_once(d)).map(|(p, _)| p)
    }

    /// A user folder (not Inbox or another special folder) that exists, for changing.
    pub(crate) fn user_folder(&self, account: &str, path: &str) -> Result<Folder, CmdError> {
        if self.account(account).is_none() {
            return Err(CmdError::Params(format!("no account `{account}`")));
        }
        let f = self.folder(account, path).cloned().ok_or_else(|| CmdError::Params(format!("no folder `{path}`")))?;
        if f.is_fixed() {
            return Err(CmdError::Failed(format!("{} can't be renamed, moved or deleted", f.display_name())));
        }
        Ok(f)
    }

    /// Create a folder called `name` inside `parent` (`None`: at the top).
    pub fn create_folder(&mut self, account: &str, parent: Option<&str>, name: &str) -> CmdResult {
        if self.account(account).is_none() {
            return Err(CmdError::Params(format!("no account `{account}`")));
        }
        if let Some(p) = parent {
            self.refuse_archive_folder(account, p)?;
        }
        if let Some(p) = parent
            && self.folder(account, p).is_none()
        {
            return Err(CmdError::Params(format!("no folder `{p}`")));
        }
        let parent = parent.map(str::to_string).or_else(|| self.top_parent(account));
        let path = quatrelle_model::child_path(parent.as_deref(), self.delimiter(account), name).map_err(CmdError::Params)?;
        if self.folder(account, &path).is_some() {
            return Err(CmdError::Params(format!("there is already a folder called {}", name.trim())));
        }
        self.apply_op(account, Op::CreateFolder { path: path.clone() });
        Ok(serde_json::json!({ "created": path }))
    }

    /// Rename a folder (it stays where it is).
    pub fn rename_folder(&mut self, account: &str, path: &str, name: &str) -> CmdResult {
        self.refuse_archive_folder(account, path)?;
        let f = self.user_folder(account, path)?;
        let d = f.delimiter.or(self.delimiter(account));
        let to = quatrelle_model::child_path(Self::parent_of(path, d), d, name).map_err(CmdError::Params)?;
        if to == path {
            return Ok(serde_json::json!({ "renamed": to }));
        }
        if self.folder(account, &to).is_some() {
            return Err(CmdError::Params(format!("there is already a folder called {}", name.trim())));
        }
        self.apply_op(account, Op::RenameFolder { from: path.to_string(), to: to.clone() });
        Ok(serde_json::json!({ "renamed": to }))
    }

    /// Move a folder inside another (`None`: to the top).
    pub fn move_folder(&mut self, account: &str, path: &str, parent: Option<&str>) -> CmdResult {
        self.refuse_archive_folder(account, path)?;
        if let Some(p) = parent {
            self.refuse_archive_folder(account, p)?;
        }
        let f = self.user_folder(account, path)?;
        let d = f.delimiter.or(self.delimiter(account));
        let d = d.ok_or_else(|| CmdError::Failed("this server has no subfolders".into()))?;
        let parent = match parent {
            Some(p) => {
                if self.folder(account, p).is_none() {
                    return Err(CmdError::Params(format!("no folder `{p}`")));
                }
                if quatrelle_model::is_within(p, path, Some(d)) {
                    return Err(CmdError::Params("a folder can't be moved into itself".into()));
                }
                Some(p.to_string())
            }
            None => self.top_parent(account),
        };
        let leaf = path.rsplit(d).next().unwrap_or(path);
        let to = match &parent {
            Some(p) => format!("{p}{d}{leaf}"),
            None => leaf.to_string(),
        };
        if to == path {
            return Ok(serde_json::json!({ "moved": to }));
        }
        if self.folder(account, &to).is_some() {
            return Err(CmdError::Params(format!("there is already a folder called {} there", f.display_name())));
        }
        self.apply_op(account, Op::RenameFolder { from: path.to_string(), to: to.clone() });
        Ok(serde_json::json!({ "moved": to }))
    }

    /// Delete a folder: into Deleted Items, or for good when it is already there (or
    /// `permanent`, or there is no Deleted Items folder). Subfolders go with it.
    pub fn delete_folder(&mut self, account: &str, path: &str, permanent: bool) -> CmdResult {
        self.refuse_archive_folder(account, path)?;
        let f = self.user_folder(account, path)?;
        let d = f.delimiter.or(self.delimiter(account));
        let trash = self.account(account).and_then(|a| a.folder(FolderRole::Trash)).map(|t| t.path.clone());
        let in_trash = trash.as_deref().is_some_and(|t| quatrelle_model::is_within(path, t, d));
        if let (Some(trash), Some(d), false, false) = (&trash, d, in_trash, permanent) {
            // Like deleting mail: the folder waits in Deleted Items.
            let leaf = path.rsplit(d).next().unwrap_or(path);
            let mut to = format!("{trash}{d}{leaf}");
            let mut n = 2;
            while self.folder(account, &to).is_some() {
                to = format!("{trash}{d}{leaf} ({n})");
                n += 1;
            }
            self.apply_op(account, Op::RenameFolder { from: path.to_string(), to: to.clone() });
            return Ok(serde_json::json!({ "moved": to }));
        }
        // Deepest first: servers refuse to delete a folder that still has children.
        let mut doomed: Vec<String> = self
            .account(account)
            .map(|a| a.folders.iter().filter(|x| quatrelle_model::is_within(&x.path, path, d)).map(|x| x.path.clone()).collect())
            .unwrap_or_default();
        doomed.sort_by_key(|p| std::cmp::Reverse(p.len()));
        let n = doomed.len();
        for p in doomed {
            self.apply_op(account, Op::DeleteFolder { path: p });
        }
        Ok(serde_json::json!({ "deleted": n }))
    }

    /// Favorites as shown: the user's list, or the defaults.
    pub fn favorites(&self) -> Vec<FolderRef> {
        match &self.prefs.favorites {
            Some(list) => list.iter().filter(|r| self.folder(&r.account, &r.folder).is_some()).cloned().collect(),
            None if self.accounts.len() == 1 => self
                .accounts
                .first()
                .map(|a| {
                    [FolderRole::Inbox, FolderRole::Sent, FolderRole::Drafts]
                        .iter()
                        .filter_map(|r| a.folder(*r))
                        .map(|f| FolderRef { account: a.config.id.clone(), folder: f.path.clone() })
                        .collect()
                })
                .unwrap_or_default(),
            None => Vec::new(),
        }
    }

    /// Add a folder to Favorites or take it off (`on: None` toggles).
    pub fn set_favorite(&mut self, account: &str, folder: &str, on: Option<bool>) -> CmdResult {
        if self.folder(account, folder).is_none() {
            return Err(CmdError::Params(format!("no folder `{folder}` in account `{account}`")));
        }
        let mut list = self.favorites();
        let r = FolderRef { account: account.to_string(), folder: folder.to_string() };
        let present = list.contains(&r);
        let on = on.unwrap_or(!present);
        if on && !present {
            list.push(r);
        } else if !on {
            list.retain(|x| *x != r);
        }
        self.prefs.favorites = Some(list);
        self.follow_public_favorites();
        Ok(serde_json::json!({ "on": on }))
    }

    /// Whether an Exchange public folder is kept in sync: it's one of the user's Favorites, and
    /// the account keeps them (Outlook's "Download Public Folder Favorites", off unless chosen).
    pub fn follows_public(&self, account: &str, folder: &str) -> bool {
        self.account(account).is_some_and(|a| a.config.public_favorites_kept)
            && self.prefs.favorites.as_ref().is_some_and(|l| l.iter().any(|r| r.account == account && r.folder == folder))
    }

    /// Whether a folder is read on the server when it's opened rather than kept on this
    /// computer, as Outlook's Cached Exchange Mode does: a public folder not followed, a
    /// shared mailbox's folder when the account doesn't keep them, and a folder of the online
    /// archive, always (no setting, no Favorite keeps it).
    pub fn read_on_server(&self, account: &str, folder: &str) -> bool {
        let Some(a) = self.account(account) else { return false };
        if self.in_online_archive(account, folder) {
            return true;
        }
        if quatrelle_model::is_public_folder(folder) {
            return a.config.ews.is_some() && !self.follows_public(account, folder);
        }
        a.config.shared_online && a.folders.iter().any(|f| f.path == folder && f.mailbox.is_some())
    }

    /// Whether a folder is one of the account's online archive (Exchange's archive mailbox).
    pub fn in_online_archive(&self, account: &str, folder: &str) -> bool {
        self.account(account).is_some_and(|a| a.folders.iter().any(|f| f.path == folder && f.archive))
    }

    /// Tell each Exchange account's sync which public folders to keep in sync: the Favorites
    /// among them when the account keeps them (Outlook's "Download Public Folder Favorites"),
    /// none otherwise; the others are read when opened. Written when it changes; a public folder
    /// just followed is synced at once. Favorites never set (the defaults, or before the saved
    /// ones are read) say nothing: the list kept stays.
    pub fn follow_public_favorites(&mut self) {
        let Some(store) = self.store.clone() else { return };
        let favorites = self.prefs.favorites.clone();
        let ids: Vec<(String, bool)> =
            self.accounts.iter().filter(|a| a.config.ews.is_some()).map(|a| (a.config.id.clone(), a.config.public_favorites_kept)).collect();
        for (id, kept) in ids {
            let list: Vec<String> = match (&favorites, kept) {
                (_, false) => Vec::new(),
                (Some(f), true) => {
                    f.iter().filter(|r| r.account == id && quatrelle_model::is_public_folder(&r.folder)).map(|r| r.folder.clone()).collect()
                }
                (None, true) => continue,
            };
            if self.followed_public.get(&id) == Some(&list) {
                continue;
            }
            let before: Vec<String> = store.load_account_doc(&id, quatrelle_model::FOLLOWED_PUBLIC_DOC).unwrap_or_default();
            if before != list {
                if let Err(e) = store.save_account_doc(&id, quatrelle_model::FOLLOWED_PUBLIC_DOC, &list) {
                    self.notice_with("the public folders to keep in sync couldn't be saved: {error}", &[("error", &e.to_string())], true);
                    continue;
                }
                for folder in list.iter().filter(|f| !before.contains(f)) {
                    self.jobs.push(Job::SyncFolder { account: id.clone(), folder: folder.clone() });
                }
            }
            self.followed_public.insert(id, list);
        }
    }

    /// Count a local account's folders again (`only` these, or all of them). One whose list
    /// can't be read keeps its counts (not 0).
    pub(crate) fn recount_local(&mut self, account: &str, only: Option<&[String]>) {
        // Its counts are written after the marks made before (which write them too).
        self.finish_local_marks();
        let paths: Vec<String> = match only {
            Some(list) => list.to_vec(),
            None => self.account(account).map(|a| a.folders.iter().map(|f| f.path.clone()).collect()).unwrap_or_default(),
        };
        let mut counts = Vec::new();
        for p in paths {
            if let Err(e) = self.load_cached(account, &p) {
                log::warn!("{account}: a folder isn't counted: {e}");
                continue;
            }
            let st = self.folder_state(account, &p);
            counts.push((p, st.unread(), u32::try_from(st.messages.len()).unwrap_or(u32::MAX)));
        }
        // Compared with the disk too: a flag changed here was counted in memory already
        // (`apply_op_memory`), so the counts kept on disk were never written again after a mark.
        let on_disk = self.store.as_ref().and_then(|s| s.load_folders(account).ok()).unwrap_or_default();
        let mut changed =
            counts.iter().any(|(p, unread, total)| on_disk.iter().find(|f| &f.path == p).is_some_and(|f| f.unread != *unread || f.total != *total));
        if let Some(a) = self.account_mut(account) {
            for (p, unread, total) in counts {
                if let Some(f) = a.folders.iter_mut().find(|f| f.path == p)
                    && (f.unread != unread || f.total != total)
                {
                    f.unread = unread;
                    f.total = total;
                    changed = true;
                }
            }
        }
        // Local accounts have no server to report counts: keep them on disk.
        if changed
            && let (Some(store), Some(a)) = (&self.store, self.account(account))
            && let Err(e) = store.save_folders(account, &a.folders)
        {
            log::warn!("{account}: folder counts not saved: {e}");
        }
    }

    /// The message to show after `gone` leave the list: the next one down, else the one above.
    pub(crate) fn successor(&self, gone: &[MessageKey]) -> Option<MessageKey> {
        let gone: HashSet<&MessageKey> = gone.iter().collect();
        let last = self.list.iter().rposition(|m| gone.contains(&m.key))?;
        self.list
            .iter()
            .skip(last + 1)
            .find(|m| !gone.contains(&m.key))
            .or_else(|| self.list.iter().take(last).rev().find(|m| !gone.contains(&m.key)))
            .map(|m| m.key.clone())
    }

    /// The message to show after `gone` leave the list when `read` is the one being read: the
    /// first after it that stays, else the nearest before it. Anchored at the message read, not
    /// after the last one going: an earlier version of an invitation further down doesn't take
    /// the list down there.
    pub(crate) fn successor_of(&self, read: &MessageKey, gone: &[MessageKey]) -> Option<MessageKey> {
        let gone: HashSet<&MessageKey> = gone.iter().collect();
        let at = self.index_of(read)?;
        self.list
            .iter()
            .skip(at.saturating_add(1))
            .find(|m| !gone.contains(&m.key))
            .or_else(|| self.list.iter().take(at).rev().find(|m| !gone.contains(&m.key)))
            .map(|m| m.key.clone())
    }

    /// Move the selection to a folder of the same role in each message's account (Archive,
    /// Deleted Items, Junk), or to an explicit folder.
    pub fn move_selection(&mut self, target: MoveTarget) -> CmdResult {
        let sel = self.selected();
        if sel.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        self.refuse_archive_selection()?;
        let next = self.successor(&self.selection.clone());
        let moved = self.move_messages(&sel, &target)?;
        self.after_removal(next);
        Ok(serde_json::json!({ "moved": moved }))
    }

    /// Move these messages to a folder of the same role beside each one (Archive, Deleted Items,
    /// Junk), or to an explicit folder, each one undoable; those already there stay. How many
    /// moved. What's shown next is the caller's ([`Session::after_removal`]).
    pub(crate) fn move_messages(&mut self, msgs: &[Arc<Summary>], target: &MoveTarget) -> Result<usize, CmdError> {
        // The online archive is read-only: mail in it stays, and nothing is filed into it.
        self.refuse_archive_mail(msgs.iter().map(|m| &m.key))?;
        if let MoveTarget::Folder(f) = target {
            for m in msgs {
                self.refuse_archive_folder(&m.key.account, f)?;
            }
        }
        // By account, and by mailbox within it: mail in another mailbox opened here goes to
        // that mailbox's own Deleted Items, Archive… (Exchange moves mail within a mailbox).
        let mut groups: BTreeMap<(String, Option<String>), Vec<&Summary>> = BTreeMap::new();
        for m in msgs {
            let owner = self.account(&m.key.account).and_then(|a| a.folder_mailbox(&m.key.folder));
            groups.entry((m.key.account.clone(), owner)).or_default().push(m);
        }
        let mut moved = 0usize;
        for ((a, _), group) in groups {
            let dest = match target {
                MoveTarget::Folder(f) => Some(f.clone()),
                MoveTarget::Role(r) => {
                    let from = group.first().map(|m| m.key.folder.clone()).unwrap_or_default();
                    self.account(&a).and_then(|acc| acc.folder_beside(&from, *r)).map(|f| f.path.clone())
                }
            };
            let Some(dest) = dest else {
                let what = match target {
                    MoveTarget::Role(r) => r.label().unwrap_or("that folder"),
                    MoveTarget::Folder(_) => "that folder",
                };
                return Err(CmdError::Failed(format!("this account has no {what} folder")));
            };
            if self.folder(&a, &dest).is_none() {
                return Err(CmdError::Params(format!("no folder `{dest}`")));
            }
            let group: Vec<&Summary> = group.into_iter().filter(|m| m.key.folder != dest).collect();
            moved = moved.saturating_add(group.len());
            if !group.is_empty() {
                // Undo finds them again by Message-ID (those without one can't come back).
                let back: Vec<(String, String)> =
                    group.iter().filter(|m| !m.message_id.is_empty()).map(|m| (m.message_id.clone(), m.key.folder.clone())).collect();
                let keys = group.iter().map(|m| m.key.clone()).collect();
                // One that didn't happen (a local store error) has nothing to undo, and the
                // selection stays on it.
                self.apply_op_checked(&a, Op::Move { keys, to: dest.clone() })
                    .map_err(|e| CmdError::Failed(format!("Your mail couldn't be moved, so it stays where it was: {e}")))?;
                if !back.is_empty() {
                    self.remember(Undo::Move { account: a.clone(), to: dest, back });
                }
            }
        }
        Ok(moved)
    }

    /// Delete permanently.
    pub fn expunge_selection(&mut self) -> CmdResult {
        let sel = self.selected();
        if sel.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        self.refuse_archive_selection()?;
        let next = self.successor(&self.selection.clone());
        let mut by_account: HashMap<String, Vec<MessageKey>> = HashMap::new();
        for m in &sel {
            by_account.entry(m.key.account.clone()).or_default().push(m.key.clone());
        }
        let n = sel.len();
        for (a, keys) in by_account {
            self.apply_op(&a, Op::Expunge { keys, tidy: false });
        }
        self.after_removal(next);
        Ok(serde_json::json!({ "deleted": n }))
    }

    pub(crate) fn after_removal(&mut self, next: Option<MessageKey>) {
        self.reading = None;
        self.selection.clear();
        self.focus = None;
        if let Some(k) = next
            && self.index_of(&k).is_some()
        {
            // Show the next message, but don't mark it read just because something was deleted.
            self.show_message(&k, false);
        }
    }

    /// Set or clear a flag on the selection (`on: None` toggles, following the first message).
    /// Flagged again or cleared, a follow-up marked complete no longer is (`follow_up.rs`).
    pub fn flag_selection(&mut self, flag: Flag, on: Option<bool>) -> CmdResult {
        let sel = self.selected();
        let first = sel.first().ok_or_else(|| CmdError::Disabled("no message selected".into()))?;
        let current = flag.get(&first.flags);
        let on = on.unwrap_or(!current);
        if flag == Flag::Flagged {
            let was = self.follow_up_days_of(&sel);
            let undo = self.set_flagged(&sel, on);
            self.remember_follow_up(undo, was);
            return Ok(serde_json::json!({ "on": on, "count": sel.len() }));
        }
        let mut by_account: HashMap<String, Vec<MessageKey>> = HashMap::new();
        // Undo puts back only the ones that changed.
        let mut changed: HashMap<String, Vec<MessageKey>> = HashMap::new();
        for m in &sel {
            by_account.entry(m.key.account.clone()).or_default().push(m.key.clone());
            if flag.get(&m.flags) != on {
                changed.entry(m.key.account.clone()).or_default().push(m.key.clone());
            }
        }
        for (a, keys) in changed {
            self.remember(Undo::Ops { account: a, ops: vec![Op::SetFlag { keys, flag, on: !on }] });
        }
        for (a, keys) in by_account {
            self.apply_op(&a, Op::SetFlag { keys, flag, on });
        }
        Ok(serde_json::json!({ "on": on, "count": sel.len() }))
    }

    pub(crate) fn remember(&mut self, u: Undo) {
        self.undo.push(u);
        if self.undo.len() > UNDO_DEPTH {
            self.undo.remove(0);
        }
    }

    /// Put the last change back (Undo).
    pub fn undo_last(&mut self) -> CmdResult {
        let u = self.undo.pop().ok_or_else(|| CmdError::Disabled("nothing to undo".into()))?;
        match u {
            Undo::Ops { account, ops } => {
                for op in ops {
                    self.apply_op(&account, op);
                }
                Ok(serde_json::json!({ "undone": "changes" }))
            }
            Undo::Archive { account, back } => self.undo_archive(account, back),
            Undo::PutBack { account, to, ids } => self.undo_put_back(account, to, ids),
            Undo::FollowUp { ops, days } => {
                self.undo_follow_up(ops, days);
                Ok(serde_json::json!({ "undone": "changes" }))
            }
            Undo::Move { account, to, back } => {
                // Where each one is now: the destination's messages with that Message-ID.
                let here = self.folder_state(&account, &to).messages.clone();
                let mut by_folder: std::collections::BTreeMap<String, Vec<MessageKey>> = std::collections::BTreeMap::new();
                let mut used: BTreeSet<MessageKey> = BTreeSet::new();
                for (id, from) in &back {
                    if let Some(m) = here.iter().find(|m| &m.message_id == id && !used.contains(&m.key)) {
                        used.insert(m.key.clone());
                        by_folder.entry(from.clone()).or_default().push(m.key.clone());
                    }
                }
                if by_folder.is_empty() {
                    // Not there yet (the server is still moving them): try again in a moment.
                    self.undo.push(Undo::Move { account, to, back });
                    return Err(CmdError::Failed("still being moved; try Undo again in a moment".into()));
                }
                let n = used.len();
                for (from, keys) in by_folder {
                    self.apply_op(&account, Op::Move { keys, to: from });
                }
                Ok(serde_json::json!({ "undone": "move", "count": n }))
            }
            Undo::EventMove { uid, occurrence, start, end, send } => {
                self.move_event_back(&uid, occurrence, start, end, send)?;
                Ok(serde_json::json!({ "undone": "event", "uid": uid }))
            }
        }
    }
}
