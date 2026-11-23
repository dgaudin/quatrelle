// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What a worker keeps of a zone an Exchange listing couldn't list (a shared mailbox, the public
//! folders, the online archive): its place in the folder pane and in the list on disk, its cache
//! and its numbers, left as they are till it lists again; and what it still lets go of once a
//! listing that did list says so.

use std::collections::{BTreeMap, HashSet};
use std::sync::mpsc;

use quatrelle_model::{Summary, SyncStatus};

use super::*;
use crate::exchange::missed::{Missed, PublicMissed};
use crate::fakes::fake_ews::{self, DOMAIN, Data, PASSWORD, SharedBox, USER};
use crate::worker::sync::zones::keep_missed;

pub(super) const SALES: &str = "sales@contoso.example";
pub(super) const SALES_INBOX: &str = "Sales/Inbox";
const QUOTES: &str = "Sales/Inbox/Quotes";
const TEAM: &str = "Public Folders/Team";
const YEAR: &str = "Online Archive - Riley/2019";

pub(super) fn mail(subject: &str, id: &str) -> Vec<u8> {
    format!(
        "From: Zoe Park <zoe@contoso.example>\r\nTo: riley@contoso.example\r\nSubject: {subject}\r\nMessage-ID: <{id}@contoso.example>\r\n\r\nAbout {subject}.\r\n"
    )
    .into_bytes()
}

/// The fake's ids of the folders read.
pub(super) struct Ids {
    sales_inbox: String,
    quotes: String,
    team: String,
    year: String,
}

/// Riley's Inbox; Sales, a shared mailbox Autodiscover maps to the account (its Inbox, `Quotes`
/// in it); the public folders (`Team`); the online archive (`2019`). Mail in each.
pub(super) fn zones() -> (Data, Ids) {
    let mut d = Data::default();
    d.pf_mailbox = Some("PFContent@contoso.example".into());
    d.pf_hierarchy = Some("PFHierarchy@contoso.example".into());
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.deliver(&inbox, &mail("Own", "o1"), false, "2026-10-19T08:00:00Z");
    let public = d.folder("IPM_SUBTREE", None, Some("publicfoldersroot"), "IPF.Note");
    let team = d.folder("Team", Some(&public), None, "IPF.Note");
    d.deliver(&team, &mail("Holiday hours", "t1"), false, "2026-10-19T08:10:00Z");
    let sales_inbox = d.shared_mailbox(SharedBox { address: SALES.into(), name: "Sales".into(), automap: true, legacy: false, granted: true });
    d.deliver(&sales_inbox, &mail("Order", "s1"), false, "2026-10-19T08:20:00Z");
    let quotes = d.shared_folder(SALES, "Quotes", &sales_inbox);
    d.deliver(&quotes, &mail("Quote", "q1"), false, "2026-10-19T08:30:00Z");
    let archive = d.archive_mailbox("Online Archive - Riley");
    let year = d.archive_folder("2019", &archive);
    d.deliver(&year, &mail("Old plans", "y1"), true, "2019-01-02T08:00:00Z");
    (d, Ids { sales_inbox, quotes, team, year })
}

fn account(server: &fake_ews::Server) -> AccountConfig {
    AccountConfig {
        id: "a1".into(),
        email: "riley@contoso.example".into(),
        username: format!("{DOMAIN}\\{USER}"),
        ews: Some(server.url.clone()),
        ..Default::default()
    }
}

/// A worker on the fake server, connected.
pub(super) fn connected(server: &fake_ews::Server, store: &Store, pool: &Pool) -> Worker {
    let mut w = Worker::new(account(server), Auth::shared(PASSWORD), store.clone(), pool.sink.clone(), SyncOptions::default());
    w.connect().unwrap();
    w
}

/// A worker on the fake server, connected, its folders listed; `Team` followed.
pub(super) fn listed(server: &fake_ews::Server, store: &Store, pool: &Pool) -> Worker {
    store.save_account_doc("a1", FOLLOWED_PUBLIC_DOC, &[TEAM]).unwrap();
    let mut w = connected(server, store, pool);
    assert!(w.relist());
    w
}

fn paths(folders: &[Folder]) -> Vec<String> {
    let mut out: Vec<String> = folders.iter().map(|f| f.path.clone()).collect();
    out.sort();
    out
}

fn on_disk(store: &Store) -> Vec<String> {
    paths(&store.load_folders("a1").unwrap())
}

