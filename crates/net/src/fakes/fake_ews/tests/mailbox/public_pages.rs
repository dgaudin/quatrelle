// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Tests over the fake EWS server: a public folder's folders listed a page at a time to the last,
//! each once however the server pages, and what didn't list whole (a page refused, pages without
//! end, the tree too deep) said, so that nothing under it is taken for gone; by the client, and
//! through the pool.

use std::collections::HashSet;

use quatrelle_model::FOLLOWED_PUBLIC_DOC;

use super::*;
use crate::exchange::missed::{Listed, Missed, PublicMissed};

/// Pages asked for one public folder's folders at most (the client's `PUBLIC_PAGES_MAX`).
const PAGES_MAX: usize = 50;
/// How deep the public folders are walked (the client's `PUBLIC_DEPTH`).
const DEPTH: usize = 6;

/// Riley's Inbox, and the public folders' root; its id.
fn with_root() -> (Data, String) {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let root = d.folder("IPM_SUBTREE", None, Some("publicfoldersroot"), "IPF.Note");
    (d, root)
}

/// The public folders' root holding `Projects`, which holds `n` folders: `Project 001`… Its id.
fn projects(n: usize) -> (Data, String, String) {
    let (mut d, root) = with_root();
    let projects = d.folder("Projects", Some(&root), None, "IPF.Note");
    for i in 1..=n {
        d.folder(&format!("Project {i:03}"), Some(&projects), None, "IPF.Note");
    }
    (d, root, projects)
}

/// The folders listed by a client of the fake server, and what it missed.
fn list(server: &Server) -> Listed {
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    crate::ews::list_folders(&c, &mut crate::jmap::State::default(), "a1").unwrap()
}

/// The names of the folders listed under this path.
fn under<'a>(listed: &'a Listed, path: &str) -> Vec<&'a str> {
    let prefix = format!("{path}/");
    listed.folders.iter().filter_map(|f| f.path.strip_prefix(prefix.as_str())).filter(|n| !n.contains('/')).collect()
}

/// The `Offset` of each page of this folder's folders asked for.
fn pages_of(server: &Server, folder: &str) -> Vec<usize> {
    server.data.lock().unwrap().zones.public_asked.iter().filter(|(p, _)| p == folder).map(|(_, o)| *o).collect()
}

/// What the listing says of the public folders: the folders under these didn't list whole.
fn missed_under(ids: &[&str]) -> PublicMissed {
    PublicMissed::Under(ids.iter().map(|i| i.to_string()).collect())
}

/// The survey's probe: a public folder holding more folders than one page asks for (250, in pages
/// of 200) had its first 200 listed, never the rest. Every one is, each page asked from where the
/// last left off, and nothing is said missed.
#[test]
fn every_page_of_a_public_folders_folders_is_listed() {
    let (d, _, projects) = projects(250);
    let server = serve(d);
    let listed = list(&server);
    let mut names = under(&listed, "Public Folders/Projects");
    names.sort_unstable();
    let want: Vec<String> = (1..=250).map(|i| format!("Project {i:03}")).collect();
    assert_eq!(names, want);
    assert_eq!(pages_of(&server, &projects), [0, 200]);
    assert_eq!(listed.missed, Missed::default());
}

/// A server that answers a public folder's first page again at every `Offset`, never saying it's
/// the last, filled the list with the same folders over and over, up to the cap. Each is listed
/// once, the listing ends at the page that brings none new, and that folder's folders are said
/// not listed whole: nothing past them is taken for gone. One that ignores `Offset` and says the
/// last page came, a page all seen already, is taken no more at its word.
#[test]
fn a_server_that_repeats_a_page_lists_each_folder_once() {
    let scripts: [(&str, fn(&mut Data, &str)); 2] = [
        ("the first page at every Offset", |d, id| d.zones.public_repeated.push(id.into())),
        ("Offset ignored", |d, id| d.zones.public_offset_ignored.push(id.into())),
    ];
    for (how, script) in scripts {
        let (mut d, _, projects) = projects(250);
        script(&mut d, &projects);
        let server = serve(d);
        let listed = list(&server);
        let names = under(&listed, "Public Folders/Projects");
        let distinct: HashSet<&str> = names.iter().copied().collect();
        assert_eq!((names.len(), distinct.len()), (200, 200), "{how}: {} listed in all", listed.folders.len());
        assert!(names.iter().all(|n| !n.contains('~')), "{how}: one listed twice");
        let pages = pages_of(&server, &projects);
        assert!(pages.len() < PAGES_MAX, "{how}: {} pages asked for", pages.len());
        assert_eq!(pages, [0, 200], "{how}: the first, and the one that brought nothing new");
        assert_eq!(listed.missed.public, missed_under(&[&projects]), "{how}");
    }
}

