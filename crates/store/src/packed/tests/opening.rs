// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Opening data files: from an existing one, damaged, online only, by threads at once, after a crash,
//! with the list of which is whose damaged or unreadable.

use super::*;

/// A new account starts from an existing data file: its folders, mail and documents come in,
/// the messages as this account's; junk is refused and paths that climb out are dropped.
#[test]
fn an_account_starts_from_an_existing_data_file() {
    use quatrelle_model::{Folder, FolderRole};
    let t = temp("from-file");
    let old = Store::open_packed(t.join("old"), t.join("Old Files")).unwrap();
    old.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    old.save_folders("a1", &[Folder { path: "INBOX".into(), role: FolderRole::Inbox, ..Default::default() }]).unwrap();
    let k = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 7 };
    old.write_body(&k, b"Subject: hi\r\n\r\nhello").unwrap();
    let mut st = old.load_folder("a1", "INBOX").unwrap();
    st.validity = 1;
    st.messages.push(Summary { key: k.clone(), subject: "hi".into(), ..Default::default() });
    old.save_folder("a1", "INBOX", &st).unwrap();
    old.save_account_doc("a1", "pim.json", &serde_json::json!({"notes": []})).unwrap();
    // Changes still to make on its server, and mail still to send: they stay behind.
    old.save_ops("a1", &[quatrelle_model::Op::Expunge { keys: vec![k.clone()], tidy: false }]).unwrap();
    old.outbox_add("a1", &crate::OutboxItem { id: 4, ..Default::default() }, b"Subject: later\r\n\r\nx").unwrap();
    let file = old.data_file("a1").unwrap();
    drop(old);

    let s = Store::open_packed(t.join("app"), t.join("Quatrelle Files")).unwrap();
    s.save_accounts(&[account("p7", "ana@example.org")]).unwrap();
    assert_eq!(s.import_data_file("p7", &file).unwrap(), 1);
    let folders = s.load_folders("p7").unwrap();
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0].account, "p7", "its folders are this account's (opening one named the old account)");
    assert!(s.load_ops("p7").unwrap().is_empty() && s.outbox("p7").is_empty(), "the old account's pending work stays behind");
    let st = s.load_folder("p7", "INBOX").unwrap();
    let mine = MessageKey { account: "p7".into(), ..k.clone() };
    assert_eq!(st.messages[0].key, mine, "renumbered as this account's");
    assert_eq!(s.read_body(&mine).unwrap(), b"Subject: hi\r\n\r\nhello");
    assert_eq!(s.load_account_doc::<serde_json::Value>("p7", "pim.json").unwrap(), serde_json::json!({"notes": []}));
    assert!(file.exists(), "the file it came from stays");
    assert!(!t.join("app").join(".import-p7.tmp").exists(), "the copy read from is gone");

    // Junk: refused, and nothing written.
    let junk = t.join("junk.quatrelle");
    std::fs::write(&junk, b"not a database at all").unwrap();
    assert!(s.import_data_file("p8", &junk).is_err());
    assert!(s.load_folders("p8").unwrap().is_empty());

    // A crafted file: paths that climb out of the account are left out.
    let crafted = t.join("crafted.quatrelle");
    {
        let db = redb::Database::create(&crafted).unwrap();
        let tx = db.begin_write().unwrap();
        {
            let mut table = tx.open_table(super::super::FILES).unwrap();
            for key in ["../escape.json", "a/../../b", "C:/x", "/root", "ok/fine.json"] {
                table.insert(key, b"x".as_slice()).unwrap();
            }
        }
        tx.commit().unwrap();
    }
    s.import_data_file("p9", &crafted).unwrap();
    assert!(s.files.exists(&t.join("app").join("p9").join("ok").join("fine.json")));
    assert!(!t.join("app").join("escape.json").exists() && !t.join("escape.json").exists() && !t.join("b").exists());
    let _ = std::fs::remove_dir_all(&t);
}

