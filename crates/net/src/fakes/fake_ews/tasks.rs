//! Exchange's part in a task written: a Completed status sets Outlook's "complete" flag
//! (`IsComplete`, PidLidTaskComplete), a status put back leaves it as it was (as the owner's
//! Exchange did: a task un-ticked elsewhere stayed struck through in Outlook), and the flag and the
//! day it was done, written as MAPI properties, are kept as `IsComplete` and `CompleteDate`.

use super::*;

/// PidLidTaskComplete's and PidLidTaskDateCompleted's ids in the Task property set.
const COMPLETE: &str = "33052";
const DATE_COMPLETED: &str = "33039";

fn task_property(u: &El) -> Option<&str> {
    u.find("ExtendedFieldURI").filter(|e| e.attr("DistinguishedPropertySetId") == Some("Task")).and_then(|e| e.attr("PropertyId"))
}

fn set_kid(task: &mut El, name: &str, text: &str) {
    task.kids.retain(|k| k.name != name);
    task.kids.push(El { name: name.into(), text: text.into(), ..Default::default() });
}

/// After one `SetItemField` or `DeleteItemField` (`u`) was applied to `task`.
pub(super) fn after_update(task: &mut El, u: &El) {
    let set = u.name == "SetItemField";
    match task_property(u) {
        Some(COMPLETE) => {
            let on = set && u.find("Value").is_some_and(|v| v.text.trim() == "true");
            set_kid(task, "IsComplete", if on { "true" } else { "false" });
        }
        Some(DATE_COMPLETED) if !set => task.kids.retain(|k| k.name != "CompleteDate"),
        _ => {}
    }
    // Kept as the fields they stand for, not as properties of their own.
    task.kids.retain(|k| k.name != "ExtendedProperty" || task_property(k).is_none());
    if set && u.find("Status").is_some_and(|s| s.text.trim() == "Completed") {
        set_kid(task, "IsComplete", "true");
        set_kid(task, "CompleteDate", "2026-10-28T10:00:00Z");
    }
}

/// A task just made: its status and flag, as an update of each would leave them.
pub(super) fn after_create(task: &mut El) {
    let written: Vec<El> = task.kids.clone();
    for k in written.iter().filter(|k| k.name == "Status" || (k.name == "ExtendedProperty" && task_property(k).is_some())) {
        let u = El { name: "SetItemField".into(), kids: vec![k.clone()], ..Default::default() };
        after_update(task, &u);
    }
}
