// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The To-Do List: the tasks, and every flagged message not done, of every account, as
//! Outlook's Tasks module opens on it.

use quatrelle_model::MessageKey;
use quatrelle_model::pim::tasks::{DueGroup, TaskArrange, TaskView, due_group_for};

use super::*;

/// A list in the Tasks module's My Tasks pane: the To-Do List (everything), an account's tasks,
/// the online archive's (read on the server, never changed here), or a shared mailbox's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskList {
    /// `todo`; an account id; `<account>|archive`; `<account>|mailbox:<address>`.
    pub id: String,
    pub name: String,
    /// The account tasks typed in it go to (none for the To-Do List, which takes the first).
    pub account: Option<String>,
    /// Read on the server and never changed here (the online archive).
    pub read_only: bool,
    pub archive: bool,
    /// A shared mailbox's, opened in the account.
    pub mailbox: Option<String>,
}

impl Session {
    /// The My Tasks pane's lists, in Outlook's order: the To-Do List, then each account's tasks
    /// (with its online archive's read-only list, and those of the shared mailboxes opened in
    /// it). Computed again only when the mail stamp moved (an account or folder changed), not
    /// at every call.
    pub fn task_lists(&self) -> Vec<TaskList> {
        let stamp = self.mail_stamp;
        {
            let g = self.task_lists_memo.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((s, lists)) = g.as_ref()
                && *s == stamp
            {
                return lists.clone();
            }
        }
        let lists = self.build_task_lists();
        *self.task_lists_memo.lock().unwrap_or_else(|e| e.into_inner()) = Some((stamp, lists.clone()));
        lists
    }

    /// [`Session::task_lists`], computed from the accounts and folders as they are now.
    fn build_task_lists(&self) -> Vec<TaskList> {
        let mut out = vec![TaskList { id: "todo".into(), name: "To-Do List".into(), account: None, read_only: false, archive: false, mailbox: None }];
        for a in &self.accounts {
            // By its address, as the folder pane heads the account: several accounts carry the
            // same person's name.
            let name = if a.config.email.trim().is_empty() { a.config.name.clone() } else { a.config.email.clone() };
            out.push(TaskList { id: a.config.id.clone(), name, account: Some(a.config.id.clone()), read_only: false, archive: false, mailbox: None });
            // The online archive's list only when the account has one (an archive folder among
            // its folders), not for every Exchange account.
            if a.folders.iter().any(|f| f.archive) {
                out.push(TaskList {
                    id: format!("{}|archive", a.config.id),
                    name: "Online Archive".into(),
                    account: Some(a.config.id.clone()),
                    read_only: true,
                    archive: true,
                    mailbox: None,
                });
            }
            // The online archive's folders name a mailbox too: it has its own list above.
            let mut shared: Vec<&str> = a.folders.iter().filter(|f| !f.archive).filter_map(|f| f.mailbox.as_deref()).collect();
            shared.sort_unstable();
            shared.dedup();
            for mailbox in shared {
                out.push(TaskList {
                    id: format!("{}|mailbox:{mailbox}", a.config.id),
                    name: mailbox.to_string(),
                    account: Some(a.config.id.clone()),
                    read_only: false,
                    archive: false,
                    mailbox: Some(mailbox.to_string()),
                });
            }
        }
        out
    }

    /// The list named by `id`, when it's one of [`Self::task_lists`].
    pub fn task_list(&self, id: &str) -> Option<TaskList> {
        self.task_lists().into_iter().find(|l| l.id == id)
    }

    /// The view and arrangement remembered for a list (Simple List, by due day, when none is).
    pub fn task_view(&self, list: &str) -> (TaskView, TaskArrange) {
        (self.prefs.task_views.get(list).copied().unwrap_or_default(), self.prefs.task_arranges.get(list).copied().unwrap_or_default())
    }