#[test]
fn a_damaged_data_file_is_set_aside() {
    let t = temp("damaged");
    let files = t.join("Files");
    std::fs::create_dir_all(&files).unwrap();
    std::fs::write(files.join("zoe@studio.example.quatrelle"), b"not a database at all, just junk bytes").unwrap();
    std::fs::create_dir_all(t.join("app")).unwrap();
    std::fs::write(t.join("app").join("datafiles.json"), br#"{"a1": "zoe@studio.example.quatrelle"}"#).unwrap();
    let s = Store::open_packed(t.join("app"), &files).unwrap();
    let k = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
    assert!(s.read_body(&k).is_none());
    s.write_body(&k, b"x").unwrap();
    assert!(files.join("zoe@studio.example.quatrelle.damaged").exists());
    let _ = std::fs::remove_dir_all(t);
}

/// Whether any file under `dir` holds `needle`.
fn on_disk(dir: &std::path::Path, needle: &[u8]) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else { return false };
    rd.flatten().any(|e| {
        let p = e.path();
        if p.is_dir() { on_disk(&p, needle) } else { std::fs::read(&p).is_ok_and(|b| b.windows(needle.len()).any(|w| w == needle)) }
    })
}

/// Issue #4: an account kept online only writes none of its mail anywhere. What it had on disk
/// comes into memory and its data file goes; it works as before while Quatrelle runs (lists,
/// sources, changes waiting, the Outbox), and the next start finds nothing of it. Other
/// accounts keep theirs; turned off again, what's held is written out.
#[test]
fn an_online_only_account_leaves_nothing_on_disk() {
    use quatrelle_model::{Folder, FolderRole};
    let t = temp("online-only");
    let (app, files) = (t.join("app"), t.join("Quatrelle Files"));
    let s = Store::open_packed(&app, &files).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example"), account("a2", "ana@studio.example")]).unwrap();
    let k = |a: &str, uid| MessageKey { account: a.into(), folder: "INBOX".into(), validity: 1, uid };
    let fill = |s: &Store, a: &str, uid, text: &str| {
        s.save_folders(a, &[Folder { path: "INBOX".into(), role: FolderRole::Inbox, ..Default::default() }]).unwrap();
        let mut st = s.load_folder(a, "INBOX").unwrap();
        st.validity = 1;
        st.messages.push(Summary { key: k(a, uid), subject: format!("{text} subject"), ..Default::default() });
        s.save_folder(a, "INBOX", &st).unwrap();
        s.write_body(&k(a, uid), format!("Subject: {text}\r\n\r\n{text} body").as_bytes()).unwrap();
    };
    fill(&s, "a1", 1, "EARLIER-ZOE");
    fill(&s, "a2", 1, "ANA-KEPT");
    let zoe_file = s.data_file("a1").unwrap();
    // (A data file can be read only once it's closed.)
    drop(s);
    assert!(zoe_file.exists() && on_disk(&t, b"EARLIER-ZOE"), "the search finds what's kept");
    let s = Store::open_packed(&app, &files).unwrap();

    s.set_online_only("a1", true).unwrap();
    assert!(s.is_online_only("a1") && !s.is_online_only("a2"));
    assert!(!zoe_file.exists(), "its data file goes");
    assert_eq!(s.data_file("a1"), None);
    // What it had is still there while Quatrelle runs.
    assert_eq!(s.load_folder("a1", "INBOX").unwrap().messages.len(), 1);
    assert_eq!(s.read_body(&k("a1", 1)).unwrap(), b"Subject: EARLIER-ZOE\r\n\r\nEARLIER-ZOE body");
    // New mail, changes, the Outbox and documents: in memory only.
    fill(&s, "a1", 2, "SECRET-ZOE");
    s.write_bodies(&[(k("a1", 3), b"Subject: x\r\n\r\nSECRET-BATCH".to_vec()), (k("a2", 3), b"Subject: x\r\n\r\nANA-BATCH".to_vec())]).unwrap();
    s.save_ops("a1", &[quatrelle_model::Op::Expunge { keys: vec![k("a1", 1)], tidy: false }]).unwrap();
    s.outbox_add("a1", &crate::OutboxItem { id: 4, ..Default::default() }, b"Subject: later\r\n\r\nSECRET-OUTBOX").unwrap();
    s.save_account_doc("a1", "pim.json", &serde_json::json!({"notes": ["SECRET-NOTE"]})).unwrap();
    assert_eq!(s.load_folder("a1", "INBOX").unwrap().messages.len(), 2);
    assert!(s.has_body(&k("a1", 2)) && s.read_body(&k("a1", 3)).is_some());
    assert_eq!(s.load_ops("a1").unwrap().len(), 1);
    assert_eq!(s.outbox("a1").len(), 1);
    assert!(s.account_size("a1") > 0);

    // Quatrelle closes: nothing of it on disk, and the next start finds nothing of it.
    drop(s);
    for secret in [&b"EARLIER-ZOE"[..], b"SECRET-ZOE", b"SECRET-BATCH", b"SECRET-OUTBOX", b"SECRET-NOTE"] {
        assert!(!on_disk(&t, secret), "{}", String::from_utf8_lossy(secret));
    }
    assert!(on_disk(&t, b"ANA-BATCH") && on_disk(&t, b"ANA-KEPT"), "the other account keeps its mail on disk");
    assert!(!std::fs::read_to_string(app.join(super::super::NAMES)).unwrap().contains("a1"), "no data file is named for it");
    let s = Store::open_packed(&app, &files).unwrap();
    s.set_online_only("a1", true).unwrap();
    assert!(s.load_folder("a1", "INBOX").unwrap().messages.is_empty() && s.load_ops("a1").unwrap().is_empty());
    assert_eq!(s.read_body(&k("a2", 1)).unwrap(), b"Subject: ANA-KEPT\r\n\r\nANA-KEPT body");

    // Turned off: what's held is written out to a data file again.
    fill(&s, "a1", 5, "BACK-ON-DISK");
    s.set_online_only("a1", false).unwrap();
    assert!(!s.is_online_only("a1"));
    assert!(s.data_file("a1").is_some_and(|f| f.exists()));
    assert_eq!(s.read_body(&k("a1", 5)).unwrap(), b"Subject: BACK-ON-DISK\r\n\r\nBACK-ON-DISK body");
    drop(s);
    let s = Store::open_packed(&app, &files).unwrap();
    assert_eq!(s.load_folder("a1", "INBOX").unwrap().messages.len(), 1, "kept across a start");

    // Removing an online-only account: nothing to keep, nothing left.
    s.set_online_only("a1", true).unwrap();
    assert_eq!(s.remove_account("a1").unwrap(), None);
    assert!(!s.is_online_only("a1") && s.load_folder("a1", "INBOX").unwrap().messages.is_empty());
    drop(s);
    assert!(!on_disk(&t, b"BACK-ON-DISK"));
    let _ = std::fs::remove_dir_all(t);
}

