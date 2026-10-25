//! Outlook's Tasks: things to do, with the day they start and the day they're due, where they
//! stand and how far along, a priority and a reminder — kept with an account's calendar
//! (Exchange's Tasks folder, [MS-OXOTASK]) or on this computer.
//!
//! Days are days since 1970 (a task's dates have no time of day, as Outlook's), the reminder a
//! moment (Unix seconds).

use serde::{Deserialize, Serialize};

/// Where a task stands, as Outlook's Status has it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskStatus {
    #[default]
    NotStarted,
    InProgress,
    Completed,
    WaitingOnOthers,
    Deferred,
}

impl TaskStatus {
    pub const ALL: [TaskStatus; 5] =
        [TaskStatus::NotStarted, TaskStatus::InProgress, TaskStatus::Completed, TaskStatus::WaitingOnOthers, TaskStatus::Deferred];

    /// As EWS (and the commands) name it.
    pub fn name(self) -> &'static str {
        match self {
            TaskStatus::NotStarted => "NotStarted",
            TaskStatus::InProgress => "InProgress",
            TaskStatus::Completed => "Completed",
            TaskStatus::WaitingOnOthers => "WaitingOnOthers",
            TaskStatus::Deferred => "Deferred",
        }
    }

    /// From its name, in any case.
    pub fn named(name: &str) -> Option<TaskStatus> {
        TaskStatus::ALL.into_iter().find(|s| s.name().eq_ignore_ascii_case(name.trim()))
    }

    /// As the interface says it (a key of the interface's catalogs).
    pub fn label(self) -> &'static str {
        match self {
            TaskStatus::NotStarted => "Not Started",
            TaskStatus::InProgress => "In Progress",
            TaskStatus::Completed => "Completed",
            TaskStatus::WaitingOnOthers => "Waiting on someone else",
            TaskStatus::Deferred => "Deferred",
        }
    }
}

/// A task.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Task {
    pub uid: String,
    pub subject: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub notes: String,
    /// The day it starts, and the day it's due.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due: Option<i64>,
    pub status: TaskStatus,
    /// How far along it is, 0 to 100 (100 once it's complete).
    pub percent: u8,
    /// The day it was completed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed: Option<i64>,
    /// Its priority as the mail's importance: 1 high, -1 low, 0 normal.
    pub importance: i8,
    /// When its reminder comes; `None`: no reminder.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reminder: Option<i64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub private: bool,
    /// When it was last changed (Unix seconds).
    pub modified: i64,
}

impl Task {
    pub fn is_complete(&self) -> bool {
        self.status == TaskStatus::Completed
    }

    /// Due before `today` and not done: Outlook shows it in red.
    pub fn is_overdue(&self, today: i64) -> bool {
        !self.is_complete() && self.due.is_some_and(|d| d < today)
    }

    /// Mark it complete on `today` (its reminder done with), or not complete again, as
    /// Outlook's Mark Complete and its tick in the list.
    pub fn set_complete(&mut self, done: bool, today: i64) {
        if done {
            self.status = TaskStatus::Completed;
            self.percent = 100;
            self.completed = Some(today);
            self.reminder = None;
        } else if self.is_complete() {
            self.status = TaskStatus::NotStarted;
            self.percent = 0;
            self.completed = None;
        }
    }

    /// Its status and how far along it is kept in step as Outlook keeps them: 100% is
    /// complete and complete is 100%; begun is in progress, and not started is 0%. `percent_set`:
    /// the percentage is what was just changed (else the status was).
    pub fn settle(&mut self, percent_set: bool, today: i64) {
        self.percent = self.percent.min(100);
        if percent_set {
            match self.percent {
                100 => self.set_complete(true, today),
                0 if self.status == TaskStatus::Completed => self.set_complete(false, today),
                0 if self.status == TaskStatus::InProgress => self.status = TaskStatus::NotStarted,
                0 => {}
                _ if matches!(self.status, TaskStatus::NotStarted | TaskStatus::Completed) => {
                    self.status = TaskStatus::InProgress;
                    self.completed = None;
                }
                _ => {}
            }
        } else {
            match self.status {
                TaskStatus::Completed => self.set_complete(true, today),
                TaskStatus::NotStarted => {
                    self.percent = 0;
                    self.completed = None;
                }
                _ => {
                    if self.percent == 100 {
                        self.percent = 0;
                    }
                    self.completed = None;
                }
            }
        }
    }
}

/// A task with where it lives: `href` is its item on the server (Exchange's Tasks folder),
/// empty for one kept on this computer only.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredTask {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub href: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub etag: String,
    pub task: Task,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dirty: bool,
}

/// Where a task falls in Outlook's To-Do List, grouped by its due date (weeks from Monday).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DueGroup {
    Overdue,
    Today,
    Tomorrow,
    ThisWeek,
    NextWeek,
    Later,
    NoDate,
    Completed,
}

impl DueGroup {
    /// The group's heading (a key of the interface's catalogs).
    pub fn label(self) -> &'static str {
        match self {
            DueGroup::Overdue => "Overdue",
            DueGroup::Today => "Today",
            DueGroup::Tomorrow => "Tomorrow",
            DueGroup::ThisWeek => "This Week",
            DueGroup::NextWeek => "Next Week",
            DueGroup::Later => "Later",
            DueGroup::NoDate => "No Date",
            DueGroup::Completed => "Completed",
        }
    }
}

