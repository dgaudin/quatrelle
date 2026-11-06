//! MailCraft: an ANSI file's NDB layer as the writer reads and writes it: its header, B-tree
//! pages and blocks read with the reader's Ansi types and widened to the writer's (Unicode)
//! ones, and narrowed back, each value checked, where they are written.

use super::*;

/// The header of a file the writer adds to, in either format, its values as the writer's
/// (Unicode) types: an ANSI file's are widened when read and checked when they are set.
pub(crate) enum FileHeader {
    Unicode(UnicodeHeader),
    Ansi(AnsiHeader),
}

fn ansi_block(id: UnicodeBlockId) -> io::Result<AnsiBlockId> {
    u32::try_from(u64::from(id))
        .map(AnsiBlockId::from)
        .map_err(|_| too_big())
}

fn ansi_page(id: UnicodePageId) -> io::Result<AnsiPageId> {
    u32::try_from(u64::from(id))
        .map(AnsiPageId::from)
        .map_err(|_| too_big())
}

fn ansi_index(index: u64) -> io::Result<AnsiByteIndex> {
    if index > ANSI_MAX_FILE_SIZE {
        return Err(too_big());
    }
    u32::try_from(index)
        .map(AnsiByteIndex::new)
        .map_err(|_| too_big())
}

fn ansi_page_ref(page: UnicodePageRef) -> io::Result<AnsiPageRef> {
    Ok(AnsiPageRef::new(
        ansi_page(page.block())?,
        ansi_index(page.index().index())?,
    ))
}

pub(crate) fn wide_block(id: AnsiBlockId) -> UnicodeBlockId {
    UnicodeBlockId::from(u64::from(u32::from(id)))
}

fn wide_page_ref(page: AnsiPageRef) -> UnicodePageRef {
    UnicodePageRef::new(
        UnicodePageId::from(u64::from(u32::from(page.block()))),
        UnicodeByteIndex::new(u64::from(page.index().index())),
    )
}

fn wide_bbt_entry(entry: &AnsiBlockBTreeEntry) -> UnicodeBlockBTreeEntry {
    let block = entry.block();
    let mut wide = UnicodeBlockBTreeEntry::new(
        UnicodeBlockRef::new(
            wide_block(block.block()),
            UnicodeByteIndex::new(u64::from(block.index().index())),
        ),
        entry.size(),
    );
    wide.set_ref_count(entry.ref_count());
    wide
}

fn wide_nbt_entry(entry: &AnsiNodeBTreeEntry) -> UnicodeNodeBTreeEntry {
    UnicodeNodeBTreeEntry::new(
        entry.node(),
        wide_block(entry.data()),
        entry.sub_node().map(wide_block),
        entry.parent(),
    )
}

pub(crate) fn ansi_bbt_entry(entry: &UnicodeBlockBTreeEntry) -> io::Result<AnsiBlockBTreeEntry> {
    let block = entry.block();
    let mut narrow = AnsiBlockBTreeEntry::new(
        AnsiBlockRef::new(
            ansi_block(block.block())?,
            ansi_index(block.index().index())?,
        ),
        entry.size(),
    );
    narrow.set_ref_count(entry.ref_count());
    Ok(narrow)
}

pub(super) fn ansi_nbt_entry(entry: &UnicodeNodeBTreeEntry) -> io::Result<AnsiNodeBTreeEntry> {
    Ok(AnsiNodeBTreeEntry::new(
        entry.node(),
        ansi_block(entry.data())?,
        entry.sub_node().map(ansi_block).transpose()?,
        entry.parent(),
    ))
}

fn ansi_sub_node_entry(
    entry: &UnicodeLeafSubNodeTreeEntry,
) -> io::Result<AnsiLeafSubNodeTreeEntry> {
    Ok(AnsiLeafSubNodeTreeEntry::new(
        entry.node(),
        ansi_block(entry.block())?,
        entry.sub_node().map(ansi_block).transpose()?,
    ))
}

