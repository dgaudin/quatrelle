// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! What an account's worker writes once its account was removed: nothing. A removal waits for a
//! write being made ([`Store::while_kept`]), and a worker started before it writes nothing after it:
//! the account's files, and its folder with them, would come back.

use super::*;
use crate::fakes::fake_ews::{Data, PASSWORD, serve};
use quatrelle_model::ServerConfig;

/// An Exchange account whose server Autodiscover finds (the fake's, standing for the address's own
/// domain by `email`, see `TEST_BASES`), on a site it trusts.
fn discovering(email: &str, server_url: &str) -> AccountConfig {
    crate::ews::TEST_BASES.lock().unwrap().push((email.into(), vec![server_url.into()]));
    AccountConfig {
        id: "a1".into(),
        email: email.into(),
        username: format!("{}\\{}", crate::fakes::fake_ews::DOMAIN, crate::fakes::fake_ews::USER),
        ews: Some(crate::ews::AUTODISCOVER.into()),
        trusted_domains: vec!["127.0.0.1".into()],
        ..Default::default()
    }
}

fn folder(path: &str) -> Folder {
    Folder { account: "a1".into(), path: path.into(), selectable: true, ..Default::default() }
}

/// Found in the audit: the folder list was written once a look said the account was still there,
/// so a removal coming between the look and the write brought its `folders.json` back (and the
/// account's folder with it). Written while it's kept, it isn't, and it isn't told either.
#[test]
fn a_folder_list_published_as_its_account_is_removed_writes_nothing() {
    let store = crate::fake::store("removed-folder-list");
    store.save_folders("a1", &[folder("INBOX")]).unwrap();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let mut w = Worker::new(
        AccountConfig { id: "a1".into(), ..Default::default() },
        Auth::shared("pw"),
        store.clone(),
        pool.sink.clone(),
        SyncOptions::default(),
    );
    w.folders = vec![folder("Archive")];
    // Removed after the worker decided to write its list, before it's written.
    let removing = store.clone();
    crate::worker::sync::BEFORE_FOLDERS_SAVED.with(|b| *b.borrow_mut() = Some(Box::new(move || drop(removing.remove_account("a1")))));
    w.publish_folders();
    assert!(store.load_folders("a1").unwrap_or_default().is_empty(), "the folder list written again");
    assert!(!store.root().join("a1").exists(), "the account's files came back");
    assert!(!pool.poll().into_iter().any(|e| matches!(e, Event::Folders { .. })), "its folders said");
}

/// The list of a worker that couldn't read the one on disk (held in memory only, `folders_unread`)
/// is no more told, or held, once its account is gone.
#[test]
fn an_unread_folder_list_of_a_removed_account_is_neither_held_nor_told() {
    let store = crate::fake::store("removed-folder-list-unread");
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let mut w = Worker::new(
        AccountConfig { id: "a1".into(), ..Default::default() },
        Auth::shared("pw"),
        store.clone(),
        pool.sink.clone(),
        SyncOptions::default(),
    );
    w.folders_unread = true;
    w.folders = vec![Folder { archive: true, ..folder("Archive") }];
    store.remove_account("a1").unwrap();
    w.publish_folders();
    assert!(!pool.poll().into_iter().any(|e| matches!(e, Event::Folders { .. })), "its folders said");
    assert!(store.load_folders("a1").unwrap_or_default().is_empty(), "its folders held in the account's place");
}

/// A POP account's worker whose account was removed since it started.
fn removed_pop_worker(name: &str) -> (Store, Worker) {
    let store = crate::fake::store(name);
    store.save_folders("p1", &[Folder { account: "p1".into(), path: "INBOX".into(), selectable: true, ..Default::default() }]).unwrap();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let pop = ServerConfig { host: "pop.studio.example".into(), port: 995, security: quatrelle_model::Security::Tls };
    let acc = AccountConfig { id: "p1".into(), email: "me@studio.example".into(), pop: Some(pop), ..Default::default() };
    let w = Worker::new(acc, Auth::shared("pw"), store.clone(), pool.sink.clone(), SyncOptions::default());
    store.remove_account("p1").unwrap();
    (store, w)
}

