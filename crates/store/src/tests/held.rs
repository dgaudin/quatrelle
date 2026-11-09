// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Tests: an online archive's folders are held in memory, never written to this computer.

use super::*;

fn on_disk(dir: &std::path::Path, needle: &[u8]) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else { return false };
    rd.flatten().any(|e| {
        let p = e.path();
        if p.is_dir() { on_disk(&p, needle) } else { std::fs::read(&p).is_ok_and(|b| b.windows(needle.len()).any(|w| w == needle)) }
    })
}

fn folder(path: &str, archive: bool) -> Folder {
    Folder { account: "a1".into(), path: path.into(), delimiter: Some('/'), selectable: true, archive, ..Default::default() }
}

fn listed(folder: &str, subject: &str) -> FolderState {
    let mut m = msg(1, false);
    m.key.folder = folder.into();
    m.subject = subject.into();
    FolderState { validity: 1, uid_next: 2, messages: vec![m], ..Default::default() }
}

/// What's read of an online archive's folder (its list, a message's source, its search index, a
/// document kept with it) is there while Quatrelle runs and never reaches the disk, in a data
/// file or in plain files; the account's own folders are kept as before. A new run starts with
/// nothing of the archive.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn an_archive_folders_files_are_never_written() {
    let t = std::env::temp_dir().join(format!("quatrelle-store-held-{}", std::process::id()));
    for packed in [true, false] {
        let _ = std::fs::remove_dir_all(&t);
        let open = || if packed { Store::open_packed(t.join("app"), t.join("files")).unwrap() } else { Store::open(t.join("app")).unwrap() };
        let s = open();
        let folders = [folder("INBOX", false), folder("Online Archive", true), folder("Online Archive/2019", true)];
        s.save_folders("a1", &folders).unwrap();
        for (path, subject) in [("INBOX", "KEPT-SUBJECT"), ("Online Archive/2019", "ARCHIVE-SUBJECT")] {
            let st = listed(path, subject);
            s.save_folder("a1", path, &st).unwrap();
            let key = st.messages[0].key.clone();
            s.write_body(&key, format!("Subject: {subject}\r\n\r\n{subject}-BODY\r\n").as_bytes()).unwrap();
            s.save_search_index("a1", path, format!("{subject}-INDEX").as_bytes()).unwrap();
            s.save_folder_doc("a1", path, "doc.json", &vec![format!("{subject}-DOC")]).unwrap();
            // There while it runs.
            assert_eq!(s.load_folder("a1", path).unwrap().messages[0].subject, subject);
            assert!(s.has_body(&key), "{path}");
            assert!(s.load_search_index("a1", path).is_some(), "{path}");
        }
        // The clone the account's thread works with sees the same.
        assert_eq!(s.clone().load_folder("a1", "Online Archive/2019").unwrap().messages.len(), 1);
        drop(s);
        assert!(on_disk(&t, b"KEPT-SUBJECT") && on_disk(&t, b"KEPT-SUBJECT-BODY"), "packed {packed}: the account's own folder is kept");
        for needle in ["ARCHIVE-SUBJECT", "ARCHIVE-SUBJECT-BODY", "ARCHIVE-SUBJECT-INDEX", "ARCHIVE-SUBJECT-DOC"] {
            assert!(!on_disk(&t, needle.as_bytes()), "packed {packed}: {needle} reached the disk");
        }
        // A new run: the folder list says which are the archive's before anything is read.
        let s = open();
        assert_eq!(s.load_folders("a1").unwrap().iter().filter(|f| f.archive).count(), 2);
        assert!(s.load_folder("a1", "Online Archive/2019").unwrap().messages.is_empty(), "nothing of the archive from the last run");
        assert_eq!(s.load_folder("a1", "INBOX").unwrap().messages.len(), 1);
        // Read again after the restart, held again.
        s.save_folder("a1", "Online Archive/2019", &listed("Online Archive/2019", "ARCHIVE-AGAIN")).unwrap();
        drop(s);
        assert!(!on_disk(&t, b"ARCHIVE-AGAIN"), "packed {packed}");
    }
    let _ = std::fs::remove_dir_all(&t);
}

/// A folder no longer the archive's (the archive went from the server) lets go of what was held;
/// an account turned online only and back keeps its own mail and still writes nothing of the
/// archive.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn held_folders_follow_the_folder_list() {
    let t = std::env::temp_dir().join(format!("quatrelle-store-held-list-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&t);
    let s = Store::open_packed(t.join("app"), t.join("files")).unwrap();
    s.save_folders("a1", &[folder("INBOX", false), folder("Online Archive", true)]).unwrap();
    s.save_folder("a1", "INBOX", &listed("INBOX", "KEPT-SUBJECT")).unwrap();
    s.save_folder("a1", "Online Archive", &listed("Online Archive", "ARCHIVE-SUBJECT")).unwrap();
    s.set_online_only("a1", true).unwrap();
    assert_eq!(s.load_folder("a1", "Online Archive").unwrap().messages.len(), 1);
    s.set_online_only("a1", false).unwrap();
    assert_eq!(s.load_folder("a1", "INBOX").unwrap().messages.len(), 1);
    // The archive is gone from the list: what was held of it goes.
    s.save_folders("a1", &[folder("INBOX", false)]).unwrap();
    assert!(s.load_folder("a1", "Online Archive").unwrap().messages.is_empty());
    drop(s);
    assert!(on_disk(&t, b"KEPT-SUBJECT"));
    assert!(!on_disk(&t, b"ARCHIVE-SUBJECT"), "written out with the account");
    let _ = std::fs::remove_dir_all(&t);
}
