// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: Outlook's Notes, their colour, their order and their window's size.

use super::*;

#[test]
fn notes_are_written_coloured_and_deleted() {
    let mut s = sample_session("notes");
    assert_eq!(s.notes("").len(), 3, "the sample's notes");
    let uid = s.run("note.create", &json!({"text": "Call the print shop", "color": 9})).unwrap()["uid"].as_str().unwrap().to_string();
    let n = s.notes("").into_iter().find(|n| n.note.uid == uid).unwrap();
    assert_eq!((n.note.color, n.note.text.as_str()), (4, "Call the print shop"), "a colour past the last is the last");
    s.set_clock(NOW + 60, 0);
    s.run("note.update", &json!({"uid": uid, "text": "Call the print shop\nabout paper", "color": 0})).unwrap();
    let first = &s.notes("")[0];
    assert_eq!((first.note.uid.as_str(), first.note.title(), first.note.color), (uid.as_str(), "Call the print shop", 0), "the latest first");
    assert_eq!(s.notes("paper").len(), 2);
    assert!(s.run("note.update", &json!({"uid": "nope", "text": "x"})).is_err());
    s.run("note.delete", &json!({"uid": uid})).unwrap();
    assert!(s.notes("").iter().all(|n| n.note.uid != uid));
    // Kept with the account's calendar and contacts, across starts.
    let root = s.store.as_ref().unwrap().root().to_path_buf();
    drop(s);
    let s = Session::open(Some(Store::open(&root).unwrap()), quatrelle_model::dates::Clock::new(NOW, 0));
    assert_eq!(s.notes("").len(), 3);
    done(s);

    // An Exchange account: notes go to its Notes folder, and what the server says follows.
    let mut s = Session::open(Some(temp("notes-ews")), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s
        .run("account.add", &json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    let uid = s.run("note.create", &json!({"text": "Sync me"})).unwrap()["uid"].as_str().unwrap().to_string();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::SyncDav { .. })), "asks for a sync");
    assert!(s.pim[&id].notes[0].dirty);
    let stored = quatrelle_model::pim::StoredNote { href: "note:X1".into(), note: s.pim[&id].notes[0].note.clone(), ..Default::default() };
    let server_copy =
        quatrelle_model::pim::StoredNote { note: quatrelle_model::pim::Note { uid: "note:X1".into(), ..stored.note.clone() }, ..stored.clone() };
    let other = quatrelle_model::pim::StoredNote {
        href: "note:X2".into(),
        note: quatrelle_model::pim::Note { uid: "note:X2".into(), text: "From Outlook".into(), color: 2, modified: 1, ..Default::default() },
        ..Default::default()
    };
    let sync = quatrelle_model::pim::DavSync { pushed_notes: vec![stored], notes: Some(vec![server_copy, other]), ..Default::default() };
    s.apply_dav(&id, &sync);
    let texts: Vec<String> = s.notes("").into_iter().map(|n| n.note.text).collect();
    assert_eq!(texts.len(), 2, "{texts:?}: the note sent isn't doubled");
    assert!(texts.contains(&"From Outlook".to_string()) && texts.contains(&"Sync me".to_string()));
    let _ = uid;
    s.run("note.delete", &json!({"uid": "note:X2"})).unwrap();
    assert!(s.pim[&id].deleted.iter().any(|t| t.href == "note:X2"), "deleted on the server next time");
    done(s);
}

/// Issue #1: notes sort A to Z (by their first line), by colour, or the last changed first.
#[test]
fn notes_can_be_arranged() {
    let mut s = sample_session("notes-order");
    for (text, color) in [("banana bread", 3), ("Apple pie\nwith cream", 1), ("cherry jam", 1)] {
        s.run("note.create", &json!({"text": text, "color": color})).unwrap();
    }
    let titles = |s: &Session| {
        s.notes("")
            .iter()
            .filter(|n| ["Apple", "banana", "cherry"].iter().any(|w| n.note.text.starts_with(w)))
            .map(|n| n.note.text.lines().next().unwrap_or("").to_string())
            .collect::<Vec<_>>()
    };
    s.run("note.arrange", &json!({"by": "title"})).unwrap();
    assert_eq!(titles(&s), ["Apple pie", "banana bread", "cherry jam"]);
    s.run("note.arrange", &json!({"by": "color"})).unwrap();
    assert_eq!(titles(&s), ["Apple pie", "cherry jam", "banana bread"]);
    assert!(s.run("note.arrange", &json!({"by": "size"})).is_err());
}