/// Turning online-only on never removes a data file it couldn't open: with the file held by
/// another copy of Quatrelle, the walk of what's in it said "nothing", nothing was copied, and the
/// file went with the changes waiting for the server and the Outbox in it. It fails, and the file
/// stays as it was.
#[test]
fn online_only_never_deletes_a_data_file_it_couldnt_open() {
    let t = temp("online-only-held");
    let (app, files) = (t.join("app"), t.join("Quatrelle Files"));
    let k = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
    let s = Store::open_packed(&app, &files).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    s.write_body(&k, b"Subject: hi\r\n\r\nWAITING-BODY").unwrap();
    s.add_op("a1", quatrelle_model::Op::Expunge { keys: vec![k.clone()], tidy: false }).unwrap();
    let file = s.data_file("a1").unwrap();
    drop(s);

    // Another holder has the data file open.
    let holder = redb::Database::create(&file).unwrap();
    let s = Store::open_packed(&app, &files).unwrap();
    assert!(s.set_online_only("a1", true).is_err(), "a file it can't open stops the switch");
    assert!(!s.is_online_only("a1"));
    assert!(file.exists(), "the file is still there");
    drop(s);
    drop(holder);

    let s = Store::open_packed(&app, &files).unwrap();
    assert!(file.exists() && s.data_file("a1").is_some());
    assert_eq!(s.load_ops("a1").unwrap().len(), 1, "the change waiting is still in it");
    assert_eq!(s.read_body(&k).unwrap(), b"Subject: hi\r\n\r\nWAITING-BODY");
    // Closed by the other holder, it switches, and all of it comes over.
    s.set_online_only("a1", true).unwrap();
    assert!(s.is_online_only("a1") && !file.exists());
    assert_eq!(s.load_ops("a1").unwrap().len(), 1);
    assert_eq!(s.read_body(&k).unwrap(), b"Subject: hi\r\n\r\nWAITING-BODY");
    let _ = std::fs::remove_dir_all(t);
}

