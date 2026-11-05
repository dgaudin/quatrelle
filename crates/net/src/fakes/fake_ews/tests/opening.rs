// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Opening a message while the account's main connection is at work (issue #24): a folder's
//! sync, the calendars' and contacts'. The message comes at once, on a copy of the main
//! connection, and whole in the end, kept as any.

use std::sync::Arc;

use quatrelle_model::MessageKey;

use super::*;
use crate::worker::{MemoryCredentials, Pool};

/// A message with a text and a file.
fn with_a_file(subject: &str, id: &str) -> Vec<u8> {
    let bytes = crate::imap::base64(&vec![7u8; 30_000]);
    format!(
        "From: Zoe Park <zoe@contoso.example>\r\nTo: Riley <riley@contoso.example>\r\nSubject: {subject}\r\nMessage-ID: <{id}@x>\r\n\
         MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n\
         --b\r\nContent-Type: text/plain\r\n\r\nHere's the reel.\r\n\
         --b\r\nContent-Type: video/quicktime\r\nContent-Disposition: attachment; filename=reel.mov\r\nContent-Transfer-Encoding: base64\r\n\r\n{bytes}\r\n--b--\r\n"
    )
    .into_bytes()
}

/// An Inbox of two messages, an Archive of one, a calendar and contacts.
fn mailbox() -> Data {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.deliver(&inbox, &with_a_file("Big", "m1"), false, "2026-10-30T08:00:00Z");
    d.deliver(&inbox, &mail("Small", "m2"), false, "2026-10-29T08:00:00Z");
    let archive = d.folder("Archive", None, None, "IPF.Note");
    d.deliver(&archive, &mail("Old", "m3"), false, "2026-01-02T08:00:00Z");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    d
}

fn start(server: &super::super::server::Server, store: &Store) -> Pool {
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
    until(&pool, "the calendars", |e| matches!(e, Event::DavSynced { .. }));
    pool
}

fn key_of(store: &Store, subject: &str) -> MessageKey {
    store.load_folder("a1", "INBOX").unwrap().messages.iter().find(|m| m.subject == subject).unwrap().key.clone()
}

fn archive(store: &Store) -> String {
    store.load_folders("a1").unwrap().into_iter().find(|f| f.path.ends_with("Archive")).unwrap().path
}

/// How long each of `jobs`, asked together, takes to show its message: its text, or the whole.
fn shown_in(pool: &mut Pool, jobs: Vec<(Job, MessageKey)>) -> Vec<Duration> {
    let began = Instant::now();
    let mut took: Vec<Option<Duration>> = vec![None; jobs.len()];
    let keys: Vec<MessageKey> = jobs.iter().map(|(_, k)| k.clone()).collect();
    for (job, _) in jobs {
        pool.submit(job);
    }
    until(pool, "the messages shown", |e| {
        if let Event::Body { key, .. } | Event::TextFirst { key, .. } = e
            && let Some(i) = keys.iter().position(|k| k == key)
        {
            took[i].get_or_insert(began.elapsed());
        }
        took.iter().all(Option::is_some)
    });
    took.into_iter().flatten().collect()
}

/// Opened while the main connection reads a folder for 8 seconds: a small message whole, a big
/// one's text, each at once; the big one whole after, kept.
#[test]
fn a_message_opened_during_a_folders_sync_doesnt_wait_for_it() {
    let server = serve(mailbox());
    let store = crate::fake::tests_store("ews-busy-folder");
    let mut pool = start(&server, &store);
    let (big, small) = (key_of(&store, "Big"), key_of(&store, "Small"));
    assert!(!store.has_body(&big) && !store.has_body(&small));
    server.data.lock().unwrap().slow_counts = Some(Duration::from_secs(8));
    let path = archive(&store);
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: path.clone() });
    until(&pool, "the Archive's sync", |e| matches!(e, Event::Status { status: SyncStatus::Syncing { folder }, .. } if *folder == path));
    std::thread::sleep(Duration::from_millis(300));
    let took =
        shown_in(&mut pool, vec![(Job::FetchBody { key: small.clone() }, small.clone()), (Job::FetchTextFirst { key: big.clone() }, big.clone())]);
    let (small_in, big_in) = (took[0], took[1]);
    eprintln!("during a folder's sync: a small message in {small_in:?}, a big one's text in {big_in:?}");
    assert!(small_in < Duration::from_secs(3), "the small one waited for the Archive: {small_in:?}");
    assert!(big_in < Duration::from_secs(3), "the big one waited for the Archive: {big_in:?}");
    eventually("the big one whole, kept", || store.has_body(&big));
    assert!(store.has_body(&small), "kept too");
    server.data.lock().unwrap().slow_counts = None;
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

