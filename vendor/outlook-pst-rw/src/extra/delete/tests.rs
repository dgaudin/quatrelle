use super::*;
use crate::extra::tests::{folder, message, temp_pst};

/// A batch of `subjects`, those marked unread going in unread.
fn batch<'a>(
    subjects: &'a [&'a str],
    unread: &'a [UnicodePstProperty<'a>],
    read: &'a [UnicodePstProperty<'a>],
) -> Vec<UnicodePstBatchMessage<'a>> {
    subjects
        .iter()
        .map(|subject| UnicodePstBatchMessage {
            message: message(
                subject,
                "<delete@example.com>",
                if subject.starts_with("unread") {
                    unread
                } else {
                    read
                },
            ),
            attachments: &[],
        })
        .collect()
}

fn unread_flags() -> [UnicodePstProperty<'static>; 1] {
    [UnicodePstProperty::tagged(
        MESSAGE_FLAGS,
        UnicodePstValue::Integer32(0),
    )]
}

/// The folder at `path`'s messages, by subject, with their node IDs, in its contents table's
/// order; and its content and unread counts, in its property context and in its row of its
/// parent's hierarchy table.
struct Seen {
    messages: Vec<(NodeId, String)>,
    counts: (i32, i32),
    row_counts: (i32, i32),
}

fn integer(value: Option<PropertyValue>) -> i32 {
    match value {
        Some(PropertyValue::Integer32(value)) => value,
        value => panic!("not a 32-bit integer: {value:?}"),
    }
}