/// An account's data file opened by several threads at once (the account's own and its sync
/// connections, at start) opens once: two that both found it not open yet both opened it, and
/// the second heard "open in another copy of Quatrelle" — a sync that failed for nothing.
#[test]
fn a_data_file_opened_by_threads_at_once_opens_once() {
    for round in 0..20 {
        let t = temp(&format!("open-race-{round}"));
        {
            let store = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
            store.save_accounts(&[account("a1", "a1@example.org")]).unwrap();
            store.save_folder("a1", "INBOX", &crate::FolderState::<quatrelle_model::Summary>::default()).unwrap();
        }
        // A fresh handle on the same folders (the first closed): nothing is open in it yet.
        let store = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let (store, barrier) = (store.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    store.load_folder("a1", "INBOX").map(|_| ())
                })
            })
            .collect();
        for th in threads {
            let r = th.join().unwrap();
            assert!(r.is_ok(), "round {round}: {r:?}");
        }
        drop(store);
        let _ = std::fs::remove_dir_all(&t);
    }
}

/// A data file whose writer stopped dead (a crash, a power cut) opens without being gone through
/// first: each commit keeps what a repair would work out. Without it, a 45 GB file took
/// 72 s to open after Quatrelle was killed. The writer is this test run again, which writes and
/// leaves without closing anything.
#[test]
fn a_data_file_left_by_a_crash_needs_no_repair() {
    const WRITER: &str = "QUATRELLE_TEST_CRASHED_WRITER";
    if let Some(dir) = std::env::var_os(WRITER) {
        let dir = std::path::PathBuf::from(dir);
        let store = Store::open_packed(dir.join("app"), dir.join("Files")).unwrap();
        store.save_accounts(&[account("a1", "a1@example.org")]).unwrap();
        let st = crate::FolderState { validity: 7, ..crate::FolderState::<Summary>::default() };
        store.save_folder("a1", "INBOX", &st).unwrap();
        // No destructor runs: the data file is left as a crash leaves it.
        std::process::exit(3);
    }
    let t = temp("crashed");
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "packed::tests::opening::a_data_file_left_by_a_crash_needs_no_repair", "--test-threads=1"])
        .env(WRITER, &t)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(3), "the writer stopped dead after writing");
    let file = std::fs::read_dir(t.join("Files"))
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| p.extension().is_some_and(|x| x == super::super::EXTENSION))
        .unwrap();
    let repaired = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = repaired.clone();
    let db = redb::Database::builder().set_repair_callback(move |_| seen.set(true)).create(&file).unwrap();
    assert!(!repaired.get(), "no repair was needed");
    drop(db);
    let store = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    assert_eq!(store.load_folder("a1", "INBOX").unwrap().validity, 7, "and what was written is there");
    drop(store);
    let _ = std::fs::remove_dir_all(&t);
}