fn wide_sub_node_entry(entry: &AnsiLeafSubNodeTreeEntry) -> UnicodeLeafSubNodeTreeEntry {
    UnicodeLeafSubNodeTreeEntry::new(
        entry.node(),
        wide_block(entry.block()),
        entry.sub_node().map(wide_block),
    )
}

impl FileHeader {
    /// The header of `file`, in the format it says it is in.
    pub(crate) fn read(file: &mut File) -> io::Result<Self> {
        let mut head = [0; 12];
        file.seek(SeekFrom::Start(0))?;
        file.read_exact(&mut head)?;
        file.seek(SeekFrom::Start(0))?;
        Ok(match Format::of(&head)? {
            Format::Unicode => Self::Unicode(UnicodeHeader::read(file)?),
            Format::Ansi => Self::Ansi(AnsiHeader::read(file)?),
        })
    }

    pub(crate) fn format(&self) -> Format {
        match self {
            Self::Unicode(_) => Format::Unicode,
            Self::Ansi(_) => Format::Ansi,
        }
    }

    pub(crate) fn crypt_method(&self) -> NdbCryptMethod {
        match self {
            Self::Unicode(header) => header.crypt_method(),
            Self::Ansi(header) => header.crypt_method(),
        }
    }

    pub(crate) fn next_block_index(&self) -> u64 {
        match self {
            Self::Unicode(header) => header.next_block().index(),
            Self::Ansi(header) => u64::from(header.next_block().index()),
        }
    }

    pub(crate) fn next_page(&self) -> UnicodePageId {
        match self {
            Self::Unicode(header) => header.next_page(),
            Self::Ansi(header) => UnicodePageId::from(u64::from(u32::from(header.next_page()))),
        }
    }

    pub(crate) fn unique_value(&self) -> u32 {
        match self {
            Self::Unicode(header) => header.unique_value(),
            Self::Ansi(header) => header.unique_value(),
        }
    }

    pub(crate) fn update_unique(&mut self) {
        match self {
            Self::Unicode(header) => header.update_unique(),
            Self::Ansi(header) => header.update_unique(),
        }
    }

    pub(crate) fn next_node_index(&self, id_type: NodeIdType) -> io::Result<u32> {
        Ok(match self {
            Self::Unicode(header) => header.next_node_index(id_type)?,
            Self::Ansi(header) => header.next_node_index(id_type)?,
        })
    }

    pub(crate) fn reserve_node(&mut self, node: NodeId) -> io::Result<()> {
        match self {
            Self::Unicode(header) => header.reserve_node(node)?,
            Self::Ansi(header) => header.reserve_node(node)?,
        }
        Ok(())
    }

    pub(crate) fn block_btree(&self) -> UnicodePageRef {
        match self {
            Self::Unicode(header) => *header.root().block_btree(),
            Self::Ansi(header) => wide_page_ref(*header.root().block_btree()),
        }
    }

    pub(crate) fn node_btree(&self) -> UnicodePageRef {
        match self {
            Self::Unicode(header) => *header.root().node_btree(),
            Self::Ansi(header) => wide_page_ref(*header.root().node_btree()),
        }
    }

    pub(crate) fn file_eof(&self) -> u64 {
        match self {
            Self::Unicode(header) => header.root().file_eof_index().index(),
            Self::Ansi(header) => u64::from(header.root().file_eof_index().index()),
        }
    }

    pub(crate) fn amap_status(&self) -> AmapStatus {
        match self {
            Self::Unicode(header) => header.root().amap_is_valid(),
            Self::Ansi(header) => header.root().amap_is_valid(),
        }
    }

    pub(crate) fn set_amap_status(&mut self, status: AmapStatus) {
        match self {
            Self::Unicode(header) => header.root_mut().set_amap_status(status),
            Self::Ansi(header) => header.root_mut().set_amap_status(status),
        }
    }

