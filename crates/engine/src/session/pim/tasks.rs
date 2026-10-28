//! Outlook's Tasks: each account's, kept with its calendar and address book (Exchange's Tasks
//! folder), or on this computer.

use quatrelle_model::pim::{DavSync, StoredTask, Task, TaskStatus};

use super::*;

/// A task with the account it's in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskRef {
    pub account: String,
    pub task: Task,
}

/// What a new task has, or what a change changes: each field left `None` stays as it is; a
/// date or the reminder taken off is `Some(None)`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskChange {
    pub subject: Option<String>,
    pub notes: Option<String>,
    pub start: Option<Option<i64>>,
    pub due: Option<Option<i64>>,
    pub status: Option<TaskStatus>,
    pub percent: Option<u8>,
    pub importance: Option<i8>,
    pub reminder: Option<Option<i64>>,
    pub categories: Option<Vec<String>>,
    pub private: Option<bool>,
}

/// Most tasks an account keeps, most characters in a subject and in the notes.
const MAX_TASKS: usize = 10_000;
const MAX_SUBJECT: usize = 255;
const MAX_NOTES: usize = 100_000;

impl Session {
    /// Every task matching a search (in its subject or notes), in the To-Do List's order; the
    /// completed ones too when `completed`.
    pub fn tasks(&self, query: &str, completed: bool) -> Vec<TaskRef> {
        let q = query.trim().to_lowercase();
        let today = quatrelle_model::dates::today(&self.clock);
        let mut list: Vec<TaskRef> = self
            .pim
            .iter()
            .flat_map(|(a, p)| p.tasks.iter().map(move |t| (a, t)))
            .filter(|(_, t)| completed || !t.task.is_complete())
            .filter(|(_, t)| q.is_empty() || t.task.subject.to_lowercase().contains(&q) || t.task.notes.to_lowercase().contains(&q))
            .map(|(a, t)| TaskRef { account: a.clone(), task: t.task.clone() })
            .collect();
        list.sort_by_cached_key(|r| {
            let t = &r.task;
            (quatrelle_model::pim::tasks::due_group(t, today), t.due.unwrap_or(i64::MAX), -t.importance, t.subject.to_lowercase(), t.uid.clone())
        });
        list
    }

    /// A task by its uid, with its account.
    pub fn find_task(&self, uid: &str) -> Option<TaskRef> {
        self.pim.iter().find_map(|(a, p)| p.tasks.iter().find(|t| t.task.uid == uid).map(|t| TaskRef { account: a.clone(), task: t.task.clone() }))
    }

    fn task_account(&self, uid: &str) -> Result<String, CmdError> {
        self.find_task(uid).map(|r| r.account).ok_or_else(|| CmdError::Params(format!("no task `{uid}`")))
    }

    /// Tasks go to the server only where it keeps them (Exchange's Tasks folder).
    fn tasks_sync(&self, account: &str) -> bool {
        self.account(account).is_some_and(|a| !a.is_local() && a.config.ews.is_some())
    }

    /// A new task; returns its uid.
    pub fn create_task(&mut self, account: Option<&str>, change: TaskChange) -> CmdResult {
        if change.subject.as_deref().is_none_or(|s| s.trim().is_empty()) {
            return Err(CmdError::Params("a task needs a subject".into()));
        }
        let account = self.pim_account(account)?;
        let today = quatrelle_model::dates::today(&self.clock);
        let mut task = Task { uid: self.new_uid(), modified: self.clock.now, ..Default::default() };
        apply(&mut task, change, today);
        let uid = task.uid.clone();
        let remote = self.tasks_sync(&account);
        let p = self.pim_mut(&account);
        if p.tasks.len() >= MAX_TASKS {
            return Err(CmdError::Params(format!("an account keeps at most {MAX_TASKS} tasks")));
        }
        p.tasks.push(StoredTask { task, dirty: remote, ..Default::default() });
        self.after_pim_change(&account, remote);
        Ok(serde_json::json!({"uid": uid, "account": account}))
    }