/// A note's window size (1.4, the note's window): kept with it, sent to Exchange as its text
/// is, within its bounds; doubtful params refused and nothing changed.
#[test]
fn a_notes_size_is_kept_and_sent() {
    let mut s = Session::open(Some(temp("notes-size")), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s
        .run("account.add", &json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    let uid = s.run("note.create", &json!({"text": "Sized", "width": 320, "height": 240})).unwrap()["uid"].as_str().unwrap().to_string();
    let note = |s: &Session| s.pim[&id].notes.iter().find(|n| n.note.uid == uid).unwrap().clone();
    assert_eq!(note(&s).note.size(), Some((320, 240)));
    let plain = s.run("note.create", &json!({"text": "Usual"})).unwrap()["uid"].as_str().unwrap().to_string();
    assert!(s.pim[&id].notes.iter().any(|n| n.note.uid == plain && n.note.size().is_none()), "never set: the usual size");
    // Stored on the server: no longer to send.
    let stored = quatrelle_model::pim::StoredNote { href: "note:S1".into(), ..note(&s) };
    s.apply_dav(&id, &quatrelle_model::pim::DavSync { pushed_notes: vec![stored], ..Default::default() });
    assert!(!note(&s).dirty);
    s.take_jobs();
    s.set_clock(NOW + 60, 0);
    s.run("note.update", &json!({"uid": uid, "width": 500, "height": 400})).unwrap();
    let n = note(&s);
    assert_eq!(
        (n.note.size(), n.dirty, n.note.modified, n.note.text.as_str()),
        (Some((500, 400)), true, NOW + 60, "Sized"),
        "to send, the text as it was"
    );
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::SyncDav { .. })), "asks for a sync");
    let listed = s.run("note.list", &json!({})).unwrap();
    assert!(listed["notes"].as_array().unwrap().iter().any(|x| x["uid"] == uid.as_str() && x["width"] == 500 && x["height"] == 400));
    assert_eq!(s.run("note.update", &json!({"uid": uid, "width": 500.0, "height": 400})).unwrap()["changed"], false);
    for bad in [
        json!({"width": 159, "height": 200}),
        json!({"width": 4001, "height": 200}),
        json!({"width": 200, "height": 119}),
        json!({"width": 300}),
        json!({"height": 300}),
        json!({"width": "300", "height": 300}),
        json!({"width": -1, "height": 300}),
        json!({"width": 300.5, "height": 300}),
        json!({"width": null, "height": 300}),
        json!({"width": 70_000, "height": 300}),
        json!({"width": 1e300, "height": 300}),
    ] {
        let mut v = bad.clone();
        v["uid"] = json!(uid);
        assert!(matches!(s.run("note.update", &v), Err(CmdError::Params(_))), "{bad}");
        v["text"] = json!("x");
        assert!(matches!(s.run("note.create", &v), Err(CmdError::Params(_))), "{bad}");
    }
    assert_eq!(note(&s).note.size(), Some((500, 400)), "nothing changed by what was refused");
    assert_eq!(s.pim[&id].notes.len(), 2, "no note made by what was refused");
    // A note kept before had no size: it reads as none set.
    let old: quatrelle_model::pim::Note = serde_json::from_str(r#"{"uid": "a", "text": "b"}"#).unwrap();
    assert_eq!((old.width, old.height, old.color, old.size()), (0, 0, quatrelle_model::pim::NOTE_YELLOW, None));
    done(s);
}

/// A server that didn't keep a note's size: the size goes back to the server's (none), the note
/// is no longer to send, and a notice says why.
#[test]
fn a_notes_size_the_server_refused_goes_back() {
    let mut s = Session::open(Some(temp("notes-size-refused")), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s
        .run("account.add", &json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    let uid = s.run("note.create", &json!({"text": "Wide", "width": 900, "height": 700})).unwrap()["uid"].as_str().unwrap().to_string();
    let note = |s: &Session| s.pim[&id].notes.iter().find(|n| n.note.uid == uid).unwrap().clone();
    let sent = quatrelle_model::pim::Note { width: 0, height: 0, ..note(&s).note };
    let sync = quatrelle_model::pim::DavSync {
        pushed_notes: vec![quatrelle_model::pim::StoredNote { href: "note:S2".into(), note: sent, ..Default::default() }],
        notes_size_refused: vec![uid.clone()],
        errors: vec!["Wide: the server didn't keep its size (ErrorInvalidExtendedProperty)".into()],
        ..Default::default()
    };
    s.apply_dav(&id, &sync);
    let n = note(&s);
    assert_eq!((n.note.size(), n.dirty, n.href.as_str(), n.note.text.as_str()), (None, false, "note:S2", "Wide"));
    assert!(s.notices.iter().any(|x| x.text.contains("didn't keep its size")), "{:?}", s.notices);
    done(s);
}
