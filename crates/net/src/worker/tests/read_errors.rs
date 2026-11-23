// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! What a worker keeps when the cache can't be read (the disk failing) or is damaged: a read that
//! fails is never taken for "nothing there", and nothing is cleared or written over on it.

use quatrelle_model::{ServerConfig, Summary};
use quatrelle_store::FolderState;

use super::*;

pub(super) fn folder(path: &str) -> Folder {
    Folder { account: "a1".into(), path: path.into(), delimiter: Some('/'), selectable: true, ..Default::default() }
}

/// A folder `path` cached with `n` messages.
pub(super) fn cache(store: &Store, path: &str, n: u32) {
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
pub(super) fn cached(store: &Store, path: &str) -> usize {
    store.load_folder("a1", path).map(|f| f.messages.len()).unwrap_or(0)
}

/// An IMAP account on the fake server at `port`.
pub(super) fn imap_account(port: u16) -> AccountConfig {
    AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port, security: quatrelle_model::Security::None },
        ..Default::default()
    }
}

/// A worker for `acc`, its events in `pool`.
pub(super) fn worker(acc: AccountConfig, secret: &str, store: &Store, pool: &Pool) -> Worker {
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

/// A folder gone from the server whose cache can't be cleared (the disk failing) is looked at
/// again at the next listing: passed over, it kept its cache for good.
#[test]
fn a_folder_gone_whose_cache_cant_be_cleared_is_looked_at_again() {
    let st = crate::fake::State::new();
    let port = crate::fake::serve_tcp(&st);
    let store = Store::failing();
    store.save_folders("a1", &[folder("INBOX"), folder("Plans")]).unwrap();
    cache(&store, "Plans", 1);
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let mut w = worker(imap_account(port), "pw", &store, &pool);
    w.connect().unwrap();
    // The folder's own place in the cache, which clearing it takes away.
    let key = MessageKey { account: "a1".into(), folder: "Plans".into(), validity: 7, uid: 1 };
    let body = store.body_path(&key);
    let place = body.parent().and_then(std::path::Path::parent).and_then(std::path::Path::file_name).unwrap().to_string_lossy().into_owned();
    store.fail(None, Some(&place));
    assert!(w.relist());
    store.fail(None, None);
    assert_eq!(cached(&store, "Plans"), 1);
    assert_eq!(w.gone_meanwhile, ["Plans"], "passed over, its cache left for good");
    // It can be now: cleared, as ever.
    assert!(w.relist());
    assert_eq!(cached(&store, "Plans"), 0);
    assert!(w.gone_meanwhile.is_empty());
}

/// What the app syncs with: every message kept (`SetSync { messages: 0 }`), of any age.
fn app_options() -> SyncOptions {
    SyncOptions { window: usize::MAX, ..SyncOptions::default() }
}

/// An Exchange account whose public folder `Reports` holds `n` messages on the server.
fn public_reports(n: usize) -> (crate::fakes::fake_ews::Server, AccountConfig) {
    use crate::fakes::fake_ews::{DOMAIN, Data, USER, serve};
    let mut d = Data::default();
    d.pf_mailbox = Some("PFContent@contoso.example".into());
    d.pf_hierarchy = Some("PFHierarchy@contoso.example".into());
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let root = d.folder("IPM_SUBTREE", None, Some("publicfoldersroot"), "IPF.Note");
    let reports = d.folder("Reports", Some(&root), None, "IPF.Note");
    for i in 0..n {
        let raw = format!("From: backup@contoso.example\r\nSubject: Report {i}\r\nMessage-ID: <r{i}@contoso.example>\r\n\r\nDone.\r\n");
        d.deliver(&reports, raw.as_bytes(), true, &format!("2026-09-{:02}T09:{:02}:00Z", 1 + i % 28, i % 60));
    }
    let server = serve(d);
    let acc = AccountConfig {
        id: "a1".into(),
        email: "riley@contoso.example".into(),
        username: format!("{DOMAIN}\\{USER}"),
        ews: Some(server.url.clone()),
        ..Default::default()
    };
    (server, acc)
}

/// A worker for `acc` with the app's options, connected, its folders listed: the list of public
/// folders followed can't be read as it starts (and after, till `store.fail(None, None)`).
fn started_unread(acc: &AccountConfig, store: &Store, pool: &Pool) -> Worker {
    use crate::fakes::fake_ews::PASSWORD;
    store.fail(Some(FOLLOWED_PUBLIC_DOC), None);
    let mut w = Worker::new(acc.clone(), Auth::shared(PASSWORD), store.clone(), pool.sink.clone(), app_options());
    w.connect().unwrap();
    assert!(w.relist());
    w
}

/// The public folders followed can't be read at the start, nor when one of them is opened: taken
/// for a folder read on the server, it was listed as an opened one is, its newest few, and what
/// it kept beyond them let go of with their sources (downloaded again once the list was read).
/// It's listed with nothing let go of, and not as one followed; read again, one not followed is
/// listed as an opened one is, as ever.
#[test]
fn a_public_folder_opened_while_those_followed_cant_be_read_keeps_its_mail() {
    let n = ONLINE_WINDOW + 5;
    let (_server, acc) = public_reports(n);
    let store = Store::failing();
    let path = "Public Folders/Reports";
    store.save_account_doc("a1", FOLLOWED_PUBLIC_DOC, &[path]).unwrap();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let sources = || store.load_folder("a1", path).map(|f| f.messages.iter().filter(|m| store.has_body(&m.key)).count()).unwrap_or(0);
    // Followed: kept, with its sources.
    let mut w = Worker::new(acc.clone(), Auth::shared(crate::fakes::fake_ews::PASSWORD), store.clone(), pool.sink.clone(), app_options());
    w.connect().unwrap();
    assert!(w.relist());
    w.sync_one(path);
    assert_eq!((cached(&store, path), sources()), (n, n));
    // The list can't be read at the start, nor when the folder is opened.
    let mut w = started_unread(&acc, &store, &pool);
    w.handle(Job::SyncFolder { account: "a1".into(), folder: path.into() });
    store.fail(None, None);
    assert_eq!((cached(&store, path), sources()), (n, n), "let go of, with their sources");
    // Read, and not followed: listed as an opened one is, its newest few.
    store.save_account_doc("a1", FOLLOWED_PUBLIC_DOC, &Vec::<String>::new()).unwrap();
    w.handle(Job::SyncFolder { account: "a1".into(), folder: path.into() });
    assert_eq!(cached(&store, path), ONLINE_WINDOW);
}

/// The public folders followed can't be read: a public folder not followed, kept nowhere here,
/// was listed whole when opened (the app keeps every message: up to 100,000 of any age at each
/// opening). It's listed as an opened one is, its newest few.
#[test]
fn a_public_folder_not_followed_opened_while_those_followed_cant_be_read_lists_its_newest_few() {
    let (_server, acc) = public_reports(ONLINE_WINDOW + 5);
    let store = Store::failing();
    let path = "Public Folders/Reports";
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let mut w = started_unread(&acc, &store, &pool);
    w.handle(Job::SyncFolder { account: "a1".into(), folder: path.into() });
    store.fail(None, None);
    assert_eq!(cached(&store, path), ONLINE_WINDOW, "listed whole");
}

/// Show more on a public folder while those followed can't be read: what it was to keep was
/// written as a million (the window of a folder kept whole), kept after: the folder, not
/// followed once the list was read, was listed whole at each opening and never let go of. It's
/// what it lists, and the few asked for.
#[test]
fn show_more_while_those_followed_cant_be_read_asks_for_a_few_more() {
    let (_server, acc) = public_reports(ONLINE_WINDOW + 5);
    let store = Store::failing();
    let path = "Public Folders/Reports";
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let mut w = started_unread(&acc, &store, &pool);
    w.handle(Job::MoreMail { account: "a1".into(), folder: path.into(), more: 10 });
    store.fail(None, None);
    assert_eq!(w.windows.get(path).copied(), Some(ONLINE_WINDOW + 10), "what it is to keep");
}

/// An Exchange account with two folders on the server, two messages in each.
fn two_folders() -> (crate::fakes::fake_ews::Server, AccountConfig) {
    use crate::fakes::fake_ews::{DOMAIN, Data, USER, serve};
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let plans = d.folder("Plans", None, None, "IPF.Note");
    for (folder, name) in [(&plans, "plan"), (&inbox, "mail")] {
        for i in 0..2 {
            let raw = format!(
                "From: zoe@studio.example\r\nTo: riley@contoso.example\r\nSubject: {name} {i}\r\nMessage-ID: <{name}{i}@studio.example>\r\n\r\nHi.\r\n"
            );
            d.deliver(folder, raw.as_bytes(), false, &format!("2026-10-1{i}T09:00:00Z"));
        }
    }
    let server = serve(d);
    let acc = AccountConfig {
        id: "a1".into(),
        email: "riley@contoso.example".into(),
        username: format!("{DOMAIN}\\{USER}"),
        ews: Some(server.url.clone()),
        ..Default::default()
    };
    (server, acc)
}

/// What the server gave of its messages so far (details or sources).
fn downloads(server: &crate::fakes::fake_ews::Server) -> usize {
    server.data.lock().unwrap().given_from.len()
}

/// An account read in full by a worker that is then let go: Plans first (its messages take the
/// numbers 1 and 2, the Inbox's 3 and 4: a numbering made again would give the Inbox's other
/// ones), a source in the Inbox downloaded.
struct Read {
    server: crate::fakes::fake_ews::Server,
    acc: AccountConfig,
    store: Store,
    pool: Pool,
    /// An Inbox message, its server id, and how many messages the server gave by then.
    key: MessageKey,
    id: String,
    given: usize,
}

fn read_in_full() -> Read {
    let (server, acc) = two_folders();
    let store = Store::failing();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let mut w = worker(acc.clone(), crate::fakes::fake_ews::PASSWORD, &store, &pool);
    w.connect().unwrap();
    assert!(w.relist());
    w.sync_one("Plans");
    w.sync_one("INBOX");
    let key = store.load_folder("a1", "INBOX").unwrap().messages.first().map(|m| m.key.clone()).unwrap();
    assert!(key.uid > 2, "the Inbox's numbers come after Plans': {}", key.uid);
    assert!(store.has_body(&key), "a source of the Inbox is here");
    let id = w.jstate.id(key.uid).unwrap();
    let given = downloads(&server);
    Read { server, acc, store, pool, key, id, given }
}

impl Read {
    /// A new worker for the account, as when Quatrelle starts again.
    fn restarted(&self) -> Worker {
        worker(self.acc.clone(), crate::fakes::fake_ews::PASSWORD, &self.store, &self.pool)
    }

    /// Nothing of the account was numbered again, nor downloaded again.
    fn assert_as_it_was(&self, why: &str) {
        let uids: Vec<u32> = self.store.load_folder("a1", "INBOX").unwrap().messages.iter().map(|m| m.key.uid).collect();
        assert!(uids.contains(&self.key.uid), "{why}: the Inbox's message was numbered again: {uids:?}");
        assert!(self.store.has_body(&self.key), "{why}: its source went");
        assert_eq!(downloads(&self.server), self.given, "{why}: the server gave messages again");
    }
}

/// The account's numbering (`jmap.json`) that can't be read was taken for an empty one: at the
/// first folder read, every message got a new number, the cache kept under the old ones was taken
/// for gone and the whole account was downloaded again (an old number could even stand for
/// another message). It's set aside, as it is: nothing is asked of the server, listed, read or
/// written.
#[test]
fn a_numbering_that_cant_be_read_is_never_started_over() {
    let read = read_in_full();
    read.store.fail(Some("jmap.json"), None);
    let mut w = read.restarted();
    // What a worker that connected would go on to do.
    let connected = w.connect();
    if connected.is_ok() {
        w.relist();
        w.sync_one("INBOX");
    }
    read.store.fail(None, None);
    read.assert_as_it_was("unreadable");
    assert!(matches!(&connected, Err(NetError::Refused(m)) if m.contains("numbering")), "{connected:?}");
    assert!(w.conn.is_none());
    // Nothing is listed or read while it can't be.
    assert!(!w.relist());
    w.sync_one("INBOX");
    w.prune_jmap_numbers();
    w.let_go_of_online();
    read.assert_as_it_was("set aside");
    assert_eq!(downloads(&read.server), read.given);
}

/// Read again, the numbering is the account's again: its numbers, its cache and its sources are
/// what they were.
#[test]
fn a_numbering_read_again_lets_the_account_go_on() {
    let read = read_in_full();
    read.store.fail(Some("jmap.json"), None);
    let mut w = read.restarted();
    assert!(w.connect().is_err());
    read.store.fail(None, None);
    w.connect().unwrap();
    assert!(w.relist());
    w.sync_one("INBOX");
    assert_eq!(w.jstate.id(read.key.uid).as_deref(), Some(read.id.as_str()));
    read.assert_as_it_was("read again");
}

/// A damaged numbering stays where it is, byte for byte (it was set aside as `jmap.json.bad`
/// and started over): the way out is the user's.
#[test]
fn a_damaged_numbering_stays_where_it_is_and_sets_the_account_aside() {
    let read = read_in_full();
    let damaged = b"{\"next\":4,\"ids\":[".to_vec();
    read.store.write_account_files("a1", &[("jmap.json".to_string(), damaged.clone())]).unwrap();
    let mut w = read.restarted();
    let connected = w.connect();
    if connected.is_ok() {
        w.relist();
        w.sync_one("INBOX");
    }
    assert_eq!(read.store.account_file("a1", "jmap.json").unwrap(), Some(damaged), "the file was replaced");
    assert_eq!(read.store.account_file("a1", "jmap.json.bad").unwrap(), None, "the file was set aside");
    read.assert_as_it_was("damaged");
    assert!(matches!(&connected, Err(NetError::Refused(m)) if m.contains("numbering") && m.contains("jmap.json")), "{connected:?}");
    // Tried again at each connection, and still refused.
    assert!(matches!(w.connect(), Err(NetError::Refused(_))));
    assert_eq!(read.store.account_file("a1", "jmap.json.bad").unwrap(), None);
}

/// A numbering that wasn't read is never saved over the one that couldn't be: the guard of an
/// empty one (`State::save`) lapses at the first number given.
#[test]
fn nothing_is_saved_over_a_numbering_that_wasnt_read() {
    let read = read_in_full();
    read.store.fail(Some("jmap.json"), None);
    let mut w = read.restarted();
    w.jstate.number("x");
    w.save_jstate();
    read.store.fail(None, None);
    let kept = jmap::State::load(&read.store, "a1").unwrap();
    assert_eq!(kept.id(read.key.uid).as_deref(), Some(read.id.as_str()), "the numbering on disk was written over");
    assert_eq!(kept.known("x"), None);
}

/// The second connection has no numbering: it sends whatever `jmap.json` is.
#[test]
fn the_second_connection_still_sends_without_the_numbering() {
    let read = read_in_full();
    let damaged = b"{\"next\":4,\"ids\":[".to_vec();
    read.store.write_account_files("a1", &[("jmap.json".to_string(), damaged.clone())]).unwrap();
    let auth = Auth::shared(crate::fakes::fake_ews::PASSWORD);
    let mut q = Worker::with_numbering(read.acc.clone(), auth, read.store.clone(), read.pool.sink.clone(), SyncOptions::default(), false);
    assert!(q.numbering_unread.is_none());
    let raw = b"From: riley@contoso.example\r\nTo: zoe@studio.example\r\nSubject: sent\r\nMessage-ID: <sent@studio.example>\r\n\r\nHi.\r\n";
    q.deliver(raw, "riley@contoso.example", &["zoe@studio.example".to_string()], None).unwrap();
    assert_eq!(read.server.data.lock().unwrap().sent.len(), 1);
    assert_eq!(read.store.account_file("a1", "jmap.json").unwrap(), Some(damaged), "read or written by the second connection");
}

/// A JMAP message is filed in the Sent folder by its number: with the numbering unread, it waits
/// in the Outbox, asking nothing of the server, rather than failing for want of a Sent folder; read
/// again, the numbering is the account's again.
#[test]
fn a_jmap_message_waits_in_the_outbox_while_the_numbering_cant_be_read() {
    use crate::fakes::fake_jmap::{Data, PASSWORD, USER, serve};
    let mut d = Data::default();
    d.mailbox("Inbox", None, Some("inbox"));
    d.mailbox("Sent", None, Some("sent"));
    let server = serve(d);
    let store = Store::failing();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let acc = AccountConfig { id: "a1".into(), email: USER.into(), username: USER.into(), jmap: Some(server.base.clone()), ..Default::default() };
    let mut w = worker(acc.clone(), PASSWORD, &store, &pool);
    w.connect().unwrap();
    assert!(w.relist());
    // A numbering with a message in it is kept (an empty one is not: `State::save`).
    w.jstate.number("M-kept");
    w.save_jstate();
    drop(w);
    store.fail(Some("jmap.json"), None);
    let mut q = worker(acc, PASSWORD, &store, &pool);
    let raw = b"From: me@studio.example\r\nTo: zoe@studio.example\r\nSubject: kept\r\nMessage-ID: <kept@studio.example>\r\n\r\nHi.\r\n";
    q.send(7, raw, USER, &["zoe@studio.example".to_string()], Some("Sent".into()));
    store.fail(None, None);
    assert_eq!(store.outbox("a1").iter().map(|i| i.id).collect::<Vec<_>>(), [7], "not kept in the Outbox");
    assert!(server.data.lock().unwrap().submissions.is_empty(), "asked of the server");
    // Read again: it goes.
    q.send(8, raw, USER, &["zoe@studio.example".to_string()], Some("Sent".into()));
    assert_eq!(server.data.lock().unwrap().submissions.len(), 1, "not sent once the numbering was read");
}
