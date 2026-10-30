//! A message's text first over EWS: what's asked of the server, what it answers, and which
//! requests a real Exchange would refuse.

use quatrelle_mime as mime;
use quatrelle_model::MessageKey;

use super::super::server::Server;
use super::*;
use crate::ews::{Client, fetch_body, fetch_text, sync_folder};
use crate::jmap::{State, VALIDITY};
use crate::worker::{MemoryCredentials, Pool};

/// The attached file's size.
const FILE: usize = 30_000;

/// A message with a text, and a file of [`FILE`] bytes.
fn with_a_file(id: &str) -> Vec<u8> {
    titled("Reel", id)
}

/// [`with_a_file`] under another subject.
fn titled(subject: &str, id: &str) -> Vec<u8> {
    let bytes = crate::imap::base64(&vec![7u8; FILE]);
    format!(
        "From: Zoe Park <zoe@contoso.example>\r\nTo: Riley <riley@contoso.example>\r\nSubject: {subject}\r\nMessage-ID: <{id}@x>\r\nImportance: high\r\n\
         MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n\
         --b\r\nContent-Type: text/plain\r\n\r\nHere's the reel.\r\n\
         --b\r\nContent-Type: video/quicktime\r\nContent-Disposition: attachment; filename=reel.mov\r\nContent-Transfer-Encoding: base64\r\n\r\n{bytes}\r\n--b--\r\n"
    )
    .into_bytes()
}

fn connect(server: &Server) -> Client {
    Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap()
}

fn one_message(raw: &[u8]) -> (Data, String) {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let id = d.deliver(&inbox, raw, false, "2026-10-30T08:00:00Z");
    (d, id)
}

fn get_item(field: &str, id: &str) -> String {
    format!(
        "<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties><t:FieldURI FieldURI=\"{field}\"/></t:AdditionalProperties></m:ItemShape>\
         <m:ItemIds><t:ItemId Id=\"{id}\"/></m:ItemIds></m:GetItem>"
    )
}

/// The name upstream asked for: Exchange checks every field against its schema, and one it
/// doesn't know refuses the whole request.
#[test]
fn exchange_refuses_a_field_it_does_not_know() {
    let (d, id) = one_message(&mail("Reel", "m1"));
    let server = serve(d);
    let c = connect(&server);
    let refused = c.call_one(&get_item("message:InReplyTo", &id)).unwrap_err();
    assert!(refused.to_string().contains("ErrorSchemaValidation"), "{refused}");
    c.call_one(&get_item("item:InReplyTo", &id)).unwrap();
    c.call_one(&get_item("calendar:NotChecked", &id)).unwrap();
}

#[test]
fn a_message_text_from_the_server() {
    let (d, id) = one_message(&with_a_file("m1"));
    let server = serve(d);
    let c = connect(&server);
    let raw = c.download_text(&id).unwrap().unwrap();
    let p = mime::parse_text_first(&raw).unwrap();
    assert_eq!(p.subject, "Reel");
    assert_eq!(p.to, [quatrelle_model::EmailAddress::new("Riley", "riley@contoso.example")]);
    assert_eq!(p.from.email, "zoe@contoso.example");
    assert!(p.text.contains("Here's the reel."), "{}", p.text);
    assert_eq!(p.attachments.len(), 1);
    assert_eq!((p.attachments[0].name.as_str(), p.attachments[0].size), ("reel.mov", FILE));
    assert_eq!(mime::attachment_data(&raw, 0).unwrap().len(), 0, "listed empty");
    assert!(String::from_utf8_lossy(&raw).contains("Importance: high"));
    let shapes = || server.data.lock().unwrap().item_shapes.clone();
    assert_eq!(shapes(), ["IdOnly"]);
    c.download(&id).unwrap();
    assert_eq!(shapes(), ["IdOnly", "MimeContent"], "the source is asked for otherwise");
}

