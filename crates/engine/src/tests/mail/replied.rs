// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: the original of a reply or a forward sent is marked replied to or forwarded;
//! on Exchange, which keeps the last thing done only, the last one shows, as in Outlook. A reply
//! to all marks it replied to all, also from the Outbox, across a restart.

use super::*;

/// An account with one message open in its Inbox (`ews`: an Exchange account), its body in.
fn one_message(name: &str, ews: bool) -> (Session, String, MessageKey) {
    let mut s = Session::open(Some(temp(name)), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    if ews && let Some(a) = s.accounts.iter_mut().find(|a| a.config.id == id) {
        a.config.ews = Some("https://mail.example.org/EWS/Exchange.asmx".into());
    }
    let folders = vec![
        quatrelle_model::Folder { account: id.clone(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() },
        quatrelle_model::Folder { account: id.clone(), path: "Sent".into(), role: FolderRole::Sent, selectable: true, ..Default::default() },
    ];
    // Kept on disk as the network layer keeps them, for a restart to find.
    s.store.as_ref().unwrap().save_folders(&id, &folders).unwrap();
    s.apply(Event::Folders { account: id.clone(), folders });
    let key = MessageKey { account: id.clone(), folder: "INBOX".into(), validity: 9, uid: 1 };
    let st = quatrelle_store::FolderState {
        validity: 9,
        uid_next: 2,
        highest_modseq: 0,
        messages: vec![Summary { key: key.clone(), subject: "Hello".into(), date: NOW - 60, ..Default::default() }],
        window: 0,
    };
    s.store.as_ref().unwrap().save_folder(&id, "INBOX", &st).unwrap();
    s.apply(Event::FolderChanged { account: id.clone(), folder: "INBOX".into() });
    s.take_jobs();
    (s, id, key)
}

/// Write `command` on the message open and send it (`queued`: the network layer keeps it in the
/// Outbox, its mark owed until delivery): the id it goes by.
fn send(s: &mut Session, id: &str, key: &MessageKey, command: &str, queued: bool) -> u64 {
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    s.apply(Event::Body { key: key.clone(), raw: b"From: a@b.example\r\nSubject: Hello\r\nMessage-ID: <h@b>\r\n\r\nHi there\r\n".to_vec() });
    s.run(command, &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ann@example.org"})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    let send_id = match s.take_jobs().into_iter().find(|j| matches!(j, Job::Send { .. })) {
        Some(Job::Send { id, .. }) => id,
        other => panic!("{other:?}"),
    };
    if queued {
        s.apply(Event::Queued { account: id.to_string(), id: send_id, error: "offline".into() });
        assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::Apply { op: Op::SetFlag { .. }, .. })), "owed until delivery");
    }
    send_id
}

/// `send_id` delivered: the changes made to the original, in order (flag, on, to all), each
/// written to the cache on disk as the network layer writes them (journaled, then made).
fn delivered(s: &mut Session, id: &str, key: &MessageKey, send_id: u64) -> Vec<(Flag, bool, bool)> {
    s.apply(Event::Sent { account: id.to_string(), id: send_id });
    let store = s.store.clone().unwrap();
    let mut changes = Vec::new();
    for j in s.take_jobs() {
        if let Job::Apply { account, op } = j {
            store.add_op(&account, op.clone()).unwrap();
            store.apply_op(&account, &op).unwrap();
            if let Op::SetFlag { keys, flag, on, all } = op
                && keys.contains(key)
                && flag != Flag::Seen
            {
                changes.push((flag, on, all));
            }
        }
    }
    changes
}

/// Write `command` on the message open, send it, and have it delivered (`queued`: kept in the
/// Outbox first): the changes made to the original, in order (flag, on, to all).
fn sent_all(s: &mut Session, id: &str, key: &MessageKey, command: &str, queued: bool) -> Vec<(Flag, bool, bool)> {
    let send_id = send(s, id, key, command, queued);
    delivered(s, id, key, send_id)
}

/// [`sent_all`], each change's flag and whether it's on.
fn sent(s: &mut Session, id: &str, key: &MessageKey, command: &str, queued: bool) -> Vec<(Flag, bool)> {
    sent_all(s, id, key, command, queued).into_iter().map(|(flag, on, _)| (flag, on)).collect()
}

fn arrows(s: &Session, key: &MessageKey) -> (bool, bool) {
    let f = &s.summary(key).unwrap().flags;
    (f.answered, f.forwarded)
}

