// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: the original of a reply or a forward sent is marked replied to or forwarded;
//! on Exchange, which keeps the last thing done only, the last one shows, as in Outlook.

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

/// Write `command` on the message open, send it, and have it delivered (`queued`: kept in the
/// Outbox first): the changes made to the original, in order.
fn sent(s: &mut Session, id: &str, key: &MessageKey, command: &str, queued: bool) -> Vec<(Flag, bool)> {
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
    s.apply(Event::Sent { account: id.to_string(), id: send_id });
    s.take_jobs()
        .into_iter()
        .filter_map(|j| match j {
            Job::Apply { op: Op::SetFlag { keys, flag, on }, .. } if keys.contains(key) && flag != Flag::Seen => Some((flag, on)),
            _ => None,
        })
        .collect()
}

fn arrows(s: &Session, key: &MessageKey) -> (bool, bool) {
    let f = &s.summary(key).unwrap().flags;
    (f.answered, f.forwarded)
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
