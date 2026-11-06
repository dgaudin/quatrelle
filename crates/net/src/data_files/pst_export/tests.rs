// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
use super::*;
use quatrelle_model::{EmailAddress, Flags};
use quatrelle_store::FolderState;
use std::path::Path;

#[test]
fn an_account_round_trips_through_a_data_file() {
    let dir = std::env::temp_dir().join(format!("quatrelle-pst-export-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = Store::open(dir.join("cache")).unwrap();
    let mk = |folder: &str, uid: u32, subject: &str, with_file: bool| {
        let d = mime::Draft {
            to: vec![EmailAddress::new("Ana", "ana@example.com")],
            cc: vec![EmailAddress::new("", "ben@example.com")],
            subject: subject.into(),
            body: "Hello there.".into(),
            html: Some("<p>Hello <b>there</b>.</p>".into()),
            attachments: if with_file {
                vec![mime::OutAttachment { name: "notes.txt".into(), mime: "text/plain".into(), data: b"remember".to_vec(), ..Default::default() }]
            } else {
                Vec::new()
            },
            ..Default::default()
        };
        let raw = mime::build(&d, &EmailAddress::new("Riley", "riley@example.com"), 1_704_067_200, &format!("m{uid}@example.com")).unwrap();
        let key = MessageKey { account: "a1".into(), folder: folder.into(), validity: 1, uid };
        store.write_body(&key, &raw).unwrap();
        mime::summary_from_full(&raw, key, Flags::default(), 10, 0)
    };
    let folder = |path: &str, role| quatrelle_model::Folder {
        account: "a1".into(),
        path: path.into(),
        delimiter: Some('/'),
        role,
        selectable: true,
        ..Default::default()
    };
    let inbox = FolderState {
        validity: 1,
        uid_next: 3,
        messages: vec![mk("INBOX", 1, "Plan — café", true), mk("INBOX", 2, "Second", false)],
        ..Default::default()
    };
    let mut projects = FolderState { validity: 1, uid_next: 2, messages: vec![mk("INBOX/Projects", 1, "Nested", false)], ..Default::default() };
    // Known by its header only: skipped.
    let mut header_only = projects.messages[0].clone();
    header_only.key.uid = 7;
    projects.messages.push(header_only);
    store.save_folder("a1", "INBOX", &inbox).unwrap();
    store.save_folder("a1", "INBOX/Projects", &projects).unwrap();
    store.save_folders("a1", &[folder("INBOX", FolderRole::Inbox), folder("INBOX/Projects", FolderRole::Other)]).unwrap();

    let mut pim = quatrelle_model::pim::Pim::default();
    pim.events.push(quatrelle_model::pim::StoredEvent {
        event: quatrelle_model::ical::CalEvent {
            uid: "e1".into(),
            summary: "Screening".into(),
            start: 1_792_000_000,
            end: 1_792_003_600,
            ..Default::default()
        },
        ..Default::default()
    });
    pim.cards.push(quatrelle_model::pim::StoredCard {
        card: quatrelle_model::vcard::Card { uid: "c1".into(), name: "Ana Lima".into(), ..Default::default() },
        ..Default::default()
    });
    store.save_account_doc("a1", quatrelle_model::pim::PIM_DOC, &pim).unwrap();
    let file = dir.join("Out.pst");
    let mut ticks = 0;
    let done = export(&store, "a1", "Riley's mail", &file, &mut |_| ticks += 1).unwrap();
    assert_eq!(done, Exported { messages: 3, skipped: 1, already: 0, events: 1, contacts: 1 });
    let ics = std::fs::read_to_string(dir.join("Out Calendar.ics")).unwrap();
    assert!(ics.contains("SUMMARY:Screening") && ics.contains("BEGIN:VCALENDAR"), "{ics}");
    let vcf = std::fs::read_to_string(dir.join("Out Contacts.vcf")).unwrap();
    assert!(vcf.contains("FN:Ana Lima"), "{vcf}");
    assert!(ticks > 0);

    // Read back with the importer.
    let back = Store::open(dir.join("back")).unwrap();
    let imported = crate::pst::import(&file, &back, "b1", &mut |_| {}).unwrap();
    assert_eq!(imported.messages, 3, "{imported:?}");
    let all = back.load_folders("b1").unwrap();
    let inbox = back.load_folder("b1", "INBOX").unwrap();
    assert_eq!(inbox.messages.len(), 2, "{all:?}");
    let plan = inbox.messages.iter().find(|m| m.subject == "Plan — café").unwrap();
    let raw = back.read_body(&MessageKey { account: "b1".into(), folder: "INBOX".into(), validity: 1, uid: plan.key.uid }).unwrap();
    let p = mime::parse(&raw).unwrap();
    assert_eq!(p.from.email, "riley@example.com");
    assert_eq!(p.cc[0].email, "ben@example.com");
    assert_eq!(p.message_id, "m1@example.com");
    assert_eq!(p.date, 1_704_067_200);
    assert!(p.attachments.iter().any(|a| a.name == "notes.txt"));
    assert!(all.iter().any(|f| f.path.ends_with("Projects") && f.total == 1), "{all:?}");

    // A file that can't be replaced: the error says where the mail is, and it is there.
    let busy = dir.join("Busy.pst");
    std::fs::create_dir_all(busy.join("inside")).unwrap();
    let e = export(&store, "a1", "Riley's mail", &busy, &mut |_| {}).unwrap_err();
    assert!(e.contains("Busy.pst.part"), "{e}");
    let back = Store::open(dir.join("back3")).unwrap();
    assert_eq!(crate::pst::import(&dir.join("Busy.pst.part"), &back, "b3", &mut |_| {}).unwrap().messages, 3);

    // Nothing to write: an error, and the old file stays.
    let empty = dir.join("Empty.pst");
    assert!(export(&store, "nobody", "x", &empty, &mut |_| {}).is_err());
    assert!(!empty.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The subjects of `file`'s mail as the import reads them back, in order.
pub(super) fn subjects_back(file: &Path, dir: &Path, label: &str) -> Vec<String> {
    let back = Store::open(dir.join(label)).unwrap();
    crate::pst::import(file, &back, "b", &mut |_| {}).unwrap();
    let mut out: Vec<String> = Vec::new();
    for f in back.load_folders("b").unwrap() {
        out.extend(back.load_folder("b", &f.path).unwrap().messages.iter().map(|m| m.subject.clone()));
    }
    out.sort();
    out
}

/// An account with `messages` (uid, subject) in its Inbox, each with Message-ID `m<uid>@example.com`.
pub(super) fn inbox_account(dir: &Path, messages: &[(u32, &str)]) -> Store {
    let store = Store::open(dir.join("cache")).unwrap();
    let mk = |uid: u32, subject: &str| {
        let d = mime::Draft { subject: subject.into(), body: "Hello there.".into(), ..Default::default() };
        let raw = mime::build(&d, &EmailAddress::new("Riley", "riley@example.com"), 1_704_067_200, &format!("m{uid}@example.com")).unwrap();
        let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
        store.write_body(&key, &raw).unwrap();
        mime::summary_from_full(&raw, key, Flags::default(), 10, 0)
    };
    let inbox = FolderState { validity: 1, uid_next: 100, messages: messages.iter().map(|(uid, s)| mk(*uid, s)).collect(), ..Default::default() };
    store.save_folder("a1", "INBOX", &inbox).unwrap();
    let folder =
        quatrelle_model::Folder { account: "a1".into(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
    store.save_folders("a1", &[folder]).unwrap();
    store
}

/// A data file another program made, with mail of its own and one of the account's messages
/// (by Message-ID).
pub(super) fn theirs(file: &Path) {
    let theirs = |subject, message_id| UnicodePstMessage {
        subject,
        sender_name: "Ana",
        sender_email: "ana@example.com",
        recipients: &[],
        body: "Kept as it was.",
        html_body: None,
        message_id,
        delivery_time: 133_485_408_000_000_000,
        message_class: "IPM.Note",
        properties: &[],
    };
    drop(UnicodePstFile::create_in_folder(file, &["Inbox"], &theirs("Theirs", "<theirs@example.com>")).unwrap());
    drop(UnicodePstFile::append_in_folder(file, &["Inbox"], &theirs("Both", "<m1@example.com>")).unwrap());
}

/// Exporting into a data file already there adds to it, as Outlook's export does: what it had
/// stays, a message it already has (by Message-ID) isn't written twice, and a second export adds
/// nothing. Something there that isn't a data file is left alone, and said so.
#[test]
fn a_data_file_already_there_is_added_to() {
    let dir = std::env::temp_dir().join(format!("quatrelle-pst-export-added-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = inbox_account(&dir, &[(1, "Both"), (2, "Ours")]);
    let file = dir.join("Outlook.pst");
    theirs(&file);
    let done = export(&store, "a1", "Riley's mail", &file, &mut |_| {}).unwrap();
    assert_eq!((done.messages, done.already), (1, 1));
    assert_eq!(subjects_back(&file, &dir, "back1"), ["Both", "Ours", "Theirs"]);
    let again = export(&store, "a1", "Riley's mail", &file, &mut |_| {}).unwrap();
    assert_eq!((again.messages, again.already), (0, 2));
    assert_eq!(subjects_back(&file, &dir, "back2"), ["Both", "Ours", "Theirs"]);
    assert!(!dir.join("Outlook.pst.part").exists());

    let other = dir.join("Notes.pst");
    std::fs::write(&other, b"an older file").unwrap();
    assert!(export(&store, "a1", "Riley's mail", &other, &mut |_| {}).is_err_and(|e| e.contains("isn't an Outlook data file")));
    assert_eq!(std::fs::read(&other).unwrap(), b"an older file");
    let _ = std::fs::remove_dir_all(&dir);
}
