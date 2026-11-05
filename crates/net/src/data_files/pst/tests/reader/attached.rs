// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Attachments read from damaged and made-up files: messages attached inside messages, files
//! the reader fails on, links, and hidden files by the item's class.

use super::*;

/// What the import makes of a message attached to itself: it finishes in time, the zero-byte
/// file comes in, and the attached message, which holds the message it is attached to (its
/// search key is that message's), is counted and left out rather than read again and again.
#[test]
fn the_import_of_a_message_attached_to_itself_ends() {
    let dir = scratch("attached-import");
    let path = with_an_attached_message(&dir);
    let (done, sources) = import_in_time(&path, &dir);
    assert_eq!((done.messages, done.skipped), (1, 0), "{done:?}");
    assert_eq!(done.files_left_out.into_iter().collect::<Vec<_>>(), [(super::super::FileLeftOut::MessageUnreadable, 1)]);
    assert_eq!(sources.iter().map(|s| tree(s)).collect::<Vec<_>>(), ["Outer[empty.txt]"]);
    let _ = std::fs::remove_dir_all(dir);
}

/// A message of a data file whose subject is `subject`: its property context's block and its
/// subnode block, from the node B-tree's leaf pages (ptype 0x81, cLevel 0) and the subject's
/// UTF-16 in the block.
fn message_blocks(f: &[u8], all: &HashMap<u64, (usize, usize)>, subject: &str) -> (u64, u64) {
    let wide: Vec<u8> = subject.encode_utf16().flat_map(u16::to_le_bytes).collect();
    (0..f.len() - 32)
        .step_by(32)
        .filter(|&o| (u64_at(f, o) & 0x1F) == 0x04 && f.get(o / 512 * 512 + 496) == Some(&0x81) && f[o / 512 * 512 + 491] == 0)
        .map(|o| (u64_at(f, o + 8), u64_at(f, o + 16)))
        .find(|(pc, _)| all.get(pc).is_some_and(|&(start, size)| f[start..start + size].windows(wide.len()).any(|w| w == wide)))
        .unwrap()
}

/// The entries of a subnode block (an SLBLOCK) whose node is of `kind`: node, data block,
/// subnode block, and where the entry starts.
fn entries_of(f: &[u8], (start, size): (usize, usize), kind: u64) -> Vec<(u64, u64, u64, usize)> {
    assert_eq!((f[start], f[start + 1]), (0x02, 0), "an SLBLOCK");
    let count = usize::from(u16_at(f, start + 2));
    (0..count)
        .map(|i| start + 8 + i * 24)
        .filter(|&at| at + 24 <= start + size && (u64_at(f, at) & 0x1F) == kind)
        .map(|at| (u64_at(f, at), u64_at(f, at + 8), u64_at(f, at + 16), at))
        .collect()
}

/// The first attachment of the message whose subnode block is `holder` becomes one holding the
/// message whose blocks are `target` (its property context's, its subnode block): its method
/// becomes 5, its 0x3701 a `PtypObject` naming its own first subnode, which now points at the
/// target's blocks.
fn attach(f: &mut [u8], holder: u64, target: (u64, u64)) {
    let all = blocks(f);
    let (_, pc, own, _) = *entries_of(f, all[&holder], 0x05).iter().min_by_key(|e| e.0).unwrap();
    let attachment = all[&pc];
    let own = all[&own];
    let (object, _, _, at) = entries_of(f, own, 0x01)[0];
    put64(f, at + 8, target.0);
    put64(f, at + 16, target.1);
    seal(f, own);
    let method = record(f, attachment, 0x3705, 0x0003);
    put32(f, method + 4, 5);
    let mime = u32_at(f, record(f, attachment, 0x370E, 0x001F) + 4);
    let item = heap_item(f, attachment, mime);
    put32(f, item, object as u32);
    put32(f, item + 4, 0);
    let value = record(f, attachment, 0x3701, 0x0102);
    f[value + 2..value + 4].copy_from_slice(&0x000D_u16.to_le_bytes());
    put32(f, value + 4, mime);
    seal(f, attachment);
}

