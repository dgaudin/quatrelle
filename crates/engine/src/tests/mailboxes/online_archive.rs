// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: Exchange's online archive, read on the server inside an account kept here.

use quatrelle_model::{AccountConfig, Folder};

use super::*;

mod read_only;

const TOP: &str = "Online Archive - Riley";
const OLD_INBOX: &str = "Online Archive - Riley/Inbox";
const OLD_SENT: &str = "Online Archive - Riley/Sent Items";
const PROJECTS: &str = "Online Archive - Riley/2019/Projects";
const ADDRESS: &str = "archive@contoso.example";

fn own(id: &str, path: &str, role: FolderRole, total: u32) -> Folder {
    Folder { account: id.into(), path: path.into(), delimiter: Some('/'), role, selectable: true, total, ..Default::default() }
}

fn archived(id: &str, path: &str, total: u32) -> Folder {
    Folder { mailbox: Some(ADDRESS.into()), archive: true, ..own(id, path, FolderRole::Other, total) }
}

fn folders(id: &str, archive: bool) -> Vec<Folder> {
    let mut list =
        vec![own(id, "INBOX", FolderRole::Inbox, 1), own(id, "Sent Items", FolderRole::Sent, 0), own(id, "Deleted Items", FolderRole::Trash, 0)];
    if archive {
        list.extend([archived(id, TOP, 0), archived(id, OLD_INBOX, 2), archived(id, OLD_SENT, 1), archived(id, &format!("{TOP}/2019"), 0)]);
        list.push(archived(id, PROJECTS, 20_000));
        list.push(Folder { mailbox_role: FolderRole::Trash, ..archived(id, &format!("{TOP}/Deleted Items"), 0) });
    }
    list
}

fn message(id: &str, folder: &str, uid: u32, subject: &str) -> Summary {
    Summary {
        key: MessageKey { account: id.into(), folder: folder.into(), validity: 1, uid },
        subject: subject.into(),
        from: quatrelle_model::EmailAddress::new("Zoe Park", "zoe@contoso.example"),
        date: NOW - 2_000 * 86_400 - i64::from(uid),
        received: NOW - 2_000 * 86_400 - i64::from(uid),
        ..Default::default()
    }
}

/// What the worker holds of a folder once it has read it (in the archive, in memory only).
fn listed(s: &mut Session, store: &Store, id: &str, folder: &str, messages: Vec<Summary>) {
    store.save_folder(id, folder, &quatrelle_store::FolderState { validity: 1, messages, ..Default::default() }).unwrap();
    s.apply(Event::FolderChanged { account: id.into(), folder: folder.into() });
}

/// An Exchange account with an online archive, its Inbox on view.
fn with_archive(name: &str) -> (Session, Store, String) {
    let store = temp(name);
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    let config = AccountConfig {
        name: "Riley".into(),
        email: "riley@contoso.example".into(),
        username: "riley".into(),
        ews: Some("https://mail.contoso.example/EWS/Exchange.asmx".into()),
        ..Default::default()
    };
    let id = s.add_account(config, Some("pw".into())).unwrap();
    s.apply(Event::Folders { account: id.clone(), folders: folders(&id, true) });
    listed(&mut s, &store, &id, "INBOX", vec![Summary { subject: "harbour plans".into(), ..message(&id, "INBOX", 1, "") }]);
    s.run("folder.open", &json!({"account": id, "folder": "INBOX"})).unwrap();
    s.take_jobs();
    (s, store, id)
}

fn synced(jobs: &[Job]) -> Vec<String> {
    jobs.iter()
        .filter_map(|j| match j {
            Job::SyncFolder { folder, .. } => Some(folder.clone()),
            _ => None,
        })
        .collect()
}