    /// Change a task.
    pub fn update_task(&mut self, uid: &str, change: TaskChange) -> CmdResult {
        if change.subject.as_deref().is_some_and(|s| s.trim().is_empty()) {
            return Err(CmdError::Params("a task needs a subject".into()));
        }
        let account = self.task_account(uid)?;
        let remote = self.tasks_sync(&account);
        let (now, today) = (self.clock.now, quatrelle_model::dates::today(&self.clock));
        let p = self.pim_mut(&account);
        let Some(t) = p.tasks.iter_mut().find(|t| t.task.uid == uid) else { return Err(CmdError::Params(format!("no task `{uid}`"))) };
        let before = t.task.clone();
        apply(&mut t.task, change, today);
        if t.task == before {
            return Ok(serde_json::json!({"uid": uid, "changed": false}));
        }
        t.task.modified = now;
        t.dirty |= remote;
        self.after_pim_change(&account, remote);
        Ok(serde_json::json!({"uid": uid}))
    }

    /// Mark a task complete, or not complete again (Outlook's Mark Complete, its tick).
    pub fn complete_task(&mut self, uid: &str, done: bool) -> CmdResult {
        let status = if done { TaskStatus::Completed } else { TaskStatus::NotStarted };
        let known = self.find_task(uid).ok_or_else(|| CmdError::Params(format!("no task `{uid}`")))?;
        if known.task.is_complete() == done {
            return Ok(serde_json::json!({"uid": uid, "changed": false}));
        }
        self.update_task(uid, TaskChange { status: Some(status), ..Default::default() })
    }

    pub fn delete_task(&mut self, uid: &str) -> CmdResult {
        let account = self.task_account(uid)?;
        let p = self.pim_mut(&account);
        let mut remote = false;
        for t in p.tasks.iter().filter(|t| t.task.uid == uid && !t.href.is_empty()) {
            remote = true;
            p.deleted.push(Tombstone { href: t.href.clone(), etag: t.etag.clone(), ..Default::default() });
        }
        p.tasks.retain(|t| t.task.uid != uid);
        self.after_pim_change(&account, remote);
        Ok(serde_json::json!({"deleted": uid}))
    }
}

/// A change made to a task, as Outlook makes it: a start after the due date moves the due date
/// with it (a due date before the start, the start), and its status and how far along it is
/// stay in step.
fn apply(t: &mut Task, c: TaskChange, today: i64) {
    if let Some(s) = c.subject {
        t.subject = s.trim().chars().take(MAX_SUBJECT).collect();
    }
    if let Some(n) = c.notes {
        t.notes = n.chars().take(MAX_NOTES).collect();
    }
    let start_set = c.start.is_some();
    if let Some(d) = c.start {
        t.start = d;
    }
    if let Some(d) = c.due {
        t.due = d;
    }
    if let (Some(s), Some(d)) = (t.start, t.due)
        && s > d
    {
        if start_set {
            t.due = Some(s);
        } else {
            t.start = Some(d);
        }
    }
    if let Some(i) = c.importance {
        t.importance = i.clamp(-1, 1);
    }
    if let Some(r) = c.reminder {
        t.reminder = r;
    }
    if let Some(list) = c.categories {
        t.categories = list;
    }
    if let Some(p) = c.private {
        t.private = p;
    }
    if let Some(s) = c.status {
        t.status = s;
        t.settle(false, today);
    }
    if let Some(p) = c.percent {
        t.percent = p;
        t.settle(true, today);
    }
}

/// Merge the tasks a sync found: those stored on the server get their item there and are no
/// longer to send; the server's list replaces the rest, those changed here (and those never
/// sent) staying.
pub(super) fn merge_tasks(p: &mut Pim, sync: &DavSync) {
    for x in &sync.pushed_tasks {
        for t in p.tasks.iter_mut().filter(|t| t.task.uid == x.task.uid) {
            t.href = x.href.clone();
            t.etag = x.etag.clone();
            t.dirty &= t.task != x.task;
        }
    }
    if let Some(server) = &sync.tasks {
        p.tasks.retain(|t| t.dirty || t.href.is_empty());
        for x in server {
            // Deleted here, the server not told yet (offline): it stays deleted.
            let deleting = p.deleted.iter().any(|d| d.href == x.href);
            if !deleting && !p.tasks.iter().any(|t| !t.href.is_empty() && t.href == x.href) {
                p.tasks.push(x.clone());
            }
        }
    }
}