    pub(crate) fn set_next_block(&mut self, index: u64) -> io::Result<()> {
        match self {
            Self::Unicode(header) => header.set_next_block(UnicodeBlockId::new(false, index)?),
            Self::Ansi(header) => {
                let index = u32::try_from(index).map_err(|_| too_big())?;
                header.set_next_block(AnsiBlockId::new(false, index)?)
            }
        }
        Ok(())
    }

    pub(crate) fn set_next_page(&mut self, page: UnicodePageId) -> io::Result<()> {
        match self {
            Self::Unicode(header) => header.set_next_page(page),
            Self::Ansi(header) => header.set_next_page(ansi_page(page)?),
        }
        Ok(())
    }

    pub(crate) fn set_btrees(
        &mut self,
        node_btree: UnicodePageRef,
        block_btree: UnicodePageRef,
    ) -> io::Result<()> {
        match self {
            Self::Unicode(header) => header.root_mut().set_btrees(node_btree, block_btree),
            Self::Ansi(header) => header
                .root_mut()
                .set_btrees(ansi_page_ref(node_btree)?, ansi_page_ref(block_btree)?),
        }
        Ok(())
    }

    pub(crate) fn set_file_eof(&mut self, eof: u64, amap_last: u64) -> io::Result<()> {
        match self {
            Self::Unicode(header) => {
                header
                    .root_mut()
                    .set_file_eof_index(UnicodeByteIndex::new(eof));
                header
                    .root_mut()
                    .set_amap_last_index(UnicodeByteIndex::new(amap_last));
            }
            Self::Ansi(header) => {
                header.root_mut().set_file_eof_index(ansi_index(eof)?);
                header
                    .root_mut()
                    .set_amap_last_index(ansi_index(amap_last)?);
            }
        }
        Ok(())
    }

    pub(crate) fn write(&self, file: &mut File) -> io::Result<()> {
        file.seek(SeekFrom::Start(0))?;
        match self {
            Self::Unicode(header) => header.write(file),
            Self::Ansi(header) => header.write(file),
        }
    }
}

/// A file's block B-tree as the writer reads it, an ANSI one's entries widened.
pub(crate) fn read_block_btree(
    file: &mut File,
    format: Format,
    root: UnicodePageRef,
) -> io::Result<BTreeLayout<UnicodeBlockBTreeEntry>> {
    BTreeLayout::read(file, root, |file, page_ref| {
        Ok(match format {
            Format::Unicode => match UnicodeBlockBTree::read(file, page_ref)? {
                RootBTreePage::Leaf(page) => BTreePageItems::Leaf(page.entries().to_vec()),
                RootBTreePage::Intermediate(page, ..) => BTreePageItems::Intermediate(
                    page.level(),
                    page.entries()
                        .iter()
                        .map(|entry| (entry.key(), entry.block()))
                        .collect(),
                ),
            },
            Format::Ansi => match AnsiBlockBTree::read(file, ansi_page_ref(page_ref)?)? {
                RootBTreePage::Leaf(page) => {
                    BTreePageItems::Leaf(page.entries().iter().map(wide_bbt_entry).collect())
                }
                RootBTreePage::Intermediate(page, ..) => BTreePageItems::Intermediate(
                    page.level(),
                    page.entries()
                        .iter()
                        .map(|entry| (u64::from(entry.key()), wide_page_ref(entry.block())))
                        .collect(),
                ),
            },
        })
    })
}