/// A folder of the archive is read on the server each time it's opened, whatever the account
/// keeps and whether it's a Favorite; what's past its first page comes a page at a time, held in
/// memory, never by keeping more of the folder; "Download all" brings nothing of it.
#[test]
fn an_archive_folder_is_read_on_the_server_when_opened() {
    let (mut s, store, id) = with_archive("online-archive-opened");
    s.prefs.keep_days = 0;
    assert!(s.read_on_server(&id, PROJECTS) && s.read_on_server(&id, TOP) && !s.read_on_server(&id, "INBOX"));
    s.run("folder.favorite", &json!({"account": id, "folder": PROJECTS, "on": true})).unwrap();
    s.take_jobs();
    assert!(s.read_on_server(&id, PROJECTS), "a Favorite of the archive is read on the server all the same");
    s.run("folder.open", &json!({"account": id, "folder": PROJECTS})).unwrap();
    assert_eq!(synced(&s.take_jobs()), [PROJECTS]);
    assert!(s.list.is_empty(), "nothing of it is here before the server answers");
    // Its newest come.
    let page: Vec<Summary> = (1..=3).map(|uid| message(&id, PROJECTS, uid, &format!("Old {uid}"))).collect();
    listed(&mut s, &store, &id, PROJECTS, page);
    assert_eq!(s.list.len(), 3);
    assert_eq!(s.more_on_server().map(|m| m.2), Some(19_997));
    // Past them: a page read on the server, the next from where the last ended.
    let asked = |s: &mut Session| {
        let jobs = s.take_jobs();
        assert!(!jobs.iter().any(|j| matches!(j, Job::MoreMail { .. })), "an archive folder never keeps more of itself");
        jobs.into_iter().find_map(|j| match j {
            Job::Older { folder, offset, .. } if folder == PROJECTS => Some(offset),
            _ => None,
        })
    };
    s.run("folder.more", &json!({})).unwrap();
    assert_eq!(asked(&mut s), Some(0));
    let older: Vec<Summary> = (4..=5).map(|uid| message(&id, PROJECTS, uid, &format!("Old {uid}"))).collect();
    s.apply(Event::Older { account: id.clone(), folder: PROJECTS.into(), items: older, more: true, error: None });
    assert_eq!(s.list.len(), 5);
    s.run("folder.more", &json!({})).unwrap();
    assert_eq!(asked(&mut s), Some(2));
    // An older one opened: read on the server for showing.
    let old = message(&id, PROJECTS, 4, "").key;
    s.open_message(&old);
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::ReadOnline { key } if *key == old)));
    // Download all: the account's own mail, nothing of the archive.
    s.download_all(Some(&id), None).unwrap();
    let fetched: Vec<String> = s
        .take_jobs()
        .into_iter()
        .filter_map(|j| match j {
            Job::FetchBody { key } => Some(key.folder),
            _ => None,
        })
        .collect();
    assert_eq!(fetched, ["INBOX"]);
    assert!(s.download_all(Some(&id), Some(PROJECTS)).is_err(), "nor when it's asked for by name");
    // Opened again: read again.
    s.run("folder.open", &json!({"account": id, "folder": "INBOX"})).unwrap();
    s.take_jobs();
    s.run("folder.open", &json!({"account": id, "folder": PROJECTS})).unwrap();
    assert_eq!(synced(&s.take_jobs()), [PROJECTS]);
    let _ = std::fs::remove_dir_all(store.root());
}