    /// A list as a view shows it: its items filtered (Active, Completed, the next seven days,
    /// Overdue) and arranged (by due or start date, importance, categories, folder or type).
    /// `completed` also shows the done ones where the view keeps them.
    pub fn todo_view(&self, list: &str, query: &str, completed: bool, view: TaskView, arrange: TaskArrange) -> Vec<TodoRef> {
        let today = quatrelle_model::dates::today(&self.clock);
        let mut items = self.todo_for(list, query, completed || view == TaskView::Completed);
        match view {
            TaskView::Active => items.retain(|r| !r.is_complete()),
            TaskView::Completed => items.retain(|r| r.is_complete()),
            TaskView::NextSevenDays => items.retain(|r| r.due().is_some_and(|d| (today..today.saturating_add(7)).contains(&d)) && !r.is_complete()),
            TaskView::Overdue => items.retain(|r| !r.is_complete() && r.due().is_some_and(|d| d < today)),
            TaskView::SimpleList | TaskView::Detailed | TaskView::Prioritized => {}
        }
        let due = |r: &TodoRef| r.due().unwrap_or(i64::MAX);
        // Prioritized is by importance, whatever the arrangement (as Outlook's view says).
        let arrange = if view == TaskView::Prioritized { TaskArrange::Importance } else { arrange };
        match arrange {
            TaskArrange::DueDate => items.sort_by_cached_key(|r| (r.group(today), due(r), -r.importance(), r.subject().to_lowercase(), r.uid())),
            TaskArrange::StartDate => items.sort_by_cached_key(|r| {
                let start = match r {
                    TodoRef::Task(t) => t.task.start,
                    TodoRef::Mail(m) => m.start,
                };
                (start.unwrap_or(i64::MAX), due(r), r.subject().to_lowercase(), r.uid())
            }),
            TaskArrange::Importance => items.sort_by_cached_key(|r| (-r.importance(), due(r), r.subject().to_lowercase(), r.uid())),
            TaskArrange::Categories => items.sort_by_cached_key(|r| {
                let category = match r {
                    TodoRef::Task(t) => t.task.categories.first().map(|c| c.to_lowercase()).unwrap_or_default(),
                    TodoRef::Mail(m) => m.categories.first().map(|c| c.to_lowercase()).unwrap_or_default(),
                };
                (category, due(r), r.subject().to_lowercase(), r.uid())
            }),
            TaskArrange::Folder => items.sort_by_cached_key(|r| {
                let folder = match r {
                    TodoRef::Task(t) => t.account.to_lowercase(),
                    TodoRef::Mail(m) => format!("{}/{}", m.account, m.key.folder).to_lowercase(),
                };
                (folder, due(r), r.subject().to_lowercase(), r.uid())
            }),
            TaskArrange::Type => items.sort_by_cached_key(|r| (r.mail().is_some(), due(r), r.subject().to_lowercase(), r.uid())),
        }
        items
    }

    /// What a list holds: the To-Do List's tasks and flagged mail, or an account's tasks.
    /// The archive's and a shared mailbox's tasks aren't kept here (the first is read on the
    /// server and changes refused, the other's sync isn't here yet): those lists are empty.
    pub fn todo_for(&self, list: &str, query: &str, completed: bool) -> Vec<TodoRef> {
        let Some(list) = self.task_list(list) else { return Vec::new() };
        if list.id == "todo" {
            return self.todo(query, completed);
        }
        if list.archive || list.mailbox.is_some() {
            return Vec::new();
        }
        let Some(account) = &list.account else { return Vec::new() };
        self.tasks(query, completed).into_iter().filter(|t| &t.account == account).map(TodoRef::Task).collect()
    }
}

/// A flagged message in the To-Do List: the message, and how its follow-up stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlaggedMail {
    pub account: String,
    pub key: MessageKey,
    /// Its follow-up's title when one was set (`Send the report`), else its subject.
    pub subject: String,
    pub due: Option<i64>,
    pub start: Option<i64>,
    pub reminder: Option<i64>,
    /// Its categories, as a task's are (Arrange By › Categories and the Detailed view's column).
    pub categories: Vec<String>,
    /// Its follow-up marked complete (then it shows only with the completed ones).
    pub complete: bool,
}

