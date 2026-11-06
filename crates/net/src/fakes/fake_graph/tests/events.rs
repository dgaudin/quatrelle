// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Appointments through the fake Graph server: what they carry besides their time and subject.

use quatrelle_model::ical::{CalEvent, PrivateKind, ShowAs};
use quatrelle_model::pim::{LOCAL, Pim, StoredEvent};
use serde_json::json;

use super::*;

fn stored(sync: &quatrelle_model::pim::DavSync, summary: &str) -> StoredEvent {
    sync.events.iter().find(|e| e.event.summary == summary).cloned().unwrap()
}

/// Privacy and Show As go up with the event (every create and every update) and come back.
#[test]
fn privacy_and_show_as_round_trip_through_graph() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"));
    d.calendars.push(("CAL1".into(), "Calendar".into()));
    let server = serve(d);
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let mut pim = Pim::default();
    let new = |uid: &str, summary: &str, private: bool, show_as: ShowAs| StoredEvent {
        calendar: LOCAL.into(),
        event: CalEvent {
            uid: uid.into(),
            summary: summary.into(),
            start: 1_792_000_000,
            end: 1_792_003_600,
            private,
            show_as,
            ..Default::default()
        },
        dirty: true,
        ..Default::default()
    };
    pim.events.push(new("p-1", "Secret", true, ShowAs::OutOfOffice));
    pim.events.push(new("p-2", "Open", false, ShowAs::Busy));
    let sync = crate::graph_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    {
        let d = server.data.lock().unwrap();
        let secret = d.events.iter().find(|e| e["subject"] == "Secret").unwrap();
        assert_eq!((&secret["sensitivity"], &secret["showAs"]), (&json!("private"), &json!("oof")));
        let open = d.events.iter().find(|e| e["subject"] == "Open").unwrap();
        assert_eq!((&open["sensitivity"], &open["showAs"]), (&json!("normal"), &json!("busy")));
    }
    let back = stored(&sync, "Secret");
    assert!(back.event.private);
    assert_eq!((back.event.private_kind, back.event.show_as), (PrivateKind::Private, ShowAs::OutOfOffice));
    let open = stored(&sync, "Open");
    assert!(!open.event.private);
    assert_eq!(open.event.show_as, ShowAs::Busy);

    // Changed here, then sent again as an update.
    let mut changed = back;
    changed.event.private_kind = PrivateKind::Confidential;
    changed.event.show_as = ShowAs::Free;
    changed.dirty = true;
    let mut pim = Pim::default();
    pim.events.push(changed);
    let sync = crate::graph_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    {
        let d = server.data.lock().unwrap();
        let secret = d.events.iter().find(|e| e["subject"] == "Secret").unwrap();
        assert_eq!((&secret["sensitivity"], &secret["showAs"]), (&json!("confidential"), &json!("free")));
    }
    let back = stored(&sync, "Secret");
    assert!(back.event.private);
    assert_eq!((back.event.private_kind, back.event.show_as), (PrivateKind::Confidential, ShowAs::Free));
}

/// Graph's `unknown` Show As, a missing sensitivity or one we don't know never fail a sync.
#[test]
fn unknown_show_as_and_no_sensitivity_read_as_busy_and_public() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"));
    d.calendars.push(("CAL1".into(), "Calendar".into()));
    let ev = |id: &str, subject: &str, extra: serde_json::Value| {
        let mut e = json!({
            "id": id, "iCalUId": id, "calendar": "CAL1", "subject": subject,
            "start": {"dateTime": "2026-10-10T10:00:00.0000000", "timeZone": "UTC"},
            "end": {"dateTime": "2026-10-10T11:00:00.0000000", "timeZone": "UTC"},
        });
        for (k, v) in extra.as_object().cloned().unwrap_or_default() {
            e[k.as_str()] = v;
        }
        e
    };
    d.events.push(ev("E1", "Plain", json!({"showAs": "unknown"})));
    d.events.push(ev("E2", "Personal", json!({"sensitivity": "personal", "showAs": "workingElsewhere"})));
    d.events.push(ev("E3", "Odd", json!({"sensitivity": "bizarre", "showAs": 7})));
    let server = serve(d);
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let sync = crate::graph_pim::sync(&c, &Pim::default(), 1_792_000_000).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    let plain = stored(&sync, "Plain");
    assert!(!plain.event.private);
    assert_eq!(plain.event.show_as, ShowAs::Busy);
    let personal = stored(&sync, "Personal");
    assert!(personal.event.private);
    assert_eq!((personal.event.private_kind, personal.event.show_as), (PrivateKind::Personal, ShowAs::WorkingElsewhere));
    let odd = stored(&sync, "Odd");
    assert!(!odd.event.private);
    assert_eq!(odd.event.show_as, ShowAs::Busy);
}

