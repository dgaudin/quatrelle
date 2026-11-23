// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Store tests: a document read strictly (`Store::load_account_doc_strict`), which must never be
//! taken for an empty one when it can't be read or is damaged.

use super::*;

/// A damaged document is an error that names it, and stays where it is, as it is: not set aside
/// and started afresh as the usual read does. One that isn't there is empty; one that can't be
/// read is the disk's error.
#[test]
fn a_document_read_strictly_is_never_taken_for_an_empty_one() {
    let s = Store::failing();
    let none: Vec<String> = s.load_account_doc_strict("a1", "followed.json").unwrap();
    assert!(none.is_empty(), "none there: empty");
    s.save_account_doc("a1", "followed.json", &["Public Folders/Team"]).unwrap();
    let read: Vec<String> = s.load_account_doc_strict("a1", "followed.json").unwrap();
    assert_eq!(read, ["Public Folders/Team"]);
    let damaged = b"[\"Public Folders/Te".to_vec();
    s.write_account_files("a1", &[("followed.json".into(), damaged.clone())]).unwrap();
    let read = s.load_account_doc_strict::<Vec<String>>("a1", "followed.json");
    assert!(matches!(&read, Err(StoreError::Damaged { path }) if path.ends_with("followed.json")), "{read:?}");
    assert_eq!(s.account_file("a1", "followed.json").unwrap(), Some(damaged.clone()), "left as it is");
    assert_eq!(s.account_file("a1", "followed.json.bad").unwrap(), None, "not set aside");
    s.fail(Some("followed.json"), None);
    let read = s.load_account_doc_strict::<Vec<String>>("a1", "followed.json");
    s.fail(None, None);
    assert!(matches!(&read, Err(StoreError::Io { .. })), "{read:?}");
    assert_eq!(s.account_file("a1", "followed.json").unwrap(), Some(damaged), "left as it is");
}

fn two_accounts() -> Vec<quatrelle_model::AccountConfig> {
    let account = |id: &str, email: &str| quatrelle_model::AccountConfig { id: id.into(), email: email.into(), ..Default::default() };
    vec![account("a1", "zoe@studio.example"), account("a2", "kim@contoso.example")]
}

/// A list of accounts that is damaged is an error that names it, and stays where it is, byte for
/// byte: not set aside and read as none, which the next save would have written over.
#[test]
fn a_damaged_list_of_accounts_is_an_error_and_stays_where_it_is() {
    let s = temp_store("accounts-damaged");
    s.save_accounts(&two_accounts()).unwrap();
    let list = s.root().join("accounts.json");
    let damaged = b"[{\"id\": \"a1\", \"email\": \"zoe@stu".to_vec();
    fs::write(&list, &damaged).unwrap();
    let read = s.load_accounts();
    assert!(matches!(&read, Err(StoreError::Damaged { path }) if path.ends_with("accounts.json")), "{read:?}");
    assert_eq!(fs::read(&list).unwrap(), damaged, "left as it is");
    assert!(!s.root().join("accounts.json.bad").exists(), "not set aside");
    let _ = fs::remove_dir_all(s.root());
}

/// A list of accounts that can't be read (here a folder in its place) is the disk's error, and is
/// left alone.
#[test]
fn an_unreadable_list_of_accounts_is_an_error_and_stays_where_it_is() {
    let s = temp_store("accounts-unreadable");
    let list = s.root().join("accounts.json");
    fs::create_dir(&list).unwrap();
    let read = s.load_accounts();
    assert!(matches!(&read, Err(StoreError::Io { path, .. }) if path.ends_with("accounts.json")), "{read:?}");
    assert!(list.is_dir() && !s.root().join("accounts.json.bad").exists());
    let _ = fs::remove_dir_all(s.root());
}

/// Guard: no list of accounts yet is no account; one that is there is read.
#[test]
fn no_list_of_accounts_is_no_account() {
    let s = temp_store("accounts-none");
    assert!(s.load_accounts().unwrap().is_empty());
    s.save_accounts(&two_accounts()).unwrap();
    assert_eq!(s.load_accounts().unwrap().len(), 2);
    let _ = fs::remove_dir_all(s.root());
}

fn folder(path: &str) -> Folder {
    Folder { account: "a1".into(), path: path.into(), delimiter: Some('/'), selectable: true, ..Default::default() }
}

/// What a damaged folder list's file holds.
const DAMAGED_LIST: &[u8] = b"[{\"account\":\"a1\",\"path\":\"Pla";

/// Found in review: a folder list whose file was damaged was renamed `.json.bad` and read as an
/// empty one, with no error: every folder kept here was taken for gone. It's an error that names
/// the file, which stays where it is, byte for byte.
#[test]
fn a_damaged_folder_list_is_an_error_and_stays_where_it_is() {
    let s = Store::failing();
    s.save_folders("a1", &[folder("INBOX"), folder("Plans")]).unwrap();
    s.write_account_files("a1", &[("folders.json".into(), DAMAGED_LIST.to_vec())]).unwrap();
    let read = s.load_folders("a1");
    assert!(matches!(&read, Err(StoreError::Damaged { path }) if path.ends_with("folders.json")), "{read:?}");
    assert_eq!(s.account_file("a1", "folders.json").unwrap().as_deref(), Some(DAMAGED_LIST), "left as it is");
    assert_eq!(s.account_file("a1", "folders.json.bad").unwrap(), None, "set aside");
}

