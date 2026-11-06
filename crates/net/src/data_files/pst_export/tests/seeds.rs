// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The `pst_append` fuzz target's seeds (`fuzz/seeds/pst_append`): made here by the writer
//! (`make_the_pst_append_seeds`, ignored: it writes into the checkout), and read as the target
//! reads them, without cargo-fuzz: added to, then one message taken out, none of it panicking
//! and no mail lost.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

use outlook_pst::messaging::store::{AnsiStore, UnicodeStore};
use outlook_pst::{
    ADD_MAX_VALUE_SIZE, AnsiPstFile, UnicodePstAttachmentContent, UnicodePstAttachmentSource, UnicodePstEmbeddedMessage,
    UnicodePstSpecialFolder as Special,
};

use super::*;

fn seeds_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/seeds/pst_append")
}

/// The target's message, with what an export gives one.
fn message(subject: &str) -> UnicodePstMessage<'_> {
    UnicodePstMessage {
        subject,
        sender_name: "Riley Moreno",
        sender_email: "riley@example.com",
        recipients: &[],
        body: "Hello there.",
        html_body: Some("<p>Hello <b>there</b>.</p>"),
        message_id: "<m1@example.com>",
        delivery_time: 133_485_408_000_000_000,
        message_class: "IPM.Note",
        properties: &[],
    }
}

fn notes() -> UnicodePstAttachment<'static> {
    UnicodePstAttachment { filename: "notes.txt", mime_type: "text/plain", content_id: None, data: b"remember", ..Default::default() }
}

/// The good file the target damages: Inbox with one message and its file, Projects beside it.
fn good_file(path: &Path) {
    let _ = std::fs::remove_file(path);
    drop(UnicodePstFile::create_in_folder_with_attachments(path, &["Inbox"], &message("Plan"), &[notes()]).unwrap());
    drop(UnicodePstFile::append_in_folder(path, &["Projects"], &message("Nested")).unwrap());
}

/// Write the seeds: five-byte patches to the good file (`good-file`), that file encoded as
/// Outlook encodes its own (`encoded`), and an ANSI file with the same mail (`ansi`).
#[test]
#[ignore = "writes the fuzz target's seeds into the checkout"]
fn make_the_pst_append_seeds() {
    let dir = seeds_dir();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("good-file"), [0, 0, 0, 0, b'!']).unwrap();
    let scratch = std::env::temp_dir().join(format!("quatrelle-pst-append-seeds-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let file = scratch.join("encoded.pst");
    good_file(&file);
    permute_encode(&file);
    std::fs::copy(&file, dir.join("encoded")).unwrap();
    let ansi = scratch.join("ansi.pst");
    let _ = std::fs::remove_file(&ansi);
    drop(AnsiPstFile::create_empty(&ansi).unwrap());
    let batch = [UnicodePstBatchMessage { message: message("Plan"), attachments: &[notes()] }];
    drop(AnsiPstFile::append_many_in_folder_with_attachments(&ansi, &["Inbox"], &batch).unwrap());
    let batch = [UnicodePstBatchMessage { message: message("Nested"), attachments: &[] }];
    drop(AnsiPstFile::append_many_in_folder_with_attachments(&ansi, &["Projects"], &batch).unwrap());
    std::fs::copy(&ansi, dir.join("ansi")).unwrap();
    let _ = std::fs::remove_dir_all(&scratch);
}

/// A file of 20,000 bytes, read as it is written.
struct Source;

impl UnicodePstAttachmentSource for Source {
    fn size(&self) -> u64 {
        20_000
    }

    fn open(&self) -> io::Result<Box<dyn io::Read + '_>> {
        Ok(Box::new(io::Read::take(io::repeat(b'x'), 20_000)))
    }
}

