//! MailCraft: taking messages out of a file the writer adds to (MS-PST 2.6.1: a message is its
//! node in the node B-tree, the blocks of its property context and its subnode tree, and a row
//! of its folder's contents table).
//!
//! In one commit, as an append is: each message's node leaves the node B-tree, so its data and
//! subnode blocks lose a reference each and those no node names any more leave the block B-tree
//! (internal blocks' children with them, [`update_block_ref_counts`]); its row leaves its
//! folder's contents table and any search folder's that lists it; the folder's
//! `PidTagContentCount` and `PidTagContentUnreadCount` (0x3602, 0x3603) are counted again, in
//! its property context and in its row of its parent's hierarchy table; and a `SUD_MSG_DEL` for
//! it goes in the search update queue (MS-PST 2.4.8.4.2), so Outlook's search folders let it go
//! too. The header and the allocation map are written as after an append: the space the freed
//! blocks held is free in the new map and taken by the next blocks written.

use super::*;

impl UnicodePstFile {
    /// MailCraft: take `messages` (each a message's node ID, as its folder's contents table names
    /// it) out of the file at `path`, whatever folders they are in, in one commit. An ID that is
    /// not a message in a folder of the file, or is given twice, is an `InvalidInput` error before
    /// the file is touched.
    pub fn delete_messages(path: impl AsRef<Path>, messages: &[NodeId]) -> io::Result<Self> {
        expect_format(path.as_ref(), Format::Unicode)?;
        delete_messages(path.as_ref(), messages)?;
        Self::open(path)
    }
}

impl AnsiPstFile {
    /// MailCraft: take `messages` out of the ANSI file at `path`, as
    /// [`UnicodePstFile::delete_messages`] does.
    pub fn delete_messages(path: impl AsRef<Path>, messages: &[NodeId]) -> io::Result<Self> {
        expect_format(path.as_ref(), Format::Ansi)?;
        delete_messages(path.as_ref(), messages)?;
        Self::open(path)
    }
}

/// `PidTagMessageFlags`, and its `mfRead` (MS-OXCMSG 2.2.1.6).
const MESSAGE_FLAGS: u16 = 0x0E07;
const MF_READ: i32 = 0x0001;
/// `PidTagContentCount` and `PidTagContentUnreadCount`.
const CONTENT_COUNT: u16 = 0x3602;
const CONTENT_UNREAD: u16 = 0x3603;