/// A data file of three messages, each with an id of its own: "Outer" holds "Middle", which
/// holds "Deep", each beside a file of its own (attachments the writer made as files and these
/// turn into attached messages).
fn with_nested_messages(dir: &Path) -> PathBuf {
    let path = dir.join("nested.pst");
    let outer = [file("a.bin", b"one"), file("empty.txt", b"")];
    drop(Writer::create_with_attachments(&path, &with_id("Outer", "<outer@example.com>"), &outer).unwrap());
    let middle = [file("b.bin", b"two"), file("middle.txt", b"in the middle")];
    drop(Writer::append_with_attachments(&path, &with_id("Middle", "<middle@example.com>"), &middle).unwrap());
    let deep = [file("deep.txt", b"at the bottom")];
    drop(Writer::append_with_attachments(&path, &with_id("Deep", "<deep@example.com>"), &deep).unwrap());
    let mut f = std::fs::read(&path).unwrap();
    let all = blocks(&f);
    let outer = message_blocks(&f, &all, "Outer");
    let middle = message_blocks(&f, &all, "Middle");
    let deep = message_blocks(&f, &all, "Deep");
    attach(&mut f, outer.1, middle);
    attach(&mut f, middle.1, deep);
    std::fs::write(&path, f).unwrap();
    path
}

/// Import the file into a cache of its own on a thread with a time limit: what the import did,
/// and each message's source.
fn import_in_time(path: &Path, dir: &Path) -> (super::super::super::Imported, Vec<Vec<u8>>) {
    use quatrelle_model::MessageKey;
    use quatrelle_store::Store;
    let (path, cache) = (path.to_path_buf(), dir.join("cache"));
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let store = Store::open(cache).unwrap();
        let done = super::super::import(&path, &store, "a1", &mut |_| {}).unwrap();
        let mut sources = Vec::new();
        for f in &done.folders {
            for m in store.load_folder("a1", &f.path).map(|s| s.messages).unwrap_or_default() {
                let key = MessageKey { account: "a1".into(), folder: f.path.clone(), validity: 1, uid: m.key.uid };
                sources.push(store.read_body(&key).unwrap());
            }
        }
        let _ = tx.send((done, sources));
    });
    rx.recv_timeout(Duration::from_secs(20)).expect("the file was imported in time")
}

/// A message's subject and its attachments' names, an attached message's as its own tree:
/// `Outer[Middle[…], empty.txt]`.
fn tree(raw: &[u8]) -> String {
    let p = quatrelle_mime::parse(raw).unwrap();
    let parts: Vec<String> = p
        .attachments
        .iter()
        .map(|a| match a.name.strip_suffix(".eml") {
            Some(_) => tree(&quatrelle_mime::attachment_data(raw, a.index).unwrap()),
            None => a.name.clone(),
        })
        .collect();
    format!("{}[{}]", p.subject, parts.join(", "))
}

/// What the import makes of messages attached inside messages: each comes in as an attached
/// .eml with its own files, the one attached inside it too, and the zero-byte file comes in.
/// An attached message's own files were left out before.
#[test]
fn the_import_keeps_attached_messages_with_their_own_files() {
    let dir = scratch("nested-import");
    let path = with_nested_messages(&dir);
    let (done, sources) = import_in_time(&path, &dir);
    assert_eq!((done.messages, done.skipped), (3, 0), "{done:?}");
    assert!(done.files_left_out.is_empty(), "{:?}", done.files_left_out);
    let mut trees: Vec<String> = sources.iter().map(|s| tree(s)).collect();
    trees.sort();
    assert_eq!(trees, ["Deep[deep.txt]", "Middle[Deep[deep.txt], middle.txt]", "Outer[Middle[Deep[deep.txt], middle.txt], empty.txt]"]);
    let _ = std::fs::remove_dir_all(dir);
}

/// An attachment the reader fails on, and one that only links to a file elsewhere, are counted
/// by reason; the message comes in without them, and isn't counted as skipped.
#[test]
fn an_unreadable_attachment_and_a_link_are_counted() {
    use quatrelle_model::MessageKey;
    use quatrelle_store::Store;
    let dir = scratch("unread-and-link");
    let path = dir.join("left-out.pst");
    let files = [file("letter.bin", b"twelve bytes"), file("share.txt", b"a path")];
    drop(Writer::create_with_attachments(&path, &message("Outer"), &files).unwrap());
    let mut f = std::fs::read(&path).unwrap();
    let all = blocks(&f);
    // The first one's method (0x3705) becomes 0x3706, so the reader finds none; its row in the
    // attachment table still has it.
    let (_, at) = sub_node_entry(&f, &all, ATTACHMENT_NODE);
    let first = all[&u64_at(&f, at + 8)];
    let method = record(&f, first, 0x3705, 0x0003);
    f[method..method + 2].copy_from_slice(&0x3706_u16.to_le_bytes());
    seal(&mut f, first);
    // The second one's method: by reference (2).
    let (_, at) = sub_node_entry(&f, &all, ATTACHMENT_NODE + (1 << 5));
    let second = all[&u64_at(&f, at + 8)];
    let method = record(&f, second, 0x3705, 0x0003);
    put32(&mut f, method + 4, 2);
    seal(&mut f, second);
    std::fs::write(&path, &f).unwrap();

    let store = Store::open(dir.join("cache")).unwrap();
    let done = super::super::import(&path, &store, "a1", &mut |_| {}).unwrap();
    assert_eq!((done.messages, done.skipped), (1, 0), "{done:?}");
    assert_eq!(
        done.files_left_out.into_iter().collect::<Vec<_>>(),
        [(super::super::FileLeftOut::Link, 1), (super::super::FileLeftOut::Unreadable, 1)]
    );
    let (folder, st) = done
        .folders
        .iter()
        .find_map(|f| store.load_folder("a1", &f.path).ok().filter(|s| !s.messages.is_empty()).map(|s| (f.path.clone(), s)))
        .unwrap();
    assert!(!st.messages[0].has_attachments);
    let key = MessageKey { account: "a1".into(), folder, validity: 1, uid: st.messages[0].key.uid };
    let p = quatrelle_mime::parse(&store.read_body(&key).unwrap()).unwrap();
    assert!(p.attachments.is_empty(), "{:?}", p.attachments.iter().map(|a| &a.name).collect::<Vec<_>>());
    let _ = std::fs::remove_dir_all(dir);
}

