// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: Outlook's Tasks, kept here and with Exchange.

use super::*;
use quatrelle_model::dates::days_from_civil;
use quatrelle_model::pim::TaskStatus;

fn uid_of(r: &Value) -> String {
    r["uid"].as_str().unwrap_or_default().to_string()
}

/// A task made, changed as Outlook changes one, ticked and unticked, listed and deleted.
/// The Tasks module's views and arrangements: Active, Completed, the next seven days, Overdue
/// and Prioritized filter or order the list; Arrange By orders it by start, importance,
/// categories, folder or type, remembered per list.
#[test]
fn task_views_filter_and_arrange() {
    use quatrelle_model::pim::tasks::{TaskArrange, TaskView};
    let mut s = sample_session("task-views");
    let today = quatrelle_model::dates::today(&s.clock);
    let day = |n| quatrelle_model::dates::day_string(today + n);
    let make = |s: &mut Session, subject: &str, due: Option<&str>, importance: &str| -> String {
        let mut p = json!({"subject": subject, "importance": importance});
        if let Some(d) = due {
            p["due"] = json!(d);
        }
        uid_of(&s.run("task.create", &p).unwrap())
    };
    let overdue = make(&mut s, "Overdue one", Some(&day(-2)), "normal");
    let soon = make(&mut s, "Due soon", Some(&day(2)), "normal");
    let far = make(&mut s, "Due far", Some(&day(40)), "high");
    let finished = make(&mut s, "Done one", Some(&day(2)), "low");
    s.run("task.complete", &json!({"uid": finished})).unwrap();
    let no_date = make(&mut s, "No date", None, "normal");
    let subjects = |s: &Session, view: TaskView, arrange: TaskArrange| -> Vec<String> {
        s.todo_view("todo", "", true, view, arrange).iter().map(|r| r.subject().to_string()).collect()
    };

    // Active leaves the done one out; Completed keeps it alone; Overdue only the late one.
    let active = subjects(&s, TaskView::Active, TaskArrange::DueDate);
    assert!(!active.contains(&"Done one".to_string()) && active.contains(&"Overdue one".to_string()));
    let completed = subjects(&s, TaskView::Completed, TaskArrange::DueDate);
    assert!(completed.contains(&"Done one".to_string()) && !completed.contains(&"Overdue one".to_string()), "{completed:?}");
    let overdue_view = subjects(&s, TaskView::Overdue, TaskArrange::DueDate);
    assert!(overdue_view.contains(&"Overdue one".to_string()), "{overdue_view:?}");
    // The next seven days: due soon, and the done one if completed is shown with the view, but
    // not the far one or one with no date.
    let week = subjects(&s, TaskView::NextSevenDays, TaskArrange::DueDate);
    assert!(week.contains(&"Due soon".to_string()) && !week.contains(&"Due far".to_string()) && !week.contains(&"No date".to_string()), "{week:?}");
    // Prioritized: high first, then by due day.
    let prioritized = subjects(&s, TaskView::Prioritized, TaskArrange::DueDate);
    let at = |subject: &str| prioritized.iter().position(|x| x == subject).unwrap_or_else(|| panic!("{subject} in {prioritized:?}"));
    assert!(at("Due far") < at("Overdue one"), "{prioritized:?}");
    // Arrange by importance, categories, start date and type.
    let by_importance = s.todo_view("todo", "", true, TaskView::SimpleList, TaskArrange::Importance);
    let at = |subject: &str| by_importance.iter().position(|r| r.subject() == subject).unwrap_or_else(|| panic!("{subject}"));
    assert!(at("Due far") < at("Overdue one"), "high first");
    let by_start = s.todo_view("todo", "", true, TaskView::SimpleList, TaskArrange::StartDate);
    assert!(by_start.iter().position(|r| r.uid() == no_date).is_some());
    s.run("task.view", &json!({"list": "todo", "view": "overdue", "arrange": "importance"})).unwrap();
    assert_eq!(s.task_view("todo"), (TaskView::Overdue, TaskArrange::Importance));
    let listed = s.run("task.list", &json!({"list": "todo", "completed": true})).unwrap();
    let names: Vec<&str> = listed["tasks"].as_array().unwrap().iter().filter_map(|t| t["subject"].as_str()).collect();
    assert!(names.contains(&"Overdue one"));
    // Views are remembered per list: another list keeps its own (Simple, by due day).
    assert_eq!(s.task_view("sample"), (TaskView::SimpleList, TaskArrange::DueDate));
    // Each of the ids is a view, and one that isn't is refused.
    for v in TaskView::ALL {
        assert_eq!(TaskView::from_code(v.code()), Some(v));
    }
    for a in TaskArrange::ALL {
        assert_eq!(TaskArrange::from_code(a.code()), Some(a));
    }
    assert!(s.run("task.view", &json!({"view": "nope"})).is_err());
    // (The rows made here are the sample's task list; the overdue one is still incomplete.)
    assert!(s.find_task(&overdue).is_some());
    assert!(s.find_task(&soon).is_some_and(|t| t.task.due.is_some()));
    assert!(s.find_task(&far).is_some());
    done(s);
}

