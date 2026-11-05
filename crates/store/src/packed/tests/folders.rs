// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Listing a folder.

use super::*;

/// A folder's files are listed by key alone: the folders inside it are passed over in one step
/// each, not walked message by message (a mail folder's list beside thousands of messages read
/// them all), and the names come out as they did, cached mail included.
#[test]
fn a_folder_is_listed_without_walking_what_is_inside_it() {
    let t = temp("children");
    let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    s.save_accounts(&[account("a1", "zoe@studio.example")]).unwrap();
    let key = |validity: u32, uid: u32| MessageKey { account: "a1".into(), folder: "INBOX".into(), validity, uid };
    let mut bodies: Vec<(MessageKey, Vec<u8>)> = (1..=2000).map(|uid| (key(7, uid), b"Subject: x\r\n\r\nx".to_vec())).collect();
    bodies.push((key(70, 1), b"Subject: y\r\n\r\ny".to_vec()));
    s.write_bodies(&bodies).unwrap();
    let st = crate::FolderState::<crate::Summary> { validity: 7, ..Default::default() };
    s.save_folder("a1", "INBOX", &st).unwrap();
    let dir = s.folder_dir("a1", "INBOX");
    // Names beside the folders that sort before, between and after them.
    for name in ["7.json", "6-x.json", "70.json", "8.json"] {
        s.files.write(&dir.join(name), b"{}").unwrap();
    }
    s.files.write(&dir.join("7").join("deeper").join("1.eml"), b"z").unwrap();

    let mut names: Vec<String> = s
        .files
        .list(&dir)
        .into_iter()
        .map(|(n, p)| {
            assert_eq!(p, dir.join(&n));
            n
        })
        .collect();
    names.sort();
    assert_eq!(names, ["6-x.json", "7.json", "70.json", "8.json", "state.json"]);
    let crate::Files::Packed(packs) = &s.files.main else { panic!("packed") };
    let inside = dir.strip_prefix(t.join("app").join("a1")).unwrap().to_str().unwrap().to_string();
    let (_, looked) = packs.children("a1", &inside).unwrap();
    // The five files, the two folders and the key after the last one: not the 2,002 inside.
    assert!(looked <= 8, "looked at {looked} keys");
    let mut cached = s.cached_bodies("a1", "INBOX", 7);
    cached.sort_unstable();
    assert_eq!(cached, (1..=2000).collect::<Vec<u32>>(), "only the messages, not the folder inside");
    assert_eq!(s.cached_bodies("a1", "INBOX", 70), [1]);
    assert!(s.cached_bodies("a1", "INBOX", 9).is_empty() && s.cached_bodies("a1", "Other", 7).is_empty());
    // The account's own files: its folders aren't walked either.
    let (top, looked) = packs.children("a1", "").unwrap();
    assert!(top.is_empty() && looked <= 3, "{top:?} {looked}");
    // A folder exists when anything is under it, found from the first key.
    assert!(s.files.exists(&dir.join("70")) && s.files.exists(&dir.join("7").join("deeper")));
    assert!(!s.files.exists(&dir.join("9")) && !s.files.exists(&dir.join("7").join("de")));
    drop(s);
    let _ = std::fs::remove_dir_all(&t);
}
