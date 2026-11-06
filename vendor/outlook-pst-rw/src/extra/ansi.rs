//! MailCraft: adding to Outlook 97–2002's ANSI files, and making one.
//!
//! The writer reads and writes an ANSI file as it does a Unicode one ([`super::format`]); these
//! are the calls that take one. A new ANSI file is Outlook's empty file (the one the Unicode
//! writer starts from) written in the ANSI format: each property context and table of it again,
//! its strings 8-bit and its row indexes' row numbers 16-bit, the rest as it is.

use super::*;

impl AnsiPstFile {
    /// MailCraft: create a new ANSI file (Outlook 97–2002's format), unencoded, with an empty
    /// Inbox. MS-PST 2.2.1.2 says new files should be Unicode; this one is for a program that
    /// can only read the older format, and for tests.
    pub fn create_empty(path: impl AsRef<Path>) -> io::Result<Self> {
        create_empty_ansi_pst(path.as_ref())?;
        Self::open(path)
    }

    /// MailCraft: set the store's display name of an existing ANSI file, as
    /// [`UnicodePstFile::set_store_display_name`] does a Unicode one's (in Windows-1252).
    pub fn set_store_display_name(path: impl AsRef<Path>, name: &str) -> io::Result<Self> {
        expect_format(path.as_ref(), Format::Ansi)?;
        set_unicode_store_display_name(path.as_ref(), name)?;
        Self::open(path)
    }

    /// MailCraft: append messages and attachments to the default receive folder of an existing
    /// ANSI file, unencoded or permute- or cyclic-encoded, in one commit, as
    /// [`UnicodePstFile::append_many_with_attachments`] does to a Unicode one. Each message's
    /// strings are written 8-bit (MS-PST 2.5), in Windows-1252 when they are all in it, else in
    /// UTF-8, and its `PidTagMessageCodepage` says which.
    pub fn append_many_with_attachments(
        path: impl AsRef<Path>,
        messages: &[UnicodePstBatchMessage<'_>],
    ) -> io::Result<Self> {
        expect_format(path.as_ref(), Format::Ansi)?;
        append_unicode_pst(path.as_ref(), messages, None)?;
        Self::open(path)
    }

    /// MailCraft: append messages and attachments to `folder_path` of an existing ANSI file,
    /// relative to the IPM subtree, making the folders that are missing (their names in
    /// Windows-1252), as [`UnicodePstFile::append_many_in_folder_with_attachments`] does.
    pub fn append_many_in_folder_with_attachments(
        path: impl AsRef<Path>,
        folder_path: &[&str],
        messages: &[UnicodePstBatchMessage<'_>],
    ) -> io::Result<Self> {
        expect_format(path.as_ref(), Format::Ansi)?;
        append_unicode_pst(path.as_ref(), messages, Some(folder_path))?;
        Self::open(path)
    }

    /// MailCraft: make the folders of `folder_path` that are missing, as
    /// [`UnicodePstFile::create_folder`] does.
    pub fn create_folder(
        path: impl AsRef<Path>,
        folder_path: &[&str],
        container_class: &str,
    ) -> io::Result<Self> {
        expect_format(path.as_ref(), Format::Ansi)?;
        folders::create_folder(path.as_ref(), folder_path, container_class)?;
        Self::open(path)
    }

    /// MailCraft: the folder the file records as `role`, as [`UnicodePstFile::special_folder`]
    /// finds it.
    pub fn special_folder(
        path: impl AsRef<Path>,
        role: UnicodePstSpecialFolder,
    ) -> io::Result<Option<Vec<String>>> {
        expect_format(path.as_ref(), Format::Ansi)?;
        folders::special_folder(path.as_ref(), role)
    }

    /// MailCraft: record the folder at `folder_path` as `role`, as
    /// [`UnicodePstFile::set_special_folder`] does.
    pub fn set_special_folder(
        path: impl AsRef<Path>,
        role: UnicodePstSpecialFolder,
        folder_path: &[&str],
    ) -> io::Result<Self> {
        expect_format(path.as_ref(), Format::Ansi)?;
        folders::set_special_folder(path.as_ref(), role, folder_path)?;
        Self::open(path)
    }
}

/// `PidTagStoreSupportMask`'s `STORE_UNICODE_OK`, which an ANSI store doesn't say
/// (MS-OXCSTOR 2.2.2.1).
const STORE_UNICODE_OK: i32 = 0x0004_0000;

/// The HN signature and the client signatures of a property context and a table
/// (MS-PST 2.3.1.2).
const HN_SIGNATURE: u8 = 0xEC;
const HN_PROPERTIES: u8 = 0xBC;
const HN_TABLE: u8 = 0x7C;

/// Make an ANSI file at `path` from Outlook's empty file, then give it an Inbox. The file is
/// removed again when it can't be made whole.
fn create_empty_ansi_pst(path: &Path) -> io::Result<()> {
    let template = write_template()?;
    let made = (|| {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)?;
        write_ansi_copy(&template, path, file)
    })();
    let _ = std::fs::remove_file(&template);
    if let Err(error) = made {
        let _ = std::fs::remove_file(path);
        return Err(error);
    }
    if let Err(error) = ensure_unicode_folder_path(path, &["Inbox"], MAIL_CONTAINER_CLASS) {
        let _ = std::fs::remove_file(path);
        return Err(error);
    }
    Ok(())
}

/// How many names [`write_template`] tries before it gives up.
const TEMPLATE_TRIES: u32 = 16;

/// Outlook's empty Unicode file, which the reader reads from a file, written to a new file of
/// its own in the temporary folder: never beside the file being made, where a file of the user's
/// could have its name, and never over a file already there.
fn write_template() -> io::Result<std::path::PathBuf> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_nanos())
        .unwrap_or(0);
    let mut last = io::Error::from(io::ErrorKind::AlreadyExists);
    for _ in 0..TEMPLATE_TRIES {
        let template = std::env::temp_dir().join(format!(
            "outlook-pst-template-{}-{stamp}-{}.pst",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&template)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                last = error;
                continue;
            }
            Err(error) => return Err(error),
        };
        if let Err(error) = file.write_all(OUTLOOK_UNICODE_PST_TEMPLATE) {
            drop(file);
            let _ = std::fs::remove_file(&template);
            return Err(error);
        }
        return Ok(template);
    }
    Err(last)
}

