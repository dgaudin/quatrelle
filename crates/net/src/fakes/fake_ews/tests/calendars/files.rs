// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The files of an Exchange appointment (R25), as Outlook's Cached Exchange Mode has them: kept
//! with it when small, listed and fetched when opened when big, and gone here once gone there.

use quatrelle_model::pim::files::{event_file_data, event_file_reference, event_files};
use quatrelle_model::pim::{DavSync, Pim};

use super::*;

/// The clock of these tests: Wednesday 14 October 2026, 17:46 UTC.
const NOW: i64 = 1_792_000_000;

/// The agenda's bytes: a small file, kept.
fn agenda() -> Vec<u8> {
    b"%PDF-1.7 agenda ".iter().copied().cycle().take(3000).collect()
}

/// The deck's bytes: past what's kept of one file, listed only.
fn deck() -> Vec<u8> {
    (0..1_500_000u32).map(|i| (i % 251) as u8).collect()
}

/// The minutes attached as a message.
const MINUTES: &str = "From: Zoe Park <zoe@contoso.example>\r\nSubject: Minutes\r\n\r\nWe agreed.\r\n";

fn file(id: &str, name: &str, kind: &str, data: &[u8]) -> String {
    format!(
        "<t:FileAttachment><t:AttachmentId Id=\"{id}\"/><t:Name>{name}</t:Name><t:ContentType>{kind}</t:ContentType>\
         <t:Size>{}</t:Size><t:IsInline>false</t:IsInline><t:Content>{}</t:Content></t:FileAttachment>",
        data.len(),
        crate::imap::base64(data)
    )
}

/// The budget review with its agenda, its deck and the minutes of the last one (`files`: those of
/// them it has).
fn review(files: &[&str]) -> El {
    let mut list = String::new();
    for f in files {
        list.push_str(&match *f {
            "agenda" => file("AT-1", "Agenda.pdf", "application/pdf", &agenda()),
            "deck" => file("AT-2", "Deck.pptx", "application/vnd.openxmlformats-officedocument.presentationml.presentation", &deck()),
            _ => format!(
                "<t:ItemAttachment><t:AttachmentId Id=\"AT-3\"/><t:Name>Minutes</t:Name><t:Size>4096</t:Size><t:IsInline>false</t:IsInline>\
                 <t:Message><t:MimeContent CharacterSet=\"UTF-8\">{}</t:MimeContent><t:Subject>Minutes</t:Subject></t:Message></t:ItemAttachment>",
                crate::imap::base64(MINUTES.as_bytes())
            ),
        });
    }
    parse(&format!(
        "<t:CalendarItem><t:Subject>Budget review</t:Subject><t:Start>2026-10-20T13:00:00Z</t:Start><t:End>2026-10-20T14:00:00Z</t:End>\
         <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>budget-1</t:UID><t:CalendarItemType>Single</t:CalendarItemType>\
         <t:HasAttachments>{}</t:HasAttachments><t:Attachments>{list}</t:Attachments></t:CalendarItem>",
        !files.is_empty()
    ))
    .kids
    .remove(0)
}

/// An account with the budget review in its calendar (`owner`: in a colleague's instead).
fn server(owner: Option<&str>) -> (super::super::server::Server, crate::ews::Client) {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let cal = d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let folder = owner.map_or(cal, |o| format!("mailbox:{o}"));
    d.pim.push(("R1".into(), folder, review(&["agenda", "deck", "minutes"])));
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    (server, c)
}

/// What the window keeps once a sync's result is in, as the engine merges it.
fn kept_after(pim: &Pim, sync: &DavSync) -> Pim {
    let mut events: Vec<_> =
        pim.events.iter().filter(|e| !sync.synced_calendars.contains(&e.calendar) || sync.kept.contains(&e.href)).cloned().collect();
    events.extend(sync.events.iter().cloned());
    Pim { calendars: sync.calendars.clone().unwrap_or_else(|| pim.calendars.clone()), events, ..Default::default() }
}

