// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Tests over the fake EWS server: the online archive read on the server, never synced nor kept.

use quatrelle_model::MessageKey;

use super::*;

fn old_mail(i: usize) -> Vec<u8> {
    format!(
        "From: Zoe Park <zoe@contoso.example>\r\nTo: riley@contoso.example\r\nSubject: ARCHIVE-SUBJECT-{i}\r\nMessage-ID: <old{i}@x>\r\n\r\nARCHIVE-BODY-{i} about the harbour.\r\n"
    )
    .into_bytes()
}

/// When message `i` of a folder was received: a minute after the one before, in January 2019.
fn minute(i: usize) -> String {
    format!("2019-01-{:02}T{:02}:{:02}:00Z", 1 + i / 1440, i / 60 % 24, i % 60)
}

fn account(server: &crate::fakes::fake_ews::server::Server) -> AccountConfig {
    AccountConfig {
        id: "a1".into(),
        email: "riley@contoso.example".into(),
        username: format!("{DOMAIN}\\{USER}"),
        ews: Some(server.url.clone()),
        ..Default::default()
    }
}

const TOP: &str = "Online Archive - Riley";
const PROJECTS: &str = "Online Archive - Riley/2019/Projects";
const OLD_INBOX: &str = "Online Archive - Riley/Inbox";

/// A full sync, and the events until the account is current again.
fn full_sync(pool: &mut crate::worker::Pool) -> Vec<Event> {
    pool.submit(Job::SyncAll { account: "a1".into() });
    // The folders listed again first: an "online" said before is of the sync before.
    let mut seen = until(pool, "the folders listed again", |e| matches!(e, Event::Folders { .. }));
    seen.extend(until(pool, "online again", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. })));
    seen
}

