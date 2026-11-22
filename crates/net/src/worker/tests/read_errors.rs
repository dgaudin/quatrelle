// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! What a worker keeps when the cache can't be read (the disk failing) or is damaged: a read that
//! fails is never taken for "nothing there", and nothing is cleared or written over on it.

use quatrelle_model::{ServerConfig, Summary};
use quatrelle_store::FolderState;

use super::*;

fn folder(path: &str) -> Folder {
    Folder { account: "a1".into(), path: path.into(), delimiter: Some('/'), selectable: true, ..Default::default() }
}

/// A folder `path` cached with `n` messages.
fn cache(store: &Store, path: &str, n: u32) {
    let messages = (1..=n)
        .map(|uid| Summary {
            key: MessageKey { account: "a1".into(), folder: path.into(), validity: 7, uid },
            subject: "kept".into(),
            ..Default::default()
        })
        .collect();
    store.save_folder("a1", path, &FolderState { validity: 7, uid_next: n + 1, messages, ..Default::default() }).unwrap();
}

/// The messages kept of `path`.
fn cached(store: &Store, path: &str) -> usize {
    store.load_folder("a1", path).map(|f| f.messages.len()).unwrap_or(0)
}

/// An IMAP account on the fake server at `port`.
fn imap_account(port: u16) -> AccountConfig {
    AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port, security: quatrelle_model::Security::None },
        ..Default::default()
    }
}

/// A worker for `acc`, its events in `pool`.
fn worker(acc: AccountConfig, secret: &str, store: &Store, pool: &Pool) -> Worker {
    Worker::new(acc, Auth::shared(secret), store.clone(), pool.sink.clone(), SyncOptions::default())
}

/// Found in review: a folder renamed here offline (Archive to Plans, not sent yet), not on the
/// server's list, lost its cache when the changes waiting, or the list on disk, couldn't be read:
/// taken for nothing waiting, and its mail was downloaded again. It keeps it till a listing can
/// tell.
#[test]
fn a_folder_kept_for_changes_waiting_keeps_its_cache_when_they_cant_be_read() {
    let st = crate::fake::State::new();
    let port = crate::fake::serve_tcp(&st);
    let store = Store::failing();
    store.save_folders("a1", &[folder("INBOX"), folder("Plans")]).unwrap();
    cache(&store, "Plans", 1);
    store.add_op("a1", Op::RenameFolder { from: "Archive".into(), to: "Plans".into() }).unwrap();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let mut w = worker(imap_account(port), "pw", &store, &pool);
    w.connect().unwrap();
    // The list on disk can't be read, then the changes waiting can't (the journal from before
    // is read first, there or not).
    for unreadable in ["folders.json", "ops.json"] {
        store.fail(Some(unreadable), None);
        assert!(w.relist());
        store.fail(None, None);
        assert_eq!(cached(&store, "Plans"), 1, "{unreadable}: the cache cleared");
        assert_eq!(w.gone_meanwhile, ["Plans"], "{unreadable}: looked at again at the next listing");
    }
    // Read, the list on disk (the server's now) doesn't have it: cleared, as ever.
    assert!(w.relist());
    assert_eq!(cached(&store, "Plans"), 0);
    assert!(w.gone_meanwhile.is_empty());
}

/// Found in review: the folder list read again (a folder made here) that couldn't be read left
/// the worker with no folders, and a POP account's wrote that over the list on disk. Its own
/// stays as it was.
#[test]
fn a_folder_list_that_cant_be_read_again_leaves_the_workers_as_it_was() {
    let store = Store::failing();
    store.save_folders("a1", &[folder("INBOX"), folder("Plans")]).unwrap();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let mut w = worker(AccountConfig { id: "a1".into(), ..Default::default() }, "pw", &store, &pool);
    store.fail(Some("folders.json"), None);
    let read = w.reload_folders();
    store.fail(None, None);
    assert_eq!(w.folders, [folder("INBOX"), folder("Plans")], "the worker's list emptied");
    assert!(!read);
    // Read, it's the list on disk, as ever.
    store.save_folders("a1", &[folder("INBOX")]).unwrap();
    assert!(w.reload_folders());
    assert_eq!(w.folders, [folder("INBOX")]);
}