/// A damaged folder list's bytes, told apart by their last.
fn damaged_list(last: u8) -> Vec<u8> {
    let mut bytes = DAMAGED_LIST.to_vec();
    bytes.push(last);
    bytes
}

/// A damaged folder list that a list saved replaces is set aside first, a copy written whole under
/// a name of its own (`.bad`, then `.bad.2`…, the oldest going past nine): never written over.
/// One that can't be set aside stays, and nothing is saved over it; a list that reads is replaced
/// as ever, nothing set aside.
#[test]
fn a_damaged_folder_list_is_set_aside_before_the_one_that_replaces_it() {
    let s = Store::failing();
    let list = [folder("INBOX"), folder("Plans")];
    let damage = |last: u8| s.write_account_files("a1", &[("folders.json".into(), damaged_list(last))]).unwrap();
    let kept = |i: u32| s.account_file("a1", &if i == 1 { "folders.json.bad".to_string() } else { format!("folders.json.bad.{i}") }).unwrap();
    damage(b'1');
    // A copy that can't be written: the damaged list stays, nothing saved over it.
    s.fail(None, Some("folders.json.bad"));
    let saved = s.save_folders("a1", &list);
    s.fail(None, None);
    assert!(saved.is_err(), "saved over a damaged list not set aside");
    assert_eq!(s.account_file("a1", "folders.json").unwrap(), Some(damaged_list(b'1')), "written over");
    s.save_folders("a1", &list).unwrap();
    assert_eq!(s.load_folders("a1").unwrap(), list);
    assert_eq!(kept(1), Some(damaged_list(b'1')), "not set aside as it was");
    // A list that reads is replaced, nothing set aside.
    s.save_folders("a1", &list[..1]).unwrap();
    assert_eq!(kept(2), None);
    // Each damaged one a name of its own: none set aside before is written over.
    for last in b'2'..=b'9' {
        damage(last);
        s.save_folders("a1", &list).unwrap();
    }
    for (i, last) in (1..=9).zip(b'1'..=b'9') {
        assert_eq!(kept(i), Some(damaged_list(last)), "the {i}th set aside");
    }
    // Past nine, the oldest goes: the others move down one, the newest last.
    damage(b'X');
    s.save_folders("a1", &list).unwrap();
    assert_eq!(kept(1), Some(damaged_list(b'2')), "the oldest stays");
    assert_eq!(kept(8), Some(damaged_list(b'9')));
    assert_eq!(kept(9), Some(damaged_list(b'X')), "the newest isn't kept");
    assert_eq!(kept(10), None);
}

/// Counter-probe: a new account, with no folder list and no change waiting, has an empty list, an
/// empty queue, and its first list saved with nothing set aside.
#[test]
fn a_folder_list_that_isnt_there_is_still_an_empty_one() {
    let s = Store::failing();
    assert!(s.load_folders("a1").unwrap().is_empty());
    assert!(s.load_ops("a1").unwrap().is_empty() && s.first_op("a1").unwrap().is_none());
    s.save_folders("a1", &[folder("INBOX")]).unwrap();
    assert_eq!(s.load_folders("a1").unwrap(), [folder("INBOX")]);
    assert_eq!(s.account_file("a1", "folders.json.bad").unwrap(), None);
}

/// A document read strictly (the marks owed by replies in the Outbox): damaged, it's an error and
/// stays as it is; set aside, a copy of it is kept (never written over) and it's gone from its
/// place, read next as none and saved again.
#[test]
fn a_damaged_document_is_an_error_then_set_aside_whole() {
    let s = Store::failing();
    let none: Vec<u32> = s.load_doc_strict("pending-marks.json").unwrap();
    assert!(none.is_empty());
    let damaged = b"[[7,{\"account\":\"a1\",".to_vec();
    for (round, bad) in ["pending-marks.json.bad", "pending-marks.json.bad.2"].into_iter().enumerate() {
        s.save_doc("pending-marks.json", &[round]).unwrap();
        assert_eq!(s.load_doc_strict::<Vec<usize>>("pending-marks.json").unwrap(), [round]);
        let path = s.root().join("pending-marks.json");
        s.files.write(&path, &damaged).unwrap();
        let read = s.load_doc_strict::<Vec<usize>>("pending-marks.json");
        assert!(matches!(&read, Err(StoreError::Damaged { path }) if path.ends_with("pending-marks.json")), "{read:?}");
        assert_eq!(s.files.read(&path).ok(), Some(damaged.clone()), "left as it is");
        s.set_doc_aside("pending-marks.json").unwrap();
        assert_eq!(s.files.read(&s.root().join(bad)).ok(), Some(damaged.clone()), "{round}: not set aside as it was");
        assert!(s.load_doc_strict::<Vec<usize>>("pending-marks.json").unwrap().is_empty(), "{round}: read as none next");
    }
}