/// The original's marks in the cache on disk, read again by a store of its own.
fn on_disk(s: &Session, key: &MessageKey) -> quatrelle_model::Flags {
    let again = Store::open(s.store.as_ref().unwrap().root()).unwrap();
    again.load_folder(&key.account, &key.folder).unwrap().messages.into_iter().find(|m| m.key == *key).unwrap().flags
}

/// The session open again on the same data folder, the Inbox shown.
fn reopened(s: Session, id: &str) -> Session {
    let root = s.store.as_ref().unwrap().root().to_path_buf();
    drop(s);
    let mut s = Session::open(Some(Store::open(&root).unwrap()), quatrelle_model::dates::Clock::new(NOW, 0));
    s.run("folder.open", &json!({"account": id, "folder": "INBOX"})).unwrap();
    s.take_jobs();
    s
}

/// On Exchange the last thing done shows: forwarded after a reply, the reply's arrow goes (the
/// server is told the forward only); replied to again, the forward's goes. Delivered from the
/// Outbox, the same.
#[test]
fn on_exchange_the_last_reply_or_forward_shows() {
    let (mut s, id, key) = one_message("replied-exchange", true);
    assert_eq!(sent(&mut s, &id, &key, "mail.reply", false), vec![(Flag::Answered, true)]);
    assert_eq!(arrows(&s, &key), (true, false));
    assert_eq!(sent(&mut s, &id, &key, "mail.forward", false), vec![(Flag::Answered, false), (Flag::Forwarded, true)]);
    assert_eq!(arrows(&s, &key), (false, true), "the forward was last");
    assert_eq!(sent(&mut s, &id, &key, "mail.replyAll", true), vec![(Flag::Forwarded, false), (Flag::Answered, true)]);
    assert_eq!(arrows(&s, &key), (true, false), "replied to all, from the Outbox");
}

/// IMAP keeps both marks (`\Answered`, `$Forwarded`), each a fact of its own: neither is taken
/// off.
#[test]
fn over_imap_a_reply_and_a_forward_both_stay() {
    let (mut s, id, key) = one_message("replied-imap", false);
    assert_eq!(sent(&mut s, &id, &key, "mail.reply", false), vec![(Flag::Answered, true)]);
    assert_eq!(sent(&mut s, &id, &key, "mail.forward", false), vec![(Flag::Forwarded, true)]);
    assert_eq!(arrows(&s, &key), (true, true));
}

/// A reply to all marks its original replied to all, in memory and in the cache on disk; a
/// reply, replied to only. IMAP keeps the reply to all when a reply follows (each mark a fact of
/// its own); Exchange keeps the last thing done, so there a reply after a reply to all takes
/// Answered off first (the server makes nothing of that) and the original shows replied to.
#[test]
fn a_reply_to_all_marks_its_original_so() {
    let (mut s, id, key) = one_message("replied-to-all", false);
    assert_eq!(sent_all(&mut s, &id, &key, "mail.replyAll", false), vec![(Flag::Answered, true, true)]);
    assert!(s.summary(&key).unwrap().flags.replied_all(), "in memory");
    assert!(on_disk(&s, &key).replied_all(), "on disk");
    assert_eq!(sent_all(&mut s, &id, &key, "mail.reply", false), vec![(Flag::Answered, true, false)]);
    assert!(s.summary(&key).unwrap().flags.replied_all() && on_disk(&s, &key).replied_all(), "IMAP keeps it");
    let _ = std::fs::remove_dir_all(s.store.as_ref().unwrap().root());

    let (mut s, id, key) = one_message("replied-not-to-all", false);
    assert_eq!(sent_all(&mut s, &id, &key, "mail.reply", false), vec![(Flag::Answered, true, false)]);
    let (shown, kept) = (s.summary(&key).unwrap().flags.clone(), on_disk(&s, &key));
    assert!(shown.answered && !shown.replied_all() && kept.answered && !kept.replied_all(), "{shown:?} {kept:?}");
    let _ = std::fs::remove_dir_all(s.store.as_ref().unwrap().root());

    let (mut s, id, key) = one_message("replied-to-all-exchange", true);
    assert_eq!(sent_all(&mut s, &id, &key, "mail.replyAll", false), vec![(Flag::Answered, true, true)]);
    assert!(s.summary(&key).unwrap().flags.replied_all() && on_disk(&s, &key).replied_all());
    let changes = sent_all(&mut s, &id, &key, "mail.reply", false);
    assert_eq!(changes, vec![(Flag::Answered, false, false), (Flag::Answered, true, false)], "the reply to all's mark goes first");
    let (shown, kept) = (s.summary(&key).unwrap().flags.clone(), on_disk(&s, &key));
    assert!(shown.answered && !shown.replied_all() && kept.answered && !kept.replied_all(), "{shown:?} {kept:?}");
    let _ = std::fs::remove_dir_all(s.store.as_ref().unwrap().root());
}

