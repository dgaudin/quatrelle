// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! An appointment changed here and deleted on the server meanwhile: the server wins (the
//! owner's decision, 03/11). Its update was refused as "not on the server any more" at every
//! sync, the change kept here for good; now the appointment is reported gone, once, and nothing
//! is sent for it again.

use super::*;
use quatrelle_model::ical::CalEvent;
use quatrelle_model::pim::{Pim, StoredEvent};

fn calendar_server() -> super::super::server::Server {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    serve(d)
}

fn edited(href: &str, summary: &str, start: i64) -> StoredEvent {
    StoredEvent {
        calendar: crate::ews_pim::CALENDAR.into(),
        href: href.into(),
        etag: "ck".into(),
        event: CalEvent { uid: format!("{href}-uid"), summary: summary.into(), start, end: start + 1800, ..Default::default() },
        dirty: true,
        ..Default::default()
    }
}

/// The owner's "test" (03/11): moved here, deleted in Outlook. Its `UpdateItem` is answered
/// `ErrorItemNotFound`: it's reported gone (no error: the engine says it), not pushed.
#[test]
fn an_appointment_deleted_on_the_server_is_reported_gone() {
    let server = calendar_server();
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let pim = Pim { events: vec![edited("GONE1", "test", 1_792_400_400)], ..Default::default() };
    let sync = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    assert_eq!(sync.gone_events, ["GONE1"], "reported gone");
    assert!(sync.errors.is_empty(), "said by the engine, once: {:?}", sync.errors);
    assert!(sync.pushed_events.is_empty());
    assert!(!sync.events.iter().any(|e| e.href == "GONE1"));
    // As the engine leaves it: nothing to send, nothing said.
    let after = Pim { events: Vec::new(), ..pim };
    let again = crate::ews_pim::sync(&c, &after, 1_792_000_100).unwrap();
    assert!(again.errors.is_empty() && again.gone_events.is_empty() && again.pushed_events.is_empty(), "{again:?}");
}

/// An occurrence changed here whose series was deleted on the server: the series is reported
/// gone (its occurrences go with it), once, however many of them were changed.
#[test]
fn an_occurrence_whose_series_was_deleted_reports_the_series_gone() {
    let server = calendar_server();
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let monday = 1_792_400_400;
    let mut series = edited("SERIES1", "Stand-up", monday);
    series.dirty = false;
    series.event.rrule = Some("FREQ=WEEKLY".into());
    let occurrence = |n: i64| StoredEvent {
        href: String::new(),
        etag: String::new(),
        event: CalEvent {
            recurrence_id: Some(monday + n * 7 * 86_400),
            rrule: None,
            start: monday + n * 7 * 86_400 + 3600,
            end: monday + n * 7 * 86_400 + 5400,
            ..series.event.clone()
        },
        dirty: true,
        ..series.clone()
    };
    let pim = Pim { events: vec![series.clone(), occurrence(1), occurrence(2)], ..Default::default() };
    let sync = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    assert_eq!(sync.gone_events, ["SERIES1"], "the series, once");
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    assert!(sync.pushed_events.is_empty());
}

/// A folder no longer there (a colleague's calendar no longer shared) isn't an appointment
/// deleted: it stays an error, as before.
#[test]
fn a_folder_not_found_is_not_an_appointment_gone() {
    assert!(crate::ews_pim::item_gone_code("ErrorItemNotFound"));
    assert!(!crate::ews_pim::item_gone_code("ErrorFolderNotFound"));
}