/// Found in review: a JMAP account's message numbers were pruned against changes (or a folder)
/// that couldn't be read: the number of a message a change waited on was forgotten, and the
/// change could never be sent. Nothing is pruned then.
#[test]
fn jmap_numbers_are_kept_when_the_changes_cant_be_read() {
    use crate::fakes::fake_jmap::{Data, PASSWORD, USER, serve};
    let mut d = Data::default();
    d.mailbox("Inbox", None, Some("inbox"));
    let server = serve(d);
    let store = Store::failing();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let acc = AccountConfig { id: "a1".into(), email: USER.into(), username: USER.into(), jmap: Some(server.base.clone()), ..Default::default() };
    let mut w = worker(acc, PASSWORD, &store, &pool);
    w.connect().unwrap();
    w.folders = vec![folder("INBOX")];
    cache(&store, "INBOX", 1);
    // Message 2, moved here, is waiting to be marked read on the server.
    let (cached, waiting) = (w.jstate.number("M-cached"), w.jstate.number("M-waiting"));
    assert_eq!((cached, waiting), (1, 2));
    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 7, uid: waiting };
    store.add_op("a1", Op::SetFlag { keys: vec![key], flag: Flag::Seen, on: true }).unwrap();
    for unreadable in ["ops.json", "state.json"] {
        store.fail(Some(unreadable), None);
        w.prune_jmap_numbers();
        store.fail(None, None);
        assert_eq!(w.jstate.id(waiting).as_deref(), Some("M-waiting"), "{unreadable}: forgotten");
        assert_eq!(w.jstate.id(cached).as_deref(), Some("M-cached"), "{unreadable}: forgotten");
    }
    // Read, the numbers not used go, as ever.
    let gone = w.jstate.number("M-gone");
    w.prune_jmap_numbers();
    assert_eq!(w.jstate.id(waiting).as_deref(), Some("M-waiting"));
    assert_eq!(w.jstate.id(cached).as_deref(), Some("M-cached"));
    assert_eq!(w.jstate.id(gone), None);
}

/// Edits waiting that can't be read (the disk failing) were taken for none: an Exchange folder's
/// list was told as the server had it before they were made again on it. They may touch it.
#[test]
fn edits_waiting_that_cant_be_read_arent_taken_for_none() {
    let store = Store::failing();
    store.add_op("a1", Op::EmptyFolder { path: "Plans".into() }).unwrap();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let mut w = worker(AccountConfig { id: "a1".into(), ..Default::default() }, "pw", &store, &pool);
    assert!(w.waiting_in("Plans") && !w.waiting_in("INBOX"));
    store.fail(Some("ops.json"), None);
    let (waiting, again) = (w.waiting_in("INBOX"), w.reapply_waiting("INBOX"));
    store.fail(None, None);
    assert!(waiting, "taken for none waiting");
    assert!(again, "taken for none to make again");
    assert!(!w.reapply_waiting("INBOX"), "read, none touches it, as ever");
}

/// A folder gone from the server while a change to the folders waits is looked at once at each
/// listing, and kept for the next one once: a second look at the folders gone took each one
/// twice, and the list of those kept grew at every listing.
#[test]
fn a_folder_gone_is_looked_at_once_a_listing() {
    let st = crate::fake::State::new();
    st.lock().unwrap().boxes.push(crate::fake::Mailbox {
        name: "Other".into(),
        attr: "",
        validity: 500,
        uid_next: 1,
        msgs: Vec::new(),
        expunged: Vec::new(),
    });
    let port = crate::fake::serve_tcp(&st);
    let store = Store::failing();
    store.save_folders("a1", &[folder("INBOX"), folder("Other"), folder("Plans")]).unwrap();
    store.add_op("a1", Op::RenameFolder { from: "Archive".into(), to: "Plans".into() }).unwrap();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let mut w = worker(imap_account(port), "pw", &store, &pool);
    w.connect().unwrap();
    // Plans, renamed here, isn't on the server yet: kept for the next listing, once.
    assert!(w.relist());
    assert_eq!(w.gone_meanwhile, ["Plans"]);
    // Other goes from the server: kept in its turn, once; Plans, which the list on disk no
    // longer has, goes.
    st.lock().unwrap().boxes.retain(|b| b.name != "Other");
    assert!(w.relist());
    assert_eq!(w.gone_meanwhile, ["Other"]);
}

