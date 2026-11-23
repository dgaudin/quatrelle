// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What a worker does with a file of the account's that is there but damaged (its bytes aren't
//! what it keeps): a change waiting is never passed over, a folder list never read as none.

use quatrelle_model::Flags;
use quatrelle_store::StoreError;

use super::read_errors::{cache, cached, folder, imap_account, worker};
use super::zones;
use super::*;
use crate::fakes::fake_ews;

/// The fake server's message `uid` of `mailbox`, with its flags; `None` when it isn't there.
fn on_server(st: &Arc<Mutex<crate::fake::State>>, mailbox: &str, uid: u32) -> Option<Vec<String>> {
    let st = st.lock().unwrap();
    st.boxes.iter().find(|b| b.name == mailbox)?.msgs.iter().find(|m| m.uid == uid).map(|m| m.flags.clone())
}

/// The commands the fake server was sent that start with `verb` (after `UID`).
fn sent(st: &Arc<Mutex<crate::fake::State>>, verb: &str) -> usize {
    let st = st.lock().unwrap();
    st.log.iter().filter(|l| l.to_ascii_uppercase().trim_start_matches("UID ").starts_with(verb)).count()
}

/// Found in review: a change waiting whose file was damaged was passed over, and the ones made
/// after it were sent as if it had never been made (here a message moved, after a change to it
/// that's lost); nothing said so. It and every change made after it are kept aside, as they are,
/// and said; the one before it goes, once.
#[test]
fn a_damaged_change_waiting_is_never_skipped_over() {
    let st = crate::fake::State::new();
    let (a, b) = {
        let mut s = st.lock().unwrap();
        (s.add("INBOX", "Plans", &[]), s.add("INBOX", "Budget", &[]))
    };
    let port = crate::fake::serve_tcp(&st);
    let store = Store::failing();
    let key = |uid| MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 100, uid };
    store.add_op("a1", Op::SetFlag { keys: vec![key(a)], flag: Flag::Seen, on: true, all: false }).unwrap();
    store.add_op("a1", Op::SetFlag { keys: vec![key(b)], flag: Flag::Flagged, on: true, all: false }).unwrap();
    store.add_op("a1", Op::Move { keys: vec![key(b)], to: "Archive".into() }).unwrap();
    let mut names = store.account_files("a1", "ops").unwrap();
    names.sort();
    assert_eq!(names.len(), 3);
    let damaged = b"{\"op\":\"setFlag\",\"keys\":[{\"acc".to_vec();
    store.write_account_files("a1", &[(format!("ops/{}", names[1]), damaged.clone())]).unwrap();
    let third = store.account_file("a1", &format!("ops/{}", names[2])).unwrap().unwrap();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let mut w = worker(imap_account(port), "pw", &store, &pool);
    w.connect().unwrap();
    pool.poll();
    w.replay();
    assert_eq!(on_server(&st, "INBOX", b), Some(Vec::new()), "moved on the server past a change that couldn't be read");
    assert_eq!(sent(&st, "MOVE"), 0, "a change after it was sent");
    assert!(on_server(&st, "INBOX", a).is_some_and(|f| f.iter().any(|f| f == "\\Seen")), "the one before it wasn't sent");
    let told: Vec<usize> =
        pool.poll().into_iter().filter_map(|e| if let Event::OpsSetAside { count, .. } = e { Some(count) } else { None }).collect();
    assert_eq!(told, [2], "not said");
    let mut aside = store.account_files("a1", "ops-aside").unwrap();
    aside.sort();
    assert_eq!(aside, names[1..], "not kept aside");
    assert_eq!(store.account_file("a1", &format!("ops-aside/{}", names[1])).unwrap(), Some(damaged), "changed");
    assert_eq!(store.account_file("a1", &format!("ops-aside/{}", names[2])).unwrap(), Some(third), "changed");
    assert!(store.load_ops("a1").unwrap().is_empty());
    // Once: nothing more is sent nor set aside at the next replay.
    let stores = sent(&st, "STORE");
    w.replay();
    assert_eq!(sent(&st, "STORE"), stores, "sent twice");
    assert!(!pool.poll().iter().any(|e| matches!(e, Event::OpsSetAside { .. })), "said twice");
    assert_eq!(store.account_files("a1", "ops-aside/2").unwrap_or_default(), Vec::<String>::new());
}

/// What a damaged folder list's file holds.
const DAMAGED: &[u8] = b"[{\"account\":\"a1\",\"path\":\"Pla";

