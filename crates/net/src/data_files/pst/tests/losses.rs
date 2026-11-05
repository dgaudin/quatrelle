// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! What an import loses when its last save fails.

use super::*;

/// An empty import's state, writing into `store` for `account`.
fn out<'a>(store: &'a Store, account: &'a str, progress: &'a mut dyn FnMut(usize)) -> Out<'a> {
    Out {
        store,
        account,
        folders: Vec::new(),
        states: HashMap::new(),
        imported: 0,
        skipped: 0,
        progress,
        pim: Pim::default(),
        pending: Vec::new(),
        pending_bytes: 0,
    }
}

/// A mail item as read from a data file.
fn mail(n: usize) -> Item {
    Item {
        class: "IPM.Note".into(),
        subject: format!("Message {n}"),
        from: EmailAddress::new("", "me@example.com"),
        date: 1_700_000_000,
        body: "Hi".into(),
        ..Default::default()
    }
}

/// A last save that fails says why, and keeps the counts of what came in before it: the
/// messages are written, and the folders they're in are handed back to be saved again.
#[test]
fn a_failed_last_save_keeps_what_came_in() {
    let store = Store::failing();
    let mut progress = |_| {};
    let mut o = out(&store, "d1", &mut progress);
    let inbox = o.add_folder(None, "Inbox");
    o.add_message(&inbox, mail(1));
    store.fail(None, Some("folders.json"));
    let done = o.finish();
    assert_eq!((done.messages, done.folders.len()), (1, 1));
    assert!(done.last_save.as_deref().is_some_and(|e| e.contains("folder list")), "{done:?}");
    store.fail(None, None);
    assert_eq!(store.load_folder("d1", &inbox).unwrap().messages.len(), 1, "the message is in its folder's list");
    // The contacts' save failing is said too, the folders' having worked.
    let mut progress = |_| {};
    let mut o = out(&store, "d2", &mut progress);
    o.add_folder(None, "Inbox");
    o.pim.notes.push(StoredNote { note: Note { uid: "n1".into(), text: "Call Ana".into(), ..Default::default() }, ..Default::default() });
    store.fail(None, Some(PIM_DOC));
    let done = o.finish();
    assert!(done.last_save.as_deref().is_some_and(|e| e.contains("contacts")), "{done:?}");
    store.fail(None, None);
    assert_eq!(store.load_folders("d2").unwrap().len(), 1);
    let mut progress = |_| {};
    let mut o = out(&store, "d3", &mut progress);
    o.add_folder(None, "Inbox");
    assert_eq!(o.finish().last_save, None);
}

/// The reader failing on one folder of a data file costs that folder's rest, not the folders
/// read after it: each folder is read behind its own `catch_unwind`.
#[test]
fn a_panic_in_one_folder_keeps_the_others() {
    let dir = std::env::temp_dir().join(format!("quatrelle-pst-panic-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(dir.join("cache")).unwrap();
    let names = ["Archive", "Projects", "Receipts"];
    let mut folders = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let d = mime::Draft { subject: format!("In {name}"), body: "Hello.".into(), ..Default::default() };
        let raw = mime::build(&d, &EmailAddress::new("Riley", "riley@example.com"), 1_704_067_200, &format!("m{i}@example.com")).unwrap();
        let key = MessageKey { account: "a1".into(), folder: (*name).into(), validity: 1, uid: 1 };
        store.write_body(&key, &raw).unwrap();
        let summary = mime::summary_from_full(&raw, key, Flags::default(), 10, 0);
        store.save_folder("a1", name, &FolderState { validity: 1, uid_next: 2, messages: vec![summary], ..Default::default() }).unwrap();
        folders.push(Folder { account: "a1".into(), path: (*name).into(), delimiter: Some('/'), selectable: true, ..Default::default() });
    }
    store.save_folders("a1", &folders).unwrap();
    let file = dir.join("Three.pst");
    crate::pst_export::export(&store, "a1", "Three", &file, &mut |_| {}).unwrap();
    for (i, bad) in names.iter().enumerate() {
        fail_on(Some(bad));
        let account = format!("b{i}");
        let done = import(&file, &store, &account, &mut |_| {});
        fail_on(None);
        let done = done.unwrap();
        let came: Vec<&str> = done.folders.iter().filter(|f| f.total > 0).map(|f| f.path.as_str()).collect();
        let others: Vec<&str> = names.iter().copied().filter(|n| n != bad).collect();
        assert_eq!(came, others, "{bad} failing: {done:?}");
        assert_eq!(done.messages, 2, "{bad} failing");
        assert!(done.folders.iter().any(|f| f.path == *bad), "{bad} is there, empty");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