/// One line of the To-Do List: a task, or a flagged message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TodoRef {
    Task(TaskRef),
    Mail(FlaggedMail),
}

impl TodoRef {
    /// What names it: a task's uid, or a message's key id (as `mail.flag` takes it).
    pub fn uid(&self) -> String {
        match self {
            TodoRef::Task(t) => t.task.uid.clone(),
            TodoRef::Mail(m) => m.key.to_id(),
        }
    }

    /// The task, when it is one.
    pub fn task(&self) -> Option<&TaskRef> {
        match self {
            TodoRef::Task(t) => Some(t),
            TodoRef::Mail(_) => None,
        }
    }

    /// The flagged message, when it is one.
    pub fn mail(&self) -> Option<&FlaggedMail> {
        match self {
            TodoRef::Task(_) => None,
            TodoRef::Mail(m) => Some(m),
        }
    }

    pub fn subject(&self) -> &str {
        match self {
            TodoRef::Task(t) => &t.task.subject,
            TodoRef::Mail(m) => &m.subject,
        }
    }

    pub fn due(&self) -> Option<i64> {
        match self {
            TodoRef::Task(t) => t.task.due,
            TodoRef::Mail(m) => m.due,
        }
    }

    pub fn is_complete(&self) -> bool {
        match self {
            TodoRef::Task(t) => t.task.is_complete(),
            TodoRef::Mail(m) => m.complete,
        }
    }

    /// The To-Do List's group: by its follow-up's due day, or Completed.
    pub fn group(&self, today: i64) -> DueGroup {
        due_group_for(self.due(), self.is_complete(), today)
    }

    /// Its importance (a task's; a flag is normal).
    pub fn importance(&self) -> i8 {
        match self {
            TodoRef::Task(t) => t.task.importance,
            TodoRef::Mail(_) => 0,
        }
    }
}

/// Folders kept on disk read per call when the To-Do List or the reminders look through mail:
/// round-robin, so every folder is come to in time and the work one call does stays bounded.
#[cfg(target_arch = "wasm32")]
pub(crate) const TODO_DISK_SCAN: usize = 2;

/// The flagged or reminded messages kept of the folders read from disk, at most: a folder's
/// whole list is read off the interface thread, but only what the To-Do List shows is kept.
pub(crate) const MAX_DISK_FLAGGED: usize = 5_000;

/// Whether a message read from the disk cache is one the To-Do List or the reminders show:
/// flagged, completed, or with a reminder.
fn flagged_or_reminded(m: &Summary) -> bool {
    m.flags.flagged || m.flags.follow_up_complete() || m.flags.follow_up.reminder > 0
}

/// Keep `msgs` as the folder `k`'s disk-read list, least recently read first, bounded by
/// [`MAX_DISK_FLAGGED`].
fn insert_disk_read(g: &mut crate::session::DiskScan, k: (String, String), msgs: Vec<Summary>) {
    let mut total: usize = g.map.values().map(|v| v.len()).sum();
    total = total.saturating_add(msgs.len());
    while total > MAX_DISK_FLAGGED
        && let Some(old) = g.order.first().cloned()
    {
        if let Some(v) = g.map.remove(&old) {
            total = total.saturating_sub(v.len());
        }
        g.order.remove(0);
    }
    g.map.insert(k.clone(), msgs);
    g.order.push(k);
    g.stamp = g.stamp.wrapping_add(1);
}

impl Session {
    /// Whether a folder is in the account's own mailbox, whose followed mail the To-Do List and
    /// the reminders read, as Outlook's read the user's own store only: not a shared mailbox
    /// opened here, a public folder or the online archive.
    pub(crate) fn in_own_mailbox(&self, account: &str, folder: &str) -> bool {
        if quatrelle_model::job::is_public_folder(folder) {
            return false;
        }
        let Some(a) = self.account(account) else { return true };
        a.folder_mailbox(folder).is_none() && !a.folders.iter().any(|f| f.path == folder && f.archive)
    }