/// Whether the file `name` of `account` holds the damaged list, byte for byte; what it holds
/// instead, as text, when it doesn't.
fn damaged_still(store: &Store, account: &str, name: &str) -> std::result::Result<(), String> {
    match store.account_file(account, name).unwrap() {
        Some(bytes) if bytes == DAMAGED => Ok(()),
        other => Err(format!("{:?}", other.map(|b| String::from_utf8_lossy(&b).into_owned()))),
    }
}

/// Found in review: a list on disk that was damaged was read as an empty one with no error (and
/// renamed `.json.bad`): a folder renamed here offline (Archive to Plans, not sent yet), not on the
/// server's list, wasn't on the list on disk either, and lost its cache; its mail was downloaded
/// again. A damaged list can't say which those are: every one keeps its cache till a listing can.
#[test]
fn a_damaged_folder_list_empties_no_cache() {
    let st = crate::fake::State::new();
    let port = crate::fake::serve_tcp(&st);
    let store = Store::failing();
    store.save_folders("a1", &[folder("INBOX"), folder("Plans")]).unwrap();
    cache(&store, "Plans", 1);
    store.add_op("a1", Op::RenameFolder { from: "Archive".into(), to: "Plans".into() }).unwrap();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let mut w = worker(imap_account(port), "pw", &store, &pool);
    w.connect().unwrap();
    store.write_account_files("a1", &[("folders.json".into(), DAMAGED.to_vec())]).unwrap();
    assert!(w.relist());
    assert_eq!(cached(&store, "Plans"), 1, "the cache cleared");
    assert_eq!(w.gone_meanwhile, ["Plans"], "looked at again at the next listing");
    // The listing (IMAP's, whole) replaced it, the damaged one set aside first.
    assert_eq!(damaged_still(&store, "a1", "folders.json.bad"), Ok(()));
    // Read, the list on disk (the server's now) doesn't have it: cleared, as ever.
    assert!(w.relist());
    assert_eq!(cached(&store, "Plans"), 0);
    assert!(w.gone_meanwhile.is_empty());
}

/// Sales, a shared mailbox kept here, its mail downloaded and numbered; then the list on disk
/// damaged as a new worker starts, and Sales refused at the listings that follow.
struct SalesKept {
    server: fake_ews::Server,
    store: Store,
    _pool: Pool,
    w: Worker,
    key: MessageKey,
    id: String,
}

fn sales_kept_then_the_list_damaged() -> SalesKept {
    let (d, _) = zones::zones();
    let server = fake_ews::serve(d);
    let store = Store::failing();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let mut w = zones::listed(&server, &store, &pool);
    w.sync_one(zones::SALES_INBOX);
    let key = zones::cached(&store, zones::SALES_INBOX).first().map(|m| m.key.clone()).unwrap();
    assert!(store.has_body(&key), "its source downloaded");
    let id = w.jstate.id(key.uid).unwrap();
    drop(w);
    store.write_account_files("a1", &[("folders.json".into(), DAMAGED.to_vec())]).unwrap();
    let w = zones::connected(&server, &store, &pool);
    server.data.lock().unwrap().zones.refused.push(zones::SALES.into());
    SalesKept { server, store, _pool: pool, w, key, id }
}

/// What the fake server has given of its items so far (their details or sources).
fn given(server: &fake_ews::Server) -> usize {
    server.data.lock().unwrap().given_from.len()
}

/// Found in review: a list on disk that was damaged was read as an empty one with no error: a
/// shared mailbox kept here that the next listings didn't list was taken for nothing here, its
/// numbers pruned, and its mail downloaded again once it listed. A damaged list is one not read:
/// they're kept till a listing names every zone kept here.
#[test]
fn a_damaged_folder_list_keeps_a_zones_numbers() {
    let mut t = sales_kept_then_the_list_damaged();
    for pass in ["first", "second"] {
        assert!(t.w.relist());
        t.w.prune_jmap_numbers();
        assert_eq!(t.w.jstate.id(t.key.uid), Some(t.id.clone()), "{pass} pass: its number pruned");
    }
    assert!(t.w.folders_unread, "a damaged list taken for one read");
    // Sales lists: its mail is as it was, nothing given again.
    zones::mend(&mut t.server.data.lock().unwrap());
    assert!(t.w.relist());
    assert!(!t.w.folders_unread);
    let before = given(&t.server);
    t.w.sync_one(zones::SALES_INBOX);
    assert!(zones::cached(&t.store, zones::SALES_INBOX).iter().any(|m| m.key == t.key), "taken for new");
    assert_eq!(given(&t.server), before, "downloaded again");
}