fn seen(path: &Path, folder_path: &[&str]) -> Seen {
    let store = AddStore::open(path).unwrap();
    let mut parent = store
        .properties()
        .ipm_sub_tree_entry_id()
        .unwrap()
        .node_id();
    let mut node = parent;
    for name in folder_path {
        parent = node;
        node = find_child_folder(&*store, node, name).unwrap().unwrap();
    }
    let opened = store
        .open_folder(&store.properties().make_entry_id(node).unwrap())
        .unwrap();
    let messages = opened
        .contents_table()
        .map(|table| {
            table
                .rows_matrix()
                .map(|row| {
                    let id = NodeId::from(u32::from(row.id()));
                    let entry = store.properties().make_entry_id(id).unwrap();
                    let message = store.open_message(&entry, None).unwrap();
                    let subject = match message.properties().get(0x0037) {
                        Some(PropertyValue::Unicode(value)) => value.to_string(),
                        Some(PropertyValue::String8(value)) => value.to_string(),
                        value => panic!("no subject: {value:?}"),
                    };
                    (
                        id,
                        subject.trim_start_matches(['\u{1}', '\u{2}']).to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let props = opened.properties();
    let counts = (
        integer(props.get(CONTENT_COUNT).cloned()),
        integer(props.get(CONTENT_UNREAD).cloned()),
    );
    let parent = store
        .open_folder(&store.properties().make_entry_id(parent).unwrap())
        .unwrap();
    let hierarchy = parent.hierarchy_table().unwrap();
    let row_counts = (
        integer(crate::extra::tests::cell(
            hierarchy.as_ref(),
            u32::from(node),
            CONTENT_COUNT,
        )),
        integer(crate::extra::tests::cell(
            hierarchy.as_ref(),
            u32::from(node),
            CONTENT_UNREAD,
        )),
    );
    Seen {
        messages,
        counts,
        row_counts,
    }
}

fn subjects(seen: &Seen) -> Vec<&str> {
    seen.messages
        .iter()
        .map(|(_, subject)| subject.as_str())
        .collect()
}

/// The file at `path` as MS-PST keeps one: its end where the header says; no two blocks or B-tree
/// pages sharing space; every block a node names in the block B-tree, and every block there
/// named; and its allocation map trusted (it reads, and marks no live block free).
fn check_file(path: &Path) {
    let length = std::fs::metadata(path).unwrap().len();
    let mut file = File::open(path).unwrap();
    let header = FileHeader::read(&mut file).unwrap();
    let format = header.format();
    assert_eq!(header.file_eof(), length);
    let bbt = read_block_btree(&mut file, format, header.block_btree()).unwrap();
    let nbt = read_node_btree(&mut file, format, header.node_btree()).unwrap();
    let trailer = u64::from(format.block_trailer_size());
    let mut taken = Vec::new();
    for entry in bbt.entries() {
        let start = entry.block().index().index();
        taken.push((
            start,
            start + (u64::from(entry.size()) + trailer).div_ceil(64) * 64,
        ));
    }
    let pages = bbt
        .leaves
        .iter()
        .map(|(page, _)| *page)
        .chain(nbt.leaves.iter().map(|(page, _)| *page))
        .chain(bbt.inner.iter().flatten().map(|(page, _)| *page))
        .chain(nbt.inner.iter().flatten().map(|(page, _)| *page));
    for page in pages {
        let start = page.index().index();
        taken.push((start, start + PAGE_SIZE as u64));
    }
    taken.sort();
    for pair in taken.windows(2) {
        assert!(
            pair[0].1 <= pair[1].0,
            "{:X?} overlaps {:X?}",
            pair[0],
            pair[1]
        );
    }
    assert!(taken.last().is_some_and(|last| last.1 <= length));
    // The counts the commit left are those a walk from every node gives (none at zero).
    let mut walked = bbt.entries();
    compact_unicode_block_tree(&mut file, format, &mut walked, &nbt.entries()).unwrap();
    let counts = |entries: &[UnicodeBlockBTreeEntry]| {
        let mut counts: Vec<_> = entries.iter().map(|e| (e.key(), e.ref_count())).collect();
        counts.sort();
        counts
    };
    assert_eq!(counts(&bbt.entries()), counts(&walked));
    assert_eq!(header.amap_status(), AmapStatus::Valid2);
    let space = FileSpace::read_map(&mut file, length, format).unwrap();
    assert!(!space.overlaps_live(&bbt, &nbt));
}

/// The bytes the file's allocation map marks free.
fn free_bytes(path: &Path) -> u64 {
    let length = std::fs::metadata(path).unwrap().len();
    let mut file = File::open(path).unwrap();
    let format = FileHeader::read(&mut file).unwrap().format();
    let space = FileSpace::read_map(&mut file, length, format).unwrap();
    space.free.iter().map(|(start, end)| end - start).sum()
}

fn ids(seen: &Seen, wanted: &[&str]) -> Vec<NodeId> {
    seen.messages
        .iter()
        .filter(|(_, subject)| wanted.contains(&subject.as_str()))
        .map(|(id, _)| *id)
        .collect()
}

#[test]
fn messages_deleted_are_gone_and_their_folder_counts_again() {
    let path = temp_pst("delete");
    drop(UnicodePstFile::create_empty(&path).unwrap());
    let (unread, read) = (unread_flags(), []);
    let subjects_in = ["one", "unread two", "three", "unread four", "five"];
    drop(
        UnicodePstFile::append_many_in_folder_with_attachments(
            &path,
            &["Notes"],
            &batch(&subjects_in, &unread, &read),
        )
        .unwrap(),
    );
    let before = seen(&path, &["Notes"]);
    assert_eq!(before.counts, (5, 2));
    assert_eq!(before.row_counts, (5, 2));
    let free = free_bytes(&path);

    let gone = ids(&before, &["unread two", "three"]);
    drop(UnicodePstFile::delete_messages(&path, &gone).unwrap());
    check_file(&path);
    let after = seen(&path, &["Notes"]);
    assert_eq!(subjects(&after), ["one", "unread four", "five"]);
    assert_eq!(after.counts, (3, 1));
    assert_eq!(after.row_counts, (3, 1));
    // The messages' nodes are gone, and the space their blocks held is free.
    let store = AddStore::open(&path).unwrap();
    for id in &gone {
        let entry = store.properties().make_entry_id(*id).unwrap();
        assert!(store.open_message(&entry, None).is_err());
    }
    assert!(free_bytes(&path) > free);
    // A deleted message is noted for Outlook's search folders.
    let updates = store.search_update_queue().unwrap();
    let deleted = updates
        .updates()
        .iter()
        .filter(|update| matches!(update.data(), Some(SearchUpdateData::MessageDeleted { .. })))
        .count();
    assert_eq!(deleted, 2);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn messages_are_deleted_one_by_one_until_none_is_left_and_the_folder_takes_more() {
    let path = temp_pst("delete-all");
    drop(UnicodePstFile::create_empty(&path).unwrap());
    let (unread, read) = (unread_flags(), []);
    drop(
        UnicodePstFile::append_many_in_folder_with_attachments(
            &path,
            &["Tasks"],
            &batch(&["unread a", "b", "c"], &unread, &read),
        )
        .unwrap(),
    );
    for left in [2, 1, 0] {
        let before = seen(&path, &["Tasks"]);
        let first = before.messages[0].0;
        drop(UnicodePstFile::delete_messages(&path, &[first]).unwrap());
        check_file(&path);
        let after = seen(&path, &["Tasks"]);
        assert_eq!(after.messages.len(), left);
        assert_eq!(after.counts.0, left as i32);
        assert_eq!(after.counts.1, 0);
    }
    drop(
        UnicodePstFile::append_many_in_folder_with_attachments(
            &path,
            &["Tasks"],
            &batch(&["d", "unread e"], &unread, &read),
        )
        .unwrap(),
    );
    check_file(&path);
    let after = seen(&path, &["Tasks"]);
    assert_eq!(subjects(&after), ["d", "unread e"]);
    assert_eq!(after.counts, (2, 1));
    assert_eq!(after.row_counts, (2, 1));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn space_a_deleted_message_held_is_taken_again() {
    let path = temp_pst("delete-reuse");
    drop(UnicodePstFile::create_empty(&path).unwrap());
    let body = "a body of many blocks ".repeat(20_000);
    let file = vec![7u8; 300_000];
    let attachments = [UnicodePstAttachment {
        filename: "big.bin",
        mime_type: "application/octet-stream",
        data: &file,
        ..Default::default()
    }];
    let big = |subject| UnicodePstBatchMessage {
        message: UnicodePstMessage {
            body: &body,
            ..message(subject, "<big@example.com>", &[])
        },
        attachments: &attachments,
    };
    drop(
        UnicodePstFile::append_many_in_folder_with_attachments(
            &path,
            &["Calendar"],
            &[big("first")],
        )
        .unwrap(),
    );
    drop(
        UnicodePstFile::append_many_in_folder_with_attachments(
            &path,
            &["Calendar"],
            &[big("kept")],
        )
        .unwrap(),
    );
    let first = ids(&seen(&path, &["Calendar"]), &["first"]);
    drop(UnicodePstFile::delete_messages(&path, &first).unwrap());
    check_file(&path);
    let length = std::fs::metadata(&path).unwrap().len();
    assert!(free_bytes(&path) > 600_000);

    drop(
        UnicodePstFile::append_many_in_folder_with_attachments(
            &path,
            &["Calendar"],
            &[big("again")],
        )
        .unwrap(),
    );
    check_file(&path);
    // The new message went where the old one was, not past the end.
    let grown = std::fs::metadata(&path)
        .unwrap()
        .len()
        .saturating_sub(length);
    assert!(grown < 200_000, "grew {grown} bytes");
    let after = seen(&path, &["Calendar"]);
    assert_eq!(subjects(&after), ["kept", "again"]);
    let store = AddStore::open(&path).unwrap();
    for (id, _) in &after.messages {
        let entry = store.properties().make_entry_id(*id).unwrap();
        let message = store.open_message(&entry, None).unwrap();
        let text = match message.properties().get(0x1000) {
            Some(PropertyValue::Unicode(value)) => value.to_string(),
            value => panic!("no body: {value:?}"),
        };
        assert_eq!(text, body);
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn messages_are_deleted_from_an_ansi_file() {
    let path = temp_pst("delete-ansi");
    drop(AnsiPstFile::create_empty(&path).unwrap());
    let (unread, read) = (unread_flags(), []);
    drop(
        AnsiPstFile::append_many_in_folder_with_attachments(
            &path,
            &["Contacts"],
            &batch(&["unread one", "two", "three"], &unread, &read),
        )
        .unwrap(),
    );
    let before = seen(&path, &["Contacts"]);
    let gone = ids(&before, &["unread one", "three"]);
    // Each format through its own type.
    assert_eq!(
        UnicodePstFile::delete_messages(&path, &gone)
            .err()
            .map(|e| e.kind()),
        Some(io::ErrorKind::InvalidInput)
    );
    drop(AnsiPstFile::delete_messages(&path, &gone).unwrap());
    check_file(&path);
    crate::extra::ansi::tests::check_ansi_file(&path);
    let after = seen(&path, &["Contacts"]);
    assert_eq!(subjects(&after), ["two"]);
    assert_eq!(after.counts, (1, 0));
    assert_eq!(after.row_counts, (1, 0));
    drop(
        AnsiPstFile::append_many_in_folder_with_attachments(
            &path,
            &["Contacts"],
            &batch(&["four"], &unread, &read),
        )
        .unwrap(),
    );
    crate::extra::ansi::tests::check_ansi_file(&path);
    assert_eq!(subjects(&seen(&path, &["Contacts"])), ["two", "four"]);
    std::fs::remove_file(path).unwrap();
}

/// Outlook's own file's search folder and its contents table (`examples/Empty.pst`).
const SEARCH_FOLDER: u32 = 0x2223;
const SEARCH_CONTENTS: u32 = 0x2230;

/// The rows of the search folder's contents table.
fn search_rows(path: &Path) -> Vec<NodeId> {
    let mut file = File::open(path).unwrap();
    let header = FileHeader::read(&mut file).unwrap();
    let nbt = read_node_btree(&mut file, header.format(), header.node_btree()).unwrap();
    let entry = nbt
        .entries()
        .into_iter()
        .find(|entry| u32::from(entry.node()) == SEARCH_CONTENTS)
        .unwrap();
    let table = AddStore::open(path).unwrap().table(entry).unwrap();
    let rows = table
        .rows_matrix()
        .map(|row| NodeId::from(u32::from(row.id())))
        .collect();
    rows
}

/// List `messages` in the search folder's contents table, as Outlook does those that match
/// its criteria (each row its ID alone).
fn list_in_search_folder(path: &Path, messages: &[NodeId]) {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    let mut header = FileHeader::read(&mut file).unwrap();
    let format = header.format();
    let old_bbt = read_block_btree(&mut file, format, header.block_btree()).unwrap();
    let old_nbt = read_node_btree(&mut file, format, header.node_btree()).unwrap();
    let bbt_entries = old_bbt.entries();
    let mut nbt_entries = old_nbt.entries();
    assert!(nbt_entries
        .iter()
        .any(|entry| u32::from(entry.node()) == SEARCH_FOLDER));
    let entry = *nbt_entries
        .iter()
        .find(|entry| u32::from(entry.node()) == SEARCH_CONTENTS)
        .unwrap();
    let table = AddStore::open(path).unwrap().table(entry).unwrap();
    let mut subnodes =
        collect_subnode_entries(&mut file, format, header.block_btree(), entry.sub_node()).unwrap();
    let rows_node = table_rows_node(table.as_ref(), &subnodes).unwrap();
    let mut next_bid = header.next_block_index().max(
        bbt_entries
            .iter()
            .map(|entry| entry.block().block().index())
            .max()
            .unwrap()
            + 1,
    );
    let mut space = FileSpace::read(&mut file, &header, &old_bbt, &old_nbt).unwrap();
    header.update_unique();
    let columns = table.context().columns().len();
    let rows = messages
        .iter()
        .map(|message| (*message, (0..columns).map(|_| None).collect()))
        .collect();
    let (pages, rows, _) = rebuild_table_with_rows_and_version(
        table.as_ref(),
        rows,
        Some(rows_node),
        header.unique_value(),
        format,
    )
    .unwrap();
    let mut blocks = Vec::new();
    let data = push_data_tree_chunks(&mut blocks, &mut next_bid, &mut space, pages).unwrap();
    let rows = push_data_tree_chunks(&mut blocks, &mut next_bid, &mut space, rows).unwrap();
    subnodes.retain(|entry| entry.node() != rows_node);
    subnodes.push(UnicodeLeafSubNodeTreeEntry::new(rows_node, rows, None));
    let subnode = push_subnode_tree(&mut blocks, &mut next_bid, &mut space, subnodes).unwrap();
    nbt_entries.retain(|candidate| candidate.node() != entry.node());
    nbt_entries.push(UnicodeNodeBTreeEntry::new(
        entry.node(),
        data,
        Some(subnode),
        entry.parent(),
    ));
    commit_unicode_changes(
        path,
        file,
        header,
        &old_bbt,
        &old_nbt,
        bbt_entries,
        nbt_entries,
        blocks,
        next_bid,
        space,
        &[],
    )
    .unwrap();
}

#[test]
fn messages_are_deleted_from_outlooks_own_file() {
    let path = temp_pst("delete-outlook");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/Empty.pst"),
        &path,
    )
    .unwrap();
    let (unread, read) = (unread_flags(), []);
    drop(
        UnicodePstFile::append_many_in_folder_with_attachments(
            &path,
            &["Deleted Items"],
            &batch(&["a", "unread b", "c", "unread d"], &unread, &read),
        )
        .unwrap(),
    );
    // The reader opens the file as Outlook left it, as added to, and as deleted from.
    let before = seen(&path, &["Deleted Items"]);
    assert_eq!(before.counts, (4, 2));
    // Outlook's search folder lists two of them, one of which goes.
    list_in_search_folder(&path, &ids(&before, &["c", "unread d"]));
    check_file(&path);
    assert_eq!(search_rows(&path).len(), 2);
    drop(UnicodePstFile::delete_messages(&path, &ids(&before, &["a", "unread d"])).unwrap());
    check_file(&path);
    assert_eq!(search_rows(&path), ids(&before, &["c"]));
    let store = UnicodeStore::read(Rc::new(UnicodePstFile::open(&path).unwrap())).unwrap();
    let (_, opened) = folder(&store, &["Deleted Items"]);
    assert_eq!(opened.contents_table().unwrap().rows_matrix().count(), 2);
    let after = seen(&path, &["Deleted Items"]);
    assert_eq!(subjects(&after), ["unread b", "c"]);
    assert_eq!(after.counts, (2, 1));
    assert_eq!(after.row_counts, (2, 1));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn what_is_not_a_message_of_the_file_is_refused_before_it_is_touched() {
    let path = temp_pst("delete-refused");
    drop(UnicodePstFile::create_empty(&path).unwrap());
    let (unread, read) = (unread_flags(), []);
    drop(
        UnicodePstFile::append_many_in_folder_with_attachments(
            &path,
            &["Notes"],
            &batch(&["one", "two"], &unread, &read),
        )
        .unwrap(),
    );
    let seen_before = seen(&path, &["Notes"]);
    let one = seen_before.messages[0].0;
    let bytes = std::fs::read(&path).unwrap();
    let missing = NodeId::new(NodeIdType::NormalMessage, 0x7FFF).unwrap();
    let not_a_message = NodeId::from(u32::from(NID_ROOT_FOLDER));
    for messages in [
        vec![],
        vec![one, one],
        vec![one, missing],
        vec![not_a_message],
    ] {
        let error = UnicodePstFile::delete_messages(&path, &messages)
            .err()
            .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{messages:?}");
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    std::fs::remove_file(path).unwrap();
}