/// An appointment changed here and deleted on the server meanwhile: its `PATCH` is answered 404
/// `ErrorItemNotFound`, and it's reported gone (the server wins), not refused at every sync.
#[test]
fn an_appointment_deleted_on_the_server_is_reported_gone() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"));
    d.calendars.push(("CAL1".into(), "Calendar".into()));
    let server = serve(d);
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let edited = StoredEvent {
        calendar: "CAL1".into(),
        href: "GONE1".into(),
        etag: "ck".into(),
        event: CalEvent { uid: "gone-1".into(), summary: "test".into(), start: 1_792_000_000, end: 1_792_001_800, ..Default::default() },
        dirty: true,
        ..Default::default()
    };
    let pim = Pim { events: vec![edited], ..Default::default() };
    let sync = crate::graph_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    assert_eq!(sync.gone_events, ["GONE1"]);
    assert!(sync.errors.is_empty(), "said by the engine, once: {:?}", sync.errors);
    assert!(sync.pushed_events.is_empty());
}

/// An appointment changed by a version that didn't read Show As and Private from Exchange Online
/// (before 1.1), and not yet sent: it holds Busy and public whatever the server has, which the
/// update must not write over the server's (a private appointment made public, a free one busy).
/// What the user chose since goes as before.
#[test]
fn an_appointment_changed_before_show_as_was_read_keeps_the_servers() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"));
    d.calendars.push(("CAL1".into(), "Calendar".into()));
    d.events.push(
        json!({"id": "E1", "calendar": "CAL1", "iCalUId": "e1", "subject": "Dentist", "start": {"dateTime": "2026-10-20T09:00:00", "timeZone": "UTC"},
        "end": {"dateTime": "2026-10-20T09:30:00", "timeZone": "UTC"}, "sensitivity": "private", "showAs": "free"}),
    );
    let server = serve(d);
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    // As the older version kept it: renamed here, its Show As and Private the defaults.
    let old = StoredEvent {
        calendar: "CAL1".into(),
        href: "E1".into(),
        etag: "ck".into(),
        event: CalEvent { uid: "e1".into(), summary: "Dentist, Tuesday".into(), start: 1_792_486_800, end: 1_792_488_600, ..Default::default() },
        dirty: true,
        ..Default::default()
    };
    let pim = Pim { events: vec![old.clone()], ..Default::default() };
    let sync = crate::graph_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    let server_has = || {
        let d = server.data.lock().unwrap();
        let e = d.events.iter().find(|e| e["id"] == "E1").unwrap().clone();
        (e["subject"].clone(), e["sensitivity"].clone(), e["showAs"].clone())
    };
    assert_eq!(server_has(), (json!("Dentist, Tuesday"), json!("private"), json!("free")), "the change went, the server's Show As and Private kept");
    // One chosen since (out of the office) goes; kept by this version, public and busy go too.
    let chosen = StoredEvent { event: CalEvent { show_as: ShowAs::OutOfOffice, ..old.event.clone() }, ..old.clone() };
    crate::graph_pim::sync(&c, &Pim { events: vec![chosen], ..Default::default() }, 1_792_000_100).unwrap();
    assert_eq!(server_has(), (json!("Dentist, Tuesday"), json!("private"), json!("oof")));
    let now = Pim { events: vec![old], version: quatrelle_model::pim::PIM_VERSION, ..Default::default() };
    crate::graph_pim::sync(&c, &now, 1_792_000_200).unwrap();
    assert_eq!(server_has(), (json!("Dentist, Tuesday"), json!("normal"), json!("busy")), "public and busy, as chosen");
}