/// An account removed while another thread still writes for it (an import not stopped yet): a
/// write that begins just as the account lets go of its file never puts the removed account's
/// file back among the open ones under its id (adapted from the origin's 0a7dbcc3). It waits,
/// then finds no name and makes a file of its own; so a new account given the id gets that file,
/// and the removed one's is left as it was. (Closed, then forgotten in a second step, the file
/// came back with no name for every later write of the id.)
#[test]
fn a_write_as_an_account_lets_go_never_lends_it_the_old_file() {
    use std::sync::Mutex;
    use std::sync::mpsc;
    use std::time::Duration;
    let t = temp("let-go-write");
    let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    let key = |uid| MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
    s.write_body(&key(1), b"Subject: before\r\n\r\nx").unwrap();
    let old = s.data_file("a1").expect("zoe's file");
    // As it lets go, another thread starts a write for the account and has the time to open.
    let (tx, rx) = mpsc::channel::<()>();
    let tx = Mutex::new(tx);
    let crate::Files::Packed(packs) = &s.files.main else { panic!("packed") };
    *Mutex::lock(&packs.let_go_hook).unwrap() = Some(Box::new(move || {
        let _ = tx.lock().unwrap().send(());
        std::thread::sleep(Duration::from_millis(200));
    }));
    std::thread::scope(|scope| {
        let store = &s;
        scope.spawn(move || {
            rx.recv().unwrap();
            let _ = store.write_body(&key(2), b"Subject: late\r\n\r\ny");
        });
        let kept = s.remove_account("a1").unwrap();
        assert_eq!(kept.as_ref(), Some(&old), "the removed account's file is kept");
    });
    *Mutex::lock(&packs.let_go_hook).unwrap() = None;
    // The id given to a new account: its folders go in a file of its own.
    let folder = quatrelle_model::Folder { account: "a1".into(), path: "INBOX".into(), ..Default::default() };
    s.save_folders("a1", std::slice::from_ref(&folder)).unwrap();
    let new = s.data_file("a1").expect("the new account's file");
    assert_ne!(new, old, "the new account writes in the removed one's file");
    drop(s);
    let _ = std::fs::remove_dir_all(t);
}

/// The data files hold the mail, and the computer may be shared: their folder and each of them
/// are the owner's alone, a file from an earlier version and one set aside as damaged included;
/// a folder of the user's own that the files were put in keeps the access it had.
#[cfg(unix)]
#[test]
fn data_files_are_the_owners_alone() {
    use std::os::unix::fs::PermissionsExt;
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    let open = |p: &std::path::Path, m: u32| std::fs::set_permissions(p, std::fs::Permissions::from_mode(m)).unwrap();
    let t = temp("owner-only");
    let files = t.join("Quatrelle Files");
    let s = Store::open_packed(t.join("app"), &files).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    s.save_account_doc("a1", "pim.json", &serde_json::json!({"notes": []})).unwrap();
    let file = s.data_file("a1").unwrap();
    assert_eq!(mode(&files), 0o700, "the folder");
    assert_eq!(mode(&file), 0o600, "a data file made new");
    drop(s);

    // As an earlier version left them: open to the computer's other accounts.
    open(&files, 0o755);
    open(&file, 0o644);
    let s = Store::open_packed(t.join("app"), &files).unwrap();
    assert_eq!(s.load_account_doc::<serde_json::Value>("a1", "pim.json").unwrap(), serde_json::json!({"notes": []}));
    assert_eq!((mode(&files), mode(&file)), (0o700, 0o600), "closed when next opened");
    drop(s);

    // Damaged: set aside, and the one made in its place is the owner's alone too.
    std::fs::write(&file, vec![0xA5u8; 4096]).unwrap();
    open(&file, 0o644);
    let s = Store::open_packed(t.join("app"), &files).unwrap();
    s.save_account_doc("a1", "pim.json", &serde_json::json!({"notes": [1]})).unwrap();
    let aside =
        std::fs::read_dir(&files).unwrap().flatten().map(|e| e.path()).find(|p| p.to_string_lossy().ends_with(".damaged")).expect("set aside");
    assert_eq!((mode(&file), mode(&aside)), (0o600, 0o600));
    drop(s);

    // A folder of the user's own, there before: its access is theirs to choose; the files in it
    // are still the owner's alone.
    let own = t.join("Shared drive");
    std::fs::create_dir_all(&own).unwrap();
    open(&own, 0o755);
    let s = Store::open_packed(t.join("app2"), &own).unwrap();
    s.save_accounts(&[account("b1", "ana@example.org")]).unwrap();
    s.save_account_doc("b1", "pim.json", &serde_json::json!({})).unwrap();
    assert_eq!((mode(&own), mode(&s.data_file("b1").unwrap())), (0o755, 0o600));
    drop(s);
    let _ = std::fs::remove_dir_all(t);
}

