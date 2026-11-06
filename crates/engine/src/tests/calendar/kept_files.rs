// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: the files kept with an appointment or a contact (R23), listed and saved by
//! `event.saveFile` and `contact.saveFile`: a series' changed occurrence shows its own, a program
//! is never handed on to be opened, and a file that isn't there is an error, never a panic. An
//! Exchange appointment's file left on the server (R25) is asked of it, then written and opened.

use quatrelle_model::ical::CalEvent;
use quatrelle_model::vcard::{Card, base64_encode};

use super::*;

fn attach(name: &str, data: &[u8]) -> String {
    format!("ATTACH;FMTTYPE=application/octet-stream;ENCODING=BASE64;VALUE=BINARY;X-FILENAME=\"{name}\":{}", base64_encode(data))
}

#[test]
fn an_appointments_and_a_contacts_files_are_saved_where_asked() {
    let mut s = sample_session("kept-files");
    let dir = std::env::temp_dir().join(format!("quatrelle-kept-files-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let start = s.clock.now + 86_400;
    // A weekly series with a file, one occurrence changed with a file of its own.
    let series = CalEvent {
        uid: "kept-1".into(),
        summary: "Review".into(),
        start,
        end: start + 3600,
        rrule: Some("FREQ=WEEKLY".into()),
        extra: vec![attach("agenda.pdf", b"%PDF-series"), attach("setup.exe", b"MZ")],
        ..Default::default()
    };
    s.create_event(None, None, series).unwrap();
    let changed = start + 7 * 86_400;
    let own = CalEvent {
        uid: "kept-1".into(),
        summary: "Review".into(),
        start: changed + 3600,
        end: changed + 7200,
        recurrence_id: Some(changed),
        extra: vec![attach("proofs.pdf", b"%PDF-own")],
        ..Default::default()
    };
    // Beside its series, as an import or a sync keeps it (`create_event` puts a series whole).
    let (account, stored) = s.find_event("kept-1").map(|(a, e)| (a.to_string(), e.clone())).unwrap();
    s.pim.get_mut(&account).unwrap().events.push(quatrelle_model::pim::StoredEvent { event: own, ..stored });
    let names = |files: Vec<quatrelle_model::pim::files::KeptFile>| files.into_iter().map(|f| f.name).collect::<Vec<_>>();
    assert_eq!(names(s.event_files("kept-1", Some(start))), ["agenda.pdf", "setup.exe"]);
    assert_eq!(names(s.event_files("kept-1", Some(changed))), ["proofs.pdf"], "the changed occurrence's own");
    assert_eq!(names(s.event_files("kept-1", None)), ["agenda.pdf", "setup.exe"]);
    // Saved where asked, as written.
    let out = dir.join("a.pdf");
    s.run("event.saveFile", &json!({"uid": "kept-1", "occurrence": start, "index": 0, "path": out.to_string_lossy()})).unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), b"%PDF-series");
    s.run("event.saveFile", &json!({"uid": "kept-1", "occurrence": changed, "index": 0, "path": out.to_string_lossy(), "open": true})).unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), b"%PDF-own");
    // A program is saved, never handed on to be opened (whatever the path it's written to).
    let exe = dir.join("setup.exe");
    let refused = s.run("event.saveFile", &json!({"uid": "kept-1", "index": 1, "path": dir.join("setup.txt").to_string_lossy(), "open": true}));
    assert!(matches!(refused, Err(CmdError::Disabled(_))), "{refused:?}");
    s.run("event.saveFile", &json!({"uid": "kept-1", "index": 1, "path": exe.to_string_lossy()})).unwrap();
    assert_eq!(std::fs::read(&exe).unwrap(), b"MZ");
    // No such file, no such event: errors.
    for bad in [
        json!({"uid": "kept-1", "index": 2, "path": out.to_string_lossy()}),
        json!({"uid": "nobody", "index": 0, "path": "x"}),
        json!({"uid": "kept-1"}),
    ] {
        assert!(s.run("event.saveFile", &bad).is_err(), "{bad}");
    }
    // A contact's.
    let card = Card {
        uid: "kept-card".into(),
        name: "Ana Lima".into(),
        extra: vec![format!("X-QUATRELLE-ATTACH;X-FILENAME=\"cv.pdf\":data:application/pdf;base64,{}", base64_encode(b"%PDF-cv"))],
        ..Default::default()
    };
    s.create_card(None, card).unwrap();
    assert_eq!(names(s.card_files("kept-card")), ["cv.pdf"]);
    s.run("contact.saveFile", &json!({"uid": "kept-card", "index": 0, "path": out.to_string_lossy()})).unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), b"%PDF-cv");
    assert!(s.run("contact.saveFile", &json!({"uid": "kept-card", "index": 1, "path": out.to_string_lossy()})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
    done(s);
}

/// An Exchange appointment's file left on the server: Open asks the server for it, and once it
/// comes it's written and handed to the interface to open; Save As writes it where asked; asked
/// again, it's here. An imported appointment's line names no server: an error.
#[test]
fn a_file_on_the_server_is_fetched_when_opened() {
    use quatrelle_model::pim::files::server_file_line;
    use quatrelle_model::pim::{LOCAL, StoredEvent};
    let mut s = sample_session("server-files");
    let dir = std::env::temp_dir().join(format!("quatrelle-server-files-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let account = s.accounts[0].config.id.clone();
    let start = s.clock.now + 86_400;
    let deck = server_file_line("Deck.pptx", "application/vnd.ms-powerpoint", 1_500_000, "AT-2").unwrap();
    let tool = server_file_line("tool.exe", "", 10, "AT-9").unwrap();
    let event = |uid: &str, calendar: &str, href: &str| StoredEvent {
        calendar: calendar.into(),
        href: href.into(),
        event: CalEvent {
            uid: uid.into(),
            summary: "Budget review".into(),
            start,
            end: start + 3600,
            extra: vec![deck.clone(), tool.clone()],
            ..Default::default()
        },
        ..Default::default()
    };
    s.pim_mut(&account).events.push(event("ews-1", "ews:calendar", "R1"));
    s.pim_mut(&account).events.push(event("pst-1", LOCAL, ""));
    let files = s.event_files("ews-1", None);
    assert_eq!(
        files.iter().map(|f| (f.name.as_str(), f.size, f.on_server)).collect::<Vec<_>>(),
        [("Deck.pptx", 1_500_000, true), ("tool.exe", 10, true)]
    );
    s.take_jobs();
    // Opened: asked of the server, once however often it's clicked.
    let open = dir.join("Deck.pptx");
    let p = json!({"uid": "ews-1", "index": 0, "path": open.to_string_lossy(), "open": true});
    assert_eq!(s.run("event.saveFile", &p).unwrap()["fetching"], "Deck.pptx");
    s.run("event.saveFile", &p).unwrap();
    let jobs = s.take_jobs();
    let asks: Vec<u64> = jobs
        .iter()
        .filter_map(|j| match j {
            quatrelle_model::Job::FetchEventFile { account: a, reference, ask } if *a == account && reference == "AT-2" => Some(*ask),
            _ => None,
        })
        .collect();
    assert_eq!(asks.len(), 1, "{jobs:?}");
    assert!(!open.exists() && s.take_files_to_open().is_empty(), "nothing yet");
    assert!(s.event_file("ews-1", None, 0).is_err(), "not here yet");
    s.apply(quatrelle_model::Event::EventFile { ask: asks[0], data: b"PK deck".to_vec(), error: None });
    assert_eq!(std::fs::read(&open).unwrap(), b"PK deck");
    assert_eq!(s.take_files_to_open(), [open.to_string_lossy().into_owned()]);
    // Here now: saved at once, nothing asked.
    let saved = dir.join("copy.pptx");
    let r = s.run("event.saveFile", &json!({"uid": "ews-1", "index": 0, "path": saved.to_string_lossy()})).unwrap();
    assert!(r.get("fetching").is_none() && std::fs::read(&saved).unwrap() == b"PK deck");
    assert!(s.take_jobs().is_empty());
    // Refused by the server: said, nothing written.
    let other = dir.join("tool.exe");
    assert!(
        matches!(
            s.run("event.saveFile", &json!({"uid": "ews-1", "index": 1, "path": dir.join("t.txt").to_string_lossy(), "open": true})),
            Err(CmdError::Disabled(_))
        ),
        "a program is never opened"
    );
    s.run("event.saveFile", &json!({"uid": "ews-1", "index": 1, "path": other.to_string_lossy()})).unwrap();
    let ask = match s.take_jobs().as_slice() {
        [quatrelle_model::Job::FetchEventFile { ask, .. }] => *ask,
        other => panic!("{other:?}"),
    };
    s.apply(quatrelle_model::Event::EventFile { ask, data: Vec::new(), error: Some("not on the server any more".into()) });
    assert!(!other.exists());
    assert!(s.notices.iter().any(|n| n.text == "tool.exe could not be downloaded: not on the server any more"), "{:?}", s.notices);
    // An answer nobody waits for is let go.
    s.apply(quatrelle_model::Event::EventFile { ask: 999, data: b"x".to_vec(), error: None });
    // An imported appointment's line names no server.
    assert!(s.run("event.saveFile", &json!({"uid": "pst-1", "index": 0, "path": open.to_string_lossy()})).is_err());
    assert!(s.take_jobs().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
    done(s);
}
