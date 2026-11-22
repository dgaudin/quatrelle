// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Tests over the fake EWS server, through the pool: a shared mailbox whose folders' listing is
//! refused keeps its cache and its place in the folder pane.

use super::*;

const SALES: &str = "sales@contoso.example";
const SALES_INBOX: &str = "Sales/Inbox";

/// The events of a full sync, from its listing (the folders told) till the account is online.
fn a_full_pass(pool: &crate::worker::Pool) -> Vec<Event> {
    let mut seen = until(pool, "the folders listed again", |e| matches!(e, Event::Folders { .. }));
    seen.extend(until(pool, "syncing", |e| matches!(e, Event::Status { status: SyncStatus::Syncing { .. }, .. })));
    seen.extend(until(pool, "online again", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. })));
    seen
}

/// An account with Sales, a shared mailbox Autodiscover maps to it, started and synced; then a
/// full sync whose listing of Sales' folders is refused. The store, Sales' Inbox's messages (each
/// with its source) before, and the events of that pass.
fn a_refused_pass(name: &str) -> (Store, Vec<quatrelle_model::Summary>, Vec<Event>) {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.deliver(&inbox, &mail("Own", "o1"), false, "2026-10-19T08:00:00Z");
    let sales = d.shared_mailbox(SharedBox { address: SALES.into(), name: "Sales".into(), automap: true, legacy: false, granted: true });
    d.deliver(&sales, &mail("Order", "s1"), false, "2026-10-19T08:20:00Z");
    let server = serve(d);
    let data = server.data.clone();
    let store = crate::fake::tests_store(name);
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "riley@contoso.example".into(),
        username: format!("{DOMAIN}\\{USER}"),
        ews: Some(server.url.clone()),
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret(PASSWORD.into())) });
    let kept = || store.load_folder("a1", SALES_INBOX).map(|f| f.messages).unwrap_or_default();
    eventually("Sales kept", || kept().len() == 1 && kept().iter().all(|m| store.has_body(&m.key)));
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let before = kept();
    while !pool.poll().is_empty() {}
    let listings = || data.lock().unwrap().calls.iter().filter(|c| *c == "FindFolder").count();
    let listed = listings();
    data.lock().unwrap().zones.refused.push(SALES.into());
    pool.submit(Job::SyncAll { account: "a1".into() });
    let seen = a_full_pass(&pool);
    eventually("Sales listed again", || listings() > listed);
    drop(pool);
    (store, before, seen)
}

/// Found in review: a full sync whose listing of a shared mailbox was refused emptied its cache
/// (downloaded again once it listed). It keeps it, sources and all.
#[test]
fn through_the_pool_a_refused_shared_listing_keeps_the_cache() {
    let (store, before, _) = a_refused_pass("ews-zone-refused-cache");
    let now = store.load_folder("a1", SALES_INBOX).map(|f| f.messages).unwrap_or_default();
    assert_eq!(now, before, "its cache");
    assert!(now.iter().all(|m| store.has_body(&m.key)), "its sources");
    let _ = std::fs::remove_dir_all(store.root());
}

/// Found in review: the shared mailbox whose listing was refused left the folder pane till it
/// listed again. It stays.
#[test]
fn a_kept_zone_stays_in_the_folder_pane() {
    let (store, _, seen) = a_refused_pass("ews-zone-refused-pane");
    let last = seen.iter().rev().find_map(|e| match e {
        Event::Folders { folders, .. } => Some(folders.iter().map(|f| f.path.clone()).collect::<Vec<_>>()),
        _ => None,
    });
    let last = last.unwrap_or_default();
    assert!(last.iter().any(|p| p == SALES_INBOX), "{last:?}");
    let _ = std::fs::remove_dir_all(store.root());
}