/// Write each node of the Unicode file at `template` into `file`, a new ANSI file at `path`.
fn write_ansi_copy(template: &Path, path: &Path, mut file: File) -> io::Result<()> {
    let mut source = File::open(template)?;
    let header = FileHeader::read(&mut source)?;
    if header.format() != Format::Unicode {
        return Err(invalid_data("the template isn't a Unicode file"));
    }
    let encoding = header.crypt_method();
    let bbt_entries =
        read_block_btree(&mut source, Format::Unicode, header.block_btree())?.entries();
    let nbt_entries = read_node_btree(&mut source, Format::Unicode, header.node_btree())?.entries();
    let store = AddStore::open(template)?;
    let bbt = UnicodeBlockBTree::read(&mut source, header.block_btree())?;
    let mut cache = Default::default();

    // A header with no B-trees yet, its file ending where the first allocation map begins.
    let eof = u32::try_from(AMAP_FIRST_OFFSET).map_err(|_| PstError::IntegerConversion)?;
    let root = AnsiRoot::new(
        AnsiByteIndex::new(eof),
        AnsiByteIndex::new(eof),
        AnsiByteIndex::new(0),
        AnsiByteIndex::new(0),
        AnsiPageRef::default(),
        AnsiPageRef::default(),
        AmapStatus::Invalid,
    );
    let ansi = FileHeader::Ansi(AnsiHeader::new(root, NdbCryptMethod::None));
    ansi.write(&mut file)?;
    file.set_len(AMAP_FIRST_OFFSET)?;

    let mut blocks = Vec::new();
    let mut next_bid = ansi.next_block_index();
    let mut next_offset = FileSpace::new(AMAP_FIRST_OFFSET, Format::Ansi);
    let mut nodes = Vec::with_capacity(nbt_entries.len());
    for entry in &nbt_entries {
        // A node with no data keeps none.
        if entry.data().search_key() == 0 && entry.sub_node().is_none() {
            nodes.push(*entry);
            continue;
        }
        let data = data_tree_bytes(&mut source, encoding, &bbt_entries, entry.data())?;
        let kind = (data.get(2) == Some(&HN_SIGNATURE))
            .then(|| data.get(3).copied())
            .flatten();
        let (data, subnodes) = match kind {
            Some(HN_PROPERTIES) => {
                let mut properties =
                    read_properties(&mut source, &bbt, &mut cache, encoding, *entry)?;
                if entry.node() == NID_MESSAGE_STORE {
                    for (id, value) in &mut properties {
                        if let (0x340D, PropertyValue::Integer32(mask)) = (*id, &mut *value) {
                            *mask &= !STORE_UNICODE_OK;
                        }
                    }
                }
                let (pages, external) = property_context_pages(properties, 1, Format::Ansi.text())?;
                let data =
                    push_data_tree_chunks(&mut blocks, &mut next_bid, &mut next_offset, pages)?;
                let mut subnodes = Vec::with_capacity(external.len());
                for (node, value) in external {
                    let value =
                        push_data_tree(&mut blocks, &mut next_bid, &mut next_offset, &value)?;
                    subnodes.push(UnicodeLeafSubNodeTreeEntry::new(node, value, None));
                }
                (data, subnodes)
            }
            Some(HN_TABLE) => {
                let table = store.table(*entry)?;
                let (pages, rows) = ansi_table(table.as_ref())?;
                let data =
                    push_data_tree_chunks(&mut blocks, &mut next_bid, &mut next_offset, pages)?;
                let mut subnodes = Vec::new();
                if let Some((node, rows)) = rows {
                    let rows =
                        push_data_tree_chunks(&mut blocks, &mut next_bid, &mut next_offset, rows)?;
                    subnodes.push(UnicodeLeafSubNodeTreeEntry::new(node, rows, None));
                }
                (data, subnodes)
            }
            _ => {
                // Anything else (a heap of another kind, the search queues) holds no strings or
                // row indexes and is copied as it is; it has no subnodes in Outlook's file.
                if entry.sub_node().is_some() {
                    return Err(invalid_data("a template node has subnodes it can't copy"));
                }
                let data = push_data_tree(&mut blocks, &mut next_bid, &mut next_offset, &data)?;
                (data, Vec::new())
            }
        };
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
        nodes.push(UnicodeNodeBTreeEntry::new(
            entry.node(),
            data,
            subnode,
            entry.parent(),
        ));
    }
    drop(store);
    drop(source);

    // The header's next IDs pass the nodes', those of the types it counts.
    let reserved = nodes
        .iter()
        .map(|entry| entry.node())
        .filter(|node| node.id_type().is_ok())
        .collect::<Vec<_>>();
    let empty_blocks = BTreeLayout {
        leaves: Vec::new(),
        inner: Vec::new(),
    };
    let empty_nodes = BTreeLayout {
        leaves: Vec::new(),
        inner: Vec::new(),
    };
    commit_unicode_changes(
        path,
        file,
        ansi,
        &empty_blocks,
        &empty_nodes,
        Vec::new(),
        nodes,
        blocks,
        next_bid,
        next_offset,
        &reserved,
    )
}