/// Every path under `dir` and, for a file, its bytes.
fn snapshot(dir: &std::path::Path) -> std::collections::BTreeMap<std::path::PathBuf, Option<Vec<u8>>> {
    let mut out = std::collections::BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p.clone());
                out.insert(p, None);
            } else {
                out.insert(p.clone(), std::fs::read(&p).ok());
            }
        }
    }
    out
}

/// A damaged list of which data file is each account's is set aside, and each account's file is
/// found again by the name it was given (adapted from upstream f6f9f10). It was taken for an
/// empty list: the account showed no mail, and the next write gave it a second file beside the
/// first, which was forgotten.
#[test]
fn a_damaged_list_of_data_files_finds_them_again() {
    let k = |uid| MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
    let t = temp("names-damaged");
    let (app, files) = (t.join("app"), t.join("Files"));
    let s = Store::open_packed(&app, &files).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    s.write_body(&k(1), b"Subject: kept\r\n\r\nkept").unwrap();
    let file = s.data_file("a1").unwrap();
    drop(s);
    std::fs::write(app.join(super::super::NAMES), b"{\"a1\": ").unwrap();
    let s = Store::open_packed(&app, &files).unwrap();
    assert_eq!(s.data_file("a1"), Some(file.clone()), "found again");
    assert_eq!(s.read_body(&k(1)).unwrap(), b"Subject: kept\r\n\r\nkept");
    s.write_body(&k(2), b"Subject: more\r\n\r\nmore").unwrap();
    let data_files = std::fs::read_dir(&files).unwrap().flatten().filter(|e| e.path().extension().is_some_and(|x| x == super::super::EXTENSION));
    assert_eq!(data_files.count(), 1, "no second file");
    assert_eq!(std::fs::read(app.join("datafiles.json.bad")).ok().as_deref(), Some(&b"{\"a1\": "[..]), "the damaged list is set aside");
    drop(s);
    let _ = std::fs::remove_dir_all(t);
}

/// Two accounts with one address, their data files `<address>` and `<address> (2)`, with the
/// accounts listed the other way round by then; and `ana`, an address of her own.
fn two_of_one_address(name: &str) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf, Vec<std::path::PathBuf>) {
    let k = |a: &str| MessageKey { account: a.into(), folder: "INBOX".into(), validity: 1, uid: 1 };
    let t = temp(name);
    let (app, files) = (t.join("app"), t.join("Files"));
    let s = Store::open_packed(&app, &files).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example"), account("a2", "zoe@studio.example"), account("a3", "ana@studio.example")]).unwrap();
    for a in ["a1", "a2", "a3"] {
        s.write_body(&k(a), format!("Subject: of {a}\r\n\r\nx").as_bytes()).unwrap();
    }
    let made: Vec<_> = ["a1", "a2", "a3"].iter().map(|a| s.data_file(a).unwrap()).collect();
    assert!(made[1].to_string_lossy().ends_with("zoe@studio.example (2).quatrelle"));
    s.save_accounts(&[account("a3", "ana@studio.example"), account("a2", "zoe@studio.example"), account("a1", "zoe@studio.example")]).unwrap();
    (t, app, files, made)
}