/// A file's node B-tree as the writer reads it, an ANSI one's entries widened.
pub(crate) fn read_node_btree(
    file: &mut File,
    format: Format,
    root: UnicodePageRef,
) -> io::Result<BTreeLayout<UnicodeNodeBTreeEntry>> {
    BTreeLayout::read(file, root, |file, page_ref| {
        Ok(match format {
            Format::Unicode => match UnicodeNodeBTree::read(file, page_ref)? {
                RootBTreePage::Leaf(page) => BTreePageItems::Leaf(page.entries().to_vec()),
                RootBTreePage::Intermediate(page, ..) => BTreePageItems::Intermediate(
                    page.level(),
                    page.entries()
                        .iter()
                        .map(|entry| (entry.key(), entry.block()))
                        .collect(),
                ),
            },
            Format::Ansi => match AnsiNodeBTree::read(file, ansi_page_ref(page_ref)?)? {
                RootBTreePage::Leaf(page) => {
                    BTreePageItems::Leaf(page.entries().iter().map(wide_nbt_entry).collect())
                }
                RootBTreePage::Intermediate(page, ..) => BTreePageItems::Intermediate(
                    page.level(),
                    page.entries()
                        .iter()
                        .map(|entry| (u64::from(entry.key()), wide_page_ref(entry.block())))
                        .collect(),
                ),
            },
        })
    })
}

/// A block of a data tree: a leaf's data, or what an XBLOCK or XXBLOCK names.
pub(crate) enum TreeBlock {
    Leaf(Vec<u8>),
    Intermediate {
        level: u8,
        total_size: u32,
        children: Vec<UnicodeBlockId>,
    },
}

/// The block `entry` names, read as the reader reads a data tree's block.
pub(crate) fn read_tree_block(
    file: &mut File,
    format: Format,
    encoding: NdbCryptMethod,
    entry: &UnicodeBlockBTreeEntry,
) -> io::Result<TreeBlock> {
    Ok(match format {
        Format::Unicode => match UnicodeDataTree::read(file, encoding, entry)? {
            DataTree::Leaf(block) => TreeBlock::Leaf(block.data().to_vec()),
            DataTree::Intermediate(tree) => TreeBlock::Intermediate {
                level: tree.header().level(),
                total_size: tree.header().total_size(),
                children: tree.entries().iter().map(|entry| entry.block()).collect(),
            },
        },
        Format::Ansi => match AnsiDataTree::read(file, encoding, &ansi_bbt_entry(entry)?)? {
            DataTree::Leaf(block) => TreeBlock::Leaf(block.data().to_vec()),
            DataTree::Intermediate(tree) => TreeBlock::Intermediate {
                level: tree.header().level(),
                total_size: tree.header().total_size(),
                children: tree
                    .entries()
                    .iter()
                    .map(|entry| wide_block(entry.block()))
                    .collect(),
            },
        },
    })
}

/// A subnode tree's block: what an SIBLOCK names, or an SLBLOCK's entries.
pub(crate) enum SubNodeBlock {
    Intermediate(Vec<UnicodeBlockId>),
    Leaf(Vec<UnicodeLeafSubNodeTreeEntry>),
}

pub(crate) fn read_sub_node_block(
    file: &mut File,
    format: Format,
    entry: &UnicodeBlockBTreeEntry,
) -> io::Result<SubNodeBlock> {
    Ok(match format {
        Format::Unicode => match UnicodeSubNodeTree::read(file, entry)? {
            SubNodeTree::Intermediate(tree) => SubNodeBlock::Intermediate(
                tree.entries().iter().map(|entry| entry.block()).collect(),
            ),
            SubNodeTree::Leaf(tree) => SubNodeBlock::Leaf(tree.entries().to_vec()),
        },
        Format::Ansi => match AnsiSubNodeTree::read(file, &ansi_bbt_entry(entry)?)? {
            SubNodeTree::Intermediate(tree) => SubNodeBlock::Intermediate(
                tree.entries()
                    .iter()
                    .map(|entry| wide_block(entry.block()))
                    .collect(),
            ),
            SubNodeTree::Leaf(tree) => {
                SubNodeBlock::Leaf(tree.entries().iter().map(wide_sub_node_entry).collect())
            }
        },
    })
}

