// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! A message's text first over Graph: what's asked of the server, which messages are left to come
//! whole, and the worker bringing the text, then the rest.

use quatrelle_mime as mime;
use quatrelle_model::MessageKey;

use super::*;
use crate::graph::{Client, fetch_text, list_folders, sync_folder};
use crate::jmap::{State, VALIDITY};
use crate::worker::{MemoryCredentials, Pool};

/// The attached file's size.
const FILE: usize = 200_000;

/// A message with a text and one file of [`FILE`] bytes, of the given type and name.
fn with_file(subject: &str, id: &str, kind: &str, name: &str) -> Vec<u8> {
    let bytes = crate::imap::base64(&vec![7u8; FILE]);
    format!(
        "From: Sam Lee <sam@contoso.example>\r\nTo: me@contoso.example\r\nSubject: {subject}\r\nMessage-ID: <{id}@x>\r\nMIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=B\r\n\r\n--B\r\nContent-Type: text/plain\r\n\r\nThe notes, to read now.\r\n\
         --B\r\nContent-Type: {kind}; name=\"{name}\"\r\nContent-Disposition: attachment; filename=\"{name}\"\r\nContent-Transfer-Encoding: base64\r\n\r\n{bytes}\r\n--B--\r\n"
    )
    .into_bytes()
}

fn rushes(id: &str) -> Vec<u8> {
    with_file("Rushes", id, "video/quicktime", "reel.mov")
}