/// Opened while the main connection reads the calendars and contacts, 8 seconds.
#[test]
fn a_message_opened_during_the_calendars_sync_doesnt_wait_for_it() {
    let server = serve(mailbox());
    let store = crate::fake::tests_store("ews-busy-calendars");
    let mut pool = start(&server, &store);
    let small = key_of(&store, "Small");
    server.data.lock().unwrap().slow_pim = Some(Duration::from_secs(8));
    pool.submit(Job::SyncDav { account: "a1".into() });
    std::thread::sleep(Duration::from_millis(500));
    let small_in = shown_in(&mut pool, vec![(Job::FetchBody { key: small.clone() }, small.clone())])[0];
    eprintln!("during the calendars' sync: a small message in {small_in:?}");
    assert!(small_in < Duration::from_secs(3), "it waited for the calendars: {small_in:?}");
    assert!(store.has_body(&small), "kept");
    server.data.lock().unwrap().slow_pim = None;
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

/// The main connection reads the Archive for `hold`: its sync is under way when this returns.
fn busy(server: &super::super::server::Server, pool: &mut Pool, store: &Store, hold: Duration) {
    server.data.lock().unwrap().slow_counts = Some(hold);
    let path = archive(store);
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: path.clone() });
    until(pool, "the Archive's sync", |e| matches!(e, Event::Status { status: SyncStatus::Syncing { folder }, .. } if *folder == path));
    std::thread::sleep(Duration::from_millis(300));
}

/// A password changed in Active Directory while the account is open: the main connection stays
/// signed in, a copy of it is refused. That is paid once for the session: the messages come
/// whole on the main connection once its work is done, and no copy is tried again.
#[test]
fn a_copy_refused_during_a_sync_costs_one_refused_sign_in() {
    let server = serve(mailbox());
    let store = crate::fake::tests_store("ews-busy-refused");
    let mut pool = start(&server, &store);
    let (big, small) = (key_of(&store, "Big"), key_of(&store, "Small"));
    {
        let mut d = server.data.lock().unwrap();
        d.password_changed = true;
        d.forget_sign_in = false;
    }
    let refused = || server.data.lock().unwrap().refused_sign_ins.len();
    busy(&server, &mut pool, &store, Duration::from_secs(3));
    let took =
        shown_in(&mut pool, vec![(Job::FetchBody { key: small.clone() }, small.clone()), (Job::FetchTextFirst { key: big.clone() }, big.clone())]);
    eprintln!("a copy refused: shown in {took:?}, {} refused sign-ins", refused());
    assert_eq!(refused(), 1, "{:?}", server.data.lock().unwrap().refused_sign_ins);
    eventually("both kept", || store.has_body(&big) && store.has_body(&small));
    // Another sync, another message: the main connection brings it, no copy is tried.
    let other = store.load_folder("a1", &archive(&store)).unwrap().messages[0].key.clone();
    busy(&server, &mut pool, &store, Duration::from_secs(2));
    shown_in(&mut pool, vec![(Job::FetchBody { key: other.clone() }, other)]);
    assert_eq!(refused(), 1, "{:?}", server.data.lock().unwrap().refused_sign_ins);
    server.data.lock().unwrap().slow_counts = None;
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

/// A public folder's message and a shared mailbox's, opened while the main connection works:
/// the copy carries the routes the main connection learnt.
#[test]
fn a_public_folder_and_a_shared_mailbox_during_a_sync() {
    let mut d = Data {
        pf_mailbox: Some("PFContent@contoso.example".into()),
        pf_hierarchy: Some("PFHierarchy@contoso.example".into()),
        anchored_items: true,
        ..mailbox()
    };
    let root = d.folder("IPM_SUBTREE", None, Some("publicfoldersroot"), "IPF.Note");
    let news = d.folder("Announcements", Some(&root), None, "IPF.Note");
    d.deliver(&news, &with_a_file("Holiday hours", "h"), false, "2026-10-14T09:00:00Z");
    let sales =
        d.shared_mailbox(SharedBox { address: "sales@contoso.example".into(), name: "Sales".into(), automap: true, legacy: false, granted: true });
    d.deliver(&sales, &with_a_file("Order", "s"), false, "2026-10-15T09:00:00Z");
    let server = serve(d);
    let store = crate::fake::tests_store("ews-busy-routes");
    let mut pool = start(&server, &store);
    let folders = store.load_folders("a1").unwrap();
    let public = folders.iter().find(|f| f.path.ends_with("Announcements")).map(|f| f.path.clone()).unwrap();
    let shared = folders.iter().find(|f| f.mailbox.is_some() && f.path.ends_with("/Inbox")).map(|f| f.path.clone()).unwrap();
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: public.clone() });
    let listed = |path: &str| store.load_folder("a1", path).ok().and_then(|f| f.messages.first().map(|m| m.key.clone()));
    eventually("listed when opened", || listed(&public).is_some() && listed(&shared).is_some());
    let keys = [listed(&public).unwrap(), listed(&shared).unwrap()];
    busy(&server, &mut pool, &store, Duration::from_secs(8));
    let took = shown_in(&mut pool, keys.iter().map(|k| (Job::FetchTextFirst { key: k.clone() }, k.clone())).collect());
    eprintln!("a public folder's and a shared mailbox's messages during a sync: their texts in {took:?}");
    assert!(took.iter().all(|t| *t < Duration::from_secs(3)), "{took:?}");
    eventually("both kept", || keys.iter().all(|k| store.has_body(k)));
    let routes = server.data.lock().unwrap().source_routes.clone();
    for mailbox in ["PFContent@contoso.example", "sales@contoso.example"] {
        assert!(routes.iter().any(|r| r == mailbox), "{mailbox}: {routes:?}");
    }
    server.data.lock().unwrap().slow_counts = None;
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}
