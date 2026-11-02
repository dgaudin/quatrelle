//! What goes to the server from here goes once: a change made while a sync is on its way.

use super::*;

/// A new appointment moved while it went up (made at 16:32:07 on 02/11, dragged from Monday to
/// Tuesday at 16:32:10): the next sync began before the window had taken in the first one's
/// result, from the calendars as saved, where the appointment was still new, and made it a
/// second time. Outlook showed it on Monday and on Tuesday. The appointment made is moved.
#[test]
fn an_appointment_moved_while_it_went_up_is_made_once() {
    use quatrelle_model::ical::CalEvent;
    use quatrelle_model::pim::{PIM_DOC, Pim, StoredEvent};
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let cal = d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let server = serve(d);
    let store = crate::fake::tests_store("ews-made-once");
    // Monday 19 October 2026, 09:00 UTC, and the next day.
    let (monday, tuesday) = (1_792_400_400, 1_792_400_400 + 86_400);
    let new = |start: i64| Pim {
        events: vec![StoredEvent {
            calendar: crate::ews_pim::CALENDAR.into(),
            event: CalEvent {
                uid: "dentist-1@quatrelle".into(),
                summary: "Dentist".into(),
                start,
                end: start + 1800,
                private: true,
                ..Default::default()
            },
            dirty: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    store.save_account_doc("a1", PIM_DOC, &new(monday)).unwrap();
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "riley@contoso.example".into(),
        username: format!("{DOMAIN}\\{USER}"),
        ews: Some(server.url.clone()),
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret(PASSWORD.into())) });
    let sent = |e: &Event| matches!(e, Event::DavSynced { sync, .. } if !sync.pushed_events.is_empty());
    // The first sync makes it. No window here takes the result in.
    until(&pool, "made", sent);
    // Moved meanwhile: still new in the calendars as saved.
    store.save_account_doc("a1", PIM_DOC, &new(tuesday)).unwrap();
    pool.submit(Job::SyncDav { account: "a1".into() });
    until(&pool, "sent again", sent);
    let starts: Vec<String> = server
        .data
        .lock()
        .unwrap()
        .pim
        .iter()
        .filter(|(_, f, _)| *f == cal)
        .map(|(_, _, el)| el.text_of("Start").unwrap_or_default().to_string())
        .collect();
    assert_eq!(starts, [crate::ews_pim::utc(tuesday)], "one appointment, on Tuesday");
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

/// The same race for contacts and notes, which go to Exchange as new items at each change (the
/// old one deleted, the new one made), and for a new task: a new contact, a contact changed, a
/// new note and a new task, sent by one sync, then by the next before the window took the first
/// one's result in. Each is on the server once.
#[test]
fn contacts_notes_and_tasks_changed_while_they_went_up_are_made_once() {
    use quatrelle_model::pim::{Note, PIM_DOC, Pim, StoredCard, StoredNote, StoredTask, Task};
    use quatrelle_model::vcard::Card;
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    let contacts = d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let notes = d.folder("Notes", None, Some("notes"), "IPF.StickyNote");
    let tasks = d.folder("Tasks", None, Some("tasks"), "IPF.Task");
    let ana = parse("<t:Contact><t:DisplayName>Ana Lima</t:DisplayName><t:Surname>Lima</t:Surname></t:Contact>");
    d.pim.push(("C1".into(), contacts.clone(), ana.kids[0].clone()));
    let server = serve(d);
    let store = crate::fake::tests_store("ews-cards-made-once");
    let pim = |n: u8| Pim {
        cards: vec![
            StoredCard {
                book: crate::ews_pim::CONTACTS.into(),
                href: "contact:C1".into(),
                card: Card { uid: "C1".into(), name: format!("Ana Lima {n}"), family: "Lima".into(), ..Default::default() },
                dirty: true,
                ..Default::default()
            },
            StoredCard {
                book: crate::ews_pim::CONTACTS.into(),
                card: Card { uid: "bo-1@quatrelle".into(), name: format!("Bo Chen {n}"), family: "Chen".into(), ..Default::default() },
                dirty: true,
                ..Default::default()
            },
        ],
        notes: vec![StoredNote {
            note: Note { uid: "n-local".into(), text: format!("Buy paper {n}"), color: 1, modified: 5 },
            dirty: true,
            ..Default::default()
        }],
        tasks: vec![StoredTask {
            task: Task { uid: "t-local".into(), subject: format!("Order paper {n}"), ..Default::default() },
            dirty: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    store.save_account_doc("a1", PIM_DOC, &pim(1)).unwrap();
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "riley@contoso.example".into(),
        username: format!("{DOMAIN}\\{USER}"),
        ews: Some(server.url.clone()),
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret(PASSWORD.into())) });
    let sent = |e: &Event| matches!(e, Event::DavSynced { sync, .. } if !sync.pushed_cards.is_empty());
    // The first sync sends them. No window here takes the result in.
    until(&pool, "sent", sent);
    // Changed again meanwhile: as saved, still the old contact, the new ones still new.
    store.save_account_doc("a1", PIM_DOC, &pim(2)).unwrap();
    pool.submit(Job::SyncDav { account: "a1".into() });
    until(&pool, "sent again", sent);
    let in_folder = |folder: &str, field: &str| -> Vec<String> {
        let mut v: Vec<String> = server
            .data
            .lock()
            .unwrap()
            .pim
            .iter()
            .filter(|(_, f, _)| f == folder)
            .map(|(_, _, el)| el.text_of(field).unwrap_or_default().to_string())
            .collect();
        v.sort();
        v
    };
    assert_eq!(
        (in_folder(&contacts, "DisplayName"), in_folder(&notes, "Subject"), in_folder(&tasks, "Subject")),
        (vec!["Ana Lima 2".to_string(), "Bo Chen 2".into()], vec!["Buy paper 2".to_string()], vec!["Order paper 2".to_string()]),
        "each once, as changed last"
    );
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}