/// With the list of data files damaged, files are found again by their names alone (a data file
/// doesn't say whose it is): an account whose address is its own is found by its file's name; of
/// two with one address neither is taken for the other (each read the other's mail, by the order
/// the accounts are listed in). Each starts a new file, and the old ones stay (adapted from
/// upstream f6f9f10).
#[test]
fn a_damaged_list_never_guesses_between_accounts_of_one_address() {
    let k = |a: &str| MessageKey { account: a.into(), folder: "INBOX".into(), validity: 1, uid: 1 };
    let (t, app, files, made) = two_of_one_address("names-not-guessed");
    std::fs::write(app.join(super::super::NAMES), b"{\"a1\": ").unwrap();
    let s = Store::open_packed(&app, &files).unwrap();
    assert_eq!(s.data_file("a3").as_ref(), made.get(2), "found by its name");
    assert_eq!(s.read_body(&k("a3")).unwrap(), b"Subject: of a3\r\n\r\nx");
    assert_eq!((s.data_file("a1"), s.data_file("a2")), (None, None), "not guessed");
    s.write_body(&k("a2"), b"Subject: new\r\n\r\ny").unwrap();
    let new = s.data_file("a2").unwrap();
    assert!(!made.contains(&new), "{}", new.display());
    assert!(made.iter().all(|f| f.is_file()), "the old files stay");
    drop(s);
    let _ = std::fs::remove_dir_all(t);
}

/// An account removed keeps its data file, no one's, and the same account added again (its id,
/// its address) writes in a file of its own beside it. With the list damaged then, the old file
/// is never given to the account added again: by its name alone it was that address's file,
/// and the account came back reading the removed one's mail.
#[test]
fn a_damaged_list_never_gives_a_removed_accounts_file_to_the_account_added_again() {
    let k = |uid| MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
    let t = temp("names-removed");
    let (app, files) = (t.join("app"), t.join("Files"));
    let s = Store::open_packed(&app, &files).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    s.write_body(&k(1), b"Subject: removed\r\n\r\nold").unwrap();
    let old = s.remove_account("a1").unwrap().unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    s.write_body(&k(2), b"Subject: new\r\n\r\nnew").unwrap();
    assert_ne!(s.data_file("a1").as_ref(), Some(&old));
    drop(s);
    let before = std::fs::read(&old).unwrap();
    std::fs::write(app.join(super::super::NAMES), b"{\"a1\": ").unwrap();
    let s = Store::open_packed(&app, &files).unwrap();
    assert_ne!(s.data_file("a1").as_ref(), Some(&old), "the removed account's file is given to the account added again");
    assert!(s.read_body(&k(1)).is_none(), "the removed account's mail reads as the new one's");
    drop(s);
    assert_eq!(std::fs::read(&old).unwrap(), before, "the old file stays as it was");
    let _ = std::fs::remove_dir_all(t);
}

/// An account added beside a data file of its name (an account removed, its file kept) gets its
/// own file at once, numbered beside the other: made only at its first write, until then the old
/// file was the only one of that name, and a list damaged meanwhile gave it to the account added
/// again.
#[test]
fn an_account_added_beside_an_old_file_is_numbered_at_once() {
    let k = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
    let t = temp("names-numbered");
    let (app, files) = (t.join("app"), t.join("Files"));
    let s = Store::open_packed(&app, &files).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    s.write_body(&k, b"Subject: removed\r\n\r\nold").unwrap();
    let old = s.remove_account("a1").unwrap().unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    let mine = s.data_file("a1");
    assert!(mine.as_ref().is_some_and(|f| f.to_string_lossy().ends_with("zoe@studio.example (2).quatrelle") && f.is_file()), "{mine:?}");
    drop(s);
    std::fs::write(app.join(super::super::NAMES), b"{\"a1\": ").unwrap();
    let s = Store::open_packed(&app, &files).unwrap();
    // Two files of its name: neither is guessed, and the old one is no one's.
    assert_eq!(s.data_file("a1"), None);
    assert!(s.read_body(&k).is_none(), "the removed account's mail");
    assert!(old.is_file() && mine.is_some_and(|f| f.is_file()), "both files stay");
    drop(s);
    let _ = std::fs::remove_dir_all(t);
}

