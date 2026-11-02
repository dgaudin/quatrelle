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