pub(super) fn cached(store: &Store, path: &str) -> Vec<Summary> {
    store.load_folder("a1", path).map(|f| f.messages).unwrap_or_default()
}

/// The items the fake server gave (their details or sources) from one of its folders.
fn given(server: &fake_ews::Server, folder: &str) -> usize {
    server.data.lock().unwrap().given_from.iter().filter(|f| *f == folder).count()
}

/// The script mended: every zone lists.
pub(super) fn mend(d: &mut Data) {
    d.zones = Default::default();
    if let Some(a) = d.archive.as_mut() {
        a.refused = false;
    }
}

/// A zone's listing broken in each of these ways in turn, then mended: the folder at `path` read
/// before keeps its place in the pane and in the list on disk, its messages, its source (when
/// its mail is kept here: the archive's is read on the server, what's held of it), its number;
/// mended, it's read again as it was, nothing given again.
fn keeps(zone: &str, path: &str, fake: fn(&Ids) -> &str, kept_here: bool, breaks: &[(&str, fn(&mut Data))]) {
    let (d, ids) = zones();
    let server = fake_ews::serve(d);
    let store = Store::failing();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let mut w = listed(&server, &store, &pool);
    w.sync_one(path);
    let (folders, disk, messages) = (paths(&w.folders), on_disk(&store), cached(&store, path));
    assert!(folders.iter().any(|p| p == path) && disk == folders, "{zone}: {folders:?}");
    let key = messages.first().map(|m| m.key.clone()).unwrap_or_else(|| panic!("{zone}: nothing read in {path}"));
    assert_eq!(store.has_body(&key), kept_here, "{zone}");
    let id = w.jstate.id(key.uid).unwrap();
    let given_before = given(&server, fake(&ids));
    for (how, script) in breaks {
        {
            let mut d = server.data.lock().unwrap();
            mend(&mut d);
            script(&mut d);
        }
        // A connection of its own: what Autodiscover said is asked again.
        w.conn = None;
        w.connect().unwrap();
        assert!(w.relist(), "{zone}, {how}");
        w.prune_jmap_numbers();
        assert_eq!(paths(&w.folders), folders, "{zone}, {how}: its place in the pane");
        assert_eq!(on_disk(&store), folders, "{zone}, {how}: its place in the list on disk");
        assert_eq!(cached(&store, path), messages, "{zone}, {how}: its messages");
        assert_eq!(store.has_body(&key), kept_here, "{zone}, {how}: its source");
        assert_eq!(w.jstate.id(key.uid), Some(id.clone()), "{zone}, {how}: its number");
    }
    mend(&mut server.data.lock().unwrap());
    assert!(w.relist());
    w.sync_one(path);
    assert!(cached(&store, path).iter().any(|m| m.key == key), "{zone}: listed again, its message has another key");
    assert_eq!(given(&server, fake(&ids)), given_before, "{zone}: its mail given again");
}

/// Found in review: a zone whose listing failed, answered empty or couldn't be asked for lost its
/// folders from the pane and the list on disk, its cache (the archive: what was held of it) and
/// its numbers, and everything was downloaded again once it listed. It keeps them all, as they
/// were, till it lists again.
#[test]
fn a_zone_whose_listing_fails_keeps_its_place_cache_and_numbers() {
    keeps("a shared mailbox refused", SALES_INBOX, |i| &i.sales_inbox, true, &[("refused", |d| d.zones.refused.push(SALES.into()))]);
    keeps("a shared mailbox answered empty", SALES_INBOX, |i| &i.sales_inbox, true, &[("empty", |d| d.zones.emptied.push(SALES.into()))]);
    keeps("Autodiscover failing", SALES_INBOX, |i| &i.sales_inbox, true, &[("untold", |d| d.zones.discoveries_failing = 100)]);
    keeps(
        "the public folders",
        TEAM,
        |i| &i.team,
        true,
        &[("refused", |d| d.zones.public_refused = true), ("empty", |d| d.zones.public_emptied = true)],
    );
    keeps(
        "the online archive",
        YEAR,
        |i| &i.year,
        false,
        &[("refused", |d| d.archive.iter_mut().for_each(|a| a.refused = true)), ("empty", |d| d.zones.archive_emptied = true)],
    );
}

/// A folder of the listing before named `path`, under `/`.
fn folder(path: &str) -> Folder {
    Folder { account: "a1".into(), path: path.into(), delimiter: Some('/'), selectable: true, ..Default::default() }
}