    /// Calls `keep(account, folder, message)` for each message of each folder of each known
    /// account: those in the cache first, then those of the folders read from the cache on
    /// disk into [`Session::disk_lists`] — a few a call, round-robin, so a message followed in
    /// a folder not opened this session counts too, in time, and what is read stays (the list
    /// doesn't rotate under the window). Each message once (its key).
    pub(crate) fn scan_mail(&self, mut keep: impl FnMut(&str, &str, &Summary)) {
        let mut seen: std::collections::HashSet<MessageKey> = std::collections::HashSet::new();
        for ((account, folder), st) in &self.cache {
            if self.account(account).is_none() {
                continue;
            }
            for m in &st.messages {
                if seen.insert(m.key.clone()) {
                    keep(account, folder, m);
                }
            }
        }
        let Some(store) = &self.store else { return };
        {
            let mut g = self.disk_lists.lock().unwrap_or_else(|e| e.into_inner());
            g.folders = self.accounts.iter().flat_map(|a| a.folders.iter().map(|f| (a.config.id.clone(), f.path.clone()))).collect();
            for k in self.cache.keys() {
                g.done.insert(k.clone());
            }
            // The disk read itself runs off the interface thread (native); the web has no
            // threads and reads a few folders per call below instead.
            #[cfg(not(target_arch = "wasm32"))]
            if !g.started {
                g.started = true;
                drop(g);
                Self::spawn_disk_scan(store.clone(), std::sync::Arc::downgrade(&self.disk_lists));
            }
        }
        // What is read already: the background reader fills this, bounded, off this thread.
        {
            let g = self.disk_lists.lock().unwrap_or_else(|e| e.into_inner());
            for ((account, folder), msgs) in &g.map {
                if self.account(account).is_none() {
                    continue;
                }
                for m in msgs {
                    if seen.insert(m.key.clone()) {
                        keep(account, folder, m);
                    }
                }
            }
        }
        // The web (no threads): read a few folders per call here, round-robin, flagged only.
        #[cfg(target_arch = "wasm32")]
        {
            let mut g = self.disk_lists.lock().unwrap_or_else(|e| e.into_inner());
            let candidates: Vec<(String, String)> = g.folders.iter().filter(|k| !g.done.contains(*k)).cloned().collect();
            if !candidates.is_empty() {
                let start = self.todo_scan.fetch_add(TODO_DISK_SCAN, std::sync::atomic::Ordering::Relaxed) % candidates.len();
                for i in 0..TODO_DISK_SCAN.min(candidates.len()) {
                    let k = candidates[(start + i) % candidates.len()].clone();
                    if let Ok(mut st) = store.load_folder(&k.0, &k.1) {
                        st.messages.retain(flagged_or_reminded);
                        insert_disk_read(&mut g, k.clone(), st.messages);
                    }
                    g.done.insert(k);
                    g.stamp = g.stamp.wrapping_add(1);
                }
            }
        }
    }

