// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What R21 left read whole at every full sync, a quarter of an hour apart: a colleague's calendar
//! ticked, a public folder of contacts, the notes and the tasks. Each followed by its changes, as
//! Cached Exchange Mode follows every folder it keeps.

use std::time::Instant;

use quatrelle_model::pim::{DavSync, Pim};

use super::*;

/// The clock of these tests: Wednesday 14 October 2026, 17:46 UTC.
const NOW: i64 = 1_792_000_000;
const ANA: &str = "ana@contoso.example";
/// The sizes asked for: a colleague's calendar, a public address book, the tasks and the notes.
const APPOINTMENTS: usize = 200;
const CONTACTS: usize = 300;
const TASKS: usize = 50;
const NOTES: usize = 50;

fn element(xml: &str) -> El {
    parse(xml).kids.remove(0)
}

/// Ana's `n`th appointment, from four months back to two months on.
fn appointment(n: usize, subject: &str) -> El {
    let day = (NOW - 120 * 86_400 + i64::try_from(n).unwrap() * 180 * 86_400 / 200).div_euclid(86_400);
    let start = day * 86_400 + (8 + i64::try_from(n % 9).unwrap()) * 3600;
    element(&format!(
        "<t:CalendarItem><t:Subject>{subject}</t:Subject><t:Start>{}</t:Start><t:End>{}</t:End>\
         <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>ana-{n}</t:UID><t:CalendarItemType>Single</t:CalendarItemType></t:CalendarItem>",
        crate::ews_pim::utc(start),
        crate::ews_pim::utc(start + 1800)
    ))
}

fn contact(name: &str) -> El {
    element(&format!(
        "<t:Contact><t:DisplayName>{name}</t:DisplayName><t:GivenName>{name}</t:GivenName>\
         <t:EmailAddresses><t:Entry Key=\"EmailAddress1\">{}@contoso.example</t:Entry></t:EmailAddresses></t:Contact>",
        name.replace(' ', ".").to_ascii_lowercase()
    ))
}

fn task(subject: &str) -> El {
    element(&format!(
        "<t:Task><t:ItemClass>IPM.Task</t:ItemClass><t:Subject>{subject}</t:Subject><t:Importance>Normal</t:Importance>\
         <t:ReminderIsSet>false</t:ReminderIsSet><t:PercentComplete>0</t:PercentComplete><t:Status>NotStarted</t:Status></t:Task>"
    ))
}

fn note(text: &str) -> El {
    element(&format!(
        "<t:Message><t:ItemClass>IPM.StickyNote</t:ItemClass><t:Subject>{text}</t:Subject><t:Body BodyType=\"Text\">{text}</t:Body></t:Message>"
    ))
}

/// The owner's server: Exchange 2013 or later (public folders kept in a mailbox of their own, asked
/// for routed there), Ana's calendar shared with the account, each item given costing 15 ms.
fn owner() -> Data {
    let mut d = Data {
        pf_mailbox: Some("PFContent@contoso.example".into()),
        pf_hierarchy: Some("PFHierarchy@contoso.example".into()),
        pim_item_cost: Some(Duration::from_millis(15)),
        ..Default::default()
    };
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let tasks = d.folder("Tasks", None, Some("tasks"), "IPF.Task");
    let notes = d.folder("Notes", None, Some("notes"), "IPF.StickyNote");
    let root = d.folder("IPM_SUBTREE", None, Some("publicfoldersroot"), "IPF.Note");
    let book = d.folder("Company contacts", Some(&root), None, "IPF.Contact");
    for n in 0..APPOINTMENTS {
        d.pim.push((format!("XA{n}"), format!("mailbox:{ANA}"), appointment(n, &format!("Ana {n:03}"))));
    }
    for n in 0..CONTACTS {
        d.pim.push((format!("XP{n}"), book.clone(), contact(&format!("Person {n:03}"))));
    }
    for n in 0..TASKS {
        d.pim.push((format!("XT{n}"), tasks.clone(), task(&format!("Task {n:02}"))));
    }
    for n in 0..NOTES {
        d.pim.push((format!("XN{n}"), notes.clone(), note(&format!("Note {n:02}"))));
    }
    d
}