/// A public branch whose folders didn't list whole (`Under` its parent's id) is put back, a
/// folder of it under one put back too, with their ids; a public folder gone from under a parent
/// read whole isn't.
#[test]
fn keep_missed_puts_back_a_public_branch_that_failed() {
    let before: Vec<Folder> = ["INBOX", "Public Folders", "Public Folders/A", "Public Folders/A/B", "Public Folders/A/B/C", "Public Folders/D"]
        .into_iter()
        .map(folder)
        .collect();
    let ids_before: BTreeMap<String, String> = before.iter().enumerate().map(|(n, f)| (f.path.clone(), format!("P{n}"))).collect();
    let mut list: Vec<Folder> = before.iter().take(3).cloned().collect();
    let mut ids: BTreeMap<String, String> = ids_before.iter().take(3).map(|(p, i)| (p.clone(), i.clone())).collect();
    let mut owners = BTreeMap::new();
    let a = ids_before.get("Public Folders/A").unwrap().clone();
    let missed = Missed { public: PublicMissed::Under([a].into()), ..Default::default() };
    let kept = keep_missed(&before, &ids_before, &BTreeMap::new(), &mut list, &mut ids, &mut owners, &missed);
    let want: HashSet<String> = ["Public Folders/A/B".to_string(), "Public Folders/A/B/C".to_string()].into();
    assert_eq!(kept, want);
    assert_eq!(paths(&list), ["INBOX", "Public Folders", "Public Folders/A", "Public Folders/A/B", "Public Folders/A/B/C"], "D's parent read whole");
    for p in &want {
        assert_eq!(ids.get(p), ids_before.get(p), "{p}: its id");
    }
    assert!(owners.is_empty());
}

/// Counter-probe: a shared mailbox that lists, without one of its folders (deleted on the server),
/// lets that one go with its cache, while another zone (the archive) doesn't list; and a public
/// folder gone from under a parent read whole isn't put back.
#[test]
fn a_folder_really_gone_from_a_listed_zone_still_loses_its_cache() {
    let (d, ids) = zones();
    let server = fake_ews::serve(d);
    let store = Store::failing();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let mut w = listed(&server, &store, &pool);
    w.sync_one(QUOTES);
    assert_eq!(cached(&store, QUOTES).len(), 1);
    {
        let mut d = server.data.lock().unwrap();
        d.folders.retain(|f| f.id != ids.quotes);
        d.archive.iter_mut().for_each(|a| a.refused = true);
    }
    assert!(w.relist());
    assert!(!w.folders.iter().any(|f| f.path == QUOTES), "{:?}", paths(&w.folders));
    assert!(cached(&store, QUOTES).is_empty(), "its cache kept");
    // A public folder gone from under a parent read whole, while another branch didn't list.
    let before = vec![folder("Public Folders"), folder("Public Folders/A"), folder("Public Folders/D")];
    let ids_before: BTreeMap<String, String> = before.iter().map(|f| (f.path.clone(), format!("id {}", f.path))).collect();
    let mut list = vec![folder("Public Folders"), folder("Public Folders/A")];
    let mut now = ids_before.clone();
    now.remove("Public Folders/D");
    let missed = Missed { public: PublicMissed::Under(["id Public Folders/A".to_string()].into()), ..Default::default() };
    let kept = keep_missed(&before, &ids_before, &BTreeMap::new(), &mut list, &mut now, &mut BTreeMap::new(), &missed);
    assert!(kept.is_empty(), "{kept:?}");
    assert_eq!(paths(&list), ["Public Folders", "Public Folders/A"]);
}

/// Counter-probe: a mailbox Autodiscover no longer names, as it answers, lets its folders go.
#[test]
fn a_mailbox_no_longer_named_lets_its_folders_go() {
    let (d, _) = zones();
    let server = fake_ews::serve(d);
    let store = Store::failing();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let mut w = listed(&server, &store, &pool);
    w.sync_one(SALES_INBOX);
    assert_eq!(cached(&store, SALES_INBOX).len(), 1);
    server.data.lock().unwrap().shared.clear();
    w.conn = None;
    w.connect().unwrap();
    assert!(w.relist());
    assert!(!w.folders.iter().any(|f| f.path.starts_with("Sales")), "{:?}", paths(&w.folders));
    assert!(cached(&store, SALES_INBOX).is_empty(), "its cache kept");
}