/// A folder made here whose list couldn't be read back left the worker with none; a message sent
/// next, with no Sent Items on the server, made one and wrote the worker's list (that folder
/// alone) over the list on disk: the folders before were gone from it. They stay.
#[test]
fn a_folder_change_with_an_unreadable_list_leaves_the_list_on_disk_whole() {
    let st = crate::fake::State::new();
    st.lock().unwrap().boxes.retain(|b| b.name != "Sent");
    let port = crate::fake::serve_tcp(&st);
    let (smtp, said) = crate::smtp::tests::serve_once();
    let store = Store::failing();
    let three = [folder("INBOX"), folder("Archive"), folder("Plans")];
    store.save_folders("a1", &three).unwrap();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let acc = AccountConfig {
        smtp: ServerConfig { host: "127.0.0.1".into(), port: smtp, security: quatrelle_model::Security::None },
        ..imap_account(port)
    };
    let mut w = worker(acc, "pw", &store, &pool);
    assert_eq!(w.folders, three);
    // A folder made here: in the cache and the journal (the pool's), its list not read back.
    let op = Op::CreateFolder { path: "Notes".into() };
    store.add_op("a1", op.clone()).unwrap();
    store.apply_op("a1", &op).unwrap();
    store.fail(Some("folders.json"), None);
    w.handle(Job::Apply { account: "a1".into(), op });
    store.fail(None, None);
    // A message sent: Sent Items made on the server, and the worker's list kept with it.
    let raw = b"From: me@studio.example\r\nTo: zoe@studio.example\r\nSubject: sent\r\nMessage-ID: <sent@studio.example>\r\n\r\nHi.\r\n";
    w.send(1, raw, "me@studio.example", &["zoe@studio.example".to_string()], None);
    assert!(said.join().unwrap().contains("Subject: sent"));
    let kept: Vec<String> = store.load_folders("a1").unwrap().into_iter().map(|f| f.path).collect();
    for f in &three {
        assert!(kept.contains(&f.path), "{} gone from the list on disk: {kept:?}", f.path);
    }
    assert!(kept.iter().any(|p| p == "Sent"), "{kept:?}");
}

/// The public folders followed couldn't be read, or were damaged, at a pass or at the start:
/// taken for none followed, a followed one kept here was let go of, and its mail downloaded
/// again once read. None is let go of then, and the damaged list stays as it is.
#[test]
fn followed_public_folders_that_cant_be_read_keep_their_cache() {
    use crate::fakes::fake_ews::{DOMAIN, Data, PASSWORD, USER, serve};
    let mut d = Data::default();
    d.pf_mailbox = Some("PFContent@contoso.example".into());
    d.pf_hierarchy = Some("PFHierarchy@contoso.example".into());
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let root = d.folder("IPM_SUBTREE", None, Some("publicfoldersroot"), "IPF.Note");
    d.folder("Reports", Some(&root), None, "IPF.Note");
    let server = serve(d);
    let store = Store::failing();
    let reports = "Public Folders/Reports";
    store.save_account_doc("a1", FOLLOWED_PUBLIC_DOC, &[reports]).unwrap();
    // More than a folder read on the server shows: what a followed one keeps.
    let kept = u32::try_from(ONLINE_WINDOW).unwrap() + 5;
    cache(&store, reports, kept);
    let kept = usize::try_from(kept).unwrap();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let acc = AccountConfig {
        id: "a1".into(),
        email: "riley@contoso.example".into(),
        username: format!("{DOMAIN}\\{USER}"),
        ews: Some(server.url.clone()),
        ..Default::default()
    };
    let listed = |mut w: Worker| {
        w.connect().unwrap();
        assert!(w.relist());
        assert!(w.folders.iter().any(|f| f.path == reports), "{:?}", w.folders);
        w
    };
    let mut w = listed(worker(acc.clone(), PASSWORD, &store, &pool));
    // At a pass: the list can't be read, then it's damaged.
    store.fail(Some(FOLLOWED_PUBLIC_DOC), None);
    w.read_followed();
    w.let_go_of_online();
    store.fail(None, None);
    assert_eq!(cached(&store, reports), kept, "unreadable: let go of");
    let damaged = b"[\"Public Folders/Rep".to_vec();
    store.write_account_files("a1", &[(FOLLOWED_PUBLIC_DOC.to_string(), damaged.clone())]).unwrap();
    w.read_followed();
    w.let_go_of_online();
    assert_eq!(cached(&store, reports), kept, "damaged: let go of");
    assert_eq!(store.account_file("a1", FOLLOWED_PUBLIC_DOC).unwrap(), Some(damaged.clone()), "the damaged list was set aside");
    // At the start: damaged, then unreadable.
    let mut w = listed(worker(acc.clone(), PASSWORD, &store, &pool));
    w.let_go_of_online();
    assert_eq!(cached(&store, reports), kept, "damaged at the start: let go of");
    store.fail(Some(FOLLOWED_PUBLIC_DOC), None);
    let w = worker(acc.clone(), PASSWORD, &store, &pool);
    store.fail(None, None);
    let mut w = listed(w);
    w.let_go_of_online();
    assert_eq!(cached(&store, reports), kept, "unreadable at the start: let go of");
    // Read, the folder no longer followed is let go of, as ever.
    store.save_account_doc("a1", FOLLOWED_PUBLIC_DOC, &Vec::<String>::new()).unwrap();
    w.read_followed();
    w.let_go_of_online();
    assert_eq!(cached(&store, reports), 0);
}
