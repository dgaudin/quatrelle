//! Engine tests: Outlook's Tasks, kept here and with Exchange.

use super::*;
use quatrelle_model::dates::days_from_civil;
use quatrelle_model::pim::TaskStatus;

fn uid_of(r: &Value) -> String {
    r["uid"].as_str().unwrap_or_default().to_string()
}

/// A task made, changed as Outlook changes one, ticked and unticked, listed and deleted.
#[test]
fn a_task_goes_through_its_life() {
    let mut s = sample_session("tasks-life");
    let before = s.tasks("", true).len();
    let uid = uid_of(
        &s.run("task.create", &json!({"subject": " Devis Martin ", "due": "2026-10-16", "importance": "high", "categories": "Clients"})).unwrap(),
    );
    let t = s.find_task(&uid).unwrap().task;
    assert_eq!(
        (t.subject.as_str(), t.due, t.importance, t.categories.as_slice()),
        ("Devis Martin", Some(days_from_civil(2026, 10, 16)), 1, &["Clients".to_string()][..])
    );
    assert_eq!(s.tasks("", true).len(), before + 1);
    assert_eq!(s.tasks("martin", false).len(), 1, "found by its subject");

    // A start after the due date moves the due date; a percentage starts it.
    s.run("task.update", &json!({"uid": uid, "start": "2026-10-20", "percent": 40, "notes": "Relancer vendredi"})).unwrap();
    let t = s.find_task(&uid).unwrap().task;
    assert_eq!((t.start, t.due), (Some(days_from_civil(2026, 10, 20)), Some(days_from_civil(2026, 10, 20))));
    assert_eq!((t.status, t.percent, t.notes.as_str()), (TaskStatus::InProgress, 40, "Relancer vendredi"));
    // Its reminder, at 8:00 when given a day; taken off with null.
    s.run("task.update", &json!({"uid": uid, "reminder": "2026-10-19"})).unwrap();
    assert_eq!(s.find_task(&uid).unwrap().task.reminder, Some(days_from_civil(2026, 10, 19) * 86_400 + 8 * 3600));
    // Ticked: complete, its reminder done with, out of the list of what's to do.
    s.run("task.complete", &json!({"uid": uid})).unwrap();
    let t = s.find_task(&uid).unwrap().task;
    assert_eq!((t.status, t.percent, t.reminder, t.completed), (TaskStatus::Completed, 100, None, Some(days_from_civil(2026, 10, 14))));
    assert!(s.tasks("martin", false).is_empty() && s.tasks("martin", true).len() == 1);
    assert_eq!(s.run("task.complete", &json!({"uid": uid})).unwrap()["changed"], false);
    s.run("task.complete", &json!({"uid": uid, "done": false})).unwrap();
    assert_eq!(s.find_task(&uid).unwrap().task.status, TaskStatus::NotStarted);
    // The due date taken off.
    s.run("task.update", &json!({"uid": uid, "due": null, "start": ""})).unwrap();
    let t = s.find_task(&uid).unwrap().task;
    assert_eq!((t.start, t.due), (None, None));

    let listed = s.run("task.list", &json!({"query": "martin"})).unwrap();
    assert_eq!(listed["tasks"][0]["subject"], "Devis Martin");
    s.run("task.delete", &json!({"uid": uid})).unwrap();
    assert!(s.find_task(&uid).is_none());
    done(s);
}

/// Params that make no sense are refused, and change nothing.
#[test]
fn a_task_refuses_what_makes_no_sense() {
    let mut s = sample_session("tasks-refused");
    for bad in [
        json!({"subject": "  "}),
        json!({}),
        json!({"subject": "x", "due": "tomorrow"}),
        json!({"subject": "x", "due": "2026-10-16T09:00"}),
        json!({"subject": "x", "percent": 120}),
        json!({"subject": "x", "status": "Done"}),
        json!({"subject": "x", "importance": "urgent"}),
        json!({"subject": "x", "reminder": "soon"}),
    ] {
        assert!(s.run("task.create", &bad).is_err(), "{bad}");
    }
    let uid = uid_of(&s.run("task.create", &json!({"subject": "Ok"})).unwrap());
    assert!(s.run("task.update", &json!({"uid": uid, "subject": ""})).is_err());
    assert!(s.run("task.update", &json!({"uid": "nope", "subject": "x"})).is_err());
    assert_eq!(s.find_task(&uid).unwrap().task.subject, "Ok");
    done(s);
}