/// The folders asked of the server whose syncs were told, since the events were last read.
fn synced(pool: &Pool) -> Vec<String> {
    pool.poll()
        .into_iter()
        .filter_map(|e| match e {
            Event::Status { status: SyncStatus::Syncing { folder }, .. } => Some(folder),
            _ => None,
        })
        .collect()
}

/// What the fake server listed or gave from these folders since `from` (its counts then).
fn asked_of(server: &fake_ews::Server, from: (usize, usize), folders: &[&str]) -> Vec<String> {
    let d = server.data.lock().unwrap();
    let listed = d.found_in.iter().skip(from.0).map(|(f, _, _)| f.clone());
    listed.chain(d.given_from.iter().skip(from.1).cloned()).filter(|f| folders.contains(&f.as_str())).collect()
}

fn asked_so_far(server: &fake_ews::Server) -> (usize, usize) {
    let d = server.data.lock().unwrap();
    (d.found_in.len(), d.given_from.len())
}

/// A guard (the zone went from the list before): while a zone is kept for want of a listing, the
/// full sync reads nothing of it, and lets nothing of it go (a public folder no longer followed,
/// whose mail goes once its zone lists); once it lists, it's read and let go of as ever.
#[test]
fn a_zone_kept_unlisted_is_not_synced_until_it_lists_again() {
    let (mut d, ids) = zones();
    let team = ids.team.clone();
    for i in 0..ONLINE_WINDOW + 5 {
        d.deliver(&team, &mail(&format!("Report {i}"), &format!("r{i}")), true, &format!("2026-09-{:02}T09:{:02}:00Z", 1 + i % 28, i % 60));
    }
    let server = fake_ews::serve(d);
    let store = Store::failing();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let mut w = listed(&server, &store, &pool);
    let (_jobs, rx) = mpsc::channel();
    w.sync_all(&rx);
    let kept = cached(&store, TEAM).len();
    assert!(kept > ONLINE_WINDOW && !cached(&store, SALES_INBOX).is_empty());
    // Sales and the public folders don't list; Team is no longer followed.
    {
        let mut d = server.data.lock().unwrap();
        d.zones.refused.push(SALES.into());
        d.zones.public_refused = true;
    }
    store.save_account_doc("a1", FOLLOWED_PUBLIC_DOC, &Vec::<String>::new()).unwrap();
    synced(&pool);
    let from = asked_so_far(&server);
    w.sync_all(&rx);
    let zone = [ids.sales_inbox.as_str(), ids.quotes.as_str(), team.as_str()];
    assert_eq!(asked_of(&server, from, &zone), Vec::<String>::new(), "asked of the server");
    let told = synced(&pool);
    assert!(!told.iter().any(|p| p.starts_with("Sales") || is_public_folder(p)), "{told:?}");
    assert!(told.iter().any(|p| p == "INBOX"), "{told:?}");
    assert_eq!(cached(&store, TEAM).len(), kept, "let go of");
    // Listed again: read, and let go of, as ever.
    mend(&mut server.data.lock().unwrap());
    w.sync_all(&rx);
    assert!(synced(&pool).iter().any(|p| p == SALES_INBOX));
    assert!(cached(&store, TEAM).len() <= ONLINE_WINDOW, "not let go of once listed");
}

/// Found in review: a folder of a zone that the listing before took for gone, its cache kept for
/// a change to the folders waiting (Quotes renamed Offers here), lost its cache at a listing that
/// couldn't list its zone, or was never looked at again. It's kept, and looked at again once its
/// zone lists.
#[test]
fn a_folder_gone_inside_a_kept_zone_is_looked_at_again_once_it_lists() {
    let (d, _) = zones();
    let server = fake_ews::serve(d);
    let store = Store::failing();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let mut w = listed(&server, &store, &pool);
    w.sync_one(QUOTES);
    // Renamed here, not sent yet: the server still has Quotes.
    let offers = "Sales/Inbox/Offers";
    let op = Op::RenameFolder { from: QUOTES.into(), to: offers.into() };
    store.add_op("a1", op.clone()).unwrap();
    store.apply_op("a1", &op).unwrap();
    assert!(w.reload_folders());
    assert!(w.relist());
    assert_eq!(w.gone_meanwhile, [offers]);
    assert_eq!(cached(&store, offers).len(), 1);
    // Sales doesn't list.
    server.data.lock().unwrap().zones.refused.push(SALES.into());
    assert!(w.relist());
    assert_eq!(cached(&store, offers).len(), 1, "its cache cleared while its zone didn't list");
    assert_eq!(w.gone_meanwhile, [offers], "not looked at again");
    // Sales lists, without it: it goes, as a folder gone from a listing that could tell.
    mend(&mut server.data.lock().unwrap());
    assert!(w.relist());
    assert!(cached(&store, offers).is_empty());
    assert!(w.gone_meanwhile.is_empty());
}

