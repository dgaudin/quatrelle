// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A message the server may have taken without saying so (its answer lost on the way: a laptop
//! asleep, a VPN gone) is never sent again by itself, on Exchange, Graph and JMAP as on SMTP: it
//! stays in the Outbox marked so, or comes back to its window saying it may have gone.

use super::read_errors::worker;
use super::*;

/// A message waiting in the Outbox of account `a1`.
fn queue(store: &Store, id: u64, from: &str) {
    let raw = format!("From: {from}\r\nTo: zoe@studio.example\r\nSubject: Lunch {id}\r\nMessage-ID: <{id}@studio.example>\r\n\r\nAt one?\r\n");
    let item = OutboxItem {
        id,
        from: from.into(),
        recipients: vec!["zoe@studio.example".into()],
        sent_folder: Some("Sent".into()),
        queued: i64::try_from(id).unwrap_or_default(),
        ..Default::default()
    };
    store.outbox_add("a1", &item, raw.as_bytes()).unwrap();
}

/// The Outbox sent twice, then once more by a worker started again (a new start of Quatrelle):
/// how many messages the server took is `sent()`; the one in the Outbox stays, marked as one that
/// may have gone, and the user is told so.
fn sent_once_and_kept(what: &str, acc: &AccountConfig, secret: &str, sent: &dyn Fn() -> usize) {
    let store = Store::failing();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    queue(&store, 7, &acc.email);
    let mut w = worker(acc.clone(), secret, &store, &pool);
    w.connect().unwrap();
    assert!(w.relist(), "{what}");
    w.flush_outbox();
    w.flush_outbox();
    let mut again = worker(acc.clone(), secret, &store, &pool);
    again.connect().unwrap();
    assert!(again.relist(), "{what}");
    again.flush_outbox();
    assert_eq!(sent(), 1, "{what}: sent again by itself");
    let waiting = store.outbox("a1");
    assert!(waiting.first().and_then(|i| i.error.as_deref()).is_some_and(|e| e.starts_with("unconfirmed:")), "{what}: {waiting:?}");
    let mut said = Vec::new();
    while let Some(e) = pool.wait(Duration::from_millis(100)) {
        said.push(e);
    }
    assert!(said.iter().any(|e| matches!(e, Event::SendFailed { id: 7, unconfirmed: true, .. })), "{what}: {said:?}");
    // Sent from its window (not the Outbox): the same, said to the window, and not put in the
    // Outbox to go again.
    let raw = format!("From: {}\r\nTo: zoe@studio.example\r\nSubject: Now\r\nMessage-ID: <now@studio.example>\r\n\r\nNow?\r\n", acc.email);
    again.send(8, raw.as_bytes(), &acc.email, &["zoe@studio.example".to_string()], Some("Sent".into()));
    assert_eq!(sent(), 2, "{what}");
    assert!(!store.outbox("a1").iter().any(|i| i.id == 8), "{what}: put in the Outbox, to go again");
    let mut said = Vec::new();
    while let Some(e) = pool.wait(Duration::from_millis(100)) {
        said.push(e);
    }
    assert!(said.iter().any(|e| matches!(e, Event::SendFailed { id: 8, unconfirmed: true, .. })), "{what}: {said:?}");
}

/// Exchange (the owner's account): `CreateItem` sends the message, then the connection closes
/// before its answer. It was taken for a network failure: kept in the Outbox and sent again at
/// the next round, so it went twice.
#[test]
fn an_exchange_message_whose_answer_was_lost_isnt_sent_again() {
    use crate::fakes::fake_ews::{DOMAIN, Data, PASSWORD, USER, serve};
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Sent Items", None, Some("sentitems"), "IPF.Note");
    d.sends_cut = 2;
    let server = serve(d);
    let acc = AccountConfig {
        id: "a1".into(),
        email: "riley@contoso.example".into(),
        username: format!("{DOMAIN}\\{USER}"),
        ews: Some(server.url.clone()),
        ..Default::default()
    };
    sent_once_and_kept("ews", &acc, PASSWORD, &|| server.data.lock().unwrap().sent.len());
}

/// Graph: `sendMail` sends it, then the connection closes before its answer.
#[test]
fn a_graph_message_whose_answer_was_lost_isnt_sent_again() {
    use crate::fakes::fake_graph::{Data, TOKEN, serve};
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"));
    d.folder("Sent Items", None, Some("sentitems"));
    d.sends_cut = 2;
    let server = serve(d);
    let acc = AccountConfig { id: "a1".into(), email: "me@contoso.example".into(), graph: Some(server.base.clone()), ..Default::default() };
    sent_once_and_kept("graph", &acc, TOKEN, &|| server.data.lock().unwrap().sent.len());
}

/// JMAP: the submission is made, then the connection closes before its answer.
#[test]
fn a_jmap_message_whose_answer_was_lost_isnt_sent_again() {
    use crate::fakes::fake_jmap::{Data, PASSWORD, USER, serve};
    let mut d = Data::default();
    d.mailbox("Inbox", None, Some("inbox"));
    d.mailbox("Sent", None, Some("sent"));
    d.sends_cut = 2;
    let server = serve(d);
    let acc = AccountConfig { id: "a1".into(), email: USER.into(), username: USER.into(), jmap: Some(server.base.clone()), ..Default::default() };
    sent_once_and_kept("jmap", &acc, PASSWORD, &|| server.data.lock().unwrap().submissions.len());
}