/// The entries of the subnode tree `subnode` names, through its SIBLOCK when it has one.
pub(crate) fn collect_subnode_entries(
    file: &mut File,
    format: Format,
    bbt_ref: UnicodePageRef,
    subnode: Option<UnicodeBlockId>,
) -> io::Result<Vec<UnicodeLeafSubNodeTreeEntry>> {
    let Some(subnode) = subnode else {
        return Ok(Vec::new());
    };
    match format {
        Format::Unicode => {
            let bbt = UnicodeBlockBTree::read(file, bbt_ref)?;
            let mut cache = Default::default();
            let block = bbt.find_entry(file, subnode.search_key(), &mut cache)?;
            let tree = SubNodeTree::<UnicodePstFile>::read(file, &block)?;
            Ok(tree.entries(file, &bbt, &mut cache)?.collect())
        }
        Format::Ansi => {
            let bbt = AnsiBlockBTree::read(file, ansi_page_ref(bbt_ref)?)?;
            let mut cache = Default::default();
            let block = bbt.find_entry(file, ansi_block(subnode)?.search_key(), &mut cache)?;
            let tree = SubNodeTree::<AnsiPstFile>::read(file, &block)?;
            Ok(tree
                .entries(file, &bbt, &mut cache)?
                .map(|entry| wide_sub_node_entry(&entry))
                .collect())
        }
    }
}

/// Write `block` at its offset as an ANSI file keeps it: a data block encoded with `encoding`,
/// as [`write_new_block`] writes a Unicode one.
pub(crate) fn write_ansi_block<W: Write + Seek>(
    file: &mut W,
    encoding: NdbCryptMethod,
    block: NewBlock,
) -> io::Result<()> {
    let id = ansi_block(block.id)?;
    let offset = ansi_index(block.offset)?;
    let block_signature = block_sig::compute_sig(block.offset as u32, u32::from(id));
    let trailer = AnsiBlockTrailer::new(block.size, block_signature, 0, id)?;
    let entry = AnsiBlockBTreeEntry::new(AnsiBlockRef::new(id, offset), block.size);
    file.seek(SeekFrom::Start(block.offset))?;
    match block.data {
        NewBlockData::Data(data) => AnsiDataBlock::new(encoding, data, trailer)?.write(file),
        NewBlockData::DataTree(entries, level, total_size) => {
            let header = DataTreeBlockHeader::new(
                level,
                u16::try_from(entries.len()).map_err(|_| PstError::IntegerConversion)?,
                total_size,
            );
            let entries = entries
                .iter()
                .map(|entry| ansi_block(entry.block()).map(AnsiDataTreeEntry::from))
                .collect::<io::Result<Vec<_>>>()?;
            AnsiDataTreeBlock::new(header, entries, trailer)?.write(file)
        }
        NewBlockData::SubNodes(entries) => {
            let header = AnsiSubNodeTreeBlockHeader::new(
                0,
                u16::try_from(entries.len()).map_err(|_| PstError::IntegerConversion)?,
            );
            let entries = entries
                .iter()
                .map(ansi_sub_node_entry)
                .collect::<io::Result<Vec<_>>>()?;
            let tree = AnsiLeafSubNodeTreeBlock::new(header, entries, trailer)?;
            SubNodeTree::<AnsiPstFile>::Leaf(Box::new(tree)).write(file, &entry)
        }
        NewBlockData::IntermediateSubNodes(entries, level) => {
            let header = AnsiSubNodeTreeBlockHeader::new(
                level,
                u16::try_from(entries.len()).map_err(|_| PstError::IntegerConversion)?,
            );
            let entries = entries
                .iter()
                .map(|entry| {
                    Ok(AnsiIntermediateSubNodeTreeEntry::new(
                        entry.node(),
                        ansi_block(entry.block())?,
                    ))
                })
                .collect::<io::Result<Vec<_>>>()?;
            let tree = AnsiIntermediateSubNodeTreeBlock::new(header, entries, trailer)?;
            SubNodeTree::<AnsiPstFile>::Intermediate(Box::new(tree)).write(file, &entry)
        }
    }
}

