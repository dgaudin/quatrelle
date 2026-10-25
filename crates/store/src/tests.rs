use quatrelle_model::{Flag, Flags};

use super::*;

/// A big folder's list is kept in pages by number: read back newest first as it was; a flag
/// change writes one page, new mail the last one; shrunk, the pages go; and a list kept whole
/// (from before) still reads.
/// Files handed to other apps go in a folder of the user's own, made when missing; on Linux
/// and the BSDs not in the shared /tmp, and owner-only.
#[test]
fn scratch_folders_are_the_users_own() {
    let a = scratch_dir("test-scratch").unwrap();
    assert!(a.is_dir());
    assert_eq!(scratch_dir("test-scratch"), Some(a.clone()), "the same folder each time");
    if cfg!(all(unix, not(target_os = "macos"))) {
        assert!(!a.starts_with("/tmp"), "{}", a.display());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let parent = a.parent().unwrap();
            assert_eq!(fs::metadata(parent).unwrap().permissions().mode() & 0o777, 0o700);
        }
    }
    let _ = fs::remove_dir(&a);
}

#[test]
fn a_big_folder_list_is_kept_in_pages() {
    for packed in [false, true] {
        let dir = std::env::temp_dir().join(format!("quatrelle-pages-{packed}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = if packed { Store::open_packed(dir.join("app"), dir.join("Files")).unwrap() } else { Store::open(&dir).unwrap() };
        let n = PAGE_MESSAGES as u32 * 2 + 500;
        let msg = |uid: u32| Summary {
            key: MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 3, uid },
            subject: format!("Report {uid}"),
            date: 1_700_000_000 + i64::from(uid % 977) * 60,
            ..Default::default()
        };
        let mut st = FolderState { validity: 3, uid_next: n + 1, messages: (1..=n).map(msg).collect(), ..Default::default() };
        sort_newest_first(&mut st.messages);
        assert_eq!(s.save_folder_counted("a1", "INBOX", &st, None).unwrap(), 4, "three pages and the list");
        let back = s.load_folder("a1", "INBOX").unwrap();
        assert_eq!(back, st, "{packed}: read back newest first, as it was");
        // Its version: the same while the list is, whatever sources are written.
        let version = s.folder_version("a1", "INBOX");
        s.write_body(&msg(7).key, b"Subject: x\r\n\r\nhi").unwrap();
        assert_eq!((s.folder_version("a1", "INBOX"), version == 0), (version, false));
        // A flag: one page.
        if let Some(m) = st.messages.iter_mut().find(|m| m.key.uid == 10) {
            m.flags.seen = true;
        }
        assert_eq!(s.save_folder_counted("a1", "INBOX", &st, None).unwrap(), 2, "one page and the list");
        assert_ne!(s.folder_version("a1", "INBOX"), version, "a page changed: so did the version");
        assert!(s.load_folder("a1", "INBOX").unwrap().find(10).unwrap().flags.seen);
        // Told which changed: the same, without looking at the rest.
        if let Some(m) = st.messages.iter_mut().find(|m| m.key.uid == 8000) {
            m.flags.flagged = true;
        }
        let changed: std::collections::HashSet<u32> = [8000].into_iter().collect();
        assert_eq!(s.save_folder_counted("a1", "INBOX", &st, Some(&changed)).unwrap(), 2);
        assert_eq!(s.load_folder("a1", "INBOX").unwrap(), st);
        // New mail: the last page.
        st.messages.insert(0, msg(n + 1));
        st.uid_next = n + 2;
        assert_eq!(s.save_folder_counted("a1", "INBOX", &st, None).unwrap(), 2);
        assert_eq!(s.load_folder("a1", "INBOX").unwrap().messages.len(), n as usize + 1);
        // Told only of a flag, but a message came too: every page looked at, so it's kept.
        st.messages.insert(0, msg(n + 2));
        st.uid_next = n + 3;
        assert_eq!(s.save_folder_counted("a1", "INBOX", &st, Some(&changed)).unwrap(), 2);
        assert_eq!(s.load_folder("a1", "INBOX").unwrap().messages.len(), n as usize + 2);
        // Nothing changed: the list alone.
        assert_eq!(s.save_folder_counted("a1", "INBOX", &st, None).unwrap(), 1);
        // Shrunk to a small folder: kept whole again, the pages gone.
        st.messages.truncate(10);
        s.save_folder("a1", "INBOX", &st).unwrap();
        assert_eq!(s.load_folder("a1", "INBOX").unwrap(), st);
        assert!(!s.files.is_file(&s.folder_dir("a1", "INBOX").join("state-0.json")));
        assert_eq!(s.folder_version("a1", "Nowhere"), 0, "no list, no version");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

pub(crate) fn temp_store(name: &str) -> Store {
    let dir = std::env::temp_dir().join(format!("quatrelle-store-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    Store::open(dir).unwrap()
}

fn msg(uid: u32, seen: bool) -> Summary {
    Summary {
        key: MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 3, uid },
        subject: format!("m{uid}"),
        flags: Flags { seen, ..Default::default() },
        ..Default::default()
    }
}

#[test]
fn accounts_folders_and_state_round_trip() {
    let s = temp_store("rt");
    assert!(s.load_accounts().unwrap().is_empty());
    let acc = AccountConfig { id: "a1".into(), email: "z@x".into(), ..Default::default() };
    s.save_accounts(std::slice::from_ref(&acc)).unwrap();
    assert_eq!(s.load_accounts().unwrap(), vec![acc]);

    let f = Folder { account: "a1".into(), path: "Projets/Caf&AOk-".into(), delimiter: Some('/'), ..Default::default() };
    s.save_folders("a1", std::slice::from_ref(&f)).unwrap();
    assert_eq!(s.load_folders("a1").unwrap(), vec![f]);

    let st = FolderState { validity: 3, uid_next: 10, highest_modseq: 77, messages: vec![msg(9, false), msg(4, true)], window: 0 };
    s.save_folder("a1", "INBOX", &st).unwrap();
    let back = s.load_folder("a1", "INBOX").unwrap();
    assert_eq!(back, st);
    assert_eq!(back.unread(), 1);

    let k = &st.messages[0].key;
    assert!(!s.has_body(k));
    s.write_body(k, b"Subject: x\r\n\r\nhi").unwrap();
    assert_eq!(s.read_body(k).unwrap(), b"Subject: x\r\n\r\nhi");
    assert_eq!(s.cached_bodies("a1", "INBOX", 3), vec![9]);
    s.clear_folder("a1", "INBOX").unwrap();
    assert!(!s.has_body(k));
    assert_eq!(s.load_folder("a1", "INBOX").unwrap(), FolderState::default());
    s.remove_account("a1").unwrap();
    let _ = fs::remove_dir_all(s.root());
}

#[test]
fn damaged_files_start_fresh() {
    let s = temp_store("bad");
    fs::write(s.root().join("accounts.json"), b"{not json").unwrap();
    assert!(s.load_accounts().unwrap().is_empty());
    assert!(s.root().join("accounts.json.bad").exists());
    let _ = fs::remove_dir_all(s.root());
}

#[test]
fn ops_apply_locally_and_persist() {
    let s = temp_store("ops");
    let st = FolderState { validity: 3, uid_next: 10, highest_modseq: 0, messages: vec![msg(9, false), msg(4, false), msg(2, false)], window: 0 };
    s.save_folder("a1", "INBOX", &st).unwrap();
    let k9 = st.messages[0].key.clone();
    let k4 = st.messages[1].key.clone();

    let changed = s.apply_op("a1", &Op::SetFlag { keys: vec![k9.clone()], flag: Flag::Seen, on: true }).unwrap();
    assert_eq!(changed, vec!["INBOX".to_string()]);
    assert!(s.load_folder("a1", "INBOX").unwrap().find(9).unwrap().flags.seen);

    s.apply_op("a1", &Op::Move { keys: vec![k4.clone()], to: "Archive".into() }).unwrap();
    assert!(s.load_folder("a1", "INBOX").unwrap().find(4).is_none());

    s.write_body(&k9, b"x").unwrap();
    s.apply_op("a1", &Op::Expunge { keys: vec![k9.clone()] }).unwrap();
    assert!(!s.has_body(&k9));
    assert_eq!(s.load_folder("a1", "INBOX").unwrap().messages.len(), 1);

    // A stale key (old UIDVALIDITY) changes nothing.
    let mut stale = st.messages[2].key.clone();
    stale.validity = 1;
    s.apply_op("a1", &Op::Expunge { keys: vec![stale] }).unwrap();
    assert_eq!(s.load_folder("a1", "INBOX").unwrap().messages.len(), 1);

    let ops = vec![Op::Move { keys: vec![k4], to: "Archive".into() }];
    s.save_ops("a1", &ops).unwrap();
    assert_eq!(s.load_ops("a1").unwrap(), ops);
    s.save_ops("a1", &[]).unwrap();
    assert!(s.load_ops("a1").unwrap().is_empty());
    let _ = fs::remove_dir_all(s.root());
}

/// A sync reads a folder, an edit is made meanwhile (a message opened, so marked read; another
/// moved away), and the sync saves what it read: the edits, still on their way to the server,
/// stay. The old state put back made a read message go bold again until the edit got there.
#[test]
fn a_folder_saved_from_an_older_read_keeps_the_edits_waiting() {
    let s = temp_store("stale-save");
    let st = FolderState { validity: 3, uid_next: 10, highest_modseq: 0, messages: vec![msg(9, false), msg(4, false), msg(2, false)], window: 0 };
    s.save_folder("a1", "INBOX", &st).unwrap();
    // What the sync read, before the edits.
    let read_by_sync = s.load_folder("a1", "INBOX").unwrap();
    let (k9, k4) = (st.messages[0].key.clone(), st.messages[1].key.clone());
    for op in [Op::SetFlag { keys: vec![k9.clone()], flag: Flag::Seen, on: true }, Op::Move { keys: vec![k4.clone()], to: "Archive".into() }] {
        s.add_op("a1", op.clone()).unwrap();
        s.apply_op("a1", &op).unwrap();
    }
    // The sync saves what it read (the server's flags then): the edits are made again on it.
    s.save_folder("a1", "INBOX", &read_by_sync).unwrap();
    let now = s.load_folder("a1", "INBOX").unwrap();
    assert!(now.find(9).unwrap().flags.seen, "still read");
    assert!(now.find(4).is_none(), "still moved away");
    assert!(!now.find(2).unwrap().flags.seen, "the others as the sync had them");
    // Once the server has them (the journal emptied), what a sync saves is the server's word.
    s.save_ops("a1", &[]).unwrap();
    s.save_folder("a1", "INBOX", &read_by_sync).unwrap();
    assert!(!s.load_folder("a1", "INBOX").unwrap().find(9).unwrap().flags.seen);
    let _ = fs::remove_dir_all(s.root());
}

/// A folder's search index (and any document kept with it) is kept beside its messages and
/// goes with them: a folder cleared (its UIDVALIDITY changed, or it went) loses it.
#[test]
fn a_search_index_is_kept_with_its_folder() {
    for packed in [false, true] {
        let dir = std::env::temp_dir().join(format!("quatrelle-index-{packed}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let s = if packed { Store::open_packed(dir.join("app"), dir.join("Files")).unwrap() } else { Store::open(&dir).unwrap() };
        assert!(s.load_search_index("a1", "INBOX").is_none());
        s.save_search_index("a1", "INBOX", b"MCFI\x01kept").unwrap();
        assert_eq!(s.load_search_index("a1", "INBOX").as_deref(), Some(&b"MCFI\x01kept"[..]));
        // Another folder, or another account, has its own.
        assert!(s.load_search_index("a1", "Sent").is_none());
        assert!(s.load_search_index("a2", "INBOX").is_none());
        s.save_search_index("a1", "Sent", b"sent").unwrap();
        assert_eq!(s.load_search_index("a1", "INBOX").as_deref(), Some(&b"MCFI\x01kept"[..]));
        // So is a document kept with it.
        s.save_folder_doc("a1", "INBOX", "kept.json", &vec![7u32]).unwrap();
        assert_eq!(s.load_folder_doc::<Vec<u32>>("a1", "INBOX", "kept.json").unwrap(), vec![7]);
        assert!(s.load_folder_doc::<Vec<u32>>("a1", "Sent", "kept.json").unwrap().is_empty());
        s.clear_folder("a1", "INBOX").unwrap();
        assert!(s.load_search_index("a1", "INBOX").is_none());
        assert!(s.load_folder_doc::<Vec<u32>>("a1", "INBOX", "kept.json").unwrap().is_empty());
        assert!(s.load_search_index("a1", "Sent").is_some());
        s.remove_search_index("a1", "Sent");
        assert!(s.load_search_index("a1", "Sent").is_none());
        let _ = fs::remove_dir_all(s.root());
    }
}

#[test]
fn folder_ops_move_caches_with_folders() {
    let s = temp_store("folders");
    let f = |p: &str| Folder {
        account: "a1".into(),
        path: p.into(),
        delimiter: Some('/'),
        role: quatrelle_model::FolderRole::from_name(p, Some('/')),
        selectable: true,
        ..Default::default()
    };
    s.save_folders("a1", &[f("INBOX"), f("Projects"), f("Projects/Film")]).unwrap();
    let film = MessageKey { account: "a1".into(), folder: "Projects/Film".into(), validity: 3, uid: 9 };
    s.save_folder("a1", "Projects/Film", &FolderState { validity: 3, uid_next: 10, highest_modseq: 0, messages: vec![msg(9, false)], window: 0 })
        .unwrap();
    s.write_body(&film, b"Subject: x\r\n\r\nfilm").unwrap();

    assert_eq!(s.apply_op("a1", &Op::CreateFolder { path: "Clients".into() }).unwrap(), vec!["Clients".to_string()]);
    // Creating twice keeps one.
    s.apply_op("a1", &Op::CreateFolder { path: "Clients".into() }).unwrap();
    assert_eq!(s.load_folders("a1").unwrap().iter().filter(|f| f.path == "Clients").count(), 1);

    // Renaming moves the subfolder and its cached mail along.
    let changed = s.apply_op("a1", &Op::RenameFolder { from: "Projects".into(), to: "Clients/Work".into() }).unwrap();
    assert_eq!(changed, vec!["Clients/Work".to_string(), "Clients/Work/Film".to_string()]);
    let paths: Vec<String> = s.load_folders("a1").unwrap().into_iter().map(|f| f.path).collect();
    assert!(paths.contains(&"Clients/Work/Film".to_string()) && !paths.contains(&"Projects".to_string()), "{paths:?}");
    let moved = s.load_folder("a1", "Clients/Work/Film").unwrap();
    assert_eq!(moved.messages.len(), 1);
    assert_eq!(s.read_body(&moved.messages[0].key).unwrap(), b"Subject: x\r\n\r\nfilm");
    assert!(s.load_folder("a1", "Projects/Film").unwrap().messages.is_empty());

    // Deleting takes the subfolders and their caches.
    s.apply_op("a1", &Op::DeleteFolder { path: "Clients/Work".into() }).unwrap();
    let paths: Vec<String> = s.load_folders("a1").unwrap().into_iter().map(|f| f.path).collect();
    assert_eq!(paths, vec!["INBOX".to_string(), "Clients".to_string()]);
    assert!(!s.has_body(&moved.messages[0].key));
    let _ = fs::remove_dir_all(s.root());
}

#[test]
fn outbox_queues_and_forgets() {
    let s = temp_store("outbox");
    assert!(s.outbox("a1").is_empty());
    let item = |id, queued| OutboxItem { id, from: "z@x".into(), recipients: vec!["a@y".into()], queued, ..Default::default() };
    s.outbox_add("a1", &item(7, 20), b"Subject: later\r\n\r\nx").unwrap();
    s.outbox_add("a1", &item(3, 10), b"Subject: first\r\n\r\nx").unwrap();
    let ids: Vec<u64> = s.outbox("a1").iter().map(|i| i.id).collect();
    assert_eq!(ids, vec![3, 7], "oldest first");
    let mut i = s.outbox("a1")[0].clone();
    i.attempts = 2;
    i.error = Some("offline".into());
    s.outbox_update("a1", &i).unwrap();
    assert_eq!(s.outbox("a1")[0], i);
    assert_eq!(s.outbox_raw("a1", 7).unwrap(), b"Subject: later\r\n\r\nx");
    s.outbox_remove("a1", 3);
    assert_eq!(s.outbox("a1").len(), 1);
    // Damaged bookkeeping is skipped, not fatal.
    fs::write(s.root().join("a1").join("outbox").join("9.json"), b"{nope").unwrap();
    assert_eq!(s.outbox("a1").len(), 1);
    let _ = fs::remove_dir_all(s.root());
}

#[test]
fn hostile_names_stay_inside_the_root() {
    let s = temp_store("names");
    let key = MessageKey { account: "../../etc".into(), folder: "..\\..\\x".into(), validity: 1, uid: 1 };
    let p = s.body_path(&key);
    assert!(p.starts_with(s.root()));
    assert!(!p.to_string_lossy().contains(".."));
    let _ = fs::remove_dir_all(s.root());
}

#[test]
fn a_memory_cache_reports_its_changes_once_asked() {
    let s = Store::memory();
    s.save_doc("rules.json", &vec![1]).unwrap();
    assert!(s.take_changes().is_empty(), "not tracked yet");
    s.save_folders("a1", &[Folder { account: "a1".into(), path: "Projects".into(), delimiter: Some('/'), ..Default::default() }]).unwrap();
    s.track_changes();
    s.save_doc("rules.json", &vec![2]).unwrap();
    let key = MessageKey { account: "a1".into(), folder: "Projects".into(), validity: 1, uid: 4 };
    s.write_body(
        &key,
        b"Subject: x

hi",
    )
    .unwrap();
    let changes = s.take_changes();
    assert_eq!(changes.len(), 2, "{changes:?}");
    assert!(changes.contains(&("rules.json".into(), Some(b"[2]".to_vec()))));
    let body = changes.iter().find(|(p, _)| p.ends_with("/4.eml")).unwrap();
    assert!(body.0.starts_with("a1/f"));
    assert!(s.take_changes().is_empty(), "taken");
    // A rename is a removal and a write; removing an account removes each file.
    s.apply_op("a1", &Op::RenameFolder { from: "Projects".into(), to: "Work".into() }).unwrap();
    let changes = s.take_changes();
    assert!(changes.iter().any(|(p, v)| p == &body.0 && v.is_none()), "{changes:?}");
    assert!(changes.iter().any(|(p, v)| p.ends_with("/4.eml") && p != &body.0 && v.is_some()));
    s.remove_account("a1").unwrap();
    assert!(s.take_changes().iter().filter(|(p, _)| p.ends_with(".eml")).all(|(_, v)| v.is_none()));
    // A store kept from them comes back the same.
    let back = Store::memory_from(s.snapshot(|_| true));
    assert_eq!(back.load_doc::<Vec<i32>>("rules.json").unwrap(), vec![2]);
}

#[test]
fn a_memory_cache_works_like_the_disk_and_keeps_a_snapshot() {
    let s = Store::memory();
    assert!(s.is_memory());
    let acc = AccountConfig { id: "a1".into(), email: "z@x".into(), ..Default::default() };
    s.save_accounts(std::slice::from_ref(&acc)).unwrap();
    let folders = vec![
        Folder { account: "a1".into(), path: "Projects".into(), delimiter: Some('/'), ..Default::default() },
        Folder { account: "a1".into(), path: "Projects/Film".into(), delimiter: Some('/'), ..Default::default() },
    ];
    s.save_folders("a1", &folders).unwrap();
    let st = FolderState { validity: 3, uid_next: 3, messages: vec![msg(2, false), msg(1, true)], ..Default::default() };
    s.save_folder("a1", "Projects/Film", &st).unwrap();
    let key = MessageKey { account: "a1".into(), folder: "Projects/Film".into(), validity: 3, uid: 2 };
    s.write_body(
        &key,
        b"Subject: m2

hi",
    )
    .unwrap();
    assert!(s.has_body(&key));
    assert_eq!(s.cached_bodies("a1", "Projects/Film", 3), vec![2]);
    assert!(s.account_size("a1") > 0);

    // A renamed folder takes its cache with it, children too.
    s.apply_op("a1", &Op::RenameFolder { from: "Projects".into(), to: "Work".into() }).unwrap();
    assert_eq!(s.load_folder("a1", "Work/Film").unwrap().messages.len(), 2);
    let moved = MessageKey { folder: "Work/Film".into(), ..key.clone() };
    assert_eq!(
        s.read_body(&moved).unwrap(),
        b"Subject: m2

hi"
    );
    assert!(s.load_folder("a1", "Projects/Film").unwrap().messages.is_empty());
    s.apply_op("a1", &Op::SetFlag { keys: vec![MessageKey { uid: 2, ..moved.clone() }], flag: Flag::Seen, on: true }).unwrap();
    assert!(s.load_folder("a1", "Work/Film").unwrap().messages.iter().all(|m| m.flags.seen));

    let item = OutboxItem { id: 5, from: "z@x".into(), recipients: vec!["a@y".into()], ..Default::default() };
    s.outbox_add("a1", &item, b"x").unwrap();
    assert_eq!(s.outbox("a1"), vec![item]);
    s.outbox_remove("a1", 5);
    assert!(s.outbox("a1").is_empty());

    // What outlasts the page: here everything but message sources.
    let snap = s.snapshot(|p| !p.ends_with(".eml"));
    assert!(snap.iter().any(|(p, _)| p == "accounts.json"));
    assert!(snap.iter().all(|(p, _)| !p.ends_with(".eml")));
    let back = Store::memory_from(snap);
    assert_eq!(back.load_accounts().unwrap(), vec![acc]);
    assert_eq!(back.load_folder("a1", "Work/Film").unwrap().messages.len(), 2);
    assert!(!back.has_body(&moved));
    // Paths from a snapshot stay inside the cache.
    let odd = Store::memory_from(vec![("../accounts.json".into(), b"[]".to_vec()), ("a//b".into(), b"x".to_vec())]);
    assert!(odd.snapshot(|_| true).is_empty());

    s.remove_account("a1").unwrap();
    assert!(s.load_folders("a1").unwrap().is_empty() && s.account_size("a1") == 0);
    // Clones share the cache; a damaged document starts fresh.
    let other = s.clone();
    other.save_doc("rules.json", &vec![1, 2]).unwrap();
    assert_eq!(s.load_doc::<Vec<i32>>("rules.json").unwrap(), vec![1, 2]);
    let bad = Store::memory_from(vec![("rules.json".into(), b"{oops".to_vec())]);
    assert!(bad.load_doc::<Vec<i32>>("rules.json").unwrap().is_empty());
}

/// Folder Size (Local Data): what each folder's list and messages take here, named by its place,
/// in a data file as in a folder of files.
#[test]
fn each_folder_tells_the_space_it_takes_here() {
    for packed in [false, true] {
        let dir = std::env::temp_dir().join(format!("quatrelle-sizes-{packed}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = if packed { Store::open_packed(dir.join("app"), dir.join("Files")).unwrap() } else { Store::open(&dir).unwrap() };
        let folder = |path: &str| Folder { account: "a1".into(), path: path.into(), delimiter: Some('/'), selectable: true, ..Default::default() };
        s.save_folders("a1", &[folder("INBOX"), folder("INBOX/Caf&AOk-")]).unwrap();
        let key = MessageKey { account: "a1".into(), folder: "INBOX/Caf&AOk-".into(), validity: 1, uid: 1 };
        // Bytes that don't compress (a data file keeps sources compressed: what's measured is
        // the space they take).
        let mut n: u32 = 7;
        let raw: Vec<u8> = (0..5000)
            .map(|_| {
                n = n.wrapping_mul(1_103_515_245).wrapping_add(12345);
                (n >> 16) as u8
            })
            .collect();
        s.write_body(&key, &raw).unwrap();
        let sizes = s.folder_sizes("a1").unwrap();
        let paths: Vec<(Vec<String>, Option<String>)> = sizes.iter().map(|f| (f.path.clone(), f.folder.clone())).collect();
        assert_eq!(
            paths,
            [(vec!["INBOX".to_string()], Some("INBOX".to_string())), (vec!["INBOX".into(), "Café".into()], Some("INBOX/Caf&AOk-".into()))]
        );
        assert_eq!(sizes[0].size, 0, "{packed}: nothing kept in the Inbox");
        assert!(sizes[1].size >= 4000, "{packed}: {}", sizes[1].size);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