/// A damaged list on disk stays as it is while listings miss a zone kept here, at a listing and at
/// a folder's new counts (written over, the zone was gone from it); the first listing that misses
/// none replaces it, the damaged one set aside first.
#[test]
fn a_damaged_folder_list_is_set_aside_by_the_complete_listing_that_replaces_it() {
    let mut t = sales_kept_then_the_list_damaged();
    {
        let mut d = t.server.data.lock().unwrap();
        let inbox = d.folders.iter().find(|f| f.known.as_deref() == Some("inbox") && f.owner.is_empty()).map(|f| f.id.clone()).unwrap();
        d.deliver(&inbox, &zones::mail("New", "o2"), false, "2026-10-19T09:00:00Z");
    }
    assert!(t.w.relist());
    t.w.sync_one("INBOX");
    assert_eq!(damaged_still(&t.store, "a1", "folders.json"), Ok(()), "written over while a zone didn't list");
    assert_eq!(t.store.account_file("a1", "folders.json.bad").unwrap(), None, "set aside before it was replaced");
    zones::mend(&mut t.server.data.lock().unwrap());
    assert!(t.w.relist());
    assert_eq!(damaged_still(&t.store, "a1", "folders.json.bad"), Ok(()), "not set aside as it was");
    let now: Vec<String> = t.store.load_folders("a1").unwrap().into_iter().map(|f| f.path).collect();
    assert!(now.iter().any(|p| p == zones::SALES_INBOX), "{now:?}");
}

/// Found in review: an account with no server (POP, the Archives account) whose list on disk was
/// damaged had it read as none, and wrote over it the folders it touched alone: its other folders
/// were out of reach for good. Nothing is written over it: what would is refused, saying why.
#[test]
fn a_local_accounts_damaged_folder_list_is_never_written_over() {
    let store = Store::failing();
    for account in ["p1", "a9"] {
        let three = ["INBOX", "Plans", "Trips"].map(|p| Folder { account: account.into(), ..folder(p) });
        store.save_folders(account, &three).unwrap();
        store.write_account_files(account, &[("folders.json".into(), DAMAGED.to_vec())]).unwrap();
    }
    let raw = b"From: Zoe <zoe@studio.example>\r\nTo: me@studio.example\r\nSubject: Plans\r\nMessage-ID: <p1@studio.example>\r\n\r\nLunch.\r\n";
    // A POP account's folders, as its worker lists them.
    let listed = crate::pop::folders(&store, "p1");
    assert!(matches!(listed, Err(NetError::Store(StoreError::Damaged { .. }))), "{listed:?}");
    // A message archived into the Archives account.
    let archived = crate::worker::changes::copy_checked(&store, "a9", "Trips", raw, Flags::default(), 1);
    assert!(matches!(archived, Err(NetError::Store(StoreError::Damaged { .. }))), "{archived:?}");
    // A message filed in a POP account's folder: kept, its list not touched.
    crate::pop::file_locally(&store, "p1", "Trips", raw, Flags::default(), 1).unwrap();
    for account in ["p1", "a9"] {
        assert_eq!(damaged_still(&store, account, "folders.json"), Ok(()), "{account}: written over");
        assert_eq!(store.account_file(account, "folders.json.bad").unwrap(), None, "{account}: set aside");
    }
}

/// A guard (the owner's rule, as for a list that can't be read): a server whose listings always
/// miss a zone with nothing here (Exchange without public folders) still replaces a damaged list
/// at its first listing, the damaged one set aside: it isn't kept as it is for good.
#[test]
fn a_damaged_folder_list_is_replaced_by_a_server_without_public_folders() {
    let mut d = fake_ews::Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.deliver(&inbox, &zones::mail("Own", "o1"), false, "2026-10-19T08:00:00Z");
    let server = fake_ews::serve(d);
    let store = Store::failing();
    store.write_account_files("a1", &[("folders.json".into(), DAMAGED.to_vec())]).unwrap();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let mut w = zones::connected(&server, &store, &pool);
    assert!(w.folders_unread, "a damaged list taken for one read");
    assert!(w.relist());
    assert!(!w.folders_unread, "held back by public folders the server hasn't");
    assert_eq!(damaged_still(&store, "a1", "folders.json.bad"), Ok(()), "not set aside as it was");
    assert!(store.load_folders("a1").unwrap().iter().any(|f| f.path == "INBOX"));
}