/// A list of data files that can't be read at all (a folder in its place, as a file another
/// program holds would be) stops the opening, and nothing is made: not the data files' folder,
/// nothing in the app's. Taken for an empty one, every account's file was forgotten and a second
/// one made beside it at its next write.
#[test]
fn a_list_of_data_files_that_cant_be_read_at_all_stops_the_opening() {
    let t = temp("names-unreadable");
    let (app, files) = (t.join("app"), t.join("Files"));
    std::fs::create_dir_all(app.join(super::super::NAMES)).unwrap();
    std::fs::write(app.join("accounts.json"), serde_json::to_vec(&[account("a1", "zoe@studio.example")]).unwrap()).unwrap();
    let before = snapshot(&t);
    let opened = Store::open_packed(&app, &files);
    assert_eq!(snapshot(&t), before, "nothing is made");
    let list = app.join(super::super::NAMES).display().to_string();
    assert!(matches!(&opened, Err(crate::StoreError::NamesUnreadable { path, .. }) if *path == list), "{:?}", opened.map(drop));
    let _ = std::fs::remove_dir_all(t);
}

/// A damaged list set aside twice keeps both: the second went over the first.
#[test]
fn a_damaged_list_set_aside_twice_keeps_both() {
    let t = temp("names-twice");
    let (app, files) = (t.join("app"), t.join("Files"));
    let s = Store::open_packed(&app, &files).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    s.save_account_doc("a1", "pim.json", &serde_json::json!({"notes": []})).unwrap();
    let file = s.data_file("a1");
    drop(s);
    std::fs::write(app.join(super::super::NAMES), b"{\"a1\": ").unwrap();
    drop(Store::open_packed(&app, &files).unwrap());
    std::fs::write(app.join(super::super::NAMES), b"[\"second\"").unwrap();
    let s = Store::open_packed(&app, &files).unwrap();
    assert_eq!(std::fs::read(app.join("datafiles.json.bad")).ok().as_deref(), Some(&b"{\"a1\": "[..]), "the first");
    assert_eq!(std::fs::read(app.join("datafiles.json.bad.2")).ok().as_deref(), Some(&b"[\"second\""[..]), "the second");
    assert_eq!(s.data_file("a1"), file, "found again both times");
    drop(s);
    let _ = std::fs::remove_dir_all(t);
}

/// A damaged list is made again from the list of accounts: one that can't be read, or is damaged,
/// stops the opening, and neither list is set aside or written. Read as no accounts, the list of
/// data files was set aside and written again empty.
#[test]
fn a_damaged_list_with_a_list_of_accounts_that_cant_be_read_touches_neither() {
    let t = temp("names-no-accounts");
    let (app, files) = (t.join("app"), t.join("Files"));
    let accounts = app.join("accounts.json");
    let said = accounts.display().to_string();
    let s = Store::open_packed(&app, &files).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    s.save_account_doc("a1", "pim.json", &serde_json::json!({"notes": []})).unwrap();
    drop(s);
    std::fs::write(app.join(super::super::NAMES), b"{\"a1\": ").unwrap();
    std::fs::write(&accounts, b"[{\"id\": ").unwrap();
    let before = snapshot(&t);
    let opened = Store::open_packed(&app, &files);
    assert_eq!(snapshot(&t), before, "a damaged list of accounts: neither list is set aside or written");
    assert!(matches!(&opened, Err(crate::StoreError::NamesUnreadable { path, .. }) if *path == said), "{:?}", opened.map(drop));
    // One that can't be read at all: the same.
    std::fs::remove_file(&accounts).unwrap();
    std::fs::create_dir(&accounts).unwrap();
    let before = snapshot(&t);
    let opened = Store::open_packed(&app, &files);
    assert_eq!(snapshot(&t), before, "an unreadable list of accounts: neither list is set aside or written");
    assert!(matches!(&opened, Err(crate::StoreError::NamesUnreadable { path, .. }) if *path == said), "{:?}", opened.map(drop));
    let _ = std::fs::remove_dir_all(t);
}
