// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
use super::*;
use outlook_pst::messaging::message::{Message as _, UnicodeMessage};

mod ansi;
mod big;
mod damaged;
mod follow_up;
mod kinds;
mod pim;
mod pinned;
mod round_trip;
mod seeds;
mod special;
mod state;
use quatrelle_model::{EmailAddress, Flags};
use quatrelle_store::FolderState;
use std::path::Path;

/// No follow-up days, no category names.
pub(super) const NONE: &ExportMarks =
    &ExportMarks { due: std::collections::BTreeMap::new(), done: std::collections::BTreeMap::new(), categories: Vec::new() };

#[test]
fn an_account_round_trips_through_a_data_file() {
    let dir = std::env::temp_dir().join(format!("quatrelle-pst-export-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = Store::open(dir.join("cache")).unwrap();
    let mk = |folder: &str, uid: u32, subject: &str, with_file: bool| {
        let d = mime::Draft {
            to: vec![EmailAddress::new("Ana", "ana@example.com")],
            cc: vec![EmailAddress::new("", "ben@example.com")],
            subject: subject.into(),
            body: "Hello there.".into(),
            html: Some("<p>Hello <b>there</b>.</p>".into()),
            attachments: if with_file {
                vec![mime::OutAttachment { name: "notes.txt".into(), mime: "text/plain".into(), data: b"remember".to_vec(), ..Default::default() }]
            } else {
                Vec::new()
            },
            ..Default::default()
        };
        let raw = mime::build(&d, &EmailAddress::new("Riley", "riley@example.com"), 1_704_067_200, &format!("m{uid}@example.com")).unwrap();
        let key = MessageKey { account: "a1".into(), folder: folder.into(), validity: 1, uid };
        store.write_body(&key, &raw).unwrap();
        mime::summary_from_full(&raw, key, Flags::default(), 10, 0)
    };
    let folder = |path: &str, role| quatrelle_model::Folder {
        account: "a1".into(),
        path: path.into(),
        delimiter: Some('/'),
        role,
        selectable: true,
        ..Default::default()
    };
    let inbox = FolderState {
        validity: 1,
        uid_next: 3,
        messages: vec![mk("INBOX", 1, "Plan — café", true), mk("INBOX", 2, "Second", false)],
        ..Default::default()
    };
    let mut projects = FolderState { validity: 1, uid_next: 2, messages: vec![mk("INBOX/Projects", 1, "Nested", false)], ..Default::default() };
    // Known by its header only: skipped.
    let mut header_only = projects.messages[0].clone();
    header_only.key.uid = 7;
    projects.messages.push(header_only);
    store.save_folder("a1", "INBOX", &inbox).unwrap();
    store.save_folder("a1", "INBOX/Projects", &projects).unwrap();
    store.save_folders("a1", &[folder("INBOX", FolderRole::Inbox), folder("INBOX/Projects", FolderRole::Other)]).unwrap();

    let mut pim = quatrelle_model::pim::Pim::default();
    pim.events.push(quatrelle_model::pim::StoredEvent {
        event: quatrelle_model::ical::CalEvent {
            uid: "e1".into(),
            summary: "Screening".into(),
            start: 1_792_000_000,
            end: 1_792_003_600,
            ..Default::default()
        },
        ..Default::default()
    });
    pim.cards.push(quatrelle_model::pim::StoredCard {
        card: quatrelle_model::vcard::Card { uid: "c1".into(), name: "Ana Lima".into(), ..Default::default() },
        ..Default::default()
    });
    store.save_account_doc("a1", quatrelle_model::pim::PIM_DOC, &pim).unwrap();
    let file = dir.join("Out.pst");
    let mut ticks = 0;
    let done = export(&store, "a1", "Riley's mail", &file, NONE, &mut |_| ticks += 1).unwrap();
    assert_eq!(done, Exported { messages: 3, skipped: 1, already: 0, events: 1, tasks: 0, notes: 0, contacts: 1 });
    // The calendar beside it; the contact inside it, no address book beside it.
    let ics = std::fs::read_to_string(dir.join("Out Calendar.ics")).unwrap();
    assert!(ics.contains("SUMMARY:Screening") && ics.contains("BEGIN:VCALENDAR"), "{ics}");
    assert!(!dir.join("Out Contacts.vcf").exists());
    assert!(ticks > 0);

    // Read back with the importer.
    let back = Store::open(dir.join("back")).unwrap();
    let imported = crate::pst::import(&file, &back, "b1", &mut |_| {}).unwrap();
    assert_eq!((imported.messages, imported.contacts), (3, 1), "{imported:?}");
    let all = back.load_folders("b1").unwrap();
    let inbox = back.load_folder("b1", "INBOX").unwrap();
    assert_eq!(inbox.messages.len(), 2, "{all:?}");
    let plan = inbox.messages.iter().find(|m| m.subject == "Plan — café").unwrap();
    let raw = back.read_body(&MessageKey { account: "b1".into(), folder: "INBOX".into(), validity: 1, uid: plan.key.uid }).unwrap();
    let p = mime::parse(&raw).unwrap();
    assert_eq!(p.from.email, "riley@example.com");
    assert_eq!(p.cc[0].email, "ben@example.com");
    assert_eq!(p.message_id, "m1@example.com");
    assert_eq!(p.date, 1_704_067_200);
    assert!(p.attachments.iter().any(|a| a.name == "notes.txt"));
    assert!(all.iter().any(|f| f.path.ends_with("Projects") && f.total == 1), "{all:?}");

    // A file that can't be replaced: the error says where the mail is, and it is there.
    let busy = dir.join("Busy.pst");
    std::fs::create_dir_all(busy.join("inside")).unwrap();
    let e = export(&store, "a1", "Riley's mail", &busy, NONE, &mut |_| {}).unwrap_err();
    assert!(e.contains("Busy.pst.part"), "{e}");
    let back = Store::open(dir.join("back3")).unwrap();
    assert_eq!(crate::pst::import(&dir.join("Busy.pst.part"), &back, "b3", &mut |_| {}).unwrap().messages, 3);

    // Nothing to write: an error, and the old file stays.
    let empty = dir.join("Empty.pst");
    assert!(export(&store, "nobody", "x", &empty, NONE, &mut |_| {}).is_err());
    assert!(!empty.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The subjects of `file`'s mail as the import reads them back, in order.
pub(super) fn subjects_back(file: &Path, dir: &Path, label: &str) -> Vec<String> {
    let back = Store::open(dir.join(label)).unwrap();
    crate::pst::import(file, &back, "b", &mut |_| {}).unwrap();
    let mut out: Vec<String> = Vec::new();
    for f in back.load_folders("b").unwrap() {
        out.extend(back.load_folder("b", &f.path).unwrap().messages.iter().map(|m| m.subject.clone()));
    }
    out.sort();
    out
}

/// An account with `messages` (uid, subject) in its Inbox, each with Message-ID `m<uid>@example.com`.
pub(super) fn inbox_account(dir: &Path, messages: &[(u32, &str)]) -> Store {
    let store = Store::open(dir.join("cache")).unwrap();
    let mk = |uid: u32, subject: &str| {
        let d = mime::Draft { subject: subject.into(), body: "Hello there.".into(), ..Default::default() };
        let raw = mime::build(&d, &EmailAddress::new("Riley", "riley@example.com"), 1_704_067_200, &format!("m{uid}@example.com")).unwrap();
        let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
        store.write_body(&key, &raw).unwrap();
        mime::summary_from_full(&raw, key, Flags::default(), 10, 0)
    };
    let inbox = FolderState { validity: 1, uid_next: 100, messages: messages.iter().map(|(uid, s)| mk(*uid, s)).collect(), ..Default::default() };
    store.save_folder("a1", "INBOX", &inbox).unwrap();
    let folder =
        quatrelle_model::Folder { account: "a1".into(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
    store.save_folders("a1", &[folder]).unwrap();
    store
}

/// A data file another program made, with mail of its own and one of the account's messages
/// (by Message-ID).
pub(super) fn theirs(file: &Path) {
    let theirs = |subject, message_id| UnicodePstMessage {
        subject,
        sender_name: "Ana",
        sender_email: "ana@example.com",
        recipients: &[],
        body: "Kept as it was.",
        html_body: None,
        message_id,
        delivery_time: 133_485_408_000_000_000,
        message_class: "IPM.Note",
        properties: &[],
    };
    drop(UnicodePstFile::create_in_folder(file, &["Inbox"], &theirs("Theirs", "<theirs@example.com>")).unwrap());
    drop(UnicodePstFile::append_in_folder(file, &["Inbox"], &theirs("Both", "<m1@example.com>")).unwrap());
}

/// Exporting into a data file already there adds to it, as Outlook's export does: what it had
/// stays, a message it already has (by Message-ID) isn't written twice, and a second export adds
/// nothing. Something there that isn't a data file is left alone, and said so.
#[test]
fn a_data_file_already_there_is_added_to() {
    let dir = std::env::temp_dir().join(format!("quatrelle-pst-export-added-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = inbox_account(&dir, &[(1, "Both"), (2, "Ours")]);
    let file = dir.join("Outlook.pst");
    theirs(&file);
    let done = export(&store, "a1", "Riley's mail", &file, NONE, &mut |_| {}).unwrap();
    assert_eq!((done.messages, done.already), (1, 1));
    assert_eq!(subjects_back(&file, &dir, "back1"), ["Both", "Ours", "Theirs"]);
    let again = export(&store, "a1", "Riley's mail", &file, NONE, &mut |_| {}).unwrap();
    assert_eq!((again.messages, again.already), (0, 2));
    assert_eq!(subjects_back(&file, &dir, "back2"), ["Both", "Ours", "Theirs"]);
    assert!(!dir.join("Outlook.pst.part").exists());

    let other = dir.join("Notes.pst");
    std::fs::write(&other, b"an older file").unwrap();
    assert!(export(&store, "a1", "Riley's mail", &other, NONE, &mut |_| {}).is_err_and(|e| e.contains("isn't an Outlook data file")));
    assert_eq!(std::fs::read(&other).unwrap(), b"an older file");
    let _ = std::fs::remove_dir_all(&dir);
}

/// MS-PST 5.1's permutation (`mpbbCrypt`'s first 256 bytes), which encodes a data block's bytes.
pub(super) const PERMUTE: [u8; 256] = [
    0x41, 0x36, 0x13, 0x62, 0xA8, 0x21, 0x6E, 0xBB, 0xF4, 0x16, 0xCC, 0x04, 0x7F, 0x64, 0xE8, 0x5D, 0x1E, 0xF2, 0xCB, 0x2A, 0x74, 0xC5, 0x5E, 0x35,
    0xD2, 0x95, 0x47, 0x9E, 0x96, 0x2D, 0x9A, 0x88, 0x4C, 0x7D, 0x84, 0x3F, 0xDB, 0xAC, 0x31, 0xB6, 0x48, 0x5F, 0xF6, 0xC4, 0xD8, 0x39, 0x8B, 0xE7,
    0x23, 0x3B, 0x38, 0x8E, 0xC8, 0xC1, 0xDF, 0x25, 0xB1, 0x20, 0xA5, 0x46, 0x60, 0x4E, 0x9C, 0xFB, 0xAA, 0xD3, 0x56, 0x51, 0x45, 0x7C, 0x55, 0x00,
    0x07, 0xC9, 0x2B, 0x9D, 0x85, 0x9B, 0x09, 0xA0, 0x8F, 0xAD, 0xB3, 0x0F, 0x63, 0xAB, 0x89, 0x4B, 0xD7, 0xA7, 0x15, 0x5A, 0x71, 0x66, 0x42, 0xBF,
    0x26, 0x4A, 0x6B, 0x98, 0xFA, 0xEA, 0x77, 0x53, 0xB2, 0x70, 0x05, 0x2C, 0xFD, 0x59, 0x3A, 0x86, 0x7E, 0xCE, 0x06, 0xEB, 0x82, 0x78, 0x57, 0xC7,
    0x8D, 0x43, 0xAF, 0xB4, 0x1C, 0xD4, 0x5B, 0xCD, 0xE2, 0xE9, 0x27, 0x4F, 0xC3, 0x08, 0x72, 0x80, 0xCF, 0xB0, 0xEF, 0xF5, 0x28, 0x6D, 0xBE, 0x30,
    0x4D, 0x34, 0x92, 0xD5, 0x0E, 0x3C, 0x22, 0x32, 0xE5, 0xE4, 0xF9, 0x9F, 0xC2, 0xD1, 0x0A, 0x81, 0x12, 0xE1, 0xEE, 0x91, 0x83, 0x76, 0xE3, 0x97,
    0xE6, 0x61, 0x8A, 0x17, 0x79, 0xA4, 0xB7, 0xDC, 0x90, 0x7A, 0x5C, 0x8C, 0x02, 0xA6, 0xCA, 0x69, 0xDE, 0x50, 0x1A, 0x11, 0x93, 0xB9, 0x52, 0x87,
    0x58, 0xFC, 0xED, 0x1D, 0x37, 0x49, 0x1B, 0x6A, 0xE0, 0x29, 0x33, 0x99, 0xBD, 0x6C, 0xD9, 0x94, 0xF3, 0x40, 0x54, 0x6F, 0xF0, 0xC6, 0x73, 0xB8,
    0xD6, 0x3E, 0x65, 0x18, 0x44, 0x1F, 0xDD, 0x67, 0x10, 0xF1, 0x0C, 0x19, 0xEC, 0xAE, 0x03, 0xA1, 0x14, 0x7B, 0xA9, 0x0B, 0xFF, 0xF8, 0xA3, 0xC0,
    0xA2, 0x01, 0xF7, 0x2E, 0xBC, 0x24, 0x68, 0x75, 0x0D, 0xFE, 0xBA, 0x2F, 0xB5, 0xD0, 0xDA, 0x3D,
];

/// MS-PST's CRC: CRC-32's table, started at 0 and not inverted at the end.
pub(super) fn crc(data: &[u8]) -> u32 {
    let mut c = 0u32;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 == 1 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
        }
    }
    c
}

/// Encode a data file the writer made the way Outlook encodes its own by default: every data
/// block's bytes permuted (internal blocks never are), found from the 16-byte trailers that end
/// blocks on 64-byte boundaries, with their CRCs, and the header's `bCryptMethod` (0x201) set
/// to permute, with the CRC over the header (0x20C, of the 516 bytes from 8).
pub(super) fn permute_encode(path: &Path) {
    let mut f = std::fs::read(path).unwrap();
    let u32_at = |f: &[u8], at: usize| u32::from_le_bytes(f[at..at + 4].try_into().unwrap());
    for end in (64..=f.len()).step_by(64) {
        let size = usize::from(u16::from_le_bytes([f[end - 16], f[end - 15]]));
        let bid = u64::from_le_bytes(f[end - 8..end].try_into().unwrap());
        let Some(start) = end.checked_sub((size + 16).div_ceil(64) * 64) else { continue };
        if size == 0 || size > 8176 || bid & 2 != 0 || crc(&f[start..start + size]) != u32_at(&f, end - 12) {
            continue;
        }
        for b in &mut f[start..start + size] {
            *b = PERMUTE[usize::from(*b)];
        }
        let c = crc(&f[start..start + size]);
        f[end - 12..end - 8].copy_from_slice(&c.to_le_bytes());
    }
    f[0x201] = 1;
    let c = crc(&f[8..8 + 516]);
    f[0x20C..0x210].copy_from_slice(&c.to_le_bytes());
    std::fs::write(path, f).unwrap();
}

/// A data file Outlook made, as Outlook encodes them, is added to (the writer from MailCraft's
/// fork): what it had stays, a message it already has isn't written twice, it stays encoded,
/// and the import reads it all back.
#[test]
fn an_export_adds_to_an_encoded_data_file() {
    let dir = std::env::temp_dir().join(format!("quatrelle-pst-export-encoded-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("Outlook.pst");
    theirs(&file);
    permute_encode(&file);
    let store = inbox_account(&dir, &[(1, "Both"), (2, "Ours")]);

    let done = export(&store, "a1", "Riley's mail", &file, NONE, &mut |_| {}).unwrap();
    assert_eq!((done.messages, done.already), (1, 1));
    assert_eq!(std::fs::read(&file).unwrap()[0x201], 1, "still permute-encoded");
    let back = Store::open(dir.join("back")).unwrap();
    assert_eq!(crate::pst::import(&file, &back, "b1", &mut |_| {}).unwrap().messages, 3);
    let mut subjects: Vec<String> = back.load_folder("b1", "INBOX").unwrap().messages.iter().map(|m| m.subject.clone()).collect();
    subjects.sort();
    assert_eq!(subjects, ["Both", "Ours", "Theirs"]);
    let _ = std::fs::remove_dir_all(&dir);
}