/// What the window keeps once a sync's result is in, as the engine merges it: in a calendar or an
/// address book read, those the sync says are unchanged (`kept`) and those it gave; the notes and
/// tasks the sync gives in place of those kept; the lists as it gave them.
pub(super) fn kept_after(pim: &Pim, sync: &DavSync) -> Pim {
    let mut events: Vec<_> =
        pim.events.iter().filter(|e| !sync.synced_calendars.contains(&e.calendar) || sync.kept.contains(&e.href)).cloned().collect();
    events.extend(sync.events.iter().cloned());
    let mut calendars = pim.calendars.clone();
    for x in sync.calendars.iter().flatten() {
        match calendars.iter_mut().find(|k| k.id == x.id) {
            Some(k) => *k = x.clone(),
            None => calendars.push(x.clone()),
        }
    }
    let mut cards: Vec<_> = pim.cards.iter().filter(|k| !sync.synced_books.contains(&k.book) || sync.kept.contains(&k.href)).cloned().collect();
    cards.extend(sync.cards.iter().cloned());
    let mut books = pim.books.clone();
    for x in sync.books.iter().flatten() {
        match books.iter_mut().find(|k| k.id == x.id) {
            Some(k) => *k = x.clone(),
            None => books.push(x.clone()),
        }
    }
    let (notes, notes_state) = match &sync.notes {
        Some(n) => (n.clone(), sync.notes_state.clone()),
        None => (pim.notes.clone(), pim.notes_state.clone()),
    };
    let (tasks, tasks_state) = match &sync.tasks {
        Some(t) => (t.clone(), sync.tasks_state.clone()),
        None => (pim.tasks.clone(), pim.tasks_state.clone()),
    };
    Pim { calendars, events, books, cards, notes, tasks, notes_state, tasks_state, shared: pim.shared.clone(), ..Default::default() }
}

/// What's kept of the four, to compare: Ana's appointments, the public contacts, the tasks and the
/// notes, each by its item and what it says, in order.
#[derive(Debug, PartialEq, Eq)]
struct Kept {
    ana: Vec<(String, String, i64, i64)>,
    people: Vec<(String, String)>,
    tasks: Vec<(String, String)>,
    notes: Vec<(String, String)>,
}

fn kept(pim: &Pim) -> Kept {
    let ana = quatrelle_model::pim::shared_calendar_id(ANA);
    let mut k = Kept {
        ana: pim.events.iter().filter(|e| e.calendar == ana).map(|e| (e.href.clone(), e.event.summary.clone(), e.event.start, e.event.end)).collect(),
        people: pim.cards.iter().filter(|c| c.book.starts_with("public:")).map(|c| (c.href.clone(), c.card.display())).collect(),
        tasks: pim.tasks.iter().map(|t| (t.href.clone(), t.task.subject.clone())).collect(),
        notes: pim.notes.iter().map(|n| (n.href.clone(), n.note.text.clone())).collect(),
    };
    k.ana.sort();
    k.people.sort();
    k.tasks.sort();
    k.notes.sort();
    k
}

/// What a sync cost the server: items given, requests, and how long it took.
fn cost(server: &super::super::server::Server, f: impl FnOnce() -> DavSync) -> (DavSync, usize, usize, f32) {
    let (given, calls) = {
        let d = server.data.lock().unwrap();
        (d.pim_given, d.calls.len())
    };
    let began = Instant::now();
    let out = f();
    let took = began.elapsed().as_secs_f32();
    let d = server.data.lock().unwrap();
    (out, d.pim_given - given, d.calls.len() - calls, took)
}

/// One item changed on the server, by its id: `with` makes it again.
fn change(server: &super::super::server::Server, id: &str, with: El) {
    let mut d = server.data.lock().unwrap();
    *d.pim.iter_mut().find(|(i, _, _)| i == id).map(|(_, _, el)| el).unwrap() = with;
}

/// A whole read now, as a first sync makes it.
fn whole(c: &crate::ews::Client, now: i64) -> Kept {
    let fresh = Pim { shared: vec![ANA.into()], ..Default::default() };
    kept(&kept_after(&fresh, &crate::ews_pim::sync(c, &fresh, now).unwrap()))
}

/// One item changed in each, then the next full sync: what it reads, and the four as kept here are
/// those a whole read gives.
#[test]
fn one_change_in_each_is_read_alone() {
    let server = serve(owner());
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let fresh = Pim { shared: vec![ANA.into()], ..Default::default() };
    let (first, given, requests, took) = cost(&server, || crate::ews_pim::sync(&c, &fresh, NOW).unwrap());
    eprintln!("R22 probe: first sync: {given} items given, {requests} requests, {took:.2} s");
    assert!(first.errors.is_empty(), "{:?}", first.errors);
    let mut pim = kept_after(&fresh, &first);
    let k = kept(&pim);
    assert_eq!((k.ana.len(), k.people.len(), k.tasks.len(), k.notes.len()), (APPOINTMENTS, CONTACTS, TASKS, NOTES));
    let steps: [(&str, &str, El); 4] = [
        ("one of Ana's appointments", "XA120", appointment(120, "Ana 120, moved")),
        ("one public contact", "XP150", contact("Person 150 Renamed")),
        ("one task", "XT25", task("Task 25, done soon")),
        ("one note", "XN25", note("Note 25, again")),
    ];
    for (n, (what, id, with)) in steps.into_iter().enumerate() {
        change(&server, id, with);
        let now = NOW + 900 * (i64::try_from(n).unwrap() + 1);
        let (next, given, requests, took) = cost(&server, || crate::ews_pim::sync(&c, &pim, now).unwrap());
        eprintln!("R22 probe: {what} changed, the next full sync: {given} items given, {requests} requests, {took:.2} s");
        assert!(next.errors.is_empty(), "{:?}", next.errors);
        assert_eq!(given, 1, "{what}: the one changed, alone");
        pim = kept_after(&pim, &next);
        assert_eq!(kept(&pim), whole(&c, now), "{what}: as a whole read has it");
    }
}

