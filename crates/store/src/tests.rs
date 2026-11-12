// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
use quatrelle_model::{Flag, Flags};

use super::*;

mod held;
mod journal;
mod listing;
mod noted;

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
        // Told which changed: kept beside the pages, which aren't even looked at.
        if let Some(m) = st.messages.iter_mut().find(|m| m.key.uid == 8000) {
            m.flags.flagged = true;
        }
        let changed: std::collections::HashSet<u32> = [8000].into_iter().collect();
        let version = s.folder_version("a1", "INBOX");
        assert_eq!(s.save_folder_counted("a1", "INBOX", &st, Some(&changed)).unwrap(), 1, "the marks alone");
        assert_eq!(s.load_folder("a1", "INBOX").unwrap(), st);
        assert_eq!(s.folder_version("a1", "INBOX"), version, "the head as it was");
        // New mail: the last page, and the page the mark was kept beside (the marks emptied).
        st.messages.insert(0, msg(n + 1));
        st.uid_next = n + 2;
        assert_eq!(s.save_folder_counted("a1", "INBOX", &st, None).unwrap(), 4);
        assert!(s.load_folder("a1", "INBOX").unwrap().find(8000).unwrap().flags.flagged);
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
    s.apply_op("a1", &Op::Expunge { keys: vec![k9.clone()], tidy: false }).unwrap();
    assert!(!s.has_body(&k9));
    assert_eq!(s.load_folder("a1", "INBOX").unwrap().messages.len(), 1);

    // A stale key (old UIDVALIDITY) changes nothing.
    let mut stale = st.messages[2].key.clone();
    stale.validity = 1;
    s.apply_op("a1", &Op::Expunge { keys: vec![stale], tidy: false }).unwrap();
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

/// A store that fails says so: a source that isn't kept is told from one that can't be read, and
/// the changes waiting for the server aren't written over when they can't be read.
#[test]
fn a_failing_store_says_so_and_writes_nothing_over() {
    let s = Store::failing();
    let st = FolderState { validity: 3, uid_next: 10, messages: vec![msg(9, false)], ..Default::default() };
    s.save_folder("a1", "INBOX", &st).unwrap();
    let k9 = st.messages[0].key.clone();
    s.write_body(&k9, b"x").unwrap();
    let missing = MessageKey { uid: 4, ..k9.clone() };
    assert_eq!(s.load_body(&missing).unwrap(), None, "not kept: no error");
    let waiting = vec![Op::Move { keys: vec![k9.clone()], to: "Archive".into() }];
    s.save_ops("a1", &waiting).unwrap();

    s.fail(Some(".eml"), None);
    assert!(s.load_body(&k9).is_err() && s.load_body(&missing).is_err());
    assert!(s.load_folder("a1", "INBOX").is_ok(), "only what ends so fails");
    // The change waiting can't be read: it isn't taken out (nor written over); one added goes
    // after it in a file of its own, nothing read or written again.
    s.fail(Some("00000000000000000001.json"), None);
    assert!(s.remove_op("a1", &waiting[0]).is_err());
    let added = Op::Expunge { keys: vec![k9.clone()], tidy: false };
    s.add_op("a1", added.clone()).unwrap();
    s.fail(None, Some("state.json"));
    assert!(s.save_folder("a1", "INBOX", &FolderState::<Summary>::default()).is_err());
    assert!(s.apply_op("a1", &Op::Expunge { keys: vec![k9.clone()], tidy: false }).is_err());

    s.fail(None, None);
    assert_eq!(s.load_ops("a1").unwrap(), [waiting[0].clone(), added]);
    assert_eq!(s.load_folder("a1", "INBOX").unwrap(), st);
    assert_eq!(s.load_body(&k9).unwrap().as_deref(), Some(&b"x"[..]));
}

/// Message sources read are counted, across the cache's clones (a search that needs none can be
/// shown to read none); lists, flags and listing the cached mail read none.
#[test]
fn message_sources_read_are_counted() {
    let s = Store::memory();
    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
    s.write_body(&key, b"Subject: x\r\n\r\nx").unwrap();
    s.save_folder("a1", "INBOX", &FolderState::<Summary> { validity: 1, ..Default::default() }).unwrap();
    let _ = s.load_folder("a1", "INBOX").unwrap();
    assert!(s.has_body(&key));
    assert_eq!(s.cached_bodies("a1", "INBOX", 1), [1u32]);
    assert_eq!(s.reads(), 0);
    let other = s.clone();
    assert!(other.read_body(&key).is_some());
    assert!(s.load_body(&MessageKey { uid: 2, ..key.clone() }).unwrap().is_none());
    assert_eq!((s.reads(), other.reads()), (2, 2));
    assert_eq!(Store::memory().reads(), 0, "a cache of its own counts its own");
}

