//! Commands: Outlook's Tasks.

use crate::{TaskChange, parse_local};
use quatrelle_model::pim::TaskStatus;

use super::*;

/// What a task.create or task.update can say, as their params describe it.
macro_rules! fields {
    () => {
        r#""subject"?: string, "notes"?: string, "start"?: "YYYY-MM-DD" | null, "due"?: "YYYY-MM-DD" | null, "status"?: "NotStarted"|"InProgress"|"Completed"|"WaitingOnOthers"|"Deferred", "percent"?: 0-100, "importance"?: "high"|"normal"|"low", "reminder"?: "YYYY-MM-DD[THH:MM]" (8:00 when no time) | null, "categories"?: [string] | "Work, Clients", "private"?: bool"#
    };
}

pub(super) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("task.list", "Tasks", "Tasks", |s, v| {
            let list: Vec<Value> =
                s.tasks(p::str(v, "query").unwrap_or(""), p::bool(v, "completed").unwrap_or(false)).into_iter().map(|r| task_json(&r)).collect();
            Ok(json!({"tasks": list}))
        })
        .params(r#"{"query"?: string, "completed"?: bool (the completed ones too)}"#),
        CommandSpec::new("task.create", "New task", "Tasks", |s, v| {
            let change = change_of(v, &s.clock)?;
            s.create_task(p::str(v, "account"), change)
        })
        .params(concat!(r#"{"subject": string, "#, fields!(), r#", "account"?: string}"#)),
        CommandSpec::new("task.update", "Edit task", "Tasks", |s, v| {
            let change = change_of(v, &s.clock)?;
            s.update_task(p::req_str(v, "uid")?, change)
        })
        .params(concat!(r#"{"uid": string, "#, fields!(), "}")),
        CommandSpec::new("task.complete", "Mark complete", "Tasks", |s, v| {
            s.complete_task(p::req_str(v, "uid")?, p::bool(v, "done").unwrap_or(true))
        })
        .params(r#"{"uid": string, "done"?: bool (false: not complete again; default true)}"#),
        CommandSpec::new("task.delete", "Delete task", "Tasks", |s, v| s.delete_task(p::req_str(v, "uid")?)).params(r#"{"uid": string}"#),
    ]
}

/// A task as the commands give it.
fn task_json(r: &crate::TaskRef) -> Value {
    let t = &r.task;
    let day = |d: Option<i64>| {
        d.map(|d| {
            let (y, m, dd) = quatrelle_model::dates::civil_from_days(d);
            format!("{y:04}-{m:02}-{dd:02}")
        })
    };
    json!({
        "uid": t.uid, "subject": t.subject, "notes": t.notes, "start": day(t.start), "due": day(t.due), "status": t.status.name(),
        "percent": t.percent, "completed": day(t.completed), "importance": t.importance, "reminder": t.reminder,
        "categories": t.categories, "private": t.private, "account": r.account,
    })
}

/// What the params change (each one absent stays as it is).
fn change_of(v: &Value, clock: &quatrelle_model::dates::Clock) -> Result<TaskChange, CmdError> {
    let date = |k: &str| -> Result<Option<Option<i64>>, CmdError> {
        match v.get(k) {
            None => Ok(None),
            Some(Value::Null) => Ok(Some(None)),
            Some(Value::String(x)) if x.trim().is_empty() => Ok(Some(None)),
            Some(Value::String(x)) => match parse_local(x, clock) {
                Some((t, true)) => Ok(Some(Some(t.div_euclid(86_400)))),
                _ => Err(CmdError::Params(format!("`{k}` is a day, YYYY-MM-DD"))),
            },
            Some(_) => Err(CmdError::Params(format!("`{k}` is a day, YYYY-MM-DD"))),
        }
    };
    let reminder = match v.get("reminder") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(x)) if matches!(x.trim(), "" | "none") => Some(None),
        Some(x) => {
            let text = x.as_str().map(str::trim).map(|t| if t.len() == 10 { format!("{t}T08:00") } else { t.to_string() });
            match text.as_deref().and_then(|t| parse_local(t, clock)).or_else(|| x.as_i64().map(|n| (n, false))) {
                Some((t, false)) => Some(Some(t)),
                _ => return Err(CmdError::Params("`reminder` is YYYY-MM-DDTHH:MM, or null for none".into())),
            }
        }
    };
    let status = match p::str(v, "status") {
        Some(x) => Some(TaskStatus::named(x).ok_or_else(|| CmdError::Params(format!("`{x}` is not a task status")))?),
        None => None,
    };
    let percent = match v.get("percent") {
        Some(x) => Some(
            x.as_u64().filter(|n| *n <= 100).and_then(|n| u8::try_from(n).ok()).ok_or_else(|| CmdError::Params("`percent` is 0 to 100".into()))?,
        ),
        None => None,
    };
    let importance = match v.get("importance") {
        None => None,
        Some(x) => Some(match (x.as_str().map(|s| s.trim().to_ascii_lowercase()), x.as_i64()) {
            (Some(s), _) if s == "high" => 1,
            (Some(s), _) if s == "low" => -1,
            (Some(s), _) if s == "normal" => 0,
            (_, Some(n)) if (-1..=1).contains(&n) => i8::try_from(n).unwrap_or(0),
            _ => return Err(CmdError::Params("`importance` is high, normal or low".into())),
        }),
    };
    Ok(TaskChange {
        subject: p::str(v, "subject").map(str::to_string),
        notes: p::str(v, "notes").map(str::to_string),
        start: date("start")?,
        due: date("due")?,
        status,
        percent,
        importance,
        reminder,
        categories: super::pim::categories(v),
        private: p::bool(v, "private"),
    })
}