/// The four read once whole, on a server whose items cost nothing: the server, a connection, and
/// what the window keeps.
fn read_once(d: Data) -> (super::super::server::Server, crate::ews::Client, Pim) {
    let server = serve(Data { pim_item_cost: None, ..d });
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let fresh = Pim { shared: vec![ANA.into()], ..Default::default() };
    let first = crate::ews_pim::sync(&c, &fresh, NOW).unwrap();
    assert!(first.errors.is_empty(), "{:?}", first.errors);
    (server, c, kept_after(&fresh, &first))
}

fn calls_of(server: &super::super::server::Server, op: &str) -> usize {
    server.data.lock().unwrap().calls.iter().filter(|c| c.as_str() == op).count()
}

/// Ana stops sharing her calendar: it's said as before, at each sync, with a few requests and
/// nothing read whole; what's kept of it stays, and once she shares it again it's followed by its
/// changes from where it was.
#[test]
fn a_calendar_no_longer_shared_is_said_without_reading_it_all() {
    let (server, c, pim) = read_once(owner());
    server.data.lock().unwrap().withdrawn.push(ANA.into());
    let ana = quatrelle_model::pim::shared_calendar_id(ANA);
    let mut pim = pim;
    for round in 0..2 {
        let (next, given, _, _) = cost(&server, || crate::ews_pim::sync(&c, &pim, NOW + 60 * (round + 1)).unwrap());
        assert!(next.errors.iter().any(|e| e.starts_with(&format!("{ANA}'s calendar"))), "said: {:?}", next.errors);
        assert_eq!(given, 0, "nothing read");
        assert!(!next.synced_calendars.contains(&ana), "what's kept of it stays");
        let entry = next.calendars.iter().flatten().find(|k| k.id == ana).expect("still listed");
        let kept_state = pim.calendars.iter().find(|k| k.id == ana).map(|k| k.ctag.clone()).unwrap();
        assert_eq!(entry.ctag, kept_state, "its state kept with what's kept of it");
        pim = kept_after(&pim, &next);
        assert_eq!(kept(&pim).ana.len(), APPOINTMENTS);
    }
    server.data.lock().unwrap().withdrawn.clear();
    change(&server, "XA7", appointment(7, "Ana 007, moved"));
    let (next, given, _, _) = cost(&server, || crate::ews_pim::sync(&c, &pim, NOW + 600).unwrap());
    assert!(next.errors.is_empty(), "{:?}", next.errors);
    assert_eq!(given, 1, "by its changes again");
    let pim = kept_after(&pim, &next);
    assert_eq!(kept(&pim), whole(&c, NOW + 600));
}

/// A server that won't give a public folder's changes (Microsoft's reference says
/// `SyncFolderItems` isn't for public folders): read whole, as before, and not asked about again
/// that session.
#[test]
fn a_public_folder_whose_changes_are_refused_is_read_whole() {
    let (server, c, pim) = read_once(Data { public_changes_refused: true, ..owner() });
    assert_eq!(kept(&pim).people.len(), CONTACTS);
    let book = pim.books.iter().find(|b| b.id.starts_with("public:")).unwrap();
    assert!(book.ctag.is_empty(), "no state to keep");
    change(&server, "XP3", contact("Person 003 Renamed"));
    let asked = calls_of(&server, "SyncFolderItems pim");
    let (next, given, _, _) = cost(&server, || crate::ews_pim::sync(&c, &pim, NOW + 60).unwrap());
    assert!(next.errors.is_empty(), "{:?}", next.errors);
    assert_eq!(given, CONTACTS, "read whole, as before");
    // Ana's calendar, the notes, the tasks and the account's own, by their changes; not the public
    // folder.
    assert_eq!(calls_of(&server, "SyncFolderItems pim") - asked, 5, "the public folder not asked again");
    let pim = kept_after(&pim, &next);
    assert_eq!(kept(&pim), whole(&c, NOW + 60));
}