/// Review focus 1: the archive's Inbox and Sent Items are never the account's. Mail that shows
/// up in the archive's Inbox is no new mail (no notice, no rule, not counted as unread Inbox
/// mail); the account's Inbox, Sent Items and Deleted Items are its own; nothing is sent from
/// the archive, and a reply to its mail is the account's.
#[test]
fn the_archives_inbox_isnt_the_accounts() {
    let (mut s, store, id) = with_archive("online-archive-inbox");
    let account = s.account(&id).unwrap();
    assert_eq!(account.folder(FolderRole::Inbox).map(|f| f.path.as_str()), Some("INBOX"));
    assert_eq!(account.folder(FolderRole::Sent).map(|f| f.path.as_str()), Some("Sent Items"));
    assert_eq!(account.folder(FolderRole::Trash).map(|f| f.path.as_str()), Some("Deleted Items"));
    // Beside a folder of the archive: the account's own, as the archive is the account's.
    assert_eq!(account.folder_mailbox(OLD_INBOX), None);
    assert_eq!(account.folder_beside(OLD_INBOX, FolderRole::Sent).map(|f| f.path.as_str()), Some("Sent Items"));
    assert_eq!(account.folder_beside(OLD_SENT, FolderRole::Trash).map(|f| f.path.as_str()), Some("Deleted Items"));
    assert_eq!(s.senders().iter().map(|x| x.email.as_str()).collect::<Vec<_>>(), ["riley@contoso.example"], "nothing is sent from the archive");
    // The archive's Inbox read, then read again with unread mail come since.
    let unread = |uid: u32, subject: &str| Summary { received: NOW - 60, date: NOW - 60, ..message(&id, OLD_INBOX, uid, subject) };
    listed(&mut s, &store, &id, OLD_INBOX, vec![unread(1, "Moved by the server")]);
    listed(&mut s, &store, &id, OLD_INBOX, vec![unread(2, "Moved since"), unread(1, "Moved by the server")]);
    assert!(s.take_new_mail().is_empty(), "the archive's Inbox announces nothing");
    let mut all = folders(&id, true);
    for f in all.iter_mut().filter(|f| f.path == OLD_INBOX) {
        f.unread = 2;
    }
    s.apply(Event::Folders { account: id.clone(), folders: all });
    assert_eq!(s.inbox_unread(), 0, "its unread mail isn't the Inbox's");
    // The account's own Inbox does, the same way.
    let fresh = |uid: u32, subject: &str| Summary { received: NOW - 60, date: NOW - 60, ..message(&id, "INBOX", uid, subject) };
    listed(&mut s, &store, &id, "INBOX", vec![fresh(1, "harbour plans")]);
    listed(&mut s, &store, &id, "INBOX", vec![fresh(2, "Just in"), fresh(1, "harbour plans")]);
    assert_eq!(s.take_new_mail().iter().map(|m| m.subject.as_str()).collect::<Vec<_>>(), ["Just in"]);
    // A unified Inbox holds the account's own only.
    s.open_view(View::Unified { role: FolderRole::Inbox });
    assert_eq!(s.scope_folders(SearchScope::Current), [(id.clone(), "INBOX".to_string())]);
    // A reply to the archive's mail is written in the account, from the account.
    s.run("folder.open", &json!({"account": id, "folder": OLD_INBOX})).unwrap();
    let key = message(&id, OLD_INBOX, 1, "").key;
    store
        .write_body(&key, b"From: Zoe Park <zoe@contoso.example>\r\nTo: riley@contoso.example\r\nSubject: Moved by the server\r\n\r\nHello.\r\n")
        .unwrap();
    s.open_message(&key);
    s.run("mail.reply", &json!({})).unwrap();
    let c = s.compose.as_ref().expect("a reply");
    assert_eq!((c.account.as_str(), c.from.as_str()), (id.as_str(), ""), "from the account's own address");
    let _ = std::fs::remove_dir_all(store.root());
}

/// Review focus 3: the archive goes from the folder list (turned off, its server down) while
/// one of its folders is on view: the view goes to the account's Inbox, once, with no read asked
/// of the folder that went and none at the lists that follow; it's opened again when it's back.
#[test]
fn an_archive_that_goes_away_leaves_the_view() {
    let (mut s, store, id) = with_archive("online-archive-gone");
    s.run("folder.open", &json!({"account": id, "folder": PROJECTS})).unwrap();
    s.take_jobs();
    listed(&mut s, &store, &id, PROJECTS, vec![message(&id, PROJECTS, 1, "Old 1")]);
    s.apply(Event::Older { account: id.clone(), folder: PROJECTS.into(), items: vec![message(&id, PROJECTS, 2, "Old 2")], more: true, error: None });
    assert_eq!(s.list.len(), 2);
    for _ in 0..3 {
        s.apply(Event::Folders { account: id.clone(), folders: folders(&id, false) });
        assert_eq!(s.view, View::Folder { account: id.clone(), folder: "INBOX".into() });
        assert_eq!(s.list.iter().map(|m| m.subject.as_str()).collect::<Vec<_>>(), ["harbour plans"]);
        let jobs = s.take_jobs();
        assert!(synced(&jobs).is_empty() && !jobs.iter().any(|j| matches!(j, Job::Older { .. })), "{jobs:?}");
    }
    assert!(!s.read_on_server(&id, PROJECTS), "a folder that's gone is nothing to read");
    assert!(s.account(&id).unwrap().folders.iter().all(|f| !f.archive));
    assert_eq!(s.more_on_server(), None, "the Inbox is all here");
    // Back: its folders show again, read when opened.
    s.apply(Event::Folders { account: id.clone(), folders: folders(&id, true) });
    assert!(synced(&s.take_jobs()).is_empty(), "nothing read until one is opened");
    s.run("folder.open", &json!({"account": id, "folder": PROJECTS})).unwrap();
    assert_eq!(synced(&s.take_jobs()), [PROJECTS]);
    let _ = std::fs::remove_dir_all(store.root());
}