/// A guard (one page was asked for): a server whose list of a public folder's folders never ends,
/// each page bringing folders never seen and saying more follow, is given up after so many pages;
/// the account's own folders are listed all the same, and that folder's said not listed whole.
#[test]
fn a_listing_that_never_ends_is_given_up() {
    let (mut d, root) = with_root();
    d.zones.public_page = Some(1);
    d.zones.public_endless.push(root.clone());
    let server = serve(d);
    let listed = list(&server);
    assert!(listed.folders.iter().any(|f| f.path == "INBOX"), "{:?}", listed.folders.iter().map(|f| &f.path).collect::<Vec<_>>());
    let pages = pages_of(&server, &root);
    assert!(!pages.is_empty() && pages.len() <= PAGES_MAX, "{} pages asked for", pages.len());
    assert_eq!(under(&listed, "Public Folders").len(), pages.len(), "one folder a page");
    assert_eq!(listed.missed.public, missed_under(&[&root]));
}

/// Found in review: the public folders' root whose second page failed failed the whole tree, none
/// listed. The page read is listed, and the root said not listed whole; its first page failing
/// still says none listed.
#[test]
fn a_root_whose_second_page_fails_keeps_its_first() {
    let (mut d, root) = with_root();
    for i in 1..=250 {
        d.folder(&format!("Team {i:03}"), Some(&root), None, "IPF.Note");
    }
    d.zones.public_failing.push((root.clone(), 2));
    let server = serve(d);
    let listed = list(&server);
    assert_eq!(under(&listed, "Public Folders").len(), 200);
    assert_eq!(listed.missed.public, missed_under(&[&root]));
    server.data.lock().unwrap().zones.public_failing = vec![(root, 1)];
    let listed = list(&server);
    assert!(under(&listed, "Public Folders").is_empty());
    assert_eq!(listed.missed.public, PublicMissed::All);
}

/// Public folders deeper than the walk goes: the deepest listed one, which holds others, is said
/// not listed whole (an earlier version may have listed what's under it).
#[test]
fn a_tree_deeper_than_walked_says_what_wasnt_listed() {
    let (mut d, root) = with_root();
    let mut ids = Vec::new();
    let mut parent = root;
    for level in 1..=DEPTH + 1 {
        parent = d.folder(&format!("Level {level}"), Some(&parent), None, "IPF.Note");
        ids.push(parent.clone());
    }
    let server = serve(d);
    let listed = list(&server);
    let deepest = (1..=DEPTH).map(|l| format!("Level {l}")).collect::<Vec<_>>().join("/");
    let paths: Vec<&str> = listed.folders.iter().map(|f| f.path.as_str()).collect();
    assert!(paths.contains(&format!("Public Folders/{deepest}").as_str()), "{paths:?}");
    assert!(!paths.iter().any(|p| p.ends_with(&format!("Level {}", DEPTH + 1))), "{paths:?}");
    assert_eq!(listed.missed.public, missed_under(&[&ids[DEPTH - 1]]));
}

/// The events of a full sync, from its listing (the folders told) till the account is online.
fn a_full_pass(pool: &crate::worker::Pool) -> Vec<Event> {
    let mut seen = until(pool, "the folders listed again", |e| matches!(e, Event::Folders { .. }));
    seen.extend(until(pool, "syncing", |e| matches!(e, Event::Status { status: SyncStatus::Syncing { .. }, .. })));
    seen.extend(until(pool, "online again", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. })));
    seen
}

/// Found in review: a full sync whose listing of a public folder's folders was refused (a level
/// under the root) took its branch for gone: the folder followed in it lost its cache and its
/// place in the folder pane. It keeps them.
#[test]
fn through_the_pool_a_public_branch_that_fails_keeps_its_cache() {
    let path = "Public Folders/Team/Projects";
    let (mut d, root) = with_root();
    let team = d.folder("Team", Some(&root), None, "IPF.Note");
    let projects = d.folder("Projects", Some(&team), None, "IPF.Note");
    d.deliver(&projects, &mail("Plans", "p1"), false, "2026-10-19T08:00:00Z");
    let server = serve(d);
    let data = server.data.clone();
    let store = crate::fake::tests_store("ews-public-branch-refused");
    store.save_account_doc("a1", FOLLOWED_PUBLIC_DOC, &[path]).unwrap();
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "riley@contoso.example".into(),
        username: format!("{DOMAIN}\\{USER}"),
        ews: Some(server.url.clone()),
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret(PASSWORD.into())) });
    let kept = || store.load_folder("a1", path).map(|f| f.messages).unwrap_or_default();
    eventually("Projects kept", || kept().len() == 1 && kept().iter().all(|m| store.has_body(&m.key)));
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let before = kept();
    while !pool.poll().is_empty() {}
    let asked = || data.lock().unwrap().zones.public_asked.iter().filter(|(p, _)| *p == team).count();
    let listed = asked();
    data.lock().unwrap().zones.public_failing.push((team.clone(), 1));
    pool.submit(Job::SyncAll { account: "a1".into() });
    let seen = a_full_pass(&pool);
    eventually("Team asked for again", || asked() > listed);
    drop(pool);
    let now = kept();
    assert_eq!(now, before, "its cache");
    assert!(now.iter().all(|m| store.has_body(&m.key)), "its sources");
    let last = seen.iter().rev().find_map(|e| match e {
        Event::Folders { folders, .. } => Some(folders.iter().map(|f| f.path.clone()).collect::<Vec<_>>()),
        _ => None,
    });
    let last = last.unwrap_or_default();
    assert!(last.iter().any(|p| p == path), "its place in the folder pane: {last:?}");
    let _ = std::fs::remove_dir_all(store.root());
}
