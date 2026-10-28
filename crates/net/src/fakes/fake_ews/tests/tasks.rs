//! Tasks over EWS: the Tasks folder read, a new task made there, a changed one updated in place.

use super::*;

#[test]
fn tasks_sync_with_the_tasks_folder() {
    use quatrelle_model::pim::{Pim, StoredTask, Task, TaskStatus};
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let tasks = d.folder("Tasks", None, Some("tasks"), "IPF.Task");
    let quote = parse(
        "<t:Task><t:ItemClass>IPM.Task</t:ItemClass><t:Subject>Devis Martin</t:Subject><t:Body BodyType=\"Text\">Relancer</t:Body>\
         <t:Importance>High</t:Importance><t:ReminderIsSet>false</t:ReminderIsSet><t:DueDate>2026-10-26T23:00:00Z</t:DueDate>\
         <t:PercentComplete>0</t:PercentComplete><t:Status>NotStarted</t:Status></t:Task>",
    );
    d.pim.push(("T1".into(), tasks.clone(), quote.kids[0].clone()));
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let day = |y, m, d| quatrelle_model::dates::days_from_civil(y, m, d);

    // A first sync reads it, and makes the one written here.
    let mut pim = Pim::default();
    let local = Task { uid: "t-local".into(), subject: "Commander le papier".into(), due: Some(day(2026, 10, 30)), ..Default::default() };
    pim.tasks.push(StoredTask { task: local, dirty: true, ..Default::default() });
    let sync = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    let list = sync.tasks.clone().unwrap();
    let martin = list.iter().find(|s| s.href == "task:T1").unwrap();
    assert_eq!((martin.task.subject.as_str(), martin.task.due, martin.task.importance), ("Devis Martin", Some(day(2026, 10, 27)), 1));
    assert_eq!(sync.pushed_tasks.len(), 1);
    let made = sync.pushed_tasks[0].href.clone();
    assert!(list.iter().any(|s| s.href == made && s.task.subject == "Commander le papier" && s.task.due == Some(day(2026, 10, 30))));

    // Marked complete here: the same item on the server, completed.
    let mut done = martin.clone();
    done.task.set_complete(true, day(2026, 10, 27));
    done.dirty = true;
    let pim = Pim { tasks: vec![done], ..Default::default() };
    let sync = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    assert_eq!(sync.pushed_tasks[0].href, "task:T1", "updated in place, not made again");
    let now = sync.tasks.unwrap().into_iter().find(|s| s.href == "task:T1").unwrap();
    assert_eq!((now.task.status, now.task.percent), (TaskStatus::Completed, 100));
    let on_server = |id: &str| server.data.lock().unwrap().pim.iter().find(|(i, _, _)| i == id).map(|(_, _, el)| el.clone()).unwrap();
    {
        let data = server.data.lock().unwrap();
        assert_eq!(data.pim.iter().filter(|(_, f, _)| *f == tasks).count(), 2, "nothing made twice");
    }
    assert_eq!(on_server("T1").text_of("IsComplete"), Some("true"), "ticked in Outlook");

    // Un-ticked here (the owner's slip): un-ticked in Outlook too, not only its status put back,
    // which leaves Outlook's flag, and the task struck through there.
    let mut undone = now.clone();
    undone.task.set_complete(false, day(2026, 10, 27));
    undone.dirty = true;
    let sync = crate::ews_pim::sync(&c, &Pim { tasks: vec![undone], ..Default::default() }, 1_792_000_000).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    let back = sync.tasks.unwrap().into_iter().find(|s| s.href == "task:T1").unwrap();
    assert_eq!((back.task.status, back.task.percent, back.task.completed), (TaskStatus::NotStarted, 0, None));
    let t1 = on_server("T1");
    assert_eq!((t1.text_of("IsComplete"), t1.text_of("CompleteDate"), t1.text_of("Status")), (Some("false"), None, Some("NotStarted")));

    // Deleted in Outlook meanwhile, then changed here: made again rather than lost.
    server.data.lock().unwrap().pim.retain(|(id, _, _)| id != "T1");
    let mut again = now.clone();
    again.task.subject = "Devis Martin (relancé)".into();
    again.dirty = true;
    let sync = crate::ews_pim::sync(&c, &Pim { tasks: vec![again], ..Default::default() }, 1_792_000_000).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    let remade = sync.tasks.unwrap().into_iter().find(|s| s.task.subject == "Devis Martin (relancé)").unwrap().href;

    // Deleted here: deleted in Outlook too (Exchange deletes a task only when told which of its
    // occurrences go), and not read back.
    let deleted = Pim { deleted: vec![quatrelle_model::pim::Tombstone { href: remade.clone(), ..Default::default() }], ..Default::default() };
    let sync = crate::ews_pim::sync(&c, &deleted, 1_792_000_000).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    assert_eq!(sync.deleted, std::slice::from_ref(&remade));
    assert!(!sync.tasks.unwrap().iter().any(|s| s.href == remade), "gone from the Tasks folder");
    // Refused by the server (someone else's, which this account may only read): said, and not
    // tried again.
    let theirs = parse("<t:Task><t:ItemClass>IPM.Task</t:ItemClass><t:Subject>Leur tâche</t:Subject></t:Task>");
    server.data.lock().unwrap().pim.push(("B1".into(), "mailbox:boss@contoso.example".into(), theirs.kids[0].clone()));
    let refused = Pim { deleted: vec![quatrelle_model::pim::Tombstone { href: "task:B1".into(), ..Default::default() }], ..Default::default() };
    let sync = crate::ews_pim::sync(&c, &refused, 1_792_000_000).unwrap();
    assert_eq!(sync.deleted, ["task:B1"]);
    assert!(sync.errors.iter().any(|e| e.starts_with("not deleted on the server")), "{:?}", sync.errors);
}
