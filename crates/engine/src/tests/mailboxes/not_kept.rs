// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: a message read on the server and not kept on this computer (older mail past
//! what's kept, a folder of the online archive) has its source held in memory while it's open,
//! for what needs the whole message: its headers, its attachments, Save As. Closed, it goes;
//! nothing of it is written to the cache.

use quatrelle_model::{AccountConfig, Folder, MessageKey, Summary};

use super::*;

const ARCHIVE: &str = "Online Archive - Riley/Inbox";

/// A message with a text and a file, as the server sends it.
const RAW: &[u8] = b"From: Zoe Park <zoe@contoso.example>\r\nTo: riley@contoso.example\r\nSubject: Harbour plans\r\n\
MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"b1\"\r\n\r\n--b1\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n\
The plans are attached.\r\n--b1\r\nContent-Type: text/plain; name=\"notes.txt\"\r\nContent-Disposition: attachment; filename=\"notes.txt\"\r\n\r\n\
Quay seven, berth two.\r\n--b1--\r\n";

/// An Exchange account with its Inbox (one message kept here, two in all) and a folder of its
/// online archive, the Inbox on view.
fn exchange(name: &str) -> (Session, Store, String) {
    let store = temp(name);
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    let config = AccountConfig {
        name: "Riley".into(),
        email: "riley@contoso.example".into(),
        username: "riley".into(),
        ews: Some("https://mail.contoso.example/EWS/Exchange.asmx".into()),
        ..Default::default()
    };
    let id = s.add_account(config, Some("pw".into())).unwrap();
    let f = |path: &str, role: FolderRole, total: u32, archive: bool| Folder {
        account: id.clone(),
        path: path.into(),
        delimiter: Some('/'),
        role,
        selectable: true,
        total,
        archive,
        mailbox: archive.then(|| "archive@contoso.example".to_string()),
        ..Default::default()
    };
    s.apply(Event::Folders { account: id.clone(), folders: vec![f("INBOX", FolderRole::Inbox, 2, false), f(ARCHIVE, FolderRole::Other, 1, true)] });
    let kept = summary(&id, "INBOX", 1, "Kept here", 2);
    store.save_folder(&id, "INBOX", &quatrelle_store::FolderState { validity: 1, messages: vec![kept.clone()], ..Default::default() }).unwrap();
    store.write_body(&kept.key, b"Subject: Kept here\r\n\r\nOn this computer.\r\n").unwrap();
    store
        .save_folder(
            &id,
            ARCHIVE,
            &quatrelle_store::FolderState { validity: 1, messages: vec![summary(&id, ARCHIVE, 1, "Harbour plans", 2_000)], ..Default::default() },
        )
        .unwrap();
    s.apply(Event::FolderChanged { account: id.clone(), folder: "INBOX".into() });
    s.run("folder.open", &json!({"account": id, "folder": "INBOX"})).unwrap();
    s.take_jobs();
    (s, store, id)
}

fn summary(id: &str, folder: &str, uid: u32, subject: &str, days_ago: i64) -> Summary {
    Summary {
        key: MessageKey { account: id.into(), folder: folder.into(), validity: 1, uid },
        subject: subject.into(),
        date: NOW - days_ago * 86_400,
        ..Default::default()
    }
}

/// The open message's file's index.
fn notes_index(s: &Session) -> usize {
    let parsed = s.reading.as_ref().and_then(|r| r.parsed.as_ref()).expect("shown");
    parsed.attachments.iter().find(|a| a.name == "notes.txt").map(|a| a.index).expect("its file")
}

/// What needs the whole of the open message works from the source held with it.
fn whole_message_works(s: &mut Session, store: &Store, key: &MessageKey) {
    let h = s.run("message.headers", &json!({})).unwrap();
    let head = h["text"].as_str().unwrap();
    assert!(head.contains("Subject: Harbour plans") && !head.contains("Quay seven"), "{head}");
    assert_eq!(s.message_source().unwrap(), RAW);
    assert_eq!(s.source_of(key).as_deref(), Some(RAW));
    let index = notes_index(s);
    assert!(String::from_utf8_lossy(&s.attachment_bytes(index).unwrap()).contains("Quay seven, berth two."));
    let p = s.run("attachment.preview", &json!({"index": index})).unwrap();
    assert!(p["text"].as_str().is_some_and(|t| t.contains("Quay seven")), "{p}");
    let out = store.root().join("saved");
    std::fs::create_dir_all(&out).unwrap();
    let eml = out.join("plans.eml");
    s.run("message.saveAs", &json!({"path": eml.to_string_lossy()})).unwrap();
    assert_eq!(std::fs::read(&eml).unwrap(), RAW, "Save As writes the message as it came");
    let file = out.join("notes.txt");
    s.run("attachment.save", &json!({"index": index, "path": file.to_string_lossy()})).unwrap();
    assert!(String::from_utf8_lossy(&std::fs::read(&file).unwrap()).contains("Quay seven"));
    let all = out.join("all");
    std::fs::create_dir_all(&all).unwrap();
    let r = s.run("attachment.saveAll", &json!({"dir": all.to_string_lossy()})).unwrap();
    assert_eq!(r["saved"].as_array().map(Vec::len), Some(1), "{r}");
    let r = s.run("messages.saveAs", &json!({"keys": [key.to_id()], "dir": out.to_string_lossy()})).unwrap();
    assert_eq!(r["missing"], json!(0), "{r}");
    s.run("message.exportPdf", &json!({"path": out.join("plans.pdf").to_string_lossy()})).unwrap();
    // Its source window opens on it, without asking the server again.
    let v = s.run("message.source", &json!({})).unwrap();
    assert_eq!(v["pending"], json!(false));
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::FetchBody { .. } | Job::ReadOnline { .. })), "not brought again");
    assert!(!store.has_body(key), "nothing of it is written to the cache");
}