/// What a write left in a memory cache: the names of the files written, sorted, and their bytes.
fn written(s: &Store) -> (Vec<String>, usize) {
    let changes = s.take_changes();
    let bytes = changes.iter().map(|(_, b)| b.as_ref().map_or(0, Vec::len)).sum();
    let mut names: Vec<String> = changes.into_iter().map(|(p, _)| p.rsplit('/').next().unwrap_or_default().to_string()).collect();
    names.sort_unstable();
    (names, bytes)
}

/// A big folder of `n` unread messages, kept in pages.
fn big_inbox(s: &Store, n: u32) -> FolderState {
    let mut st = FolderState { validity: 3, uid_next: n + 1, messages: (1..=n).map(|uid| msg(uid, false)).collect(), ..Default::default() };
    sort_newest_first(&mut st.messages);
    s.save_folder("a1", "INBOX", &st).unwrap();
    st
}

/// One message marked read in a folder of 10,941 (the pool's `Store::apply_op`, on the edit
/// writer's thread, journaled first as it is) writes under 64 KiB beside the folder's pages,
/// not the page of 4,096 it's in (about 2 MB on a real folder, every page turned into text
/// again to see which changed); read back, the folder has the mark, and so does the cache
/// opened again.
#[test]
fn one_mark_in_a_big_folder_writes_under_64_kib_and_survives_a_reopen() {
    let s = Store::memory();
    let mut st = big_inbox(&s, 10_941);
    s.track_changes();
    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 3, uid: 5000 };
    let op = Op::SetFlag { keys: vec![key.clone()], flag: Flag::Seen, on: true };
    s.add_op("a1", op.clone()).unwrap();
    s.take_changes();
    let version = s.folder_version("a1", "INBOX");
    assert_eq!(s.apply_op("a1", &op).unwrap(), vec!["INBOX".to_string()]);
    let (names, bytes) = written(&s);
    assert!(bytes < 64 << 10, "{bytes} bytes in {names:?}");
    assert_eq!(names, ["state-marks.json"], "no page, not the head");
    assert_eq!(s.folder_version("a1", "INBOX"), version, "the head alone is hashed: a mark leaves it");
    if let Some(m) = st.messages.iter_mut().find(|m| m.key.uid == 5000) {
        m.flags.seen = true;
    }
    assert_eq!(s.load_folder("a1", "INBOX").unwrap(), st, "read back with the mark");
    // On disk, opened again.
    let d = temp_store("one-mark-reopen");
    big_inbox(&d, 10_941);
    d.add_op("a1", op.clone()).unwrap();
    d.apply_op("a1", &op).unwrap();
    let root = d.root().to_path_buf();
    drop(d);
    let again = Store::open(&root).unwrap();
    assert!(again.load_folder("a1", "INBOX").unwrap().find(5000).unwrap().flags.seen, "opened again: read");
    let _ = fs::remove_dir_all(root);
}

/// A follow-up marked complete, with the time Exchange says it was done, keeps both through a
/// mark kept beside the pages and a reopen; the keyword taken off here goes with its time.
#[test]
fn a_completed_follow_up_and_its_time_survive_the_marks() {
    let d = temp_store("complete-marks");
    let mut st = big_inbox(&d, 10_941);
    if let Some(m) = st.messages.iter_mut().find(|m| m.key.uid == 5000) {
        m.flags.set_keyword(quatrelle_model::FOLLOW_UP_COMPLETE, true);
        m.flags.completed = 1_793_000_000;
    }
    d.save_folder("a1", "INBOX", &st).unwrap();
    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 3, uid: 5000 };
    d.apply_op("a1", &Op::SetFlag { keys: vec![key.clone()], flag: Flag::Seen, on: true }).unwrap();
    let root = d.root().to_path_buf();
    drop(d);
    let again = Store::open(&root).unwrap();
    let f = again.load_folder("a1", "INBOX").unwrap().find(5000).unwrap().flags.clone();
    assert!(f.seen && f.follow_up_complete() && f.completed == 1_793_000_000, "{f:?}");
    let off = Op::SetKeyword { keys: vec![key], keyword: quatrelle_model::FOLLOW_UP_COMPLETE.into(), on: false, name: None };
    again.apply_op("a1", &off).unwrap();
    let f = again.load_folder("a1", "INBOX").unwrap().find(5000).unwrap().flags.clone();
    assert!(!f.follow_up_complete() && f.completed == 0, "{f:?}");
    let _ = fs::remove_dir_all(root);
}

