// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Opening data files: from an existing one, damaged, online only, by threads at once, after a crash.

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