/// The bytes of the data tree `block` names.
fn data_tree_bytes(
    file: &mut File,
    encoding: NdbCryptMethod,
    bbt_entries: &[UnicodeBlockBTreeEntry],
    block: UnicodeBlockId,
) -> io::Result<Vec<u8>> {
    let mut data = Vec::new();
    for (leaf, _) in data_tree_leaves(file, Format::Unicode, encoding, bbt_entries, block)? {
        data.extend(read_data_block(
            file,
            Format::Unicode,
            encoding,
            bbt_entries,
            leaf,
        )?);
    }
    Ok(data)
}

/// The properties of the property context `entry` names, read as the reader reads a folder's.
fn read_properties(
    file: &mut File,
    bbt: &UnicodeBlockBTree,
    cache: &mut RootBTreePageCache<UnicodeBlockBTree>,
    encoding: NdbCryptMethod,
    entry: UnicodeNodeBTreeEntry,
) -> io::Result<Vec<(u16, PropertyValue)>> {
    let heap = UnicodeHeapNode::read(file, bbt, cache, encoding, entry.data().search_key())?;
    let header = heap.header()?;
    let tree = <UnicodePstFile as PstFile>::PropertyTree::new(heap, header.user_root());
    let context = UnicodePropertyContext::new(entry, tree);
    context
        .properties()?
        .into_iter()
        .map(|(id, record)| {
            context
                .read_property(file, encoding, bbt, cache, record)
                .map(|value| (id, value))
        })
        .collect()
}

