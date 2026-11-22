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
