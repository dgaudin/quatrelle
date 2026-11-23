// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Tests: turning online-only on never removes a file it didn't read.

use super::*;

/// Turning online-only on with one of the account's files unreadable leaves it all where it was,
/// and says so. That file was left behind unread and its only copy removed with the rest (its
/// changes waiting for the server among what could go).
#[test]
fn online_only_with_a_file_that_cant_be_read_keeps_it() {
    let s = Store::failing();
    s.save_folder("a1", "INBOX", &FolderState { validity: 3, uid_next: 2, messages: vec![msg(1, false)], ..Default::default() }).unwrap();
    s.add_op("a1", Op::SetFlag { keys: vec![msg(1, false).key], flag: Flag::Seen, on: true, all: false }).unwrap();
    let waiting = s.account_files("a1", "ops").unwrap();
    assert_eq!(waiting.len(), 1, "one change waiting");
    let name = waiting[0].clone();
    assert!(s.files.main.walk(&s.account_dir("a1")).unwrap().iter().any(|p| p.ends_with(&name)), "the walk lists the file");
    s.fail(Some(&name), None);
    assert!(s.set_online_only("a1", true).is_err());
    s.fail(None, None);
    assert!(!s.is_online_only("a1"));
    assert_eq!((s.load_ops("a1").unwrap().len(), s.load_folder("a1", "INBOX").unwrap().messages.len()), (1, 1), "still there");
    // Readable again, all of it goes over.
    s.set_online_only("a1", true).unwrap();
    assert_eq!((s.load_ops("a1").unwrap().len(), s.load_folder("a1", "INBOX").unwrap().messages.len()), (1, 1));
}

/// Gives a folder its access back when dropped, so a failed assertion leaves nothing unreadable.
#[cfg(unix)]
struct Access(PathBuf);

#[cfg(unix)]
impl Drop for Access {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o700));
    }
}

/// A disk cache with a folder of the account that can't be read keeps all of it: the walk said
/// so by an error, not by a list without what's in that folder.
#[cfg(unix)]
#[test]
fn online_only_on_disk_with_a_folder_that_cant_be_read_keeps_it() {
    use std::os::unix::fs::PermissionsExt;
    let s = temp_store("online-only-disk");
    s.save_folder("a1", "INBOX", &FolderState { validity: 3, uid_next: 2, messages: vec![msg(1, false)], ..Default::default() }).unwrap();
    s.add_op("a1", Op::SetFlag { keys: vec![msg(1, false).key], flag: Flag::Seen, on: true, all: false }).unwrap();
    let ops = s.account_dir("a1").join("ops");
    let _access = Access(ops.clone());
    fs::set_permissions(&ops, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read_dir(&ops).is_ok() {
        log::warn!("this account reads any folder (root): nothing is asserted");
        return;
    }
    assert!(s.set_online_only("a1", true).is_err(), "a folder it can't read stops the switch");
    assert!(!s.is_online_only("a1"));
    fs::set_permissions(&ops, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(s.load_ops("a1").unwrap().len(), 1, "the change waiting is still there");
    assert_eq!(s.load_folder("a1", "INBOX").unwrap().messages.len(), 1);
    let _ = fs::remove_dir_all(s.account_dir("a1").parent().unwrap_or(Path::new("")));
}

/// A tree deeper than the walk goes isn't skipped and then removed unread: the switch is refused.
#[test]
fn online_only_with_a_tree_deeper_than_the_walk_goes_keeps_it() {
    let s = temp_store("online-only-deep");
    s.save_folder("a1", "INBOX", &FolderState { validity: 3, uid_next: 2, messages: vec![msg(1, false)], ..Default::default() }).unwrap();
    let mut deep = s.account_dir("a1").join("deep");
    for i in 0..12 {
        deep = deep.join(format!("d{i}"));
    }
    fs::create_dir_all(&deep).unwrap();
    fs::write(deep.join("kept.json"), b"only copy").unwrap();
    assert!(s.set_online_only("a1", true).is_err(), "what it can't walk stops the switch");
    assert!(!s.is_online_only("a1"));
    assert_eq!(fs::read(deep.join("kept.json")).unwrap(), b"only copy", "nothing removed");
    assert_eq!(s.load_folder("a1", "INBOX").unwrap().messages.len(), 1);
    let _ = fs::remove_dir_all(s.account_dir("a1").parent().unwrap_or(Path::new("")));
}

/// A listing of the account that fails (as `try_list` does) stops the switch in memory too.
#[test]
fn online_only_with_a_folder_that_fails_to_list_keeps_it() {
    let s = Store::failing();
    s.save_folder("a1", "INBOX", &FolderState { validity: 3, uid_next: 2, messages: vec![msg(1, false)], ..Default::default() }).unwrap();
    s.add_op("a1", Op::SetFlag { keys: vec![msg(1, false).key], flag: Flag::Seen, on: true, all: false }).unwrap();
    s.fail(Some("a1"), None);
    assert!(s.set_online_only("a1", true).is_err());
    s.fail(None, None);
    assert!(!s.is_online_only("a1"));
    assert_eq!((s.load_ops("a1").unwrap().len(), s.load_folder("a1", "INBOX").unwrap().messages.len()), (1, 1), "still there");
}