/// Write a B-tree page as an ANSI file keeps it, as [`write_btree_page`] writes a Unicode one.
pub(crate) fn write_ansi_btree_page(file: &mut File, page: NewBTreePage) -> io::Result<()> {
    let page_type = match &page.data {
        NewBTreePageData::BlockLeaf(_) => PageType::BlockBTree,
        NewBTreePageData::NodeLeaf(_) => PageType::NodeBTree,
        NewBTreePageData::Intermediate(page_type, ..) => *page_type,
    };
    let page_ref = ansi_page_ref(page.page_ref)?;
    let trailer = AnsiPageTrailer::new(
        page_type,
        page_type.signature(
            u64::from(page_ref.index().index()),
            u64::from(u32::from(page_ref.block())),
        ),
        page_ref.block(),
        0,
    );
    let (block_max, node_max, inner_max) = Format::Ansi.btree_capacity();
    let max = |count: usize| u8::try_from(count).map_err(|_| PstError::IntegerConversion);
    match page.data {
        NewBTreePageData::BlockLeaf(entries) => {
            let entries = entries
                .iter()
                .map(ansi_bbt_entry)
                .collect::<io::Result<Vec<_>>>()?;
            AnsiBlockBTree::Leaf(Box::new(AnsiBlockBTreePage::new(
                0,
                max(block_max)?,
                12,
                &entries,
                trailer,
            )?))
            .write(file, page_ref)
        }
        NewBTreePageData::NodeLeaf(entries) => {
            let entries = entries
                .iter()
                .map(ansi_nbt_entry)
                .collect::<io::Result<Vec<_>>>()?;
            AnsiNodeBTree::Leaf(Box::new(AnsiNodeBTreePage::new(
                0,
                max(node_max)?,
                16,
                &entries,
                trailer,
            )?))
            .write(file, page_ref)
        }
        NewBTreePageData::Intermediate(_, level, entries) => {
            let entries = entries
                .iter()
                .map(|entry| {
                    let key = u32::try_from(entry.key()).map_err(|_| too_big())?;
                    Ok(AnsiBTreePageEntry::new(key, ansi_page_ref(entry.block())?))
                })
                .collect::<io::Result<Vec<_>>>()?;
            file.seek(SeekFrom::Start(u64::from(page_ref.index().index())))?;
            let page = AnsiBTreeEntryPage::new(level, max(inner_max)?, 12, &entries, trailer)?;
            <AnsiBTreeEntryPage as AnsiBTreePageReadWrite<AnsiBTreePageEntry>>::write(&page, file)
        }
    }
}

/// An allocation map page's bits, if it reads as the reader reads one (type, its repeat and
/// CRC) with a zero signature and its own offset as its ID.
pub(crate) fn read_map_page(bytes: &[u8], format: Format, offset: u64) -> Option<MapBits> {
    match format {
        Format::Unicode => {
            let page =
                <<UnicodePstFile as PstFile>::AllocationMapPage as AllocationMapPageReadWrite<
                    UnicodePstFile,
                >>::read(&mut &bytes[..])
                .ok()?;
            let trailer = page.trailer();
            (trailer.signature() == 0 && u64::from(trailer.block_id()) == offset)
                .then(|| *page.map_bits())
        }
        Format::Ansi => {
            let page =
                <<AnsiPstFile as PstFile>::AllocationMapPage as AllocationMapPageReadWrite<
                    AnsiPstFile,
                >>::read(&mut &bytes[..])
                .ok()?;
            let trailer = page.trailer();
            (trailer.signature() == 0 && u64::from(u32::from(trailer.block_id())) == offset)
                .then(|| *page.map_bits())
        }
    }
}