/// Found in the audit: a change made here to a POP account (a folder made: its worker makes it, as
/// there's no server copy), handled by a worker whose account was removed since it started, wrote
/// the change and the change queued: the account's files came back. Nothing is written.
#[test]
fn changes_handled_after_their_account_was_removed_write_nothing() {
    let (store, mut w) = removed_pop_worker("removed-changes");
    w.handle(Job::Apply { account: "p1".into(), op: Op::CreateFolder { path: "Plans".into() } });
    assert!(!store.root().join("p1").exists(), "a change made brought the account's files back");
    assert!(store.first_op("p1").ok().flatten().is_none(), "a change queued for an account gone");
}

/// The same for Show more: the folder's window isn't written for an account removed since.
#[test]
fn a_folders_window_asked_for_after_its_account_was_removed_isnt_written() {
    let (store, mut w) = removed_pop_worker("removed-window");
    w.handle(Job::MoreMail { account: "p1".into(), folder: "INBOX".into(), more: 25 });
    assert!(!store.root().join("p1").exists(), "Show more brought the account's files back");
}

/// Found in the audit: the Exchange server autodiscovery found was kept (`ews.json`) whatever
/// happened meanwhile, so an account removed while autodiscovery went on had its folder back.
#[test]
fn a_server_autodiscovered_as_its_account_is_removed_isnt_kept() {
    let server = serve(Data::default());
    let store = crate::fake::store("removed-autodiscovered");
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let mut w = Worker::new(
        discovering("found@studio.example", &server.url),
        Auth::shared(PASSWORD),
        store.clone(),
        pool.sink.clone(),
        SyncOptions::default(),
    );
    // Found, then removed before it's kept.
    let removing = store.clone();
    let then: Box<dyn FnOnce()> = Box::new(move || drop(removing.remove_account("a1")));
    crate::worker::connect::AUTODISCOVER_IN_TEST.with(|a| *a.borrow_mut() = Some(then));
    let _ = w.connect();
    assert_eq!(store.load_account_doc::<String>("a1", "ews.json").unwrap_or_default(), "", "the server found kept");
    assert!(!store.root().join("a1").exists(), "the account's files came back");
}

/// A worker started, then its account removed, then it connects: the removal came before the
/// connection, so what it finds isn't kept. (The count the worker was made at is the one it keeps
/// the server under: a count read again when it connects said nothing had happened.)
#[test]
fn a_worker_started_then_its_account_removed_writes_nothing_at_connect() {
    let server = serve(Data::default());
    let store = crate::fake::store("removed-before-connect");
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let mut w = Worker::new(
        discovering("before@studio.example", &server.url),
        Auth::shared(PASSWORD),
        store.clone(),
        pool.sink.clone(),
        SyncOptions::default(),
    );
    store.remove_account("a1").unwrap();
    let _ = w.connect();
    assert_eq!(store.load_account_doc::<String>("a1", "ews.json").unwrap_or_default(), "", "the server found kept");
    assert!(!store.root().join("a1").exists(), "the account's files came back");
}

/// The same for a reader connection a worker started (`sync_folders`): it takes the count its
/// worker had, not the store's at the time it connects.
#[test]
fn a_reader_started_then_its_account_removed_writes_nothing_at_connect() {
    let server = serve(Data::default());
    let store = crate::fake::store("removed-reader");
    let acc = discovering("reader@studio.example", &server.url);
    let seen = store.removals("a1");
    store.remove_account("a1").unwrap();
    let (tx, rx) = mpsc::channel();
    let queue = Arc::new(Mutex::new(VecDeque::new()));
    crate::worker::connect::sync_folders(acc, store.clone(), PASSWORD.into(), jmap::State::default(), queue, tx, seen);
    drop(rx);
    assert_eq!(store.load_account_doc::<String>("a1", "ews.json").unwrap_or_default(), "", "the server found kept");
    assert!(!store.root().join("a1").exists(), "the account's files came back");
}