/// Marks kept beside the pages, past the most they hold, go into their pages and are emptied; a
/// damaged marks file is set aside and the list read as its pages have it; a list written afresh
/// (an import's) has none of the marks from before; a mark from the window's own save (a local
/// account's, `save_folder_changed`) is kept the same way.
#[test]
fn marks_beside_the_pages_fold_in_and_stand_damage() {
    let s = Store::memory();
    let n = PAGE_MESSAGES as u32 * 2 + 749;
    let mut st = big_inbox(&s, n);
    s.track_changes();
    let mark = |st: &mut FolderState, uids: &[u32]| {
        for m in st.messages.iter_mut().filter(|m| uids.contains(&m.key.uid)) {
            m.flags.seen = true;
        }
        let changed: std::collections::HashSet<u32> = uids.iter().copied().collect();
        s.save_folder_changed("a1", "INBOX", &*st, &changed).unwrap();
    };
    mark(&mut st, &[5000]);
    assert_eq!(written(&s).0, ["state-marks.json"]);
    assert_eq!(s.load_folder("a1", "INBOX").unwrap(), st);
    // More than the marks hold: into their pages, the marks emptied.
    let many: Vec<u32> = (1..=u32::try_from(folders::MARKS_MAX).unwrap()).map(|i| i * 17).collect();
    mark(&mut st, &many);
    assert_eq!(written(&s).0, ["state-0.json", "state-1.json", "state-2.json", "state-marks.json", "state.json"]);
    let marks = s.folder_dir("a1", "INBOX").join(folders::MARKS);
    assert!(!s.has_marks(&marks), "emptied");
    assert_eq!(s.load_folder("a1", "INBOX").unwrap(), st);
    // A damaged marks file: set aside, the list read as its pages have it.
    mark(&mut st, &[7]);
    s.files.write(&marks, b"[[7, {\"seen\": tr").unwrap();
    let back = s.load_folder("a1", "INBOX").unwrap();
    assert!(!back.find(7).unwrap().flags.seen && back.find(17).unwrap().flags.seen);
    // A list written afresh (an import's) has no marks from before.
    mark(&mut st, &[9]);
    let mut w = s.list_writer("a1", "INBOX");
    let mut fresh: Vec<Summary> = (1..=n).map(|uid| msg(uid, false)).collect();
    fresh.sort_by_key(|m| m.key.uid);
    w.push(&s, fresh).unwrap();
    w.finish(&s, &st).unwrap();
    assert!(!s.load_folder("a1", "INBOX").unwrap().find(9).unwrap().flags.seen);
}

/// Marks waiting to reach the server, offline: each kept writes only the marks, and doesn't
/// make the ones waiting before it again on the whole list (each save cloned the list and went
/// over it once per edit waiting: a burst cost the square of its length).
#[test]
fn marks_made_offline_dont_make_the_waiting_ones_again() {
    let s = Store::memory();
    big_inbox(&s, 10_941);
    s.track_changes();
    for uid in 1..=200u32 {
        let op = Op::SetFlag {
            keys: vec![MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 3, uid: uid * 50 }],
            flag: Flag::Seen,
            on: true,
        };
        s.add_op("a1", op.clone()).unwrap();
        s.apply_op("a1", &op).unwrap();
    }
    let (names, bytes) = written(&s);
    assert!(names.iter().all(|n| n == "state-marks.json" || n.starts_with("000")), "{names:?}");
    assert!(bytes < 4 << 20, "{bytes}");
    let back = s.load_folder("a1", "INBOX").unwrap();
    assert!((1..=200u32).all(|uid| back.find(uid * 50).is_some_and(|m| m.flags.seen)));
    assert_eq!(back.messages.iter().filter(|m| m.flags.seen).count(), 200);
}