fn searches(jobs: &[Job]) -> Vec<Option<String>> {
    jobs.iter()
        .filter_map(|j| match j {
            Job::Search { folder, .. } => Some(folder.clone()),
            _ => None,
        })
        .collect()
}

/// A search with a folder of the archive on view runs on the server, in that folder, whether
/// the box says the folder or the mailbox; what the server finds shows, mail that was never
/// listed here.
#[test]
fn a_search_in_the_archive_runs_on_the_server() {
    let (mut s, store, id) = with_archive("online-archive-search");
    s.run("folder.open", &json!({"account": id, "folder": PROJECTS})).unwrap();
    s.take_jobs();
    for scope in ["mailbox", "current", "subfolders"] {
        s.run("search.run", &json!({"query": "harbour", "scope": scope})).unwrap();
        let jobs = s.take_jobs();
        assert_eq!(searches(&jobs), [Some(PROJECTS.to_string())], "{scope}");
        assert!(s.list.is_empty(), "{scope}: the account's own mail isn't looked in from the archive");
        s.run("search.run", &json!({"query": ""})).unwrap();
        s.take_jobs();
    }
    s.run("search.run", &json!({"query": "harbour", "scope": "mailbox"})).unwrap();
    s.take_jobs();
    s.apply(Event::Found {
        account: id.clone(),
        query: "harbour".into(),
        items: vec![message(&id, PROJECTS, 9, "The harbour in 2019")],
        error: None,
    });
    assert_eq!(s.list.iter().map(|m| m.subject.as_str()).collect::<Vec<_>>(), ["The harbour in 2019"]);
    // Asked of the server by hand ("search the server"): that folder too.
    s.run("search.server", &json!({})).unwrap();
    let asked: Vec<Vec<String>> = s
        .take_jobs()
        .into_iter()
        .filter_map(|j| match j {
            Job::SearchServer { folders, .. } => Some(folders),
            _ => None,
        })
        .collect();
    assert_eq!(asked, [vec![PROJECTS.to_string()]]);
    let _ = std::fs::remove_dir_all(store.root());
}

/// A search of the mailbox, or of every mailbox, doesn't look in the archive: its folders aren't
/// among those searched here nor those the server is asked about, what's held of one that was
/// opened isn't found, and from the archive itself "all mailboxes" is the mailboxes.
#[test]
fn a_search_of_all_mailboxes_skips_the_archive() {
    let (mut s, store, id) = with_archive("online-archive-search-all");
    // A folder of the archive was opened: its first page is held.
    listed(&mut s, &store, &id, PROJECTS, vec![message(&id, PROJECTS, 1, "The harbour in 2019")]);
    for scope in [SearchScope::All, SearchScope::Mailbox] {
        let folders = s.scope_folders(scope);
        assert!(folders.contains(&(id.clone(), "INBOX".to_string())) && folders.iter().all(|(_, f)| !f.starts_with(TOP)), "{scope:?}: {folders:?}");
    }
    for scope in ["all", "mailbox"] {
        s.run("search.run", &json!({"query": "harbour", "scope": scope})).unwrap();
        assert_eq!(searches(&s.take_jobs()), [None], "{scope}: the whole account, which the worker asks without its archive");
        assert_eq!(s.list.iter().map(|m| m.subject.as_str()).collect::<Vec<_>>(), ["harbour plans"], "{scope}");
        s.run("search.server", &json!({})).unwrap();
        let jobs = s.take_jobs();
        let asked: Vec<&String> =
            jobs.iter().filter_map(|j| if let Job::SearchServer { folders, .. } = j { Some(folders) } else { None }).flatten().collect();
        assert!(asked.iter().any(|f| *f == "INBOX") && asked.iter().all(|f| !f.starts_with(TOP)), "{scope}: {asked:?}");
        s.run("search.run", &json!({"query": ""})).unwrap();
        s.take_jobs();
    }
    // From a folder of the archive, every mailbox is still the mailboxes.
    s.run("folder.open", &json!({"account": id, "folder": PROJECTS})).unwrap();
    s.take_jobs();
    s.run("search.run", &json!({"query": "harbour", "scope": "all"})).unwrap();
    assert_eq!(searches(&s.take_jobs()), [None]);
    assert_eq!(s.list.iter().map(|m| m.subject.as_str()).collect::<Vec<_>>(), ["harbour plans"]);
    let _ = std::fs::remove_dir_all(store.root());
}
