// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Tests over the fake EWS server: the account's online archive, listed under its own folder.

use quatrelle_model::{Folder, FolderRole};

use super::*;

mod policy;
pub(super) mod reading;
mod retention;

fn connect(server: &super::super::super::server::Server) -> crate::ews::Client {
    crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap()
}

fn listed(server: &super::super::super::server::Server) -> Vec<Folder> {
    crate::ews::list_folders(&connect(server), &mut crate::jmap::State::default(), "a1").unwrap()
}

/// Riley's mailbox (Inbox, Sent Items) and an archive "Online Archive - Riley" holding `Inbox`,
/// `2019/Projects` and its Deleted Items: the data, and the ids of the two Inboxes (the
/// account's, the archive's) and of `Projects`.
pub(super) fn with_archive() -> (Data, String, String, String) {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Sent Items", None, Some("sentitems"), "IPF.Note");
    let root = d.archive_mailbox("Online Archive - Riley");
    let old_inbox = d.archive_folder("Inbox", &root);
    let year = d.archive_folder("2019", &root);
    let projects = d.archive_folder("Projects", &year);
    d.archive_deleted_items(&root);
    (d, inbox, old_inbox, projects)
}

/// The archive Autodiscover names is listed after the account's own folders, under a folder
/// named as the server names it: every folder of it says it's the archive's and whose it is, and
/// none has a role in the account (its Inbox isn't the account's). It isn't one of the shared
/// mailboxes. Its tree is asked for routed to its address.
#[test]
fn the_online_archive_lists_under_its_own_top_folder() {
    let (d, ..) = with_archive();
    let server = serve(d);
    let c = connect(&server);
    let folders = crate::ews::list_folders(&c, &mut crate::jmap::State::default(), "a1").unwrap();
    let paths: Vec<&str> = folders.iter().map(|f| f.path.as_str()).collect();
    let top = "Online Archive - Riley";
    let want = [top.to_string(), format!("{top}/Inbox"), format!("{top}/2019"), format!("{top}/2019/Projects"), format!("{top}/Deleted Items")];
    for w in &want {
        assert!(paths.contains(&w.as_str()), "{w} in {paths:?}");
    }
    let archive: Vec<&Folder> = folders.iter().filter(|f| f.archive).collect();
    assert_eq!(archive.len(), want.len(), "{paths:?}");
    for f in &archive {
        assert!(want.contains(&f.path), "{}", f.path);
        assert_eq!(f.role, FolderRole::Other, "{}: no role in the account", f.path);
        assert_eq!(f.mailbox.as_deref(), Some("archive@contoso.example"), "{}", f.path);
        assert_eq!(f.shared_mailbox(), None, "{}: the archive isn't a shared mailbox", f.path);
    }
    let role_there = |leaf: &str| folders.iter().find(|f| f.path == format!("{top}/{leaf}")).map(|f| f.mailbox_role);
    assert_eq!((role_there("Deleted Items"), role_there("Inbox")), (Some(FolderRole::Trash), Some(FolderRole::Other)));
    // After the account's own folders, which are as they were.
    let first = folders.iter().position(|f| f.archive).unwrap();
    assert!(folders.iter().skip(first).all(|f| f.archive), "the archive comes last: {paths:?}");
    let own: Vec<&Folder> = folders.iter().filter(|f| !f.archive).collect();
    assert_eq!(own.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["INBOX", "Sent Items"]);
    assert!(own.iter().all(|f| f.mailbox.is_none()));
    assert_eq!(folders.iter().filter(|f| f.role == FolderRole::Inbox).count(), 1, "only the account's own Inbox is its Inbox");
    // Sorted for the folder pane: still after everything else.
    let mut sorted = folders.clone();
    quatrelle_model::sort_folders(&mut sorted);
    assert_eq!(sorted.iter().map(|f| f.archive).collect::<Vec<_>>(), [false, false, true, true, true, true, true]);
    assert_eq!(sorted[2].path, top);
    assert!(c.shared_mailboxes().is_empty(), "the archive isn't a shared mailbox");
    let d = server.data.lock().unwrap();
    assert!(d.anchors.iter().any(|a| a == "archive@contoso.example"), "{:?}", d.anchors);
    assert_eq!(d.calls.iter().filter(|c| *c == "FindItem" || *c == "GetItem").count(), 0, "listing the tree reads no mail");
}

/// No archive announced: none is listed, and nothing is asked of one.
#[test]
fn an_account_without_an_archive_lists_none() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Archive", None, Some("archive"), "IPF.Note");
    let server = serve(d);
    let folders = listed(&server);
    assert_eq!(folders.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["INBOX", "Archive"]);
    assert!(folders.iter().all(|f| !f.archive && f.mailbox.is_none()));
    assert_eq!(folders[1].role, FolderRole::Archive, "the mailbox's own Archive folder is no online archive");
    assert!(server.data.lock().unwrap().anchors.is_empty());
}

