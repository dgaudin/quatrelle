// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Names: the fork's old notes and extension, each account's data file named after it, folders from before.

use super::*;

/// A data file keeps the notes it wrote in its own bytes, so the ones from before the rename
/// (the fork's name) are still what most of it carries: they're read, and mail, calendars and
/// settings come back whole rather than as "unreadable, starting fresh".
#[test]
fn values_written_before_the_rename_are_read() {
    use redb::ReadableTable;

    use super::super::{DEFLATED, FILES, PIECED, WHOLE_MAX};
    use crate::legacy::{OLD_DEFLATED, OLD_PIECED};
    let t = temp("old-markers");
    let (root, files) = (t.join("app"), t.join("Files"));
    let small = serde_json::json!({ "not_read": (0..200).map(|i| format!("cal{i}@example.org")).collect::<Vec<_>>() });
    // Random hex: compresses by about a half, so it still goes in pieces (a run of one letter
    // would go whole, it deflates so well).
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let big: String = (0..WHOLE_MAX * 4)
        .map(|_| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            char::from_digit(((seed >> 33) % 16) as u32, 16).unwrap_or('0')
        })
        .collect();
    {
        let s = Store::open_packed(&root, &files).unwrap();
        s.save_account_doc("a1", "pim.json", &small).unwrap();
        s.save_account_doc("a1", "big.txt", &big).unwrap();
    }
    // Rewrite both values with the notes the fork used, as a data file from before has them.
    let db = redb::Database::open(files.join("a1.quatrelle")).unwrap();
    {
        let tx = db.begin_write().unwrap();
        {
            let mut table = tx.open_table(FILES).unwrap();
            let kept = table.get("pim.json").unwrap().map(|v| v.value().to_vec()).unwrap();
            assert!(kept.starts_with(DEFLATED), "the test needs a compressed value");
            let mut old = OLD_DEFLATED.to_vec();
            old.extend_from_slice(&kept[DEFLATED.len()..]);
            table.insert("pim.json", old.as_slice()).unwrap();
            let note = table.get("big.txt").unwrap().map(|v| v.value().to_vec()).unwrap();
            assert!(note.starts_with(PIECED) && note.len() == PIECED.len() + 8, "the test needs a value in pieces");
            let mut old = OLD_PIECED.to_vec();
            old.extend_from_slice(&note[PIECED.len()..]);
            table.insert("big.txt", old.as_slice()).unwrap();
            // Its stream, read as its pieces are, starts with the old prefix too.
            let mut pieces = tx.open_table(super::super::PIECES).unwrap();
            let first = pieces.get(("big.txt", 0)).unwrap().map(|v| v.value().to_vec()).unwrap();
            assert!(first.starts_with(DEFLATED), "the test needs a value compressed in pieces");
            let mut old = OLD_DEFLATED.to_vec();
            old.extend_from_slice(&first[DEFLATED.len()..]);
            pieces.insert(("big.txt", 0), old.as_slice()).unwrap();
        }
        tx.commit().unwrap();
    }
    drop(db);

    let s = Store::open_packed(&root, &files).unwrap();
    assert_eq!(s.load_account_doc::<serde_json::Value>("a1", "pim.json").unwrap(), small, "a compressed value comes back");
    assert_eq!(s.load_account_doc::<String>("a1", "big.txt").unwrap(), big, "one kept in pieces comes back");
    let _ = std::fs::remove_dir_all(&t);
}

/// A data file written under the fork's old extension is renamed, once, and the names file
/// follows it: the account keeps its mail, its file and its place in the folder.
#[test]
fn an_old_extension_data_file_is_taken_over() {
    use super::super::{EXTENSION, NAMES};
    use quatrelle_model::{Folder, FolderRole};
    let t = temp("old-extension");
    let root = t.join("root");
    let files = t.join("Files");
    {
        let store = Store::open_packed(&root, &files).unwrap();
        store.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
        store.save_folders("a1", &[Folder { path: "INBOX".into(), role: FolderRole::Inbox, ..Default::default() }]).unwrap();
        let k = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 7 };
        store.write_body(&k, b"Subject: hi\r\n\r\nhello").unwrap();
    }
    let made = std::fs::read_dir(&files).unwrap().flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|e| e == EXTENSION)).unwrap();
    let old_name = format!("{}.{}", made.file_stem().unwrap().to_string_lossy(), crate::legacy::OLD_EXTENSION);
    std::fs::rename(&made, files.join(&old_name)).unwrap();
    // The names file still points at the old name, as it did before the rename.
    let names = std::fs::read_to_string(root.join(NAMES)).unwrap();
    std::fs::write(root.join(NAMES), names.replace(&*made.file_name().unwrap().to_string_lossy(), &old_name)).unwrap();

    let store = Store::open_packed(&root, &files).unwrap();
    let taken = store.data_file("a1").unwrap();
    assert_eq!(taken.extension().unwrap(), EXTENSION);
    assert!(taken.exists(), "{} is there", taken.display());
    assert!(!files.join(&old_name).exists(), "the old file is gone, renamed");
    let k = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 7 };
    assert_eq!(store.read_body(&k).unwrap(), b"Subject: hi\r\n\r\nhello", "its mail came along");
    assert!(store.load_folders("a1").unwrap().iter().any(|f| f.path == "INBOX"), "its folders too");
    let _ = std::fs::remove_dir_all(&t);
}

