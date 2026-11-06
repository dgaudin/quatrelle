// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The account's Sent, Trash, Drafts and Junk folders go to the data file's own special folders.

use outlook_pst::UnicodePstSpecialFolder as Special;

use super::*;

/// An account whose special folders have names of their own (`Sent`, `Bin`, `Spam`), one inside
/// Sent, and Junk with no mail.
fn account(dir: &Path) -> Store {
    let store = Store::open(dir.join("cache")).unwrap();
    let mut folders = Vec::new();
    let mut uid = 0;
    for (path, role, subject) in [
        ("INBOX", FolderRole::Inbox, Some("Plan")),
        ("Sent", FolderRole::Sent, Some("Sent one")),
        ("Sent/2024", FolderRole::Other, Some("Older sent")),
        ("Drafts", FolderRole::Drafts, Some("Draft one")),
        ("Bin", FolderRole::Trash, Some("Thrown away")),
        ("Spam", FolderRole::Junk, None),
    ] {
        let mut messages = Vec::new();
        if let Some(subject) = subject {
            uid += 1;
            let d = mime::Draft { subject: subject.into(), body: "Hello.".into(), ..Default::default() };
            let raw = mime::build(&d, &EmailAddress::new("Riley", "riley@example.com"), 1_704_067_200, &format!("m{uid}@example.com")).unwrap();
            let key = MessageKey { account: "a1".into(), folder: path.into(), validity: 1, uid };
            store.write_body(&key, &raw).unwrap();
            messages.push(mime::summary_from_full(&raw, key, Flags { seen: true, ..Default::default() }, 10, 0));
        }
        store.save_folder("a1", path, &FolderState { validity: 1, uid_next: 100, messages, ..Default::default() }).unwrap();
        folders.push(quatrelle_model::Folder {
            account: "a1".into(),
            path: path.into(),
            delimiter: Some('/'),
            role,
            selectable: true,
            ..Default::default()
        });
    }
    store.save_folders("a1", &folders).unwrap();
    store
}

/// A new file records its special folders as Outlook names them, the account's mail in them, and
/// the import finds them again by what the file records.
#[test]
fn special_folders_go_to_the_file_s_own() {
    let dir = std::env::temp_dir().join(format!("quatrelle-pst-export-special-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = account(&dir);
    let file = dir.join("Out.pst");
    assert_eq!(export(&store, "a1", "Riley's mail", &file, NONE, &mut |_| {}).unwrap().messages, 5);
    for (special, name) in [
        (Special::SentItems, "Sent Items"),
        (Special::DeletedItems, "Deleted Items"),
        (Special::Drafts, "Drafts"),
        (Special::JunkEmail, "Junk Email"),
    ] {
        assert_eq!(UnicodePstFile::special_folder(&file, special).unwrap(), Some(vec![name.to_string()]), "{special:?}");
    }
    assert_eq!(UnicodePstFile::special_folder(&file, Special::Outbox).unwrap(), None);
    let back = Store::open(dir.join("back")).unwrap();
    assert_eq!(crate::pst::import(&file, &back, "b1", &mut |_| {}).unwrap().messages, 5);
    let all = back.load_folders("b1").unwrap();
    for (role, path) in
        [(FolderRole::Sent, "Sent Items"), (FolderRole::Trash, "Deleted Items"), (FolderRole::Drafts, "Drafts"), (FolderRole::Junk, "Junk Email")]
    {
        assert!(all.iter().any(|f| f.role == role && f.path == path), "{role:?} {all:?}");
    }
    assert!(all.iter().any(|f| f.path == "Sent Items/2024" && f.total == 1), "{all:?}");
    assert_eq!(back.load_folder("b1", "Deleted Items").unwrap().messages[0].subject, "Thrown away");
    // Exported again: nothing twice, and no second Sent Items beside the first.
    assert_eq!(export(&store, "a1", "Riley's mail", &file, NONE, &mut |_| {}).unwrap().already, 5);
    let back = Store::open(dir.join("back2")).unwrap();
    crate::pst::import(&file, &back, "b2", &mut |_| {}).unwrap();
    assert!(!back.load_folders("b2").unwrap().iter().any(|f| f.path.starts_with("Sent Items (") || f.path == "Sent"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file added to keeps its own special folders: the account's Sent mail goes into the folder
/// it records, whatever its name, and none of its own is recorded in its place.
#[test]
fn an_export_uses_the_special_folders_a_file_has() {
    let dir = std::env::temp_dir().join(format!("quatrelle-pst-export-special-theirs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("Theirs.pst");
    let theirs = UnicodePstMessage {
        subject: "Theirs",
        sender_name: "Ana",
        sender_email: "ana@example.com",
        recipients: &[],
        body: "Kept.",
        html_body: None,
        message_id: "<theirs@example.com>",
        delivery_time: 133_485_408_000_000_000,
        message_class: "IPM.Note",
        properties: &[],
    };
    drop(UnicodePstFile::create_in_folder(&file, &["Gesendete Elemente"], &theirs).unwrap());
    drop(UnicodePstFile::set_special_folder(&file, Special::SentItems, &["Gesendete Elemente"]).unwrap());
    let store = account(&dir);
    export(&store, "a1", "Riley's mail", &file, NONE, &mut |_| {}).unwrap();
    assert_eq!(UnicodePstFile::special_folder(&file, Special::SentItems).unwrap(), Some(vec!["Gesendete Elemente".to_string()]));
    let back = Store::open(dir.join("back")).unwrap();
    crate::pst::import(&file, &back, "b1", &mut |_| {}).unwrap();
    let folders = back.load_folders("b1").unwrap();
    let sent = folders.iter().find(|f| f.path == "Gesendete Elemente").unwrap();
    assert_eq!((sent.role, sent.total), (FolderRole::Sent, 2), "{folders:?}");
    assert!(!folders.iter().any(|f| f.path == "Sent Items" || f.path == "Sent"), "{folders:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