/// The background never takes a folder of the archive: after the first sync, a second pass, the
/// calendars and contacts, and with the server telling of changes as they happen, no item of an
/// archive folder was listed or given, none is kept, no subscription names one, and mail that
/// comes into the archive's Inbox changes nothing here. Its folders are listed with the server's
/// counts all the same.
#[test]
fn the_background_sync_never_takes_the_archive() {
    let (mut d, inbox, old_inbox, projects) = with_archive();
    d.streaming = true;
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.deliver(&inbox, &mail("Own", "o1"), false, "2026-10-19T08:00:00Z");
    for i in 0..40 {
        d.deliver(&projects, &old_mail(i), i % 2 == 0, &minute(i));
    }
    d.deliver(&old_inbox, &old_mail(100), false, &minute(100));
    let server = serve(d);
    let data = server.data.clone();
    let store = crate::fake::tests_store("ews-archive-background");
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    pool.submit(Job::Start { account: account(&server), password: Some(Secret(PASSWORD.into())) });
    let mut seen = until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    seen.extend(full_sync(&mut pool));
    // Mail comes into the archive's Inbox (the server moved it there), and into the account's.
    {
        let mut d = data.lock().unwrap();
        d.deliver(&old_inbox, &old_mail(101), false, &minute(101));
        d.deliver(&inbox, &mail("Own again", "o2"), false, "2026-10-19T09:00:00Z");
    }
    eventually("the account's new mail", || store.load_folder("a1", "INBOX").is_ok_and(|f| f.messages.len() == 2));
    // Its folders are listed with the server's counts at the next full sync (nothing tells of
    // the archive's new mail meanwhile).
    let counts = |path: &str| store.load_folders("a1").unwrap_or_default().iter().find(|f| f.path == path).map(|f| (f.total, f.unread));
    assert_eq!(counts(OLD_INBOX), Some((1, 1)));
    pool.submit(Job::SyncAll { account: "a1".into() });
    eventually("the server's counts", || counts(OLD_INBOX) == Some((2, 2)));
    assert_eq!(counts(PROJECTS), Some((40, 20)));
    seen.extend(full_sync(&mut pool));
    let folders = store.load_folders("a1").unwrap();
    let archive: Vec<&quatrelle_model::Folder> = folders.iter().filter(|f| f.archive).collect();
    assert_eq!(archive.len(), 5, "{folders:?}");
    for f in &archive {
        assert!(store.load_folder("a1", &f.path).unwrap().messages.is_empty(), "{}: nothing listed", f.path);
        assert!(
            !seen.iter().any(|e| match e {
                Event::Status { status: SyncStatus::Syncing { folder }, .. } => *folder == f.path,
                Event::FolderChanged { folder, .. } | Event::FolderSynced { folder, .. } => *folder == f.path,
                _ => false,
            }),
            "{}: taken by the sync",
            f.path
        );
    }
    let d = data.lock().unwrap();
    assert!(!d.found_in.iter().any(|(f, _, _)| d.in_archive(f)), "an archive folder's items were listed: {:?}", d.found_in);
    assert!(!d.given_from.iter().any(|f| d.in_archive(f)), "an archive folder's items were read");
    assert!(d.found_in.iter().any(|(f, _, _)| *f == inbox), "the account's own Inbox is read as before");
    assert!(!d.subscriptions.is_empty(), "the server tells of changes");
    for (_, scope) in &d.subscriptions {
        assert!(scope.as_ref().is_none_or(|ids| !ids.iter().any(|i| d.in_archive(i))), "the archive is listened to: {scope:?}");
    }
    drop(d);
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

/// A folder of the archive holding twenty thousand messages, opened: one page of its newest
/// comes (the worker's window for a folder read on the server), then older pages when asked,
/// each following the last with none twice. The server is never asked for more than a page, and
/// gives the properties of the first page's messages only.
#[test]
fn an_archive_folder_opened_reads_its_newest_then_older_pages() {
    let (mut d, _, _, projects) = with_archive();
    // The archive is in another database than the mailbox: its items are given only to a
    // request routed to it.
    d.anchored_items = true;
    for i in 0..20_000 {
        d.deliver(&projects, &old_mail(i), true, &minute(i));
    }
    let server = serve(d);
    let data = server.data.clone();
    let store = crate::fake::tests_store("ews-archive-pages");
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    pool.submit(Job::Start { account: account(&server), password: Some(Secret(PASSWORD.into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    assert_eq!(data.lock().unwrap().listings_of(&projects), (0, 0), "not before it's opened");
    let window = crate::worker::ONLINE_WINDOW;
    // Opened (the session asks for it, as for a public folder).
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: PROJECTS.into() });
    eventually("its newest", || store.load_folder("a1", PROJECTS).is_ok_and(|f| f.messages.len() == window));
    let subjects = |list: &[quatrelle_model::Summary]| list.iter().map(|m| m.subject.clone()).collect::<Vec<_>>();
    let expect = |from: usize, n: usize| (0..n).map(|k| format!("ARCHIVE-SUBJECT-{}", from - k)).collect::<Vec<_>>();
    let first = store.load_folder("a1", PROJECTS).unwrap().messages;
    assert_eq!(subjects(&first), expect(19_999, window));
    assert_eq!(store.load_folders("a1").unwrap().iter().find(|f| f.path == PROJECTS).map(|f| f.total), Some(20_000));
    {
        let d = data.lock().unwrap();
        let (listings, most) = d.listings_of(&projects);
        assert!(listings >= 1 && most <= window, "{listings} listings, {most} at most");
        assert_eq!(d.given_from.iter().filter(|f| **f == projects).count(), window, "only the first page's messages are read");
    }
    // Asked to keep more of it (not what the session does there): still its newest page.
    pool.submit(Job::MoreMail { account: "a1".into(), folder: PROJECTS.into(), more: 50_000 });
    // Scrolled past the end: the next page, then the one after.
    let mut older = |offset: u32| {
        pool.submit(Job::Older { account: "a1".into(), folder: PROJECTS.into(), offset });
        let seen = until(&pool, "an older page", |e| matches!(e, Event::Older { .. }));
        match seen.into_iter().last() {
            Some(Event::Older { folder, items, more, error, .. }) => {
                assert_eq!((folder.as_str(), error), (PROJECTS, None));
                (items, more)
            }
            other => panic!("{other:?}"),
        }
    };
    let (second, more) = older(0);
    assert!(more && second.len() == 100, "{}", second.len());
    assert_eq!(subjects(&second), expect(19_999 - window, 100));
    let (third, more) = older(100);
    assert!(more);
    assert_eq!(subjects(&third), expect(19_899 - window, 100));
    let mut keys: Vec<&MessageKey> = first.iter().chain(&second).chain(&third).map(|m| &m.key).collect();
    keys.sort_by_key(|k| k.uid);
    keys.dedup();
    assert_eq!(keys.len(), window + 200, "no message twice");
    assert_eq!(store.load_folder("a1", PROJECTS).unwrap().messages.len(), window, "older pages aren't kept, and it never keeps more");
    let d = data.lock().unwrap();
    let (_, most) = d.listings_of(&projects);
    assert!(most <= 100, "a listing asked for {most} at once");
    let asked: usize = d.found_in.iter().filter(|(f, _, _)| *f == projects).map(|(_, _, max)| *max).sum();
    assert!(asked <= 4 * window + 200, "{asked} messages asked for in all, of 20,000");
    assert_eq!(d.given_from.iter().filter(|f| **f == projects).count(), window);
    drop(d);
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

fn on_disk(dir: &std::path::Path, needle: &[u8]) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else { return false };
    rd.flatten().any(|e| {
        let p = e.path();
        if p.is_dir() { on_disk(&p, needle) } else { std::fs::read(&p).is_ok_and(|b| b.windows(needle.len()).any(|w| w == needle)) }
    })
}

/// Nothing of the archive's mail is kept on this computer: a folder of it opened, a message of
/// its list opened, an older page read and a message of it opened, a search of it, a message
/// marked read — and once Quatrelle closes, no subject and no text of the archive's mail is
/// anywhere under the data folders, while the account's own mail is there as before.
#[test]
fn nothing_of_the_archive_is_kept() {
    let (mut d, inbox, _, projects) = with_archive();
    d.anchored_items = true;
    d.deliver(&inbox, b"From: ana@studio.example\r\nSubject: KEPT-SUBJECT\r\nMessage-ID: <k1@x>\r\n\r\nKEPT-BODY\r\n", false, "2026-10-19T08:00:00Z");
    for i in 0..60 {
        d.deliver(&projects, &old_mail(i), false, &minute(i));
    }
    let server = serve(d);
    let data = server.data.clone();
    let t = std::env::temp_dir().join(format!("quatrelle-fake-ews-archive-kept-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&t);
    let store = Store::open_packed(t.join("app"), t.join("Quatrelle Files")).unwrap();
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    pool.submit(Job::Start { account: account(&server), password: Some(Secret(PASSWORD.into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    eventually("the account's own mail, whole", || store.load_folder("a1", "INBOX").is_ok_and(|f| f.messages.iter().any(|m| store.has_body(&m.key))));
    let window = crate::worker::ONLINE_WINDOW;
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: PROJECTS.into() });
    eventually("its newest", || store.load_folder("a1", PROJECTS).is_ok_and(|f| f.messages.len() == window));
    // A message of the list, opened as the session opens it.
    let listed = store.load_folder("a1", PROJECTS).unwrap().messages;
    let key = listed[0].key.clone();
    pool.submit(Job::FetchBody { key: key.clone() });
    let seen = until(&pool, "the opened message", |e| matches!(e, Event::Body { key: k, .. } if *k == key));
    assert!(seen.iter().any(|e| matches!(e, Event::Body { raw, .. } if String::from_utf8_lossy(raw).contains("ARCHIVE-BODY-59"))));
    // An older page, and a message of it.
    pool.submit(Job::Older { account: "a1".into(), folder: PROJECTS.into(), offset: 0 });
    let seen = until(&pool, "an older page", |e| matches!(e, Event::Older { .. }));
    let Some(Event::Older { items, .. }) = seen.into_iter().last() else { panic!("no older page") };
    assert_eq!(items.len(), 60 - window);
    let old = items[0].key.clone();
    pool.submit(Job::ReadOnline { key: old.clone() });
    let seen = until(&pool, "the older message", |e| matches!(e, Event::Body { key: k, .. } if *k == old));
    assert!(seen.iter().any(|e| matches!(e, Event::Body { raw, .. } if String::from_utf8_lossy(raw).contains("ARCHIVE-BODY-"))));
    // A search of the folder, both ways the session asks one.
    pool.submit(Job::Search { account: "a1".into(), query: "harbour".into(), folder: Some(PROJECTS.into()) });
    let seen = until(&pool, "the search", |e| matches!(e, Event::Found { .. }));
    assert!(seen.iter().any(|e| matches!(e, Event::Found { items, error: None, .. } if !items.is_empty())));
    pool.submit(Job::SearchServer { account: "a1".into(), folders: vec![PROJECTS.into()], query: "harbour".into(), search: 1 });
    until(&pool, "the server's search", |e| matches!(e, Event::ServerSearched { done: true, .. }));
    // Marked read: the mark reaches the server (the archive takes that change).
    pool.submit(Job::Apply { account: "a1".into(), op: Op::SetFlag { keys: vec![key.clone()], flag: Flag::Seen, on: true } });
    eventually("the read mark on the server", || data.lock().unwrap().items.iter().any(|i| i.folder == projects && i.read));
    assert_eq!(data.lock().unwrap().items.iter().filter(|i| i.folder == projects).count(), 60, "nothing moved or deleted");

    // Quatrelle closes (its threads let the data file go).
    let file = store.data_file("a1").unwrap();
    drop(pool);
    drop(store);
    let end = Instant::now() + Duration::from_secs(20);
    while std::fs::read(&file).is_err() {
        assert!(Instant::now() < end, "the data file stayed open");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(on_disk(&t, b"KEPT-SUBJECT") && on_disk(&t, b"KEPT-BODY"), "the account's own mail is kept");
    for needle in ["ARCHIVE-SUBJECT", "ARCHIVE-BODY", "old59@x", "harbour"] {
        assert!(!on_disk(&t, needle.as_bytes()), "{needle} reached the disk");
    }
    // The data file read again (what it keeps may be compressed): the folder list names the
    // archive's folders, and holds no list and no source of them.
    let store = Store::open_packed(t.join("app"), t.join("Quatrelle Files")).unwrap();
    let folders = store.load_folders("a1").unwrap();
    assert_eq!(folders.iter().filter(|f| f.archive).count(), 5);
    assert!(folders.iter().any(|f| f.archive && f.path == TOP));
    for f in folders.iter().filter(|f| f.archive) {
        assert!(store.load_folder("a1", &f.path).unwrap().messages.is_empty(), "{}: a list was kept", f.path);
        assert!(store.cached_bodies("a1", &f.path, 1).is_empty(), "{}: a source was kept", f.path);
        assert!(store.load_search_index("a1", &f.path).is_none(), "{}: an index was kept", f.path);
    }
    assert!(!store.has_body(&key) && !store.has_body(&old));
    let inbox = store.load_folder("a1", "INBOX").unwrap();
    assert!(inbox.messages.iter().any(|m| m.subject == "KEPT-SUBJECT" && store.has_body(&m.key)));
    drop(store);
    let _ = std::fs::remove_dir_all(&t);
}

/// Review focus 3: the archive goes away between two listings (turned off, its server down)
/// while one of its folders was open: its folders leave the list, what was held of them goes,
/// the account stays online and its own folders are as they were; a read asked for the folder
/// that went is said once and changes nothing. It shows again when it answers.
#[test]
fn an_archive_that_goes_away_leaves_the_view() {
    let (mut d, inbox, _, projects) = with_archive();
    d.deliver(&inbox, &mail("Own", "o1"), false, "2026-10-19T08:00:00Z");
    for i in 0..10 {
        d.deliver(&projects, &old_mail(i), true, &minute(i));
    }
    let server = serve(d);
    let data = server.data.clone();
    let store = crate::fake::tests_store("ews-archive-gone");
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    pool.submit(Job::Start { account: account(&server), password: Some(Secret(PASSWORD.into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: PROJECTS.into() });
    eventually("its mail", || store.load_folder("a1", PROJECTS).is_ok_and(|f| f.messages.len() == 10));
    data.lock().unwrap().archive.as_mut().unwrap().refused = true;
    let seen = full_sync(&mut pool);
    let listed = seen.iter().rev().find_map(|e| if let Event::Folders { folders, .. } = e { Some(folders.clone()) } else { None }).unwrap();
    assert_eq!(listed.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["INBOX", "Sent Items"]);
    assert!(!seen.iter().any(|e| matches!(e, Event::Status { status: SyncStatus::Error { .. } | SyncStatus::NeedsPassword { .. }, .. })), "{seen:?}");
    assert!(store.load_folder("a1", PROJECTS).unwrap().messages.is_empty(), "what was held of it goes");
    assert_eq!(store.load_folder("a1", "INBOX").unwrap().messages.len(), 1);
    // A read of the folder that went, asked for before the list came: no loop, the account
    // stays online.
    let calls = data.lock().unwrap().calls.len();
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: PROJECTS.into() });
    let seen = full_sync(&mut pool);
    assert!(!seen.iter().any(|e| matches!(e, Event::Status { status: SyncStatus::Error { .. }, .. })), "{seen:?}");
    assert!(data.lock().unwrap().calls.len() - calls < 60, "asked again and again");
    assert!(!data.lock().unwrap().found_in.iter().skip(1).any(|(f, o, _)| *f == projects && *o > 0));
    // It answers again: listed again, read again when opened.
    data.lock().unwrap().archive.as_mut().unwrap().refused = false;
    full_sync(&mut pool);
    assert_eq!(store.load_folders("a1").unwrap().iter().filter(|f| f.archive).count(), 5);
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: PROJECTS.into() });
    eventually("its mail again", || store.load_folder("a1", PROJECTS).is_ok_and(|f| f.messages.len() == 10));
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

/// A search of the archive runs on the server, in the folder asked for (`FindItem` with the
/// query); a search of the whole mailbox doesn't ask the archive.
#[test]
fn a_search_in_the_archive_runs_on_the_server() {
    let (mut d, inbox, old_inbox, projects) = with_archive();
    d.deliver(&inbox, &mail("harbour plans", "o1"), false, "2026-10-19T08:00:00Z");
    for i in 0..30 {
        d.deliver(&projects, &old_mail(i), true, &minute(i));
    }
    d.deliver(&old_inbox, &old_mail(500), true, &minute(500));
    let server = serve(d);
    let data = server.data.clone();
    let store = crate::fake::tests_store("ews-archive-search");
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    pool.submit(Job::Start { account: account(&server), password: Some(Secret(PASSWORD.into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let mut found = |folder: Option<&str>| {
        pool.submit(Job::Search { account: "a1".into(), query: "harbour".into(), folder: folder.map(str::to_string) });
        let seen = until(&pool, "the search", |e| matches!(e, Event::Found { .. }));
        match seen.into_iter().last() {
            Some(Event::Found { items, error: None, .. }) => items,
            other => panic!("{other:?}"),
        }
    };
    // In the folder: found there, mail that was never listed here (nothing of it is).
    let from = data.lock().unwrap().found_in.len();
    let items = found(Some(PROJECTS));
    assert_eq!(items.len(), 30);
    assert!(items.iter().all(|m| m.key.folder == PROJECTS && m.subject.starts_with("ARCHIVE-SUBJECT-")));
    assert_eq!(data.lock().unwrap().found_in[from..].iter().map(|(f, _, _)| f.clone()).collect::<Vec<_>>(), [projects.clone()]);
    // Of the whole mailbox: the account's own folders only.
    let from = data.lock().unwrap().found_in.len();
    let items = found(None);
    assert_eq!(items.iter().map(|m| (m.key.folder.as_str(), m.subject.as_str())).collect::<Vec<_>>(), [("INBOX", "harbour plans")]);
    let d = data.lock().unwrap();
    assert!(!d.found_in[from..].iter().any(|(f, _, _)| d.in_archive(f)), "a search of the mailbox asked the archive: {:?}", &d.found_in[from..]);
    drop(d);
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}