/// Every message the file's folders list that reads, by node, with its properties as text.
fn messages(path: &Path, ansi: bool) -> BTreeMap<u32, String> {
    let mut out = BTreeMap::new();
    let store: Option<Rc<dyn PstStore>> = if ansi {
        AnsiPstFile::open(path).ok().and_then(|pst| {
            pst.set_max_value_size(ADD_MAX_VALUE_SIZE);
            AnsiStore::read(Rc::new(pst)).ok().map(|s| s as Rc<dyn PstStore>)
        })
    } else {
        UnicodePstFile::open(path).ok().and_then(|pst| {
            pst.set_max_value_size(ADD_MAX_VALUE_SIZE);
            UnicodeStore::read(Rc::new(pst)).ok().map(|s| s as Rc<dyn PstStore>)
        })
    };
    let Some(store) = store else { return out };
    let Ok(top) = store.properties().ipm_sub_tree_entry_id() else { return out };
    let mut stack = vec![(top, 0)];
    let mut seen = HashSet::new();
    while let Some((id, depth)) = stack.pop() {
        if depth > 32 || seen.len() > 1_000 || !seen.insert(u32::from(id.node_id())) {
            continue;
        }
        let Ok(folder) = store.open_folder(&id) else { continue };
        for row in folder.contents_table().iter().flat_map(|table| table.rows_matrix()) {
            let node = u32::from(row.id());
            let Ok(entry) = store.properties().make_entry_id(NodeId::from(node)) else { continue };
            let Ok(message) = store.open_message(&entry, None) else { continue };
            let mut properties: Vec<_> = message.properties().iter().collect();
            properties.sort_by_key(|(id, _)| **id);
            out.insert(node, format!("{properties:?}"));
        }
        for row in folder.hierarchy_table().iter().flat_map(|table| table.rows_matrix()) {
            if let Ok(sub) = store.properties().make_entry_id(NodeId::from(u32::from(row.id()))) {
                stack.push((sub, depth + 1));
            }
        }
    }
    out
}

/// Each seed, as the target reads it: added to without losing mail, then one message taken out.
#[test]
fn the_pst_append_seeds_take_mail_and_lose_none() {
    let scratch = std::env::temp_dir().join(format!("quatrelle-pst-append-replay-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    let good = scratch.join("good.pst");
    good_file(&good);
    let good = std::fs::read(&good).unwrap();
    let mut seen = Vec::new();
    for entry in std::fs::read_dir(seeds_dir()).unwrap() {
        let seed = entry.unwrap().path();
        let data = std::fs::read(&seed).unwrap();
        let file = if data.starts_with(b"!BDN") {
            data
        } else {
            let mut file = good.clone();
            for p in data.chunks_exact(5) {
                let at = u32::from_le_bytes([p[0], p[1], p[2], p[3]]) as usize % file.len();
                file[at] = p[4];
            }
            file
        };
        let ansi = matches!(file.get(10..12), Some(&[14 | 15, 0]));
        let path = scratch.join("existing.pst");
        std::fs::write(&path, &file).unwrap();
        let attached = UnicodePstAttachment {
            filename: "Forwarded",
            content: UnicodePstAttachmentContent::Message(Box::new(UnicodePstEmbeddedMessage {
                message: message("Forwarded"),
                attachments: vec![notes()],
            })),
            ..Default::default()
        };
        let read = UnicodePstAttachment { filename: "big.bin", content: UnicodePstAttachmentContent::Source(&Source), ..Default::default() };
        let attachments = [notes(), attached, read];
        let batch = [UnicodePstBatchMessage { message: message("Added"), attachments: &attachments }];
        for role in [Special::DeletedItems, Special::SentItems, Special::Outbox, Special::Drafts, Special::JunkEmail] {
            let _ = if ansi { AnsiPstFile::special_folder(&path, role) } else { UnicodePstFile::special_folder(&path, role) };
        }
        let before = messages(&path, ansi);
        let added = if ansi {
            AnsiPstFile::append_many_in_folder_with_attachments(&path, &["Inbox"], &batch).is_ok()
        } else {
            UnicodePstFile::append_many_in_folder_with_attachments(&path, &["Inbox"], &batch).is_ok()
        };
        assert!(added, "{} wasn't added to", seed.display());
        let after = messages(&path, ansi);
        assert_eq!(before.len(), 2, "{}", seed.display());
        assert!(before.iter().all(|(node, was)| after.get(node) == Some(was)), "{}", seed.display());
        assert_eq!(after.len(), 3, "{}", seed.display());
        let gone = before.keys().next().copied().unwrap();
        let ids = [NodeId::from(gone)];
        if ansi { AnsiPstFile::delete_messages(&path, &ids).map(drop) } else { UnicodePstFile::delete_messages(&path, &ids).map(drop) }.unwrap();
        let left = messages(&path, ansi);
        assert!(!left.contains_key(&gone) && after.iter().all(|(node, was)| *node == gone || left.get(node) == Some(was)), "{}", seed.display());
        seen.push((seed.file_name().unwrap().to_string_lossy().into_owned(), ansi));
    }
    seen.sort();
    assert_eq!(seen, [("ansi".to_string(), true), ("encoded".to_string(), false), ("good-file".to_string(), false)]);
    let _ = std::fs::remove_dir_all(&scratch);
}
