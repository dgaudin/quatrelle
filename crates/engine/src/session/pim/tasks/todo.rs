// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The To-Do List: the tasks, and every flagged message not done, of every account, as
//! Outlook's Tasks module opens on it.

use quatrelle_model::MessageKey;
use quatrelle_model::pim::tasks::{DueGroup, due_group_for};

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
    /// it).
    pub fn task_lists(&self) -> Vec<TaskList> {
        let mut out = vec![TaskList { id: "todo".into(), name: "To-Do List".into(), account: None, read_only: false, archive: false, mailbox: None }];
        for a in &self.accounts {
            let name = if a.config.name.trim().is_empty() { a.config.email.clone() } else { a.config.name.clone() };
            out.push(TaskList { id: a.config.id.clone(), name, account: Some(a.config.id.clone()), read_only: false, archive: false, mailbox: None });
            if a.config.ews.is_some() {
                out.push(TaskList {
                    id: format!("{}|archive", a.config.id),
                    name: "Online Archive".into(),
                    account: Some(a.config.id.clone()),
                    read_only: true,
                    archive: true,
                    mailbox: None,
                });
            }
            let mut shared: Vec<&str> = a.folders.iter().filter_map(|f| f.mailbox.as_deref()).collect();
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

impl Session {
    /// The To-Do List's row named by `uid` (a task's, or a message's key id).
    pub fn find_todo(&self, uid: &str, completed: bool) -> Option<TodoRef> {
        self.todo("", completed).into_iter().find(|r| r.uid() == uid)
    }

    /// The To-Do List: every task matching `query`, with every flagged message of the accounts
    /// kept on this computer that isn't done (and the done ones too when `completed`), grouped
    /// by due day and in that order. A row flagged here appears at once: the cache has it.
    pub fn todo(&self, query: &str, completed: bool) -> Vec<TodoRef> {
        let q = quatrelle_model::search::fold_word(query.trim());
        let has = |text: &str| quatrelle_model::search::fold_word(text).contains(q.as_ref());
        let today = quatrelle_model::dates::today(&self.clock);
        let mut list: Vec<TodoRef> = self.tasks(query, completed).into_iter().map(TodoRef::Task).collect();
        for ((account, _), st) in &self.cache {
            if self.account(account).is_none() {
                continue;
            }
            for m in &st.messages {
                // Flagged and not done, or (with the completed ones shown) a follow-up done.
                let done = m.flags.follow_up_complete();
                if m.flags.deleted || (!done && !m.flags.flagged) || (done && !completed) {
                    continue;
                }
                let subject = self.follow_up_title(m).unwrap_or(&m.subject).to_string();
                if !q.is_empty() && !has(&subject) && !has(&m.subject) {
                    continue;
                }
                list.push(TodoRef::Mail(FlaggedMail {
                    account: account.clone(),
                    key: m.key.clone(),
                    subject,
                    due: self.follow_up_due_any(m),
                    start: self.follow_up_start(m),
                    reminder: self.follow_up_reminder(m),
                    complete: done,
                }));
            }
        }
        list.sort_by_cached_key(|r| (r.group(today), r.due().unwrap_or(i64::MAX), -r.importance(), r.subject().to_lowercase(), r.uid()));
        list
    }
}