/// The My Tasks lists: the To-Do List, each account's tasks, its online archive's (read-only:
/// every change refused) and the shared mailboxes opened in it; a task typed in a list goes to
/// its account.
#[test]
fn task_lists_are_per_account_with_the_archive_read_only() {
    let mut s = sample_session("task-lists");
    let lists = s.task_lists();
    assert_eq!(lists.first().map(|l| l.id.as_str()), Some("todo"));
    assert!(lists.iter().any(|l| l.id == "sample" && !l.read_only), "{lists:?}");
    assert!(!lists.iter().any(|l| l.archive), "no archive on this computer's account");
    // An Exchange account has its online archive's list, read-only, and a shared mailbox's.
    s.accounts[0].config.ews = Some("https://mail.contoso.example/EWS/Exchange.asmx".into());
    let shared = quatrelle_model::Folder {
        account: "sample".into(),
        path: "Team/Inbox".into(),
        delimiter: Some('/'),
        role: quatrelle_model::FolderRole::Inbox,
        selectable: true,
        mailbox: Some("team@contoso.example".into()),
        ..Default::default()
    };
    s.accounts[0].folders.push(shared);
    let lists = s.task_lists();
    let archive = lists.iter().find(|l| l.archive).expect("the online archive");
    assert!(archive.read_only && archive.id == "sample|archive" && archive.account.as_deref() == Some("sample"));
    assert!(lists.iter().any(|l| l.mailbox.as_deref() == Some("team@contoso.example")), "{lists:?}");
    // Its changes are refused first, as the archive's mail is.
    let refused = s.run("task.create", &json!({"subject": "Archived", "list": archive.id})).unwrap_err();
    assert!(refused.to_string().contains("online archive"), "{refused}");
    // Typed in an account's list: there, and only there.
    let uid = uid_of(&s.run("task.create", &json!({"subject": "Zoe's task", "list": "sample"})).unwrap());
    assert!(s.find_task(&uid).unwrap().account == "sample");
    assert!(s.todo_for("sample", "", false).iter().any(|r| r.task().is_some_and(|t| t.task.uid == uid)));
    assert!(s.todo_for("todo", "", false).iter().any(|r| r.task().is_some_and(|t| t.task.uid == uid)));
    assert!(s.todo_for("sample|archive", "", false).is_empty());
    let listed = s.run("task.lists", &json!({})).unwrap();
    assert!(listed.as_array().is_some_and(|a| a.iter().any(|l| l["archive"] == true && l["readOnly"] == true)), "{listed}");
    done(s);
}