/// A reply to all waiting in the Outbox owes a reply to all: after a restart it's still one, and
/// `pending-marks.json` keeps the shape 1.5.0 reads strictly (its ids, originals and flags).
#[test]
fn a_reply_to_all_owed_survives_a_restart_and_keeps_the_old_file_shape() {
    let (mut s, id, key) = one_message("replied-to-all-owed", false);
    let send_id = send(&mut s, &id, &key, "mail.replyAll", true);
    let owed = s.store.as_ref().unwrap().load_doc_strict::<Vec<(u64, MessageKey, Flag)>>("pending-marks.json");
    assert_eq!(owed.unwrap(), vec![(send_id, key.clone(), Flag::Answered)], "as 1.5.0 reads it");
    let mut s = reopened(s, &id);
    assert_eq!(delivered(&mut s, &id, &key, send_id), vec![(Flag::Answered, true, true)]);
    assert!(s.summary(&key).unwrap().flags.replied_all(), "in memory");
    assert!(on_disk(&s, &key).replied_all(), "on disk");
    let store = s.store.clone().unwrap();
    assert!(store.load_doc_strict::<Vec<(u64, MessageKey, Flag)>>("pending-marks.json").unwrap().is_empty(), "nothing owed now");
    assert!(store.load_doc_strict::<Vec<u64>>("pending-reply-all.json").unwrap().is_empty(), "nothing owed now");
    let _ = std::fs::remove_dir_all(store.root());
}

/// The list of owed replies to all, damaged, naming ids that aren't owed, or gone: the session
/// opens with nothing said, the original is still marked replied to (to all or not, the list
/// can't tell), and `pending-marks.json` isn't set aside.
#[test]
fn a_damaged_reply_all_list_still_marks_the_reply() {
    let damages: [Option<&[u8]>; 3] = [Some(b"{\"x\":"), Some(b"[18446744073709551615, 5]"), None];
    for (i, damage) in damages.into_iter().enumerate() {
        let (mut s, id, key) = one_message(&format!("replied-to-all-damaged-{i}"), false);
        let send_id = send(&mut s, &id, &key, "mail.replyAll", true);
        let root = s.store.as_ref().unwrap().root().to_path_buf();
        let beside = root.join("pending-reply-all.json");
        assert!(beside.exists(), "{i}: the reply to all owed is kept beside");
        match damage {
            Some(bytes) => std::fs::write(&beside, bytes).unwrap(),
            None => std::fs::remove_file(&beside).unwrap(),
        }
        let mut s = reopened(s, &id);
        assert!(!s.notices.iter().any(|n| n.error), "{i}: {:?}", s.notices);
        assert_eq!(delivered(&mut s, &id, &key, send_id), vec![(Flag::Answered, true, false)], "{i}");
        assert!(s.summary(&key).unwrap().flags.answered && on_disk(&s, &key).answered, "{i}");
        assert!(!root.join("pending-marks.json.bad").exists(), "{i}: pending-marks.json set aside");
        assert_eq!(root.join("pending-reply-all.json.bad").exists(), i == 0, "{i}: only the damaged list is set aside");
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// A reply to all taken back out of the Outbox to be changed is still a reply to all.
#[test]
fn a_reply_to_all_taken_back_from_the_outbox_is_still_one() {
    let (mut s, _, key) = one_message("replied-to-all-edited", false);
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    s.apply(Event::Body { key: key.clone(), raw: b"From: a@b.example\r\nSubject: Hello\r\nMessage-ID: <h@b>\r\n\r\nHi there\r\n".to_vec() });
    s.run("mail.replyAll", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ann@studio.example", "sendAfter": NOW + 3600})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    s.run("folder.outbox", &json!({})).unwrap();
    let queued = s.list[0].key.clone();
    s.run("outbox.edit", &json!({"key": queued.to_id()})).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!((c.source, c.mode), (Some(key), Some(quatrelle_mime::ReplyMode::ReplyAll)));
    let _ = std::fs::remove_dir_all(s.store.as_ref().unwrap().root());
}