/// An Exchange account's tasks go to its Tasks folder: what's changed asks for a sync, what the
/// server says follows, and one deleted is deleted there next time.
#[test]
fn an_exchange_accounts_tasks_go_to_its_tasks_folder() {
    let mut s = Session::open(Some(temp("tasks-ews")), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s
        .run("account.add", &json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    let uid = uid_of(&s.run("task.create", &json!({"subject": "Sync me"})).unwrap());
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::SyncDav { .. })), "asks for a sync");
    assert!(s.pim[&id].tasks[0].dirty);
    let sent = quatrelle_model::pim::StoredTask { href: "task:X1".into(), task: s.pim[&id].tasks[0].task.clone(), ..Default::default() };
    let server_copy =
        quatrelle_model::pim::StoredTask { task: quatrelle_model::pim::Task { uid: "task:X1".into(), ..sent.task.clone() }, ..sent.clone() };
    let outlook = quatrelle_model::pim::StoredTask {
        href: "task:X2".into(),
        task: quatrelle_model::pim::Task { uid: "task:X2".into(), subject: "From Outlook".into(), ..Default::default() },
        ..Default::default()
    };
    let sync = quatrelle_model::pim::DavSync { pushed_tasks: vec![sent], tasks: Some(vec![server_copy, outlook]), ..Default::default() };
    s.apply_dav(&id, &sync);
    let subjects: Vec<String> = s.tasks("", true).into_iter().map(|r| r.task.subject).collect();
    assert_eq!(subjects.len(), 2, "{subjects:?}: the task sent isn't doubled");
    assert!(s.find_task(&uid).is_none() && s.find_task("task:X1").is_some(), "known by its item on the server from then on");
    // Changed: sent again, the same item.
    s.run("task.complete", &json!({"uid": "task:X2"})).unwrap();
    let x2 = s.pim[&id].tasks.iter().find(|t| t.href == "task:X2").unwrap();
    assert!(x2.dirty && x2.task.is_complete());
    s.run("task.delete", &json!({"uid": "task:X1"})).unwrap();
    assert!(s.pim[&id].deleted.iter().any(|t| t.href == "task:X1"), "deleted on the server next time");
    // The server not told yet (it was busy) while its folder was read: still deleted here.
    let x2 = s.pim[&id].tasks.iter().find(|t| t.href == "task:X2").unwrap().clone();
    let read = |href: &str| quatrelle_model::pim::StoredTask {
        href: href.into(),
        task: quatrelle_model::pim::Task { uid: href.into(), subject: href.into(), ..Default::default() },
        ..Default::default()
    };
    let not_yet = quatrelle_model::pim::DavSync { tasks: Some(vec![read("task:X1"), x2.clone()]), ..Default::default() };
    s.apply_dav(&id, &not_yet);
    assert!(s.find_task("task:X1").is_none(), "doesn't come back");
    assert!(s.pim[&id].deleted.iter().any(|t| t.href == "task:X1"), "still to delete there");
    // Deleted there: forgotten.
    let gone = quatrelle_model::pim::DavSync { deleted: vec!["task:X1".into()], tasks: Some(vec![x2]), ..Default::default() };
    s.apply_dav(&id, &gone);
    assert!(s.pim[&id].deleted.is_empty() && s.find_task("task:X1").is_none());
    done(s);
}