/// A table row's ID and the cells it takes.
type RowCells = (NodeId, Vec<(u16, PropertyValue)>);

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn delete_messages(path: &Path, messages: &[NodeId]) -> io::Result<()> {
    if messages.is_empty() {
        return Err(invalid("deleting requires at least one message".into()));
    }
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let mut header = FileHeader::read(&mut file)?;
    let format = header.format();
    let old_bbt = read_block_btree(&mut file, format, header.block_btree())?;
    let old_nbt = read_node_btree(&mut file, format, header.node_btree())?;
    let bbt_entries = old_bbt.entries();
    let mut nbt_entries = old_nbt.entries();
    let nodes: HashMap<u32, UnicodeNodeBTreeEntry> = nbt_entries
        .iter()
        .map(|entry| (u32::from(entry.node()), *entry))
        .collect();

    // Each message, in a folder of the file: the folders and what each loses.
    let mut removed = HashSet::new();
    let mut folders: BTreeMap<u32, Vec<NodeId>> = BTreeMap::new();
    for message in messages {
        let entry = nodes
            .get(&u32::from(*message))
            .filter(|entry| matches!(entry.node().id_type(), Ok(NodeIdType::NormalMessage)))
            .ok_or_else(|| {
                invalid(format!(
                    "node {:X} is not a message of the file",
                    u32::from(*message)
                ))
            })?;
        let folder = entry
            .parent()
            .filter(|parent| {
                matches!(parent.id_type(), Ok(NodeIdType::NormalFolder))
                    && nodes.contains_key(&u32::from(*parent))
            })
            .ok_or_else(|| {
                invalid(format!(
                    "message {:X} is in no folder of the file",
                    u32::from(*message)
                ))
            })?;
        if !removed.insert(u32::from(*message)) {
            return Err(invalid(format!(
                "message {:X} is given twice",
                u32::from(*message)
            )));
        }
        folders.entry(u32::from(folder)).or_default().push(*message);
    }

    let store = AddStore::open(path)
        .map_err(|err| io::Error::new(err.kind(), format!("read message store: {err}")))?;
    // The tables that list them: their folders' contents tables, and any search folder's.
    let mut listings = Vec::new();
    for folder in folders.keys() {
        let folder = NodeId::from(*folder);
        listings.push((
            folder,
            NodeId::new(NodeIdType::ContentsTable, folder.index())?,
        ));
    }
    for entry in &nbt_entries {
        if !matches!(entry.node().id_type(), Ok(NodeIdType::SearchContentsTable)) {
            continue;
        }
        let search_folder = NodeId::new(NodeIdType::SearchFolder, entry.node().index())?;
        // A search folder's table that doesn't read is left as it is: Outlook brings it up to
        // date from the search update queue.
        let Ok(table) = store.table(*entry) else {
            continue;
        };
        if nodes.contains_key(&u32::from(search_folder))
            && table
                .rows_matrix()
                .any(|row| removed.contains(&u32::from(row.id())))
        {
            listings.push((search_folder, entry.node()));
        }
    }

    let mut next_bid = header.next_block_index().max(
        bbt_entries
            .iter()
            .map(|entry| entry.block().block().index())
            .max()
            .unwrap_or(0)
            .saturating_add(1),
    );
    let mut next_offset = FileSpace::read(&mut file, &header, &old_bbt, &old_nbt)?;
    header.update_unique();
    let row_version = header.unique_value();
    let mut blocks = Vec::new();
    let mut replaced = Vec::new();
    // Each parent's hierarchy table rows, by folder, with the counts they take.
    let mut counted: BTreeMap<u32, Vec<RowCells>> = BTreeMap::new();

    for (folder, table_node) in listings {
        let (Some(table_entry), Some(folder_entry)) = (
            nodes.get(&u32::from(table_node)).copied(),
            nodes.get(&u32::from(folder)).copied(),
        ) else {
            continue;
        };
        let table = store
            .table(table_entry)
            .map_err(|err| io::Error::new(err.kind(), format!("read folder contents: {err}")))?;
        let gone: Vec<&TableRowData> = table
            .rows_matrix()
            .filter(|row| removed.contains(&u32::from(row.id())))
            .collect();
        if gone.is_empty() {
            continue;
        }
        // The cells kept in the table's subnodes go with their rows.
        let mut dropped = HashSet::new();
        let mut unread_gone = 0i32;
        for row in &gone {
            for value in row.columns(table.context())?.into_iter().flatten() {
                if let TableRowColumnValue::Node(node) = value {
                    dropped.insert(u32::from(node));
                }
            }
            if row_unread(&store, table.as_ref(), row) {
                unread_gone = unread_gone.saturating_add(1);
            }
        }
        let mut subnodes = collect_subnode_entries(
            &mut file,
            format,
            header.block_btree(),
            table_entry.sub_node(),
        )?;
        let rows_node = table_rows_node(table.as_ref(), &subnodes)?;
        let (pages, rows, row_ids) = rebuild_rows(
            table.as_ref(),
            &removed,
            &[],
            rows_node,
            row_version,
            format,
        )?;
        let data = push_data_tree_chunks(&mut blocks, &mut next_bid, &mut next_offset, pages)?;
        subnodes.retain(|entry| {
            entry.node() != rows_node && !dropped.contains(&u32::from(entry.node()))
        });
        if !row_ids.is_empty() {
            let rows = push_data_tree_chunks(&mut blocks, &mut next_bid, &mut next_offset, rows)?;
            subnodes.push(UnicodeLeafSubNodeTreeEntry::new(rows_node, rows, None));
        }
        let subnode = if subnodes.is_empty() {
            None
        } else {
            Some(push_subnode_tree(
                &mut blocks,
                &mut next_bid,
                &mut next_offset,
                subnodes,
            )?)
        };
        replaced.push(UnicodeNodeBTreeEntry::new(
            table_node,
            data,
            subnode,
            table_entry.parent(),
        ));

        // The folder's counts, in its property context.
        let opened = store
            .open_folder(&store.properties().make_entry_id(folder)?)
            .map_err(|err| io::Error::new(err.kind(), format!("open folder: {err}")))?;
        let mut properties = writable_folder_properties(opened.as_ref());
        let content_count =
            i32::try_from(row_ids.len()).map_err(|_| PstError::IntegerConversion)?;
        let unread_before = properties
            .iter()
            .find_map(|(id, value)| match (id, value) {
                (&CONTENT_UNREAD, PropertyValue::Integer32(count)) => Some((*count).max(0)),
                _ => None,
            })
            .unwrap_or(0);
        let unread_count = unread_before
            .saturating_sub(unread_gone)
            .clamp(0, content_count);
        set_folder_integer_property(&mut properties, CONTENT_COUNT, content_count);
        set_folder_integer_property(&mut properties, CONTENT_UNREAD, unread_count);
        let mut folder_subnodes = collect_subnode_entries(
            &mut file,
            format,
            header.block_btree(),
            folder_entry.sub_node(),
        )?;
        let next_property_subnode = folder_subnodes
            .iter()
            .filter(|entry| matches!(entry.node().id_type(), Ok(NodeIdType::Internal)))
            .map(|entry| entry.node().index())
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(PstError::IntegerConversion)?;
        let (folder_pages, external) =
            property_context_pages(properties, next_property_subnode, format.text())?;
        let folder_data =
            push_data_tree_chunks(&mut blocks, &mut next_bid, &mut next_offset, folder_pages)?;
        for (node, data) in external {
            let data = push_data_tree(&mut blocks, &mut next_bid, &mut next_offset, &data)?;
            folder_subnodes.push(UnicodeLeafSubNodeTreeEntry::new(node, data, None));
        }
        let folder_subnode = if folder_subnodes.is_empty() {
            None
        } else {
            Some(push_subnode_tree(
                &mut blocks,
                &mut next_bid,
                &mut next_offset,
                folder_subnodes,
            )?)
        };
        replaced.push(UnicodeNodeBTreeEntry::new(
            folder,
            folder_data,
            folder_subnode,
            folder_entry.parent(),
        ));
        if let Some(parent) = folder_entry.parent() {
            counted.entry(u32::from(parent)).or_default().push((
                folder,
                vec![
                    (CONTENT_COUNT, PropertyValue::Integer32(content_count)),
                    (CONTENT_UNREAD, PropertyValue::Integer32(unread_count)),
                ],
            ));
        }
    }

    // Each parent's hierarchy table, its folders' rows with their new counts (the counts its
    // columns have), once for all of them.
    for (parent, updates) in counted {
        let node = NodeId::new(NodeIdType::HierarchyTable, NodeId::from(parent).index())?;
        let Some(entry) = nodes.get(&u32::from(node)).copied() else {
            continue;
        };
        let table = store.table(entry).map_err(|err| {
            io::Error::new(err.kind(), format!("read parent hierarchy table: {err}"))
        })?;
        let mut subnodes =
            collect_subnode_entries(&mut file, format, header.block_btree(), entry.sub_node())?;
        let rows_node = table_rows_node(table.as_ref(), &subnodes)?;
        let (pages, rows, row_ids) = rebuild_rows(
            table.as_ref(),
            &HashSet::new(),
            &updates,
            rows_node,
            row_version,
            format,
        )?;
        let data = push_data_tree_chunks(&mut blocks, &mut next_bid, &mut next_offset, pages)?;
        subnodes.retain(|entry| entry.node() != rows_node);
        if !row_ids.is_empty() {
            let rows = push_data_tree_chunks(&mut blocks, &mut next_bid, &mut next_offset, rows)?;
            subnodes.push(UnicodeLeafSubNodeTreeEntry::new(rows_node, rows, None));
        }
        let subnode = if subnodes.is_empty() {
            None
        } else {
            Some(push_subnode_tree(
                &mut blocks,
                &mut next_bid,
                &mut next_offset,
                subnodes,
            )?)
        };
        replaced.push(UnicodeNodeBTreeEntry::new(
            node,
            data,
            subnode,
            entry.parent(),
        ));
    }

    let changed: HashSet<u32> = replaced
        .iter()
        .map(|entry| u32::from(entry.node()))
        .collect();
    nbt_entries.retain(|entry| {
        let node = u32::from(entry.node());
        !removed.contains(&node) && !changed.contains(&node)
    });
    nbt_entries.extend(replaced);
    append_search_updates(
        &*store,
        &mut nbt_entries,
        &mut blocks,
        &mut next_bid,
        &mut next_offset,
        folders.iter().flat_map(|(folder, messages)| {
            messages
                .iter()
                .map(|message| SearchUpdateData::MessageDeleted {
                    parent: NodeId::from(*folder),
                    message: *message,
                })
        }),
    )?;

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
        next_offset,
        &[],
    )
}