/// Mark the attachment `nid` hidden (PidTagAttachmentHidden): its rendering position's record
/// (0x370B) becomes a `PtypBoolean` 0x7FFE that is true; the reader keeps records by id, in any
/// order.
fn hide(f: &mut [u8], nid: u64) {
    let all = blocks(f);
    let (_, at) = sub_node_entry(f, &all, nid);
    let attachment = all[&u64_at(f, at + 8)];
    let position = record(f, attachment, 0x370B, 0x0003);
    f[position..position + 4].copy_from_slice(&[0xFE, 0x7F, 0x0B, 0x00]);
    put32(f, position + 4, 1);
    seal(f, attachment);
}

/// Give the message whose subject is `subject` the class that subject names: its class's record
/// (0x001A) points at the subject's text.
fn class_from_subject(f: &mut [u8], subject: &str) {
    let all = blocks(f);
    let pc = all[&message_blocks(f, &all, subject).0];
    let subject = u32_at(f, record(f, pc, 0x0037, 0x001F) + 4);
    let class = record(f, pc, 0x001A, 0x001F);
    put32(f, class + 4, subject);
    seal(f, pc);
}

/// The import reads each item's class and leaves hidden files out by it: a message's hidden
/// attached message comes in; a meeting request's (a recurring series' changed occurrence)
/// doesn't, and its other files still do.
#[test]
fn hidden_files_are_left_out_by_the_items_class() {
    use quatrelle_model::MessageKey;
    use quatrelle_store::Store;
    let names = |path: &Path, cache: &Path| {
        let store = Store::open(cache).unwrap();
        let done = super::super::import(path, &store, "a1", &mut |_| {}).unwrap();
        assert_eq!((done.messages, done.skipped), (2, 0), "{done:?}");
        let mut names = Vec::new();
        for f in &done.folders {
            for m in store.load_folder("a1", &f.path).map(|s| s.messages).unwrap_or_default() {
                let key = MessageKey { account: "a1".into(), folder: f.path.clone(), validity: 1, uid: m.key.uid };
                let p = quatrelle_mime::parse(&store.read_body(&key).unwrap()).unwrap();
                names.extend(p.attachments.iter().map(|a| a.name.clone()));
            }
        }
        names
    };
    let dir = scratch("hidden-by-class");
    for (subject, kept) in [("IPM.Note", true), ("IPM.Schedule.Meeting.Request", false)] {
        let at = dir.join(subject);
        std::fs::create_dir_all(&at).unwrap();
        let path = at.join("hidden.pst");
        let files = [file("letter.bin", b"twelve bytes"), file("empty.txt", b"")];
        drop(Writer::create_with_attachments(&path, &with_id(subject, "<outer@example.com>"), &files).unwrap());
        drop(Writer::append_with_attachments(&path, &with_id("Other", "<other@example.com>"), &[]).unwrap());
        let mut f = std::fs::read(&path).unwrap();
        let all = blocks(&f);
        // The first file becomes the other message, which isn't one holding the message.
        let (holder, other) = (message_blocks(&f, &all, subject).1, message_blocks(&f, &all, "Other"));
        attach(&mut f, holder, other);
        hide(&mut f, ATTACHMENT_NODE);
        class_from_subject(&mut f, subject);
        std::fs::write(&path, &f).unwrap();
        let names = names(&path, &at.join("cache"));
        assert_eq!(names.iter().any(|n| n.ends_with(".eml")), kept, "{subject}: {names:?}");
        assert!(names.iter().any(|n| n == "empty.txt"), "{subject}: {names:?}");
    }
    let _ = std::fs::remove_dir_all(dir);
}