/// A change Exchange tells of in the Tasks folder reads that folder alone (28/10): the calendar,
/// the contacts and the notes it brings nothing of stay as they were.
#[test]
fn a_read_of_the_tasks_alone_leaves_the_rest() {
    use quatrelle_model::ical::CalEvent;
    use quatrelle_model::pim::{BookInfo, CalendarInfo, DavSync, Note, StoredCard, StoredEvent, StoredNote, StoredTask, Task};
    use quatrelle_model::vcard::Card;
    let mut s = Session::open(Some(temp("tasks-alone")), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s
        .run("account.add", &json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    let ews = || Some("ews".to_string());
    let event = CalEvent { uid: "e1".into(), summary: "Budget".into(), start: NOW + 3600, end: NOW + 7200, ..Default::default() };
    let full = DavSync {
        caldav_home: ews(),
        carddav_home: ews(),
        checked: NOW,
        calendars: Some(vec![CalendarInfo { id: "ews:calendar".into(), name: "Calendar".into(), ..Default::default() }]),
        events: vec![StoredEvent { calendar: "ews:calendar".into(), href: "E1".into(), event, ..Default::default() }],
        synced_calendars: vec!["ews:calendar".into()],
        books: Some(vec![BookInfo { id: "ews:contacts".into(), name: "Contacts".into(), ..Default::default() }]),
        cards: vec![StoredCard {
            book: "ews:contacts".into(),
            href: "contact:c1".into(),
            card: Card { uid: "c1".into(), ..Default::default() },
            ..Default::default()
        }],
        synced_books: vec!["ews:contacts".into()],
        notes: Some(vec![StoredNote { href: "note:n1".into(), note: Note { uid: "n1".into(), ..Default::default() }, ..Default::default() }]),
        tasks: Some(Vec::new()),
        ..Default::default()
    };
    s.apply_dav(&id, &full);
    let before = s.pim[&id].clone();
    assert_eq!((before.events.len(), before.cards.len(), before.notes.len()), (1, 1, 1));
    let made = StoredTask {
        href: "task:T3".into(),
        task: Task { uid: "task:T3".into(), subject: "test3".into(), ..Default::default() },
        ..Default::default()
    };
    let alone = DavSync { caldav_home: ews(), carddav_home: ews(), checked: NOW + 60, tasks: Some(vec![made]), ..Default::default() };
    s.apply_dav(&id, &alone);
    assert!(s.find_task("task:T3").is_some(), "the task made in Outlook");
    let p = &s.pim[&id];
    assert_eq!(
        (&p.calendars, &p.events, &p.books, &p.cards, &p.notes),
        (&before.calendars, &before.events, &before.books, &before.cards, &before.notes)
    );
    done(s);
}

/// A task's reminder comes in the Reminders with the appointments'; snoozed, it waits; dismissed,
/// it's taken off the task (as Outlook does, and Exchange learns it); done, it never comes.
#[test]
fn a_tasks_reminder_comes_and_goes() {
    let mut s = sample_session("tasks-reminders");
    // NOW is 12:00 UTC on Wednesday 14 October 2026.
    let uid = uid_of(&s.run("task.create", &json!({"subject": "Rappeler Ana", "due": "2026-10-14", "reminder": "2026-10-14T11:00"})).unwrap());
    let due = |s: &Session| s.due_reminders(NOW).into_iter().find(|r| r.uid == uid);
    let r = due(&s).expect("its time has come");
    assert!(r.task && r.all_day && r.summary == "Rappeler Ana");
    s.snooze_reminder(&r.key, NOW + 600);
    assert!(due(&s).is_none(), "snoozed");
    assert!(s.due_reminders(NOW + 700).iter().any(|x| x.uid == uid), "back after the snooze");
    s.dismiss_reminders(Some(&r.key), NOW + 700);
    assert!(s.find_task(&uid).unwrap().task.reminder.is_none(), "taken off the task");
    assert!(s.due_reminders(NOW + 700).iter().all(|x| x.uid != uid));
    // One done never reminds.
    let other = uid_of(&s.run("task.create", &json!({"subject": "Fini", "reminder": "2026-10-14T10:00"})).unwrap());
    assert!(s.due_reminders(NOW).iter().any(|x| x.uid == other));
    s.run("task.complete", &json!({"uid": other})).unwrap();
    assert!(s.due_reminders(NOW).iter().all(|x| x.uid != other));
    done(s);
}
