// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What a message is, over the fake Graph server: its class and icon come with the listing and
//! the verb, so mail kept from before learns it at the next sync, without its source.

use quatrelle_model::MessageKind;

use super::*;

/// Three messages with their classes; the sync lists them with what they are.
fn mail(d: &mut Data, inbox: &str) {
    for (subject, class, day) in
        [("accepted", "IPM.Schedule.Meeting.Resp.Pos", "01"), ("bounced", "REPORT.IPM.Note.NDR", "02"), ("ordinary", "IPM.Note", "03")]
    {
        let id =
            d.deliver(inbox, format!("From: a@x.example\r\nSubject: {subject}\r\n\r\nx\r\n").as_bytes(), true, &format!("2024-05-{day}T09:00:00Z"));
        if let Some(m) = d.messages.iter_mut().find(|m| m.id == id) {
            m.class = Some(class.into());
        }
    }
}

fn kinds(store: &Store) -> Vec<(String, MessageKind)> {
    let mut v: Vec<_> = store.load_folder("a1", "INBOX").unwrap().messages.iter().map(|m| (m.subject.clone(), m.kind)).collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

/// Mail already kept takes its kind at the next sync, from the listing alone: no source is
/// asked for.
#[test]
fn mail_kept_takes_its_kind_at_the_next_sync_without_its_source() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"));
    mail(&mut d, &inbox);
    let server = serve(d);
    let store = crate::fake::tests_store("graph-kinds-kept");
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let mut st = crate::jmap::State::default();
    crate::graph::list_folders(&c, &mut st, "a1", &[]).unwrap();
    let opts = crate::sync::SyncOptions { prefetch: 0, ..Default::default() };
    crate::graph::sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    // As the version before left it.
    let mut fs = store.load_folder("a1", "INBOX").unwrap();
    for m in fs.messages.iter_mut() {
        m.kind = MessageKind::Unknown;
    }
    store.save_folder("a1", "INBOX", &fs).unwrap();
    let report = crate::graph::sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    let want = vec![
        ("accepted".to_string(), MessageKind::MeetingAccepted),
        ("bounced".to_string(), MessageKind::NotDelivered),
        ("ordinary".to_string(), MessageKind::Unknown),
    ];
    assert_eq!(kinds(&store), want);
    assert_eq!(report.changed, 2);
    assert!(!server.data.lock().unwrap().calls.iter().any(|c| c.contains("$value")), "no source asked for");
    let _ = std::fs::remove_dir_all(store.root());
}

/// A message new to the cache takes its kind with its first listing.
#[test]
fn a_new_message_takes_its_kind() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"));
    mail(&mut d, &inbox);
    let server = serve(d);
    let store = crate::fake::tests_store("graph-kinds-new");
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let mut st = crate::jmap::State::default();
    crate::graph::list_folders(&c, &mut st, "a1", &[]).unwrap();
    let opts = crate::sync::SyncOptions { prefetch: 0, ..Default::default() };
    crate::graph::sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    let want = vec![
        ("accepted".to_string(), MessageKind::MeetingAccepted),
        ("bounced".to_string(), MessageKind::NotDelivered),
        ("ordinary".to_string(), MessageKind::Plain),
    ];
    assert_eq!(kinds(&store), want);
    let _ = std::fs::remove_dir_all(store.root());
}

/// A server that refuses the longer `$expand` still lists the mail, with its arrows; the refusal
/// is asked once for the session, not at each folder.
#[test]
fn a_server_that_refuses_the_longer_expand_still_lists_mail() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"));
    let other = d.folder("Projects", None, None);
    mail(&mut d, &inbox);
    let replied = d.deliver(&other, b"From: a@x.example\r\nSubject: replied\r\n\r\nx\r\n", true, "2024-05-04T09:00:00Z");
    if let Some(m) = d.messages.iter_mut().find(|m| m.id == replied) {
        (m.verb, m.verb_time) = (Some("102".into()), Some("2026-10-30T09:12:00Z".into()));
    }
    d.refuse_kinds = true;
    let server = serve(d);
    let store = crate::fake::tests_store("graph-kinds-refused");
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let mut st = crate::jmap::State::default();
    crate::graph::list_folders(&c, &mut st, "a1", &[]).unwrap();
    let opts = crate::sync::SyncOptions { prefetch: 0, ..Default::default() };
    for folder in ["INBOX", "Projects"] {
        let synced = crate::graph::sync_folder(&c, &mut st, &store, "a1", folder, &opts);
        assert!(synced.is_ok(), "{folder}: {synced:?}");
    }
    let inbox_kept = store.load_folder("a1", "INBOX").unwrap();
    assert_eq!(inbox_kept.messages.len(), 3, "listed all the same");
    assert!(inbox_kept.messages.iter().all(|m| m.kind == MessageKind::Plain || m.kind == MessageKind::Unknown));
    let projects = store.load_folder("a1", "Projects").unwrap();
    assert!(projects.messages.iter().any(|m| m.subject == "replied" && m.flags.answered), "its arrow is read");
    let refused = server.data.lock().unwrap().calls.iter().filter(|c| c.contains("0x001A")).count();
    assert_eq!(refused, 1, "one refused request for the two folders");
    let _ = std::fs::remove_dir_all(store.root());
}