/// An archive that refuses (turned off, its server down): the account's own folders are listed
/// all the same, and the archive shows again once it answers.
#[test]
fn an_archive_that_refuses_lists_the_rest() {
    let (mut d, ..) = with_archive();
    d.archive.as_mut().unwrap().refused = true;
    let server = serve(d);
    let c = connect(&server);
    let mut st = crate::jmap::State::default();
    let folders = crate::ews::list_folders(&c, &mut st, "a1").unwrap();
    assert_eq!(folders.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["INBOX", "Sent Items"]);
    assert!(folders.iter().all(|f| !f.archive));
    server.data.lock().unwrap().archive.as_mut().unwrap().refused = false;
    let folders = crate::ews::list_folders(&c, &mut st, "a1").unwrap();
    assert_eq!(folders.iter().filter(|f| f.archive).count(), 5);
    // And gone again: none of its folders stays marked.
    server.data.lock().unwrap().archive.as_mut().unwrap().refused = true;
    let folders = crate::ews::list_folders(&c, &mut st, "a1").unwrap();
    assert_eq!(folders.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["INBOX", "Sent Items"]);
}

/// Review focus 2: the archive's name is already a folder's, or a shared mailbox's: both show,
/// the archive told apart by its address in brackets (the account's own when it has none), as a
/// shared mailbox is. An archive Autodiscover gives no name is "Online Archive".
#[test]
fn an_archive_named_like_a_folder_is_told_apart() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let own = d.folder("Old mail", None, None, "IPF.Note");
    d.deliver(&own, &mail("Kept", "k1"), true, "2026-10-19T08:00:00Z");
    let root = d.archive_mailbox("Old mail");
    d.archive_folder("2019", &root);
    let server = serve(d);
    let folders = listed(&server);
    let paths: Vec<&str> = folders.iter().map(|f| f.path.as_str()).collect();
    let top = "Old mail (archive@contoso.example)";
    assert_eq!(paths, ["INBOX", "Old mail", top, &format!("{top}/2019")]);
    assert_eq!(folders.iter().map(|f| f.archive).collect::<Vec<_>>(), [false, false, true, true]);
    assert_eq!(folders[1].total, 1, "the account's own folder is as it was");

    // A shared mailbox of that name: it keeps it, the archive is told apart.
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.shared_mailbox(SharedBox { address: "sales@contoso.example".into(), name: "Sales".into(), automap: true, legacy: false, granted: true });
    let root = d.archive_mailbox("Sales");
    d.archive_folder("2019", &root);
    let server = serve(d);
    let folders = listed(&server);
    let top = |path: &str| folders.iter().find(|f| f.path == path).map(|f| (f.archive, f.mailbox.clone()));
    assert_eq!(top("Sales"), Some((false, Some("sales@contoso.example".into()))));
    assert_eq!(top("Sales (archive@contoso.example)"), Some((true, Some("archive@contoso.example".into()))));
    assert_eq!(top("Sales/Inbox").map(|(a, _)| a), Some(false));

    // No name, no address: "Online Archive", and beside a folder so named, the account's address.
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Online Archive", None, None, "IPF.Note");
    d.archive = Some(ArchiveBox { name: String::new(), address: None, refused: false });
    let root = d.archive_folder("Top of Information Store", "");
    d.folders.last_mut().unwrap().known = Some("archivemsgfolderroot".into());
    d.archive_folder("2019", &root);
    let server = serve(d);
    let folders = listed(&server);
    let paths: Vec<&str> = folders.iter().map(|f| f.path.as_str()).collect();
    let top = "Online Archive (riley@contoso.example)";
    assert_eq!(paths, ["INBOX", "Online Archive", top, &format!("{top}/2019")]);
    assert_eq!(folders[2].mailbox.as_deref(), Some("archive"), "an archive without an address is still a mailbox of its own");
    assert!(server.data.lock().unwrap().anchors.is_empty(), "nothing to route by");
}

/// A hostile archive: ten thousand folders, names of five thousand characters with control
/// characters in them, a name of its own as long. The tree is capped, the names shortened.
#[test]
fn a_hostile_archive_is_capped() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let root = d.archive_mailbox(&format!("Online\u{7} Archive {}", "x".repeat(5_000)));
    d.archive_folder(&format!("Bell\u{7}\u{0}\u{1b} {}", "y".repeat(5_000)), &root);
    for i in 0..10_000 {
        d.archive_folder(&format!("F{i}"), &root);
    }
    let server = serve(d);
    let folders = listed(&server);
    let archive: Vec<&Folder> = folders.iter().filter(|f| f.archive).collect();
    assert!(archive.len() > 1_000 && archive.len() <= 5_000, "{} folders of the archive", archive.len());
    assert_eq!(folders.iter().filter(|f| !f.archive).count(), 1);
    for f in &archive {
        let decoded = quatrelle_model::utf7::decode(&f.path);
        assert!(decoded.split('/').all(|part| part.chars().count() <= 120), "{}", decoded.chars().count());
        assert!(!decoded.chars().any(char::is_control), "{decoded:?}");
    }
    let top = quatrelle_model::utf7::decode(&archive[0].path);
    assert!(top.starts_with("Online Archive xxx") && top.chars().count() == 120, "{top}");
    assert!(archive.iter().any(|f| quatrelle_model::utf7::decode(&f.path).contains("/Bell yyy")));
}