fn start(server: &Server, store: &Store, shared: &[&str]) -> Pool {
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    pool.opts = crate::sync::SyncOptions { prefetch: 0, ..Default::default() };
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@contoso.example".into(),
        graph: Some(server.base.clone()),
        shared_mailboxes: shared.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret(TOKEN.into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    pool
}

fn is_text(e: &Event, key: &MessageKey) -> bool {
    matches!(e, Event::TextFirst { key: k, .. } if k == key)
}

fn is_body(e: &Event, key: &MessageKey) -> bool {
    matches!(e, Event::Body { key: k, .. } if k == key)
}

fn calls(server: &Server) -> Vec<String> {
    server.data.lock().unwrap().calls.clone()
}

/// The text of a message with a file comes first, the file named and sized, no bytes; then the
/// whole message, which is what's kept.
#[test]
fn a_message_text_first_from_graph() {
    let source = rushes("r1");
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"));
    d.deliver(&inbox, &source, false, "2024-05-06T10:00:00Z");
    d.source_delay = Duration::from_secs(1);
    let server = serve(d);
    let store = crate::fake::tests_store("graph-text-first");
    let mut pool = start(&server, &store, &[]);
    let m = store.load_folder("a1", "INBOX").unwrap().messages[0].clone();
    assert!(m.has_attachments && !store.has_body(&m.key), "not downloaded ahead");
    pool.submit(Job::FetchTextFirst { key: m.key.clone() });
    let seen = until(&pool, "the text", |e| is_text(e, &m.key));
    assert!(!store.has_body(&m.key), "shown before the source came");
    let Some(Event::TextFirst { raw, .. }) = seen.last() else { panic!("{seen:?}") };
    let p = mime::parse_text_first(raw).unwrap();
    assert_eq!((p.from.name.as_str(), p.from.email.as_str()), ("Sam Lee", "sam@contoso.example"));
    assert_eq!(p.subject, "Rushes");
    assert!(p.text.contains("The notes, to read now."), "{}", p.text);
    assert_eq!(p.attachments.iter().map(|a| (a.name.as_str(), a.size)).collect::<Vec<_>>(), [("reel.mov", FILE)]);
    assert!(raw.len() < 4096, "no bytes of the file: {}", raw.len());
    let seen = until(&pool, "the whole message", |e| is_body(e, &m.key));
    assert!(matches!(seen.last(), Some(Event::Body { raw, .. }) if *raw == source));
    assert_eq!(store.read_body(&m.key).unwrap(), source, "what's kept is the source, not the text built");
    let calls = calls(&server);
    assert!(calls.iter().any(|c| c.contains("$expand=attachments")) && calls.iter().any(|c| c.ends_with("$value")), "{calls:?}");
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

/// What the text alone wouldn't show comes whole: an invitation, which Graph types
/// `eventMessage`, and a signed or encrypted message, which has its `smime.p7m` among its files.
#[test]
fn graph_leaves_an_invitation_and_a_signed_message_whole() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"));
    let invitation = d.deliver(&inbox, &with_file("Review", "i1", "text/calendar", "invite.ics"), false, "2024-05-06T10:00:00Z");
    d.invitations.push(invitation.clone());
    let signed = d.deliver(&inbox, &with_file("Contract", "s1", "application/pkcs7-mime", "smime.p7m"), false, "2024-05-06T11:00:00Z");
    let plain = d.deliver(&inbox, &rushes("r1"), false, "2024-05-06T12:00:00Z");
    let server = serve(d);
    let c = Client::connect(&server.base, TOKEN).unwrap();
    assert!(c.download_text(&plain, None).unwrap().is_some(), "and the same shape of message with an ordinary file does come");
    assert!(c.download_text(&invitation, None).unwrap().is_none(), "an invitation");
    assert!(c.download_text(&signed, None).unwrap().is_none(), "a signed message");

    // Through the worker: no text first, the message whole at once.
    let store = crate::fake::tests_store("graph-text-first-whole");
    let mut pool = start(&server, &store, &[]);
    for subject in ["Review", "Contract"] {
        let m = store.load_folder("a1", "INBOX").unwrap().messages.into_iter().find(|m| m.subject == subject).unwrap();
        assert!(m.has_attachments, "{subject}: the list says it has files");
        pool.submit(Job::FetchTextFirst { key: m.key.clone() });
        let seen = until(&pool, "the whole message", |e| is_body(e, &m.key));
        assert!(!seen.iter().any(|e| matches!(e, Event::TextFirst { .. })), "{subject}: {seen:?}");
        assert!(store.has_body(&m.key));
    }
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

/// A message of another mailbox opened in the account is asked for under that mailbox, its text
/// and its source alike; asked for under this one it isn't found.
#[test]
fn a_shared_mailbox_message_comes_text_first_under_its_owner() {
    let support = "support@contoso.example";
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"));
    d.shared = vec![support.into()];
    let theirs = d.shared_folder(support, "Inbox", None, Some("inbox"));
    let ticket = d.deliver(&theirs, &with_file("Printer", "p1", "image/png", "photo.png"), false, "2024-05-06T10:00:00Z");
    let server = serve(d);
    let c = Client::connect(&server.base, TOKEN).unwrap();
    assert!(c.download_text(&ticket, Some(support)).unwrap().is_some());
    assert!(c.download_text(&ticket, None).is_err(), "not in this mailbox");

    let store = crate::fake::tests_store("graph-text-first-shared");
    let mut pool = start(&server, &store, &[support]);
    eventually("their inbox", || store.load_folder("a1", "support@contoso.example/Inbox").is_ok_and(|f| f.messages.len() == 1));
    let key = store.load_folder("a1", "support@contoso.example/Inbox").unwrap().messages[0].key.clone();
    let before = calls(&server).len();
    pool.submit(Job::FetchTextFirst { key: key.clone() });
    until(&pool, "the text", |e| is_text(e, &key));
    until(&pool, "the whole message", |e| is_body(e, &key));
    let asked: Vec<String> = calls(&server).into_iter().skip(before).filter(|c| c.contains("/messages/")).collect();
    assert_eq!(asked.len(), 2, "{asked:?}");
    assert!(asked.iter().all(|c| c.starts_with("GET /users/support%40contoso.example/messages/")), "{asked:?}");
    assert!(asked[0].contains("$expand=attachments") && asked[1].ends_with("$value"), "{asked:?}");
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

/// A numbering and a store that know the Inbox, and its message's key.
fn synced(c: &Client, name: &str) -> (Store, State, MessageKey) {
    let mut st = State::default();
    list_folders(c, &mut st, "a1", &[]).unwrap();
    let store = crate::fake::tests_store(name);
    sync_folder(c, &mut st, &store, "a1", "INBOX", &crate::sync::SyncOptions { prefetch: 0, full: true, ..Default::default() }).unwrap();
    let key = store.load_folder("a1", "INBOX").unwrap().messages[0].key.clone();
    (store, st, key)
}

#[test]
fn the_text_of_a_listed_message_and_its_id() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"));
    let id = d.deliver(&inbox, &rushes("r1"), false, "2024-05-06T10:00:00Z");
    let server = serve(d);
    let c = Client::connect(&server.base, TOKEN).unwrap();
    let (store, st, key) = synced(&c, "graph-text-first-id");
    let (raw, found) = fetch_text(&c, &st, &store, &key).unwrap().unwrap();
    assert_eq!(found, id);
    assert_eq!(mime::parse_text_first(&raw).unwrap().attachments.len(), 1);
    assert!(!store.has_body(&key), "the text isn't the message: nothing kept");
    let _ = std::fs::remove_dir_all(store.root());
}

/// Nothing to ask for: another numbering, a message that's here, a message the numbering doesn't
/// know (the whole path says it's gone).
#[test]
fn no_text_for_what_is_here_or_out_of_reach() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"));
    d.deliver(&inbox, &rushes("r1"), false, "2024-05-06T10:00:00Z");
    let server = serve(d);
    let c = Client::connect(&server.base, TOKEN).unwrap();
    let (store, st, key) = synced(&c, "graph-text-first-none");
    let asked = || calls(&server).iter().filter(|c| c.contains("$expand=attachments")).count();
    let other = MessageKey { validity: VALIDITY + 1, ..key.clone() };
    assert!(fetch_text(&c, &st, &store, &other).unwrap().is_none(), "another numbering");
    let unknown = MessageKey { uid: 999, ..key.clone() };
    assert!(fetch_text(&c, &st, &store, &unknown).unwrap().is_none(), "not numbered: left to the whole path to say so");
    assert!(fetch_text(&c, &State::default(), &store, &key).unwrap().is_none(), "no numbering at all");
    store.write_body(&key, b"Subject: x\r\n\r\nhere\r\n").unwrap();
    assert!(fetch_text(&c, &st, &store, &key).unwrap().is_none(), "it's here");
    assert_eq!(asked(), 0, "nothing asked of the server");
    let _ = std::fs::remove_dir_all(store.root());
}

/// One message with a file, listed, on a pool whose server is `d`.
fn opened_with(d: Data, name: &str) -> (Server, Store, Pool, MessageKey) {
    let server = serve(d);
    let store = crate::fake::tests_store(name);
    let pool = start(&server, &store, &[]);
    let key = store.load_folder("a1", "INBOX").unwrap().messages[0].key.clone();
    (server, store, pool, key)
}

fn one_message() -> Data {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"));
    d.deliver(&inbox, &rushes("r1"), false, "2024-05-06T10:00:00Z");
    d
}

/// A sign-in the server no longer takes is told as the connection's failure; asking for the whole
/// message would only be refused again.
#[test]
fn a_refused_token_on_the_text_is_told_and_the_whole_message_is_not_asked_for() {
    let (server, store, mut pool, key) = opened_with(one_message(), "graph-text-first-401");
    server.data.lock().unwrap().token = Some("renewed-elsewhere".into());
    pool.submit(Job::FetchTextFirst { key: key.clone() });
    let seen = until(&pool, "the failure", |e| matches!(e, Event::BodyFailed { key: k, .. } if *k == key));
    assert_eq!(seen.iter().filter(|e| matches!(e, Event::BodyFailed { .. })).count(), 1, "{seen:?}");
    assert!(!seen.iter().any(|e| matches!(e, Event::TextFirst { .. } | Event::Body { .. })), "{seen:?}");
    assert!(!calls(&server).iter().any(|c| c.ends_with("$value")), "{:?}", calls(&server));
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

/// Any other error on the text is the text's alone (a gateway timing out on `$expand`): the whole
/// message comes as it would have.
#[test]
fn a_gateway_error_on_the_text_leaves_the_whole_message_to_come() {
    let (server, store, mut pool, key) = opened_with(one_message(), "graph-text-first-503");
    server.data.lock().unwrap().text_status = Some(503);
    pool.submit(Job::FetchTextFirst { key: key.clone() });
    let seen = until(&pool, "the whole message", |e| is_body(e, &key));
    assert!(!seen.iter().any(|e| matches!(e, Event::TextFirst { .. } | Event::BodyFailed { .. })), "{seen:?}");
    assert!(store.has_body(&key));
    let calls = calls(&server);
    assert!(calls.iter().any(|c| c.contains("$expand=attachments")) && calls.iter().any(|c| c.ends_with("$value")), "{calls:?}");
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}