/// Older mail read on the server, shown and not kept: while it's open (in the reading pane or a
/// window of its own) its headers, its files and Save As work from its source held with it;
/// another message opened, or its window closed, and the copy goes. A message kept here is
/// read from the cache, as before, and holds nothing.
#[test]
fn older_mail_open_holds_its_source_until_it_closes() {
    let (mut s, store, id) = exchange("held-older");
    s.run("folder.more", &json!({})).unwrap();
    s.take_jobs();
    let older = summary(&id, "INBOX", 7, "Harbour plans", 1_100);
    s.apply(Event::Older { account: id.clone(), folder: "INBOX".into(), items: vec![older.clone()], more: false, error: None });
    let old = older.key;
    s.open_message(&old);
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::ReadOnline { key } if *key == old)));
    s.apply(Event::Body { key: old.clone(), raw: RAW.to_vec() });
    whole_message_works(&mut s, &store, &old);

    // Another message opened: the copy goes with the one that closed.
    let kept = summary(&id, "INBOX", 1, "", 0).key;
    s.open_message(&kept);
    assert_eq!(s.source_of(&old), None, "let go once it closed");
    let r = s.run("messages.saveAs", &json!({"keys": [old.to_id()], "dir": store.root().to_string_lossy()})).unwrap();
    assert_eq!(r["missing"], json!(1));
    // The one kept here: from the cache, as before, nothing held.
    let h = s.run("message.headers", &json!({})).unwrap();
    assert!(h["text"].as_str().is_some_and(|t| t.contains("Subject: Kept here")), "{h}");
    assert!(s.reading.as_ref().is_some_and(|r| r.held.is_none()), "a message kept here holds no copy");
    s.apply(Event::Body { key: kept.clone(), raw: b"Subject: Kept here\r\n\r\nOn this computer.\r\n".to_vec() });
    assert!(s.reading.as_ref().is_some_and(|r| r.held.is_none()), "nor when its source comes again");

    // In a window of its own: held while the window is open, let go when it closes.
    s.pop_out(&old).unwrap();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::ReadOnline { key } if *key == old)));
    s.apply(Event::Body { key: old.clone(), raw: RAW.to_vec() });
    assert_eq!(s.source_of(&old).as_deref(), Some(RAW));
    s.close_window(&old);
    assert_eq!(s.source_of(&old), None, "let go with its window");
    assert!(!store.has_body(&old));
    let _ = std::fs::remove_dir_all(store.root());
}

/// A folder of the online archive is read on the server: a message of it whose source came and
/// isn't in the cache is held the same way while it's open, and let go when another folder is.
#[test]
fn an_online_archive_message_not_kept_holds_its_source_while_open() {
    let (mut s, store, id) = exchange("held-archive");
    assert!(s.read_on_server(&id, ARCHIVE));
    s.run("folder.open", &json!({"account": id, "folder": ARCHIVE})).unwrap();
    s.take_jobs();
    let key = summary(&id, ARCHIVE, 1, "", 0).key;
    s.open_message(&key);
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::FetchBody { key: k } | Job::FetchTextFirst { key: k } if *k == key)));
    assert!(s.run("message.headers", &json!({})).is_err(), "nothing to read before it comes");
    s.apply(Event::Body { key: key.clone(), raw: RAW.to_vec() });
    whole_message_works(&mut s, &store, &key);
    s.run("folder.open", &json!({"account": id, "folder": "INBOX"})).unwrap();
    assert_eq!(s.source_of(&key), None, "let go with the folder");
    let _ = std::fs::remove_dir_all(store.root());
}

/// A source past the cap isn't held: what needs it says the message isn't here, as before.
#[test]
fn a_source_past_the_cap_is_not_held() {
    let (mut s, store, id) = exchange("held-cap");
    s.run("folder.open", &json!({"account": id, "folder": ARCHIVE})).unwrap();
    let key = summary(&id, ARCHIVE, 1, "", 0).key;
    s.open_message(&key);
    s.take_jobs();
    s.apply(Event::Body { key: key.clone(), raw: RAW.to_vec() });
    let shown = s.reading.as_mut().unwrap();
    assert!(shown.held.is_some());
    shown.held = None;
    s.hold_if_not_kept(&key, &vec![b'a'; crate::HELD_SOURCE_MAX + 1]);
    assert!(s.reading.as_ref().is_some_and(|r| r.held.is_none()), "past the cap");
    let e = s.run("message.headers", &json!({})).unwrap_err().to_string();
    assert_eq!(e, crate::NOT_DOWNLOADED);
    s.hold_if_not_kept(&key, RAW);
    assert!(s.reading.as_ref().is_some_and(|r| r.held.is_some()), "within it");
    let _ = std::fs::remove_dir_all(store.root());
}