/// Found in review: Graph wasn't covered: a shared mailbox that wouldn't open lost its folders,
/// its cache and its numbers. Kept, as on EWS.
#[test]
fn a_graph_shared_mailbox_that_wont_open_keeps_its_cache() {
    use crate::fakes::fake_graph::{self, TOKEN};
    let support = "support@contoso.example";
    let path = "support@contoso.example/Inbox";
    let mut d = fake_graph::Data::default();
    d.folder("Inbox", None, Some("inbox"));
    d.shared = vec![support.into()];
    let theirs = d.shared_folder(support, "Inbox", None, Some("inbox"));
    d.deliver(&theirs, &mail("Printer on fire", "p1"), false, "2026-10-19T10:00:00Z");
    let server = fake_graph::serve(d);
    let store = Store::failing();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let acc = AccountConfig {
        id: "a1".into(),
        email: "me@contoso.example".into(),
        graph: Some(server.base.clone()),
        shared_mailboxes: vec![support.into()],
        ..Default::default()
    };
    let mut w = Worker::new(acc, Auth::shared(TOKEN), store.clone(), pool.sink.clone(), SyncOptions::default());
    w.connect().unwrap();
    assert!(w.relist());
    w.sync_one(path);
    let (folders, messages) = (paths(&w.folders), cached(&store, path));
    let key = messages.first().map(|m| m.key.clone()).unwrap();
    assert!(store.has_body(&key));
    let id = w.jstate.id(key.uid).unwrap();
    // It won't open now.
    server.data.lock().unwrap().shared.clear();
    assert!(w.relist());
    w.prune_jmap_numbers();
    assert_eq!(paths(&w.folders), folders, "its place in the pane");
    assert_eq!(on_disk(&store), folders, "its place in the list on disk");
    assert_eq!(cached(&store, path), messages, "its messages");
    assert!(store.has_body(&key), "its source");
    assert_eq!(w.jstate.id(key.uid), Some(id), "its number");
    assert_eq!(w.jstate.owner(path), Some(support), "whose it is");
    // It opens again: read as it was.
    server.data.lock().unwrap().shared = vec![support.into()];
    assert!(w.relist());
    let downloads = || server.data.lock().unwrap().calls.iter().filter(|c| c.contains("$value")).count();
    let before = downloads();
    w.sync_one(path);
    assert!(cached(&store, path).iter().any(|m| m.key == key));
    assert_eq!(downloads(), before, "its source downloaded again");
}

/// Found in review: the folder list before couldn't be read at the start; a listing that didn't
/// list a shared mailbox lifted that at once, and the next pass, the mailbox still not listed,
/// pruned its numbers (it wasn't in the worker's list): its mail was taken for new once it
/// listed. They're kept till a listing names every zone kept here.
#[test]
fn numbers_are_kept_until_a_listing_misses_nothing() {
    let (d, ids) = zones();
    let server = fake_ews::serve(d);
    let store = Store::failing();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let mut w = listed(&server, &store, &pool);
    w.sync_one(SALES_INBOX);
    let key = cached(&store, SALES_INBOX).first().map(|m| m.key.clone()).unwrap();
    let id = w.jstate.id(key.uid).unwrap();
    drop(w);
    // The list on disk can't be read as it starts again; Sales doesn't list, twice.
    store.fail(Some("folders.json"), None);
    let mut w = connected(&server, &store, &pool);
    server.data.lock().unwrap().zones.refused.push(SALES.into());
    for pass in ["first", "second"] {
        assert!(w.relist());
        w.prune_jmap_numbers();
        assert_eq!(w.jstate.id(key.uid), Some(id.clone()), "{pass} pass: its number pruned");
    }
    // Sales lists: every zone kept here is named, numbers are pruned again.
    mend(&mut server.data.lock().unwrap());
    assert!(w.relist());
    assert!(!w.folders_unread);
    let given_before = given(&server, &ids.sales_inbox);
    w.sync_one(SALES_INBOX);
    assert!(cached(&store, SALES_INBOX).iter().any(|m| m.key == key), "taken for new");
    assert_eq!(given(&server, &ids.sales_inbox), given_before);
    let gone = w.jstate.number("M-gone");
    w.prune_jmap_numbers();
    assert_eq!(w.jstate.id(gone), None);
    store.fail(None, None);
}

