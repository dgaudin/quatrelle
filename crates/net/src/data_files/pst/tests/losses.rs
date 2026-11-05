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
