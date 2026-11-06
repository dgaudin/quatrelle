use super::*;
use crate::extra::tests::{cell, folder, message, messages, temp_pst, text};

const ROLES: [UnicodePstSpecialFolder; 5] = [
    UnicodePstSpecialFolder::DeletedItems,
    UnicodePstSpecialFolder::SentItems,
    UnicodePstSpecialFolder::Outbox,
    UnicodePstSpecialFolder::Drafts,
    UnicodePstSpecialFolder::JunkEmail,
];

fn outlook_pst(label: &str) -> std::path::PathBuf {
    let path = temp_pst(label);
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/Empty.pst"),
        &path,
    )
    .unwrap();
    path
}

fn open_store(path: &Path) -> Rc<UnicodeStore> {
    UnicodeStore::read(Rc::new(UnicodePstFile::open(path).unwrap())).unwrap()
}

#[test]
fn a_folder_is_made_with_its_container_class() {
    let path = temp_pst("container-class");
    drop(UnicodePstFile::create(&path, &message("First", "<first@example.com>", &[])).unwrap());
    drop(UnicodePstFile::create_folder(&path, &["Plans", "Dates"], "IPF.Appointment").unwrap());
    // A folder already there keeps its own.
    drop(UnicodePstFile::create_folder(&path, &["Inbox"], "IPF.Contact").unwrap());
    assert_eq!(
        UnicodePstFile::create_folder(&path, &["Other"], "")
            .err()
            .unwrap()
            .kind(),
        io::ErrorKind::InvalidInput
    );

    let store = open_store(&path);
    let class = |names: &[&str]| text(folder(&store, names).1.properties().get(0x3613));
    assert_eq!(class(&["Plans"]), "IPF.Note");
    assert_eq!(class(&["Plans", "Dates"]), "IPF.Appointment");
    assert_eq!(class(&["Inbox"]), "IPF.Note");
    let (dates, _) = folder(&store, &["Plans", "Dates"]);
    let (_, plans) = folder(&store, &["Plans"]);
    let hierarchy = plans.hierarchy_table().unwrap();
    if let Some(value) = cell(hierarchy.as_ref(), u32::from(dates), 0x3613) {
        assert_eq!(text(Some(&value)), "IPF.Appointment");
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn special_folders_are_recorded_and_found() {
    let path = temp_pst("special-folders");
    drop(UnicodePstFile::create(&path, &message("First", "<first@example.com>", &[])).unwrap());
    // Outlook's empty file, which the writer starts from, records only Deleted Items.
    for role in ROLES {
        let found = UnicodePstFile::special_folder(&path, role).unwrap();
        if role == UnicodePstSpecialFolder::DeletedItems {
            assert_eq!(found, Some(vec!["Deleted Items".to_string()]));
        } else {
            assert_eq!(found, None, "{role:?}");
        }
    }
    for role in ROLES {
        drop(UnicodePstFile::set_special_folder(&path, role, &[role.default_name()]).unwrap());
    }
    // One in a folder of its own, and recorded again in place of the first.
    drop(
        UnicodePstFile::set_special_folder(
            &path,
            UnicodePstSpecialFolder::JunkEmail,
            &["Inbox", "Spam"],
        )
        .unwrap(),
    );
    for role in ROLES {
        let expected: Vec<String> = match role {
            UnicodePstSpecialFolder::JunkEmail => vec!["Inbox".into(), "Spam".into()],
            role => vec![role.default_name().into()],
        };
        assert_eq!(
            UnicodePstFile::special_folder(&path, role).unwrap(),
            Some(expected),
            "{role:?}"
        );
    }

    let store = open_store(&path);
    let record_key = *store.properties().record_key().unwrap().record_key();
    let entry_id = |names: &[&str]| {
        Vec::try_from(&EntryId::new(
            StoreRecordKey::new(record_key),
            folder(&store, names).0,
        ))
        .unwrap()
    };
    for (id, name) in [
        (0x35E3, "Deleted Items"),
        (0x35E4, "Sent Items"),
        (0x35E2, "Outbox"),
    ] {
        assert!(
            matches!(store.properties().get(id), Some(PropertyValue::Binary(b)) if b.buffer() == entry_id(&[name]))
        );
    }
    let top = store
        .properties()
        .ipm_sub_tree_entry_id()
        .unwrap()
        .node_id();
    let (inbox, _) = folder(&store, &["Inbox"]);
    for node in [NID_ROOT_FOLDER, top, inbox] {
        let f = store
            .open_folder(&store.properties().make_entry_id(node).unwrap())
            .unwrap();
        let p = f.properties();
        assert!(
            matches!(p.get(0x36D7), Some(PropertyValue::Binary(b)) if b.buffer() == entry_id(&["Drafts"]))
        );
        match p.get(0x36D8) {
            Some(PropertyValue::MultipleBinary(values)) => {
                assert_eq!(values.len(), 5);
                assert_eq!(values[4].buffer(), entry_id(&["Inbox", "Spam"]));
            }
            value => panic!("{value:?}"),
        }
    }
    // The file still reads and takes mail.
    drop(store);
    drop(
        UnicodePstFile::append_in_folder(
            &path,
            &["Sent Items"],
            &message("Sent", "<sent@example.com>", &[]),
        )
        .unwrap(),
    );
    let store = open_store(&path);
    assert!(matches!(
        folder(&store, &["Sent Items"]).1.properties().get(0x3602),
        Some(PropertyValue::Integer32(1))
    ));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn outlooks_special_folders_are_kept() {
    let path = outlook_pst("special-outlook-kept");
    let before: Vec<_> = ROLES
        .iter()
        .map(|role| UnicodePstFile::special_folder(&path, *role).unwrap())
        .collect();
    assert_eq!(before[0], Some(vec!["Deleted Items".to_string()]));
    let junk_before = {
        let store = open_store(&path);
        let top = store
            .properties()
            .ipm_sub_tree_entry_id()
            .unwrap()
            .node_id();
        let f = store
            .open_folder(&store.properties().make_entry_id(top).unwrap())
            .unwrap();
        f.properties().get(0x36D8).cloned()
    };
    drop(
        UnicodePstFile::set_special_folder(&path, UnicodePstSpecialFolder::JunkEmail, &["Spam"])
            .unwrap(),
    );
    for (role, before) in ROLES.iter().zip(before) {
        let now = UnicodePstFile::special_folder(&path, *role).unwrap();
        if *role == UnicodePstSpecialFolder::JunkEmail {
            assert_eq!(now, Some(vec!["Spam".to_string()]));
        } else {
            assert_eq!(now, before, "{role:?}");
        }
    }
    // The other values of PidTagAdditionalRenEntryIds stay as they were.
    let store = open_store(&path);
    let top = store
        .properties()
        .ipm_sub_tree_entry_id()
        .unwrap()
        .node_id();
    let f = store
        .open_folder(&store.properties().make_entry_id(top).unwrap())
        .unwrap();
    if let (Some(PropertyValue::MultipleBinary(old)), Some(PropertyValue::MultipleBinary(new))) =
        (junk_before, f.properties().get(0x36D8))
    {
        for (index, value) in old.iter().enumerate().filter(|(index, _)| *index != 4) {
            assert_eq!(new[index].buffer(), value.buffer());
        }
    }
    std::fs::remove_file(path).unwrap();
}

/// An Inbox by another name (a file Outlook made in German) is found by what it records, not by
/// its name: Drafts and Junk Email are recorded on it, and not on a folder named Inbox beside it.
#[test]
fn an_inbox_by_another_name_is_found_by_what_it_records() {
    let path = temp_pst("special-renamed-inbox");
    drop(
        UnicodePstFile::create_in_folder(
            &path,
            &["Posteingang"],
            &message("First", "<first@example.com>", &[]),
        )
        .unwrap(),
    );
    drop(UnicodePstFile::create_folder(&path, &["Inbox"], MAIL_CONTAINER_CLASS).unwrap());
    // Outlook records the Calendar on its Inbox (PidTagIpmAppointmentEntryId).
    let (posteingang, calendar) = {
        let store = open_store(&path);
        let (node, f) = folder(&store, &["Posteingang"]);
        let record_key = *store.properties().record_key().unwrap().record_key();
        let calendar = Vec::try_from(&EntryId::new(StoreRecordKey::new(record_key), node)).unwrap();
        let mut properties = writable_folder_properties(f.as_ref());
        properties.push((
            0x36D0,
            PropertyValue::Binary(BinaryValue::new(calendar.clone())),
        ));
        drop(f);
        drop(store);
        rewrite_properties(&path, vec![(node, properties)]).unwrap();
        (node, calendar)
    };

    for role in [
        UnicodePstSpecialFolder::Drafts,
        UnicodePstSpecialFolder::JunkEmail,
    ] {
        drop(UnicodePstFile::set_special_folder(&path, role, &["Entw\u{fc}rfe"]).unwrap());
        assert_eq!(
            UnicodePstFile::special_folder(&path, role).unwrap(),
            Some(vec!["Entw\u{fc}rfe".to_string()])
        );
    }
    let store = open_store(&path);
    let record_key = *store.properties().record_key().unwrap().record_key();
    let drafts = Vec::try_from(&EntryId::new(
        StoreRecordKey::new(record_key),
        folder(&store, &["Entw\u{fc}rfe"]).0,
    ))
    .unwrap();
    let open = |node| {
        store
            .open_folder(&store.properties().make_entry_id(node).unwrap())
            .unwrap()
    };
    let f = open(posteingang);
    let p = f.properties();
    assert!(matches!(p.get(0x36D7), Some(PropertyValue::Binary(b)) if b.buffer() == drafts));
    assert!(
        matches!(p.get(0x36D8), Some(PropertyValue::MultipleBinary(v)) if v.get(4).is_some_and(|b| b.buffer() == drafts))
    );
    // What it recorded before stays.
    assert!(matches!(p.get(0x36D0), Some(PropertyValue::Binary(b)) if b.buffer() == calendar));
    let inbox = open(folder(&store, &["Inbox"]).0);
    assert!(inbox.properties().get(0x36D7).is_none());
    assert!(inbox.properties().get(0x36D8).is_none());
    std::fs::remove_file(path).unwrap();
}

/// A damaged hierarchy that lists a folder under itself, twice: looking for a folder that isn't
/// there looks through each folder once, where it took one look per path (2^64 of them).
#[test]
fn a_folder_listed_under_itself_is_looked_through_once() {
    let path = temp_pst("special-cyclic");
    drop(
        UnicodePstFile::create_in_folder(
            &path,
            &["A", "B"],
            &message("First", "<first@example.com>", &[]),
        )
        .unwrap(),
    );
    drop(UnicodePstFile::create_folder(&path, &["A", "C"], MAIL_CONTAINER_CLASS).unwrap());
    let (a, b, c) = {
        let store = open_store(&path);
        (
            u32::from(folder(&store, &["A"]).0),
            u32::from(folder(&store, &["A", "B"]).0),
            u32::from(folder(&store, &["A", "C"]).0),
        )
    };
    // A's hierarchy table (its row index and its rows, the blocks that name both B and C) made
    // to name A in their place, the blocks' CRCs made right.
    let mut f = std::fs::read(&path).unwrap();
    let mut patched = 0;
    for end in (64..=f.len()).step_by(64) {
        let size = usize::from(u16::from_le_bytes([f[end - 16], f[end - 15]]));
        let Some(start) = end.checked_sub((size + 16).div_ceil(64) * 64) else {
            continue;
        };
        let stored = u32::from_le_bytes(f[end - 12..end - 8].try_into().unwrap());
        if !(4..=8176).contains(&size)
            || crate::crc::compute_crc(0, &f[start..start + size]) != stored
        {
            continue;
        }
        let block = &mut f[start..start + size];
        let has = |block: &[u8], id: u32| block.windows(4).any(|w| w == id.to_le_bytes());
        if !has(block, b) || !has(block, c) {
            continue;
        }
        for at in 0..size - 3 {
            if block[at..at + 4] == b.to_le_bytes() || block[at..at + 4] == c.to_le_bytes() {
                block[at..at + 4].copy_from_slice(&a.to_le_bytes());
            }
        }
        let crc = crate::crc::compute_crc(0, &f[start..start + size]);
        f[end - 12..end - 8].copy_from_slice(&crc.to_le_bytes());
        patched += 1;
    }
    assert!(patched > 0);
    std::fs::write(&path, f).unwrap();

    let store = open_store(&path);
    let (_, a_folder) = folder(&store, &["A"]);
    let rows: Vec<u32> = a_folder
        .hierarchy_table()
        .unwrap()
        .rows_matrix()
        .map(|row| u32::from(row.id()))
        .collect();
    assert_eq!(rows, [a, a]);
    assert_eq!(
        folder_names(store.as_ref(), NodeId::from(0x7FFF_FFE2)).unwrap(),
        None
    );
    std::fs::remove_file(path).unwrap();
}

/// MailCraft: the Calendar, Contacts, the Journal, Notes and Tasks are made with their own
/// container classes and recorded on the root folder, the IPM subtree and the Inbox; a folder
/// already there keeps its class.
#[test]
fn pim_folders_are_made_with_their_class_and_recorded() {
    const PIM: [(UnicodePstSpecialFolder, u16); 5] = [
        (UnicodePstSpecialFolder::Calendar, 0x36D0),
        (UnicodePstSpecialFolder::Contacts, 0x36D1),
        (UnicodePstSpecialFolder::Journal, 0x36D2),
        (UnicodePstSpecialFolder::Notes, 0x36D3),
        (UnicodePstSpecialFolder::Tasks, 0x36D4),
    ];
    let path = temp_pst("special-pim");
    drop(UnicodePstFile::create(&path, &message("First", "<first@example.com>", &[])).unwrap());
    drop(UnicodePstFile::create_folder(&path, &["Journal"], "IPF.Note").unwrap());
    for (role, _) in PIM {
        assert_eq!(
            UnicodePstFile::special_folder(&path, role).unwrap(),
            None,
            "{role:?}"
        );
        drop(UnicodePstFile::set_special_folder(&path, role, &[role.default_name()]).unwrap());
    }
    for (role, _) in PIM {
        assert_eq!(
            UnicodePstFile::special_folder(&path, role).unwrap(),
            Some(vec![role.default_name().to_string()]),
            "{role:?}"
        );
    }
    let store = open_store(&path);
    let class = |names: &[&str]| text(folder(&store, names).1.properties().get(0x3613));
    assert_eq!(class(&["Calendar"]), "IPF.Appointment");
    assert_eq!(class(&["Contacts"]), "IPF.Contact");
    assert_eq!(class(&["Journal"]), "IPF.Note");
    assert_eq!(class(&["Notes"]), "IPF.StickyNote");
    assert_eq!(class(&["Tasks"]), "IPF.Task");
    let record_key = *store.properties().record_key().unwrap().record_key();
    let top = store
        .properties()
        .ipm_sub_tree_entry_id()
        .unwrap()
        .node_id();
    let (inbox, _) = folder(&store, &["Inbox"]);
    for node in [NID_ROOT_FOLDER, top, inbox] {
        let f = store
            .open_folder(&store.properties().make_entry_id(node).unwrap())
            .unwrap();
        for (role, id) in PIM {
            let entry_id = Vec::try_from(&EntryId::new(
                StoreRecordKey::new(record_key),
                folder(&store, &[role.default_name()]).0,
            ))
            .unwrap();
            assert!(
                matches!(f.properties().get(id), Some(PropertyValue::Binary(b)) if b.buffer() == entry_id),
                "{role:?}"
            );
        }
    }
    // The mail roles are as they were.
    assert_eq!(
        UnicodePstFile::special_folder(&path, UnicodePstSpecialFolder::DeletedItems).unwrap(),
        Some(vec!["Deleted Items".to_string()])
    );
    std::fs::remove_file(path).unwrap();
}

/// MailCraft: a file made with no message has an empty Inbox, and its first item can go in a
/// folder of its own class.
#[test]
fn an_empty_file_takes_a_folder_of_its_own_class_first() {
    let path = temp_pst("empty-file");
    drop(UnicodePstFile::create_empty(&path).unwrap());
    assert!(UnicodePstFile::create_empty(&path).is_err());
    drop(UnicodePstFile::create_folder(&path, &["Contacts"], "IPF.Contact").unwrap());
    let contact = UnicodePstMessage {
        message_class: "IPM.Contact",
        ..message("Ana Ruiz", "", &[])
    };
    drop(UnicodePstFile::append_in_folder(&path, &["Contacts"], &contact).unwrap());
    let store = open_store(&path);
    let (_, inbox) = folder(&store, &["Inbox"]);
    assert_eq!(messages(&store, &inbox).len(), 0);
    let (_, contacts) = folder(&store, &["Contacts"]);
    assert_eq!(text(contacts.properties().get(0x3613)), "IPF.Contact");
    let all = messages(&store, &contacts);
    assert_eq!(all.len(), 1);
    assert_eq!(text(all[0].1.properties().get(0x001A)), "IPM.Contact");
    std::fs::remove_file(path).unwrap();
}