/// Deleted in Outlook: one of Ana's appointments, a public contact, a task and a note go here too;
/// one made there comes, once.
#[test]
fn what_is_deleted_or_made_on_the_server_follows() {
    let (server, c, pim) = read_once(owner());
    {
        let mut d = server.data.lock().unwrap();
        d.pim.retain(|(id, _, _)| !["XA3", "XP4", "XT5", "XN6"].contains(&id.as_str()));
        let tasks = d.folders.iter().find(|f| f.name == "Tasks").map(|f| f.id.clone()).unwrap();
        d.pim.push(("XT99".into(), tasks, task("Task 99")));
        d.pim.push(("XA999".into(), format!("mailbox:{ANA}"), appointment(150, "Ana 999")));
    }
    let (next, given, _, _) = cost(&server, || crate::ews_pim::sync(&c, &pim, NOW + 60).unwrap());
    assert_eq!(given, 2, "the two made, alone");
    let pim = kept_after(&pim, &next);
    let k = kept(&pim);
    assert_eq!(k, whole(&c, NOW + 60));
    assert!(!k.ana.iter().any(|(h, ..)| h == "XA3") && !k.people.iter().any(|(h, _)| h == "public:XP4"));
    assert!(!k.tasks.iter().any(|(h, _)| h == "task:XT5") && !k.notes.iter().any(|(h, _)| h == "note:XN6"));
    assert_eq!(k.tasks.iter().filter(|(h, _)| h == "task:XT99").count(), 1);
    assert_eq!(k.ana.iter().filter(|(h, ..)| h == "XA999").count(), 1);
}

/// A deletion the server refused in Ana's calendar (an editor's), and of a task and a note: each
/// comes back where it was, not in the account's own calendar.
#[test]
fn a_deletion_the_server_refused_comes_back_where_it_was() {
    use quatrelle_model::pim::Tombstone;
    let (server, c, mut pim) = read_once(owner());
    let ana = quatrelle_model::pim::shared_calendar_id(ANA);
    pim.events.retain(|e| e.href != "XA9");
    pim.tasks.retain(|t| t.href != "task:XT9");
    pim.notes.retain(|n| n.href != "note:XN9");
    pim.deleted = vec![
        Tombstone { href: "XA9".into(), etag: String::new(), calendar: ana.clone() },
        Tombstone { href: "task:XT9".into(), ..Default::default() },
        Tombstone { href: "note:XN9".into(), ..Default::default() },
    ];
    server.data.lock().unwrap().undeletable.extend(["XA9".to_string(), "XT9".to_string(), "XN9".to_string()]);
    let next = crate::ews_pim::sync(&c, &pim, NOW + 60).unwrap();
    assert!(next.errors.iter().filter(|e| e.starts_with("not deleted on the server")).count() == 3, "{:?}", next.errors);
    assert!(next.events.iter().any(|e| e.href == "XA9" && e.calendar == ana), "back in Ana's calendar");
    assert!(!next.events.iter().any(|e| e.href == "XA9" && e.calendar == crate::ews_pim::CALENDAR), "not in the account's own");
    let pim = kept_after(&pim, &next);
    let k = kept(&pim);
    assert!(k.tasks.iter().any(|(h, _)| h == "task:XT9") && k.notes.iter().any(|(h, _)| h == "note:XN9"));
}

/// A task and a note changed here go up and are read back by their changes once; one changed here
/// and not sent (the server busy) stays the engine's, not given back as the server has it.
#[test]
fn a_task_and_a_note_changed_here_are_kept_once() {
    let (_server, c, mut pim) = read_once(owner());
    let t = pim.tasks.iter_mut().find(|t| t.href == "task:XT1").unwrap();
    t.task.subject = "Task 01, here".into();
    t.dirty = true;
    let n = pim.notes.iter_mut().find(|n| n.href == "note:XN1").unwrap();
    n.note.text = "Note 01, here".into();
    n.dirty = true;
    let next = crate::ews_pim::sync(&c, &pim, NOW + 60).unwrap();
    assert!(next.errors.is_empty(), "{:?}", next.errors);
    let tasks = next.tasks.clone().unwrap();
    assert_eq!(tasks.iter().filter(|t| t.href == "task:XT1").map(|t| t.task.subject.as_str()).collect::<Vec<_>>(), ["Task 01, here"]);
    let notes = next.notes.clone().unwrap();
    let made = next.pushed_notes.first().map(|n| n.href.clone()).unwrap();
    assert!(!notes.iter().any(|n| n.href == "note:XN1"), "the note's old item gone (made again)");
    assert_eq!(notes.iter().filter(|n| n.href == made).count(), 1);
    assert_eq!(notes.len(), NOTES);
}