/// Whether the message of a contents table's `row` is unread: its row's `PidTagMessageFlags`, or
/// the message's own when the table has no such column; read when neither reads.
fn row_unread(store: &AddStore, table: &dyn TableContext, row: &TableRowData) -> bool {
    let context = table.context();
    let column = context.columns().iter().position(|column| {
        column.prop_id() == MESSAGE_FLAGS && column.prop_type() == PropertyType::Integer32
    });
    let flags = match column {
        Some(index) => {
            row.columns(context)
                .ok()
                .and_then(|values| match values.into_iter().nth(index) {
                    Some(Some(TableRowColumnValue::Small(PropertyValue::Integer32(flags)))) => {
                        Some(flags)
                    }
                    _ => None,
                })
        }
        None => store
            .properties()
            .make_entry_id(NodeId::from(u32::from(row.id())))
            .and_then(|entry| store.open_message(&entry, Some(&[MESSAGE_FLAGS])))
            .ok()
            .and_then(|message| match message.properties().get(MESSAGE_FLAGS) {
                Some(PropertyValue::Integer32(flags)) => Some(*flags),
                _ => None,
            }),
    };
    flags.is_some_and(|flags| flags & MF_READ == 0)
}

/// `table` written again in a heap of its own: without the rows `removed` names, and with the
/// cells `updates` gives each of its rows (bumping their row versions to `row_version`). Rows
/// keep their cells; those kept in subnodes keep their nodes.
fn rebuild_rows(
    table: &dyn TableContext,
    removed: &HashSet<u32>,
    updates: &[RowCells],
    rows_node: NodeId,
    row_version: u32,
    format: Format,
) -> io::Result<RebuiltTable> {
    let block_size = format.data_block_size();
    let context = table.context();
    let row_size = usize::from(context.end_existence_bitmap());
    if row_size == 0 || row_size > block_size {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "table rows do not fit in a PST data block",
        ));
    }
    let mut heap = HeapBuilder::new();
    let mut chunks: Vec<Vec<u8>> = Vec::new();
    let mut row_ids = Vec::new();
    for row in table.rows_matrix() {
        let row_id = NodeId::from(u32::from(row.id()));
        if removed.contains(&u32::from(row_id)) {
            continue;
        }
        let mut cells = row
            .columns(context)?
            .into_iter()
            .zip(context.columns())
            .map(|(value, column)| match value {
                None => Ok(None),
                Some(TableRowColumnValue::Small(value)) => Ok(Some(TableCell::Value(value))),
                Some(TableRowColumnValue::Heap(id)) if u32::from(id) == 0 => Ok(None),
                Some(TableRowColumnValue::Node(id)) if u32::from(id) == 0 => Ok(None),
                Some(value @ TableRowColumnValue::Heap(_)) => table
                    .read_column(&value, column.prop_type())
                    .map(TableCell::Value)
                    .map(Some),
                Some(TableRowColumnValue::Node(node)) => Ok(Some(TableCell::Node(node))),
            })
            .collect::<io::Result<Vec<_>>>()?;
        let mut unique = row.unique();
        if let Some((_, cells_given)) = updates.iter().find(|(id, _)| *id == row_id) {
            unique = row_version;
            for (column, cell) in context.columns().iter().zip(&mut cells) {
                if let Some((_, value)) = cells_given.iter().find(|(property, value)| {
                    column.prop_id() == *property && column.prop_type() == PropertyType::from(value)
                }) {
                    *cell = Some(TableCell::Value(value.clone()));
                } else if column.prop_id() == LTP_ROW_VERSION_PROP_ID
                    && column.prop_type() == PropertyType::Integer32
                {
                    *cell = Some(TableCell::Value(PropertyValue::Integer32(
                        row_version as i32,
                    )));
                }
            }
        }
        let data = encode_table_row(context, &mut heap, row_id, unique, cells)?;
        match chunks.last_mut() {
            Some(chunk) if chunk.len() + row_size <= block_size => chunk.extend(data),
            _ => {
                let mut chunk = Vec::with_capacity(block_size);
                chunk.extend(data);
                chunks.push(chunk);
            }
        }
        row_ids.push(row_id);
    }

    let row_index = build_row_index(&mut heap, &row_ids, format)?;
    let rows_node = (!row_ids.is_empty()).then_some(rows_node);
    let info = TableContextInfo::new(
        context.end_4byte_values(),
        context.end_2byte_values(),
        context.end_1byte_values(),
        context.end_existence_bitmap(),
        row_index,
        rows_node,
        context.columns().to_vec(),
    )?;
    let mut info_data = Vec::new();
    info.write(&mut info_data)?;
    let user_root = heap.alloc(info_data)?;
    let pages = heap.finish(HeapNodeType::Table, user_root)?;
    Ok((pages, chunks, row_ids))
}

#[cfg(test)]
mod tests;
