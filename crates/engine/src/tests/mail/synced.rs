// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: what a sync changed in a folder, told as it is (`FolderSynced`, noted where the
//! list is saved), taken by the folder in memory and the list on screen without reading the
//! folder again; read again when what's in memory isn't the list on the disk with it.

use quatrelle_model::{Folder, ListChange, MessageKey, Summary};
use quatrelle_store::{FolderState, Noted};

use super::*;

/// A session over an Exchange account whose Inbox of `n` unread messages is on disk and shown.
fn server_inbox(n: u32) -> (Session, String, Store) {
    let store = Store::memory();
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    let add = json!({"email": "riley@contoso.example", "ews": "mail.contoso.example", "username": "CONTOSO\\riley", "password": "pw"});
    let id = s.run("account.add", &add).unwrap()["account"].as_str().unwrap().to_string();
    let messages = (1..=n)
        .map(|uid| Summary {
            key: MessageKey { account: id.clone(), folder: "INBOX".into(), validity: 1, uid },
            date: NOW - 86_400 - i64::from(uid) * 37,
            message_id: format!("<m{uid}@x>"),
            subject: format!("Plan {}", uid / 3),
            ..Default::default()
        })
        .collect();
    store.save_folder(&id, "INBOX", &FolderState { validity: 1, uid_next: n + 1, messages, ..Default::default() }).unwrap();
    let inbox = Folder { account: id.clone(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
    s.apply(Event::Folders { account: id.clone(), folders: vec![inbox] });
    s.open_view(View::Folder { account: id.clone(), folder: "INBOX".into() });
    s.apply(Event::FolderChanged { account: id.clone(), folder: "INBOX".into() });
    s.take_jobs();
    let _ = s.take_new_mail();
    (s, id, store)
}

/// A sync, as an account's thread makes one: the list read from the disk, changed by `change`,
/// saved while its changes are noted; what's noted.
fn sync(store: &Store, id: &str, change: impl FnOnce(&mut FolderState)) -> Noted {
    let noting = store.note_changes(id, "INBOX");
    let mut st = store.load_folder(id, "INBOX").unwrap();
    change(&mut st);
    quatrelle_model::sort_newest_first(&mut st.messages);
    store.save_folder(id, "INBOX", &st).unwrap();
    noting.take()
}

fn told(s: &mut Session, id: &str, noted: Noted) {
    match noted {
        Noted::Changed(change) => s.apply(Event::FolderSynced { account: id.into(), folder: "INBOX".into(), change }),
        other => panic!("not told: {other:?}"),
    }
}

/// What memory holds is what the disk holds: the list built from it is the same.
fn as_on_disk(s: &mut Session, id: &str) {
    let shown: Vec<std::sync::Arc<Summary>> = s.list.clone();
    s.cache.remove(&(id.to_string(), "INBOX".to_string()));
    s.refresh_list();
    assert_eq!(s.list, shown, "memory as the disk has it");
}

/// Three come, one goes, one flagged on a big server folder on screen: the folder in memory and
/// the list take it, nothing read again; the new mail is noted; a read mark made here and not yet
/// on the disk when the sync read the folder stays read.
#[test]
fn a_sync_takes_its_change_without_reading_the_folder() {
    let (mut s, id, store) = server_inbox(10_941);
    let marked = s.row_keys()[2].clone();
    s.open_message(&marked);
    let sent = s.take_jobs();
    assert!(sent.iter().any(|j| matches!(j, Job::Apply { .. })), "the mark sent");
    let read = s.messages_read();
    let noted = sync(&store, &id, |st| {
        st.messages.retain(|m| m.key.uid != 5_000);
        if let Some(m) = st.find_mut(9) {
            m.flags.flagged = true;
        }
        for uid in 10_942..=10_944 {
            st.messages.push(Summary {
                key: MessageKey { account: id.clone(), folder: "INBOX".into(), validity: 1, uid },
                date: NOW - 60,
                subject: format!("Came {uid}"),
                ..Default::default()
            });
        }
    });
    told(&mut s, &id, noted);
    assert_eq!(s.messages_read(), read, "nothing read again");
    assert_eq!(s.list.len(), 10_943);
    assert_eq!(s.list.first().map(|m| m.subject.as_str()), Some("Came 10944"), "the newest first");
    assert!(s.summary(&marked).is_some_and(|m| m.flags.seen), "the mark made here stays");
    assert!(s.list.iter().all(|m| m.key.uid != 5_000));
    assert!(s.list.iter().any(|m| m.key.uid == 9 && m.flags.flagged));
    assert_eq!(s.take_new_mail().len(), 3, "the new mail noted");
    // The mark reaches the disk, its echo comes back.
    for j in sent {
        if let Job::Apply { account, op } = j {
            store.apply_op(&account, &op).unwrap();
        }
    }
    s.apply(Event::FolderChanged { account: id.clone(), folder: "INBOX".into() });
    assert_eq!(s.messages_read(), read, "the echo read nothing either");
    as_on_disk(&mut s, &id);
}

/// A sync that changes read marks alone changes the rows' counts in place: no rows built, no
/// conversations worked out.
#[test]
fn read_marks_from_the_server_change_the_rows_in_place() {
    let (mut s, id, store) = server_inbox(3_000);
    s.run("view.conversations", &json!({"on": true})).unwrap();
    let (built, threadings, revision) = (s.nav_structure(), s.nav.threadings(), s.revision);
    let noted = sync(&store, &id, |st| {
        for m in st.messages.iter_mut().filter(|m| m.key.uid % 100 == 0) {
            m.flags.seen = true;
        }
    });
    told(&mut s, &id, noted);
    assert_eq!(s.nav_structure(), built, "no rows built");
    assert_eq!(s.nav.threadings(), threadings, "no conversations worked out");
    assert_ne!(s.revision, revision, "drawn again");
    let (rows, _, _) = s.build_rows();
    assert_eq!(s.rows(), &rows[..], "unread counts as built afresh");
    as_on_disk(&mut s, &id);
}

/// A change that doesn't take what's in memory to the list on the disk (another change saved
/// meanwhile and not told): the folder is read again, as when nothing is told.
#[test]
fn a_change_that_isnt_the_list_on_disk_reads_it_again() {
    let (mut s, id, store) = server_inbox(50);
    // Saved, not told.
    let mut st = store.load_folder(&id, "INBOX").unwrap();
    st.messages.retain(|m| m.key.uid != 7);
    store.save_folder(&id, "INBOX", &st).unwrap();
    let read = s.messages_read();
    let noted = sync(&store, &id, |st| {
        st.messages.push(Summary {
            key: MessageKey { account: id.clone(), folder: "INBOX".into(), validity: 1, uid: 51 },
            date: NOW,
            ..Default::default()
        });
    });
    told(&mut s, &id, noted);
    assert_eq!(s.messages_read(), read + 50, "read again");
    assert!(s.list.iter().all(|m| m.key.uid != 7));
    as_on_disk(&mut s, &id);
    // Numbered anew: read again too.
    let change = ListChange { validity: 2, count: 1, print: ListChange::print([1]), ..ListChange::default() };
    let read = s.messages_read();
    s.apply(Event::FolderSynced { account: id.clone(), folder: "INBOX".into(), change: Box::new(change) });
    assert_eq!(s.messages_read(), read + 50, "read again");
}
