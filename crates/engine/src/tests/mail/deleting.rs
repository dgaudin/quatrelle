// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: Delete for good (Shift+Delete), on whatever Delete acts on: the mail goes
//! without passing through Deleted Items, and nothing claims it can be undone.

use super::*;

/// Whether the sample's `folder` still holds a message called `subject` (on disk, not only
/// in the list shown).
fn kept(s: &Session, folder: &str, subject: &str) -> bool {
    s.store.as_ref().unwrap().load_folder(sample::SAMPLE_ACCOUNT, folder).unwrap().messages.iter().any(|m| m.subject == subject)
}

#[test]
fn delete_for_good_skips_deleted_items_and_leaves_nothing_to_undo() {
    let mut s = sample_session("for-good");
    let k = key_of(&s, "Sound mix, version 3");
    s.run("list.open", &k).unwrap();
    let v = s.run("mail.deleteForGood", &json!({})).unwrap();
    assert_eq!(v["deleted"], 1);
    assert!(!s.list.iter().any(|m| m.subject == "Sound mix, version 3"));
    assert!(!kept(&s, "INBOX", "Sound mix, version 3"), "gone from the Inbox");
    assert!(!kept(&s, "Trash", "Sound mix, version 3"), "not in Deleted Items either");
    // Undo (Ctrl+Z) has nothing to bring back: it doesn't pretend to.
    assert!(s.undo.is_empty());
    assert!(matches!(s.run("edit.undo", &json!({})), Err(CmdError::Disabled(_))));
    // Nothing selected: nothing to delete.
    s.selection.clear();
    assert!(s.run("mail.deleteForGood", &json!({})).is_err());
    done(s);
}

/// A whole conversation selected, and a search's results: the same messages as Delete takes.
#[test]
fn delete_for_good_takes_a_conversation_and_search_results() {
    let mut s = sample_session("for-good-many");
    s.run("view.conversations", &json!({"on": true})).unwrap();
    let Some(t) = s.threads.iter().find(|t| t.messages.len() > 1).cloned() else { panic!("the sample has a conversation") };
    let keys: Vec<MessageKey> = t.messages.iter().filter_map(|&i| s.list.get(i)).map(|m| m.key.clone()).collect();
    let subjects: Vec<String> = keys.iter().filter_map(|k| s.summary(k)).map(|m| m.subject.clone()).collect();
    for (i, k) in keys.iter().enumerate() {
        s.run("list.select", &json!({"key": k.to_id(), "add": i > 0})).unwrap();
    }
    assert_eq!(s.run("mail.deleteForGood", &json!({})).unwrap()["deleted"], keys.len());
    for k in &keys {
        assert!(s.store.as_ref().unwrap().load_folder(&k.account, &k.folder).unwrap().messages.iter().all(|m| m.key != *k));
    }
    let trash: Vec<String> =
        s.store.as_ref().unwrap().load_folder(sample::SAMPLE_ACCOUNT, "Trash").unwrap().messages.iter().map(|m| m.subject.clone()).collect();
    assert!(subjects.iter().all(|x| !trash.contains(x)), "none in Deleted Items: {trash:?}");

    // From a search's results.
    s.run("view.conversations", &json!({"on": false})).unwrap();
    s.run("search.run", &json!({"query": "turnarounds", "scope": "all"})).unwrap();
    let k = key_of(&s, "Character turnarounds");
    s.run("list.select", &k).unwrap();
    s.run("mail.deleteForGood", &json!({})).unwrap();
    assert!(!kept(&s, "INBOX", "Character turnarounds") && !kept(&s, "Trash", "Character turnarounds"));
    done(s);
}

/// In the Outbox, Shift+Delete does what Delete does: the message isn't sent.
#[test]
fn delete_for_good_in_the_outbox_cancels_the_send_as_delete_does() {
    let mut s = sample_session("for-good-outbox");
    let store = s.store.clone().unwrap();
    let raw = b"From: riley@studio.example\r\nTo: ann@example.org\r\nSubject: Plans\r\n\r\nSee you\r\n".to_vec();
    let item = quatrelle_store::OutboxItem { id: 41, from: "riley@studio.example".into(), queued: NOW, ..Default::default() };
    store.outbox_add(sample::SAMPLE_ACCOUNT, &item, &raw).unwrap();
    s.apply(Event::Queued { account: sample::SAMPLE_ACCOUNT.into(), id: 41, error: "offline".into() });
    s.run("folder.outbox", &json!({})).unwrap();
    assert_eq!(s.list.len(), 1);
    s.run("list.selectAll", &json!({})).unwrap();
    s.run("mail.deleteForGood", &json!({})).unwrap();
    assert!(s.list.is_empty() && store.outbox(sample::SAMPLE_ACCOUNT).is_empty());
    done(s);
}