/// The owner's rule: a zone that never lists and keeps nothing here (a server without public
/// folders, whose listing always misses them) doesn't keep the folder list from being read
/// again: the list before couldn't be read, a listing of everything else lists all there is.
#[test]
fn a_zone_with_nothing_here_doesnt_hold_the_list_back() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.deliver(&inbox, &mail("Own", "o1"), false, "2026-10-19T08:00:00Z");
    let server = fake_ews::serve(d);
    let store = Store::failing();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let mut w = connected(&server, &store, &pool);
    assert!(w.relist());
    w.sync_one("INBOX");
    drop(w);
    store.fail(Some("folders.json"), None);
    let mut w = connected(&server, &store, &pool);
    assert!(w.folders_unread, "set as it starts");
    assert!(w.relist());
    assert!(!w.folders_unread, "held back by public folders the server hasn't");
    let gone = w.jstate.number("M-gone");
    w.prune_jmap_numbers();
    assert_eq!(w.jstate.id(gone), None);
    store.fail(None, None);
}

/// Found in review: while the list before couldn't be read, the worker's (a listing that missed a
/// zone) was kept over it, at the listing and at a folder's new counts: the zone was gone from the
/// list on disk too. It's told for the folder pane and kept only once a listing misses none.
#[test]
fn a_folder_list_known_short_isnt_kept_over_the_one_before() {
    let (d, _) = zones();
    let server = fake_ews::serve(d);
    let store = Store::failing();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let mut w = listed(&server, &store, &pool);
    // Sales kept here: its numbers are written.
    w.sync_one(SALES_INBOX);
    let whole = on_disk(&store);
    assert!(whole.iter().any(|p| p == SALES_INBOX));
    drop(w);
    store.fail(Some("folders.json"), None);
    let mut w = connected(&server, &store, &pool);
    {
        let mut d = server.data.lock().unwrap();
        d.zones.refused.push(SALES.into());
        let inbox = d.folders.iter().find(|f| f.known.as_deref() == Some("inbox") && f.owner.is_empty()).map(|f| f.id.clone()).unwrap();
        d.deliver(&inbox, &mail("New", "o2"), false, "2026-10-19T09:00:00Z");
    }
    pool.poll();
    assert!(w.relist());
    w.sync_one("INBOX");
    assert!(pool.poll().iter().any(|e| matches!(e, Event::Folders { .. })), "told for the folder pane");
    store.fail(None, None);
    assert_eq!(on_disk(&store), whole, "kept over the list before");
    // Every zone lists: the list is kept.
    store.fail(Some("folders.json"), None);
    {
        let mut d = server.data.lock().unwrap();
        mend(&mut d);
        d.folder("Projects", None, None, "IPF.Note");
    }
    assert!(w.relist());
    store.fail(None, None);
    let now = on_disk(&store);
    assert!(now.iter().any(|p| p == "Projects") && now.iter().any(|p| p == SALES_INBOX), "{now:?}");
}

/// Riley's Inbox and the public folders' root holding `Projects` (its id), which holds `n`
/// folders, `Project 001`… (their ids), a message in each.
fn public_projects(n: usize) -> (Data, String, Vec<String>) {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.deliver(&inbox, &mail("Own", "o1"), false, "2026-10-19T08:00:00Z");
    let root = d.folder("IPM_SUBTREE", None, Some("publicfoldersroot"), "IPF.Note");
    let projects = d.folder("Projects", Some(&root), None, "IPF.Note");
    let mut kids = Vec::new();
    for i in 1..=n {
        let id = d.folder(&format!("Project {i:03}"), Some(&projects), None, "IPF.Note");
        d.deliver(&id, &mail(&format!("Plan {i}"), &format!("p{i}")), true, "2026-10-19T09:00:00Z");
        kids.push(id);
    }
    (d, projects, kids)
}

fn project(i: usize) -> String {
    format!("Public Folders/Projects/Project {i:03}")
}

/// A worker on the fake server, connected, following these public folders, its folders listed.
fn following(server: &fake_ews::Server, store: &Store, pool: &Pool, followed: &[String]) -> Worker {
    store.save_account_doc("a1", FOLLOWED_PUBLIC_DOC, followed).unwrap();
    let mut w = connected(server, store, pool);
    assert!(w.relist());
    w
}