fn names(pim: &Pim) -> Vec<String> {
    let e = pim.events.iter().find(|e| e.event.summary == "Budget review").expect("the budget review");
    event_files(&e.event).into_iter().map(|f| f.name).collect()
}

/// An appointment with a small file, a big one and an attached message: each shows; the small
/// file and the message are kept with it, the big one is listed and fetched when it's opened.
#[test]
fn an_appointments_files_come_with_it() {
    let (server, c) = server(None);
    let first = crate::ews_pim::sync_changes(&c, &Pim::default(), NOW).unwrap();
    let pim = kept_after(&Pim::default(), &first);
    assert_eq!(names(&pim), ["Agenda.pdf", "Deck.pptx", "Minutes.eml"]);
    let e = &pim.events.iter().find(|e| e.event.summary == "Budget review").unwrap().event;
    let files = event_files(e);
    assert_eq!(files.iter().map(|f| (f.size, f.on_server)).collect::<Vec<_>>(), [(3000, false), (1_500_000, true), (MINUTES.len(), false)]);
    assert_eq!(files[1].mime, "application/vnd.openxmlformats-officedocument.presentationml.presentation");
    assert_eq!(event_file_data(e, 0), Some(agenda()));
    assert_eq!(event_file_data(e, 2).as_deref(), Some(MINUTES.as_bytes()), "the message as its source");
    assert_eq!(event_file_data(e, 1), None, "the deck isn't here");
    assert_eq!(server.data.lock().unwrap().attachments_given, ["AT-1", "AT-3"], "the small ones, asked together");
    // Opened: fetched from the server.
    let reference = event_file_reference(e, 1).unwrap();
    assert_eq!(crate::ews_pim::attachment(&c, &reference).unwrap(), deck());
    assert_eq!(server.data.lock().unwrap().attachments_given.last().map(String::as_str), Some("AT-2"));
    // Unchanged on the server: nothing asked again.
    let next = crate::ews_pim::sync_changes(&c, &pim, NOW + 60).unwrap();
    assert!(next.events.is_empty() && server.data.lock().unwrap().attachments_given.len() == 3, "{:?}", next.events);
}

/// A colleague's appointment, read-only: its files are listed, none fetched until opened.
#[test]
fn a_colleagues_appointment_lists_its_files() {
    let (server, c) = server(Some("ana@contoso.example"));
    let pim = Pim { shared: vec!["ana@contoso.example".into()], ..Default::default() };
    let sync = crate::ews_pim::sync(&c, &pim, NOW).unwrap();
    let id = quatrelle_model::pim::shared_calendar_id("ana@contoso.example");
    let e = &sync.events.iter().find(|e| e.calendar == id && e.event.summary == "Budget review").unwrap().event;
    let files = event_files(e);
    assert_eq!(
        files.iter().map(|f| (f.name.as_str(), f.on_server)).collect::<Vec<_>>(),
        [("Agenda.pdf", true), ("Deck.pptx", true), ("Minutes.eml", true)]
    );
    assert!(server.data.lock().unwrap().attachments_given.is_empty());
    assert_eq!(crate::ews_pim::attachment(&c, &event_file_reference(e, 2).unwrap()).unwrap(), MINUTES.as_bytes());
}

/// A file taken off the appointment on the server goes from here at the next sync by changes.
#[test]
fn a_file_removed_on_the_server_goes_at_the_next_sync() {
    let (server, c) = server(None);
    let first = crate::ews_pim::sync_changes(&c, &Pim::default(), NOW).unwrap();
    let pim = kept_after(&Pim::default(), &first);
    server.data.lock().unwrap().pim.iter_mut().find(|(id, _, _)| id == "R1").unwrap().2 = review(&["deck", "minutes"]);
    let next = crate::ews_pim::sync_changes(&c, &pim, NOW + 60).unwrap();
    let pim = kept_after(&pim, &next);
    assert_eq!(names(&pim), ["Deck.pptx", "Minutes.eml"]);
}

