//! EWS tasks: the mailbox's Tasks folder ([MS-OXOTASK]), read as tasks and written back field
//! by field — a task changed here is updated in place (its item, its reminder and its history
//! in Outlook stay), a new one created, one deleted in Outlook meanwhile made again.
//!
//! A task's start and due dates have no time of day. Exchange gives them as a moment, local
//! midnight as UTC or midnight UTC depending on who wrote them; the nearest UTC midnight is the
//! day either way (as all-day appointments are read), and a day goes back as its UTC midnight.

use quatrelle_model::pim::{StoredTask, Task, TaskStatus};

use super::*;

/// The day (since 1970) a task's date is.
fn day_of(text: &str) -> Option<i64> {
    quatrelle_jmap::utc_date(text.trim()).map(|t| (t + 43_200).div_euclid(86_400))
}

/// A task of the Tasks folder (`IPM.Task`; a task request or anything else is left out).
pub fn task_of(it: &El) -> Option<StoredTask> {
    let class = it.text_of("ItemClass").map(str::trim).unwrap_or("IPM.Task");
    if it.name != "Task" || !(class == "IPM.Task" || class.starts_with("IPM.Task.")) {
        return None;
    }
    let id = it.child("ItemId")?;
    let href = format!("task:{}", id.attr("Id")?);
    let status = it.text_of("Status").and_then(TaskStatus::named).unwrap_or_default();
    let percent = it.text_of("PercentComplete").and_then(|p| p.trim().parse::<f64>().ok()).map_or(0, |p| p.clamp(0.0, 100.0).round() as u8);
    let reminder = (it.text_of("ReminderIsSet") == Some("true")).then(|| it.text_of("ReminderDueBy").and_then(quatrelle_jmap::utc_date)).flatten();
    let categories = it
        .child("Categories")
        .map(|c| c.children("String").map(|s| s.text.trim().to_string()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();
    let task = Task {
        uid: href.clone(),
        subject: text(it, "Subject"),
        notes: it.text_of("Body").unwrap_or_default().replace("\r\n", "\n").trim_end().to_string(),
        start: it.text_of("StartDate").and_then(day_of),
        due: it.text_of("DueDate").and_then(day_of),
        completed: it.text_of("CompleteDate").and_then(day_of).filter(|_| status == TaskStatus::Completed),
        percent: if status == TaskStatus::Completed { 100 } else { percent.min(99) },
        status,
        importance: match it.text_of("Importance").map(str::trim) {
            Some("High") => 1,
            Some("Low") => -1,
            _ => 0,
        },
        reminder,
        categories,
        private: matches!(it.text_of("Sensitivity").map(str::trim), Some("Private" | "Confidential")),
        modified: it.text_of("LastModifiedTime").and_then(quatrelle_jmap::utc_date).unwrap_or(0),
    };
    Some(StoredTask { href, etag: id.attr("ChangeKey").unwrap_or("").to_string(), task, dirty: false })
}

/// A task's fields in the order EWS's schema wants them (the item's, then the task's), each
/// with the field URI that names it in an update; the dates and categories it hasn't are
/// taken off an item being updated (`absent`).
fn task_fields(t: &Task) -> (Vec<(&'static str, String)>, Vec<&'static str>) {
    let mut f = vec![
        ("item:Subject", format!("<t:Subject>{}</t:Subject>", esc(&t.subject))),
        ("item:Sensitivity", format!("<t:Sensitivity>{}</t:Sensitivity>", if t.private { "Private" } else { "Normal" })),
        ("item:Body", format!("<t:Body BodyType=\"Text\">{}</t:Body>", esc(&t.notes))),
    ];
    let mut absent = Vec::new();
    if t.categories.is_empty() {
        absent.push("item:Categories");
    } else {
        let list: String = t.categories.iter().map(|c| format!("<t:String>{}</t:String>", esc(c))).collect();
        f.push(("item:Categories", format!("<t:Categories>{list}</t:Categories>")));
    }
    let importance = match t.importance {
        1.. => "High",
        ..0 => "Low",
        0 => "Normal",
    };
    f.push(("item:Importance", format!("<t:Importance>{importance}</t:Importance>")));
    if let Some(at) = t.reminder {
        f.push(("item:ReminderDueBy", format!("<t:ReminderDueBy>{}</t:ReminderDueBy>", utc(at))));
    }
    f.push(("item:ReminderIsSet", format!("<t:ReminderIsSet>{}</t:ReminderIsSet>", t.reminder.is_some())));
    match t.due {
        Some(d) => f.push(("task:DueDate", format!("<t:DueDate>{}</t:DueDate>", utc(d.saturating_mul(86_400))))),
        None => absent.push("task:DueDate"),
    }
    f.push(("task:PercentComplete", format!("<t:PercentComplete>{}</t:PercentComplete>", t.percent.min(100))));
    match t.start {
        Some(d) => f.push(("task:StartDate", format!("<t:StartDate>{}</t:StartDate>", utc(d.saturating_mul(86_400))))),
        None => absent.push("task:StartDate"),
    }
    f.push(("task:Status", format!("<t:Status>{}</t:Status>", t.status.name())));
    (f, absent)
}

/// A new task in the mailbox's Tasks folder.
fn create_task(c: &Client, t: &Task) -> Result<(String, String)> {
    let body: String = task_fields(t).0.iter().map(|(_, x)| x.as_str()).collect();
    c.call_one(&format!(
        "<m:CreateItem><m:SavedItemFolderId><t:DistinguishedFolderId Id=\"tasks\"/></m:SavedItemFolderId>\
         <m:Items><t:Task>{body}</t:Task></m:Items></m:CreateItem>"
    ))
    .map(|m| new_id(&m))
}

/// A task changed field by field, over whatever the server has (as an appointment is when no
/// one is to be told).
fn update_task(c: &Client, id: &str, t: &Task) -> Result<(String, String)> {
    let (fields, absent) = task_fields(t);
    let mut updates: String =
        fields.iter().map(|(uri, x)| format!("<t:SetItemField><t:FieldURI FieldURI=\"{uri}\"/><t:Task>{x}</t:Task></t:SetItemField>")).collect();
    for uri in absent {
        updates.push_str(&format!("<t:DeleteItemField><t:FieldURI FieldURI=\"{uri}\"/></t:DeleteItemField>"));
    }
    c.call_one(&format!(
        "<m:UpdateItem ConflictResolution=\"AlwaysOverwrite\"><m:ItemChanges><t:ItemChange><t:ItemId Id=\"{}\"/>\
         <t:Updates>{updates}</t:Updates></t:ItemChange></m:ItemChanges></m:UpdateItem>",
        esc(id)
    ))
    .map(|m| {
        let (new, key) = new_id(&m);
        (if new.is_empty() { id.to_string() } else { new }, key)
    })
}

/// Tasks: what changed here goes up, then the folder is read again. A mailbox without one
/// keeps tasks on this computer.
pub(super) fn read_tasks(c: &Client, pim: &Pim, out: &mut DavSync) {
    for s in pim.tasks.iter().filter(|s| s.dirty) {
        let r = match s.href.strip_prefix("task:") {
            Some(id) => match update_task(c, id, &s.task) {
                // Deleted in Outlook meanwhile: the change made here keeps it, as a new task.
                Err(NetError::No(why)) if why.starts_with("not on the server") => create_task(c, &s.task),
                r => r,
            },
            None => create_task(c, &s.task),
        };
        match r {
            Ok((id, etag)) => out.pushed_tasks.push(StoredTask { href: format!("task:{id}"), etag, task: s.task.clone(), dirty: false }),
            Err(x) => out.errors.push(format!("{}: {x}", s.task.subject)),
        }
    }
    match items(c, "tasks", None) {
        Ok(list) => out.tasks = Some(list.iter().filter_map(task_of).collect()),
        Err(e) => log::info!("no Tasks folder: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ews::parse;

    #[test]
    fn a_task_reads_as_outlook_wrote_it() {
        // As Exchange gives one Outlook wrote in Paris: its dates as local midnight in UTC.
        let doc = parse(
            "<t:Task><t:ItemId Id=\"T1\" ChangeKey=\"k1\"/><t:ItemClass>IPM.Task</t:ItemClass><t:Subject>Devis Martin</t:Subject>\
             <t:Sensitivity>Private</t:Sensitivity><t:Body BodyType=\"Text\">Relancer\r\nvendredi</t:Body>\
             <t:Categories><t:String>Clients</t:String></t:Categories><t:Importance>High</t:Importance>\
             <t:ReminderDueBy>2026-10-27T08:00:00Z</t:ReminderDueBy><t:ReminderIsSet>true</t:ReminderIsSet>\
             <t:DueDate>2026-10-26T23:00:00Z</t:DueDate><t:PercentComplete>25</t:PercentComplete>\
             <t:StartDate>2026-10-25T22:00:00Z</t:StartDate><t:Status>InProgress</t:Status></t:Task>",
        );
        let s = task_of(&doc.kids[0]).unwrap();
        let t = &s.task;
        let day = |y, m, d| quatrelle_model::dates::days_from_civil(y, m, d);
        assert_eq!((s.href.as_str(), s.etag.as_str(), t.uid.as_str()), ("task:T1", "k1", "task:T1"));
        assert_eq!((t.subject.as_str(), t.notes.as_str()), ("Devis Martin", "Relancer\nvendredi"));
        assert_eq!((t.due, t.start), (Some(day(2026, 10, 27)), Some(day(2026, 10, 26))));
        assert_eq!((t.status, t.percent, t.importance, t.private), (TaskStatus::InProgress, 25, 1, true));
        assert_eq!(t.reminder, quatrelle_jmap::utc_date("2026-10-27T08:00:00Z"));
        assert_eq!(t.categories, ["Clients"]);
        // Midnight UTC is the same day; a task request or a message isn't a task.
        let utc = parse(
            "<t:Task><t:ItemId Id=\"T2\"/><t:Subject>x</t:Subject><t:DueDate>2026-10-27T00:00:00Z</t:DueDate><t:Status>Completed</t:Status></t:Task>",
        );
        let done = task_of(&utc.kids[0]).unwrap().task;
        assert_eq!((done.due, done.percent, done.reminder), (Some(day(2026, 10, 27)), 100, None));
        let request = parse("<t:Task><t:ItemId Id=\"T3\"/><t:ItemClass>IPM.TaskRequest</t:ItemClass></t:Task>");
        assert!(task_of(&request.kids[0]).is_none());
        assert!(task_of(&parse("<t:Message><t:ItemId Id=\"M\"/></t:Message>").kids[0]).is_none());
    }

    #[test]
    fn a_tasks_fields_follow_the_schema() {
        let due = quatrelle_model::dates::days_from_civil(2026, 10, 27);
        let t = Task { subject: "A & B".into(), due: Some(due), status: TaskStatus::Completed, percent: 100, ..Default::default() };
        let (fields, absent) = task_fields(&t);
        let names: Vec<&str> = fields.iter().map(|(u, _)| *u).collect();
        assert_eq!(
            names,
            [
                "item:Subject",
                "item:Sensitivity",
                "item:Body",
                "item:Importance",
                "item:ReminderIsSet",
                "task:DueDate",
                "task:PercentComplete",
                "task:Status"
            ]
        );
        assert_eq!(absent, ["item:Categories", "task:StartDate"]);
        assert!(fields[0].1.contains("A &amp; B"));
        assert_eq!(fields[5].1, "<t:DueDate>2026-10-27T00:00:00Z</t:DueDate>");
    }
}
