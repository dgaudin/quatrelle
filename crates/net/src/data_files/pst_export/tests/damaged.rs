// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Data files added to that are damaged in ways the writer can work around.

use outlook_pst::ltp::prop_context::PropertyValue as V;

use super::state::props_of;
use super::*;

/// A data file whose name-to-ID map can't take names (here: its bucket count is 0) is still
/// added to: a flagged message in a category goes in flagged, without its named properties.
#[test]
fn an_export_adds_to_a_file_whose_name_map_is_damaged() {
    let dir = std::env::temp_dir().join(format!("quatrelle-pst-export-name-map-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("Theirs.pst");
    theirs(&file);
    // The map's PidTagNameidBucketCount record (its ID 0x0001, PtypInteger32) set to 0, in the
    // one block that has it, its CRC made right.
    let mut f = std::fs::read(&file).unwrap();
    let mut patched = 0;
    for end in (64..=f.len()).step_by(64) {
        let size = usize::from(u16::from_le_bytes([f[end - 16], f[end - 15]]));
        let Some(start) = end.checked_sub((size + 16).div_ceil(64) * 64) else { continue };
        if size == 0 || size > 8176 || crc(&f[start..start + size]) != u32::from_le_bytes(f[end - 12..end - 8].try_into().unwrap()) {
            continue;
        }
        let Some(at) = f[start..start + size].windows(8).position(|w| w[..4] == [1, 0, 3, 0] && w[4..] != [0, 0, 0, 0]) else { continue };
        f[start + at + 4..start + at + 8].copy_from_slice(&0u32.to_le_bytes());
        let c = crc(&f[start..start + size]);
        f[end - 12..end - 8].copy_from_slice(&c.to_le_bytes());
        patched += 1;
    }
    assert_eq!(patched, 1);
    std::fs::write(&file, f).unwrap();

    let store = Store::open(dir.join("cache")).unwrap();
    let d = mime::Draft { subject: "Ours".into(), body: "Flagged.".into(), ..Default::default() };
    let mut messages = Vec::new();
    for uid in [1, 2] {
        let raw = mime::build(&d, &EmailAddress::new("Riley", "riley@example.com"), 1_704_067_200, &format!("ours{uid}@example.com")).unwrap();
        let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
        store.write_body(&key, &raw).unwrap();
        let flags = Flags { flagged: true, keywords: vec!["Red".into()], ..Default::default() };
        messages.push(mime::summary_from_full(&raw, key, flags, 10, 0));
    }
    store.save_folder("a1", "INBOX", &FolderState { validity: 1, uid_next: 3, messages, ..Default::default() }).unwrap();
    let inbox =
        quatrelle_model::Folder { account: "a1".into(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
    store.save_folders("a1", &[inbox]).unwrap();

    assert_eq!(export(&store, "a1", "Riley's mail", &file, NONE, &mut |_| {}).unwrap().messages, 2);
    let [flag, subject] = props_of(&file, &["Inbox"], "Ours", &[0x1090, 0x0037]).try_into().unwrap();
    assert!(matches!(flag, Some(V::Integer32(2))));
    assert!(subject.is_some());
    let back = Store::open(dir.join("back")).unwrap();
    assert_eq!(crate::pst::import(&file, &back, "b1", &mut |_| {}).unwrap().messages, 4);
    let _ = std::fs::remove_dir_all(&dir);
}