/// `HasAttachments` says so on the list, which is how a message gets here at all.
#[test]
fn the_list_says_a_message_has_files() {
    let (mut d, _) = one_message(&with_a_file("m1"));
    let inbox = d.by_name("Inbox").unwrap().id.clone();
    d.deliver(&inbox, &mail("Plain", "m2"), false, "2026-10-29T08:00:00Z");
    let server = serve(d);
    let (store, ..) = synced(&connect(&server), "ews-text-first-list");
    let list = store.load_folder("a1", "INBOX").unwrap().messages;
    let has = |subject: &str| list.iter().find(|m| m.subject == subject).unwrap().has_attachments;
    assert!(has("Reel") && !has("Plain"));
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn the_class_the_server_gives_decides() {
    let (mut d, id) = one_message(&with_a_file("m1"));
    d.items[0].class = "IPM.Note.SMIME".into();
    let server = serve(d);
    assert!(connect(&server).download_text(&id).unwrap().is_none());
}

/// The two ways the whole message and the text can part: each fails on its own.
#[test]
fn the_text_and_the_source_fail_apart() {
    let (mut d, id) = one_message(&with_a_file("m1"));
    d.refuse_text = true;
    let server = serve(d);
    let c = connect(&server);
    let refused = c.download_text(&id).unwrap_err();
    assert!(refused.to_string().contains("ErrorSchemaValidation"), "{refused}");
    assert!(!c.download(&id).unwrap().is_empty(), "refused for the body it names, not for the source");
    {
        let mut d = server.data.lock().unwrap();
        d.refuse_text = false;
        d.unreadable.push(id.clone());
    }
    assert!(c.download(&id).is_err());
    assert!(c.download_text(&id).unwrap().is_some(), "a source it can't give isn't the text");
}

/// A store and a numbering that know the Inbox, and the key of its newest message.
fn synced(c: &Client, name: &str) -> (Store, State, MessageKey) {
    let mut st = State::default();
    crate::ews::list_folders(c, &mut st, "a1").unwrap();
    let store = crate::fake::tests_store(name);
    sync_folder(c, &mut st, &store, "a1", "INBOX", &crate::sync::SyncOptions { prefetch: 0, full: true, ..Default::default() }).unwrap();
    let key = store.load_folder("a1", "INBOX").unwrap().messages[0].key.clone();
    (store, st, key)
}

#[test]
fn the_text_of_a_listed_message_and_its_id() {
    let (d, id) = one_message(&with_a_file("m1"));
    let server = serve(d);
    let c = connect(&server);
    let (store, mut st, key) = synced(&c, "ews-text-first-id");
    let (raw, found) = fetch_text(&c, &mut st, &store, &key).unwrap().unwrap();
    assert_eq!(found, id);
    assert_eq!(mime::parse_text_first(&raw).unwrap().attachments.len(), 1);
    assert!(!store.has_body(&key), "the text isn't the message: nothing kept");
    let _ = std::fs::remove_dir_all(store.root());
}

/// As [`fetch_body`]: a message the numbering lost is found again by its Message-ID.
#[test]
fn the_text_of_a_message_the_numbering_lost() {
    let (d, id) = one_message(&with_a_file("m1"));
    let server = serve(d);
    let c = connect(&server);
    let (store, st, key) = synced(&c, "ews-text-first-lost");
    let mut lost = State::default();
    lost.mailboxes = st.mailboxes.clone();
    let (_, found) = fetch_text(&c, &mut lost, &store, &key).unwrap().unwrap();
    assert_eq!(found, id);
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn no_text_for_what_is_here_or_out_of_reach() {
    let (d, _) = one_message(&with_a_file("m1"));
    let server = serve(d);
    let c = connect(&server);
    let (store, mut st, key) = synced(&c, "ews-text-first-none");
    let asked = || server.data.lock().unwrap().item_shapes.len();
    let before = asked();
    let other = MessageKey { validity: VALIDITY + 1, ..key.clone() };
    assert!(fetch_text(&c, &mut st, &store, &other).unwrap().is_none(), "another numbering");
    fetch_body(&c, &mut st, &store, &key).unwrap();
    let whole = asked();
    assert_eq!(whole, before + 1);
    assert!(fetch_text(&c, &mut st, &store, &key).unwrap().is_none(), "it's here");
    assert_eq!(asked(), whole, "nothing asked of the server");
    let mut gone = State::default();
    gone.mailboxes = st.mailboxes.clone();
    let unknown = MessageKey { uid: 999, ..key.clone() };
    assert!(fetch_text(&c, &mut gone, &store, &unknown).unwrap().is_none(), "not found: left to the whole path to say so");
    let _ = std::fs::remove_dir_all(store.root());
}

// Through the worker: the text on the main connection, the rest on a thread of its own.

/// The account on a pool, with nothing downloaded ahead: every message is still to be opened.
fn start(server: &Server, store: &Store) -> Pool {
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    pool.opts = crate::sync::SyncOptions { prefetch: 0, ..Default::default() };
    let account = AccountConfig {
        id: "a1".into(),
        email: "riley@contoso.example".into(),
        username: format!("{DOMAIN}\\{USER}"),
        ews: Some(server.url.clone()),
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret(PASSWORD.into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    pool
}

/// An Inbox of `n` messages, `M0` the oldest; their ids.
fn inbox_of(n: usize) -> (Data, Vec<String>) {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let ids =
        (0..n).map(|i| d.deliver(&inbox, &titled(&format!("M{i}"), &format!("m{i}")), false, &format!("2026-10-{:02}T08:00:00Z", i + 1))).collect();
    (d, ids)
}

fn key_of(store: &Store, subject: &str) -> MessageKey {
    store.load_folder("a1", "INBOX").unwrap().messages.iter().find(|m| m.subject == subject).unwrap().key.clone()
}

fn open(pool: &mut Pool, key: &MessageKey) {
    pool.submit(Job::FetchTextFirst { key: key.clone() });
}

fn is_text(e: &Event, key: &MessageKey) -> bool {
    matches!(e, Event::TextFirst { key: k, .. } if k == key)
}

fn is_body(e: &Event, key: &MessageKey) -> bool {
    matches!(e, Event::Body { key: k, .. } if k == key)
}

fn is_failed(e: &Event, key: &MessageKey) -> bool {
    matches!(e, Event::BodyFailed { key: k, .. } if k == key)
}

/// Until `saw` has been true of every key, each with its own event; what came meanwhile.
fn until_each(pool: &Pool, what: &str, keys: &[MessageKey], saw: fn(&Event, &MessageKey) -> bool) -> Vec<Event> {
    let mut left = keys.to_vec();
    until(pool, what, |e| {
        left.retain(|k| !saw(e, k));
        left.is_empty()
    })
}

fn sources_asked(server: &Server) -> usize {
    server.data.lock().unwrap().item_shapes.iter().filter(|s| *s == "MimeContent").count()
}

#[test]
fn the_text_comes_before_the_whole_message() {
    let source = with_a_file("m1");
    let (mut d, _) = one_message(&source);
    d.slow_sources = Some(Duration::from_secs(2));
    let server = serve(d);
    let store = crate::fake::tests_store("ews-text-first-before");
    let mut pool = start(&server, &store);
    let key = key_of(&store, "Reel");
    open(&mut pool, &key);
    let seen = until(&pool, "the text", |e| is_text(e, &key));
    assert!(!store.has_body(&key), "shown before the source came");
    let Some(Event::TextFirst { raw, .. }) = seen.last() else { panic!("{seen:?}") };
    assert!(mime::parse_text_first(raw).unwrap().text.contains("Here's the reel."));
    let seen = until(&pool, "the whole message", |e| is_body(e, &key));
    assert!(matches!(seen.last(), Some(Event::Body { raw, .. }) if *raw == source));
    assert_eq!(store.read_body(&key).unwrap(), source, "what's kept is the source, not the text built");
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn another_message_opened_meanwhile_shows_at_once() {
    let (mut d, _) = inbox_of(2);
    d.slow_sources = Some(Duration::from_secs(2));
    let server = serve(d);
    let store = crate::fake::tests_store("ews-text-first-meanwhile");
    let mut pool = start(&server, &store);
    let keys = [key_of(&store, "M0"), key_of(&store, "M1")];
    open(&mut pool, &keys[0]);
    open(&mut pool, &keys[1]);
    let opened = Instant::now();
    until_each(&pool, "both texts", &keys, is_text);
    assert!(opened.elapsed() < Duration::from_secs(1), "{:?}", opened.elapsed());
    until_each(&pool, "both messages", &keys, is_body);
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

/// Arrowing down through big messages, and coming back to one: every text comes while the first
/// message downloads, the last asked for goes next, and nothing downloads twice.
#[test]
fn the_last_opened_comes_first_and_each_once() {
    let (mut d, _) = inbox_of(5);
    d.slow_sources = Some(Duration::from_secs(1));
    let server = serve(d);
    let store = crate::fake::tests_store("ews-text-first-order");
    let mut pool = start(&server, &store);
    let keys: Vec<MessageKey> = (0..5).map(|i| key_of(&store, &format!("M{i}"))).collect();
    for key in &keys {
        open(&mut pool, key);
    }
    let mut seen = until_each(&pool, "every text", &keys, is_text);
    let third = keys[2].clone();
    open(&mut pool, &third);
    seen.extend(until(&pool, "its text again", |e| is_text(e, &third)));
    let done = seen.iter().filter(|e| matches!(e, Event::Body { .. })).count();
    seen.extend(until_each(&pool, "every message", &keys, is_body));
    let order: Vec<&MessageKey> = seen.iter().filter_map(|e| if let Event::Body { key, .. } = e { Some(key) } else { None }).collect();
    assert_eq!(order.len(), 5, "each once: {order:?}");
    assert!(keys.iter().all(|k| order.contains(&k)), "{order:?}");
    let at = order.iter().position(|k| **k == third).unwrap();
    assert!(at <= done + 1, "next after the one downloading when it was asked for again ({done} done): {order:?}");
    assert_eq!(sources_asked(&server), 5);
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn sending_waits_for_nothing() {
    let (mut d, _) = inbox_of(1);
    let inbox = d.by_name("Inbox").unwrap().id.clone();
    d.folder("Sent Items", None, Some("sentitems"), "IPF.Note");
    d.slow_sources = Some(Duration::from_secs(3));
    let server = serve(d);
    let store = crate::fake::tests_store("ews-text-first-send");
    let mut pool = start(&server, &store);
    let key = key_of(&store, "M0");
    open(&mut pool, &key);
    until(&pool, "the text", |e| is_text(e, &key));
    pool.submit(Job::Send {
        account: "a1".into(),
        id: 7,
        raw: b"From: riley@contoso.example\r\nTo: zoe@contoso.example\r\nSubject: Out\r\nMessage-ID: <out@x>\r\n\r\nhi\r\n".to_vec(),
        from: "riley@contoso.example".into(),
        recipients: vec!["zoe@contoso.example".into()],
        sent_folder: Some("Sent Items".into()),
    });
    let mut seen = until(&pool, "sent", |e| matches!(e, Event::Sent { id: 7, .. }));
    // Sending has a connection of its own: what shows the main one is free is a job on it.
    server.data.lock().unwrap().deliver(&inbox, &mail("New", "n1"), false, "2026-10-30T08:00:00Z");
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() });
    let read = |store: &Store| store.load_folder("a1", "INBOX").is_ok_and(|f| f.messages.iter().any(|m| m.subject == "New"));
    seen.extend(until(&pool, "the folder read", |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "INBOX") && read(&store)));
    assert!(!seen.iter().any(|e| is_body(e, &key)), "{seen:?}");
    until(&pool, "the whole message", |e| is_body(e, &key));
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

/// A copy of the connection knows where a public folder's and a shared mailbox's items are.
#[test]
fn a_public_folder_and_a_shared_mailbox_work() {
    let mut d = Data {
        pf_mailbox: Some("PFContent@contoso.example".into()),
        pf_hierarchy: Some("PFHierarchy@contoso.example".into()),
        anchored_items: true,
        slow_sources: Some(Duration::from_millis(200)),
        ..Default::default()
    };
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let root = d.folder("IPM_SUBTREE", None, Some("publicfoldersroot"), "IPF.Note");
    let news = d.folder("Announcements", Some(&root), None, "IPF.Note");
    d.deliver(&news, &titled("Holiday hours", "h"), false, "2026-10-14T09:00:00Z");
    let sales =
        d.shared_mailbox(SharedBox { address: "sales@contoso.example".into(), name: "Sales".into(), automap: true, legacy: false, granted: true });
    d.deliver(&sales, &titled("Order", "s"), false, "2026-10-15T09:00:00Z");
    let server = serve(d);
    let store = crate::fake::tests_store("ews-text-first-routes");
    let mut pool = start(&server, &store);
    let folders = store.load_folders("a1").unwrap();
    let public = folders.iter().find(|f| f.path.ends_with("Announcements")).map(|f| f.path.clone()).unwrap();
    let shared = folders.iter().find(|f| f.mailbox.is_some() && f.path.ends_with("/Inbox")).map(|f| f.path.clone()).unwrap();
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: public.clone() });
    let listed = |path: &str| store.load_folder("a1", path).ok().and_then(|f| f.messages.first().map(|m| m.key.clone()));
    eventually("listed when opened", || listed(&public).is_some());
    for (path, mailbox) in [(&public, "PFContent@contoso.example"), (&shared, "sales@contoso.example")] {
        let key = listed(path).unwrap();
        open(&mut pool, &key);
        until(&pool, "the text", |e| is_text(e, &key));
        until(&pool, "the whole message", |e| is_body(e, &key));
        let routes = server.data.lock().unwrap().source_routes.clone();
        assert!(routes.iter().any(|r| r == mailbox), "{mailbox}: {routes:?}");
    }
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

/// Two messages opened on a server refusing the text: both come whole, and it's asked once.
fn refused_once(d: Data, name: &str) {
    let server = serve(d);
    let store = crate::fake::tests_store(name);
    let mut pool = start(&server, &store);
    let texts_asked = || server.data.lock().unwrap().item_shapes.iter().filter(|s| *s == "IdOnly").count();
    let before = texts_asked();
    let mut seen = Vec::new();
    for subject in ["M0", "M1"] {
        let key = key_of(&store, subject);
        open(&mut pool, &key);
        seen.extend(until(&pool, "the whole message", |e| is_body(e, &key)));
    }
    assert!(!seen.iter().any(|e| matches!(e, Event::TextFirst { .. })), "{seen:?}");
    assert_eq!(texts_asked(), before + 1, "asked once, not at every message");
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn a_refused_text_falls_back_and_is_not_asked_again() {
    let (mut d, _) = inbox_of(2);
    d.refuse_text = true;
    refused_once(d, "ews-text-first-refused");
}

/// The same refusal item by item: the message's words hide its code from the error it would give.
#[test]
fn a_text_refused_item_by_item_is_not_asked_again() {
    let (mut d, _) = inbox_of(2);
    d.refuse_text_each = true;
    refused_once(d, "ews-text-first-refused-each");
}

#[test]
fn a_refusal_says_its_code() {
    let (mut d, id) = one_message(&with_a_file("m1"));
    d.refuse_text_each = true;
    let server = serve(d);
    let refused = connect(&server).download_text(&id).unwrap_err();
    assert!(matches!(&refused, crate::NetError::No(code) if code == "ErrorInvalidPropertyRequest"), "{refused:?}");
}

/// Any other refusal is the message's own: its words stay, for the person reading them.
#[test]
fn another_refusal_is_said_as_before() {
    let (d, _) = one_message(&with_a_file("m1"));
    let server = serve(d);
    let gone = connect(&server).download_text("I999").unwrap_err();
    assert!(matches!(&gone, crate::NetError::No(m) if m.starts_with("not on the server any more")), "{gone:?}");
}

#[test]
fn a_failed_download_is_told() {
    let (mut d, ids) = inbox_of(1);
    d.unreadable.push(ids[0].clone());
    let server = serve(d);
    let store = crate::fake::tests_store("ews-text-first-failed");
    let mut pool = start(&server, &store);
    let key = key_of(&store, "M0");
    open(&mut pool, &key);
    until(&pool, "the text", |e| is_text(e, &key));
    until(&pool, "the failure", |e| is_failed(e, &key));
    assert!(!store.has_body(&key));
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn a_message_deleted_meanwhile_is_told() {
    let (mut d, ids) = inbox_of(1);
    d.slow_sources = Some(Duration::from_millis(300));
    let server = serve(d);
    let store = crate::fake::tests_store("ews-text-first-deleted");
    let mut pool = start(&server, &store);
    let key = key_of(&store, "M0");
    open(&mut pool, &key);
    until(&pool, "the text", |e| is_text(e, &key));
    server.data.lock().unwrap().items.retain(|i| i.id != ids[0]);
    until(&pool, "the failure", |e| is_failed(e, &key));
    assert!(!store.has_body(&key));
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

/// Downloading ahead and the sync write sources without a word: a message waiting that one of
/// them brought meanwhile is still told of, or it would keep its spinner, and isn't asked for.
#[test]
fn a_message_written_while_it_waits_is_told_of_and_not_asked_for() {
    let (mut d, _) = inbox_of(2);
    d.slow_sources = Some(Duration::from_secs(1));
    let server = serve(d);
    let store = crate::fake::tests_store("ews-text-first-written");
    let mut pool = start(&server, &store);
    let (running, waiting) = (key_of(&store, "M0"), key_of(&store, "M1"));
    open(&mut pool, &running);
    until(&pool, "the first text", |e| is_text(e, &running));
    open(&mut pool, &waiting);
    until(&pool, "the second text", |e| is_text(e, &waiting));
    let source = titled("M1", "m1");
    store.write_body(&waiting, &source).unwrap();
    let seen = until(&pool, "the message written", |e| is_body(e, &waiting));
    assert!(matches!(seen.last(), Some(Event::Body { raw, .. }) if *raw == source));
    assert_eq!(sources_asked(&server), 1, "only the one downloading");
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

/// Working offline with a message downloading and another waiting: neither keeps its spinner, and
/// neither is written once the account has stopped. The pool stays, to listen.
#[test]
fn nothing_is_written_after_the_stop() {
    let (mut d, _) = inbox_of(2);
    d.slow_sources = Some(Duration::from_secs(2));
    let server = serve(d);
    let store = crate::fake::tests_store("ews-text-first-stop");
    let mut pool = start(&server, &store);
    let keys = [key_of(&store, "M0"), key_of(&store, "M1")];
    open(&mut pool, &keys[0]);
    open(&mut pool, &keys[1]);
    until_each(&pool, "both texts", &keys, is_text);
    pool.submit(Job::WorkOffline { on: true });
    let stopped = Instant::now();
    let seen = until_each(&pool, "both told", &keys, is_failed);
    assert!(!seen.iter().any(|e| matches!(e, Event::Body { .. })), "{seen:?}");
    std::thread::sleep(Duration::from_secs(3).saturating_sub(stopped.elapsed()));
    assert!(keys.iter().all(|k| !store.has_body(k)));
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

/// The account stops while the main connection is in a slow call, so the worker can't end yet:
/// the copy of the connection hears of it all the same, as an account removed must not get a
/// file written back meanwhile.
#[test]
fn the_stop_is_heard_during_a_slow_call() {
    let (mut d, _) = inbox_of(1);
    d.slow_sources = Some(Duration::from_secs(1));
    let server = serve(d);
    let store = crate::fake::tests_store("ews-text-first-slow-stop");
    let mut pool = start(&server, &store);
    let key = key_of(&store, "M0");
    open(&mut pool, &key);
    until(&pool, "the text", |e| is_text(e, &key));
    server.data.lock().unwrap().slow_counts = Some(Duration::from_secs(3));
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() });
    until(&pool, "the slow call", |e| matches!(e, Event::Status { status: SyncStatus::Syncing { folder }, .. } if folder == "INBOX"));
    pool.submit(Job::WorkOffline { on: true });
    let seen = until(&pool, "told", |e| is_failed(e, &key) || is_body(e, &key));
    assert!(matches!(seen.last(), Some(Event::BodyFailed { error, .. }) if error == "the account stopped"), "{:?}", seen.last());
    assert!(!store.has_body(&key));
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

/// A wrong password counts against the account at each try: a message opened tries once.
#[test]
fn a_refused_sign_in_is_tried_once_for_a_message() {
    let (d, _) = inbox_of(1);
    let server = serve(d);
    let store = crate::fake::tests_store("ews-text-first-sign-in");
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "riley@contoso.example".into(),
        username: format!("{DOMAIN}\\{USER}"),
        ews: Some(server.url.clone()),
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret("wrong".into())) });
    until(&pool, "sign in again", |e| matches!(e, Event::Status { status: SyncStatus::NeedsPassword { .. }, .. }));
    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: VALIDITY, uid: 1 };
    open(&mut pool, &key);
    let seen = until(&pool, "the failure", |e| is_failed(e, &key));
    let tries = seen.iter().filter(|e| matches!(e, Event::Status { status: SyncStatus::Connecting, .. })).count();
    assert_eq!(tries, 1, "{seen:?}");
    until(&pool, "asked for the password", |e| matches!(e, Event::Status { status: SyncStatus::NeedsPassword { .. }, .. }));
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

/// A password changed while the account is open: the text request is refused, and that is told as
/// the connection's failure. A second request for the whole message would cost a second refused
/// sign-in, which on Active Directory counts toward locking the account.
#[test]
fn a_password_changed_meanwhile_costs_one_refused_sign_in_for_a_message() {
    let (d, _) = inbox_of(1);
    let server = serve(d);
    let store = crate::fake::tests_store("ews-text-first-changed");
    let mut pool = start(&server, &store);
    let key = key_of(&store, "M0");
    {
        let mut d = server.data.lock().unwrap();
        d.password_changed = true;
        d.forget_sign_in = true;
    }
    // The sync's own requests meet the refusal too: only the message's are counted.
    let refused = || server.data.lock().unwrap().refused_sign_ins.iter().filter(|op| *op == "GetItem").count();
    open(&mut pool, &key);
    let seen = until(&pool, "the failure", |e| is_failed(e, &key));
    assert!(!seen.iter().any(|e| matches!(e, Event::TextFirst { .. } | Event::Body { .. })), "{seen:?}");
    assert_eq!(refused(), 1, "one request for the text, none for the whole message");
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

/// Only the source is written: a sync saving the folder can't be raced, and nothing tells of a
/// change to it.
#[test]
fn the_folder_list_stays_as_it_was() {
    let (mut d, _) = inbox_of(1);
    d.slow_sources = Some(Duration::from_millis(200));
    let server = serve(d);
    let store = crate::fake::tests_store("ews-text-first-list-kept");
    let mut pool = start(&server, &store);
    let key = key_of(&store, "M0");
    let version = store.folder_version("a1", "INBOX");
    open(&mut pool, &key);
    let seen = until(&pool, "the whole message", |e| is_body(e, &key));
    assert_eq!(store.folder_version("a1", "INBOX"), version);
    assert!(!seen.iter().any(|e| matches!(e, Event::FolderChanged { .. })), "{seen:?}");
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

/// Each message asked for as the one before arrives, as the thread is about to end: a key that
/// falls between its last look at the queue and its end would never download.
#[test]
fn a_message_opened_as_the_last_one_ends_still_comes() {
    let (d, _) = inbox_of(20);
    let server = serve(d);
    let store = crate::fake::tests_store("ews-text-first-relay");
    let mut pool = start(&server, &store);
    for i in 0..20 {
        let key = key_of(&store, &format!("M{i}"));
        open(&mut pool, &key);
        until(&pool, "the whole message", |e| is_body(e, &key));
    }
    assert_eq!(sources_asked(&server), 20);
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}