/// A table again as an ANSI file keeps it: its string columns 8-bit, its row index's row
/// numbers 16-bit (MS-PST 2.3.4.3.1), its rows in a subnode in blocks of an ANSI file's size.
fn ansi_table(table: &dyn TableContext) -> io::Result<RebuiltRecipientTable> {
    let old = table.context();
    let text = Format::Ansi.text();
    let narrow = |prop_type: PropertyType| match prop_type {
        PropertyType::Unicode => PropertyType::String8,
        PropertyType::MultipleUnicode => PropertyType::MultipleString8,
        other => other,
    };
    let columns = old
        .columns()
        .iter()
        .map(|column| {
            TableColumnDescriptor::new(
                narrow(column.prop_type()),
                column.prop_id(),
                column.offset(),
                column.size(),
                column.existence_bitmap_index(),
            )
        })
        .collect::<Vec<_>>();
    let ends = (
        old.end_4byte_values(),
        old.end_2byte_values(),
        old.end_1byte_values(),
        old.end_existence_bitmap(),
    );
    let row_context = TableContextInfo::new(
        ends.0,
        ends.1,
        ends.2,
        ends.3,
        HeapId::default(),
        None,
        columns.clone(),
    )?;
    let mut heap = HeapBuilder::new();
    let mut rows = Vec::new();
    let mut row_ids = Vec::new();
    for row in table.rows_matrix() {
        let cells = row
            .columns(old)?
            .into_iter()
            .zip(old.columns())
            .map(|(value, column)| match value {
                None => Ok(None),
                Some(TableRowColumnValue::Small(value)) => Ok(Some(TableCell::Value(value))),
                Some(TableRowColumnValue::Heap(id)) if u32::from(id) == 0 => Ok(None),
                Some(TableRowColumnValue::Node(id)) if u32::from(id) == 0 => Ok(None),
                Some(value) => table
                    .read_column(&value, column.prop_type())
                    .map(|value| Some(TableCell::Value(text.narrow(value)))),
            })
            .collect::<io::Result<Vec<_>>>()?;
        let row_id = NodeId::from(u32::from(row.id()));
        rows.push(encode_table_row(
            &row_context,
            &mut heap,
            row_id,
            row.unique(),
            cells,
        )?);
        row_ids.push(row_id);
    }
    let row_index = build_row_index(&mut heap, &row_ids, Format::Ansi)?;
    let rows_node = match old.rows() {
        _ if rows.is_empty() => None,
        Some(node) if !matches!(node.id_type(), Ok(NodeIdType::HeapNode)) => Some(node),
        _ => Some(NodeId::new(NodeIdType::Internal, 1)?),
    };
    let context = TableContextInfo::new(
        ends.0, ends.1, ends.2, ends.3, row_index, rows_node, columns,
    )?;
    let mut context_bytes = Vec::new();
    context.write(&mut context_bytes)?;
    let user_root = heap.alloc(context_bytes)?;
    let pages = heap.finish(HeapNodeType::Table, user_root)?;
    let row_size = usize::from(ends.3).max(1);
    let rows_per_block = (Format::Ansi.data_block_size() / row_size).max(1);
    let row_chunks = rows
        .chunks(rows_per_block)
        .map(|chunk| chunk.concat())
        .collect::<Vec<_>>();
    Ok((pages, rows_node.map(|node| (node, row_chunks))))
}

#[cfg(test)]
pub(super) mod tests;
