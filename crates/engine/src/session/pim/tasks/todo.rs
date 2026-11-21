// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The To-Do List: the tasks, and every flagged message not done, of every account, as
//! Outlook's Tasks module opens on it.

use quatrelle_model::MessageKey;
use quatrelle_model::pim::tasks::{DueGroup, due_group_for};

use super::*;

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
