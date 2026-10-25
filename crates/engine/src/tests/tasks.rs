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
    done(s);
}