/// A head written before the pages' first numbers were kept: a mark reads the whole list, and is
/// kept beside its pages all the same; the next whole save numbers them.
#[test]
fn a_mark_in_a_list_kept_before_reads_it_whole() {
    let s = Store::memory();
    big_inbox(&s, 10_000);
    let head = s.folder_dir("a1", "INBOX").join("state.json");
    let mut file: serde_json::Value = serde_json::from_slice(&s.files.read(&head).unwrap()).unwrap();
    assert!(file.as_object_mut().unwrap().remove("firsts").is_some());
    s.files.write(&head, &serde_json::to_vec(&file).unwrap()).unwrap();
    s.track_changes();
    let op =
        Op::SetFlag { keys: vec![MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 3, uid: 9_000 }], flag: Flag::Seen, on: true };
    s.apply_op("a1", &op).unwrap();
    assert_eq!(written(&s).0, ["state-marks.json"]);
    assert!(s.load_folder("a1", "INBOX").unwrap().find(9_000).unwrap().flags.seen);
    s.save_folder("a1", "INBOX", &s.load_folder("a1", "INBOX").unwrap()).unwrap();
    let file: serde_json::Value = serde_json::from_slice(&s.files.read(&head).unwrap()).unwrap();
    assert_eq!(file["firsts"], serde_json::json!([1, 4097, 8193]));
}

/// A memory cache lists a folder from where its files start: only the files directly in it, not
/// a sibling whose name starts the same, nor what's in a folder inside; and a folder with files
/// under it exists.
#[test]
fn a_memory_folder_lists_only_its_own_files() {
    let s = Store::memory();
    let key = |folder: &str, validity: u32, uid: u32| MessageKey { account: "a1".into(), folder: folder.into(), validity, uid };
    let bodies: Vec<(MessageKey, Vec<u8>)> = [key("INBOX", 7, 1), key("INBOX", 7, 2), key("INBOX", 70, 3), key("INBOX2", 7, 4), key("INB", 7, 5)]
        .into_iter()
        .map(|k| (k, b"Subject: x\r\n\r\nx".to_vec()))
        .collect();
    s.write_bodies(&bodies).unwrap();
    let mut cached = s.cached_bodies("a1", "INBOX", 7);
    cached.sort_unstable();
    assert_eq!(cached, [1, 2]);
    assert_eq!(s.cached_bodies("a1", "INBOX", 70), [3]);
    assert_eq!(s.cached_bodies("a1", "INBOX2", 7), [4]);
    assert!(s.cached_bodies("a1", "INBOX", 9).is_empty() && s.cached_bodies("a1", "Other", 7).is_empty());
    let dir = s.folder_dir("a1", "INBOX");
    assert!(s.files.exists(&dir) && s.files.exists(&dir.join("70")));
    assert!(!s.files.exists(&dir.join("8")) && !s.files.exists(&s.folder_dir("a1", "Other")));
    assert_eq!(s.files.main.walk(&dir).len(), 3);
}

/// A message's page of a big list read alone, with the marks kept beside the pages; a small list
/// is its own page.
#[test]
fn the_page_a_message_is_in_is_read_alone() {
    let s = Store::memory();
    let messages: Vec<Summary> = (1..=10_000).rev().map(|u| msg(u, false)).collect();
    s.save_folder("a1", "INBOX", &FolderState { validity: 3, messages, ..Default::default() }).unwrap();
    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 3, uid: 5_000 };
    s.apply_op("a1", &Op::SetFlag { keys: vec![key.clone()], flag: quatrelle_model::Flag::Seen, on: true }).unwrap();
    let page = s.load_page_of("a1", "INBOX", 5_000).unwrap();
    assert_eq!(page.len(), PAGE_MESSAGES);
    assert!(page.iter().all(|m| (4_097..=8_192).contains(&m.key.uid)), "the second page");
    assert!(page.iter().any(|m| m.key == key && m.flags.seen), "marked as the list is");
    assert_eq!(s.load_page_of("a1", "INBOX", 0).unwrap().first().map(|m| m.key.uid), Some(1), "below them all: the first");
    assert_eq!(s.load_page_of("a1", "INBOX", 9_999).unwrap().len(), 10_000 - 2 * PAGE_MESSAGES);
    s.save_folder("a1", "Small", &FolderState { validity: 3, messages: (1..=5).map(|u| msg(u, false)).collect(), ..Default::default() }).unwrap();
    let small = s.load_page_of("a1", "Small", 3).unwrap();
    assert_eq!(small.len(), 5);
    assert!(small.iter().all(|m| m.key.folder == "Small"), "keys rebuilt");
    assert!(s.load_page_of("a1", "Nowhere", 1).unwrap_or_default().is_empty());
}