/// The To-Do List: flagged mail beside the tasks, grouped by due day, done with or removed
/// from the list by its key, and across accounts.
#[test]
fn the_todo_list_holds_the_flagged_mail_and_the_tasks() {
    let mut s = sample_session("todo-list");
    let today = quatrelle_model::dates::today(&s.clock);
    let due = |subject: &str| {
        s.cache
            .values()
            .find_map(|st| st.messages.iter().find(|m| m.subject == subject))
            .map(|m| m.key.to_id())
            .unwrap_or_else(|| panic!("no {subject}"))
    };
    let a = due("Character turnarounds");
    let b = due("Sound mix, version 3");
    // Flagged for today, and tomorrow, without being in the list's selection at all.
    s.run("mail.flag", &json!({"key": a, "due": "today"})).unwrap();
    s.run("mail.flag", &json!({"key": b, "due": "tomorrow"})).unwrap();
    let uid = uid_of(&s.run("task.create", &json!({"subject": "Devis Martin", "due": quatrelle_model::dates::day_string(today + 3)})).unwrap());
    // Its follow-up title, when one was set (`PidLidFlagRequest`), is the row's subject.
    {
        let key = MessageKey::from_id(&a).unwrap();
        let st = s.cache.get_mut(&(key.account.clone(), key.folder.clone())).unwrap();
        let m = st.messages.iter_mut().find(|m| m.key == key).unwrap();
        std::sync::Arc::make_mut(m).flags.follow_up.title = "Send the report".into();
    }
    let list = s.todo("", false);
    let mail = |id: &str| list.iter().find(|r| r.uid() == id).cloned();
    let row = mail(&a).expect("the flagged message");
    assert_eq!(row.subject(), "Send the report");
    assert_eq!((row.due(), row.is_complete()), (Some(today), false));
    assert!(row.task().is_none() && row.mail().is_some());
    assert!(mail(&b).is_some());
    assert!(list.iter().any(|r| r.task().is_some_and(|t| t.task.uid == uid)), "the task too");
    // Grouped by due day: today's before tomorrow's, the task (in three days) after both.
    let at = |id: &str| list.iter().position(|r| r.uid() == id).unwrap();
    assert!(at(&a) < at(&b) && at(&b) < at(&uid), "{:?}", list.iter().map(|r| (r.uid(), r.group(today))).collect::<Vec<_>>());

    // Ticked from the To-Do List: done, out of what's to do, still there with the completed.
    s.run("mail.flag", &json!({"key": a, "complete": true})).unwrap();
    assert!(s.todo("", false).iter().all(|r| r.uid() != a), "done: out");
    let row_done = s.todo("", true).into_iter().find(|r| r.uid() == a).expect("shown with the completed");
    assert!(row_done.is_complete() && row_done.group(today) == quatrelle_model::pim::tasks::DueGroup::Completed);
    // Ticked again (not done): back among the flagged, a fresh flag (its old due day gone,
    // as flagging one again after Mark Complete always leaves it).
    s.run("mail.flag", &json!({"key": a, "on": true})).unwrap();
    assert!(s.todo("", false).iter().any(|r| r.uid() == a && !r.is_complete() && r.due().is_none()));

    // Removed from the list (the flag cleared): neither list has it.
    s.run("mail.flag", &json!({"key": a, "on": false})).unwrap();
    assert!(s.todo("", true).iter().all(|r| r.uid() != a));

    // Another account's task is there too.
    let other =
        s.run("account.add", &json!({"email": "zoe@studio.example", "local": true, "name": "Zoe"})).unwrap()["account"].as_str().unwrap().to_string();
    s.run("task.create", &json!({"subject": "Zoe's task", "account": other})).unwrap();
    let list = s.todo("", false);
    assert!(list.iter().any(|r| r.task().is_some_and(|t| t.account == other && t.task.subject == "Zoe's task")), "{list:?}");
    assert!(list.iter().any(|r| r.task().is_some_and(|t| t.account == "sample")), "the first account's too");
    done(s);
}

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
    assert_eq!(s.tasks("MÁRTÍN", false).len(), 1, "accents read away, both ways round");

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

/// A task changed here and deleted in Outlook meanwhile: the server wins, as for appointments
/// (03/11; it was made again): it goes here too, said once, and doesn't come back.
#[test]
fn a_task_deleted_on_the_server_goes_here_too_said_once() {
    use quatrelle_model::pim::{DavSync, StoredTask, Task};
    let mut s = Session::open(Some(temp("task-gone")), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s
        .run("account.add", &json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    let task = |href: &str, subject: &str, dirty: bool| StoredTask {
        href: href.into(),
        task: Task { uid: href.into(), subject: subject.into(), ..Default::default() },
        dirty,
        ..Default::default()
    };
    s.pim_mut(&id).tasks = vec![task("task:T1", "Devis Martin", true), task("task:T2", "Kept", false)];
    let read = |gone: &[&str]| DavSync {
        tasks: Some(vec![task("task:T2", "Kept", false)]),
        gone_tasks: gone.iter().map(|g| (*g).to_string()).collect(),
        ..Default::default()
    };
    s.apply_dav(&id, &read(&["task:T1"]));
    assert!(s.find_task("task:T1").is_none(), "gone here too");
    let said = |s: &Session| s.notices.iter().filter(|n| n.text.starts_with("Devis Martin: deleted on the server meanwhile")).count();
    assert_eq!(said(&s), 1);
    s.apply_dav(&id, &read(&[]));
    assert!(s.find_task("task:T1").is_none() && s.find_task("task:T2").is_some());
    assert_eq!(said(&s), 1, "said once");
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
