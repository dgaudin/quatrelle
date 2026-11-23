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