/// A data file set aside is closed, renamed beside the others and never opened again as the
/// account's: the account starts a file of its own, and a name taken already isn't written over.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn set_aside_closes_and_renames_the_data_file() {
    let t = std::env::temp_dir().join(format!("quatrelle-store-set-aside-{}", std::process::id()));
    let _ = fs::remove_dir_all(&t);
    let s = Store::open_packed(t.join("app"), t.join("files")).unwrap();
    assert_eq!(s.set_aside_data_file("a1", "a1.old").unwrap(), None, "no file yet");
    s.save_account_doc("a1", "doc.json", &vec![1]).unwrap();
    let file = s.data_file("a1").unwrap();
    let aside = s.set_aside_data_file("a1", "a1-x.quatrelle.old").unwrap().unwrap();
    assert_eq!(aside, t.join("files").join("a1-x.quatrelle.old"));
    assert!(aside.exists() && !file.exists());
    assert!(s.load_account_doc::<Vec<i32>>("a1", "doc.json").unwrap().is_empty(), "the old file isn't read");
    s.save_account_doc("a1", "doc.json", &vec![2]).unwrap();
    assert_eq!(s.load_account_doc::<Vec<i32>>("a1", "doc.json").unwrap(), [2]);
    let again = s.set_aside_data_file("a1", "a1-x.quatrelle.old").unwrap().unwrap();
    assert_eq!(again, t.join("files").join("a1-x (2).quatrelle.old"));
    // A new store over the same folders doesn't know the files set aside.
    drop(s);
    let s = Store::open_packed(t.join("app"), t.join("files")).unwrap();
    assert!(s.load_account_doc::<Vec<i32>>("a1", "doc.json").unwrap().is_empty());
    // Nor a name Windows keeps for a device, whatever follows its first dot, nor one ending in a
    // dot or a space (Windows drops them).
    for bad in ["", "..", "../x.old", "a/b.old", "c:x", "a\u{0}b", "CON", "nul.quatrelle.old", "Com1.x", "LPT9", "lpt¹.old", "a.old.", "a.old "] {
        assert!(s.set_aside_data_file("a1", bad).is_err(), "{bad:?}");
    }
    assert_eq!(Store::memory().set_aside_data_file("a1", "a1.old").unwrap(), None, "a cache without data files");
    let _ = fs::remove_dir_all(&t);
}

/// Files kept with an account by their place in it: written together, read, listed by folder and
/// removed with what's under them, the same in memory and in a data file; a place that climbs out
/// is refused.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn an_accounts_files_by_their_place() {
    let t = std::env::temp_dir().join(format!("quatrelle-store-account-files-{}", std::process::id()));
    let _ = fs::remove_dir_all(&t);
    for s in [Store::memory(), Store::open_packed(t.join("app"), t.join("files")).unwrap()] {
        let files = [("top.json", "1"), ("chats/a.json", "2"), ("chats/b.json", "3"), ("msgs/a/0.json", "4"), ("msgs/a/1.json", "5")];
        s.write_account_files("T", &files.map(|(k, v)| (k.to_string(), v.as_bytes().to_vec()))).unwrap();
        assert_eq!(s.account_file("T", "chats/b.json").unwrap().as_deref(), Some(&b"3"[..]));
        assert_eq!(s.account_file("T", "chats/c.json").unwrap(), None);
        let mut chats = s.account_files("T", "chats").unwrap();
        chats.sort();
        assert_eq!(chats, ["a.json", "b.json"]);
        assert_eq!(s.account_files("T", "").unwrap(), ["top.json"], "files only, not the folders");
        s.remove_account_files("T", "msgs/a").unwrap();
        s.remove_account_files("T", "chats/a.json").unwrap();
        s.remove_account_files("T", "nothing/here").unwrap();
        assert!(s.account_files("T", "msgs/a").unwrap().is_empty() && s.account_files("T", "chats").unwrap() == ["b.json"]);
        for bad in ["", "../x", "a//b", "/abs", "a/./b", "a b"] {
            assert!(s.account_file("T", bad).is_err() && s.write_account_files("T", &[(bad.to_string(), vec![])]).is_err(), "{bad:?}");
        }
    }
    let _ = fs::remove_dir_all(&t);
}

/// A listing that fails says so: an account's folder that can't be listed isn't taken for empty.
#[test]
fn a_listing_that_fails_is_an_error() {
    let s = Store::failing();
    s.write_account_files("T", &[("chats/a.json".into(), b"1".to_vec())]).unwrap();
    s.fail(Some("chats"), None);
    assert!(s.account_files("T", "chats").is_err());
    s.fail(None, None);
    assert_eq!(s.account_files("T", "chats").unwrap(), ["a.json"]);
    // Names that are fine still go aside under them.
    for fine in ["CONSOLE.old", "COM10.old", "Teams-1a2b3c4d.quatrelle.old"] {
        assert!(Store::memory().set_aside_data_file("a1", fine).is_ok(), "{fine}");
    }
}