    /// Read the folders of the cache on disk for the To-Do List and the reminders, off the
    /// interface thread: one folder at a time, only its flagged or reminded messages kept,
    /// bounded, until the session is gone (the weak reference fails then).
    #[cfg(not(target_arch = "wasm32"))]
    fn spawn_disk_scan(store: quatrelle_store::Store, weak: std::sync::Weak<std::sync::Mutex<crate::session::DiskScan>>) {
        let _ = std::thread::Builder::new().name("todo-disk-scan".into()).spawn(move || {
            while let Some(g) = weak.upgrade() {
                let next = {
                    let g = g.lock().unwrap_or_else(|e| e.into_inner());
                    g.folders.iter().find(|k| !g.done.contains(*k)).cloned()
                };
                let Some(k) = next else {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    continue;
                };
                // A folder whose list can't be read is left out, not fatal, and not retried
                // until a change touches it again.
                let loaded = store.load_folder(&k.0, &k.1).ok().map(|mut st| {
                    st.messages.retain(flagged_or_reminded);
                    st.messages
                });
                let mut g = g.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(msgs) = loaded {
                    insert_disk_read(&mut g, k.clone(), msgs);
                }
                g.done.insert(k);
                g.stamp = g.stamp.wrapping_add(1);
                drop(g);
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        });
    }

    /// The To-Do List's row named by `uid` (a task's, or a message's key id).
    pub fn find_todo(&self, uid: &str, completed: bool) -> Option<TodoRef> {
        self.todo("", completed).into_iter().find(|r| r.uid() == uid)
    }

    /// The To-Do List: every task matching `query`, with every flagged message of the accounts
    /// kept on this computer that isn't done (and the done ones too when `completed`), grouped
    /// by due day and in that order. A row flagged here appears at once: the cache has it; one
    /// flagged in a folder not opened this session appears too, read from the cache on disk.
    pub fn todo(&self, query: &str, completed: bool) -> Vec<TodoRef> {
        let q = quatrelle_model::search::fold_word(query.trim());
        let has = |text: &str| quatrelle_model::search::fold_word(text).contains(q.as_ref());
        let today = quatrelle_model::dates::today(&self.clock);
        let mut list: Vec<TodoRef> = self.tasks(query, completed).into_iter().map(TodoRef::Task).collect();
        // The folders' roles, to leave Junk and Trash out as the reminders do: a row that
        // never rings isn't listed either.
        let roles: std::collections::HashMap<(&str, &str), quatrelle_model::FolderRole> =
            self.accounts.iter().flat_map(|a| a.folders.iter().map(move |f| ((a.config.id.as_str(), f.path.as_str()), f.role))).collect();
        // A message under several of Gmail's labels (Inbox, Starred, All Mail…) is one row, as
        // its reminder rings once (both by its Message-ID): the Inbox's, else an ordinary
        // folder's, the archive's last.
        let rank = |account: &str, folder: &str| match roles.get(&(account, folder)) {
            Some(quatrelle_model::FolderRole::Inbox) => 0u8,
            Some(quatrelle_model::FolderRole::Archive) => 2,
            _ => 1,
        };
        let mut by_id: std::collections::HashMap<(String, String), (usize, u8)> = std::collections::HashMap::new();
        self.scan_mail(|account, folder, m| {
            // Flagged and not done, or (with the completed ones shown) a follow-up done.
            let done = m.flags.follow_up_complete();
            if m.flags.deleted || (!done && !m.flags.flagged) || (done && !completed) {
                return;
            }
            if matches!(roles.get(&(account, folder)), Some(quatrelle_model::FolderRole::Junk | quatrelle_model::FolderRole::Trash)) {
                return;
            }
            if !self.in_own_mailbox(account, folder) {
                return;
            }
            let subject = self.follow_up_title(m).unwrap_or(&m.subject).to_string();
            if !q.is_empty() && !has(&subject) && !has(&m.subject) {
                return;
            }
            let row = TodoRef::Mail(FlaggedMail {
                account: account.to_string(),
                key: m.key.clone(),
                subject,
                due: self.follow_up_due_any(m),
                start: self.follow_up_start(m),
                reminder: self.follow_up_reminder(m),
                categories: self.categories_of(m).into_iter().map(|c| c.name.clone()).collect(),
                complete: done,
            });
            let here = rank(account, folder);
            if m.message_id.is_empty() {
                list.push(row);
                return;
            }
            match by_id.entry((account.to_string(), m.message_id.clone())) {
                std::collections::hash_map::Entry::Occupied(mut seen) => {
                    let (at, was) = *seen.get();
                    if here < was
                        && let Some(slot) = list.get_mut(at)
                    {
                        *slot = row;
                        seen.insert((at, here));
                    }
                }
                std::collections::hash_map::Entry::Vacant(v) => {
                    v.insert((list.len(), here));
                    list.push(row);
                }
            }
        });
        list.sort_by_cached_key(|r| (r.group(today), r.due().unwrap_or(i64::MAX), -r.importance(), r.subject().to_lowercase(), r.uid()));
        list
    }
}