/// Opened in the app: the account's worker fetches the file from the server and gives its bytes,
/// or says why it can't.
#[test]
fn the_worker_fetches_a_file_left_on_the_server() {
    let (server, _c) = server(None);
    let store = crate::fake::tests_store("ews-appointment-files");
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "riley@contoso.example".into(),
        username: format!("{DOMAIN}\\{USER}"),
        ews: Some(server.url.clone()),
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret(PASSWORD.into())) });
    pool.submit(Job::FetchEventFile { account: "a1".into(), reference: "AT-2".into(), ask: 7 });
    let seen = until(&pool, "the deck", |e| matches!(e, Event::EventFile { ask: 7, .. }));
    let Some(Event::EventFile { data, error, .. }) = seen.last() else { panic!("{seen:?}") };
    assert_eq!((data.len(), error), (deck().len(), &None));
    assert!(data == &deck());
    pool.submit(Job::FetchEventFile { account: "a1".into(), reference: "AT-404".into(), ask: 8 });
    let seen = until(&pool, "the answer", |e| matches!(e, Event::EventFile { ask: 8, .. }));
    let Some(Event::EventFile { data, error, .. }) = seen.last() else { panic!("{seen:?}") };
    assert!(data.is_empty() && error.as_deref().is_some_and(|e| e.contains("not on the server")), "{error:?}");
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

/// A calendar followed by its changes before its files were read (R21, R22): read whole once, its
/// appointments' files with it, then by its changes again.
#[test]
fn a_calendar_kept_before_comes_with_its_files_once() {
    let (server, c) = server(None);
    let first = crate::ews_pim::sync_changes(&c, &Pim::default(), NOW).unwrap();
    let mut pim = kept_after(&Pim::default(), &first);
    // As the version before kept it: the server's state, the appointment without its files.
    for k in &mut pim.calendars {
        k.ctag = k.ctag.strip_prefix("f1:").unwrap_or(&k.ctag).to_string();
    }
    for e in &mut pim.events {
        e.event.extra.retain(|l| !l.starts_with("ATTACH") && !l.starts_with("X-QUATRELLE-SERVER-ATTACH"));
    }
    assert!(names(&pim).is_empty());
    let next = crate::ews_pim::sync_changes(&c, &pim, NOW + 60).unwrap();
    assert!(next.kept.is_empty(), "read whole");
    let pim = kept_after(&pim, &next);
    assert_eq!(names(&pim), ["Agenda.pdf", "Deck.pptx", "Minutes.eml"]);
    let given = server.data.lock().unwrap().pim_given;
    let again = crate::ews_pim::sync_changes(&c, &pim, NOW + 120).unwrap();
    assert!(again.events.is_empty() && server.data.lock().unwrap().pim_given == given, "then by its changes");
}

/// A public calendar's appointment lists its files, none fetched until opened.
#[test]
fn a_public_calendars_appointment_lists_its_files() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let root = d.folder("IPM_SUBTREE", None, Some("publicfoldersroot"), "IPF.Note");
    let events = d.folder("Company events", Some(&root), None, "IPF.Appointment");
    d.pim.push(("P1".into(), events, review(&["agenda", "deck"])));
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let sync = crate::ews_pim::sync(&c, &Pim::default(), NOW).unwrap();
    let e = &sync.events.iter().find(|e| e.calendar.starts_with("public:")).expect("the public appointment").event;
    let files: Vec<(String, bool)> = event_files(e).into_iter().map(|f| (f.name, f.on_server)).collect();
    assert_eq!(files, [("Agenda.pdf".to_string(), true), ("Deck.pptx".to_string(), true)]);
    assert!(server.data.lock().unwrap().attachments_given.is_empty());
}
