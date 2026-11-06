// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: the files kept with an appointment or a contact (R23), listed and saved by
//! `event.saveFile` and `contact.saveFile`: a series' changed occurrence shows its own, a program
//! is never handed on to be opened, and a file that isn't there is an error, never a panic.

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