/// The group a task falls in on `today`.
pub fn due_group(t: &Task, today: i64) -> DueGroup {
    if t.is_complete() {
        return DueGroup::Completed;
    }
    let Some(due) = t.due else { return DueGroup::NoDate };
    // Monday of this week (1970-01-01 was a Thursday).
    let monday = today - (today + 3).rem_euclid(7);
    match due - today {
        d if d < 0 => DueGroup::Overdue,
        0 => DueGroup::Today,
        1 => DueGroup::Tomorrow,
        _ if due < monday + 7 => DueGroup::ThisWeek,
        _ if due < monday + 14 => DueGroup::NextWeek,
        _ => DueGroup::Later,
    }
}

/// Tasks in the To-Do List's order: by group, then by due day, the higher priority first, then
/// by subject.
pub fn sort_for_list(tasks: &mut [Task], today: i64) {
    tasks.sort_by_cached_key(|t| (due_group(t, today), t.due.unwrap_or(i64::MAX), -t.importance, t.subject.to_lowercase(), t.uid.clone()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dates::days_from_civil;

    #[test]
    fn complete_and_percent_move_together() {
        let today = days_from_civil(2026, 10, 27);
        let mut t = Task { reminder: Some(5), ..Default::default() };
        t.set_complete(true, today);
        assert_eq!((t.status, t.percent, t.completed, t.reminder), (TaskStatus::Completed, 100, Some(today), None));
        t.set_complete(false, today);
        assert_eq!((t.status, t.percent, t.completed), (TaskStatus::NotStarted, 0, None));
        // Begun: in progress; all of it: complete; back to nothing: not started.
        t.percent = 40;
        t.settle(true, today);
        assert_eq!(t.status, TaskStatus::InProgress);
        t.percent = 100;
        t.settle(true, today);
        assert!(t.is_complete() && t.completed == Some(today));
        t.percent = 0;
        t.settle(true, today);
        assert_eq!((t.status, t.percent), (TaskStatus::NotStarted, 0));
        t.percent = 20;
        t.settle(true, today);
        t.percent = 0;
        t.settle(true, today);
        assert_eq!(t.status, TaskStatus::NotStarted, "in progress back to nothing");
        // The status set: the percentage follows.
        t.percent = 60;
        t.status = TaskStatus::Completed;
        t.settle(false, today);
        assert_eq!(t.percent, 100);
        t.status = TaskStatus::WaitingOnOthers;
        t.settle(false, today);
        assert_eq!((t.percent, t.completed), (0, None), "no longer complete");
        t.percent = 30;
        t.status = TaskStatus::NotStarted;
        t.settle(false, today);
        assert_eq!(t.percent, 0);
        t.percent = 250;
        t.settle(true, today);
        assert!(t.is_complete(), "capped at 100");
    }

    #[test]
    fn tasks_fall_in_outlooks_groups() {
        // Tuesday 27 October 2026.
        let today = days_from_civil(2026, 10, 27);
        let due = |d: Option<i64>| Task { due: d, ..Default::default() };
        let group = |d: Option<i64>| due_group(&due(d), today);
        assert_eq!(group(Some(today - 1)), DueGroup::Overdue);
        assert_eq!(group(Some(today)), DueGroup::Today);
        assert_eq!(group(Some(today + 1)), DueGroup::Tomorrow);
        assert_eq!(group(Some(days_from_civil(2026, 11, 1))), DueGroup::ThisWeek, "Sunday ends the week");
        assert_eq!(group(Some(days_from_civil(2026, 11, 2))), DueGroup::NextWeek);
        assert_eq!(group(Some(days_from_civil(2026, 11, 9))), DueGroup::Later);
        assert_eq!(group(None), DueGroup::NoDate);
        let mut done = due(Some(today - 3));
        done.set_complete(true, today);
        assert_eq!(due_group(&done, today), DueGroup::Completed);
        assert!(due(Some(today - 1)).is_overdue(today) && !done.is_overdue(today));
        // Sorted: by group, then day, the important first.
        let mut list = vec![
            Task { uid: "c".into(), subject: "Later".into(), due: Some(today + 30), ..Default::default() },
            Task { uid: "b".into(), subject: "b".into(), due: Some(today), ..Default::default() },
            Task { uid: "a".into(), subject: "a".into(), due: Some(today), importance: 1, ..Default::default() },
            Task { uid: "d".into(), subject: "None".into(), ..Default::default() },
            Task { uid: "e".into(), subject: "Late".into(), due: Some(today - 2), ..Default::default() },
        ];
        sort_for_list(&mut list, today);
        let order: Vec<&str> = list.iter().map(|t| t.uid.as_str()).collect();
        assert_eq!(order, ["e", "a", "b", "c", "d"]);
    }

    #[test]
    fn statuses_read_by_name() {
        for s in TaskStatus::ALL {
            assert_eq!(TaskStatus::named(s.name()), Some(s));
        }
        assert_eq!(TaskStatus::named(" inprogress "), Some(TaskStatus::InProgress));
        assert_eq!(TaskStatus::named("done"), None);
        // An old file without the newer fields still reads.
        let t: Task = serde_json::from_str(r#"{"uid":"x","subject":"Call"}"#).unwrap();
        assert_eq!((t.status, t.percent, t.importance), (TaskStatus::NotStarted, 0, 0));
    }
}