/// An event's files (R25): a small one and the attached minutes kept with it, a big one listed and
/// fetched when it's opened; read again only when the event changed, and one taken off there goes.
#[test]
fn an_events_files_come_with_it() {
    use quatrelle_model::pim::files::{event_file_data, event_file_reference, event_files};
    let agenda: Vec<u8> = b"%PDF-1.7 agenda ".iter().copied().cycle().take(3000).collect();
    let deck: Vec<u8> = (0..1_500_000u32).map(|i| (i % 251) as u8).collect();
    let minutes = "From: Zoe Park <zoe@contoso.example>\r\nSubject: Minutes\r\n\r\nWe agreed.\r\n";
    let file = |id: &str, name: &str, kind: &str, data: &[u8]| json!({"@odata.type": "#microsoft.graph.fileAttachment", "id": id, "name": name, "contentType": kind, "size": data.len(), "isInline": false, "contentBytes": crate::imap::base64(data)});
    let all = json!([
        file("GA1", "Agenda.pdf", "application/pdf", &agenda),
        file("GA2", "Deck.pptx", "application/vnd.ms-powerpoint", &deck),
        {"@odata.type": "#microsoft.graph.itemAttachment", "id": "GA3", "name": "Minutes", "size": 900, "isInline": false, "contentBytes": crate::imap::base64(minutes.as_bytes())},
        {"@odata.type": "#microsoft.graph.fileAttachment", "id": "GA4", "name": "logo.png", "contentType": "image/png", "size": 10, "isInline": true},
        {"@odata.type": "#microsoft.graph.referenceAttachment", "id": "GA5", "name": "On the share"},
    ]);
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"));
    d.calendars.push(("CAL1".into(), "Calendar".into()));
    d.events.push(json!({"id": "E1", "calendar": "CAL1", "iCalUId": "e1", "subject": "Budget review", "changeKey": "ck1", "hasAttachments": true,
        "start": {"dateTime": "2026-10-20T13:00:00", "timeZone": "UTC"}, "end": {"dateTime": "2026-10-20T14:00:00", "timeZone": "UTC"}, "attachments": all}));
    let server = serve(d);
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let asked = || server.data.lock().unwrap().calls.iter().filter(|c| c.contains("/attachments")).count();
    let sync = crate::graph_pim::sync(&c, &Pim::default(), 1_792_000_000).unwrap();
    let e = stored(&sync, "Budget review");
    let files = event_files(&e.event);
    let shown: Vec<(&str, bool)> = files.iter().map(|f| (f.name.as_str(), f.on_server)).collect();
    assert_eq!(shown, [("Agenda.pdf", false), ("Deck.pptx", true), ("Minutes.eml", false)], "no picture of the text, no link");
    assert_eq!(event_file_data(&e.event, 0), Some(agenda));
    assert_eq!(event_file_data(&e.event, 2).as_deref(), Some(minutes.as_bytes()));
    assert_eq!(files[1].size, deck.len());
    assert_eq!(crate::graph_pim::attachment(&c, &event_file_reference(&e.event, 1).unwrap()).unwrap(), deck, "fetched when opened");
    assert!(crate::graph_pim::attachment(&c, "/me/messages/M1/attachments/A1").is_err(), "only an event's");
    // Unchanged: kept as it was, nothing asked.
    let pim = Pim { events: vec![e], ..Default::default() };
    let before = asked();
    let again = crate::graph_pim::sync(&c, &pim, 1_792_000_060).unwrap();
    assert_eq!(asked(), before);
    assert_eq!(event_files(&stored(&again, "Budget review").event).len(), 3);
    // The agenda taken off on the server: gone from here.
    {
        let mut d = server.data.lock().unwrap();
        let ev = d.events.iter_mut().find(|e| e["id"] == "E1").unwrap();
        ev["attachments"].as_array_mut().unwrap().remove(0);
        ev["changeKey"] = json!("ck2");
    }
    let pim = Pim { events: vec![stored(&again, "Budget review")], ..Default::default() };
    let next = crate::graph_pim::sync(&c, &pim, 1_792_000_120).unwrap();
    let names: Vec<String> = event_files(&stored(&next, "Budget review").event).into_iter().map(|f| f.name).collect();
    assert_eq!(names, ["Deck.pptx", "Minutes.eml"]);
}