#[test]
fn each_account_is_one_data_file_named_after_it() {
    let t = temp("one-file");
    let s = Store::open_packed(t.join("app"), t.join("Quatrelle Files")).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example"), account("a2", "old.pst")]).unwrap();
    let k = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 7 };
    s.write_body(&k, b"Subject: hi\r\n\r\nhello").unwrap();
    let mut st = s.load_folder("a1", "INBOX").unwrap();
    st.validity = 1;
    st.messages.push(Summary { key: k.clone(), subject: "hi".into(), flags: Flags::default(), ..Default::default() });
    s.save_folder("a1", "INBOX", &st).unwrap();
    s.save_account_doc("a2", "pim.json", &serde_json::json!({"x": 1})).unwrap();
    // One file per account, named as Outlook names a .pst; nothing loose in the app folder.
    let mut files: Vec<String> =
        std::fs::read_dir(t.join("Quatrelle Files")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    files.sort();
    assert_eq!(files, ["old.quatrelle", "zoe@studio.example.quatrelle"]);
    assert!(!t.join("app").join("a1").exists());
    assert_eq!(s.data_file("a1"), Some(t.join("Quatrelle Files").join("zoe@studio.example.quatrelle")));
    // Read back, also after opening again.
    drop(s);
    let s = Store::open_packed(t.join("app"), t.join("Quatrelle Files")).unwrap();
    assert_eq!(s.read_body(&k).unwrap(), b"Subject: hi\r\n\r\nhello");
    assert_eq!(s.load_folder("a1", "INBOX").unwrap().messages.len(), 1);
    assert!(s.account_size("a1") > 0);
    // Folder renames move inside the file; the account going takes its file.
    s.save_folders("a1", &[quatrelle_model::Folder { account: "a1".into(), path: "INBOX".into(), selectable: true, ..Default::default() }]).unwrap();
    s.apply_op("a1", &quatrelle_model::Op::RenameFolder { from: "INBOX".into(), to: "Kept".into() }).unwrap();
    assert_eq!(s.load_folder("a1", "Kept").unwrap().messages.len(), 1);
    assert!(s.read_body(&MessageKey { folder: "Kept".into(), ..k.clone() }).is_some(), "its messages moved too");
    // Removing the account keeps its file, as Outlook keeps a .pst: closed, and no one's.
    let kept = t.join("Quatrelle Files").join("zoe@studio.example.quatrelle");
    assert_eq!(s.remove_account("a1").unwrap(), Some(kept.clone()));
    assert!(kept.exists() && s.data_file("a1").is_none());
    // It still reads (as a file opened from Open & Export).
    let mut keys = Vec::new();
    super::super::read_data_file(&kept, &t.join("read.tmp"), &mut |batch| {
        keys.extend(batch.into_iter().map(|(k, _)| k));
        Ok(())
    })
    .unwrap();
    assert!(keys.iter().any(|k| k.ends_with(".eml")) && keys.iter().any(|k| k == "folders.json"), "{keys:?}");
    // The same address added again: a file of its own beside the kept one, which stays as it was.
    let before = std::fs::read(&kept).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    s.save_folder("a1", "INBOX", &st).unwrap();
    assert_eq!(s.data_file("a1"), Some(t.join("Quatrelle Files").join("zoe@studio.example (2).quatrelle")));
    assert_eq!(std::fs::read(&kept).unwrap(), before);
    // Deleting (what never held anyone's mail) takes the file.
    s.delete_account("a1").unwrap();
    assert!(!t.join("Quatrelle Files").join("zoe@studio.example (2).quatrelle").exists() && kept.exists());
    let _ = std::fs::remove_dir_all(t);
}

#[test]
fn folders_from_before_come_into_the_data_file() {
    let t = temp("bring-in");
    let app = t.join("app");
    let old = Store::open(&app).unwrap();
    old.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    let k = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 3, uid: 1 };
    old.write_body(&k, b"Subject: kept\r\n\r\nbody").unwrap();
    old.save_account_doc("a1", "pop-seen.json", &vec!["u1".to_string()]).unwrap();
    assert!(app.join("a1").is_dir());
    let s = Store::open_packed(&app, t.join("Files")).unwrap();
    assert!(!app.join("a1").exists(), "the folder went once it was in");
    assert_eq!(s.read_body(&k).unwrap(), b"Subject: kept\r\n\r\nbody");
    assert_eq!(s.load_account_doc::<Vec<String>>("a1", "pop-seen.json").unwrap(), vec!["u1".to_string()]);
    assert!(t.join("Files").join("zoe@studio.example.quatrelle").is_file());
    let _ = std::fs::remove_dir_all(t);
}