/// Found in review: a public folder's page past the first that failed ended its listing there
/// without a word: what was past it was taken for gone, and lost its cache. The pages read are
/// listed as the server has them now, and what was past them is kept as it was.
#[test]
fn a_page_that_fails_keeps_the_pages_read_and_what_was_cached_past_them() {
    let (d, projects, kids) = public_projects(250);
    let server = fake_ews::serve(d);
    let store = Store::failing();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let all: Vec<String> = (1..=250).map(project).collect();
    let mut w = following(&server, &store, &pool, &all);
    let listed = paths(&w.folders);
    assert!(all.iter().all(|p| listed.contains(p)), "{} of them listed", all.iter().filter(|p| listed.contains(p)).count());
    for p in &all {
        w.sync_one(p);
    }
    let caches: Vec<Vec<Summary>> = all.iter().map(|p| cached(&store, p)).collect();
    assert!(caches.iter().all(|c| c.len() == 1));
    // A message more in each on the server; the second page of Projects' folders fails.
    {
        let mut d = server.data.lock().unwrap();
        for (i, k) in kids.iter().enumerate() {
            d.deliver(k, &mail(&format!("Update {i}"), &format!("u{i}")), true, "2026-10-20T09:00:00Z");
        }
        d.zones.public_failing.push((projects.clone(), 2));
    }
    assert!(w.relist());
    let total = |p: &str| w.folders.iter().find(|f| f.path == p).map(|f| f.total);
    for (n, (p, cache)) in all.iter().zip(&caches).enumerate() {
        assert_eq!(&cached(&store, p), cache, "{p}: its cache");
        assert_eq!(total(p), Some(if n < 200 { 2 } else { 1 }), "{p}: as the server has it now, or as it was");
    }
    assert_eq!(on_disk(&store), paths(&w.folders), "the list on disk");
}

/// Found in review: a public folder whose own folders wouldn't list (a level under the root
/// refused) was passed over: its branch, every folder under it, was taken for gone, and lost its
/// place, its cache and its numbers. All are kept as they were till it lists; then read as ever.
#[test]
fn a_public_branch_that_fails_keeps_what_was_under_it() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let root = d.folder("IPM_SUBTREE", None, Some("publicfoldersroot"), "IPF.Note");
    let team = d.folder("Team", Some(&root), None, "IPF.Note");
    let projects = d.folder("Projects", Some(&team), None, "IPF.Note");
    let year = d.folder("2025", Some(&projects), None, "IPF.Note");
    d.folder("News", Some(&root), None, "IPF.Note");
    d.deliver(&projects, &mail("Plans", "p1"), false, "2026-10-19T08:00:00Z");
    d.deliver(&year, &mail("Old plans", "y1"), true, "2025-01-02T08:00:00Z");
    let server = fake_ews::serve(d);
    let store = Store::failing();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let followed = ["Public Folders/Team/Projects".to_string(), "Public Folders/Team/Projects/2025".to_string()];
    let mut w = following(&server, &store, &pool, &followed);
    for p in &followed {
        w.sync_one(p);
    }
    let (folders, disk) = (paths(&w.folders), on_disk(&store));
    let held: Vec<(Vec<Summary>, MessageKey, String)> = followed
        .iter()
        .map(|p| {
            let messages = cached(&store, p);
            let key = messages.first().map(|m| m.key.clone()).unwrap_or_else(|| panic!("nothing read in {p}"));
            let id = w.jstate.id(key.uid).unwrap();
            (messages, key, id)
        })
        .collect();
    assert!(held.iter().all(|(_, key, _)| store.has_body(key)));
    let given_before = (given(&server, &projects), given(&server, &year));
    server.data.lock().unwrap().zones.public_failing.push((team.clone(), 1));
    assert!(w.relist());
    w.prune_jmap_numbers();
    assert_eq!(paths(&w.folders), folders, "its place in the pane");
    assert_eq!(on_disk(&store), disk, "its place in the list on disk");
    for (p, (messages, key, id)) in followed.iter().zip(&held) {
        assert_eq!(&cached(&store, p), messages, "{p}: its messages");
        assert!(store.has_body(key), "{p}: its source");
        assert_eq!(w.jstate.id(key.uid), Some(id.clone()), "{p}: its number");
    }
    // It lists again: read as it was, nothing given again.
    mend(&mut server.data.lock().unwrap());
    assert!(w.relist());
    for p in &followed {
        w.sync_one(p);
    }
    for (p, (_, key, _)) in followed.iter().zip(&held) {
        assert!(cached(&store, p).iter().any(|m| m.key == *key), "{p}: listed again, its message has another key");
    }
    assert_eq!((given(&server, &projects), given(&server, &year)), given_before, "their mail given again");
}

/// Counter-probe: a public folder deleted on the server, kept while a page of its parent failed,
/// goes with its cache once its parent lists whole (in two pages here), while another branch
/// still doesn't list.
#[test]
fn a_public_folder_deleted_on_the_server_goes_once_its_parent_lists_whole() {
    let (mut d, projects, kids) = public_projects(250);
    let root = d.folders.iter().find(|f| f.known.as_deref() == Some("publicfoldersroot")).map(|f| f.id.clone()).unwrap();
    let team = d.folder("Team", Some(&root), None, "IPF.Note");
    d.folder("Minutes", Some(&team), None, "IPF.Note");
    let server = fake_ews::serve(d);
    let store = Store::failing();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let gone = project(10);
    let mut w = following(&server, &store, &pool, std::slice::from_ref(&gone));
    w.sync_one(&gone);
    assert_eq!(cached(&store, &gone).len(), 1);
    {
        let mut d = server.data.lock().unwrap();
        d.folders.retain(|f| f.id != kids[9]);
        d.zones.public_failing.push((projects.clone(), 2));
    }
    assert!(w.relist());
    // Projects lists whole; Team doesn't.
    server.data.lock().unwrap().zones.public_failing = vec![(team, 1)];
    assert!(w.relist());
    assert!(!w.folders.iter().any(|f| f.path == gone), "{gone} still listed");
    assert!(cached(&store, &gone).is_empty(), "its cache kept");
}

/// Found in review: the cap on the public folders listed (500) cut the walk, and what was past it
/// was taken for gone: 50 folders kept here lost their cache. What the cap leaves out is kept as
/// it was, the folder whose folders it cut said not listed whole.
#[test]
fn the_cap_on_public_folders_takes_nothing_from_the_cache() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let root = d.folder("IPM_SUBTREE", None, Some("publicfoldersroot"), "IPF.Note");
    let [a, b, c] = ["A", "B", "C"].map(|n| d.folder(n, Some(&root), None, "IPF.Note"));
    // 500 public folders: these three, and 150, 150 and 197 in them; C's last 50 followed.
    let mut in_c = Vec::new();
    for (parent, n) in [(&a, 150), (&b, 150), (&c, 197)] {
        for i in 1..=n {
            let id = d.folder(&format!("Folder {i:03}"), Some(parent), None, "IPF.Note");
            if *parent == c {
                in_c.push(id);
            }
        }
    }
    for (i, id) in in_c.iter().enumerate().skip(147) {
        d.deliver(id, &mail(&format!("Minutes {i}"), &format!("c{i}")), true, "2026-10-19T09:00:00Z");
    }
    let last: Vec<String> = (148..=197).map(|i| format!("Public Folders/C/Folder {i:03}")).collect();
    let server = fake_ews::serve(d);
    let store = Store::failing();
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    let mut w = following(&server, &store, &pool, &last);
    assert_eq!(w.folders.iter().filter(|f| is_public_folder(&f.path)).count(), 501, "every one listed, and Public Folders");
    for p in &last {
        w.sync_one(p);
    }
    let caches: Vec<Vec<Summary>> = last.iter().map(|p| cached(&store, p)).collect();
    assert!(caches.iter().all(|m| m.len() == 1));
    // 100 more, in A and B: 600, and the cap cuts C's folders after its 97th.
    {
        let mut d = server.data.lock().unwrap();
        for parent in [&a, &b] {
            for i in 151..=200 {
                d.folder(&format!("Folder {i:03}"), Some(parent), None, "IPF.Note");
            }
        }
    }
    assert!(w.relist());
    let listed = paths(&w.folders);
    for p in ["Public Folders/A/Folder 200", "Public Folders/B/Folder 200", "Public Folders/C/Folder 097"] {
        assert!(listed.iter().any(|l| l == p), "{p} listed");
    }
    for (p, cache) in last.iter().zip(&caches) {
        assert_eq!(&cached(&store, p), cache, "{p}: its cache");
        assert!(listed.contains(p), "{p}: its place in the pane");
    }
}
