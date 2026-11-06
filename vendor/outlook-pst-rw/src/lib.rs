#![doc = include_str!("../README.md")]

use byteorder::WriteBytesExt;
use std::{
    cell::RefMut,
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    fmt::Debug,
    fs::{File, OpenOptions},
    io::{self, BufWriter, Cursor, Read, Seek, SeekFrom, Write},
    mem,
    path::Path,
    rc::Rc,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};
use thiserror::Error;
use tracing::{error, instrument, warn};

pub mod ltp;
pub mod messaging;
pub mod ndb;

mod block_sig;
mod crc;
mod encode;
// MailCraft: a message's class and added properties.
mod extra;

use extra::*;
pub use extra::{
    UnicodePstAttachmentContent, UnicodePstAttachmentSource, UnicodePstEmbeddedMessage,
    UnicodePstName, UnicodePstProperty, UnicodePstPropertyKey, UnicodePstSpecialFolder,
    UnicodePstValue, MAX_EMBEDDED_DEPTH,
};

use ltp::{
    heap::*, prop_context::*, prop_type::PropertyType, read_write::*, table_context::*, tree::*,
};
use messaging::read_write::SearchReadWrite;
use messaging::{folder::*, message::*, named_prop::*, search::*, store::*};
use ndb::{
    block::*, block_id::*, block_ref::*, byte_index::*, header::*, node_id::*, page::*,
    read_write::*, root::*, *,
};

#[derive(Error, Debug)]
pub enum PstError {
    #[error("Opened read-only")]
    OpenedReadOnly,
    #[error("Cannot write to file: {0}")]
    NoWriteAccess(String),
    #[error("I/O error: {0:?}")]
    Io(#[from] io::Error),
    #[error("I/O error: {0}")]
    BorrowedIo(String),
    #[error("Failed to lock file")]
    LockError,
    #[error("Integer conversion failed")]
    IntegerConversion,
    #[error("Node Database error: {0}")]
    NodeDatabaseError(#[from] NdbError),
    #[error("AllocationMapPage not found: {0}")]
    AllocationMapPageNotFound(usize),
    #[error("Invalid BTree page: offset: 0x{0:X}")]
    InvalidBTreePage(u64),
}

impl From<&PstError> for io::Error {
    fn from(err: &PstError) -> Self {
        match err {
            PstError::NoWriteAccess(path) => {
                Self::new(io::ErrorKind::PermissionDenied, path.as_str())
            }
            err => Self::other(format!("{err:?}")),
        }
    }
}

impl From<PstError> for io::Error {
    fn from(err: PstError) -> Self {
        match err {
            PstError::NoWriteAccess(path) => {
                Self::new(io::ErrorKind::PermissionDenied, path.as_str())
            }
            PstError::Io(err) => err,
            err => Self::other(err),
        }
    }
}

impl From<&io::Error> for PstError {
    fn from(err: &io::Error) -> Self {
        Self::BorrowedIo(format!("{err:?}"))
    }
}

type PstResult<T> = std::result::Result<T, PstError>;

/// The methods on this trait and the [`PstFileInner`] struct are not public, PST modifications
/// have to go through `pub fn` methods on the [`PstFileLockGuard`] type which encapsulates a `dyn`
/// reference to this trait.
trait PstFileLock<Pst>
where
    Pst: PstFile,
{
    fn start_write(&mut self) -> io::Result<()>;
    fn finish_write(&mut self) -> io::Result<()>;

    fn block_cache(&self) -> RefMut<'_, RootBTreePageCache<<Pst as PstFile>::BlockBTree>>;
    fn node_cache(&self) -> RefMut<'_, RootBTreePageCache<<Pst as PstFile>::NodeBTree>>;
}

/// This is the public interface for writing to a PST.
pub struct PstFileLockGuard<'a, Pst>
where
    Pst: PstFile,
{
    pst: &'a mut dyn PstFileLock<Pst>,
}

impl<'a, Pst> PstFileLockGuard<'a, Pst>
where
    Pst: PstFile,
{
    fn new(pst: &'a mut dyn PstFileLock<Pst>) -> io::Result<Self> {
        pst.start_write()?;
        Ok(Self { pst })
    }

    /// Explicitly flush pending updates to the PST file. This will still happen implicitly when
    /// the [`PstFileLockGuard`] is dropped, but this allows you to handle errors.
    #[instrument(skip_all)]
    pub fn flush(&mut self) -> io::Result<()> {
        self.pst.finish_write().inspect_err(|err| {
            error!(
                name: "PstFinishWriteFailed",
                ?err,
                "PstFileLock::finish_write failed"
            );
        })?;

        Ok(())
    }
}

impl<Pst> Drop for PstFileLockGuard<'_, Pst>
where
    Pst: PstFile,
{
    #[instrument(skip_all)]
    fn drop(&mut self) {
        if let Err(err) = self.flush() {
            error!(
                name: "PstFileLockGuardFlushFailed",
                ?err,
                "Writing to the PST file failed"
            );
        }
    }
}

pub trait PstReader: Read + Seek {}

impl<T> PstReader for T where T: Read + Seek {}

/// [PST File](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-pst/6b57253b-0853-47bb-99bb-d4b8f78105f0)
pub trait PstFile: Sized {
    type BlockId: BlockId<Index = Self::BTreeKey> + BlockIdReadWrite;
    type PageId: BlockId<Index = Self::BTreeKey> + BlockIdReadWrite;
    type ByteIndex: ByteIndex + ByteIndexReadWrite;
    type BlockRef: BlockRef<Block = Self::BlockId, Index = Self::ByteIndex> + BlockRefReadWrite;
    type PageRef: BlockRef<Block = Self::PageId, Index = Self::ByteIndex> + BlockRefReadWrite;
    type Root: Root<Self>;
    type Header: Header<Self>;
    type PageTrailer: PageTrailer<BlockId = Self::PageId> + PageTrailerReadWrite;
    type BTreeKey: BTreeEntryKey;
    type NodeBTreeEntry: NodeBTreeEntry<Block = Self::BlockId> + BTreeEntry<Key = Self::BTreeKey>;
    type NodeBTree: NodeBTree<Self, Self::NodeBTreeEntry>;
    type BlockBTreeEntry: BlockBTreeEntry<Block = Self::BlockRef> + BTreeEntry<Key = Self::BTreeKey>;
    type BlockBTree: BlockBTree<Self, Self::BlockBTreeEntry>;
    type BlockTrailer: BlockTrailer<BlockId = Self::BlockId>;
    type AllocationMapPage: AllocationMapPage<Self>;
    type AllocationPageMapPage: AllocationPageMapPage<Self>;
    type FreeMapPage: FreeMapPage<Self>;
    type FreePageMapPage: FreePageMapPage<Self>;
    type DensityListPage: DensityListPage<Self>;
    type DataTreeEntry: IntermediateTreeEntry + IntermediateDataTreeEntry<Self>;
    type DataTreeBlock: IntermediateTreeBlock<
        Header = DataTreeBlockHeader,
        Entry = Self::DataTreeEntry,
        Trailer = Self::BlockTrailer,
    >;
    type DataBlock: Block<Trailer = Self::BlockTrailer>;
    type SubNodeTreeBlockHeader: IntermediateTreeHeader;
    type SubNodeTreeBlock: IntermediateTreeBlock<
        Header = Self::SubNodeTreeBlockHeader,
        Entry = IntermediateSubNodeTreeEntry<Self::BlockId>,
        Trailer = Self::BlockTrailer,
    >;
    type SubNodeBlock: IntermediateTreeBlock<
        Header = Self::SubNodeTreeBlockHeader,
        Entry = LeafSubNodeTreeEntry<Self::BlockId>,
        Trailer = Self::BlockTrailer,
    >;
    type TableContext: TableContext;
    type PropertyContext: PropertyContext;
    type HeapNode: HeapNode;
    type PropertyTree: HeapTree<Key = PropertyTreeRecordKey, Value = PropertyTreeRecordValue>;
    type Store: Store;
    type Folder: Folder;
    type Message: Message;
    type NamedPropertyMap: NamedPropertyMap;
    type SearchUpdateQueue: SearchUpdateQueue;

    fn header(&self) -> &Self::Header;
    fn density_list(&self) -> Result<&dyn DensityListPage<Self>, &io::Error>;
    fn reader(&self) -> &Mutex<Box<dyn PstReader>>;
    fn lock(&mut self) -> io::Result<PstFileLockGuard<'_, Self>>;

    fn read_node(&self, node: NodeId) -> io::Result<Self::NodeBTreeEntry>;
    fn read_block(&self, block: Self::BlockId) -> io::Result<Vec<u8>>;

    /// MailCraft: the most bytes a message's or an attachment's value may hold in this file
    /// (no limit unless one was set, beyond the size each value declares).
    fn max_value_size(&self) -> u64;
}

struct PstFileInner<Pst>
where
    Pst: PstFile,
{
    reader: Mutex<Box<dyn PstReader>>,
    writer: PstResult<Mutex<BufWriter<File>>>,
    header: Pst::Header,
    density_list: io::Result<Pst::DensityListPage>,
    node_cache: NodeBTreePageCache<Pst>,
    block_cache: BlockBTreePageCache<Pst>,
    max_value_size: AtomicU64,
}

pub struct UnicodePstFile {
    inner: PstFileInner<Self>,
}

/// A recipient type written to `PR_RECIPIENT_TYPE`.
#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum UnicodePstRecipientType {
    #[default]
    To = 1,
    Cc = 2,
    Bcc = 3,
}

/// A message recipient for the Unicode PST writer.
#[derive(Clone, Copy, Default)]
pub struct UnicodePstRecipient<'a> {
    pub name: &'a str,
    pub email: &'a str,
    pub recipient_type: UnicodePstRecipientType,
    /// MailCraft: `PidTagRecipientFlags` (0x5FFD, MS-OXOCAL 2.2.4.10.1: `recipSendable` 0x1,
    /// `recipOrganizer` 0x2…), in the recipient's row when given.
    pub flags: Option<i32>,
    /// MailCraft: `PidTagRecipientTrackStatus` (0x5FFF, MS-OXOCAL 2.2.4.10.2: how a meeting's
    /// attendee answered), in the recipient's row when given.
    pub track_status: Option<i32>,
}

/// Message input for the Unicode PST writer.
#[derive(Clone, Copy)]
pub struct UnicodePstMessage<'a> {
    pub subject: &'a str,
    pub sender_name: &'a str,
    pub sender_email: &'a str,
    pub recipients: &'a [UnicodePstRecipient<'a>],
    pub body: &'a str,
    /// UTF-8 HTML for `PR_HTML`; `body` remains the plain-text fallback.
    pub html_body: Option<&'a str>,
    pub message_id: &'a str,
    /// Number of 100-nanosecond intervals since 1601-01-01 UTC.
    pub delivery_time: i64,
    /// MailCraft: `PR_MESSAGE_CLASS`, such as `IPM.Note`.
    pub message_class: &'a str,
    /// MailCraft: properties written beside the writer's own, in place of the same ones; in the
    /// message and in its row of the folder's contents table. `PR_MESSAGE_FLAGS` keeps
    /// `mfHasAttach` as the message is (read when not given).
    pub properties: &'a [UnicodePstProperty<'a>],
}

/// A by-value file attachment for the Unicode PST writer.
#[derive(Default)]
pub struct UnicodePstAttachment<'a> {
    /// File name shown by mail clients.
    pub filename: &'a str,
    /// MIME content type, such as `application/pdf`.
    pub mime_type: &'a str,
    /// MIME Content-ID for an inline HTML attachment. Surrounding `<` and `>` are removed.
    /// The HTML body must reference the normalized value with the `cid:` scheme.
    pub content_id: Option<&'a str>,
    /// Exact attachment bytes.
    pub data: &'a [u8],
    /// MailCraft: what the attachment holds when it isn't `data`: a file read from a source as
    /// it is written, or an attached message.
    pub content: UnicodePstAttachmentContent<'a>,
    /// MailCraft: tagged properties of the attachment's own, in place of the writer's of the
    /// same ID (not its size, value or method).
    pub properties: &'a [UnicodePstProperty<'a>],
}

/// One message and its attachments for a bulk append operation.
#[derive(Clone, Copy)]
pub struct UnicodePstBatchMessage<'a> {
    pub message: UnicodePstMessage<'a>,
    pub attachments: &'a [UnicodePstAttachment<'a>],
}

impl UnicodePstFile {
    pub fn read_from(reader: Box<dyn PstReader>) -> io::Result<Self> {
        let inner = PstFileInner::read_from(reader)?;
        Ok(Self { inner })
    }

    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let inner = PstFileInner::open(path)?;
        Ok(Self { inner })
    }

    /// Create a new Unicode PST containing Inbox and one message.
    pub fn create(path: impl AsRef<Path>, message: &UnicodePstMessage<'_>) -> io::Result<Self> {
        create_unicode_pst(path.as_ref(), message, &[], None)?;
        Self::open(path)
    }

    /// Create a new Unicode PST containing one message and its by-value attachments.
    pub fn create_with_attachments(
        path: impl AsRef<Path>,
        message: &UnicodePstMessage<'_>,
        attachments: &[UnicodePstAttachment<'_>],
    ) -> io::Result<Self> {
        create_unicode_pst(path.as_ref(), message, attachments, None)?;
        Self::open(path)
    }

    /// Create a new Unicode PST and place its first message in `folder_path`, relative to the
    /// IPM subtree. Missing folders are created; for example `&["Projects", "2026"]`.
    pub fn create_in_folder(
        path: impl AsRef<Path>,
        folder_path: &[&str],
        message: &UnicodePstMessage<'_>,
    ) -> io::Result<Self> {
        create_unicode_pst(path.as_ref(), message, &[], Some(folder_path))?;
        Self::open(path)
    }

    /// Create a new Unicode PST and place its first message and attachments in `folder_path`.
    pub fn create_in_folder_with_attachments(
        path: impl AsRef<Path>,
        folder_path: &[&str],
        message: &UnicodePstMessage<'_>,
        attachments: &[UnicodePstAttachment<'_>],
    ) -> io::Result<Self> {
        create_unicode_pst(path.as_ref(), message, attachments, Some(folder_path))?;
        Self::open(path)
    }

    /// MailCraft: create a new Unicode PST with an empty Inbox and no message, for a file whose
    /// first items go in folders made with a container class of their own
    /// ([`UnicodePstFile::create_folder`]).
    pub fn create_empty(path: impl AsRef<Path>) -> io::Result<Self> {
        create_empty_unicode_pst(path.as_ref())?;
        Self::open(path)
    }

    /// Set the store display name (PR_DISPLAY_NAME of the message store) of an existing
    /// Unicode PST. Outlook shows it as the top-level name when the file is opened.
    pub fn set_store_display_name(path: impl AsRef<Path>, name: &str) -> io::Result<Self> {
        expect_format(path.as_ref(), Format::Unicode)?;
        set_unicode_store_display_name(path.as_ref(), name)?;
        Self::open(path)
    }

    /// Append one message to the default receive folder (normally Inbox) of an
    /// existing Unicode PST, unencoded or permute- or cyclic-encoded.
    pub fn append(path: impl AsRef<Path>, message: &UnicodePstMessage<'_>) -> io::Result<Self> {
        expect_format(path.as_ref(), Format::Unicode)?;
        append_unicode_pst(
            path.as_ref(),
            &[UnicodePstBatchMessage {
                message: *message,
                attachments: &[],
            }],
            None,
        )?;
        Self::open(path)
    }

    /// Append one message and its by-value attachments to the default receive folder of an
    /// existing Unicode PST, unencoded or permute- or cyclic-encoded.
    pub fn append_with_attachments(
        path: impl AsRef<Path>,
        message: &UnicodePstMessage<'_>,
        attachments: &[UnicodePstAttachment<'_>],
    ) -> io::Result<Self> {
        expect_format(path.as_ref(), Format::Unicode)?;
        append_unicode_pst(
            path.as_ref(),
            &[UnicodePstBatchMessage {
                message: *message,
                attachments,
            }],
            None,
        )?;
        Self::open(path)
    }

    /// Append messages to the default receive folder in one bulk commit.
    pub fn append_many(
        path: impl AsRef<Path>,
        messages: &[UnicodePstMessage<'_>],
    ) -> io::Result<Self> {
        let messages = messages
            .iter()
            .copied()
            .map(|message| UnicodePstBatchMessage {
                message,
                attachments: &[],
            })
            .collect::<Vec<_>>();
        expect_format(path.as_ref(), Format::Unicode)?;
        append_unicode_pst(path.as_ref(), &messages, None)?;
        Self::open(path)
    }

    /// Append messages and attachments to the default receive folder in one bulk commit. Message
    /// and attachment blocks are written incrementally instead of retained for the whole batch.
    pub fn append_many_with_attachments(
        path: impl AsRef<Path>,
        messages: &[UnicodePstBatchMessage<'_>],
    ) -> io::Result<Self> {
        expect_format(path.as_ref(), Format::Unicode)?;
        append_unicode_pst(path.as_ref(), messages, None)?;
        Self::open(path)
    }

    /// Append one message to `folder_path`, relative to the IPM subtree. Missing folders are
    /// created and existing folder contents are preserved.
    pub fn append_in_folder(
        path: impl AsRef<Path>,
        folder_path: &[&str],
        message: &UnicodePstMessage<'_>,
    ) -> io::Result<Self> {
        expect_format(path.as_ref(), Format::Unicode)?;
        append_unicode_pst(
            path.as_ref(),
            &[UnicodePstBatchMessage {
                message: *message,
                attachments: &[],
            }],
            Some(folder_path),
        )?;
        Self::open(path)
    }

    /// Append one message and its attachments to `folder_path`, creating missing folders.
    pub fn append_in_folder_with_attachments(
        path: impl AsRef<Path>,
        folder_path: &[&str],
        message: &UnicodePstMessage<'_>,
        attachments: &[UnicodePstAttachment<'_>],
    ) -> io::Result<Self> {
        expect_format(path.as_ref(), Format::Unicode)?;
        append_unicode_pst(
            path.as_ref(),
            &[UnicodePstBatchMessage {
                message: *message,
                attachments,
            }],
            Some(folder_path),
        )?;
        Self::open(path)
    }

    /// Append messages to `folder_path` in one bulk commit, creating missing folders.
    pub fn append_many_in_folder(
        path: impl AsRef<Path>,
        folder_path: &[&str],
        messages: &[UnicodePstMessage<'_>],
    ) -> io::Result<Self> {
        let messages = messages
            .iter()
            .copied()
            .map(|message| UnicodePstBatchMessage {
                message,
                attachments: &[],
            })
            .collect::<Vec<_>>();
        expect_format(path.as_ref(), Format::Unicode)?;
        append_unicode_pst(path.as_ref(), &messages, Some(folder_path))?;
        Self::open(path)
    }

    /// Append messages and attachments to `folder_path` in one bulk commit, creating missing
    /// folders. Message and attachment blocks are written incrementally.
    pub fn append_many_in_folder_with_attachments(
        path: impl AsRef<Path>,
        folder_path: &[&str],
        messages: &[UnicodePstBatchMessage<'_>],
    ) -> io::Result<Self> {
        expect_format(path.as_ref(), Format::Unicode)?;
        append_unicode_pst(path.as_ref(), messages, Some(folder_path))?;
        Self::open(path)
    }

    /// MailCraft: a message's or an attachment's value of more than `bytes` is an error when
    /// it is read, before any of it is read; a value can't be bigger than it declares anyway.
    pub fn set_max_value_size(&self, bytes: u64) {
        self.inner.max_value_size.store(bytes, Ordering::Relaxed);
    }
}

impl PstFileLock<UnicodePstFile> for UnicodePstFile {
    fn start_write(&mut self) -> io::Result<()> {
        self.inner.start_write()
    }

    fn finish_write(&mut self) -> io::Result<()> {
        self.inner.finish_write()
    }

    fn block_cache(&self) -> RefMut<'_, RootBTreePageCache<<Self as PstFile>::BlockBTree>> {
        self.inner.block_cache.borrow_mut()
    }

    fn node_cache(&self) -> RefMut<'_, RootBTreePageCache<<Self as PstFile>::NodeBTree>> {
        self.inner.node_cache.borrow_mut()
    }
}

impl PstFile for UnicodePstFile {
    type BlockId = UnicodeBlockId;
    type PageId = UnicodePageId;
    type ByteIndex = UnicodeByteIndex;
    type BlockRef = UnicodeBlockRef;
    type PageRef = UnicodePageRef;
    type Root = UnicodeRoot;
    type Header = UnicodeHeader;
    type PageTrailer = UnicodePageTrailer;
    type BTreeKey = u64;
    type NodeBTreeEntry = UnicodeNodeBTreeEntry;
    type NodeBTree = UnicodeNodeBTree;
    type BlockBTreeEntry = UnicodeBlockBTreeEntry;
    type BlockBTree = UnicodeBlockBTree;
    type BlockTrailer = UnicodeBlockTrailer;
    type AllocationMapPage = UnicodeMapPage<{ PageType::AllocationMap as u8 }>;
    type AllocationPageMapPage = UnicodeMapPage<{ PageType::AllocationPageMap as u8 }>;
    type FreeMapPage = UnicodeMapPage<{ PageType::FreeMap as u8 }>;
    type FreePageMapPage = UnicodeMapPage<{ PageType::FreePageMap as u8 }>;
    type DensityListPage = UnicodeDensityListPage;
    type DataTreeEntry = UnicodeDataTreeEntry;
    type DataTreeBlock = UnicodeDataTreeBlock;
    type DataBlock = UnicodeDataBlock;
    type SubNodeTreeBlockHeader = UnicodeSubNodeTreeBlockHeader;
    type SubNodeTreeBlock = UnicodeIntermediateSubNodeTreeBlock;
    type SubNodeBlock = UnicodeLeafSubNodeTreeBlock;
    type HeapNode = UnicodeHeapNode;
    type PropertyTree = UnicodeHeapTree<PropertyTreeRecordKey, PropertyTreeRecordValue>;
    type TableContext = UnicodeTableContext;
    type PropertyContext = UnicodePropertyContext;
    type Store = UnicodeStore;
    type Folder = UnicodeFolder;
    type Message = UnicodeMessage;
    type NamedPropertyMap = UnicodeNamedPropertyMap;
    type SearchUpdateQueue = UnicodeSearchUpdateQueue;

    fn header(&self) -> &Self::Header {
        &self.inner.header
    }

    fn density_list(&self) -> Result<&dyn DensityListPage<Self>, &io::Error> {
        self.inner.density_list.as_ref().map(|dl| dl as _)
    }

    fn reader(&self) -> &Mutex<Box<dyn PstReader>> {
        &self.inner.reader
    }

    fn lock(&mut self) -> io::Result<PstFileLockGuard<'_, Self>> {
        PstFileLockGuard::new(self)
    }

    fn read_node(&self, node: NodeId) -> io::Result<UnicodeNodeBTreeEntry> {
        self.inner.read_node(node)
    }

    fn read_block(&self, block: UnicodeBlockId) -> io::Result<Vec<u8>> {
        self.inner.read_block(block)
    }

    fn max_value_size(&self) -> u64 {
        self.inner.max_value_size.load(Ordering::Relaxed)
    }
}

pub struct AnsiPstFile {
    inner: PstFileInner<Self>,
}

impl AnsiPstFile {
    pub fn read_from(reader: Box<dyn PstReader>) -> io::Result<Self> {
        let inner = PstFileInner::read_from(reader)?;
        Ok(Self { inner })
    }

    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let inner = PstFileInner::open(path)?;
        Ok(Self { inner })
    }

    /// MailCraft: a message's or an attachment's value of more than `bytes` is an error when
    /// it is read, before any of it is read; a value can't be bigger than it declares anyway.
    pub fn set_max_value_size(&self, bytes: u64) {
        self.inner.max_value_size.store(bytes, Ordering::Relaxed);
    }
}

impl PstFileLock<AnsiPstFile> for AnsiPstFile {
    fn start_write(&mut self) -> io::Result<()> {
        self.inner.start_write()
    }

    fn finish_write(&mut self) -> io::Result<()> {
        self.inner.finish_write()
    }

    fn block_cache(&self) -> RefMut<'_, RootBTreePageCache<<Self as PstFile>::BlockBTree>> {
        self.inner.block_cache.borrow_mut()
    }

    fn node_cache(&self) -> RefMut<'_, RootBTreePageCache<<Self as PstFile>::NodeBTree>> {
        self.inner.node_cache.borrow_mut()
    }
}

impl PstFile for AnsiPstFile {
    type BlockId = AnsiBlockId;
    type PageId = AnsiPageId;
    type ByteIndex = AnsiByteIndex;
    type BlockRef = AnsiBlockRef;
    type PageRef = AnsiPageRef;
    type Root = AnsiRoot;
    type Header = AnsiHeader;
    type PageTrailer = AnsiPageTrailer;
    type BTreeKey = u32;
    type NodeBTreeEntry = AnsiNodeBTreeEntry;
    type NodeBTree = AnsiNodeBTree;
    type BlockBTreeEntry = AnsiBlockBTreeEntry;
    type BlockBTree = AnsiBlockBTree;
    type BlockTrailer = AnsiBlockTrailer;
    type AllocationMapPage = AnsiMapPage<{ PageType::AllocationMap as u8 }>;
    type AllocationPageMapPage = AnsiMapPage<{ PageType::AllocationPageMap as u8 }>;
    type FreeMapPage = AnsiMapPage<{ PageType::FreeMap as u8 }>;
    type FreePageMapPage = AnsiMapPage<{ PageType::FreePageMap as u8 }>;
    type DensityListPage = AnsiDensityListPage;
    type DataTreeEntry = AnsiDataTreeEntry;
    type DataTreeBlock = AnsiDataTreeBlock;
    type DataBlock = AnsiDataBlock;
    type SubNodeTreeBlockHeader = AnsiSubNodeTreeBlockHeader;
    type SubNodeTreeBlock = AnsiIntermediateSubNodeTreeBlock;
    type SubNodeBlock = AnsiLeafSubNodeTreeBlock;
    type HeapNode = AnsiHeapNode;
    type PropertyTree = AnsiHeapTree<PropertyTreeRecordKey, PropertyTreeRecordValue>;
    type TableContext = AnsiTableContext;
    type PropertyContext = AnsiPropertyContext;
    type Store = AnsiStore;
    type Folder = AnsiFolder;
    type Message = AnsiMessage;
    type NamedPropertyMap = AnsiNamedPropertyMap;
    type SearchUpdateQueue = AnsiSearchUpdateQueue;

    fn header(&self) -> &Self::Header {
        &self.inner.header
    }

    fn density_list(&self) -> Result<&dyn DensityListPage<Self>, &io::Error> {
        self.inner.density_list.as_ref().map(|dl| dl as _)
    }

    fn reader(&self) -> &Mutex<Box<dyn PstReader>> {
        &self.inner.reader
    }

    fn lock(&mut self) -> io::Result<PstFileLockGuard<'_, Self>> {
        PstFileLockGuard::new(self)
    }

    fn read_node(&self, node: NodeId) -> io::Result<AnsiNodeBTreeEntry> {
        self.inner.read_node(node)
    }

    fn read_block(&self, block: AnsiBlockId) -> io::Result<Vec<u8>> {
        self.inner.read_block(block)
    }

    fn max_value_size(&self) -> u64 {
        self.inner.max_value_size.load(Ordering::Relaxed)
    }
}

const AMAP_FIRST_OFFSET: u64 = 0x4400;
const AMAP_DATA_SIZE: u64 = size_of::<MapBits>() as u64 * 8 * 64;

const PMAP_FIRST_OFFSET: u64 = AMAP_FIRST_OFFSET + PAGE_SIZE as u64;
const PMAP_PAGE_COUNT: u64 = 8;
const PMAP_DATA_SIZE: u64 = AMAP_DATA_SIZE * PMAP_PAGE_COUNT;

const FMAP_FIRST_SIZE: u64 = 128;
const FMAP_FIRST_DATA_SIZE: u64 = AMAP_DATA_SIZE * FMAP_FIRST_SIZE;
const FMAP_FIRST_OFFSET: u64 = AMAP_FIRST_OFFSET + FMAP_FIRST_DATA_SIZE + (2 * PAGE_SIZE) as u64;
const FMAP_PAGE_COUNT: u64 = size_of::<MapBits>() as u64;
const FMAP_DATA_SIZE: u64 = AMAP_DATA_SIZE * FMAP_PAGE_COUNT;

const FPMAP_FIRST_SIZE: u64 = 128 * 64;
const FPMAP_FIRST_DATA_SIZE: u64 = AMAP_DATA_SIZE * FPMAP_FIRST_SIZE;
const FPMAP_FIRST_OFFSET: u64 = AMAP_FIRST_OFFSET + FPMAP_FIRST_DATA_SIZE + (2 * PAGE_SIZE) as u64;
const FPMAP_PAGE_COUNT: u64 = size_of::<MapBits>() as u64 * 64;
const FPMAP_DATA_SIZE: u64 = AMAP_DATA_SIZE * FPMAP_PAGE_COUNT;

const MAX_DATA_BLOCK_SIZE: usize =
    (ndb::block::MAX_BLOCK_SIZE - UnicodeBlockTrailer::SIZE) as usize;
// MS-PST limit for one HN allocation. Outlook never writes above it; SCANPST stops on padding above it (Fatal Error 80040813). Larger values go to subnodes and BTH nodes are split to stay below it.
const MAX_HN_ALLOCATION_SIZE: usize = 3580;

const OUTLOOK_UNICODE_PST_TEMPLATE: &[u8] = include_bytes!("../examples/Empty.pst");

fn heap_id(allocation_index: usize) -> io::Result<HeapId> {
    Ok(HeapId::new(
        u16::try_from(allocation_index).map_err(|_| PstError::IntegerConversion)?,
        0,
    )?)
}

fn utf16_bytes(value: &str) -> Vec<u8> {
    value
        .encode_utf16()
        .chain(Some(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

fn finish_heap(
    mut allocations: Vec<Vec<u8>>,
    client_signature: HeapNodeType,
    user_root: HeapId,
) -> io::Result<Vec<u8>> {
    const HEAP_HEADER_SIZE: usize = 12;
    let page_map_offset =
        (allocations.iter().map(Vec::len).sum::<usize>() + HEAP_HEADER_SIZE).next_multiple_of(2);
    let page_map_offset =
        u16::try_from(page_map_offset).map_err(|_| PstError::IntegerConversion)?;
    let header = HeapNodeHeader::new(
        page_map_offset,
        client_signature,
        user_root,
        [HeapFillLevel::Empty; 8],
    );
    header.write(&mut allocations[0])?;

    let mut data = allocations.remove(0);
    let mut offsets = Vec::with_capacity(allocations.len() + 1);
    offsets.push(u16::try_from(data.len()).map_err(|_| PstError::IntegerConversion)?);
    for allocation in allocations {
        data.extend_from_slice(&allocation);
        offsets.push(u16::try_from(data.len()).map_err(|_| PstError::IntegerConversion)?);
    }

    data.resize(usize::from(page_map_offset), 0);
    data.write_u16::<byteorder::LittleEndian>(
        u16::try_from(offsets.len() - 1).map_err(|_| PstError::IntegerConversion)?,
    )?;
    data.write_u16::<byteorder::LittleEndian>(0)?;
    for offset in offsets {
        data.write_u16::<byteorder::LittleEndian>(offset)?;
    }
    Ok(data)
}

struct HeapBuilder {
    pages: Vec<Vec<Vec<u8>>>,
    /// Pages whose encoding changed since `from_pages`; `finish_dirty` re-encodes only these.
    dirty: Vec<bool>,
}

impl HeapBuilder {
    fn new() -> Self {
        Self {
            pages: vec![vec![Vec::new()]],
            dirty: vec![true],
        }
    }

    fn from_pages(pages: Vec<Vec<Vec<u8>>>) -> Self {
        let dirty = vec![false; pages.len()];
        Self { pages, dirty }
    }

    fn slot(&self, heap_id: HeapId) -> io::Result<(usize, usize)> {
        let page = usize::from(heap_id.block_index());
        let index = usize::from(heap_id.index()?) + 1;
        if self.pages.get(page).is_none_or(|page| index >= page.len()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "heap allocation is missing",
            ));
        }
        Ok((page, index))
    }

    fn get(&self, heap_id: HeapId) -> io::Result<&[u8]> {
        let (page, index) = self.slot(heap_id)?;
        Ok(&self.pages[page][index])
    }

    /// Overwrite an allocation in place, keeping its HID. Callers keep the size unchanged.
    fn replace(&mut self, heap_id: HeapId, data: Vec<u8>) -> io::Result<()> {
        let (page, index) = self.slot(heap_id)?;
        self.pages[page][index] = data;
        self.dirty[page] = true;
        Ok(())
    }

    /// Free an allocation: it is encoded as a zero-length page map entry so other HIDs stay valid.
    fn free(&mut self, heap_id: HeapId) -> io::Result<Vec<u8>> {
        let (page, index) = self.slot(heap_id)?;
        self.dirty[page] = true;
        Ok(std::mem::take(&mut self.pages[page][index]))
    }

    fn header_size(page: usize) -> usize {
        if page == 0 {
            12
        } else if page % 128 == 8 {
            66
        } else {
            2
        }
    }

    fn alloc(&mut self, data: Vec<u8>) -> io::Result<HeapId> {
        if data.len() > MAX_HN_ALLOCATION_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "heap allocation exceeds one Unicode PST data block",
            ));
        }

        let mut page = self.pages.len() - 1;
        let fits = |page: usize, allocations: &[Vec<u8>]| {
            let allocation_bytes = allocations.iter().map(Vec::len).sum::<usize>() + data.len();
            let allocation_count = allocations.len() + 1;
            // Leave one byte for alignment after reserving a non-final page's padding entry.
            Self::header_size(page) + allocation_bytes + 4 + 2 * (allocation_count + 1) + 2
                < MAX_DATA_BLOCK_SIZE
        };
        if !fits(page, &self.pages[page]) {
            // The old last page becomes non-final and must be re-encoded with its padding.
            self.dirty[page] = true;
            self.pages.push(vec![Vec::new()]);
            self.dirty.push(true);
            page += 1;
            if !fits(page, &self.pages[page]) {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "heap allocation does not fit in a Unicode PST data block",
                ));
            }
        }

        let allocation = self.pages[page].len();
        let heap_id = HeapId::new(
            u16::try_from(allocation).map_err(|_| PstError::IntegerConversion)?,
            u16::try_from(page).map_err(|_| PstError::IntegerConversion)?,
        )?;
        self.pages[page].push(data);
        self.dirty[page] = true;
        Ok(heap_id)
    }

    fn finish(
        mut self,
        client_signature: HeapNodeType,
        user_root: HeapId,
    ) -> io::Result<Vec<Vec<u8>>> {
        self.dirty.fill(true);
        self.finish_dirty(client_signature, user_root)?
            .into_iter()
            .map(|page| page.ok_or_else(|| io::Error::other("heap page was not encoded")))
            .collect()
    }

    /// Encode the dirty pages; `None` marks a page whose existing block can be reused as is.
    fn finish_dirty(
        mut self,
        client_signature: HeapNodeType,
        user_root: HeapId,
    ) -> io::Result<Vec<Option<Vec<u8>>>> {
        let page_count = self.pages.len();
        let dirty = std::mem::take(&mut self.dirty);
        self.pages
            .into_iter()
            .zip(dirty)
            .enumerate()
            .map(|(page, (mut allocations, dirty))| {
                if !dirty {
                    return Ok(None);
                }
                let header_size = Self::header_size(page);
                if page + 1 < page_count {
                    let allocations_end =
                        header_size + allocations.iter().skip(1).map(Vec::len).sum::<usize>();
                    let encoded_size = allocations_end + 4 + 2 * allocations.len();
                    if encoded_size < MAX_DATA_BLOCK_SIZE {
                        // Outlook rejects a heap whose padding is one allocation over the HN limit,
                        // so split it; each extra allocation costs a 2-byte page map entry.
                        let free = MAX_DATA_BLOCK_SIZE - encoded_size;
                        let count = free.div_ceil(MAX_HN_ALLOCATION_SIZE + 2);
                        let bytes = free.checked_sub(2 * count).ok_or_else(|| {
                            io::Error::new(
                                io::ErrorKind::InvalidData,
                                "heap page has no room for a padding allocation",
                            )
                        })?;
                        for index in 0..count {
                            let size = bytes / count + usize::from(index < bytes % count);
                            allocations.push(vec![0; size]);
                        }
                    } else if encoded_size > MAX_DATA_BLOCK_SIZE {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "encoded heap page exceeds one Unicode PST data block",
                        ));
                    }
                }
                let page_map_offset = (header_size
                    + allocations.iter().skip(1).map(Vec::len).sum::<usize>())
                .next_multiple_of(2);
                let page_map_offset =
                    u16::try_from(page_map_offset).map_err(|_| PstError::IntegerConversion)?;
                let mut header = Vec::with_capacity(header_size);
                if page == 0 {
                    HeapNodeHeader::new(
                        page_map_offset,
                        client_signature,
                        user_root,
                        [HeapFillLevel::Empty; 8],
                    )
                    .write(&mut header)?;
                } else if page % 128 == 8 {
                    HeapNodeBitmapHeader::new(page_map_offset, [HeapFillLevel::Empty; 128])
                        .write(&mut header)?;
                } else {
                    HeapNodePageHeader::new(page_map_offset).write(&mut header)?;
                }
                allocations[0] = header;

                let mut data = allocations.remove(0);
                let mut offsets = Vec::with_capacity(allocations.len() + 1);
                offsets.push(u16::try_from(data.len()).map_err(|_| PstError::IntegerConversion)?);
                for allocation in allocations {
                    data.extend_from_slice(&allocation);
                    offsets
                        .push(u16::try_from(data.len()).map_err(|_| PstError::IntegerConversion)?);
                }
                data.resize(usize::from(page_map_offset), 0);
                data.write_u16::<byteorder::LittleEndian>(
                    u16::try_from(offsets.len() - 1).map_err(|_| PstError::IntegerConversion)?,
                )?;
                let free_count = offsets
                    .windows(2)
                    .filter(|offsets| offsets[0] == offsets[1])
                    .count();
                data.write_u16::<byteorder::LittleEndian>(
                    u16::try_from(free_count).map_err(|_| PstError::IntegerConversion)?,
                )?;
                for offset in offsets {
                    data.write_u16::<byteorder::LittleEndian>(offset)?;
                }
                if data.len() > MAX_DATA_BLOCK_SIZE {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "encoded heap page exceeds one Unicode PST data block",
                    ));
                }
                Ok(Some(data))
            })
            .collect()
    }
}

fn property_value_bytes(value: &PropertyValue) -> io::Result<Vec<u8>> {
    let mut data = Vec::new();
    value.write(&mut data)?;
    match value {
        PropertyValue::String8(_) => data.push(0),
        PropertyValue::Unicode(_) => data.extend_from_slice(&[0, 0]),
        _ => {}
    }
    Ok(data)
}

fn small_property_record(value: &PropertyValue) -> Option<u32> {
    match value {
        PropertyValue::Integer16(value) => Some(u32::from(*value as u16)),
        PropertyValue::Integer32(value) | PropertyValue::ErrorCode(value) => Some(*value as u32),
        PropertyValue::Floating32(value) => Some(value.to_bits()),
        PropertyValue::Boolean(value) => Some(u32::from(*value)),
        _ => None,
    }
}

type RebuiltPropertyContext = (Vec<Vec<u8>>, Vec<(NodeId, Vec<u8>)>);

fn build_property_tree(
    heap: &mut HeapBuilder,
    records: &[(u16, PropertyType, PropertyValueRecord)],
) -> io::Result<HeapId> {
    let mut children = records
        .chunks(MAX_HN_ALLOCATION_SIZE / 8)
        .map(|chunk| {
            let mut data = Vec::with_capacity(chunk.len() * 8);
            for (id, prop_type, value) in chunk {
                PropertyTreeRecord::new(*id, *prop_type, *value).write(&mut data)?;
            }
            Ok((chunk[0].0, heap.alloc(data)?))
        })
        .collect::<io::Result<Vec<_>>>()?;
    let mut levels = 0;
    while children.len() > 1 {
        children = children
            .chunks(MAX_HN_ALLOCATION_SIZE / 6)
            .map(|chunk| {
                let mut data = Vec::with_capacity(chunk.len() * 6);
                for (key, child) in chunk {
                    HeapTreeIntermediateEntry::new(*key, *child).write(&mut data)?;
                }
                Ok((chunk[0].0, heap.alloc(data)?))
            })
            .collect::<io::Result<Vec<_>>>()?;
        levels += 1;
    }
    let root = children.first().map(|entry| entry.1).unwrap_or_default();
    let mut header = Vec::new();
    HeapTreeHeader::new(2, 6, levels, root)?.write(&mut header)?;
    heap.alloc(header)
}

/// A property context's heap pages and the values kept in subnodes of its own. MailCraft: a
/// string is written as `text` says (8-bit in an ANSI file, MS-PST 2.5).
fn property_context_pages(
    properties: impl IntoIterator<Item = (u16, PropertyValue)>,
    mut next_subnode_index: u32,
    text: Text,
) -> io::Result<RebuiltPropertyContext> {
    let mut heap = HeapBuilder::new();
    let mut external = Vec::new();
    let mut records = properties
        .into_iter()
        .filter(|(_, value)| !matches!(value, PropertyValue::Null))
        .map(|(id, value)| {
            let value = text.narrow(value);
            let prop_type = PropertyType::from(&value);
            let record = if let Some(small) = small_property_record(&value) {
                PropertyValueRecord::Small(small)
            } else {
                let data = property_value_bytes(&value)?;
                if data.is_empty() {
                    PropertyValueRecord::Heap(HeapId::default())
                } else if data.len() > MAX_HN_ALLOCATION_SIZE {
                    let node = NodeId::new(NodeIdType::Internal, next_subnode_index)?;
                    next_subnode_index = next_subnode_index
                        .checked_add(1)
                        .ok_or(PstError::IntegerConversion)?;
                    external.push((node, data));
                    PropertyValueRecord::Node(node)
                } else {
                    PropertyValueRecord::Heap(heap.alloc(data)?)
                }
            };
            Ok((id, prop_type, record))
        })
        .collect::<io::Result<Vec<_>>>()?;
    records.sort_by_key(|record| record.0);
    let user_root = build_property_tree(&mut heap, &records)?;
    Ok((heap.finish(HeapNodeType::Properties, user_root)?, external))
}

enum TableCell {
    Value(PropertyValue),
    Bytes(Vec<u8>),
    Node(NodeId),
}

type RebuiltTable = (Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeId>);

fn table_pointer_type(prop_type: PropertyType) -> bool {
    matches!(
        prop_type,
        PropertyType::String8
            | PropertyType::Unicode
            | PropertyType::Guid
            | PropertyType::Binary
            | PropertyType::Object
            | PropertyType::MultipleInteger16
            | PropertyType::MultipleInteger32
            | PropertyType::MultipleFloating32
            | PropertyType::MultipleFloating64
            | PropertyType::MultipleCurrency
            | PropertyType::MultipleFloatingTime
            | PropertyType::MultipleInteger64
            | PropertyType::MultipleString8
            | PropertyType::MultipleUnicode
            | PropertyType::MultipleTime
            | PropertyType::MultipleGuid
            | PropertyType::MultipleBinary
    )
}

fn write_fixed_table_value(
    mut target: &mut [u8],
    prop_type: PropertyType,
    value: &PropertyValue,
) -> io::Result<()> {
    match (prop_type, value) {
        (PropertyType::Integer16, PropertyValue::Integer16(value)) => {
            target.write_i16::<byteorder::LittleEndian>(*value)
        }
        (PropertyType::Integer32, PropertyValue::Integer32(value)) => {
            target.write_i32::<byteorder::LittleEndian>(*value)
        }
        (PropertyType::Floating32, PropertyValue::Floating32(value)) => {
            target.write_f32::<byteorder::LittleEndian>(*value)
        }
        (PropertyType::Floating64, PropertyValue::Floating64(value)) => {
            target.write_f64::<byteorder::LittleEndian>(*value)
        }
        (PropertyType::Currency, PropertyValue::Currency(value)) => {
            target.write_i64::<byteorder::LittleEndian>(*value)
        }
        (PropertyType::FloatingTime, PropertyValue::FloatingTime(value)) => {
            target.write_f64::<byteorder::LittleEndian>(*value)
        }
        (PropertyType::ErrorCode, PropertyValue::ErrorCode(value)) => {
            target.write_i32::<byteorder::LittleEndian>(*value)
        }
        (PropertyType::Boolean, PropertyValue::Boolean(value)) => target.write_u8(u8::from(*value)),
        (PropertyType::Integer64, PropertyValue::Integer64(value)) => {
            target.write_i64::<byteorder::LittleEndian>(*value)
        }
        (PropertyType::Time, PropertyValue::Time(value)) => {
            target.write_i64::<byteorder::LittleEndian>(*value)
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("table value does not match {prop_type:?}"),
        )),
    }
}

fn encode_table_row(
    context: &TableContextInfo,
    heap: &mut HeapBuilder,
    row_id: impl Into<u32>,
    unique: u32,
    cells: Vec<Option<TableCell>>,
) -> io::Result<Vec<u8>> {
    let mut row = vec![0; usize::from(context.end_existence_bitmap())];
    // MailCraft: a row too short for its ID and version is an error, not a panic.
    if row.len() < 8 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "table rows are too short for their ID and version",
        ));
    }
    (&mut row[0..4]).write_u32::<byteorder::LittleEndian>(row_id.into())?;
    (&mut row[4..8]).write_u32::<byteorder::LittleEndian>(unique)?;

    for (column, cell) in context.columns().iter().zip(cells) {
        let Some(cell) = cell else { continue };
        // MailCraft: a column the file places past the row is an error, not a panic.
        let outside = || {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "table column lies outside its row",
            )
        };
        let bit = usize::from(column.existence_bitmap_index());
        *row.get_mut(usize::from(context.end_1byte_values()) + bit / 8)
            .ok_or_else(outside)? |= 0x80 >> (bit % 8);

        let offset = usize::from(column.offset());
        let size = usize::from(column.size());
        let mut target = row.get_mut(offset..offset + size).ok_or_else(outside)?;
        if table_pointer_type(column.prop_type()) {
            let value = match cell {
                TableCell::Node(node) => u32::from(node),
                TableCell::Bytes(data) => u32::from(heap.alloc(data)?),
                TableCell::Value(value) => u32::from(heap.alloc(property_value_bytes(&value)?)?),
            };
            target.write_u32::<byteorder::LittleEndian>(value)?;
        } else {
            let TableCell::Value(value) = cell else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "fixed-width table column has an indirect value",
                ));
            };
            write_fixed_table_value(target, column.prop_type(), &value)?;
        }
    }
    Ok(row)
}

fn build_row_index<T>(heap: &mut HeapBuilder, rows: &[T], format: Format) -> io::Result<HeapId>
where
    T: Copy,
    u32: From<T>,
{
    let children = rows
        .chunks(ROW_INDEX_FANOUT)
        .enumerate()
        .map(|(chunk_index, chunk)| {
            let mut data = Vec::with_capacity(chunk.len() * row_index_record_size(format));
            for (index, row) in chunk.iter().enumerate() {
                TableRowId::new(u32::from(*row)).write(&mut data)?;
                write_row_number(&mut data, chunk_index * ROW_INDEX_FANOUT + index, format)?;
            }
            Ok((u32::from(chunk[0]), heap.alloc(data)?))
        })
        .collect::<io::Result<Vec<_>>>()?;
    let header = row_index_header(heap, children, 0, format)?;
    heap.alloc(header)
}

const ROW_INDEX_FANOUT: usize = MAX_HN_ALLOCATION_SIZE / 8;

/// MailCraft: the bytes of a row index's leaf record: the row ID and its row number, 32 bits in
/// a Unicode file and 16 in an ANSI one (MS-PST 2.3.4.3.1).
fn row_index_record_size(format: Format) -> usize {
    4 + usize::from(format.row_index_size())
}

/// MailCraft: a row's number in the row matrix, as the file's format keeps it. An ANSI file's
/// table holds 2^16 rows at most.
fn write_row_number(data: &mut Vec<u8>, number: usize, format: Format) -> io::Result<()> {
    match format {
        Format::Unicode => data.write_u32::<byteorder::LittleEndian>(
            u32::try_from(number).map_err(|_| PstError::IntegerConversion)?,
        ),
        Format::Ansi => data.write_u16::<byteorder::LittleEndian>(u16::try_from(number).map_err(
            |_| {
                io::Error::other("a folder of an ANSI data file (Outlook 97–2002's format) holds 65,536 items at most")
            },
        )?),
    }
}

/// Stack intermediate levels over `children` (`(first row id, node)`) and encode the BTH header.
fn row_index_header(
    heap: &mut HeapBuilder,
    mut children: Vec<(u32, HeapId)>,
    mut levels: u8,
    format: Format,
) -> io::Result<Vec<u8>> {
    while children.len() > 1 {
        children = children
            .chunks(ROW_INDEX_FANOUT)
            .map(|chunk| {
                let mut data = Vec::with_capacity(chunk.len() * 8);
                for (key, child) in chunk {
                    HeapTreeIntermediateEntry::new(TableRowId::new(*key), *child)
                        .write(&mut data)?;
                }
                Ok((chunk[0].0, heap.alloc(data)?))
            })
            .collect::<io::Result<Vec<_>>>()?;
        levels += 1;
    }
    let root = children.first().map(|entry| entry.1).unwrap_or_default();
    let mut header = Vec::new();
    let entry_size = format.row_index_size();
    HeapTreeHeader::new(4, entry_size, levels, root)?.write(&mut header)?;
    Ok(header)
}

/// Free the right spine of a row-index BTH, root first, returning each node without its last
/// (rewritten) child entry. Freeing before new rows are encoded lets them reuse the space.
fn take_row_index_spine(
    heap: &mut HeapBuilder,
    index: &HeapTreeHeader,
    format: Format,
) -> io::Result<Vec<Vec<u8>>> {
    let mut node = index.root();
    let mut spine = Vec::new();
    for level in (0..=index.levels()).rev() {
        let mut data = heap.free(node)?;
        // MailCraft: a leaf's records are as big as the file's format makes them.
        let record = if level > 0 {
            8
        } else {
            row_index_record_size(format)
        };
        if data.is_empty() || data.len() % record != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "table row index node is malformed",
            ));
        }
        if level > 0 {
            let last = data.len() - 8;
            // MailCraft: read without `unwrap`; the node's length is a multiple of 8 above.
            let mut child = [0; 4];
            child.copy_from_slice(data.get(last + 4..last + 8).unwrap_or(&[0; 4]));
            node = HeapId::from(u32::from_le_bytes(child));
            data.truncate(last);
        }
        spine.push(data);
    }
    Ok(spine)
}

/// Re-allocate a spine from `take_row_index_spine` with `(row id, row index)` records appended
/// and return the new BTH header.
fn append_row_index_spine(
    heap: &mut HeapBuilder,
    mut spine: Vec<Vec<u8>>,
    levels: u8,
    records: &[(u32, u32)],
    format: Format,
) -> io::Result<Vec<u8>> {
    let mut children: Vec<(u32, HeapId)> = Vec::new();
    let mut leaf = true;
    while let Some(mut data) = spine.pop() {
        let record = if leaf {
            for (key, value) in records {
                data.extend_from_slice(&key.to_le_bytes());
                let value = usize::try_from(*value).map_err(|_| PstError::IntegerConversion)?;
                write_row_number(&mut data, value, format)?;
            }
            leaf = false;
            row_index_record_size(format)
        } else {
            8
        };
        for (key, child) in children {
            data.extend_from_slice(&key.to_le_bytes());
            data.extend_from_slice(&u32::from(child).to_le_bytes());
        }
        children = data
            .chunks(ROW_INDEX_FANOUT * record)
            .map(|chunk| {
                // MailCraft: without `unwrap`; every chunk holds whole records.
                let mut key = [0; 4];
                key.copy_from_slice(chunk.get(..4).unwrap_or(&[0; 4]));
                let key = u32::from_le_bytes(key);
                Ok((key, heap.alloc(chunk.to_vec())?))
            })
            .collect::<io::Result<Vec<_>>>()?;
    }
    row_index_header(heap, children, levels, format)
}

/// Overwrite fixed-width cells of one row in a subnode row matrix, bumping its row version. Only
/// the row's data block and the XBLOCKs are rewritten and the heap is reused as is. Returns `None`
/// when the row matrix lives in the heap or a cell does not fit, so callers fall back to a rebuild.
#[allow(clippy::too_many_arguments)]
fn patch_table_row(
    file: &mut File,
    encoding: NdbCryptMethod,
    bbt_entries: &[UnicodeBlockBTreeEntry],
    table: &dyn TableContext,
    rows_block: Option<UnicodeBlockId>,
    row: NodeId,
    cells: &[(u16, PropertyValue)],
    row_version: u32,
    blocks: &mut Vec<NewBlock>,
    next_bid: &mut u64,
    next_offset: &mut FileSpace,
) -> io::Result<Option<UnicodeBlockId>> {
    let context = table.context();
    let row_size = usize::from(context.end_existence_bitmap());
    let position = table
        .rows_matrix()
        .position(|candidate| u32::from(candidate.id()) == u32::from(row));
    let (Some(rows_block), Some(position)) = (rows_block, position) else {
        return Ok(None);
    };
    // MailCraft: a block holds as many rows as its format makes room for (MS-PST 2.3.4.4).
    let format = next_offset.format;
    let block_size = format.data_block_size();
    if !(8..=block_size).contains(&row_size)
        || context
            .rows()
            .is_none_or(|rows| matches!(rows.id_type(), Ok(NodeIdType::HeapNode)))
    {
        return Ok(None);
    }

    let version = PropertyValue::Integer32(row_version as i32);
    let mut patches = vec![(4, row_version.to_le_bytes().to_vec(), None)];
    for (prop_id, value) in cells.iter().chain([&(LTP_ROW_VERSION_PROP_ID, version)]) {
        let column = context.columns().iter().find(|column| {
            column.prop_id() == *prop_id && column.prop_type() == PropertyType::from(value)
        });
        let Some(column) = column.filter(|column| !table_pointer_type(column.prop_type())) else {
            if *prop_id == LTP_ROW_VERSION_PROP_ID {
                continue;
            }
            return Ok(None);
        };
        let mut bytes = vec![0; usize::from(column.size())];
        write_fixed_table_value(&mut bytes, column.prop_type(), value)?;
        patches.push((
            usize::from(column.offset()),
            bytes,
            Some(usize::from(column.existence_bitmap_index())),
        ));
    }

    let mut leaves = data_tree_leaves(file, format, encoding, bbt_entries, rows_block)?;
    let rows_per_block = block_size / row_size;
    let Some(&(leaf, _)) = leaves.get(position / rows_per_block) else {
        return Ok(None);
    };
    let mut data = read_data_block(file, format, encoding, bbt_entries, leaf)?;
    let start = position % rows_per_block * row_size;
    let Some(row_data) = data.get_mut(start..start + row_size) else {
        return Ok(None);
    };
    for (offset, bytes, bit) in patches {
        // MailCraft: a column the file places past its row falls back to the rebuild.
        let Some(cell) = row_data.get_mut(offset..offset + bytes.len()) else {
            return Ok(None);
        };
        cell.copy_from_slice(&bytes);
        if let Some(bit) = bit {
            let Some(flags) = row_data.get_mut(usize::from(context.end_1byte_values()) + bit / 8)
            else {
                return Ok(None);
            };
            *flags |= 0x80 >> (bit % 8);
        }
    }
    let size = u32::try_from(data.len()).map_err(|_| PstError::IntegerConversion)?;
    let block = push_block(
        blocks,
        next_bid,
        next_offset,
        false,
        NewBlockData::Data(data),
    )?;
    leaves[position / rows_per_block] = (block, size);
    Ok(Some(push_data_tree_leaves(
        blocks,
        next_bid,
        next_offset,
        leaves,
    )?))
}

/// A string's cell in a column of `prop_type`. MailCraft: an 8-bit column takes the string in
/// the code page `text` says (an ANSI file's), else in UTF-8, as before.
fn string_table_cell(prop_type: PropertyType, value: &str, text: Text) -> Option<TableCell> {
    match (prop_type, text) {
        (PropertyType::String8, Text::Ansi(_)) => Some(TableCell::Bytes(text.bytes(value))),
        (PropertyType::String8, Text::Unicode) => {
            let mut data = value.as_bytes().to_vec();
            data.push(0);
            Some(TableCell::Bytes(data))
        }
        (PropertyType::Unicode, _) => Some(TableCell::Bytes(utf16_bytes(value))),
        _ => None,
    }
}

fn recipient_display(
    recipients: &[UnicodePstRecipient<'_>],
    recipient_type: UnicodePstRecipientType,
) -> String {
    recipients
        .iter()
        .filter(|recipient| recipient.recipient_type == recipient_type)
        .map(|recipient| {
            if recipient.name.is_empty() {
                recipient.email
            } else {
                recipient.name
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn recipient_display_table_cell(
    prop_type: PropertyType,
    recipients: &[UnicodePstRecipient<'_>],
    recipient_type: UnicodePstRecipientType,
    text: Text,
) -> Option<TableCell> {
    let display = recipient_display(recipients, recipient_type);
    (!display.is_empty())
        .then(|| string_table_cell(prop_type, &display, text))
        .flatten()
}

fn message_size(
    input: &UnicodePstMessage<'_>,
    attachments: &[UnicodePstAttachment<'_>],
    named: &NamedIds,
    message: NodeId,
    row_version: u32,
    text: Text,
) -> io::Result<i32> {
    let size = message_bytes(input, attachments, named, message, row_version, text)?;
    // Outlook excludes the six-byte HN client/root header from message-owned payloads.
    i32::try_from(size.checked_sub(6).ok_or(PstError::IntegerConversion)?)
        .map_err(|_| PstError::IntegerConversion.into())
}

/// MailCraft: the bytes a message's nodes hold, what its attachments hold counted in: a file's
/// bytes, or an attached message's own nodes alike.
fn message_bytes(
    input: &UnicodePstMessage<'_>,
    attachments: &[UnicodePstAttachment<'_>],
    named: &NamedIds,
    message: NodeId,
    row_version: u32,
    text: Text,
) -> io::Result<usize> {
    let (message_pages, external) =
        message_data(input, attachments, named, message, row_version, 0, text)?;
    let (recipient_pages, recipient_rows) =
        recipient_table_data(input.recipients, row_version, text)?;
    let attachment_ids = (0..attachments.len())
        .map(|index| {
            let index = u32::try_from(index).map_err(|_| PstError::IntegerConversion)?;
            Ok::<_, io::Error>(NodeId::new(NodeIdType::Attachment, 0x100 + index)?)
        })
        .collect::<io::Result<Vec<_>>>()?;
    let contents = attachment_contents(attachments, named, row_version, text)?;
    let (attachment_pages, attachment_rows) =
        attachment_table_data(&attachment_ids, attachments, &contents, row_version, text)?;

    let mut size = message_pages
        .iter()
        .chain(recipient_pages.iter())
        .chain(attachment_pages.iter())
        .chain(external.iter().map(|(_, data)| data))
        .try_fold(0_usize, |size, data| size.checked_add(data.len()))
        .ok_or(PstError::IntegerConversion)?;
    for rows in [recipient_rows, attachment_rows].into_iter().flatten() {
        size = rows
            .1
            .iter()
            .try_fold(size, |size, data| size.checked_add(data.len()))
            .ok_or(PstError::IntegerConversion)?;
    }
    for (index, (attachment, content)) in attachments.iter().zip(&contents).enumerate() {
        let index = u32::try_from(index).map_err(|_| PstError::IntegerConversion)?;
        let (data_node, object) = match embedded(attachment) {
            Some(_) => (None, Some(sizing_node()?)),
            None => (
                (*content > 0)
                    .then(|| NodeId::new(NodeIdType::Internal, 0x200 + index))
                    .transpose()?,
                None,
            ),
        };
        size = size
            .checked_add(attachment_data(attachment, data_node, *content, object, text)?.len())
            .and_then(|size| size.checked_add(*content))
            .ok_or(PstError::IntegerConversion)?;
    }
    Ok(size)
}

/// MailCraft: the size of what each attachment holds: its file, or its attached message's
/// nodes ([`message_bytes`]).
fn attachment_contents(
    attachments: &[UnicodePstAttachment<'_>],
    named: &NamedIds,
    row_version: u32,
    text: Text,
) -> io::Result<Vec<usize>> {
    attachments
        .iter()
        .map(|attachment| match embedded(attachment) {
            Some(message) => message_bytes(
                &message.message,
                &message.attachments,
                named,
                sizing_node()?,
                row_version,
                text,
            ),
            None => file_size(attachment),
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn new_message_table_cell(
    column: &TableColumnDescriptor,
    input: &UnicodePstMessage<'_>,
    attachments: &[UnicodePstAttachment<'_>],
    named: &NamedIds,
    store_record_key: [u8; 16],
    folder: NodeId,
    message: NodeId,
    row_version: u32,
    message_size: i32,
    text: Text,
) -> io::Result<Option<TableCell>> {
    let prop_type = column.prop_type();
    // MailCraft: an added property's value, as the message has it.
    if let Some(cell) = added_table_cell(input, named, column, text)? {
        return Ok(cell);
    }
    let cell = match column.prop_id() {
        LTP_ROW_ID_PROP_ID if prop_type == PropertyType::Integer32 => Some(TableCell::Value(
            PropertyValue::Integer32(u32::from(message) as i32),
        )),
        LTP_ROW_VERSION_PROP_ID if prop_type == PropertyType::Integer32 => Some(TableCell::Value(
            PropertyValue::Integer32(row_version as i32),
        )),
        0x001A => string_table_cell(prop_type, input.message_class, text),
        0x0037 | 0x0E1D | 0x0070 | 0x3001 => string_table_cell(prop_type, input.subject, text),
        0x0042 | 0x0C1A => string_table_cell(prop_type, input.sender_name, text),
        0x0C1F => string_table_cell(prop_type, input.sender_email, text),
        0x0E02 => recipient_display_table_cell(
            prop_type,
            input.recipients,
            UnicodePstRecipientType::Bcc,
            text,
        ),
        0x0E03 => recipient_display_table_cell(
            prop_type,
            input.recipients,
            UnicodePstRecipientType::Cc,
            text,
        ),
        0x0E04 => recipient_display_table_cell(
            prop_type,
            input.recipients,
            UnicodePstRecipientType::To,
            text,
        ),
        0x1035 => string_table_cell(prop_type, input.message_id, text),
        0x0039 | 0x0E06 | 0x3008 if prop_type == PropertyType::Time => {
            Some(TableCell::Value(PropertyValue::Time(input.delivery_time)))
        }
        0x0E17 if prop_type == PropertyType::Integer32 => {
            Some(TableCell::Value(PropertyValue::Integer32(0)))
        }
        0x0E07 if prop_type == PropertyType::Integer32 => Some(TableCell::Value(
            PropertyValue::Integer32(message_flags(input, !attachments.is_empty())),
        )),
        0x0E08 if prop_type == PropertyType::Integer32 => {
            Some(TableCell::Value(PropertyValue::Integer32(message_size)))
        }
        0x0E1B if prop_type == PropertyType::Boolean => Some(TableCell::Value(
            PropertyValue::Boolean(!attachments.is_empty()),
        )),
        0x3FDE if prop_type == PropertyType::Integer32 => {
            Some(TableCell::Value(PropertyValue::Integer32(65001)))
        }
        0x0E09 | 0x0FFF if prop_type == PropertyType::Binary => {
            let node = if column.prop_id() == 0x0E09 {
                folder
            } else {
                message
            };
            let entry_id = EntryId::new(StoreRecordKey::new(store_record_key), node);
            Some(TableCell::Bytes(Vec::try_from(&entry_id)?))
        }
        0x300B if prop_type == PropertyType::Binary => {
            Some(TableCell::Bytes(input.message_id.as_bytes().to_vec()))
        }
        0x3013 if prop_type == PropertyType::Binary => None,
        _ => None,
    };
    Ok(cell)
}

fn rebuild_table_with_rows_and_version(
    table: &dyn TableContext,
    new_rows: Vec<(NodeId, Vec<Option<TableCell>>)>,
    rows_node: Option<NodeId>,
    row_version: u32,
    format: Format,
) -> io::Result<RebuiltTable> {
    rebuild_table(table, new_rows, rows_node, row_version, None, false, format)
}

fn rebuild_table(
    table: &dyn TableContext,
    new_rows: Vec<(NodeId, Vec<Option<TableCell>>)>,
    rows_node: Option<NodeId>,
    row_version: u32,
    // MailCraft: a row's cells, any number of them (a folder's content and unread counts).
    property_update: Option<(NodeId, &[(u16, PropertyValue)])>,
    preserve_existing_rows: bool,
    // MailCraft: the file's format, which says how many rows a block holds and how big a row
    // index record's row number is.
    format: Format,
) -> io::Result<RebuiltTable> {
    let block_size = format.data_block_size();
    let old_context = table.context();
    let mut heap = if preserve_existing_rows {
        HeapBuilder::from_pages(table.writer_heap_pages()?)
    } else {
        HeapBuilder::new()
    };
    let row_size = usize::from(old_context.end_existence_bitmap());
    let mut row_chunks: Vec<Vec<u8>> = Vec::new();
    let mut row_ids = Vec::new();
    let push_row = |row_chunks: &mut Vec<Vec<u8>>, row: Vec<u8>| -> io::Result<()> {
        if row_size == 0 || row_size > block_size {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "table rows do not fit in a PST data block",
            ));
        }
        if row_chunks
            .last()
            .is_none_or(|chunk| chunk.len() + row_size > block_size)
        {
            row_chunks.push(Vec::with_capacity(block_size));
        }
        row_chunks.last_mut().unwrap().extend(row);
        Ok(())
    };

    for old_row in table.rows_matrix() {
        let row_id = NodeId::from(u32::from(old_row.id()));
        if preserve_existing_rows
            && property_update
                .as_ref()
                .is_none_or(|(update_row, _)| *update_row != row_id)
        {
            push_row(&mut row_chunks, old_row.writer_data()?)?;
            row_ids.push(row_id);
            continue;
        }
        let mut cells = old_row
            .columns(old_context)?
            .into_iter()
            .zip(old_context.columns())
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
        if preserve_existing_rows {
            for (column, cell) in old_context.columns().iter().zip(&mut cells) {
                if matches!(column.prop_id(), 0x0E30 | 0x0E33 | 0x0E34) {
                    *cell = None;
                }
            }
        }
        let mut unique = old_row.unique();
        if let Some((update_row, updates)) = &property_update {
            if row_id == *update_row {
                unique = row_version;
                for (column, cell) in old_context.columns().iter().zip(&mut cells) {
                    if let Some((_, value)) = updates.iter().find(|(property, value)| {
                        column.prop_id() == *property
                            && column.prop_type() == PropertyType::from(value)
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
        }
        let row = encode_table_row(old_context, &mut heap, row_id, unique, cells)?;
        push_row(&mut row_chunks, row)?;
        row_ids.push(row_id);
    }

    for (row_id, cells) in new_rows {
        let row = encode_table_row(old_context, &mut heap, row_id, row_version, cells)?;
        push_row(&mut row_chunks, row)?;
        row_ids.push(row_id);
    }

    let row_index = build_row_index(&mut heap, &row_ids, format)?;
    let rows_node = if row_ids.is_empty() {
        None
    } else {
        Some(rows_node.ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "table rows node is missing")
        })?)
    };
    let context = TableContextInfo::new(
        old_context.end_4byte_values(),
        old_context.end_2byte_values(),
        old_context.end_1byte_values(),
        old_context.end_existence_bitmap(),
        row_index,
        rows_node,
        old_context.columns().to_vec(),
    )?;
    let mut context_data = Vec::new();
    context.write(&mut context_data)?;
    let user_root = heap.alloc(context_data)?;
    let heap_pages = heap.finish(HeapNodeType::Table, user_root)?;
    Ok((heap_pages, row_chunks, row_ids))
}

type NewTableRows = Vec<(NodeId, Vec<Option<TableCell>>)>;
type ExternalValues = Vec<(NodeId, Vec<u8>)>;

#[allow(clippy::too_many_arguments)]
fn new_contents_rows(
    table: &dyn TableContext,
    inputs: &[UnicodePstBatchMessage<'_>],
    named: &NamedIds,
    store_record_key: [u8; 16],
    folder: NodeId,
    messages: &[NodeId],
    message_sizes: &[i32],
    mut next_subnode_index: u32,
    row_version: u32,
    // MailCraft: how each message's strings are written.
    texts: &[Text],
) -> io::Result<(NewTableRows, ExternalValues)> {
    let mut external = Vec::new();
    let rows = inputs
        .iter()
        .zip(messages)
        .zip(message_sizes)
        .zip(texts)
        .map(|(((input, message), message_size), text)| {
            let mut cells = table
                .context()
                .columns()
                .iter()
                .map(|column| {
                    new_message_table_cell(
                        column,
                        &input.message,
                        input.attachments,
                        named,
                        store_record_key,
                        folder,
                        *message,
                        row_version,
                        *message_size,
                        *text,
                    )
                })
                .collect::<io::Result<Vec<_>>>()?;
            for cell in cells.iter_mut().flatten() {
                let TableCell::Bytes(data) = cell else {
                    continue;
                };
                if data.len() > MAX_HN_ALLOCATION_SIZE {
                    // Outlook only follows a table cell into a subnode of type NID_TYPE_LTP; an internal NID makes GetTable fail.
                    let node = NodeId::new(NodeIdType::ListsTablesProperties, next_subnode_index)?;
                    next_subnode_index = next_subnode_index
                        .checked_add(1)
                        .ok_or(PstError::IntegerConversion)?;
                    external.push((node, std::mem::take(data)));
                    *cell = TableCell::Node(node);
                }
            }
            Ok((*message, cells))
        })
        .collect::<io::Result<Vec<_>>>()?;
    Ok((rows, external))
}

fn bbt_entry(
    bbt_entries: &[UnicodeBlockBTreeEntry],
    block: UnicodeBlockId,
) -> io::Result<UnicodeBlockBTreeEntry> {
    bbt_entries
        .binary_search_by_key(&block.search_key(), BTreeEntry::key)
        .map(|index| bbt_entries[index])
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("referenced PST block {:X} is missing", block.search_key()),
            )
        })
}

/// The leaf data blocks of a data tree, in order, as `(block, size)`.
///
/// MailCraft: read the way the reader reads a tree (`DataTree::blocks`): an XBLOCK names leaves
/// and an XXBLOCK names XBLOCKs, nothing deeper, and leaves are named only until the size the
/// top block declares is reached. A tree that names itself is an error rather than a stack
/// overflow, and blocks named over and over can't make the list longer than the tree holds.
fn data_tree_leaves(
    file: &mut File,
    format: Format,
    encoding: NdbCryptMethod,
    bbt_entries: &[UnicodeBlockBTreeEntry],
    block: UnicodeBlockId,
) -> io::Result<Vec<(UnicodeBlockId, u32)>> {
    let entry = bbt_entry(bbt_entries, block)?;
    let (level, total_size, children) = match read_tree_block(file, format, encoding, &entry)? {
        TreeBlock::Leaf(_) => return Ok(vec![(block, u32::from(entry.size()))]),
        TreeBlock::Intermediate {
            level,
            total_size,
            children,
        } => (level, total_size, children),
    };
    let mut budget = u64::from(total_size);
    let mut leaves = Vec::new();
    let mut pending = vec![(level, children, 0_usize)];
    while let Some((level, children, next)) = pending.last_mut() {
        let Some(child) = children.get(*next).copied() else {
            pending.pop();
            continue;
        };
        *next += 1;
        if budget == 0 {
            break;
        }
        let level = *level;
        let entry = bbt_entry(bbt_entries, child)?;
        match read_tree_block(file, format, encoding, &entry)? {
            TreeBlock::Leaf(_) if level == 1 => {
                budget = budget.saturating_sub(u64::from(entry.size()));
                leaves.push((child, u32::from(entry.size())));
            }
            TreeBlock::Intermediate {
                level: child_level,
                children,
                ..
            } if child_level + 1 == level => {
                pending.push((child_level, children, 0_usize));
            }
            _ => return Err(NdbError::InvalidInternalBlockLevel(level).into()),
        }
    }
    Ok(leaves)
}

fn read_data_block(
    file: &mut File,
    format: Format,
    encoding: NdbCryptMethod,
    bbt_entries: &[UnicodeBlockBTreeEntry],
    block: UnicodeBlockId,
) -> io::Result<Vec<u8>> {
    let entry = bbt_entry(bbt_entries, block)?;
    match read_tree_block(file, format, encoding, &entry)? {
        TreeBlock::Leaf(data) => Ok(data),
        TreeBlock::Intermediate { .. } => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "expected a leaf data block",
        )),
    }
}

/// Append rows to a table in place: unchanged heap pages and full row-matrix blocks keep their
/// block IDs, only the last row block, the dirty heap pages and the XBLOCKs are written, so a call
/// writes O(new rows) bytes. Returns `None`, leaving `new_rows` untouched, when the table layout
/// (rows in the heap, empty table, out-of-order row IDs) needs the full rebuild instead.
/// ponytail: still reads the whole table through `TableContext`; read only the right spine if
/// those reads show up in profiles.
#[allow(clippy::too_many_arguments)]
fn append_table_rows(
    file: &mut File,
    encoding: NdbCryptMethod,
    bbt_entries: &[UnicodeBlockBTreeEntry],
    table: &dyn TableContext,
    heap_block: UnicodeBlockId,
    rows_block: Option<UnicodeBlockId>,
    new_rows: &mut NewTableRows,
    row_version: u32,
    blocks: &mut Vec<NewBlock>,
    next_bid: &mut u64,
    next_offset: &mut FileSpace,
) -> io::Result<Option<(UnicodeBlockId, UnicodeBlockId, usize)>> {
    let context = table.context();
    let row_size = usize::from(context.end_existence_bitmap());
    let old_count = table.rows_matrix().count();
    let mut last_id = table.rows_matrix().map(|row| u32::from(row.id())).max();
    let in_order = new_rows.iter().all(|(id, _)| {
        let id = u32::from(*id);
        let ordered = last_id.is_none_or(|last| id > last);
        last_id = Some(id);
        ordered
    });
    let Some(rows_block) = rows_block else {
        return Ok(None);
    };
    // MailCraft: a block holds as many rows as its format makes room for (MS-PST 2.3.4.4).
    let format = next_offset.format;
    let block_size = format.data_block_size();
    if old_count == 0
        || !in_order
        || !(8..=block_size).contains(&row_size)
        || context
            .rows()
            .is_none_or(|rows| matches!(rows.id_type(), Ok(NodeIdType::HeapNode)))
    {
        return Ok(None);
    }

    let mut heap = HeapBuilder::from_pages(table.writer_heap_pages()?);
    let index_hid = context.row_index();
    let index = HeapTreeHeader::read(&mut heap.get(index_hid)?)?;
    if index.key_size() != 4
        || index.entry_size() != format.row_index_size()
        || u32::from(index.root()) == 0
    {
        return Ok(None);
    }
    let heap_leaves = data_tree_leaves(file, format, encoding, bbt_entries, heap_block)?;
    let mut row_leaves = data_tree_leaves(file, format, encoding, bbt_entries, rows_block)?;
    let rows_per_block = block_size / row_size;
    // MailCraft: a row matrix with no blocks, or more than its rows fill, takes the rebuild.
    let Some((tail_block, _)) = row_leaves.pop() else {
        return Ok(None);
    };
    let tail_rows = row_leaves
        .len()
        .checked_mul(rows_per_block)
        .and_then(|full| old_count.checked_sub(full));
    let Some(tail_rows) = tail_rows.filter(|rows| (1..=rows_per_block).contains(rows)) else {
        return Ok(None);
    };
    let mut tail = read_data_block(file, format, encoding, bbt_entries, tail_block)?;
    if heap_leaves.len() != heap.pages.len() || tail.len() < tail_rows * row_size {
        return Ok(None);
    }
    tail.truncate(tail_rows * row_size);

    let spine = take_row_index_spine(&mut heap, &index, format)?;
    let mut records = Vec::with_capacity(new_rows.len());
    for (row_id, cells) in std::mem::take(new_rows) {
        let row_index =
            u32::try_from(old_count + records.len()).map_err(|_| PstError::IntegerConversion)?;
        tail.extend(encode_table_row(
            context,
            &mut heap,
            row_id,
            row_version,
            cells,
        )?);
        records.push((u32::from(row_id), row_index));
    }
    let row_count = old_count + records.len();
    let header = append_row_index_spine(&mut heap, spine, index.levels(), &records, format)?;
    heap.replace(index_hid, header)?;

    let heap_pages = heap.finish_dirty(HeapNodeType::Table, table.writer_heap_user_root()?)?;
    let heap_leaves = heap_pages
        .into_iter()
        .enumerate()
        .map(|(page, data)| match data {
            None => Ok(heap_leaves[page]),
            Some(data) => {
                let size = u32::try_from(data.len()).map_err(|_| PstError::IntegerConversion)?;
                let block = push_block(
                    blocks,
                    next_bid,
                    next_offset,
                    false,
                    NewBlockData::Data(data),
                )?;
                Ok((block, size))
            }
        })
        .collect::<io::Result<Vec<_>>>()?;
    let heap_root = push_data_tree_leaves(blocks, next_bid, next_offset, heap_leaves)?;

    let mut chunks = tail
        .chunks(rows_per_block * row_size)
        .map(<[u8]>::to_vec)
        .collect::<Vec<_>>();
    let last = chunks.len() - 1;
    for chunk in &mut chunks[..last] {
        chunk.resize(block_size, 0);
    }
    for chunk in chunks {
        let size = u32::try_from(chunk.len()).map_err(|_| PstError::IntegerConversion)?;
        let block = push_block(
            blocks,
            next_bid,
            next_offset,
            false,
            NewBlockData::Data(chunk),
        )?;
        row_leaves.push((block, size));
    }
    let rows_root = push_data_tree_leaves(blocks, next_bid, next_offset, row_leaves)?;
    Ok(Some((heap_root, rows_root, row_count)))
}

fn property_context_data(
    properties: &[(u16, PropertyType, Option<Vec<u8>>, u32)],
) -> io::Result<Vec<u8>> {
    property_context_data_with_nodes(properties, &[])
}

fn property_context_data_with_nodes(
    properties: &[(u16, PropertyType, Option<Vec<u8>>, u32)],
    nodes: &[(u16, NodeId)],
) -> io::Result<Vec<u8>> {
    let mut allocations = vec![vec![], vec![], vec![]];
    let mut records = Vec::with_capacity(properties.len());

    for (prop_id, prop_type, value, small) in properties {
        let record = if let Some((_, node)) = nodes.iter().find(|(id, _)| id == prop_id) {
            PropertyTreeRecord::new(*prop_id, *prop_type, PropertyValueRecord::Node(*node))
        } else if let Some(value) = value {
            let allocation = allocations.len();
            allocations.push(value.clone());
            PropertyTreeRecord::new(
                *prop_id,
                *prop_type,
                PropertyValueRecord::Heap(heap_id(allocation)?),
            )
        } else {
            PropertyTreeRecord::new(*prop_id, *prop_type, PropertyValueRecord::Small(*small))
        };
        record.write(&mut records)?;
    }

    let tree_root = heap_id(2)?;
    let tree_header = HeapTreeHeader::new(2, 6, 0, tree_root)?;
    tree_header.write(&mut allocations[1])?;
    allocations[2] = records;

    finish_heap(allocations, HeapNodeType::Properties, heap_id(1)?)
}

fn table_context_data(row_ids: &[NodeId], format: Format) -> io::Result<Vec<u8>> {
    let row_index = heap_id(2)?;
    let rows_heap = (!row_ids.is_empty()).then(|| heap_id(4)).transpose()?;
    let columns = vec![
        TableColumnDescriptor::new(PropertyType::Integer32, LTP_ROW_ID_PROP_ID, 0, 4, 0),
        TableColumnDescriptor::new(PropertyType::Integer32, LTP_ROW_VERSION_PROP_ID, 4, 4, 1),
    ];
    let context = TableContextInfo::new(
        8,
        8,
        8,
        9,
        row_index,
        rows_heap.map(|id| NodeId::from(u32::from(id))),
        columns,
    )?;

    let mut context_bytes = Vec::new();
    context.write(&mut context_bytes)?;

    let tree_root = if row_ids.is_empty() {
        HeapId::from(0)
    } else {
        heap_id(3)?
    };
    // MailCraft: a row number is 16 bits in an ANSI file.
    let entry_size = format.row_index_size();
    let tree_header = HeapTreeHeader::new(4, entry_size, 0, tree_root)?;
    let mut tree_header_bytes = Vec::new();
    tree_header.write(&mut tree_header_bytes)?;

    let mut index_bytes = Vec::new();
    let mut rows = Vec::new();
    for (index, node_id) in row_ids.iter().enumerate() {
        let row_id = TableRowId::new(u32::from(*node_id));
        row_id.write(&mut index_bytes)?;
        write_row_number(&mut index_bytes, index, format)?;

        rows.write_u32::<byteorder::LittleEndian>(u32::from(*node_id))?;
        rows.write_u32::<byteorder::LittleEndian>(0)?;
        rows.push(0xC0);
    }

    let mut allocations = vec![vec![], context_bytes, tree_header_bytes];
    if !row_ids.is_empty() {
        allocations.push(index_bytes);
        allocations.push(rows);
    }
    finish_heap(allocations, HeapNodeType::Table, heap_id(1)?)
}

type RebuiltRecipientTable = (Vec<Vec<u8>>, Option<(NodeId, Vec<Vec<u8>>)>);

fn one_off_entry_id(recipient: &UnicodePstRecipient<'_>) -> Vec<u8> {
    let mut entry_id = vec![0; 4];
    entry_id.extend_from_slice(&[
        0x81, 0x2B, 0x1F, 0xA4, 0xBE, 0xA3, 0x10, 0x19, 0x9D, 0x6E, 0x00, 0xDD, 0x01, 0x0F, 0x54,
        0x02,
    ]);
    entry_id.extend_from_slice(&0u16.to_le_bytes());
    entry_id.extend_from_slice(&0x8000u16.to_le_bytes());
    entry_id.extend(utf16_bytes(if recipient.name.is_empty() {
        recipient.email
    } else {
        recipient.name
    }));
    entry_id.extend(utf16_bytes("SMTP"));
    entry_id.extend(utf16_bytes(recipient.email));
    entry_id
}

fn recipient_search_key(email: &str) -> Vec<u8> {
    let mut key = format!("SMTP:{email}").to_ascii_uppercase().into_bytes();
    key.push(0);
    key
}

/// MailCraft: where a recipient row's two booleans start, after its 4-byte columns (the
/// flags and track status among them), and the row's size with its 17 columns' existence
/// bitmap (3 bytes).
const RECIPIENT_BOOLEANS: u16 = 60;
const RECIPIENT_ROW: u16 = RECIPIENT_BOOLEANS + 2 + 3;

fn recipient_table_data(
    recipients: &[UnicodePstRecipient<'_>],
    row_version: u32,
    text: Text,
) -> io::Result<RebuiltRecipientTable> {
    // MailCraft: its strings are 8-bit in an ANSI file.
    let string = text.string_type();
    let columns = vec![
        TableColumnDescriptor::new(PropertyType::Integer32, 0x0C15, 8, 4, 2),
        TableColumnDescriptor::new(PropertyType::Boolean, 0x0E0F, RECIPIENT_BOOLEANS, 1, 3),
        TableColumnDescriptor::new(PropertyType::Binary, 0x0FF9, 12, 4, 4),
        TableColumnDescriptor::new(PropertyType::Integer32, 0x0FFE, 16, 4, 5),
        TableColumnDescriptor::new(PropertyType::Binary, 0x0FFF, 20, 4, 6),
        TableColumnDescriptor::new(string, 0x3001, 24, 4, 7),
        TableColumnDescriptor::new(string, 0x3002, 28, 4, 8),
        TableColumnDescriptor::new(string, 0x3003, 32, 4, 9),
        TableColumnDescriptor::new(PropertyType::Binary, 0x300B, 36, 4, 10),
        TableColumnDescriptor::new(PropertyType::Integer32, 0x3900, 40, 4, 11),
        TableColumnDescriptor::new(string, 0x39FE, 44, 4, 12),
        TableColumnDescriptor::new(string, 0x39FF, 48, 4, 13),
        TableColumnDescriptor::new(PropertyType::Boolean, 0x3A40, RECIPIENT_BOOLEANS + 1, 1, 14),
        // MailCraft: a meeting's organizer and attendees (MS-OXOCAL 2.2.4.10).
        TableColumnDescriptor::new(PropertyType::Integer32, 0x5FFD, 52, 4, 15),
        TableColumnDescriptor::new(PropertyType::Integer32, 0x5FFF, 56, 4, 16),
        TableColumnDescriptor::new(PropertyType::Integer32, LTP_ROW_ID_PROP_ID, 0, 4, 0),
        TableColumnDescriptor::new(PropertyType::Integer32, LTP_ROW_VERSION_PROP_ID, 4, 4, 1),
    ];
    let ends = (
        RECIPIENT_BOOLEANS,
        RECIPIENT_BOOLEANS,
        RECIPIENT_BOOLEANS + 2,
        RECIPIENT_ROW,
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
    let row_ids = (1..=recipients.len())
        .map(|id| u32::try_from(id).map_err(|_| PstError::IntegerConversion.into()))
        .collect::<io::Result<Vec<_>>>()?;
    let mut heap = HeapBuilder::new();
    let rows = row_ids
        .iter()
        .zip(recipients)
        .map(|(row_id, recipient)| {
            let entry_id = one_off_entry_id(recipient);
            let display_name = if recipient.name.is_empty() {
                recipient.email
            } else {
                recipient.name
            };
            encode_table_row(
                &row_context,
                &mut heap,
                *row_id,
                row_version,
                vec![
                    Some(TableCell::Value(PropertyValue::Integer32(
                        recipient.recipient_type as i32,
                    ))),
                    Some(TableCell::Value(PropertyValue::Boolean(true))),
                    None,
                    Some(TableCell::Value(PropertyValue::Integer32(6))),
                    Some(TableCell::Bytes(entry_id)),
                    string_table_cell(string, display_name, text),
                    string_table_cell(string, "SMTP", text),
                    string_table_cell(string, recipient.email, text),
                    Some(TableCell::Bytes(recipient_search_key(recipient.email))),
                    Some(TableCell::Value(PropertyValue::Integer32(0))),
                    string_table_cell(string, recipient.email, text),
                    string_table_cell(string, display_name, text),
                    Some(TableCell::Value(PropertyValue::Boolean(false))),
                    recipient
                        .flags
                        .map(|flags| TableCell::Value(PropertyValue::Integer32(flags))),
                    recipient
                        .track_status
                        .map(|status| TableCell::Value(PropertyValue::Integer32(status))),
                    Some(TableCell::Value(PropertyValue::Integer32(
                        i32::try_from(*row_id).map_err(|_| PstError::IntegerConversion)?,
                    ))),
                    Some(TableCell::Value(PropertyValue::Integer32(
                        row_version as i32,
                    ))),
                ],
            )
        })
        .collect::<io::Result<Vec<_>>>()?;
    let row_index = build_row_index(&mut heap, &row_ids, text.format())?;
    let rows_node = (!rows.is_empty())
        .then(|| NodeId::new(NodeIdType::Internal, 0x80))
        .transpose()?;
    let context = TableContextInfo::new(
        ends.0, ends.1, ends.2, ends.3, row_index, rows_node, columns,
    )?;
    let mut context_bytes = Vec::new();
    context.write(&mut context_bytes)?;
    let user_root = heap.alloc(context_bytes)?;
    let pages = heap.finish(HeapNodeType::Table, user_root)?;
    let rows_per_block = text.format().data_block_size() / usize::from(RECIPIENT_ROW);
    let row_chunks = rows
        .chunks(rows_per_block)
        .map(|chunk| chunk.concat())
        .collect::<Vec<_>>();
    Ok((pages, rows_node.map(|node| (node, row_chunks))))
}

fn attachment_table_data(
    attachment_ids: &[NodeId],
    attachments: &[UnicodePstAttachment<'_>],
    contents: &[usize],
    row_version: u32,
    text: Text,
) -> io::Result<RebuiltRecipientTable> {
    let columns = vec![
        TableColumnDescriptor::new(PropertyType::Integer32, 0x0E20, 8, 4, 2),
        // MailCraft: 8-bit in an ANSI file.
        TableColumnDescriptor::new(text.string_type(), 0x3704, 12, 4, 3),
        TableColumnDescriptor::new(PropertyType::Integer32, 0x3705, 16, 4, 4),
        TableColumnDescriptor::new(PropertyType::Integer32, 0x370B, 20, 4, 5),
        TableColumnDescriptor::new(PropertyType::Integer32, LTP_ROW_ID_PROP_ID, 0, 4, 0),
        TableColumnDescriptor::new(PropertyType::Integer32, LTP_ROW_VERSION_PROP_ID, 4, 4, 1),
    ];
    let row_context =
        TableContextInfo::new(24, 24, 24, 25, HeapId::default(), None, columns.clone())?;
    let mut heap = HeapBuilder::new();
    let rows = attachment_ids
        .iter()
        .zip(attachments.iter().zip(contents))
        .map(|(attachment_id, (attachment, content))| {
            let size = attachment_size(attachment, *content)?;
            // MailCraft: an attached message has no file name, and its own method.
            let (filename, method) = match embedded(attachment) {
                Some(_) => (None, ATTACH_EMBEDDED_MESSAGE),
                None => (
                    string_table_cell(text.string_type(), attachment.filename, text),
                    ATTACH_BY_VALUE,
                ),
            };
            encode_table_row(
                &row_context,
                &mut heap,
                *attachment_id,
                row_version,
                vec![
                    Some(TableCell::Value(PropertyValue::Integer32(size))),
                    filename,
                    Some(TableCell::Value(PropertyValue::Integer32(method))),
                    Some(TableCell::Value(PropertyValue::Integer32(-1))),
                    Some(TableCell::Value(PropertyValue::Integer32(
                        u32::from(*attachment_id) as i32,
                    ))),
                    Some(TableCell::Value(PropertyValue::Integer32(
                        row_version as i32,
                    ))),
                ],
            )
        })
        .collect::<io::Result<Vec<_>>>()?;
    let row_index = build_row_index(&mut heap, attachment_ids, text.format())?;
    let rows_node = (!rows.is_empty())
        .then(|| NodeId::new(NodeIdType::Internal, 0x80))
        .transpose()?;
    let context = TableContextInfo::new(24, 24, 24, 25, row_index, rows_node, columns)?;
    let mut context_bytes = Vec::new();
    context.write(&mut context_bytes)?;
    let user_root = heap.alloc(context_bytes)?;
    let pages = heap.finish(HeapNodeType::Table, user_root)?;
    let rows_per_block = text.format().data_block_size() / 25;
    let row_chunks = rows
        .chunks(rows_per_block)
        .map(|chunk| chunk.concat())
        .collect::<Vec<_>>();
    Ok((pages, rows_node.map(|node| (node, row_chunks))))
}

fn validate_folder_path(folder_path: &[&str]) -> io::Result<()> {
    if folder_path.is_empty() || folder_path.iter().any(|name| name.is_empty()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "folder path must contain only non-empty names",
        ));
    }
    Ok(())
}

fn folder_data(
    name: &str,
    content_count: usize,
    has_subfolders: bool,
    container_class: &str,
    text: Text,
) -> io::Result<Vec<u8>> {
    let content_count = u32::try_from(content_count).map_err(|_| PstError::IntegerConversion)?;
    let string = text.string_type();
    property_context_data(&[
        (0x3001, string, Some(text.bytes(name)), 0),
        (0x3602, PropertyType::Integer32, None, content_count),
        (0x3603, PropertyType::Integer32, None, 0),
        (
            0x360A,
            PropertyType::Boolean,
            None,
            u32::from(has_subfolders),
        ),
        (0x3613, string, Some(text.bytes(container_class)), 0),
    ])
}

fn message_data(
    input: &UnicodePstMessage<'_>,
    attachments: &[UnicodePstAttachment<'_>],
    named: &NamedIds,
    message: NodeId,
    row_version: u32,
    message_size: i32,
    text: Text,
) -> io::Result<RebuiltPropertyContext> {
    let has_attachments = !attachments.is_empty();
    // MailCraft: 8-bit strings in an ANSI file, in the code page the message says.
    let unicode = |value: &str| text.value(value);
    let mut properties = vec![
        (0x001A, unicode(input.message_class)),
        (0x0037, unicode(input.subject)),
        (0x0039, PropertyValue::Time(input.delivery_time)),
        (0x0042, unicode(input.sender_name)),
        (0x0070, unicode(input.subject)),
        (0x0C1A, unicode(input.sender_name)),
        (0x0C1F, unicode(input.sender_email)),
        (0x0E06, PropertyValue::Time(input.delivery_time)),
        (
            0x0E07,
            PropertyValue::Integer32(message_flags(input, has_attachments)),
        ),
        (0x0E08, PropertyValue::Integer32(message_size)),
        (0x0E17, PropertyValue::Integer32(0)),
        (0x0E1B, PropertyValue::Boolean(has_attachments)),
        (0x1000, unicode(input.body)),
        (0x1035, unicode(input.message_id)),
        (0x3007, PropertyValue::Time(input.delivery_time)),
        (0x3008, PropertyValue::Time(input.delivery_time)),
        (
            0x300B,
            PropertyValue::Binary(BinaryValue::new(input.message_id.as_bytes().to_vec())),
        ),
        (0x3FDE, PropertyValue::Integer32(65001)),
        (
            LTP_ROW_ID_PROP_ID,
            PropertyValue::Integer32(u32::from(message) as i32),
        ),
        (
            LTP_ROW_VERSION_PROP_ID,
            PropertyValue::Integer32(row_version as i32),
        ),
    ];
    for (id, recipient_type) in [
        (0x0E02, UnicodePstRecipientType::Bcc),
        (0x0E03, UnicodePstRecipientType::Cc),
        (0x0E04, UnicodePstRecipientType::To),
    ] {
        let display = recipient_display(input.recipients, recipient_type);
        if !display.is_empty() {
            properties.push((id, unicode(&display)));
        }
    }
    if let Some(html) = input.html_body {
        properties.push((
            0x1013,
            PropertyValue::Binary(BinaryValue::new(html.as_bytes().to_vec())),
        ));
    }
    let mut properties = with_added(input, named, properties)?;
    // MailCraft: `PidTagMessageCodepage`, the code page of an ANSI file's message's strings
    // (MS-OXCMSG 2.2.1.4), whatever was given for it.
    if let Text::Ansi(page) = text {
        properties.retain(|(id, _)| *id != 0x3FFD);
        properties.push((0x3FFD, PropertyValue::Integer32(page.id())));
    }
    property_context_pages(properties, 0x80, text)
}

/// An attachment's property context: `data_node` holds its file, when it has one, and `content`
/// is the size of what it holds. MailCraft: `object` is its attached message's node, when it is
/// one: `PidTagAttachDataObject` is then a `PtypObject` naming that node and its size, as
/// MS-PST 2.4.6.2 keeps it, and its display name is given in place of a file's names and type.
fn attachment_data(
    input: &UnicodePstAttachment<'_>,
    data_node: Option<NodeId>,
    content: usize,
    object: Option<NodeId>,
    text: Text,
) -> io::Result<Vec<u8>> {
    let size = attachment_size(input, content)?;
    // MailCraft: its strings are 8-bit in an ANSI file.
    let string = text.string_type();
    if let Some(node) = object {
        let mut value = Vec::with_capacity(8);
        node.write(&mut value)?;
        value.write_u32::<byteorder::LittleEndian>(
            u32::try_from(content).map_err(|_| PstError::IntegerConversion)?,
        )?;
        let mut properties = vec![
            (0x0E20, PropertyType::Integer32, None, size as u32),
            (0x3001, string, Some(text.bytes(input.filename)), 0),
            (0x3701, PropertyType::Object, Some(value), 0),
            (
                0x3705,
                PropertyType::Integer32,
                None,
                ATTACH_EMBEDDED_MESSAGE as u32,
            ),
            (0x370B, PropertyType::Integer32, None, u32::MAX),
        ];
        with_attachment_properties(input, &mut properties, text)?;
        return fitted_attachment(property_context_data(&properties)?);
    }
    let mut properties = vec![
        (0x0E20, PropertyType::Integer32, None, size as u32),
        (0x3701, PropertyType::Binary, None, 0),
        (0x3704, string, Some(text.bytes(input.filename)), 0),
        (0x3705, PropertyType::Integer32, None, 1),
        (0x3707, string, Some(text.bytes(input.filename)), 0),
        (0x370B, PropertyType::Integer32, None, u32::MAX),
        (0x370E, string, Some(text.bytes(input.mime_type)), 0),
    ];
    if let Some(content_id) = attachment_content_id(input)? {
        properties.extend([
            (0x3712, string, Some(text.bytes(content_id)), 0),
            // PidTagAttachFlags bit 0x4 marks an attachment as referenced by HTML.
            (0x3714, PropertyType::Integer32, None, 0x0000_0004),
        ]);
    }
    // MailCraft: its own added properties.
    with_attachment_properties(input, &mut properties, text)?;
    fitted_attachment(property_context_data_with_nodes(
        &properties,
        &data_node.map(|node| [(0x3701, node)]).unwrap_or_default(),
    )?)
}

/// MailCraft: an attachment's property context is one block: one whose names and added
/// properties make it bigger is an error, where it was written as a block no reader takes.
fn fitted_attachment(data: Vec<u8>) -> io::Result<Vec<u8>> {
    if data.len() > MAX_DATA_BLOCK_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "an attachment's names and properties don't fit in one block",
        ));
    }
    Ok(data)
}

fn attachment_content_id<'a>(input: &'a UnicodePstAttachment<'_>) -> io::Result<Option<&'a str>> {
    let Some(content_id) = input.content_id else {
        return Ok(None);
    };
    let content_id = content_id.strip_prefix('<').unwrap_or(content_id);
    let content_id = content_id.strip_suffix('>').unwrap_or(content_id);
    if content_id.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "attachment Content-ID must not be empty",
        ));
    }
    Ok(Some(content_id))
}

/// `PidTagAttachSize` of an attachment whose content (MailCraft: a file's bytes, or an attached
/// message's nodes) takes `content` bytes.
fn attachment_size(input: &UnicodePstAttachment<'_>, content: usize) -> io::Result<i32> {
    let filenames_size = utf16_bytes(input.filename)
        .len()
        .checked_mul(2)
        .ok_or(PstError::IntegerConversion)?;
    let mut size = content
        .checked_add(12)
        .and_then(|size| size.checked_add(filenames_size))
        .and_then(|size| size.checked_add(utf16_bytes(input.mime_type).len()))
        .ok_or(PstError::IntegerConversion)?;
    if let Some(content_id) = attachment_content_id(input)? {
        size = size
            .checked_add(utf16_bytes(content_id).len())
            .and_then(|size| size.checked_add(4))
            .ok_or(PstError::IntegerConversion)?;
    }
    i32::try_from(size).map_err(|_| PstError::IntegerConversion.into())
}

enum NewBlockData {
    Data(Vec<u8>),
    DataTree(Vec<UnicodeDataTreeEntry>, u8, u32),
    SubNodes(Vec<UnicodeLeafSubNodeTreeEntry>),
    IntermediateSubNodes(Vec<UnicodeIntermediateSubNodeTreeEntry>, u8),
}

struct NewBlock {
    id: UnicodeBlockId,
    offset: u64,
    size: u16,
    data: NewBlockData,
}

enum NewBTreePageData {
    BlockLeaf(Vec<UnicodeBlockBTreeEntry>),
    NodeLeaf(Vec<UnicodeNodeBTreeEntry>),
    Intermediate(PageType, u8, Vec<UnicodeBTreePageEntry>),
}

struct NewBTreePage {
    page_ref: UnicodePageRef,
    data: NewBTreePageData,
}

/// A B-tree as read from disk: each level's pages in key order with their items, so a commit
/// can keep every page whose items did not change.
struct BTreeLayout<E> {
    leaves: BTreePages<E>,
    /// Intermediate pages; `inner[0]` is level 1.
    inner: Vec<BTreePages<(u64, UnicodePageRef)>>,
}

type BTreePages<T> = Vec<(UnicodePageRef, Vec<T>)>;

/// Block `(offset, size)` and B-tree page offsets a commit already knows are allocated.
type KnownAllocations<'a> = (&'a [(u64, u16)], &'a [u64]);

enum BTreePageItems<E> {
    Leaf(Vec<E>),
    Intermediate(u8, Vec<(u64, UnicodePageRef)>),
}

impl<E: Copy + BTreeEntry<Key = u64>> BTreeLayout<E> {
    fn read(
        file: &mut File,
        root: UnicodePageRef,
        read_page: impl Fn(&mut File, UnicodePageRef) -> io::Result<BTreePageItems<E>>,
    ) -> io::Result<Self> {
        let mut layout = Self {
            leaves: Vec::new(),
            inner: Vec::new(),
        };
        // MailCraft: each page is a level below the one that names it (the root's level is what
        // it says) and is named once, as the reader's `RootBTreePage::find_entry` checks, so
        // pages that name themselves, or one page named over and over, are an error rather than
        // a walk that never ends.
        let mut stack = vec![(root, 0_u8, None::<u8>)];
        let mut seen = HashSet::new();
        while let Some((page_ref, depth, expected)) = stack.pop() {
            let invalid = || io::Error::from(PstError::InvalidBTreePage(page_ref.index().index()));
            if depth > 8 || !seen.insert(page_ref.index().index()) {
                return Err(invalid());
            }
            match read_page(file, page_ref)? {
                BTreePageItems::Leaf(entries) => {
                    if expected.is_some_and(|level| level != 0) {
                        return Err(invalid());
                    }
                    layout.leaves.push((page_ref, entries))
                }
                BTreePageItems::Intermediate(level, children) => {
                    if level == 0 || expected.is_some_and(|expected| expected != level) {
                        return Err(invalid());
                    }
                    let index = usize::from(level);
                    if layout.inner.len() < index {
                        layout.inner.resize_with(index, Vec::new);
                    }
                    stack.extend(
                        children
                            .iter()
                            .rev()
                            .map(|(_, child)| (*child, depth + 1, Some(level - 1))),
                    );
                    layout.inner[index - 1].push((page_ref, children));
                }
            }
        }
        Ok(layout)
    }

    fn entries(&self) -> Vec<E> {
        let mut entries = self
            .leaves
            .iter()
            .flat_map(|(_, entries)| entries.iter().copied())
            .collect::<Vec<_>>();
        entries.sort_by_key(BTreeEntry::key);
        entries
    }
}

fn collect_block_btree_entries(
    file: &mut File,
    format: Format,
    page_ref: UnicodePageRef,
) -> io::Result<Vec<UnicodeBlockBTreeEntry>> {
    Ok(read_block_btree(file, format, page_ref)?.entries())
}

#[cfg(test)]
fn collect_node_btree_entries(
    file: &mut File,
    format: Format,
    page_ref: UnicodePageRef,
) -> io::Result<Vec<UnicodeNodeBTreeEntry>> {
    Ok(read_node_btree(file, format, page_ref)?.entries())
}

fn reserve_btree_page(
    pages: &mut Vec<NewBTreePage>,
    next_page: &mut UnicodePageId,
    next_offset: &mut FileSpace,
    data: NewBTreePageData,
) -> io::Result<UnicodePageRef> {
    let id = *next_page;
    *next_page = next_page.next()?;
    let offset = reserve_file_range(next_offset, PAGE_SIZE as u64, PAGE_SIZE as u64)?;
    let page_ref = UnicodePageRef::new(id, UnicodeByteIndex::new(offset));
    pages.push(NewBTreePage { page_ref, data });
    Ok(page_ref)
}

fn same_btree_entry<E: BTreeEntryReadWrite>(left: &E, right: &E) -> bool {
    let (mut a, mut b) = (Vec::with_capacity(32), Vec::with_capacity(32));
    left.write(&mut a).is_ok() && right.write(&mut b).is_ok() && a == b
}

fn same_page_ref(left: &UnicodePageRef, right: &UnicodePageRef) -> bool {
    left.block() == right.block() && left.index().index() == right.index().index()
}

/// Lay `items` out over the pages of one B-tree level. Each item goes to the old page whose
/// key range holds it; a page whose items are unchanged is kept as is, any other is rewritten
/// (split when it overflows). Returns the level's `(first key, page)` list in key order.
fn plan_btree_level<T: Clone>(
    old: &[(UnicodePageRef, Vec<T>)],
    mut items: &[T],
    key: impl Fn(&T) -> u64,
    same: impl Fn(&T, &T) -> bool,
    max: usize,
    live: &mut Vec<u64>,
    mut write: impl FnMut(Vec<T>) -> io::Result<UnicodePageRef>,
) -> io::Result<Vec<(u64, UnicodePageRef)>> {
    let mut level = Vec::new();
    let mut place = |group: &[T], old_page: Option<&(UnicodePageRef, Vec<T>)>| {
        if let Some((page, old_items)) = old_page {
            if group.len() == old_items.len()
                && group.iter().zip(old_items).all(|(a, b)| same(a, b))
            {
                live.push(page.index().index());
                level.push((key(&group[0]), *page));
                return Ok(());
            }
        }
        for chunk in group.chunks(max) {
            let page = write(chunk.to_vec())?;
            live.push(page.index().index());
            level.push((key(&chunk[0]), page));
        }
        Ok::<_, io::Error>(())
    };
    for (index, old_page) in old.iter().enumerate() {
        let end = match old[index + 1..].iter().find_map(|(_, items)| items.first()) {
            Some(next) => items.partition_point(|item| key(item) < key(next)),
            None => items.len(),
        };
        let (group, rest) = items.split_at(end);
        items = rest;
        if !group.is_empty() {
            place(group, Some(old_page))?;
        }
    }
    if !items.is_empty() {
        place(items, None)?;
    }
    Ok(level)
}

/// Plan a B-tree holding `entries`, rewriting only pages that differ from `old`. Every page
/// of the new tree (kept or written) is appended to `live`.
#[allow(clippy::too_many_arguments)]
fn plan_btree<E: Copy + BTreeEntry<Key = u64> + BTreeEntryReadWrite>(
    old: &BTreeLayout<E>,
    entries: &[E],
    (leaf_max, inner_max): (usize, usize),
    page_type: PageType,
    leaf: fn(Vec<E>) -> NewBTreePageData,
    pages: &mut Vec<NewBTreePage>,
    next_page: &mut UnicodePageId,
    next_offset: &mut FileSpace,
    live: &mut Vec<u64>,
) -> io::Result<UnicodePageRef> {
    if entries.is_empty() {
        let page = reserve_btree_page(pages, next_page, next_offset, leaf(Vec::new()))?;
        live.push(page.index().index());
        return Ok(page);
    }
    let mut children = plan_btree_level(
        &old.leaves,
        entries,
        BTreeEntry::key,
        same_btree_entry,
        leaf_max,
        live,
        |chunk| reserve_btree_page(pages, next_page, next_offset, leaf(chunk)),
    )?;
    let mut level = 1_u8;
    while children.len() > 1 {
        if level > 8 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "PST B-tree exceeds eight levels",
            ));
        }
        let old_level = old
            .inner
            .get(usize::from(level) - 1)
            .map_or(&[][..], Vec::as_slice);
        children = plan_btree_level(
            old_level,
            &children,
            |(key, _)| *key,
            |(a_key, a_page), (b_key, b_page)| a_key == b_key && same_page_ref(a_page, b_page),
            inner_max,
            live,
            |chunk| {
                let entries = chunk
                    .iter()
                    .map(|(key, page_ref)| UnicodeBTreePageEntry::new(*key, *page_ref))
                    .collect();
                reserve_btree_page(
                    pages,
                    next_page,
                    next_offset,
                    NewBTreePageData::Intermediate(page_type, level, entries),
                )
            },
        )?;
        level += 1;
    }
    Ok(children[0].1)
}

fn write_btree_page(file: &mut File, format: Format, page: NewBTreePage) -> io::Result<()> {
    // MailCraft: an ANSI file's pages are its own.
    if format == Format::Ansi {
        return write_ansi_btree_page(file, page);
    }
    let page_type = match &page.data {
        NewBTreePageData::BlockLeaf(_) => PageType::BlockBTree,
        NewBTreePageData::NodeLeaf(_) => PageType::NodeBTree,
        NewBTreePageData::Intermediate(page_type, ..) => *page_type,
    };
    let trailer = UnicodePageTrailer::new(
        page_type,
        page_type.signature(
            page.page_ref.index().index(),
            page.page_ref.block().into_u64(),
        ),
        page.page_ref.block(),
        0,
    );
    match page.data {
        NewBTreePageData::BlockLeaf(entries) => UnicodeBlockBTree::Leaf(Box::new(
            UnicodeBlockBTreePage::new(0, 20, 24, &entries, trailer)?,
        ))
        .write(file, page.page_ref),
        NewBTreePageData::NodeLeaf(entries) => UnicodeNodeBTree::Leaf(Box::new(
            UnicodeNodeBTreePage::new(0, 15, 32, &entries, trailer)?,
        ))
        .write(file, page.page_ref),
        NewBTreePageData::Intermediate(_, level, entries) => {
            file.seek(SeekFrom::Start(page.page_ref.index().index()))?;
            let page = UnicodeBTreeEntryPage::new(level, 20, 24, &entries, trailer)?;
            <UnicodeBTreeEntryPage as UnicodeBTreePageReadWrite<UnicodeBTreePageEntry>>::write(
                &page, file,
            )
        }
    }
}

fn push_block(
    blocks: &mut Vec<NewBlock>,
    next_bid: &mut u64,
    next_offset: &mut FileSpace,
    internal: bool,
    data: NewBlockData,
) -> io::Result<UnicodeBlockId> {
    let id = UnicodeBlockId::new(internal, *next_bid)?;
    *next_bid += 1;
    push_block_with_id(blocks, next_offset, id, data)?;
    Ok(id)
}

fn push_block_with_id(
    blocks: &mut Vec<NewBlock>,
    next_offset: &mut FileSpace,
    id: UnicodeBlockId,
    data: NewBlockData,
) -> io::Result<()> {
    // MailCraft: a block is as big as its format makes it, its trailer too.
    let format = next_offset.format;
    let size = format.block_data_size(&data)?;
    if usize::from(size) > format.data_block_size() {
        return Err(NdbError::InvalidBlockSize(size).into());
    }
    let disk_size = u64::from(ndb::block::block_size(size + format.block_trailer_size()));
    let offset = reserve_file_range(next_offset, disk_size, 64)?;
    blocks.push(NewBlock {
        id,
        offset,
        size,
        data,
    });
    Ok(())
}

fn map_page_count(amap_index: u64) -> u64 {
    let has_pmap = amap_index % PMAP_PAGE_COUNT == 0;
    let has_fmap = has_pmap
        && amap_index >= FMAP_FIRST_SIZE
        && (amap_index - FMAP_FIRST_SIZE) % FMAP_PAGE_COUNT == 0;
    let has_fpmap = has_pmap
        && amap_index >= FPMAP_FIRST_SIZE
        && (amap_index - FPMAP_FIRST_SIZE) % FPMAP_PAGE_COUNT == 0;
    1 + u64::from(has_pmap) + u64::from(has_fmap) + u64::from(has_fpmap)
}

/// Where new blocks and pages go: space the committed AMap marks free, then EOF.
/// Free space is unreferenced by the committed B-trees, so overwriting it before
/// the header switches over keeps the old state intact.
struct FileSpace {
    eof: u64,
    free: Vec<(u64, u64)>,
    /// MailCraft: the file's format, which says how big its blocks are and how far it can grow.
    format: Format,
}

impl FileSpace {
    fn new(eof: u64, format: Format) -> Self {
        Self {
            eof,
            free: Vec::new(),
            format,
        }
    }

    /// MailCraft: the allocation map is taken only when every page of it reads as the reader
    /// reads a map page (type, its repeat and CRC, a zero signature and the page's own offset as
    /// its ID) and none of the space it marks free holds a block the block B-tree lists, a page
    /// of either B-tree or a map page. Otherwise the map is treated as invalid and new blocks go
    /// at the end of the file. A damaged map that still claimed to be valid put new blocks over
    /// live ones, and the append succeeded with old mail lost.
    fn read(
        file: &mut File,
        header: &FileHeader,
        bbt: &BTreeLayout<UnicodeBlockBTreeEntry>,
        nbt: &BTreeLayout<UnicodeNodeBTreeEntry>,
    ) -> io::Result<Self> {
        let eof = header.file_eof();
        let format = header.format();
        if header.amap_status() == AmapStatus::Invalid {
            return Ok(Self::new(eof, format));
        }
        Ok(Self::read_map(file, eof, format)
            .filter(|space| !space.overlaps_live(bbt, nbt))
            .unwrap_or_else(|| Self::new(eof, format)))
    }

    /// The free space the allocation map marks, or `None` if a page of it doesn't read.
    fn read_map(file: &mut File, eof: u64, format: Format) -> Option<Self> {
        let mut space = Self::new(eof, format);
        let mut amap = AMAP_FIRST_OFFSET;
        let mut bytes = [0; PAGE_SIZE];
        while amap < space.eof {
            file.seek(SeekFrom::Start(amap)).ok()?;
            file.read_exact(&mut bytes).ok()?;
            let bits = read_map_page(&bytes, format, amap)?;
            for bit in 0..bits.len() * 8 {
                let offset = amap + bit as u64 * 64;
                if offset >= space.eof {
                    break;
                }
                if bits[bit / 8] & (0x80 >> (bit % 8)) != 0 {
                    continue;
                }
                match space.free.last_mut() {
                    Some((_, end)) if *end == offset => *end += 64,
                    _ => space.free.push((offset, offset + 64)),
                }
            }
            amap = amap.checked_add(AMAP_DATA_SIZE)?;
        }
        Some(space)
    }

    /// Whether any free range holds a live block, B-tree page or map page.
    fn overlaps_live(
        &self,
        bbt: &BTreeLayout<UnicodeBlockBTreeEntry>,
        nbt: &BTreeLayout<UnicodeNodeBTreeEntry>,
    ) -> bool {
        // The free list is sorted and its ranges don't overlap.
        let is_free = |start: u64, end: u64| {
            let index = self
                .free
                .partition_point(|&(_, free_end)| free_end <= start);
            self.free
                .get(index)
                .is_some_and(|&(free_start, _)| free_start < end)
        };
        let trailer = u64::from(self.format.block_trailer_size());
        let blocks = bbt
            .leaves
            .iter()
            .flat_map(|(_, entries)| entries)
            .any(|entry| {
                let start = entry.block().index().index();
                let size = (u64::from(entry.size()) + trailer).div_ceil(64) * 64;
                is_free(start, start.saturating_add(size))
            });
        let page = PAGE_SIZE as u64;
        let pages = bbt
            .leaves
            .iter()
            .map(|(page_ref, _)| page_ref)
            .chain(bbt.inner.iter().flatten().map(|(page_ref, _)| page_ref))
            .chain(nbt.leaves.iter().map(|(page_ref, _)| page_ref))
            .chain(nbt.inner.iter().flatten().map(|(page_ref, _)| page_ref))
            .any(|page_ref| {
                let start = page_ref.index().index();
                is_free(start, start.saturating_add(page))
            });
        let map_count = self
            .eof
            .saturating_sub(AMAP_FIRST_OFFSET)
            .div_ceil(AMAP_DATA_SIZE);
        let maps = (0..map_count).any(|amap_index| {
            let start = AMAP_FIRST_OFFSET + amap_index * AMAP_DATA_SIZE;
            is_free(start, start + map_page_count(amap_index) * page)
        });
        blocks || pages || maps
    }
}

/// Where `size` bytes go, aligned to `alignment`. MailCraft: an error past the most a file of
/// its format can hold (an ANSI file's 2 GB).
fn reserve_file_range(space: &mut FileSpace, size: u64, alignment: u64) -> io::Result<u64> {
    let offset = reserve_file_range_unchecked(space, size, alignment);
    if offset.saturating_add(size) > space.format.max_file_size() {
        return Err(io::Error::other(
            "an ANSI data file (Outlook 97–2002's format) can't grow past 2 GB",
        ));
    }
    Ok(offset)
}

fn reserve_file_range_unchecked(space: &mut FileSpace, size: u64, alignment: u64) -> u64 {
    // ponytail: first-fit linear scan, keep a size index if fragmented files make this hot
    for index in 0..space.free.len() {
        let (start, end) = space.free[index];
        let offset = start.div_ceil(alignment) * alignment;
        if offset + size <= end {
            let rest = [(start, offset), (offset + size, end)];
            space
                .free
                .splice(index..=index, rest.into_iter().filter(|(a, b)| a < b));
            return offset;
        }
    }
    let next_offset = &mut space.eof;
    let mut offset = next_offset.div_ceil(alignment) * alignment;
    loop {
        if offset < AMAP_FIRST_OFFSET {
            *next_offset = offset + size;
            return offset;
        }

        let amap_index = (offset - AMAP_FIRST_OFFSET) / AMAP_DATA_SIZE;
        let amap_offset = AMAP_FIRST_OFFSET + amap_index * AMAP_DATA_SIZE;
        let reserved_end = amap_offset + map_page_count(amap_index) * PAGE_SIZE as u64;
        if offset < reserved_end {
            offset = reserved_end.div_ceil(alignment) * alignment;
            continue;
        }

        let next_amap = amap_offset + AMAP_DATA_SIZE;
        if offset + size > next_amap {
            let next_index = amap_index + 1;
            offset = (next_amap + map_page_count(next_index) * PAGE_SIZE as u64)
                .div_ceil(alignment)
                * alignment;
            continue;
        }

        *next_offset = offset + size;
        return offset;
    }
}

fn push_data_tree(
    blocks: &mut Vec<NewBlock>,
    next_bid: &mut u64,
    next_offset: &mut FileSpace,
    data: &[u8],
) -> io::Result<UnicodeBlockId> {
    push_data_tree_chunks(
        blocks,
        next_bid,
        next_offset,
        data.chunks(next_offset.format.data_block_size())
            .map(<[u8]>::to_vec),
    )
}

fn push_data_tree_chunks(
    blocks: &mut Vec<NewBlock>,
    next_bid: &mut u64,
    next_offset: &mut FileSpace,
    chunks: impl IntoIterator<Item = Vec<u8>>,
) -> io::Result<UnicodeBlockId> {
    let mut chunks = chunks.into_iter().collect::<Vec<_>>();
    // MailCraft: the blocks but the last are as big as the file's format makes a block.
    let block_size = next_offset.format.data_block_size();
    if chunks.is_empty() || chunks.iter().any(Vec::is_empty) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "data tree cannot contain empty blocks",
        ));
    }
    if chunks.iter().any(|chunk| chunk.len() > block_size) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "data tree block exceeds the PST block limit",
        ));
    }
    let intermediate_count = chunks.len().saturating_sub(1);
    for chunk in chunks.iter_mut().take(intermediate_count) {
        chunk.resize(block_size, 0);
    }
    let ids = chunks
        .into_iter()
        .map(|chunk| {
            let size = u32::try_from(chunk.len()).map_err(|_| PstError::IntegerConversion)?;
            let id = push_block(
                blocks,
                next_bid,
                next_offset,
                false,
                NewBlockData::Data(chunk),
            )?;
            Ok((id, size))
        })
        .collect::<io::Result<Vec<_>>>()?;
    push_data_tree_leaves(blocks, next_bid, next_offset, ids)
}

/// Build the XBLOCK levels over existing or new leaf data blocks, given as `(block, size)`.
fn push_data_tree_leaves(
    blocks: &mut Vec<NewBlock>,
    next_bid: &mut u64,
    next_offset: &mut FileSpace,
    mut ids: Vec<(UnicodeBlockId, u32)>,
) -> io::Result<UnicodeBlockId> {
    if ids.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "data tree cannot be empty",
        ));
    }
    let mut level = 1;
    // MailCraft: as many BIDs as an XBLOCK of the file's format takes.
    let capacity = next_offset.format.data_tree_capacity();
    while ids.len() > 1 {
        ids = ids
            .chunks(capacity)
            .map(|entries| {
                let total_size = entries.iter().try_fold(0_u32, |total, (_, size)| {
                    total.checked_add(*size).ok_or(PstError::IntegerConversion)
                })?;
                let id = push_block(
                    blocks,
                    next_bid,
                    next_offset,
                    true,
                    NewBlockData::DataTree(
                        entries
                            .iter()
                            .map(|(id, _)| UnicodeDataTreeEntry::from(*id))
                            .collect(),
                        level,
                        total_size,
                    ),
                )?;
                Ok((id, total_size))
            })
            .collect::<io::Result<Vec<_>>>()?;
        level += 1;
    }
    Ok(ids[0].0)
}

fn push_recipient_table(
    blocks: &mut Vec<NewBlock>,
    next_bid: &mut u64,
    next_offset: &mut FileSpace,
    recipients: &[UnicodePstRecipient<'_>],
    row_version: u32,
    text: Text,
) -> io::Result<(UnicodeBlockId, Option<UnicodeBlockId>)> {
    let (pages, rows) = recipient_table_data(recipients, row_version, text)?;
    let data = push_data_tree_chunks(blocks, next_bid, next_offset, pages)?;
    let subnodes = if let Some((rows_node, row_chunks)) = rows {
        let rows = push_data_tree_chunks(blocks, next_bid, next_offset, row_chunks)?;
        Some(push_subnode_tree(
            blocks,
            next_bid,
            next_offset,
            vec![UnicodeLeafSubNodeTreeEntry::new(rows_node, rows, None)],
        )?)
    } else {
        None
    };
    Ok((data, subnodes))
}

fn push_subnode_tree(
    blocks: &mut Vec<NewBlock>,
    next_bid: &mut u64,
    next_offset: &mut FileSpace,
    mut entries: Vec<UnicodeLeafSubNodeTreeEntry>,
) -> io::Result<UnicodeBlockId> {
    // MailCraft: as many entries as an SLBLOCK and an SIBLOCK of the file's format take.
    let leaf_capacity = next_offset.format.leaf_sub_node_capacity();
    let intermediate_capacity = next_offset.format.intermediate_sub_node_capacity();

    if entries.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "subnode tree cannot be empty",
        ));
    }
    entries.sort_by_key(|entry| u32::from(entry.node()));
    let mut level_entries = entries
        .chunks(leaf_capacity)
        .map(|chunk| {
            let first = chunk[0].node();
            let block = push_block(
                blocks,
                next_bid,
                next_offset,
                true,
                NewBlockData::SubNodes(chunk.to_vec()),
            )?;
            Ok((first, block))
        })
        .collect::<io::Result<Vec<_>>>()?;

    let mut level = 1;
    while level_entries.len() > 1 {
        level_entries = level_entries
            .chunks(intermediate_capacity)
            .map(|chunk| {
                let first = chunk[0].0;
                let entries = chunk
                    .iter()
                    .map(|(node, block)| UnicodeIntermediateSubNodeTreeEntry::new(*node, *block))
                    .collect();
                let block = push_block(
                    blocks,
                    next_bid,
                    next_offset,
                    true,
                    NewBlockData::IntermediateSubNodes(entries, level),
                )?;
                Ok((first, block))
            })
            .collect::<io::Result<Vec<_>>>()?;
        level += 1;
    }
    Ok(level_entries[0].1)
}

/// Encodes blocks in memory and sends each run of contiguous blocks to the file in one write.
/// The block encoders write field by field, and on Windows every one of those small writes is a
/// costly system call.
struct BlockWriter {
    base: u64,
    position: u64,
    buffer: Vec<u8>,
    /// MailCraft: the file's `bCryptMethod`, which its data blocks are encoded with…
    encoding: NdbCryptMethod,
    /// …and its format.
    format: Format,
}

impl BlockWriter {
    const MAX_RUN: usize = 8 << 20;

    fn new(encoding: NdbCryptMethod, format: Format) -> Self {
        Self {
            base: 0,
            position: 0,
            buffer: Vec::new(),
            encoding,
            format,
        }
    }

    fn push(&mut self, file: &mut File, block: NewBlock) -> io::Result<()> {
        let end = self.base + self.buffer.len() as u64;
        if !self.buffer.is_empty() && (block.offset != end || self.buffer.len() >= Self::MAX_RUN) {
            self.flush(file)?;
        }
        if self.buffer.is_empty() {
            self.base = block.offset;
        }
        let encoding = self.encoding;
        match self.format {
            Format::Unicode => write_new_block(self, encoding, block),
            Format::Ansi => write_ansi_block(self, encoding, block),
        }
    }

    fn flush(&mut self, file: &mut File) -> io::Result<()> {
        if !self.buffer.is_empty() {
            file.seek(SeekFrom::Start(self.base))?;
            file.write_all(&self.buffer)?;
            self.buffer.clear();
        }
        Ok(())
    }
}

impl Write for BlockWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let start =
            usize::try_from(self.position - self.base).map_err(|_| PstError::IntegerConversion)?;
        let end = start + data.len();
        if self.buffer.len() < end {
            self.buffer.resize(end, 0);
        }
        self.buffer[start..end].copy_from_slice(data);
        self.position += data.len() as u64;
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for BlockWriter {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let target = match position {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::Current(delta) => self.position.checked_add_signed(delta),
            SeekFrom::End(delta) => {
                (self.base + self.buffer.len() as u64).checked_add_signed(delta)
            }
        };
        match target {
            Some(offset) if offset >= self.base => {
                self.position = offset;
                Ok(offset)
            }
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "block writes only seek within the current run",
            )),
        }
    }
}

/// Write `block` at its offset. MailCraft: a data block is encoded with `encoding`, the file's
/// `bCryptMethod` (permute or cyclic, keyed on the block ID); XBLOCKs, XXBLOCKs, SLBLOCKs and
/// SIBLOCKs are never encoded (MS-PST 5.1, 5.2).
fn write_new_block<W: Write + Seek>(
    file: &mut W,
    encoding: NdbCryptMethod,
    block: NewBlock,
) -> io::Result<()> {
    let block_signature = block_sig::compute_sig(block.offset as u32, u64::from(block.id) as u32);
    let trailer = UnicodeBlockTrailer::new(block.size, block_signature, 0, block.id)?;
    file.seek(SeekFrom::Start(block.offset))?;
    match block.data {
        NewBlockData::Data(data) => UnicodeDataBlock::new(encoding, data, trailer)?.write(file),
        NewBlockData::DataTree(entries, level, total_size) => {
            let header = DataTreeBlockHeader::new(
                level,
                u16::try_from(entries.len()).map_err(|_| PstError::IntegerConversion)?,
                total_size,
            );
            UnicodeDataTreeBlock::new(header, entries, trailer)?.write(file)
        }
        NewBlockData::SubNodes(entries) => {
            let header = UnicodeSubNodeTreeBlockHeader::new(
                0,
                u16::try_from(entries.len()).map_err(|_| PstError::IntegerConversion)?,
            );
            let tree = UnicodeLeafSubNodeTreeBlock::new(header, entries, trailer)?;
            SubNodeTree::<UnicodePstFile>::Leaf(Box::new(tree)).write(
                file,
                &UnicodeBlockBTreeEntry::new(
                    UnicodeBlockRef::new(block.id, UnicodeByteIndex::new(block.offset)),
                    block.size,
                ),
            )
        }
        NewBlockData::IntermediateSubNodes(entries, level) => {
            let header = UnicodeSubNodeTreeBlockHeader::new(
                level,
                u16::try_from(entries.len()).map_err(|_| PstError::IntegerConversion)?,
            );
            let tree = UnicodeIntermediateSubNodeTreeBlock::new(header, entries, trailer)?;
            SubNodeTree::<UnicodePstFile>::Intermediate(Box::new(tree)).write(
                file,
                &UnicodeBlockBTreeEntry::new(
                    UnicodeBlockRef::new(block.id, UnicodeByteIndex::new(block.offset)),
                    block.size,
                ),
            )
        }
    }
}

/// MailCraft: a message's nodes as blocks: its property context (the data tree's block, the
/// first returned), and in its subnode tree (the second) its recipient table, its values too big
/// for its heap and its attachments, an attached message's own nodes alike. `flush` sends the
/// blocks so far on to the file, so a big file read from a source is never held whole.
#[allow(clippy::too_many_arguments)]
fn push_message(
    blocks: &mut Vec<NewBlock>,
    next_bid: &mut u64,
    next_offset: &mut FileSpace,
    input: &UnicodePstMessage<'_>,
    attachments: &[UnicodePstAttachment<'_>],
    named: &NamedIds,
    message: NodeId,
    row_version: u32,
    message_size: i32,
    embedded_ids: &mut EmbeddedIds,
    flush: &mut dyn FnMut(&mut Vec<NewBlock>) -> io::Result<()>,
    text: Text,
) -> io::Result<(UnicodeBlockId, UnicodeBlockId)> {
    let (message_pages, message_properties) = message_data(
        input,
        attachments,
        named,
        message,
        row_version,
        message_size,
        text,
    )?;
    let message_bid = push_data_tree_chunks(blocks, next_bid, next_offset, message_pages)?;
    let (recipient_bid, recipient_subnodes) = push_recipient_table(
        blocks,
        next_bid,
        next_offset,
        input.recipients,
        row_version,
        text,
    )?;
    let mut message_subnodes = vec![UnicodeLeafSubNodeTreeEntry::new(
        NID_RECIPIENT_TABLE,
        recipient_bid,
        recipient_subnodes,
    )];
    for (node, data) in message_properties {
        let data = push_data_tree(blocks, next_bid, next_offset, &data)?;
        message_subnodes.push(UnicodeLeafSubNodeTreeEntry::new(node, data, None));
    }
    push_attachments(
        blocks,
        next_bid,
        next_offset,
        attachments,
        named,
        row_version,
        &mut message_subnodes,
        embedded_ids,
        flush,
        text,
    )?;
    let message_subnode_bid = push_subnode_tree(blocks, next_bid, next_offset, message_subnodes)?;
    Ok((message_bid, message_subnode_bid))
}

#[allow(clippy::too_many_arguments)]
fn push_attachments(
    blocks: &mut Vec<NewBlock>,
    next_bid: &mut u64,
    next_offset: &mut FileSpace,
    attachments: &[UnicodePstAttachment<'_>],
    named: &NamedIds,
    row_version: u32,
    sub_nodes: &mut Vec<UnicodeLeafSubNodeTreeEntry>,
    embedded_ids: &mut EmbeddedIds,
    flush: &mut dyn FnMut(&mut Vec<NewBlock>) -> io::Result<()>,
    text: Text,
) -> io::Result<()> {
    if attachments.is_empty() {
        return Ok(());
    }
    let attachment_table = NID_ATTACHMENT_TABLE;
    let attachment_ids = (0..attachments.len())
        .map(|index| {
            let index = u32::try_from(index).map_err(|_| PstError::IntegerConversion)?;
            Ok(NodeId::new(NodeIdType::Attachment, 0x100 + index)?)
        })
        .collect::<io::Result<Vec<_>>>()?;
    let contents = attachment_contents(attachments, named, row_version, text)?;
    let (table_pages, table_rows) =
        attachment_table_data(&attachment_ids, attachments, &contents, row_version, text)?;
    let table_block = push_data_tree_chunks(blocks, next_bid, next_offset, table_pages)?;
    let table_subnodes = if let Some((rows_node, rows)) = table_rows {
        let rows = push_data_tree_chunks(blocks, next_bid, next_offset, rows)?;
        Some(push_subnode_tree(
            blocks,
            next_bid,
            next_offset,
            vec![UnicodeLeafSubNodeTreeEntry::new(rows_node, rows, None)],
        )?)
    } else {
        None
    };
    sub_nodes.push(UnicodeLeafSubNodeTreeEntry::new(
        attachment_table,
        table_block,
        table_subnodes,
    ));

    for (index, ((attachment_id, attachment), content)) in attachment_ids
        .into_iter()
        .zip(attachments)
        .zip(&contents)
        .enumerate()
    {
        // MailCraft: an attached message's nodes go in the attachment's own subnode tree, under
        // the node its PtypObject names (MS-PST 2.4.6.2); a file read from a source is read as
        // it is written.
        let (data_node, object, attachment_subnode) = if let Some(message) = embedded(attachment) {
            let node = embedded_ids.take()?;
            let size = content
                .checked_sub(6)
                .and_then(|size| i32::try_from(size).ok())
                .ok_or(PstError::IntegerConversion)?;
            let (data, subnodes) = push_message(
                blocks,
                next_bid,
                next_offset,
                &message.message,
                &message.attachments,
                named,
                node,
                row_version,
                size,
                embedded_ids,
                flush,
                text,
            )?;
            let tree = push_block(
                blocks,
                next_bid,
                next_offset,
                true,
                NewBlockData::SubNodes(vec![UnicodeLeafSubNodeTreeEntry::new(
                    node,
                    data,
                    Some(subnodes),
                )]),
            )?;
            (None, Some(node), Some(tree))
        } else if *content > 0 {
            let data_node = NodeId::new(
                NodeIdType::Internal,
                0x200 + u32::try_from(index).map_err(|_| PstError::IntegerConversion)?,
            )?;
            let data_block = match attachment.content {
                UnicodePstAttachmentContent::Source(source) => {
                    push_data_tree_source(blocks, next_bid, next_offset, source, *content, flush)?
                }
                _ => push_data_tree(blocks, next_bid, next_offset, attachment.data)?,
            };
            let tree = push_block(
                blocks,
                next_bid,
                next_offset,
                true,
                NewBlockData::SubNodes(vec![UnicodeLeafSubNodeTreeEntry::new(
                    data_node, data_block, None,
                )]),
            )?;
            (Some(data_node), None, Some(tree))
        } else {
            (None, None, None)
        };
        let attachment_block = push_block(
            blocks,
            next_bid,
            next_offset,
            false,
            NewBlockData::Data(attachment_data(
                attachment, data_node, *content, object, text,
            )?),
        )?;
        sub_nodes.push(UnicodeLeafSubNodeTreeEntry::new(
            attachment_id,
            attachment_block,
            attachment_subnode,
        ));
    }
    Ok(())
}

/// MailCraft: the most the writer reads of one value, a table's rows or a column kept in a
/// subnode when it opens a file to add to it (`UnicodePstFile::set_max_value_size`). A damaged
/// file's tree of blocks named over and over can declare up to 4 GiB from a few KB on disk.
pub const ADD_MAX_VALUE_SIZE: u64 = 256 * 1024 * 1024;

#[cfg(test)]
thread_local! {
    /// A lower limit for the test running on this thread.
    static TEST_ADD_MAX_VALUE_SIZE: std::cell::Cell<Option<u64>> =
        const { std::cell::Cell::new(None) };
}

/// MailCraft: the limit every file the writer reads to add to it is opened with
/// ([`AddStore::open`]).
fn add_max_value_size() -> u64 {
    #[cfg(test)]
    let limit = TEST_ADD_MAX_VALUE_SIZE
        .with(std::cell::Cell::get)
        .unwrap_or(ADD_MAX_VALUE_SIZE);
    #[cfg(not(test))]
    let limit = ADD_MAX_VALUE_SIZE;
    limit
}

fn create_unicode_pst(
    path: &Path,
    input: &UnicodePstMessage<'_>,
    attachments: &[UnicodePstAttachment<'_>],
    folder_path: Option<&[&str]>,
) -> io::Result<()> {
    if let Some(folder_path) = folder_path {
        validate_folder_path(folder_path)?;
    }
    create_empty_unicode_pst(path)?;
    let inbox = ["Inbox"];
    let result = append_unicode_pst(
        path,
        &[UnicodePstBatchMessage {
            message: *input,
            attachments,
        }],
        Some(folder_path.unwrap_or(&inbox)),
    );
    if result.is_err() {
        let _ = std::fs::remove_file(path);
    }
    result
}

/// MailCraft: Outlook's empty file, unencoded, with an Inbox: what [`create_unicode_pst`] adds its
/// first message to. The file is removed again when it can't be made whole.
fn create_empty_unicode_pst(path: &Path) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)?;
    let setup = (|| {
        file.write_all(OUTLOOK_UNICODE_PST_TEMPLATE)?;
        file.sync_data()?;

        file.seek(SeekFrom::Start(0))?;
        let mut header = UnicodeHeader::read(&mut file)?;
        let entries =
            collect_block_btree_entries(&mut file, Format::Unicode, *header.root().block_btree())?;
        for entry in entries
            .iter()
            .filter(|entry| !entry.block().block().is_internal())
        {
            file.seek(SeekFrom::Start(entry.block().index().index()))?;
            let block = UnicodeDataBlock::read(&mut file, entry.size(), header.crypt_method())?;
            file.seek(SeekFrom::Start(entry.block().index().index()))?;
            UnicodeDataBlock::new(
                NdbCryptMethod::None,
                block.data().to_vec(),
                *block.trailer(),
            )?
            .write(&mut file)?;
        }
        header.set_crypt_method(NdbCryptMethod::None);
        file.seek(SeekFrom::Start(0))?;
        header.write(&mut file)?;
        file.sync_all()
    })();
    drop(file);
    if let Err(error) = setup {
        let _ = std::fs::remove_file(path);
        return Err(error);
    }

    if let Err(error) = ensure_unicode_folder_path(path, &["Inbox"], MAIL_CONTAINER_CLASS) {
        let _ = std::fs::remove_file(path);
        return Err(error);
    }
    Ok(())
}
fn default_receive_folder(
    store: &AddStore,
    nbt_entries: &[UnicodeNodeBTreeEntry],
) -> io::Result<NodeId> {
    let ipm = store.open_folder(&store.properties().ipm_sub_tree_entry_id()?)?;
    let hierarchy = ipm
        .hierarchy_table()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "IPM hierarchy table not found"))?;
    for row in hierarchy.rows_matrix() {
        let node = NodeId::from(u32::from(row.id()));
        let folder = store.open_folder(&store.properties().make_entry_id(node)?)?;
        if folder
            .properties()
            .display_name()
            .is_ok_and(|name| name.eq_ignore_ascii_case("Inbox"))
        {
            return Ok(node);
        }
    }

    let mut best = None;
    for entry in nbt_entries
        .iter()
        .filter(|entry| matches!(entry.node().id_type(), Ok(NodeIdType::ReceiveFolderTable)))
    {
        let Ok(table) = store.table(*entry) else {
            continue;
        };
        for row in table.rows_matrix() {
            let values = row.columns(table.context())?;
            let mut message_class = None;
            let mut folder = None;
            for (column, value) in table.context().columns().iter().zip(values) {
                let Some(value) = value else { continue };
                if matches!(&value, TableRowColumnValue::Heap(id) if u32::from(*id) == 0)
                    || matches!(&value, TableRowColumnValue::Node(id) if u32::from(*id) == 0)
                {
                    continue;
                }
                match column.prop_id() {
                    0x001A => match table.read_column(&value, column.prop_type())? {
                        PropertyValue::String8(value) => message_class = Some(value.to_string()),
                        PropertyValue::Unicode(value) => message_class = Some(value.to_string()),
                        _ => {}
                    },
                    0x6605 => {
                        if let PropertyValue::Integer32(value) =
                            table.read_column(&value, column.prop_type())?
                        {
                            if value != 0 {
                                folder = Some(NodeId::from(value as u32));
                            }
                        }
                    }
                    0x0FFF => {
                        if let PropertyValue::Binary(value) =
                            table.read_column(&value, column.prop_type())?
                        {
                            folder = Some(EntryId::try_from(value.buffer())?.node_id())
                        }
                    }
                    _ => {}
                }
            }
            let Some(folder) = folder else { continue };
            let class = message_class.unwrap_or_default();
            let rank = if class.eq_ignore_ascii_case("IPM.Note") {
                3
            } else if class.eq_ignore_ascii_case("IPM") {
                2
            } else if class.is_empty() {
                1
            } else {
                0
            };
            if rank > best.map(|(rank, _)| rank).unwrap_or(0) {
                best = Some((rank, folder));
            }
        }
    }
    if let Some((_, inbox)) = best {
        return Ok(inbox);
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "default receive-folder mapping not found",
    ))
}

fn find_child_folder(store: &dyn Store, parent: NodeId, name: &str) -> io::Result<Option<NodeId>> {
    let parent = store.open_folder(&store.properties().make_entry_id(parent)?)?;
    let Some(hierarchy) = parent.hierarchy_table() else {
        return Ok(None);
    };
    for row in hierarchy.rows_matrix() {
        let child = NodeId::from(u32::from(row.id()));
        let child_folder = store.open_folder(&store.properties().make_entry_id(child)?)?;
        // MailCraft: an ANSI file's folder name by the bytes it was written in, which the
        // reader's Latin-1 doesn't give back for what Windows-1252 lacks or keeps above 0x7F.
        let found = match child_folder.properties().get(0x3001) {
            Some(PropertyValue::String8(stored)) => same_ansi_folder_name(stored.buffer(), name),
            _ => child_folder
                .properties()
                .display_name()
                .is_ok_and(|display_name| display_name.eq_ignore_ascii_case(name)),
        };
        if found {
            return Ok(Some(child));
        }
    }
    Ok(None)
}

fn new_folder_table_cell(
    column: &TableColumnDescriptor,
    store_record_key: [u8; 16],
    folder: NodeId,
    name: &str,
    container_class: &str,
    has_subfolders: bool,
    row_version: u32,
) -> io::Result<Option<TableCell>> {
    let prop_type = column.prop_type();
    // MailCraft: a folder's names are 8-bit in an ANSI file's 8-bit columns.
    let text = Format::Ansi.text();
    let cell = match column.prop_id() {
        LTP_ROW_ID_PROP_ID if prop_type == PropertyType::Integer32 => Some(TableCell::Value(
            PropertyValue::Integer32(u32::from(folder) as i32),
        )),
        LTP_ROW_VERSION_PROP_ID if prop_type == PropertyType::Integer32 => Some(TableCell::Value(
            PropertyValue::Integer32(row_version as i32),
        )),
        0x3001 => string_table_cell(prop_type, name, text),
        0x3602 | 0x3603 if prop_type == PropertyType::Integer32 => {
            Some(TableCell::Value(PropertyValue::Integer32(0)))
        }
        0x360A if prop_type == PropertyType::Boolean => {
            Some(TableCell::Value(PropertyValue::Boolean(has_subfolders)))
        }
        0x3613 => string_table_cell(prop_type, container_class, text),
        0x0FFF if prop_type == PropertyType::Binary => {
            let entry_id = EntryId::new(StoreRecordKey::new(store_record_key), folder);
            Some(TableCell::Bytes(Vec::try_from(&entry_id)?))
        }
        _ => None,
    };
    Ok(cell)
}

#[allow(clippy::too_many_arguments)]
fn rebuild_hierarchy_table(
    table: &dyn TableContext,
    store_record_key: [u8; 16],
    folder: NodeId,
    name: &str,
    container_class: &str,
    has_subfolders: bool,
    rows_node: NodeId,
    row_version: u32,
    format: Format,
) -> io::Result<RebuiltTable> {
    let cells = table
        .context()
        .columns()
        .iter()
        .map(|column| {
            new_folder_table_cell(
                column,
                store_record_key,
                folder,
                name,
                container_class,
                has_subfolders,
                row_version,
            )
        })
        .collect::<io::Result<Vec<_>>>()?;
    rebuild_table(
        table,
        vec![(folder, cells)],
        Some(rows_node),
        row_version,
        None,
        true,
        format,
    )
}

fn table_rows_node(
    table: &dyn TableContext,
    subnodes: &[UnicodeLeafSubNodeTreeEntry],
) -> io::Result<NodeId> {
    table
        .context()
        .rows()
        .filter(|node| !matches!(node.id_type(), Ok(NodeIdType::HeapNode)))
        .map(Ok)
        .unwrap_or_else(|| {
            let index = subnodes
                .iter()
                .filter(|entry| matches!(entry.node().id_type(), Ok(NodeIdType::Internal)))
                .map(|entry| entry.node().index())
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or(PstError::IntegerConversion)?;
            Ok::<_, io::Error>(NodeId::new(NodeIdType::Internal, index)?)
        })
}

fn writable_folder_properties(folder: &dyn Folder) -> Vec<(u16, PropertyValue)> {
    folder
        .properties()
        .iter()
        .filter(|(id, _)| !matches!(**id, 0x0FFF | 0x3601))
        .map(|(id, value)| (*id, value.clone()))
        .collect()
}

fn set_folder_integer_property(properties: &mut Vec<(u16, PropertyValue)>, id: u16, value: i32) {
    if let Some((_, property)) = properties.iter_mut().find(|(property, _)| *property == id) {
        *property = PropertyValue::Integer32(value);
    } else {
        properties.push((id, PropertyValue::Integer32(value)));
    }
}

fn set_folder_boolean_property(properties: &mut Vec<(u16, PropertyValue)>, id: u16, value: bool) {
    if let Some((_, property)) = properties.iter_mut().find(|(property, _)| *property == id) {
        *property = PropertyValue::Boolean(value);
    } else {
        properties.push((id, PropertyValue::Boolean(value)));
    }
}

fn write_pending_blocks(
    file: &mut File,
    writer: &mut BlockWriter,
    bbt_entries: &mut Vec<UnicodeBlockBTreeEntry>,
    blocks: &mut Vec<NewBlock>,
) -> io::Result<()> {
    #[cfg(test)]
    extra::note_pending(blocks);
    for block in blocks.drain(..) {
        bbt_entries.push(UnicodeBlockBTreeEntry::new(
            UnicodeBlockRef::new(block.id, UnicodeByteIndex::new(block.offset)),
            block.size,
        ));
        writer.push(file, block)?;
    }
    Ok(())
}

fn append_search_updates(
    store: &dyn Store,
    nbt_entries: &mut Vec<UnicodeNodeBTreeEntry>,
    blocks: &mut Vec<NewBlock>,
    next_bid: &mut u64,
    next_offset: &mut FileSpace,
    updates: impl IntoIterator<Item = SearchUpdateData>,
) -> io::Result<()> {
    let Some(queue_entry) = nbt_entries
        .iter()
        .find(|entry| entry.node() == NID_SEARCH_MANAGEMENT_QUEUE)
        .copied()
    else {
        return Ok(());
    };
    let mut data = Vec::new();
    for update in store
        .search_update_queue()?
        .updates()
        .iter()
        .copied()
        .chain(updates.into_iter().map(SearchUpdate::new))
    {
        update.write(&mut data)?;
    }
    let data = push_data_tree(blocks, next_bid, next_offset, &data)?;
    nbt_entries.retain(|entry| entry.node() != NID_SEARCH_MANAGEMENT_QUEUE);
    nbt_entries.push(UnicodeNodeBTreeEntry::new(
        NID_SEARCH_MANAGEMENT_QUEUE,
        data,
        queue_entry.sub_node(),
        queue_entry.parent(),
    ));
    Ok(())
}

fn record_block_reference(
    block: UnicodeBlockId,
    references: &mut BTreeMap<u64, u16>,
    pending: &mut VecDeque<UnicodeBlockId>,
) -> io::Result<()> {
    let key = block.search_key();
    if key == 0 {
        return Ok(());
    }
    let count = references.entry(key).or_default();
    if *count == 0 {
        pending.push_back(block);
    }
    *count = count.checked_add(1).ok_or(PstError::IntegerConversion)?;
    Ok(())
}

fn internal_block_children(
    file: &mut File,
    format: Format,
    entry: &UnicodeBlockBTreeEntry,
) -> io::Result<Vec<UnicodeBlockId>> {
    file.seek(SeekFrom::Start(entry.block().index().index()))?;
    let mut block_type = [0];
    file.read_exact(&mut block_type)?;
    Ok(match block_type[0] {
        // Internal blocks (XBLOCKs, SLBLOCKs, SIBLOCKs) are never encoded (MS-PST 5.1, 5.2).
        0x01 => match read_tree_block(file, format, NdbCryptMethod::None, entry)? {
            TreeBlock::Intermediate { children, .. } => children,
            // MailCraft: an error rather than `unreachable!` (a leaf's ID is never internal).
            TreeBlock::Leaf(_) => return Err(NdbError::InvalidInternalBlockType(0x01).into()),
        },
        0x02 => match read_sub_node_block(file, format, entry)? {
            SubNodeBlock::Intermediate(children) => children,
            SubNodeBlock::Leaf(entries) => entries
                .iter()
                .flat_map(|entry| [Some(entry.block()), entry.sub_node()])
                .flatten()
                .collect(),
        },
        value => return Err(NdbError::InvalidInternalBlockType(value).into()),
    })
}

/// Update block reference counts from what this commit changed instead of walking every
/// internal block: node entries replaced or added, new internal blocks, and internal blocks
/// that lose their last reference. Returns false when the stored counts do not add up, so the
/// caller can fall back to [`compact_unicode_block_tree`].
fn update_block_ref_counts(
    file: &mut File,
    format: Format,
    old_blocks: &[UnicodeBlockBTreeEntry],
    old_nodes: &[UnicodeNodeBTreeEntry],
    entries: &mut Vec<UnicodeBlockBTreeEntry>,
    nodes: &[UnicodeNodeBTreeEntry],
) -> io::Result<bool> {
    entries.sort_by_key(BTreeEntry::key);
    let mut delta = HashMap::<u64, i32>::new();
    let add = |delta: &mut HashMap<u64, i32>, block: Option<UnicodeBlockId>, change| {
        if let Some(key) = block
            .map(|block| block.search_key())
            .filter(|key| *key != 0)
        {
            *delta.entry(key).or_default() += change;
        }
    };

    let (mut old, mut new) = (old_nodes.iter().peekable(), nodes.iter().peekable());
    loop {
        let order = match (old.peek(), new.peek()) {
            (None, None) => break,
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (Some(a), Some(b)) => a.key().cmp(&b.key()),
        };
        let (removed, added) = match order {
            std::cmp::Ordering::Less => (old.next(), None),
            std::cmp::Ordering::Greater => (None, new.next()),
            std::cmp::Ordering::Equal => {
                let (a, b) = (old.next(), new.next());
                if a.zip(b).is_some_and(|(a, b)| same_btree_entry(a, b)) {
                    continue;
                }
                (a, b)
            }
        };
        for (node, change) in [(removed, -1), (added, 1)] {
            if let Some(node) = node {
                add(&mut delta, Some(node.data()), change);
                add(&mut delta, node.sub_node(), change);
            }
        }
    }

    let is_old = |key: u64| {
        old_blocks
            .binary_search_by_key(&key, BTreeEntry::key)
            .is_ok()
    };
    for entry in entries.iter() {
        if entry.block().block().is_internal() && !is_old(entry.key()) {
            for child in internal_block_children(file, format, entry)? {
                add(&mut delta, Some(child), 1);
            }
        }
    }

    let mut pending = delta.keys().copied().collect::<Vec<_>>();
    let mut released = HashSet::new();
    while let Some(key) = pending.pop() {
        let Ok(index) = entries.binary_search_by_key(&key, BTreeEntry::key) else {
            return Ok(false);
        };
        let count = i32::from(entries[index].ref_count()) + delta.get(&key).copied().unwrap_or(0);
        if count < 1 {
            return Ok(false);
        }
        if count == 1 && entries[index].block().block().is_internal() && released.insert(key) {
            let entry = entries[index];
            for child in internal_block_children(file, format, &entry)? {
                add(&mut delta, Some(child), -1);
                pending.push(child.search_key());
            }
        }
    }

    for (key, change) in &delta {
        let index = entries
            .binary_search_by_key(key, BTreeEntry::key)
            .map_err(|_| PstError::IntegerConversion)?;
        let count = i32::from(entries[index].ref_count()) + change;
        let Ok(count) = u16::try_from(count) else {
            return Ok(false);
        };
        entries[index].set_ref_count(count);
    }
    entries.retain(|entry| entry.ref_count() > 1 || !delta.contains_key(&entry.key()));
    Ok(true)
}

fn compact_unicode_block_tree(
    file: &mut File,
    format: Format,
    entries: &mut Vec<UnicodeBlockBTreeEntry>,
    nodes: &[UnicodeNodeBTreeEntry],
) -> io::Result<()> {
    entries.sort_by_key(BTreeEntry::key);
    let mut references = BTreeMap::new();
    let mut pending = VecDeque::new();
    for node in nodes {
        record_block_reference(node.data(), &mut references, &mut pending)?;
        if let Some(block) = node.sub_node() {
            record_block_reference(block, &mut references, &mut pending)?;
        }
    }

    while let Some(block) = pending.pop_front() {
        let entry = entries
            .binary_search_by_key(&block.search_key(), BTreeEntry::key)
            .ok()
            .and_then(|index| entries.get(index))
            .copied()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("referenced PST block {:X} is missing", block.search_key()),
                )
            })?;
        if !block.is_internal() {
            continue;
        }
        for child in internal_block_children(file, format, &entry)? {
            record_block_reference(child, &mut references, &mut pending)?;
        }
    }

    for entry in &mut *entries {
        if let Some(references) = references.get(&entry.key()) {
            entry.set_ref_count(
                references
                    .checked_add(1)
                    .ok_or(PstError::IntegerConversion)?,
            );
        }
    }
    entries.retain(|entry| references.contains_key(&entry.key()));
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn commit_unicode_changes(
    path: &Path,
    mut file: File,
    mut header: FileHeader,
    old_bbt: &BTreeLayout<UnicodeBlockBTreeEntry>,
    old_nbt: &BTreeLayout<UnicodeNodeBTreeEntry>,
    mut bbt_entries: Vec<UnicodeBlockBTreeEntry>,
    mut nbt_entries: Vec<UnicodeNodeBTreeEntry>,
    blocks: Vec<NewBlock>,
    next_bid: u64,
    mut next_offset: FileSpace,
    reserved_nodes: &[NodeId],
) -> io::Result<()> {
    bbt_entries.extend(blocks.iter().map(|block| {
        UnicodeBlockBTreeEntry::new(
            UnicodeBlockRef::new(block.id, UnicodeByteIndex::new(block.offset)),
            block.size,
        )
    }));
    nbt_entries.sort_by_key(BTreeEntry::key);

    // New blocks and pages only go to space the committed allocation map marks free, so the
    // committed state stays intact until the header switches roots: one barrier before that
    // switch and one after it (the allocation map is invalid until it is rebuilt) suffice.
    let format = header.format();
    header.set_amap_status(AmapStatus::Invalid);
    header.update_unique();
    let mut writer = BlockWriter::new(header.crypt_method(), format);
    for block in blocks {
        writer.push(&mut file, block)?;
    }
    writer.flush(&mut file)?;
    #[cfg(test)]
    let before = bbt_entries.clone();
    if !update_block_ref_counts(
        &mut file,
        format,
        &old_bbt.entries(),
        &old_nbt.entries(),
        &mut bbt_entries,
        &nbt_entries,
    )? {
        compact_unicode_block_tree(&mut file, format, &mut bbt_entries, &nbt_entries)?;
    }
    #[cfg(test)]
    {
        let mut full = before;
        compact_unicode_block_tree(&mut file, format, &mut full, &nbt_entries)?;
        let counts = |entries: &[UnicodeBlockBTreeEntry]| {
            entries
                .iter()
                .map(|entry| (entry.key(), entry.ref_count()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            counts(&bbt_entries),
            counts(&full),
            "incremental block ref counts"
        );
    }

    let mut pages = Vec::new();
    let mut next_page = header.next_page();
    let mut live_pages = Vec::new();
    // MailCraft: as many entries as a page of the file's format holds.
    let (block_max, node_max, inner_max) = format.btree_capacity();
    let bbt_ref = plan_btree(
        old_bbt,
        &bbt_entries,
        (block_max, inner_max),
        PageType::BlockBTree,
        NewBTreePageData::BlockLeaf,
        &mut pages,
        &mut next_page,
        &mut next_offset,
        &mut live_pages,
    )?;
    let nbt_ref = plan_btree(
        old_nbt,
        &nbt_entries,
        (node_max, inner_max),
        PageType::NodeBTree,
        NewBTreePageData::NodeLeaf,
        &mut pages,
        &mut next_page,
        &mut next_offset,
        &mut live_pages,
    )?;
    let new_eof = next_offset.eof;
    // MailCraft: saturating, where an end of file before the first map overflowed.
    let amap_last = AMAP_FIRST_OFFSET
        + (new_eof.saturating_sub(AMAP_FIRST_OFFSET + 1) / AMAP_DATA_SIZE) * AMAP_DATA_SIZE;

    for page in pages {
        write_btree_page(&mut file, format, page)?;
    }
    file.set_len(new_eof)?;
    file.sync_data()?;

    header.set_next_block(next_bid)?;
    header.set_next_page(next_page)?;
    for node in reserved_nodes {
        header.reserve_node(*node)?;
    }
    header.set_btrees(nbt_ref, bbt_ref)?;
    header.set_file_eof(new_eof, amap_last)?;
    header.update_unique();
    header.write(&mut file)?;
    file.sync_data()?;
    drop(file);
    let blocks = bbt_entries
        .iter()
        .map(|entry| (entry.block().index().index(), entry.size()))
        .collect::<Vec<_>>();
    finish_metadata(path, &blocks, &live_pages)
}

fn set_unicode_store_display_name(path: &Path, name: &str) -> io::Result<()> {
    if name.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "store display name must not be empty",
        ));
    }
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    // MailCraft: an encoded file is renamed too; its new blocks are encoded like the rest.
    let header = FileHeader::read(&mut file)?;
    // MailCraft: an ANSI file's too, its values widened.
    let format = header.format();
    let old_bbt = read_block_btree(&mut file, format, header.block_btree())?;
    let old_nbt = read_node_btree(&mut file, format, header.node_btree())?;
    let bbt_entries = old_bbt.entries();
    let mut nbt_entries = old_nbt.entries();
    let store_entry = nbt_entries
        .iter()
        .find(|entry| entry.node() == NID_MESSAGE_STORE)
        .copied()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "message store node not found"))?;
    let store = AddStore::open(path)?;
    let mut properties: Vec<_> = store
        .properties()
        .iter()
        .filter(|(id, _)| **id != 0x3001)
        .map(|(id, value)| (*id, value.clone()))
        .collect();
    properties.push((0x3001, format.text().value(name)));

    let mut next_bid = header.next_block_index().max(
        bbt_entries
            .iter()
            .map(|entry| entry.block().block().index())
            .max()
            .unwrap_or(0)
            .saturating_add(1),
    );
    let mut next_offset = FileSpace::read(&mut file, &header, &old_bbt, &old_nbt)?;
    let mut blocks = Vec::new();
    let mut subnodes = collect_subnode_entries(
        &mut file,
        format,
        header.block_btree(),
        store_entry.sub_node(),
    )?;
    let next_property_subnode = subnodes
        .iter()
        .filter(|entry| matches!(entry.node().id_type(), Ok(NodeIdType::Internal)))
        .map(|entry| entry.node().index())
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(PstError::IntegerConversion)?;
    let (pages, external_properties) =
        property_context_pages(properties, next_property_subnode, format.text())?;
    let data = push_data_tree_chunks(&mut blocks, &mut next_bid, &mut next_offset, pages)?;
    for (node, value) in external_properties {
        let value = push_data_tree(&mut blocks, &mut next_bid, &mut next_offset, &value)?;
        subnodes.push(UnicodeLeafSubNodeTreeEntry::new(node, value, None));
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
    nbt_entries.retain(|entry| entry.node() != NID_MESSAGE_STORE);
    nbt_entries.push(UnicodeNodeBTreeEntry::new(
        NID_MESSAGE_STORE,
        data,
        subnode,
        store_entry.parent(),
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
        next_offset,
        &[],
    )
}

fn create_unicode_folder(
    path: &Path,
    parent: NodeId,
    name: &str,
    container_class: &str,
) -> io::Result<NodeId> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    // MailCraft: folders are made in an encoded file too; its new blocks are encoded like the
    // rest.
    let header = FileHeader::read(&mut file)?;
    // MailCraft: an ANSI file's too, its values widened.
    let format = header.format();
    let old_bbt = read_block_btree(&mut file, format, header.block_btree())?;
    let old_nbt = read_node_btree(&mut file, format, header.node_btree())?;
    let bbt_entries = old_bbt.entries();
    let mut nbt_entries = old_nbt.entries();
    let store = AddStore::open(path)?;
    let store_record_key = *store.properties().record_key()?.record_key();
    let parent_folder = store.open_folder(&store.properties().make_entry_id(parent)?)?;
    let parent_has_subfolders = parent_folder.properties().has_sub_folders()?;
    let parent_entry = nbt_entries
        .iter()
        .find(|entry| entry.node() == parent)
        .copied()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "parent folder node not found"))?;
    let mut parent_properties = writable_folder_properties(parent_folder.as_ref());
    if !parent_has_subfolders {
        set_folder_boolean_property(&mut parent_properties, 0x360A, true);
    }
    let mut parent_subnodes = collect_subnode_entries(
        &mut file,
        format,
        header.block_btree(),
        parent_entry.sub_node(),
    )?;
    let parent_row_hierarchy = (!parent_has_subfolders)
        .then(|| -> io::Result<_> {
            let grandparent = parent_entry.parent().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "parent folder has no parent")
            })?;
            let node = NodeId::new(NodeIdType::HierarchyTable, grandparent.index())?;
            let entry = nbt_entries
                .iter()
                .find(|entry| entry.node() == node)
                .copied()
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, "parent hierarchy node not found")
                })?;
            let grandparent = store.open_folder(&store.properties().make_entry_id(grandparent)?)?;
            let table = grandparent.hierarchy_table().cloned().ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "parent hierarchy table not found")
            })?;
            let subnodes =
                collect_subnode_entries(&mut file, format, header.block_btree(), entry.sub_node())?;
            let rows_node = table_rows_node(table.as_ref(), &subnodes)?;
            Ok((node, entry, table, subnodes, rows_node))
        })
        .transpose()?;

    let folder_index = nbt_entries
        .iter()
        .filter(|entry| matches!(entry.node().id_type(), Ok(NodeIdType::NormalFolder)))
        .map(|entry| entry.node().index())
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(PstError::IntegerConversion)?
        .max(header.next_node_index(NodeIdType::NormalFolder)?);
    let folder = NodeId::new(NodeIdType::NormalFolder, folder_index)?;
    let hierarchy = NodeId::new(NodeIdType::HierarchyTable, parent.index())?;
    let hierarchy_entry = nbt_entries
        .iter()
        .find(|entry| entry.node() == hierarchy)
        .copied();
    let hierarchy_template = nbt_entries
        .iter()
        .find(|entry| entry.node() == NodeId::from(0x60D))
        .map(NodeBTreeEntry::data);
    let contents_template = nbt_entries
        .iter()
        .find(|entry| entry.node() == NodeId::from(0x60E))
        .map(NodeBTreeEntry::data);
    let associated_contents_template = nbt_entries
        .iter()
        .find(|entry| entry.node() == NodeId::from(0x60F))
        .map(NodeBTreeEntry::data);

    let mut next_bid = header.next_block_index().max(
        bbt_entries
            .iter()
            .map(|entry| entry.block().block().index())
            .max()
            .unwrap_or(0)
            .saturating_add(1),
    );
    let mut next_offset = FileSpace::read(&mut file, &header, &old_bbt, &old_nbt)?;
    let mut blocks = Vec::new();

    let (hierarchy_data_bid, hierarchy_subnode_bid) =
        if let (Some(hierarchy_entry), Some(hierarchy_table)) =
            (hierarchy_entry, parent_folder.hierarchy_table())
        {
            let mut hierarchy_subnodes = collect_subnode_entries(
                &mut file,
                format,
                header.block_btree(),
                hierarchy_entry.sub_node(),
            )?;
            let rows_node = table_rows_node(hierarchy_table.as_ref(), &hierarchy_subnodes)?;
            let (pages, rows, _) = rebuild_hierarchy_table(
                hierarchy_table.as_ref(),
                store_record_key,
                folder,
                name,
                container_class,
                false,
                rows_node,
                header.unique_value(),
                format,
            )?;
            let data = push_data_tree_chunks(&mut blocks, &mut next_bid, &mut next_offset, pages)?;
            let rows = push_data_tree_chunks(&mut blocks, &mut next_bid, &mut next_offset, rows)?;
            hierarchy_subnodes.retain(|entry| entry.node() != rows_node);
            hierarchy_subnodes.push(UnicodeLeafSubNodeTreeEntry::new(rows_node, rows, None));
            let subnodes = push_subnode_tree(
                &mut blocks,
                &mut next_bid,
                &mut next_offset,
                hierarchy_subnodes,
            )?;
            (data, Some(subnodes))
        } else {
            let data = push_block(
                &mut blocks,
                &mut next_bid,
                &mut next_offset,
                false,
                NewBlockData::Data(table_context_data(&[folder], format)?),
            )?;
            (data, None)
        };

    let (parent_data, parent_subnode) = if parent_has_subfolders {
        (parent_entry.data(), parent_entry.sub_node())
    } else {
        let next_property_subnode = parent_subnodes
            .iter()
            .filter(|entry| matches!(entry.node().id_type(), Ok(NodeIdType::Internal)))
            .map(|entry| entry.node().index())
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(PstError::IntegerConversion)?;
        let (parent_pages, external_properties) =
            property_context_pages(parent_properties, next_property_subnode, format.text())?;
        let parent_data =
            push_data_tree_chunks(&mut blocks, &mut next_bid, &mut next_offset, parent_pages)?;
        for (node, data) in external_properties {
            let data = push_data_tree(&mut blocks, &mut next_bid, &mut next_offset, &data)?;
            parent_subnodes.push(UnicodeLeafSubNodeTreeEntry::new(node, data, None));
        }
        let parent_subnode = if parent_subnodes.is_empty() {
            None
        } else {
            Some(push_subnode_tree(
                &mut blocks,
                &mut next_bid,
                &mut next_offset,
                parent_subnodes,
            )?)
        };
        (parent_data, parent_subnode)
    };

    let folder_data_bid = push_block(
        &mut blocks,
        &mut next_bid,
        &mut next_offset,
        false,
        NewBlockData::Data(folder_data(name, 0, false, container_class, format.text())?),
    )?;
    let folder_hierarchy = NodeId::new(NodeIdType::HierarchyTable, folder.index())?;
    let folder_hierarchy_bid = match hierarchy_template {
        Some(template) => template,
        None => push_block(
            &mut blocks,
            &mut next_bid,
            &mut next_offset,
            false,
            NewBlockData::Data(table_context_data(&[], format)?),
        )?,
    };
    let folder_contents = NodeId::new(NodeIdType::ContentsTable, folder.index())?;
    let folder_contents_bid = match contents_template {
        Some(template) => template,
        None => push_block(
            &mut blocks,
            &mut next_bid,
            &mut next_offset,
            false,
            NewBlockData::Data(table_context_data(&[], format)?),
        )?,
    };
    let folder_associated_contents =
        NodeId::new(NodeIdType::AssociatedContentsTable, folder.index())?;
    let folder_associated_contents_bid = match associated_contents_template {
        Some(template) => template,
        None => push_block(
            &mut blocks,
            &mut next_bid,
            &mut next_offset,
            false,
            NewBlockData::Data(table_context_data(&[], format)?),
        )?,
    };

    let parent_row_hierarchy_entry = if let Some((node, entry, table, mut subnodes, rows_node)) =
        parent_row_hierarchy
    {
        let (pages, rows, _) = rebuild_table(
            table.as_ref(),
            vec![],
            Some(rows_node),
            header.unique_value(),
            Some((parent, &[(0x360A, PropertyValue::Boolean(true))])),
            true,
            format,
        )?;
        let data = push_data_tree_chunks(&mut blocks, &mut next_bid, &mut next_offset, pages)?;
        let rows = push_data_tree_chunks(&mut blocks, &mut next_bid, &mut next_offset, rows)?;
        subnodes.retain(|entry| entry.node() != rows_node);
        subnodes.push(UnicodeLeafSubNodeTreeEntry::new(rows_node, rows, None));
        let subnodes = push_subnode_tree(&mut blocks, &mut next_bid, &mut next_offset, subnodes)?;
        Some((
            node,
            UnicodeNodeBTreeEntry::new(node, data, Some(subnodes), entry.parent()),
        ))
    } else {
        None
    };

    nbt_entries.retain(|entry| entry.node() != parent && entry.node() != hierarchy);
    if let Some((node, entry)) = parent_row_hierarchy_entry {
        nbt_entries.retain(|candidate| candidate.node() != node);
        nbt_entries.push(entry);
    }
    nbt_entries.extend([
        UnicodeNodeBTreeEntry::new(parent, parent_data, parent_subnode, parent_entry.parent()),
        UnicodeNodeBTreeEntry::new(
            hierarchy,
            hierarchy_data_bid,
            hierarchy_subnode_bid,
            hierarchy_entry.and_then(|entry| entry.parent()),
        ),
        UnicodeNodeBTreeEntry::new(folder, folder_data_bid, None, Some(parent)),
        UnicodeNodeBTreeEntry::new(folder_hierarchy, folder_hierarchy_bid, None, None),
        UnicodeNodeBTreeEntry::new(folder_contents, folder_contents_bid, None, None),
        UnicodeNodeBTreeEntry::new(
            folder_associated_contents,
            folder_associated_contents_bid,
            None,
            None,
        ),
    ]);

    append_search_updates(
        &*store,
        &mut nbt_entries,
        &mut blocks,
        &mut next_bid,
        &mut next_offset,
        [
            SearchUpdateData::FolderAdded {
                parent,
                folder,
                reserved1: 0,
                reserved2: 0,
            },
            SearchUpdateData::FolderModified {
                folder: parent,
                reserved: 0,
            },
        ],
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
        &[
            folder,
            folder_hierarchy,
            folder_contents,
            folder_associated_contents,
        ],
    )?;
    Ok(folder)
}

/// MailCraft: the folders made on the way get `PR_CONTAINER_CLASS` IPF.Note, the last one
/// `container_class`; folders already there keep theirs.
fn ensure_unicode_folder_path(
    path: &Path,
    folder_path: &[&str],
    container_class: &str,
) -> io::Result<NodeId> {
    validate_folder_path(folder_path)?;
    let mut store = AddStore::open(path)?;
    let mut parent = store.properties().ipm_sub_tree_entry_id()?.node_id();
    for (index, name) in folder_path.iter().enumerate() {
        if let Some(folder) = find_child_folder(&*store, parent, name)? {
            parent = folder;
            continue;
        }
        // ponytail: one COW commit per missing segment; batch only if deep imports need it.
        let class = if index + 1 == folder_path.len() {
            container_class
        } else {
            MAIL_CONTAINER_CLASS
        };
        parent = create_unicode_folder(path, parent, name, class)?;
        store = AddStore::open(path)?;
    }
    Ok(parent)
}

fn append_unicode_pst(
    path: &Path,
    inputs: &[UnicodePstBatchMessage<'_>],
    folder_path: Option<&[&str]>,
) -> io::Result<()> {
    if inputs.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "bulk append requires at least one message",
        ));
    }
    for input in inputs {
        check_message(&input.message)?;
        // MailCraft: and its attachments, attached messages and all.
        check_attachments(input.attachments, 0)?;
    }
    let requested_folder = folder_path
        .map(|folder_path| ensure_unicode_folder_path(path, folder_path, MAIL_CONTAINER_CLASS))
        .transpose()?;
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let mut header = FileHeader::read(&mut file)?;
    // MailCraft: a permute- or cyclic-encoded file (Outlook's default is permute) is added to
    // as well: its blocks are read decoded and new data blocks are written encoded alike.
    let encoding = header.crypt_method();
    // MailCraft: an ANSI file's too, its values widened.
    let format = header.format();
    let old_bbt = read_block_btree(&mut file, format, header.block_btree())?;
    let old_nbt = read_node_btree(&mut file, format, header.node_btree())?;
    let mut bbt_entries = old_bbt.entries();
    let mut nbt_entries = old_nbt.entries();

    let store = AddStore::open(path)
        .map_err(|err| io::Error::new(err.kind(), format!("read message store: {err}")))?;
    let store_record_key = *store
        .properties()
        .record_key()
        .map_err(|err| io::Error::new(err.kind(), format!("read store record key: {err}")))?
        .record_key();
    // MailCraft: the IDs of the named properties the messages have, added to the file's map
    // when it doesn't have them.
    let named = resolve_named(&*store, inputs)
        .map_err(|err| io::Error::new(err.kind(), format!("name-to-ID map: {err}")))?;
    let folder =
        requested_folder
            .unwrap_or(default_receive_folder(&store, &nbt_entries).map_err(|err| {
                io::Error::new(err.kind(), format!("find receive folder: {err}"))
            })?);
    let target_folder = store
        .open_folder(&store.properties().make_entry_id(folder)?)
        .map_err(|err| io::Error::new(err.kind(), format!("open target folder: {err}")))?;
    let contents_table = target_folder.contents_table().cloned().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "target-folder contents table not found",
        )
    })?;
    let mut folder_properties = writable_folder_properties(target_folder.as_ref());

    let message_index = nbt_entries
        .iter()
        .filter(|entry| matches!(entry.node().id_type(), Ok(NodeIdType::NormalMessage)))
        .map(|entry| entry.node().index())
        .max()
        .unwrap_or(0x3F)
        .checked_add(1)
        .ok_or(PstError::IntegerConversion)?
        .max(header.next_node_index(NodeIdType::NormalMessage)?);
    let messages = (0..inputs.len())
        .map(|offset| {
            let offset = u32::try_from(offset).map_err(|_| PstError::IntegerConversion)?;
            let index = message_index
                .checked_add(offset)
                .ok_or(PstError::IntegerConversion)?;
            Ok::<_, io::Error>(NodeId::new(NodeIdType::NormalMessage, index)?)
        })
        .collect::<io::Result<Vec<_>>>()?;
    // MailCraft: attached messages get message IDs after these.
    let mut embedded_ids = EmbeddedIds::new(
        u32::try_from(inputs.len())
            .ok()
            .and_then(|count| message_index.checked_add(count))
            .ok_or(PstError::IntegerConversion)?,
    );
    // MailCraft: how each message's strings are written (8-bit in an ANSI file).
    let texts = inputs
        .iter()
        .map(|input| message_text(format, &input.message, input.attachments))
        .collect::<Vec<_>>();
    let message_sizes = inputs
        .iter()
        .zip(&messages)
        .zip(&texts)
        .enumerate()
        .map(|(position, ((input, message), text))| {
            message_size(
                &input.message,
                input.attachments,
                &named,
                *message,
                header.unique_value(),
                *text,
            )
            .map_err(|err| {
                let file_size = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
                io::Error::new(
                    err.kind(),
                    format!(
                        "append message {} of {} ({}) at PST size {file_size}: {err}",
                        position + 1,
                        inputs.len(),
                        input.message.message_id,
                    ),
                )
            })
        })
        .collect::<io::Result<Vec<_>>>()?;
    let contents = NodeId::new(NodeIdType::ContentsTable, folder.index())?;
    let contents_entry = nbt_entries
        .iter()
        .find(|entry| entry.node() == contents)
        .copied()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "target contents node not found"))?;
    let folder_entry = nbt_entries
        .iter()
        .find(|entry| entry.node() == folder)
        .copied()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "target folder node not found"))?;
    let parent_hierarchy = folder_entry
        .parent()
        .map(|parent| -> io::Result<_> {
            let node = NodeId::new(NodeIdType::HierarchyTable, parent.index())?;
            let entry = nbt_entries
                .iter()
                .find(|entry| entry.node() == node)
                .copied()
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, "parent hierarchy node not found")
                })?;
            let parent = store.open_folder(&store.properties().make_entry_id(parent)?)?;
            let table = parent.hierarchy_table().cloned().ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "parent hierarchy table not found")
            })?;
            let subnodes =
                collect_subnode_entries(&mut file, format, header.block_btree(), entry.sub_node())?;
            let rows_node = table_rows_node(table.as_ref(), &subnodes)?;
            Ok((node, entry, table, subnodes, rows_node))
        })
        .transpose()?;
    let mut folder_subnodes = collect_subnode_entries(
        &mut file,
        format,
        header.block_btree(),
        folder_entry.sub_node(),
    )?;
    let mut contents_subnodes = collect_subnode_entries(
        &mut file,
        format,
        header.block_btree(),
        contents_entry.sub_node(),
    )?;
    let rows_node = table_rows_node(contents_table.as_ref(), &contents_subnodes)?;

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

    let mut message_entries = Vec::with_capacity(inputs.len());
    let mut block_writer = BlockWriter::new(encoding, format);
    for (position, (((input, message), message_size), text)) in inputs
        .iter()
        .zip(&messages)
        .zip(&message_sizes)
        .zip(&texts)
        .enumerate()
    {
        let mut blocks = Vec::new();
        let result = (|| {
            // MailCraft: blocks go on to the file as a big file is read, and at the end.
            let mut flush = |blocks: &mut Vec<NewBlock>| {
                write_pending_blocks(&mut file, &mut block_writer, &mut bbt_entries, blocks)
            };
            let (message_bid, message_subnode_bid) = push_message(
                &mut blocks,
                &mut next_bid,
                &mut next_offset,
                &input.message,
                input.attachments,
                &named,
                *message,
                header.unique_value(),
                *message_size,
                &mut embedded_ids,
                &mut flush,
                *text,
            )?;
            flush(&mut blocks)?;
            Ok::<_, io::Error>(UnicodeNodeBTreeEntry::new(
                *message,
                message_bid,
                Some(message_subnode_bid),
                Some(folder),
            ))
        })();
        message_entries.push(result.map_err(|err| {
            let file_size = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
            io::Error::new(
                err.kind(),
                format!(
                    "append message {} of {} ({}) at PST size {file_size}: {err}",
                    position + 1,
                    inputs.len(),
                    input.message.message_id,
                ),
            )
        })?);
    }

    // Cell subnodes are NID_TYPE_LTP and the row matrix is internal, so the next index counts every type.
    block_writer.flush(&mut file)?;

    let next_contents_subnode = contents_subnodes
        .iter()
        .map(|entry| entry.node().index())
        .max()
        .unwrap_or(0)
        .max(rows_node.index())
        .checked_add(1)
        .ok_or(PstError::IntegerConversion)?;
    let mut blocks = Vec::new();
    let (contents_data_bid, rows_bid, row_count, external_contents) = (|| {
        let (mut new_rows, external) = new_contents_rows(
            contents_table.as_ref(),
            inputs,
            &named,
            store_record_key,
            folder,
            &messages,
            &message_sizes,
            next_contents_subnode,
            header.unique_value(),
            &texts,
        )?;
        let old_rows_bid = contents_subnodes
            .iter()
            .find(|entry| entry.node() == rows_node)
            .map(|entry| entry.block());
        if let Some((heap, rows, count)) = append_table_rows(
            &mut file,
            encoding,
            &bbt_entries,
            contents_table.as_ref(),
            contents_entry.data(),
            old_rows_bid,
            &mut new_rows,
            header.unique_value(),
            &mut blocks,
            &mut next_bid,
            &mut next_offset,
        )? {
            return Ok((heap, rows, count, external));
        }
        let (heap, rows, row_ids) = rebuild_table_with_rows_and_version(
            contents_table.as_ref(),
            new_rows,
            Some(rows_node),
            header.unique_value(),
            format,
        )?;
        let heap = push_data_tree_chunks(&mut blocks, &mut next_bid, &mut next_offset, heap)?;
        let rows = push_data_tree_chunks(&mut blocks, &mut next_bid, &mut next_offset, rows)?;
        Ok::<_, io::Error>((heap, rows, row_ids.len(), external))
    })()
    .map_err(|err| {
        io::Error::new(
            err.kind(),
            format!("rebuild receive-folder contents: {err}"),
        )
    })?;
    contents_subnodes.retain(|entry| entry.node() != rows_node);
    contents_subnodes.push(UnicodeLeafSubNodeTreeEntry::new(rows_node, rows_bid, None));
    for (node, data) in external_contents {
        let data = push_data_tree(&mut blocks, &mut next_bid, &mut next_offset, &data)?;
        contents_subnodes.push(UnicodeLeafSubNodeTreeEntry::new(node, data, None));
    }
    let contents_subnode_bid = push_subnode_tree(
        &mut blocks,
        &mut next_bid,
        &mut next_offset,
        contents_subnodes,
    )?;

    let content_count = i32::try_from(row_count).map_err(|_| PstError::IntegerConversion)?;
    set_folder_integer_property(&mut folder_properties, 0x3602, content_count);
    // MailCraft: messages that go in unread count in the folder's unread count.
    let unread_before = folder_properties
        .iter()
        .find_map(|(id, value)| match (id, value) {
            (0x3603, PropertyValue::Integer32(count)) => Some((*count).max(0)),
            _ => None,
        })
        .unwrap_or(0);
    let unread_added = inputs.iter().filter(|input| unread(&input.message)).count();
    let unread_count = i32::try_from(unread_added)
        .ok()
        .and_then(|added| unread_before.checked_add(added))
        .ok_or(PstError::IntegerConversion)?;
    set_folder_integer_property(&mut folder_properties, 0x3603, unread_count);
    let counts = [
        (0x3602, PropertyValue::Integer32(content_count)),
        (0x3603, PropertyValue::Integer32(unread_count)),
    ];
    let next_property_subnode = folder_subnodes
        .iter()
        .filter(|entry| matches!(entry.node().id_type(), Ok(NodeIdType::Internal)))
        .map(|entry| entry.node().index())
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(PstError::IntegerConversion)?;
    let (folder_pages, external_properties) =
        property_context_pages(folder_properties, next_property_subnode, format.text())?;
    let folder_data_bid =
        push_data_tree_chunks(&mut blocks, &mut next_bid, &mut next_offset, folder_pages)?;
    for (node, data) in external_properties {
        let data = push_data_tree(&mut blocks, &mut next_bid, &mut next_offset, &data)?;
        folder_subnodes.push(UnicodeLeafSubNodeTreeEntry::new(node, data, None));
    }
    let folder_subnode_bid = if folder_subnodes.is_empty() {
        None
    } else {
        Some(push_subnode_tree(
            &mut blocks,
            &mut next_bid,
            &mut next_offset,
            folder_subnodes,
        )?)
    };

    let parent_hierarchy_entry = if let Some((node, entry, table, mut subnodes, rows_node)) =
        parent_hierarchy
    {
        let old_rows = subnodes
            .iter()
            .find(|entry| entry.node() == rows_node)
            .map(|entry| entry.block());
        // MailCraft: the counts the table has columns for.
        let cells: Vec<_> = counts
            .iter()
            .filter(|(id, value)| {
                table.context().columns().iter().any(|column| {
                    column.prop_id() == *id && column.prop_type() == PropertyType::from(value)
                })
            })
            .cloned()
            .collect();
        let patched = patch_table_row(
            &mut file,
            encoding,
            &bbt_entries,
            table.as_ref(),
            old_rows,
            folder,
            &cells,
            header.unique_value(),
            &mut blocks,
            &mut next_bid,
            &mut next_offset,
        )?;
        let (data, rows) = if let Some(rows) = patched {
            (entry.data(), rows)
        } else {
            let (pages, rows, _) = rebuild_table(
                table.as_ref(),
                vec![],
                Some(rows_node),
                header.unique_value(),
                Some((folder, &counts)),
                true,
                format,
            )?;
            (
                push_data_tree_chunks(&mut blocks, &mut next_bid, &mut next_offset, pages)?,
                push_data_tree_chunks(&mut blocks, &mut next_bid, &mut next_offset, rows)?,
            )
        };
        subnodes.retain(|entry| entry.node() != rows_node);
        subnodes.push(UnicodeLeafSubNodeTreeEntry::new(rows_node, rows, None));
        let subnodes = push_subnode_tree(&mut blocks, &mut next_bid, &mut next_offset, subnodes)?;
        Some((
            node,
            UnicodeNodeBTreeEntry::new(node, data, Some(subnodes), entry.parent()),
        ))
    } else {
        None
    };

    nbt_entries.retain(|entry| entry.node() != contents && entry.node() != folder);
    if let Some((node, entry)) = parent_hierarchy_entry {
        nbt_entries.retain(|candidate| candidate.node() != node);
        nbt_entries.push(entry);
    }
    nbt_entries.extend([
        UnicodeNodeBTreeEntry::new(
            contents,
            contents_data_bid,
            Some(contents_subnode_bid),
            contents_entry.parent(),
        ),
        UnicodeNodeBTreeEntry::new(
            folder,
            folder_data_bid,
            folder_subnode_bid,
            folder_entry.parent(),
        ),
    ]);
    nbt_entries.extend(message_entries);
    if let Some(map) = named.new_map {
        push_map(
            map,
            &mut nbt_entries,
            &mut blocks,
            &mut next_bid,
            &mut next_offset,
        )?;
    }

    append_search_updates(
        &*store,
        &mut nbt_entries,
        &mut blocks,
        &mut next_bid,
        &mut next_offset,
        messages
            .iter()
            .map(|message| SearchUpdateData::MessageAdded {
                parent: folder,
                message: *message,
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
        // MailCraft: the attached messages' IDs are taken too.
        &[messages, embedded_ids.given].concat(),
    )
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod create_tests {
    use super::*;
    use crate::messaging::attachment::{Attachment, AttachmentData, UnicodeAttachment};
    use std::io::Cursor;
    use std::time::{SystemTime, UNIX_EPOCH};

    const TEST_RECIPIENTS: &[UnicodePstRecipient<'static>] = &[
        UnicodePstRecipient {
            name: "to recipient",
            email: "to@example.com",
            recipient_type: UnicodePstRecipientType::To,
            flags: None,
            track_status: None,
        },
        UnicodePstRecipient {
            name: "cc recipient",
            email: "cc@example.com",
            recipient_type: UnicodePstRecipientType::Cc,
            flags: None,
            track_status: None,
        },
        UnicodePstRecipient {
            name: "bcc recipient",
            email: "bcc@example.com",
            recipient_type: UnicodePstRecipientType::Bcc,
            flags: None,
            track_status: None,
        },
    ];

    fn last_inbox_message(path: &Path) -> Rc<dyn Message> {
        let store = UnicodeStore::read(Rc::new(UnicodePstFile::open(path).unwrap())).unwrap();
        let inbox = folder_at_path(&store, &["Inbox"]);
        let message_id = NodeId::from(u32::from(
            inbox
                .contents_table()
                .unwrap()
                .rows_matrix()
                .last()
                .unwrap()
                .id(),
        ));
        store
            .open_message(&store.properties().make_entry_id(message_id).unwrap(), None)
            .unwrap()
    }

    fn folder_at_path(store: &Rc<UnicodeStore>, folder_path: &[&str]) -> Rc<dyn Folder> {
        let mut folder = store
            .properties()
            .ipm_sub_tree_entry_id()
            .unwrap()
            .node_id();
        for name in folder_path {
            folder = find_child_folder(store.as_ref(), folder, name)
                .unwrap()
                .unwrap();
        }
        store
            .open_folder(&store.properties().make_entry_id(folder).unwrap())
            .unwrap()
    }

    #[test]
    fn sets_the_store_display_name_and_keeps_appending() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "outlook-pst-store-name-{}-{stamp}.pst",
            std::process::id()
        ));
        let input = UnicodePstMessage {
            subject: "Store name",
            sender_name: "Memex Sender",
            sender_email: "sender@example.com",
            recipients: TEST_RECIPIENTS,
            body: "Body",
            html_body: None,
            message_id: "<store-name@example.com>",
            delivery_time: 133_750_080_000_000_000,
            message_class: "IPM.Note",
            properties: &[],
        };
        drop(UnicodePstFile::create(&path, &input).unwrap());
        assert!(UnicodePstFile::set_store_display_name(&path, "").is_err());
        drop(UnicodePstFile::set_store_display_name(&path, "d0104.shin").unwrap());
        drop(UnicodePstFile::append(&path, &input).unwrap());

        let store = UnicodeStore::read(Rc::new(UnicodePstFile::open(&path).unwrap())).unwrap();
        assert_eq!(store.properties().display_name().unwrap(), "d0104.shin");
        assert!(store.properties().record_key().is_ok());
        assert_eq!(
            folder_at_path(&store, &["Inbox"])
                .properties()
                .content_count()
                .unwrap(),
            2
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn creates_a_reopenable_unicode_pst_with_one_message() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "outlook-pst-create-{}-{stamp}.pst",
            std::process::id()
        ));

        let input = UnicodePstMessage {
            subject: "Memex PST export",
            sender_name: "Memex Sender",
            sender_email: "sender@example.com",
            recipients: TEST_RECIPIENTS,
            body: "Hello from Memex PST export.",
            html_body: None,
            message_id: "<memex-pst-export@example.com>",
            delivery_time: 133_750_080_000_000_000,
            message_class: "IPM.Note",
            properties: &[],
        };
        let pst = UnicodePstFile::create(&path, &input).unwrap();
        assert_eq!(pst.header().version(), NdbVersion::Unicode);
        assert_eq!(pst.header().crypt_method(), NdbCryptMethod::None);
        drop(pst);

        let reopened = UnicodePstFile::open(&path).unwrap();
        assert_eq!(reopened.header().version(), NdbVersion::Unicode);
        // rgbFM (0x100..0x180) and rgbFP (0x180..0x200) are deprecated and MUST be 0xFF.
        let header = std::fs::read(&path).unwrap();
        assert!(header[0x100..0x200].iter().all(|byte| *byte == 0xFF));
        let store = UnicodeStore::read(Rc::new(reopened)).unwrap();
        let inbox = folder_at_path(&store, &["Inbox"]);
        assert_eq!(inbox.properties().display_name().unwrap(), "Inbox");
        assert_eq!(inbox.properties().content_count().unwrap(), 1);
        let contents = inbox.contents_table().unwrap();
        let message_row = contents.rows_matrix().next().unwrap();
        let message_id = NodeId::from(u32::from(message_row.id()));
        let message = store
            .open_message(&store.properties().make_entry_id(message_id).unwrap(), None)
            .unwrap();
        match message.properties().get(0x0037).unwrap() {
            PropertyValue::Unicode(value) => assert_eq!(value.to_string(), "Memex PST export"),
            value => panic!("unexpected subject value: {value:?}"),
        }
        match message.properties().get(0x1000).unwrap() {
            PropertyValue::Unicode(value) => {
                assert_eq!(value.to_string(), "Hello from Memex PST export.")
            }
            value => panic!("unexpected body value: {value:?}"),
        }

        let recipients = message.recipient_table().unwrap();
        let recipient_rows = recipients
            .rows_matrix()
            .map(|row| {
                let values = row.columns(recipients.context()).unwrap();
                let read = |id| {
                    let (column, value) = recipients
                        .context()
                        .columns()
                        .iter()
                        .zip(&values)
                        .find(|(column, _)| column.prop_id() == id)
                        .unwrap();
                    recipients
                        .read_column(value.as_ref().unwrap(), column.prop_type())
                        .unwrap()
                };
                let recipient_type = match read(0x0C15) {
                    PropertyValue::Integer32(value) => value,
                    value => panic!("unexpected recipient type: {value:?}"),
                };
                let smtp = match read(0x39FE) {
                    PropertyValue::Unicode(value) => value.to_string(),
                    value => panic!("unexpected recipient value: {value:?}"),
                };
                (recipient_type, smtp)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            recipient_rows,
            [
                (1, "to@example.com".to_string()),
                (2, "cc@example.com".to_string()),
                (3, "bcc@example.com".to_string()),
            ]
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn externalizes_large_recipient_display_values_in_contents_table() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "outlook-pst-large-display-to-{}-{stamp}.pst",
            std::process::id()
        ));
        let names = (0..1_500)
            .map(|index| format!("수신자 {index:04}"))
            .collect::<Vec<_>>();
        let emails = (0..1_500)
            .map(|index| format!("recipient-{index}@example.com"))
            .collect::<Vec<_>>();
        let recipients = names
            .iter()
            .zip(&emails)
            .map(|(name, email)| UnicodePstRecipient {
                name,
                email,
                recipient_type: UnicodePstRecipientType::To,
                flags: None,
                track_status: None,
            })
            .collect::<Vec<_>>();
        let display_to = recipient_display(&recipients, UnicodePstRecipientType::To);
        assert!(utf16_bytes(&display_to).len() > MAX_HN_ALLOCATION_SIZE);
        let input = UnicodePstMessage {
            subject: "large recipient display",
            sender_name: "sender",
            sender_email: "sender@example.com",
            recipients: &recipients,
            body: "body",
            html_body: None,
            message_id: "<large-display-to@example.com>",
            delivery_time: 133_750_080_000_000_000,
            message_class: "IPM.Note",
            properties: &[],
        };

        drop(UnicodePstFile::create(&path, &input).unwrap());
        drop(UnicodePstFile::append_many(&path, &[input, input]).unwrap());
        let store = UnicodeStore::read(Rc::new(UnicodePstFile::open(&path).unwrap())).unwrap();
        let inbox = folder_at_path(&store, &["Inbox"]);
        assert_eq!(inbox.properties().content_count().unwrap(), 3);
        let contents = inbox.contents_table().unwrap();
        let mut cell_nodes = std::collections::HashSet::new();
        for row in contents.rows_matrix() {
            let values = row.columns(contents.context()).unwrap();
            let (column, value) = contents
                .context()
                .columns()
                .iter()
                .zip(&values)
                .find(|(column, _)| column.prop_id() == 0x0E04)
                .unwrap();
            let Some(TableRowColumnValue::Node(node)) = value else {
                panic!("display-to cell is not in a subnode: {value:?}");
            };
            // Outlook fails GetTable unless cell subnodes are NID_TYPE_LTP, and appends must not reuse a NID.
            assert!(matches!(
                node.id_type(),
                Ok(NodeIdType::ListsTablesProperties)
            ));
            assert!(
                cell_nodes.insert(u32::from(*node)),
                "duplicate cell subnode"
            );
            match contents
                .read_column(value.as_ref().unwrap(), column.prop_type())
                .unwrap()
            {
                PropertyValue::Unicode(value) => assert_eq!(value.to_string(), display_to),
                value => panic!("unexpected display-to value: {value:?}"),
            }
        }
        let message = last_inbox_message(&path);
        assert_eq!(
            message.recipient_table().unwrap().rows_matrix().count(),
            recipients.len()
        );
        drop((message, inbox, store));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn appends_contents_rows_incrementally_without_leaking_heap() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = |name: &str| {
            std::env::temp_dir().join(format!(
                "outlook-pst-incremental-{name}-{}-{stamp}.pst",
                std::process::id()
            ))
        };
        let ids = (0..1_700)
            .map(|index| format!("<incremental-{index}@example.com>"))
            .collect::<Vec<_>>();
        let message = |index: usize| UnicodePstMessage {
            subject: "incremental subject",
            sender_name: "sender",
            sender_email: "sender@example.com",
            recipients: &[],
            body: "body",
            html_body: None,
            message_id: &ids[index],
            delivery_time: 133_750_080_000_000_000,
            message_class: "IPM.Note",
            properties: &[],
        };
        let batch = |range: std::ops::Range<usize>| range.map(message).collect::<Vec<_>>();
        let heap_pages = |path: &Path| {
            let store = UnicodeStore::read(Rc::new(UnicodePstFile::open(path).unwrap())).unwrap();
            let inbox = folder_at_path(&store, &["Inbox"]);
            let contents = inbox.contents_table().unwrap();
            assert_eq!(inbox.properties().content_count().unwrap(), 1_700);
            // MS-PST caps every HN allocation, row index nodes and page padding included, at 3580 bytes.
            assert!(contents
                .writer_heap_pages()
                .unwrap()
                .iter()
                .flatten()
                .all(|allocation| allocation.len() <= MAX_HN_ALLOCATION_SIZE));
            let rows = contents.rows_matrix().collect::<Vec<_>>();
            assert_eq!(rows.len(), 1_700);
            for row in &rows {
                assert_eq!(contents.find_row(row.id()).unwrap().id(), row.id());
            }
            for row in [rows[0], rows[799], rows[1_699]] {
                let node = NodeId::from(u32::from(row.id()));
                let message = store
                    .open_message(&store.properties().make_entry_id(node).unwrap(), None)
                    .unwrap();
                assert!(message.properties().get(0x0037).is_some());
            }
            let ipm_id = store.properties().ipm_sub_tree_entry_id().unwrap();
            let ipm = store.open_folder(&ipm_id).unwrap();
            let hierarchy = ipm.hierarchy_table().unwrap();
            let inbox_id = find_child_folder(store.as_ref(), ipm_id.node_id(), "Inbox")
                .unwrap()
                .unwrap();
            let row = hierarchy
                .find_row(TableRowId::new(u32::from(inbox_id)))
                .unwrap();
            let values = row.columns(hierarchy.context()).unwrap();
            let (column, value) = hierarchy
                .context()
                .columns()
                .iter()
                .zip(&values)
                .find(|(column, _)| column.prop_id() == 0x3602)
                .unwrap();
            assert!(matches!(
                hierarchy.read_column(value.as_ref().unwrap(), column.prop_type()),
                Ok(PropertyValue::Integer32(1_700))
            ));
            contents.writer_heap_pages().unwrap().len()
        };

        // One bulk call versus many small calls: the incremental path crosses row blocks, BTH
        // leaves and a new BTH level, and must not leak heap pages per call.
        let bulk = path("bulk");
        drop(UnicodePstFile::create(&bulk, &message(0)).unwrap());
        drop(UnicodePstFile::append_many(&bulk, &batch(1..1_700)).unwrap());
        let calls = path("calls");
        drop(UnicodePstFile::create(&calls, &message(0)).unwrap());
        for start in (1..1_700).step_by(100) {
            drop(
                UnicodePstFile::append_many(&calls, &batch(start..(start + 100).min(1_700)))
                    .unwrap(),
            );
        }
        let (bulk_pages, call_pages) = (heap_pages(&bulk), heap_pages(&calls));
        // Page-boundary padding costs at most about one BTH leaf per call; leaking the row index
        // per call would add ~2 pages per call here.
        assert!(call_pages <= bulk_pages + 4, "{call_pages} vs {bulk_pages}");
        // Space freed by earlier calls (old B-trees, rewritten table blocks) is reused.
        let size = |path: &Path| std::fs::metadata(path).unwrap().len();
        assert!(
            size(&calls) < size(&bulk) * 5 / 4,
            "{} vs {}",
            size(&calls),
            size(&bulk)
        );
        std::fs::remove_file(bulk).unwrap();
        std::fs::remove_file(calls).unwrap();
    }

    #[test]
    fn fills_nonfinal_data_tree_blocks() {
        let mut blocks = Vec::new();
        let mut next_bid = 1;
        let mut next_offset = FileSpace::new(AMAP_FIRST_OFFSET, Format::Unicode);
        let root = push_data_tree_chunks(
            &mut blocks,
            &mut next_bid,
            &mut next_offset,
            [vec![1; 100], vec![2; 50]],
        )
        .unwrap();

        assert!(root.is_internal());
        assert_eq!(blocks[0].size as usize, MAX_DATA_BLOCK_SIZE);
        assert_eq!(blocks[1].size, 50);
        let NewBlockData::DataTree(_, _, total_size) = blocks.last().unwrap().data else {
            panic!("expected XBLOCK root");
        };
        assert_eq!(total_size as usize, MAX_DATA_BLOCK_SIZE + 50);
    }

    #[test]
    fn writes_local_totals_for_multilevel_data_trees() {
        let mut blocks = Vec::new();
        let mut next_bid = 1;
        let mut next_offset = FileSpace::new(AMAP_FIRST_OFFSET, Format::Unicode);
        let root = push_data_tree_chunks(
            &mut blocks,
            &mut next_bid,
            &mut next_offset,
            (0..1_022).map(|_| vec![0x5A]),
        )
        .unwrap();

        let trees = blocks
            .iter()
            .filter_map(|block| match &block.data {
                NewBlockData::DataTree(entries, level, total_size) => {
                    Some((block.id, *level, entries.len(), *total_size))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let full_group_size = 1_021 * u32::try_from(MAX_DATA_BLOCK_SIZE).unwrap();
        assert_eq!(trees.len(), 3);
        assert_eq!(
            (trees[0].1, trees[0].2, trees[0].3),
            (1, 1_021, full_group_size)
        );
        assert_eq!((trees[1].1, trees[1].2, trees[1].3), (1, 1, 1));
        assert_eq!(
            (trees[2].1, trees[2].2, trees[2].3),
            (2, 2, full_group_size + 1)
        );
        assert_eq!(root, trees[2].0);
    }

    #[test]
    fn aligns_heap_page_maps_to_two_bytes() {
        let data = finish_heap(
            vec![Vec::new(), vec![0x5A]],
            HeapNodeType::Properties,
            heap_id(1).unwrap(),
        )
        .unwrap();
        let mut cursor = Cursor::new(&data);
        let page_map_offset = HeapNodeHeader::read(&mut cursor).unwrap().page_map_offset();
        assert_eq!(page_map_offset % 2, 0);
        cursor
            .seek(SeekFrom::Start(u64::from(page_map_offset)))
            .unwrap();
        HeapNodePageMap::read(&mut cursor).unwrap();
        assert_eq!(cursor.position() as usize, data.len());

        let mut heap = HeapBuilder::new();
        heap.alloc(vec![0x5A]).unwrap();
        let pages = heap
            .finish(HeapNodeType::Properties, HeapId::default())
            .unwrap();
        let mut cursor = Cursor::new(&pages[0]);
        let page_map_offset = HeapNodeHeader::read(&mut cursor).unwrap().page_map_offset();
        assert_eq!(page_map_offset % 2, 0);
        cursor
            .seek(SeekFrom::Start(u64::from(page_map_offset)))
            .unwrap();
        HeapNodePageMap::read(&mut cursor).unwrap();
        assert_eq!(cursor.position() as usize, pages[0].len());
    }

    #[test]
    fn splits_heap_page_padding_at_the_hn_allocation_limit() {
        // A maximal allocation that misses the first page by its page map entry leaves 3584 free bytes, more than one padding allocation may hold.
        let mut heap = HeapBuilder::new();
        for _ in 0..2 {
            heap.alloc(vec![0x5A; 2_285]).unwrap();
        }
        heap.alloc(vec![0x5A; MAX_HN_ALLOCATION_SIZE]).unwrap();
        let pages = heap
            .finish(HeapNodeType::Properties, HeapId::default())
            .unwrap();

        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].len(), MAX_DATA_BLOCK_SIZE);
        let mut cursor = Cursor::new(&pages[0]);
        let page_map_offset = HeapNodeHeader::read(&mut cursor).unwrap().page_map_offset();
        cursor
            .seek(SeekFrom::Start(u64::from(page_map_offset)))
            .unwrap();
        let page_map = HeapNodePageMap::read(&mut cursor).unwrap();
        let sizes = page_map
            .allocations()
            .iter()
            .map(|allocation| usize::from(allocation.size()))
            .collect::<Vec<_>>();
        assert!(sizes[2..].len() > 1, "{sizes:?}");
        assert!(sizes[2..]
            .iter()
            .all(|size| *size <= MAX_HN_ALLOCATION_SIZE));
        assert_eq!(cursor.position() as usize, pages[0].len());
    }

    #[test]
    fn fills_nonfinal_heap_pages_with_a_padding_allocation() {
        let mut heap = HeapBuilder::new();
        for _ in 0..100 {
            heap.alloc(vec![0x5A; 100]).unwrap();
        }
        let pages = heap
            .finish(HeapNodeType::Properties, HeapId::default())
            .unwrap();

        assert!(pages.len() > 1);
        assert_eq!(pages[0].len(), MAX_DATA_BLOCK_SIZE);
        let mut cursor = Cursor::new(&pages[0]);
        let page_map_offset = HeapNodeHeader::read(&mut cursor).unwrap().page_map_offset();
        assert_eq!(page_map_offset % 2, 0);
        cursor
            .seek(SeekFrom::Start(u64::from(page_map_offset)))
            .unwrap();
        let page_map = HeapNodePageMap::read(&mut cursor).unwrap();
        let last = page_map.allocations().last().unwrap();
        assert_eq!(
            usize::from(last.offset() + last.size()),
            usize::from(page_map_offset)
        );
        assert_eq!(cursor.position() as usize, pages[0].len());
    }

    #[test]
    fn writes_html_with_plain_text_fallback() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "outlook-pst-html-{}-{stamp}.pst",
            std::process::id()
        ));
        let html = "<p>HTML 본문</p>".repeat(1_000);
        let input = UnicodePstMessage {
            subject: "HTML message",
            sender_name: "sender",
            sender_email: "sender@example.com",
            recipients: TEST_RECIPIENTS,
            body: "plain-text fallback",
            html_body: Some(&html),
            message_id: "<html@example.com>",
            delivery_time: 133_750_080_000_000_000,
            message_class: "IPM.Note",
            properties: &[],
        };

        drop(UnicodePstFile::create(&path, &input).unwrap());
        drop(UnicodePstFile::append(&path, &input).unwrap());
        let message = last_inbox_message(&path);
        match message.properties().get(0x1000).unwrap() {
            PropertyValue::Unicode(value) => assert_eq!(value.to_string(), input.body),
            value => panic!("unexpected plain-text body: {value:?}"),
        }
        match message.properties().get(0x1013).unwrap() {
            PropertyValue::Binary(value) => assert_eq!(value.buffer(), html.as_bytes()),
            value => panic!("unexpected HTML body: {value:?}"),
        }
        drop(message);

        let empty_html = UnicodePstMessage {
            html_body: Some(""),
            message_id: "<empty-html@example.com>",
            ..input
        };
        drop(UnicodePstFile::append(&path, &empty_html).unwrap());
        let message = last_inbox_message(&path);
        match message.properties().get(0x1013).unwrap() {
            PropertyValue::Binary(value) => assert!(value.buffer().is_empty()),
            value => panic!("unexpected empty HTML body: {value:?}"),
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn appends_a_message_without_replacing_existing_messages() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "outlook-pst-append-{}-{stamp}.pst",
            std::process::id()
        ));
        let first = UnicodePstMessage {
            subject: "first",
            sender_name: "sender",
            sender_email: "sender@example.com",
            recipients: TEST_RECIPIENTS,
            body: "first body",
            html_body: None,
            message_id: "<first@example.com>",
            delivery_time: 133_750_080_000_000_000,
            message_class: "IPM.Note",
            properties: &[],
        };
        let second = UnicodePstMessage {
            subject: "second",
            body: "second body",
            message_id: "<second@example.com>",
            ..first
        };
        let third = UnicodePstMessage {
            subject: "third",
            body: "third body",
            message_id: "<third@example.com>",
            ..first
        };

        let created = UnicodePstFile::create(&path, &first).unwrap();
        let old_bbt = *created.header().root().block_btree();
        let old_nbt = *created.header().root().node_btree();
        drop(created);
        let appended = UnicodePstFile::append(&path, &second).unwrap();
        assert_ne!(
            appended.header().root().block_btree().index().index(),
            old_bbt.index().index()
        );
        assert_ne!(
            appended.header().root().node_btree().index().index(),
            old_nbt.index().index()
        );
        let second_bbt = *appended.header().root().block_btree();
        let second_nbt = *appended.header().root().node_btree();
        drop(appended);
        let appended = UnicodePstFile::append(&path, &third).unwrap();
        assert_ne!(
            appended.header().root().block_btree().index().index(),
            second_bbt.index().index()
        );
        assert_ne!(
            appended.header().root().node_btree().index().index(),
            second_nbt.index().index()
        );

        let store = UnicodeStore::read(Rc::new(appended)).unwrap();
        let inbox = folder_at_path(&store, &["Inbox"]);
        assert_eq!(inbox.properties().content_count().unwrap(), 3);
        let subjects: Vec<String> = inbox
            .contents_table()
            .unwrap()
            .rows_matrix()
            .map(|row| {
                let message_id = NodeId::from(u32::from(row.id()));
                let message = store
                    .open_message(&store.properties().make_entry_id(message_id).unwrap(), None)
                    .unwrap();
                assert_eq!(
                    message.recipient_table().unwrap().rows_matrix().count(),
                    TEST_RECIPIENTS.len()
                );
                match message.properties().get(0x0037).unwrap() {
                    PropertyValue::Unicode(value) => value.to_string(),
                    value => panic!("unexpected subject value: {value:?}"),
                }
            })
            .collect();
        assert_eq!(subjects, ["first", "second", "third"]);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn bulk_appends_messages_and_attachments_in_one_commit() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "outlook-pst-bulk-{}-{stamp}.pst",
            std::process::id()
        ));
        let first = UnicodePstMessage {
            subject: "first",
            sender_name: "sender",
            sender_email: "sender@example.com",
            recipients: TEST_RECIPIENTS,
            body: "body",
            html_body: None,
            message_id: "<bulk-first@example.com>",
            delivery_time: 133_750_080_000_000_000,
            message_class: "IPM.Note",
            properties: &[],
        };
        drop(UnicodePstFile::create(&path, &first).unwrap());
        let second = UnicodePstMessage {
            subject: "second",
            message_id: "<bulk-second@example.com>",
            ..first
        };
        let third = UnicodePstMessage {
            subject: "third",
            message_id: "<bulk-third@example.com>",
            ..first
        };
        let attachment = UnicodePstAttachment {
            filename: "bulk.txt",
            mime_type: "text/plain",
            content_id: None,
            data: b"bulk attachment",
            ..Default::default()
        };
        drop(
            UnicodePstFile::append_many_in_folder_with_attachments(
                &path,
                &["Bulk", "2026"],
                &[
                    UnicodePstBatchMessage {
                        message: second,
                        attachments: &[attachment],
                    },
                    UnicodePstBatchMessage {
                        message: third,
                        attachments: &[],
                    },
                ],
            )
            .unwrap(),
        );

        let store = UnicodeStore::read(Rc::new(UnicodePstFile::open(&path).unwrap())).unwrap();
        let folder = folder_at_path(&store, &["Bulk", "2026"]);
        assert_eq!(folder.properties().content_count().unwrap(), 2);
        let messages = folder
            .contents_table()
            .unwrap()
            .rows_matrix()
            .map(|row| {
                store
                    .open_message(
                        &store
                            .properties()
                            .make_entry_id(NodeId::from(u32::from(row.id())))
                            .unwrap(),
                        None,
                    )
                    .unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages[0]
                .attachment_table()
                .unwrap()
                .rows_matrix()
                .count(),
            1
        );
        assert!(messages[1].attachment_table().is_none());
        drop((messages, folder, store));
        assert_eq!(
            UnicodePstFile::append_many(&path, &[])
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn bulk_append_error_reports_position_message_id_and_pst_size() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "outlook-pst-bulk-error-{}-{stamp}.pst",
            std::process::id()
        ));
        let first = UnicodePstMessage {
            subject: "first",
            sender_name: "sender",
            sender_email: "sender@example.com",
            recipients: TEST_RECIPIENTS,
            body: "body",
            html_body: None,
            message_id: "<bulk-error-first@example.com>",
            delivery_time: 133_750_080_000_000_000,
            message_class: "IPM.Note",
            properties: &[],
        };
        drop(UnicodePstFile::create(&path, &first).unwrap());
        let invalid_attachment = [UnicodePstAttachment {
            filename: "invalid.png",
            mime_type: "image/png",
            content_id: Some("<>"),
            data: &[],
            ..Default::default()
        }];
        let error = UnicodePstFile::append_many_with_attachments(
            &path,
            &[
                UnicodePstBatchMessage {
                    message: UnicodePstMessage {
                        message_id: "<bulk-error-ok@example.com>",
                        ..first
                    },
                    attachments: &[],
                },
                UnicodePstBatchMessage {
                    message: UnicodePstMessage {
                        message_id: "<bulk-error-invalid@example.com>",
                        ..first
                    },
                    attachments: &invalid_attachment,
                },
            ],
        )
        .err()
        .unwrap();
        let error = error.to_string();
        assert!(error.contains("message 2 of 2"), "{error}");
        assert!(
            error.contains("<bulk-error-invalid@example.com>"),
            "{error}"
        );
        assert!(error.contains("PST size"), "{error}");
        let message = last_inbox_message(&path);
        match message.properties().get(0x0037).unwrap() {
            PropertyValue::Unicode(value) => assert_eq!(value.to_string(), "first"),
            value => panic!("unexpected subject value: {value:?}"),
        }
        drop(message);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn bulk_appends_three_thousand_messages_without_cow_explosion() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "outlook-pst-bulk-3000-{}-{stamp}.pst",
            std::process::id()
        ));
        let first = UnicodePstMessage {
            subject: "first",
            sender_name: "sender",
            sender_email: "sender@example.com",
            recipients: TEST_RECIPIENTS,
            body: "body",
            html_body: None,
            message_id: "<bulk-3000-first@example.com>",
            delivery_time: 133_750_080_000_000_000,
            message_class: "IPM.Note",
            properties: &[],
        };
        drop(UnicodePstFile::create(&path, &first).unwrap());

        let subjects = (0..3_000)
            .map(|index| format!("bulk message {index}"))
            .collect::<Vec<_>>();
        let message_ids = (0..3_000)
            .map(|index| format!("<bulk-{index}@example.com>"))
            .collect::<Vec<_>>();
        let messages = subjects
            .iter()
            .zip(&message_ids)
            .map(|(subject, message_id)| UnicodePstMessage {
                subject,
                message_id,
                ..first
            })
            .collect::<Vec<_>>();
        let attachment_data = vec![0x5A; 320 * 1_024];
        let attachments = [UnicodePstAttachment {
            filename: "large.bin",
            mime_type: "application/octet-stream",
            content_id: None,
            data: &attachment_data,
            ..Default::default()
        }];
        let batch = messages
            .iter()
            .enumerate()
            .map(|(index, message)| UnicodePstBatchMessage {
                message: *message,
                attachments: if index % 300 == 0 { &attachments } else { &[] },
            })
            .collect::<Vec<_>>();
        drop(UnicodePstFile::append_many_with_attachments(&path, &batch).unwrap());

        let store = UnicodeStore::read(Rc::new(UnicodePstFile::open(&path).unwrap())).unwrap();
        let inbox = folder_at_path(&store, &["Inbox"]);
        assert_eq!(inbox.properties().content_count().unwrap(), 3_001);
        assert_eq!(inbox.contents_table().unwrap().rows_matrix().count(), 3_001);
        assert!(std::fs::metadata(&path).unwrap().len() < 64 * 1024 * 1024);
        drop((inbox, store));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn creates_and_appends_messages_in_nested_folders() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "outlook-pst-folders-{}-{stamp}.pst",
            std::process::id()
        ));
        let first = UnicodePstMessage {
            subject: "first in Projects/2026",
            sender_name: "sender",
            sender_email: "sender@example.com",
            recipients: TEST_RECIPIENTS,
            body: "body",
            html_body: None,
            message_id: "<folder-first@example.com>",
            delivery_time: 133_750_080_000_000_000,
            message_class: "IPM.Note",
            properties: &[],
        };
        drop(
            UnicodePstFile::create_in_folder_with_attachments(
                &path,
                &["Projects", "2026"],
                &first,
                &[UnicodePstAttachment {
                    filename: "folder.txt",
                    mime_type: "text/plain",
                    content_id: None,
                    data: b"folder attachment",
                    ..Default::default()
                }],
            )
            .unwrap(),
        );
        drop(
            UnicodePstFile::append_in_folder(
                &path,
                &["projects", "2026"],
                &UnicodePstMessage {
                    subject: "second in Projects/2026",
                    message_id: "<folder-second@example.com>",
                    ..first
                },
            )
            .unwrap(),
        );
        drop(
            UnicodePstFile::append_in_folder(
                &path,
                &["Archive", "2025"],
                &UnicodePstMessage {
                    subject: "in Archive/2025",
                    message_id: "<folder-archive@example.com>",
                    ..first
                },
            )
            .unwrap(),
        );
        drop(
            UnicodePstFile::append(
                &path,
                &UnicodePstMessage {
                    subject: "still in Inbox",
                    message_id: "<folder-inbox@example.com>",
                    ..first
                },
            )
            .unwrap(),
        );

        let store = UnicodeStore::read(Rc::new(UnicodePstFile::open(&path).unwrap())).unwrap();
        let ipm = folder_at_path(&store, &[]);
        assert_eq!(ipm.hierarchy_table().unwrap().rows_matrix().count(), 4);
        let inbox = folder_at_path(&store, &["Inbox"]);
        assert_eq!(inbox.properties().content_count().unwrap(), 1);
        let projects = folder_at_path(&store, &["Projects"]);
        assert!(projects.properties().has_sub_folders().unwrap());
        assert_eq!(projects.hierarchy_table().unwrap().rows_matrix().count(), 1);
        let projects_2026 = folder_at_path(&store, &["Projects", "2026"]);
        assert_eq!(projects_2026.properties().content_count().unwrap(), 2);
        assert!(projects_2026.associated_table().is_some());
        let subjects = projects_2026
            .contents_table()
            .unwrap()
            .rows_matrix()
            .map(|row| {
                let message = store
                    .open_message(
                        &store
                            .properties()
                            .make_entry_id(NodeId::from(u32::from(row.id())))
                            .unwrap(),
                        None,
                    )
                    .unwrap();
                match message.properties().get(0x0037).unwrap() {
                    PropertyValue::Unicode(value) => value.to_string(),
                    value => panic!("unexpected subject value: {value:?}"),
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(
            subjects,
            ["first in Projects/2026", "second in Projects/2026"]
        );
        let first_message_id = NodeId::from(u32::from(
            projects_2026
                .contents_table()
                .unwrap()
                .rows_matrix()
                .next()
                .unwrap()
                .id(),
        ));
        let first_message = store
            .open_message(
                &store.properties().make_entry_id(first_message_id).unwrap(),
                None,
            )
            .unwrap();
        assert_eq!(
            first_message
                .attachment_table()
                .unwrap()
                .rows_matrix()
                .count(),
            1
        );
        let archive_2025 = folder_at_path(&store, &["Archive", "2025"]);
        assert_eq!(archive_2025.properties().content_count().unwrap(), 1);
        assert!(archive_2025.associated_table().is_some());
        drop((
            first_message,
            archive_2025,
            projects_2026,
            projects,
            inbox,
            ipm,
            store,
        ));

        assert_eq!(
            UnicodePstFile::append_in_folder(&path, &[], &first)
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn creates_and_appends_large_bodies_and_inline_attachments() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "outlook-pst-large-{}-{stamp}.pst",
            std::process::id()
        ));
        let body = "본문".repeat(2_500);
        let attachment_bytes = vec![0xA5; 9_000];
        let input = UnicodePstMessage {
            subject: "large",
            sender_name: "sender",
            sender_email: "sender@example.com",
            recipients: TEST_RECIPIENTS,
            body: &body,
            html_body: Some("<img src=\"cid:large-image@example.com\">"),
            message_id: "<large@example.com>",
            delivery_time: 133_750_080_000_000_000,
            message_class: "IPM.Note",
            properties: &[],
        };
        let attachment = UnicodePstAttachment {
            filename: "large.png",
            mime_type: "image/png",
            content_id: Some("<large-image@example.com>"),
            data: &attachment_bytes,
            ..Default::default()
        };

        drop(UnicodePstFile::create_with_attachments(&path, &input, &[attachment]).unwrap());

        let store = UnicodeStore::read(Rc::new(UnicodePstFile::open(&path).unwrap())).unwrap();
        let inbox = folder_at_path(&store, &["Inbox"]);
        let message_id = NodeId::from(u32::from(
            inbox
                .contents_table()
                .unwrap()
                .rows_matrix()
                .next()
                .unwrap()
                .id(),
        ));
        let entry_id = store.properties().make_entry_id(message_id).unwrap();
        let message = UnicodeMessage::read(store.clone(), &entry_id, None).unwrap();
        assert_eq!(message.properties().message_flags().unwrap(), 0x11);
        assert!(matches!(
            message.properties().get(0x0E1B),
            Some(PropertyValue::Boolean(true))
        ));
        match message.properties().get(0x1000).unwrap() {
            PropertyValue::Unicode(value) => assert_eq!(value.to_string(), body),
            value => panic!("unexpected body value: {value:?}"),
        }
        let attachment_id = NodeId::from(u32::from(
            message
                .attachment_table()
                .unwrap()
                .rows_matrix()
                .next()
                .unwrap()
                .id(),
        ));
        let attachment = UnicodeAttachment::read(message, attachment_id, None).unwrap();
        assert_eq!(attachment.properties().attachment_size().unwrap(), 9_124);
        match attachment.properties().get(0x3707).unwrap() {
            PropertyValue::Unicode(value) => assert_eq!(value.to_string(), "large.png"),
            value => panic!("unexpected attachment filename: {value:?}"),
        }
        match attachment.properties().get(0x370E).unwrap() {
            PropertyValue::Unicode(value) => assert_eq!(value.to_string(), "image/png"),
            value => panic!("unexpected attachment MIME type: {value:?}"),
        }
        match attachment.properties().get(0x3712).unwrap() {
            PropertyValue::Unicode(value) => {
                assert_eq!(value.to_string(), "large-image@example.com")
            }
            value => panic!("unexpected attachment Content-ID: {value:?}"),
        }
        assert!(matches!(
            attachment.properties().get(0x3714),
            Some(PropertyValue::Integer32(0x0000_0004))
        ));
        match attachment.data().unwrap() {
            AttachmentData::Binary(value) => assert_eq!(value.buffer(), attachment_bytes),
            AttachmentData::Message(_) => panic!("unexpected embedded message"),
        }
        drop(attachment);
        drop(inbox);
        drop(store);
        std::fs::remove_file(&path).unwrap();

        let small_body = "small";
        let first = UnicodePstMessage {
            subject: "first",
            body: small_body,
            message_id: "<first@example.com>",
            ..input
        };
        drop(UnicodePstFile::create(&path, &first).unwrap());
        drop(
            UnicodePstFile::append_with_attachments(
                &path,
                &input,
                &[UnicodePstAttachment {
                    filename: "large.png",
                    mime_type: "image/png",
                    content_id: Some("large-image@example.com"),
                    data: &attachment_bytes,
                    ..Default::default()
                }],
            )
            .unwrap(),
        );

        let store = UnicodeStore::read(Rc::new(UnicodePstFile::open(&path).unwrap())).unwrap();
        let inbox = folder_at_path(&store, &["Inbox"]);
        let message_id = NodeId::from(u32::from(
            inbox
                .contents_table()
                .unwrap()
                .rows_matrix()
                .last()
                .unwrap()
                .id(),
        ));
        let message = UnicodeMessage::read(
            store.clone(),
            &store.properties().make_entry_id(message_id).unwrap(),
            None,
        )
        .unwrap();
        match message.properties().get(0x1000).unwrap() {
            PropertyValue::Unicode(value) => assert_eq!(value.to_string(), body),
            value => panic!("unexpected body value: {value:?}"),
        }
        let attachment_id = NodeId::from(u32::from(
            message
                .attachment_table()
                .unwrap()
                .rows_matrix()
                .next()
                .unwrap()
                .id(),
        ));
        let attachment = UnicodeAttachment::read(message, attachment_id, None).unwrap();
        match attachment.properties().get(0x3712).unwrap() {
            PropertyValue::Unicode(value) => {
                assert_eq!(value.to_string(), "large-image@example.com")
            }
            value => panic!("unexpected attachment Content-ID: {value:?}"),
        }
        assert!(matches!(
            attachment.properties().get(0x3714),
            Some(PropertyValue::Integer32(0x0000_0004))
        ));
        match attachment.data().unwrap() {
            AttachmentData::Binary(value) => assert_eq!(value.buffer(), attachment_bytes),
            AttachmentData::Message(_) => panic!("unexpected embedded message"),
        }
        drop(attachment);
        drop(inbox);
        drop(store);
        std::fs::remove_file(path).unwrap();

        assert_eq!(
            attachment_data(
                &UnicodePstAttachment {
                    filename: "invalid.png",
                    mime_type: "image/png",
                    content_id: Some("<>"),
                    data: &[],
                    ..Default::default()
                },
                None,
                0,
                None,
                Text::Unicode,
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn reserves_allocation_metadata_through_64_gib() {
        const LIMIT: u64 = 64 * 1024 * 1024 * 1024;

        assert_eq!(FPMAP_FIRST_OFFSET, 0x7C004800);
        assert_eq!(map_page_count(FPMAP_FIRST_SIZE), 3);

        for (first, interval) in [
            (AMAP_FIRST_OFFSET, AMAP_DATA_SIZE),
            (PMAP_FIRST_OFFSET, PMAP_DATA_SIZE),
            (FMAP_FIRST_OFFSET, FMAP_DATA_SIZE),
            (FPMAP_FIRST_OFFSET, FPMAP_DATA_SIZE),
        ] {
            let mut metadata_offset = first;
            while metadata_offset < LIMIT {
                let mut next_offset = FileSpace::new(metadata_offset, Format::Unicode);
                let allocation =
                    reserve_file_range(&mut next_offset, PAGE_SIZE as u64, PAGE_SIZE as u64)
                        .unwrap();
                assert!(
                    allocation > metadata_offset,
                    "allocated over metadata page at 0x{metadata_offset:X}"
                );
                metadata_offset += interval;
            }
        }
    }

    #[test]
    fn supports_multilevel_btrees_and_multiple_amaps() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "outlook-pst-multilevel-{}-{stamp}.pst",
            std::process::id()
        ));
        let large_attachment = vec![0x5A; 320_000];
        let first = UnicodePstMessage {
            subject: "message-0",
            sender_name: "sender",
            sender_email: "sender@example.com",
            recipients: TEST_RECIPIENTS,
            body: "body",
            html_body: None,
            message_id: "<message-0@example.com>",
            delivery_time: 133_750_080_000_000_000,
            message_class: "IPM.Note",
            properties: &[],
        };
        drop(
            UnicodePstFile::create_with_attachments(
                &path,
                &first,
                &[UnicodePstAttachment {
                    filename: "large.bin",
                    mime_type: "application/octet-stream",
                    content_id: None,
                    data: &large_attachment,
                    ..Default::default()
                }],
            )
            .unwrap(),
        );
        for index in 1..20 {
            let subject = format!("message-{index}");
            let message_id = format!("<message-{index}@example.com>");
            drop(
                UnicodePstFile::append(
                    &path,
                    &UnicodePstMessage {
                        subject: &subject,
                        message_id: &message_id,
                        ..first
                    },
                )
                .unwrap(),
            );
        }

        let pst = UnicodePstFile::open(&path).unwrap();
        assert!(pst.header().root().amap_last_index().index() > AMAP_FIRST_OFFSET);
        let mut reader = pst.reader().lock().unwrap();
        assert!(matches!(
            UnicodeBlockBTree::read(&mut *reader, *pst.header().root().block_btree()).unwrap(),
            RootBTreePage::Intermediate(..)
        ));
        assert!(matches!(
            UnicodeNodeBTree::read(&mut *reader, *pst.header().root().node_btree()).unwrap(),
            RootBTreePage::Intermediate(..)
        ));
        drop(reader);

        let store = UnicodeStore::read(Rc::new(pst)).unwrap();
        let inbox = folder_at_path(&store, &["Inbox"]);
        assert_eq!(inbox.properties().content_count().unwrap(), 20);
        assert_eq!(inbox.contents_table().unwrap().rows_matrix().count(), 20);
        drop((inbox, store));

        // The allocation map a commit marks from memory equals the spec's B-tree walk rebuild.
        let amap_pages = |path: &Path| {
            let mut file = File::open(path).unwrap();
            let eof = UnicodeHeader::read(&mut file)
                .unwrap()
                .root()
                .file_eof_index()
                .index();
            let mut pages = Vec::new();
            for offset in (AMAP_FIRST_OFFSET..eof).step_by(AMAP_DATA_SIZE as usize) {
                let mut page = [0; PAGE_SIZE];
                file.seek(SeekFrom::Start(offset)).unwrap();
                file.read_exact(&mut page).unwrap();
                pages.push(page);
            }
            pages
        };
        let written = amap_pages(&path);
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let mut header = UnicodeHeader::read(&mut file).unwrap();
        header.root_mut().set_amap_status(AmapStatus::Invalid);
        file.seek(SeekFrom::Start(0)).unwrap();
        header.write(&mut file).unwrap();
        drop(file);
        let mut pst = UnicodePstFile::open(&path).unwrap();
        pst.inner.rebuild_allocation_map().unwrap();
        drop(pst);
        assert!(written.len() > 1);
        assert!(
            written == amap_pages(&path),
            "allocation map differs from a rebuild"
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn appends_to_an_outlook_created_unicode_pst() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "outlook-pst-existing-{}-{stamp}.pst",
            std::process::id()
        ));
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/Empty.pst"),
            &path,
        )
        .unwrap();

        // Outlook's fixture uses permute encoding. Decode its data blocks so the same Outlook
        // structure exercises the writer's unencoded append path (MailCraft: the encoded ones are
        // `appends_to_permute_and_cyclic_encoded_unicode_psts`).
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let mut header = UnicodeHeader::read(&mut file).unwrap();
        assert_eq!(header.crypt_method(), NdbCryptMethod::Permute);
        let entries =
            collect_block_btree_entries(&mut file, Format::Unicode, *header.root().block_btree())
                .unwrap();
        for entry in entries
            .iter()
            .filter(|entry| !entry.block().block().is_internal())
        {
            file.seek(SeekFrom::Start(entry.block().index().index()))
                .unwrap();
            let block =
                UnicodeDataBlock::read(&mut file, entry.size(), NdbCryptMethod::Permute).unwrap();
            file.seek(SeekFrom::Start(entry.block().index().index()))
                .unwrap();
            UnicodeDataBlock::new(
                NdbCryptMethod::None,
                block.data().to_vec(),
                *block.trailer(),
            )
            .unwrap()
            .write(&mut file)
            .unwrap();
        }
        header.set_crypt_method(NdbCryptMethod::None);
        file.seek(SeekFrom::Start(0)).unwrap();
        header.write(&mut file).unwrap();
        file.sync_all().unwrap();
        drop(file);

        let input = UnicodePstMessage {
            subject: "appended to Outlook PST",
            sender_name: "sender",
            sender_email: "sender@example.com",
            recipients: TEST_RECIPIENTS,
            body: "existing PST body",
            html_body: None,
            message_id: "<existing-pst@example.com>",
            delivery_time: 133_750_080_000_000_000,
            message_class: "IPM.Note",
            properties: &[],
        };
        drop(UnicodePstFile::append(&path, &input).unwrap());
        drop(UnicodePstFile::append(&path, &input).unwrap());
        drop(UnicodePstFile::append_in_folder(&path, &["Imported", "2026"], &input).unwrap());

        let pst = Rc::new(UnicodePstFile::open(&path).unwrap());
        let next_message = pst
            .header()
            .next_node_index(NodeIdType::NormalMessage)
            .unwrap();
        let mut reader = File::open(&path).unwrap();
        let nbt_entries = collect_node_btree_entries(
            &mut reader,
            Format::Unicode,
            *pst.header().root().node_btree(),
        )
        .unwrap();
        let store = UnicodeStore::read(pst).unwrap();
        let inbox_id =
            default_receive_folder(&AddStore::open(&path).unwrap(), &nbt_entries).unwrap();
        let inbox = store
            .open_folder(&store.properties().make_entry_id(inbox_id).unwrap())
            .unwrap();
        assert_eq!(inbox.properties().content_count().unwrap(), 2);
        let contents = inbox.contents_table().unwrap();
        assert!(contents.context().columns().len() > 2);
        let message_ids = contents
            .rows_matrix()
            .map(|row| NodeId::from(u32::from(row.id())))
            .collect::<Vec<_>>();
        assert_eq!(message_ids.len(), 2);
        assert!(message_ids.iter().all(|id| id.index() < next_message));
        for message_id in message_ids {
            let message = store
                .open_message(&store.properties().make_entry_id(message_id).unwrap(), None)
                .unwrap();
            match message.properties().get(0x0037).unwrap() {
                PropertyValue::Unicode(value) => assert_eq!(value.to_string(), input.subject),
                value => panic!("unexpected subject value: {value:?}"),
            }
        }
        let imported = folder_at_path(&store, &["Imported"]);
        assert!(
            imported
                .hierarchy_table()
                .unwrap()
                .context()
                .columns()
                .len()
                > 2
        );
        let imported_2026 = folder_at_path(&store, &["Imported", "2026"]);
        assert_eq!(imported_2026.properties().content_count().unwrap(), 1);
        assert!(
            imported_2026
                .contents_table()
                .unwrap()
                .context()
                .columns()
                .len()
                > 2
        );
        assert!(
            imported_2026
                .associated_table()
                .unwrap()
                .context()
                .columns()
                .len()
                > 2
        );
        drop((imported_2026, imported, inbox, store));
        std::fs::remove_file(path).unwrap();
    }

    /// MailCraft: Outlook's fixture as Outlook made it, permute-encoded, or re-encoded cyclic.
    fn encoded_outlook_pst(label: &str, method: NdbCryptMethod) -> std::path::PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "outlook-pst-{label}-{}-{stamp}.pst",
            std::process::id()
        ));
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/Empty.pst"),
            &path,
        )
        .unwrap();
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let mut header = UnicodeHeader::read(&mut file).unwrap();
        assert_eq!(header.crypt_method(), NdbCryptMethod::Permute);
        if method != NdbCryptMethod::Permute {
            let entries = collect_block_btree_entries(
                &mut file,
                Format::Unicode,
                *header.root().block_btree(),
            )
            .unwrap();
            for entry in entries
                .iter()
                .filter(|entry| !entry.block().block().is_internal())
            {
                file.seek(SeekFrom::Start(entry.block().index().index()))
                    .unwrap();
                let block =
                    UnicodeDataBlock::read(&mut file, entry.size(), NdbCryptMethod::Permute)
                        .unwrap();
                file.seek(SeekFrom::Start(entry.block().index().index()))
                    .unwrap();
                UnicodeDataBlock::new(method, block.data().to_vec(), *block.trailer())
                    .unwrap()
                    .write(&mut file)
                    .unwrap();
            }
            header.set_crypt_method(method);
            file.seek(SeekFrom::Start(0)).unwrap();
            header.write(&mut file).unwrap();
            file.sync_all().unwrap();
        }
        path
    }

    /// MailCraft: every data block of the file, read raw and decoded with its own method.
    fn data_blocks(path: &Path) -> Vec<(Vec<u8>, Vec<u8>)> {
        let mut file = File::open(path).unwrap();
        let header = UnicodeHeader::read(&mut file).unwrap();
        let entries =
            collect_block_btree_entries(&mut file, Format::Unicode, *header.root().block_btree())
                .unwrap();
        entries
            .iter()
            .filter(|entry| !entry.block().block().is_internal())
            .map(|entry| {
                let mut read = |method| {
                    file.seek(SeekFrom::Start(entry.block().index().index()))
                        .unwrap();
                    UnicodeDataBlock::read(&mut file, entry.size(), method)
                        .unwrap()
                        .data()
                        .to_vec()
                };
                (read(NdbCryptMethod::None), read(header.crypt_method()))
            })
            .collect()
    }

    #[test]
    fn appends_to_permute_and_cyclic_encoded_unicode_psts() {
        for (label, method) in [
            ("permute", NdbCryptMethod::Permute),
            ("cyclic", NdbCryptMethod::Cyclic),
        ] {
            let path = encoded_outlook_pst(&format!("encoded-{label}"), method);
            let body = "encoded body ".repeat(1_500);
            let attachment_bytes = (0..20_000_u32).map(|n| (n % 251) as u8).collect::<Vec<_>>();
            let input = UnicodePstMessage {
                subject: "appended to an encoded PST",
                sender_name: "sender",
                sender_email: "sender@example.com",
                recipients: TEST_RECIPIENTS,
                body: &body,
                html_body: Some("<p>encoded</p>"),
                message_id: "<encoded@example.com>",
                delivery_time: 133_750_080_000_000_000,
                message_class: "IPM.Note",
                properties: &[],
            };
            let attachments = [UnicodePstAttachment {
                filename: "encoded.bin",
                mime_type: "application/octet-stream",
                content_id: None,
                data: &attachment_bytes,
                ..Default::default()
            }];
            drop(UnicodePstFile::set_store_display_name(&path, "Encoded").unwrap());
            drop(UnicodePstFile::append_with_attachments(&path, &input, &attachments).unwrap());
            // The second batch appends rows in place: the contents table's tail block and the
            // parent's hierarchy row are read back decoded and rewritten encoded.
            let batch = (0..40)
                .map(|_| UnicodePstBatchMessage {
                    message: input,
                    attachments: &attachments,
                })
                .collect::<Vec<_>>();
            drop(UnicodePstFile::append_many_with_attachments(&path, &batch).unwrap());
            drop(UnicodePstFile::append_in_folder(&path, &["Imported", "2026"], &input).unwrap());

            let pst = UnicodePstFile::open(&path).unwrap();
            assert_eq!(pst.header().crypt_method(), method);
            let store = UnicodeStore::read(Rc::new(pst)).unwrap();
            assert_eq!(store.properties().display_name().unwrap(), "Encoded");
            let mut reader = File::open(&path).unwrap();
            let nbt_root = *UnicodeHeader::read(&mut reader)
                .unwrap()
                .root()
                .node_btree();
            let nbt_entries =
                collect_node_btree_entries(&mut reader, Format::Unicode, nbt_root).unwrap();
            let inbox_id =
                default_receive_folder(&AddStore::open(&path).unwrap(), &nbt_entries).unwrap();
            let inbox = store
                .open_folder(&store.properties().make_entry_id(inbox_id).unwrap())
                .unwrap();
            assert_eq!(inbox.properties().content_count().unwrap(), 41);
            let message_ids = inbox
                .contents_table()
                .unwrap()
                .rows_matrix()
                .map(|row| NodeId::from(u32::from(row.id())))
                .collect::<Vec<_>>();
            assert_eq!(message_ids.len(), 41);
            for message_id in [message_ids[0], message_ids[40]] {
                let entry_id = store.properties().make_entry_id(message_id).unwrap();
                let message = UnicodeMessage::read(store.clone(), &entry_id, None).unwrap();
                match message.properties().get(0x1000).unwrap() {
                    PropertyValue::Unicode(value) => assert_eq!(value.to_string(), body),
                    value => panic!("unexpected body value: {value:?}"),
                }
                let attachment_id = NodeId::from(u32::from(
                    message
                        .attachment_table()
                        .unwrap()
                        .rows_matrix()
                        .next()
                        .unwrap()
                        .id(),
                ));
                let attachment = UnicodeAttachment::read(message, attachment_id, None).unwrap();
                match attachment.data().unwrap() {
                    AttachmentData::Binary(value) => assert_eq!(value.buffer(), attachment_bytes),
                    AttachmentData::Message(_) => panic!("unexpected embedded message"),
                }
            }
            let imported_2026 = folder_at_path(&store, &["Imported", "2026"]);
            assert_eq!(imported_2026.properties().content_count().unwrap(), 1);
            drop((imported_2026, inbox, store));

            // Every data block, old and new, is encoded: the body's text never shows raw.
            let blocks = data_blocks(&path);
            let needle = utf16_bytes("encoded body");
            let needle = &needle[..needle.len() - 2];
            let contains = |data: &[u8]| data.windows(needle.len()).any(|window| window == needle);
            assert!(blocks.iter().any(|(_, decoded)| contains(decoded)));
            assert!(blocks.iter().all(|(raw, _)| !contains(raw)));
            std::fs::remove_file(path).unwrap();
        }
    }

    /// MailCraft: the bodies of `folder`'s messages that read back as `body`, and how many
    /// messages it lists.
    fn bodies_that_read(path: &Path, folder: &[&str], body: &str) -> (usize, usize) {
        let store = UnicodeStore::read(Rc::new(UnicodePstFile::open(path).unwrap())).unwrap();
        let folder = folder_at_path(&store, folder);
        let ids = folder
            .contents_table()
            .unwrap()
            .rows_matrix()
            .map(|row| NodeId::from(u32::from(row.id())))
            .collect::<Vec<_>>();
        let good = ids
            .iter()
            .filter(|id| {
                let Ok(entry_id) = store.properties().make_entry_id(**id) else {
                    return false;
                };
                let Ok(message) = store.open_message(&entry_id, None) else {
                    return false;
                };
                matches!(message.properties().get(0x1000),
                    Some(PropertyValue::Unicode(value)) if value.to_string() == body)
            })
            .count();
        (good, ids.len())
    }

    /// MailCraft: the space a file's allocation map marks free, as an append takes it.
    fn free_space(path: &Path) -> Vec<(u64, u64)> {
        let mut file = File::open(path).unwrap();
        let header = UnicodeHeader::read(&mut file).unwrap();
        let root = header.root();
        let bbt = read_block_btree(&mut file, Format::Unicode, *root.block_btree()).unwrap();
        let nbt = read_node_btree(&mut file, Format::Unicode, *root.node_btree()).unwrap();
        FileSpace::read(&mut file, &FileHeader::Unicode(header.clone()), &bbt, &nbt)
            .unwrap()
            .free
    }

    #[test]
    fn a_damaged_allocation_map_is_not_trusted() {
        let message = |body| UnicodePstMessage {
            subject: "kept",
            sender_name: "sender",
            sender_email: "sender@example.com",
            recipients: &[],
            body,
            html_body: None,
            message_id: "<kept@example.com>",
            delivery_time: 133_750_080_000_000_000,
            message_class: "IPM.Note",
            properties: &[],
        };
        let body = "a body that must survive ".repeat(200);
        for (label, fix_crc) in [("amap-crc", true), ("amap-stale", false)] {
            // Outlook's own file, with commits enough to free space the map then offers.
            let path = encoded_outlook_pst(label, NdbCryptMethod::Permute);
            for _ in 0..60 {
                drop(UnicodePstFile::append_in_folder(&path, &["Kept"], &message(&body)).unwrap());
            }
            assert_eq!(bodies_that_read(&path, &["Kept"], &body), (60, 60));
            assert!(
                !free_space(&path).is_empty(),
                "a sound map's free space is used"
            );

            // The last map page now marks live blocks free; its CRC is either made to match
            // (the map is wrong but reads) or left stale (the page doesn't read).
            let mut bytes = std::fs::read(&path).unwrap();
            let mut reader = Cursor::new(&bytes);
            let header = UnicodeHeader::read(&mut reader).unwrap();
            let amap = usize::try_from(header.root().amap_last_index().index()).unwrap();
            for byte in &mut bytes[amap..amap + 340] {
                *byte = 0;
            }
            if fix_crc {
                let crc = crate::crc::compute_crc(0, &bytes[amap..amap + 496]);
                bytes[amap + 500..amap + 504].copy_from_slice(&crc.to_le_bytes());
            }
            std::fs::write(&path, &bytes).unwrap();
            assert!(free_space(&path).is_empty(), "{label}");

            // New blocks used to go over old ones: as the bits fell, the append succeeded with
            // old messages, or the new one, no longer reading, or failed once the header had
            // switched to the new trees.
            drop(UnicodePstFile::append_in_folder(&path, &["Kept"], &message(&body)).unwrap());
            assert_eq!(
                bodies_that_read(&path, &["Kept"], &body),
                (61, 61),
                "{label}"
            );
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn a_file_added_to_is_read_within_the_limit() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "outlook-pst-add-limit-{}-{stamp}.pst",
            std::process::id()
        ));
        let body = "row".repeat(10);
        let message = UnicodePstMessage {
            subject: "a row of the contents table",
            sender_name: "sender",
            sender_email: "sender@example.com",
            recipients: TEST_RECIPIENTS,
            body: &body,
            html_body: None,
            message_id: "<row@example.com>",
            delivery_time: 133_750_080_000_000_000,
            message_class: "IPM.Note",
            properties: &[],
        };
        drop(UnicodePstFile::create_in_folder(&path, &["Rows"], &message).unwrap());
        drop(UnicodePstFile::append_many_in_folder(&path, &["Rows"], &[message; 300]).unwrap());
        let before = std::fs::read(&path).unwrap();

        // Below what the folder's tables hold, the append is an error and the file is as it
        // was; the writer used to read whatever size the file declared, up to 4 GiB.
        TEST_ADD_MAX_VALUE_SIZE.with(|limit| limit.set(Some(4096)));
        assert!(UnicodePstFile::append_in_folder(&path, &["Rows"], &message).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        TEST_ADD_MAX_VALUE_SIZE.with(|limit| limit.set(None));
        drop(UnicodePstFile::append_in_folder(&path, &["Rows"], &message).unwrap());
        let store = UnicodeStore::read(Rc::new(UnicodePstFile::open(&path).unwrap())).unwrap();
        let rows = folder_at_path(&store, &["Rows"]);
        assert_eq!(rows.properties().content_count().unwrap(), 302);
        drop((rows, store));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn externalizes_large_folder_properties() {
        let value = vec![0x5A; MAX_HN_ALLOCATION_SIZE + 1];
        let (pages, external) = property_context_pages(
            [(
                0x300B,
                PropertyValue::Binary(BinaryValue::new(value.clone())),
            )],
            0x300,
            Text::Unicode,
        )
        .unwrap();
        assert!(!pages.is_empty());
        assert_eq!(external.len(), 1);
        assert_eq!(
            external[0].0,
            NodeId::new(NodeIdType::Internal, 0x300).unwrap()
        );
        assert_eq!(external[0].1, value);
    }

    #[test]
    fn externalizes_unicode_body_at_heap_page_boundary() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "outlook-pst-boundary-body-{}-{stamp}.pst",
            std::process::id()
        ));
        let body = "A".repeat(4_081);
        assert_eq!(body.encode_utf16().count() * 2, 8_162);
        assert_eq!(utf16_bytes(&body).len(), 8_164);
        let input = UnicodePstMessage {
            subject: "boundary body",
            sender_name: "sender",
            sender_email: "sender@example.com",
            recipients: TEST_RECIPIENTS,
            body: &body,
            html_body: None,
            message_id: "<boundary-body@example.com>",
            delivery_time: 133_750_080_000_000_000,
            message_class: "IPM.Note",
            properties: &[],
        };

        drop(UnicodePstFile::create(&path, &input).unwrap());
        let message = last_inbox_message(&path);
        match message.properties().get(0x1000).unwrap() {
            PropertyValue::Unicode(value) => assert_eq!(value.to_string(), body),
            value => panic!("unexpected body value: {value:?}"),
        }
        drop(message);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn writes_an_empty_attachment() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "outlook-pst-empty-attachment-{}-{stamp}.pst",
            std::process::id()
        ));
        let message = UnicodePstMessage {
            subject: "empty attachment",
            sender_name: "sender",
            sender_email: "sender@example.com",
            recipients: TEST_RECIPIENTS,
            body: "body",
            html_body: None,
            message_id: "<empty-attachment@example.com>",
            delivery_time: 133_750_080_000_000_000,
            message_class: "IPM.Note",
            properties: &[],
        };
        drop(
            UnicodePstFile::create_with_attachments(
                &path,
                &message,
                &[UnicodePstAttachment {
                    filename: "empty.txt",
                    mime_type: "text/plain",
                    content_id: None,
                    data: &[],
                    ..Default::default()
                }],
            )
            .unwrap(),
        );
        drop(
            UnicodePstFile::append_with_attachments(
                &path,
                &message,
                &[UnicodePstAttachment {
                    filename: "empty.txt",
                    mime_type: "text/plain",
                    content_id: None,
                    data: &[],
                    ..Default::default()
                }],
            )
            .unwrap(),
        );

        let store = UnicodeStore::read(Rc::new(UnicodePstFile::open(&path).unwrap())).unwrap();
        let inbox = folder_at_path(&store, &["Inbox"]);
        let message_id = NodeId::from(u32::from(
            inbox
                .contents_table()
                .unwrap()
                .rows_matrix()
                .last()
                .unwrap()
                .id(),
        ));
        let message = UnicodeMessage::read(
            store.clone(),
            &store.properties().make_entry_id(message_id).unwrap(),
            None,
        )
        .unwrap();
        let attachment_id = NodeId::from(u32::from(
            message
                .attachment_table()
                .unwrap()
                .rows_matrix()
                .next()
                .unwrap()
                .id(),
        ));
        let attachment = UnicodeAttachment::read(message, attachment_id, None).unwrap();
        assert_eq!(attachment.properties().attachment_size().unwrap(), 74);
        match attachment.data().unwrap() {
            AttachmentData::Binary(value) => assert!(value.buffer().is_empty()),
            AttachmentData::Message(_) => panic!("unexpected embedded message"),
        }
        std::fs::remove_file(path).unwrap();
    }
}

struct AllocationMapPageInfo<Pst>
where
    Pst: PstFile,
    <Pst as PstFile>::AllocationMapPage: AllocationMapPageReadWrite<Pst>,
{
    amap_page: <Pst as PstFile>::AllocationMapPage,
    free_space: u64,
}

impl<Pst> AllocationMapPageInfo<Pst>
where
    Pst: PstFile,
    <Pst as PstFile>::AllocationMapPage: AllocationMapPageReadWrite<Pst>,
{
    fn max_free_slots(&self) -> u8 {
        u8::try_from(self.amap_page.find_free_bits(0xFF).len()).unwrap_or(0xFF)
    }
}

type PstFileReadWriteBTree<Pst, BTree> = RootBTreePage<
    Pst,
    <BTree as RootBTree>::Entry,
    <BTree as RootBTree>::IntermediatePage,
    <BTree as RootBTree>::LeafPage,
>;

type PstFileReadWriteNodeBTree<Pst> = PstFileReadWriteBTree<Pst, <Pst as PstFile>::NodeBTree>;

type PstFileReadWriteBlockBTree<Pst> = PstFileReadWriteBTree<Pst, <Pst as PstFile>::BlockBTree>;

impl<Pst> PstFileInner<Pst>
where
    Pst: PstFile + PstFileLock<Pst>,
    <Pst as PstFile>::BlockId: BlockId<Index = <Pst as PstFile>::BTreeKey>
        + From<<<Pst as PstFile>::ByteIndex as ByteIndex>::Index>
        + Debug,
    <Pst as PstFile>::PageId: From<<<Pst as PstFile>::ByteIndex as ByteIndex>::Index> + Debug,
    <Pst as PstFile>::ByteIndex: ByteIndex<Index: TryFrom<u64>> + Debug,
    <Pst as PstFile>::BlockRef: Debug,
    <Pst as PstFile>::PageRef: Debug,
    <Pst as PstFile>::Root: RootReadWrite<Pst>,
    <Pst as PstFile>::Header: HeaderReadWrite<Pst>,
    <Pst as PstFile>::DensityListPage: DensityListPageReadWrite<Pst>,
    <Pst as PstFile>::PageTrailer: PageTrailerReadWrite,
    <Pst as PstFile>::BTreeKey: BTreePageKeyReadWrite,
    <Pst as PstFile>::NodeBTreeEntry: NodeBTreeEntryReadWrite,
    <Pst as PstFile>::NodeBTree: NodeBTreeReadWrite<Pst, <Pst as PstFile>::NodeBTreeEntry>,
    <<Pst as PstFile>::NodeBTree as RootBTree>::IntermediatePage:
        RootBTreeIntermediatePageReadWrite<
            Pst,
            <Pst as PstFile>::NodeBTreeEntry,
            <<Pst as PstFile>::NodeBTree as RootBTree>::LeafPage,
        >,
    <<<Pst as PstFile>::NodeBTree as RootBTree>::IntermediatePage as BTreePage>::Entry:
        BTreePageEntryReadWrite,
    <<Pst as PstFile>::NodeBTree as RootBTree>::LeafPage: RootBTreeLeafPageReadWrite<Pst>,
    <Pst as PstFile>::BlockBTreeEntry: BlockBTreeEntryReadWrite,
    <Pst as PstFile>::BlockBTree: BlockBTreeReadWrite<Pst, <Pst as PstFile>::BlockBTreeEntry>,
    <<Pst as PstFile>::BlockBTree as RootBTree>::IntermediatePage:
        RootBTreeIntermediatePageReadWrite<
            Pst,
            <Pst as PstFile>::BlockBTreeEntry,
            <<Pst as PstFile>::BlockBTree as RootBTree>::LeafPage,
        >,
    <<<Pst as PstFile>::BlockBTree as RootBTree>::IntermediatePage as BTreePage>::Entry:
        BTreePageEntryReadWrite,
    <<Pst as PstFile>::BlockBTree as RootBTree>::LeafPage: RootBTreeLeafPageReadWrite<Pst>,
    <Pst as PstFile>::BlockTrailer: BlockTrailerReadWrite,
    <Pst as PstFile>::AllocationMapPage: AllocationMapPageReadWrite<Pst>,
    <Pst as PstFile>::AllocationPageMapPage: AllocationPageMapPageReadWrite<Pst>,
    <Pst as PstFile>::FreeMapPage: FreeMapPageReadWrite<Pst>,
    <Pst as PstFile>::FreePageMapPage: FreePageMapPageReadWrite<Pst>,
    <Pst as PstFile>::DensityListPage: DensityListPageReadWrite<Pst>,
    <Pst as PstFile>::DataTreeBlock: IntermediateTreeBlockReadWrite,
    <Pst as PstFile>::DataTreeEntry:
        IntermediateTreeEntryReadWrite + From<<Pst as PstFile>::BlockId>,
    <Pst as PstFile>::DataBlock: BlockReadWrite + Clone,
    <Pst as PstFile>::SubNodeTreeBlockHeader: SubNodeTreeBlockHeaderReadWrite,
    <Pst as PstFile>::SubNodeTreeBlock: IntermediateTreeBlockReadWrite,
    <<Pst as PstFile>::SubNodeTreeBlock as IntermediateTreeBlock>::Entry:
        IntermediateTreeEntryReadWrite,
    <Pst as PstFile>::SubNodeBlock: IntermediateTreeBlockReadWrite,
    <<Pst as PstFile>::SubNodeBlock as IntermediateTreeBlock>::Entry:
        IntermediateTreeEntryReadWrite,
{
    fn read_from(mut reader: Box<dyn PstReader>) -> io::Result<Self> {
        let header = <<Pst as PstFile>::Header as HeaderReadWrite<Pst>>::read(&mut reader)?;
        let density_list =
            <<Pst as PstFile>::DensityListPage as DensityListPageReadWrite<Pst>>::read(&mut reader);
        Ok(Self {
            reader: Mutex::new(Box::new(reader)),
            writer: Err(PstError::OpenedReadOnly),
            header,
            density_list,
            node_cache: Default::default(),
            block_cache: Default::default(),
            max_value_size: AtomicU64::new(u64::MAX),
        })
    }

    fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let reader = Box::new(File::open(&path)?);
        let writer = OpenOptions::new()
            .write(true)
            .open(&path)
            .map(BufWriter::new)
            .map(Mutex::new)
            .map_err(|_| PstError::NoWriteAccess(path.as_ref().display().to_string()));
        Ok(Self {
            writer,
            ..Self::read_from(reader)?
        })
    }

    /// Begin a transaction by rebuilding the allocation map if needed and initializing the density
    /// list, then set [`AmapStatus::Invalid`] in the header till the transaction is finished.
    ///
    /// See also [Transactional Semantics](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-pst/bc5a92df-7fc1-4dc2-9c7c-5677237dd73a).
    fn start_write(&mut self) -> io::Result<()> {
        self.rebuild_allocation_map()?;
        self.ensure_density_list()?;

        let header = {
            self.header.update_unique();

            let root = self.header.root_mut();
            root.set_amap_status(AmapStatus::Invalid);
            self.header.clone()
        };

        let mut writer = self
            .writer
            .as_ref()?
            .lock()
            .map_err(|_| PstError::LockError)?;
        let writer = &mut *writer;
        writer.seek(SeekFrom::Start(0))?;
        header.write(writer)?;
        writer.flush()
    }

    /// Complete a transaction by writing the header and density list to the file, and setting
    /// [`AmapStatus::Valid2`].
    ///
    /// See also [Transactional Semantics](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-pst/bc5a92df-7fc1-4dc2-9c7c-5677237dd73a).
    #[instrument(skip_all)]
    fn finish_write(&mut self) -> io::Result<()> {
        // Reset AmapStatus::Valid2 to complete the transaction and then rewrite the updated
        // density list.
        let header = {
            self.header.update_unique();
            let root = self.header.root_mut();
            root.set_amap_status(AmapStatus::Valid2);
            self.header.clone()
        };

        self.update_density_list_page_id()?;
        let density_list = {
            self.density_list.as_ref().ok().and_then(|dl| {
                <<Pst as PstFile>::DensityListPage as DensityListPageReadWrite<Pst>>::new(
                    dl.backfill_complete(),
                    dl.current_page(),
                    dl.entries(),
                    *dl.trailer(),
                )
                .ok()
            })
        };

        let mut writer = self
            .writer
            .as_ref()?
            .lock()
            .map_err(|_| PstError::LockError)?;
        let writer = &mut *writer;
        writer.seek(SeekFrom::Start(0))?;
        header.write(writer)?;
        writer.flush()?;

        if let Some(density_list) = density_list {
            density_list.write(writer)?;
            writer.flush()?;
        }

        Ok(())
    }

    /// [Crash Recovery and AMap Rebuilding](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-pst/d9bcc1fd-c66a-41b3-b6d7-ed09d2a25ced)
    fn rebuild_allocation_map(&mut self) -> io::Result<()> {
        self.rebuild_allocation_map_from(None)
    }

    /// Rebuild the allocation map, marking `known` blocks `(offset, size)` and B-tree page
    /// offsets when the caller already has them instead of walking both B-trees on disk.
    fn rebuild_allocation_map_from(
        &mut self,
        known: Option<KnownAllocations<'_>>,
    ) -> io::Result<()> {
        let root = self.header.root();
        if AmapStatus::Invalid != root.amap_is_valid() {
            return Ok(());
        }

        // MailCraft: saturating, so an end of file before the first map is no map, not a panic.
        let num_amap_pages = root
            .file_eof_index()
            .index()
            .into()
            .saturating_sub(AMAP_FIRST_OFFSET);
        let num_amap_pages = num_amap_pages.div_ceil(AMAP_DATA_SIZE);

        let mut amap_pages: Vec<_> = (0..num_amap_pages)
            .map(|amap_index| {
                let index =
                    <<<Pst as PstFile>::ByteIndex as ByteIndex>::Index as TryFrom<u64>>::try_from(
                        amap_index * AMAP_DATA_SIZE + AMAP_FIRST_OFFSET,
                    )
                    .map_err(|_| PstError::IntegerConversion)?;
                let block_id = <Pst as PstFile>::PageId::from(index);

                let trailer = <<Pst as PstFile>::PageTrailer as PageTrailerReadWrite>::new(
                    PageType::AllocationMap,
                    0,
                    block_id,
                    0,
                );

                let mut map_bits = [0; mem::size_of::<MapBits>()];
                let reserved = map_page_count(amap_index);

                let free_space = AMAP_DATA_SIZE - reserved * PAGE_SIZE as u64;

                let reserved = &[0xFF; 4][..reserved as usize];
                map_bits[..reserved.len()].copy_from_slice(reserved);

                let amap_page =
                    <<Pst as PstFile>::AllocationMapPage as AllocationMapPageReadWrite<Pst>>::new(
                        map_bits, trailer,
                    )?;
                Ok(AllocationMapPageInfo::<Pst> {
                    amap_page,
                    free_space,
                })
            })
            .collect::<PstResult<Vec<_>>>()?;

        if let Some((blocks, pages)) = known {
            for page in pages {
                Self::mark_page_allocation(*page, &mut amap_pages)?;
            }
            for (offset, size) in blocks {
                Self::mark_block_allocation(*offset, *size, &mut amap_pages)?;
            }
        } else {
            let mut reader = self.reader.lock().map_err(|_| PstError::LockError)?;
            let reader = &mut *reader;

            let node_btree =
                <Pst::NodeBTree as RootBTreeReadWrite>::read(reader, *root.node_btree())?;

            Self::mark_node_btree_allocations(
                reader,
                root.node_btree().index(),
                &node_btree,
                &mut amap_pages,
            )?;

            let block_btree =
                <Pst::BlockBTree as RootBTreeReadWrite>::read(reader, *root.block_btree())?;

            Self::mark_block_btree_allocations(
                reader,
                root.block_btree().index(),
                &block_btree,
                &mut amap_pages,
            )?;
        }

        let free_bytes =
            <<<Pst as PstFile>::ByteIndex as ByteIndex>::Index as TryFrom<u64>>::try_from(
                amap_pages.iter().map(|page| page.free_space).sum(),
            )
            .map_err(|_| PstError::IntegerConversion)?;
        let free_bytes = <<Pst as PstFile>::ByteIndex as ByteIndexReadWrite>::new(free_bytes);

        let pmap_pages: Vec<_> = (0..num_amap_pages.div_ceil(PMAP_PAGE_COUNT))
            .map(|index| {
                let index =
                    <<<Pst as PstFile>::ByteIndex as ByteIndex>::Index as TryFrom<u64>>::try_from(
                        index * PMAP_DATA_SIZE + PMAP_FIRST_OFFSET,
                    )
                    .map_err(|_| PstError::IntegerConversion)?;
                let block_id = <Pst as PstFile>::PageId::from(index);

                let trailer = <<Pst as PstFile>::PageTrailer as PageTrailerReadWrite>::new(
                    PageType::AllocationPageMap,
                    0,
                    block_id,
                    0,
                );

                let map_bits = [0xFF; mem::size_of::<MapBits>()];

                let pmap_page =
                    <<Pst as PstFile>::AllocationPageMapPage as AllocationPageMapPageReadWrite<
                        Pst,
                    >>::new(map_bits, trailer)?;
                Ok(pmap_page)
            })
            .collect::<PstResult<Vec<_>>>()?;

        let fmap_pages: Vec<_> = (0..(num_amap_pages.max(FMAP_FIRST_SIZE) - FMAP_FIRST_SIZE)
            .div_ceil(FMAP_PAGE_COUNT))
            .map(|index| {
                let amap_index =
                    FMAP_FIRST_SIZE as usize + (index as usize * mem::size_of::<MapBits>());
                let index =
                    <<<Pst as PstFile>::ByteIndex as ByteIndex>::Index as TryFrom<u64>>::try_from(
                        index * FMAP_DATA_SIZE + FMAP_FIRST_OFFSET,
                    )
                    .map_err(|_| PstError::IntegerConversion)?;
                let block_id = <Pst as PstFile>::PageId::from(index);

                let trailer = <<Pst as PstFile>::PageTrailer as PageTrailerReadWrite>::new(
                    PageType::FreeMap,
                    0,
                    block_id,
                    0,
                );

                let mut map_bits = [0; mem::size_of::<MapBits>()];
                for (entry, free_space) in map_bits.iter_mut().zip(
                    amap_pages
                        .iter()
                        .skip(amap_index)
                        .map(|page| page.max_free_slots()),
                ) {
                    *entry = free_space;
                }

                let fmap_page = <<Pst as PstFile>::FreeMapPage as FreeMapPageReadWrite<Pst>>::new(
                    map_bits, trailer,
                )?;
                Ok(fmap_page)
            })
            .collect::<PstResult<Vec<_>>>()?;

        let fpmap_pages: Vec<_> = (0..(num_amap_pages.max(FPMAP_FIRST_SIZE) - FPMAP_FIRST_SIZE)
            .div_ceil(FPMAP_PAGE_COUNT))
            .map(|index| {
                let index =
                    <<<Pst as PstFile>::ByteIndex as ByteIndex>::Index as TryFrom<u64>>::try_from(
                        index * FPMAP_DATA_SIZE + FPMAP_FIRST_OFFSET,
                    )
                    .map_err(|_| PstError::IntegerConversion)?;
                let block_id = <Pst as PstFile>::PageId::from(index);

                let trailer = <<Pst as PstFile>::PageTrailer as PageTrailerReadWrite>::new(
                    PageType::FreePageMap,
                    0,
                    block_id,
                    0,
                );

                let map_bits = [0xFF; mem::size_of::<MapBits>()];

                let fpmap_page = <<Pst as PstFile>::FreePageMapPage as FreePageMapPageReadWrite<
                    Pst,
                >>::new(map_bits, trailer)?;
                Ok(fpmap_page)
            })
            .collect::<PstResult<Vec<_>>>()?;

        {
            // Encode every map page, then write only those that differ from the file: the maps
            // sit one per 248 KiB, so rewriting all of them scatters thousands of page writes
            // over a large file, which a slow disk then has to flush.
            let mut encoded: Vec<(u64, Vec<u8>)> = Vec::new();
            let mut encode =
                |offset: u64, write: &dyn Fn(&mut Cursor<Vec<u8>>) -> io::Result<()>| {
                    let mut page = Cursor::new(Vec::with_capacity(PAGE_SIZE));
                    write(&mut page)?;
                    encoded.push((offset, page.into_inner()));
                    Ok::<_, io::Error>(())
                };
            for page in amap_pages.into_iter().map(|info| info.amap_page) {
                encode(page.trailer().block_id().into_u64(), &|f| {
                    <Pst::AllocationMapPage as AllocationMapPageReadWrite<Pst>>::write(&page, f)
                })?;
            }
            for page in pmap_pages.into_iter() {
                encode(page.trailer().block_id().into_u64(), &|f| {
                    <Pst::AllocationPageMapPage as AllocationPageMapPageReadWrite<Pst>>::write(
                        &page, f,
                    )
                })?;
            }
            for page in fmap_pages.into_iter() {
                encode(page.trailer().block_id().into_u64(), &|f| {
                    <Pst::FreeMapPage as FreeMapPageReadWrite<Pst>>::write(&page, f)
                })?;
            }
            for page in fpmap_pages.into_iter() {
                encode(page.trailer().block_id().into_u64(), &|f| {
                    <Pst::FreePageMapPage as FreePageMapPageReadWrite<Pst>>::write(&page, f)
                })?;
            }

            {
                let mut reader = self.reader.lock().map_err(|_| PstError::LockError)?;
                let mut current = vec![0; PAGE_SIZE];
                encoded.retain(|(offset, page)| {
                    current.resize(page.len(), 0);
                    let unchanged = reader.seek(SeekFrom::Start(*offset)).is_ok()
                        && reader.read_exact(&mut current).is_ok()
                        && current == *page;
                    !unchanged
                });
            }

            let mut writer = self
                .writer
                .as_ref()?
                .lock()
                .map_err(|_| PstError::LockError)?;
            let writer = &mut *writer;
            for (offset, page) in encoded {
                writer.seek(SeekFrom::Start(offset))?;
                writer.write_all(&page)?;
            }

            writer.flush()?;
            // The maps must be durable before the header marks them valid.
            writer.get_ref().sync_data()?;
        }

        let header = {
            // MS-PST deprecates the header's rgbFM and rgbFP: they MUST be 0xFF, which is what Outlook writes and SCANPST restores.
            <<Pst as PstFile>::Header as HeaderReadWrite<Pst>>::first_free_map(&mut self.header)
                .fill(0xFF);
            <<Pst as PstFile>::Header as HeaderReadWrite<Pst>>::first_free_page_map(
                &mut self.header,
            )
            .fill(0xFF);
            self.header.update_unique();

            let root = self.header.root_mut();
            let amap_last_index =
                AMAP_FIRST_OFFSET + num_amap_pages.saturating_sub(1) * AMAP_DATA_SIZE;
            let amap_last_index = <<<Pst as PstFile>::ByteIndex as ByteIndex>::Index as TryFrom<
                u64,
            >>::try_from(amap_last_index)
            .map_err(|_| PstError::IntegerConversion)?;
            root.set_amap_last_index(<<Pst as PstFile>::ByteIndex as ByteIndexReadWrite>::new(
                amap_last_index,
            ));
            root.reset_free_size(free_bytes)?;
            root.set_amap_status(AmapStatus::Valid2);

            self.header.clone()
        };

        let mut writer = self
            .writer
            .as_ref()?
            .lock()
            .map_err(|_| PstError::LockError)?;
        let writer = &mut *writer;
        writer.seek(SeekFrom::Start(0))?;
        header.write(writer)?;
        writer.flush()
    }

    /// Recursively mark all of the pages in the [`Node BTree`](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-pst/7d759bcb-7864-480c-8746-f6af913ab085).
    /// as allocated. This does not include any blocks referenced in the nodes or the sub-trees in
    /// those blocks, blocks will be marked by [`Self::mark_block_btree_allocations`].
    ///
    /// See also [Crash Recovery and AMap Rebuilding](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-pst/d9bcc1fd-c66a-41b3-b6d7-ed09d2a25ced).
    #[instrument(skip_all)]
    fn mark_node_btree_allocations<R: PstReader>(
        reader: &mut R,
        page_index: Pst::ByteIndex,
        node_btree: &PstFileReadWriteNodeBTree<Pst>,
        amap_pages: &mut Vec<AllocationMapPageInfo<Pst>>,
    ) -> io::Result<()> {
        Self::mark_page_allocation(page_index.index().into(), amap_pages)?;

        if let RootBTreePage::Intermediate(page, ..) = node_btree {
            let level = page.level();
            for entry in page.entries() {
                let block = entry.block();
                let node_btree = <Pst::NodeBTree as RootBTreeReadWrite>::read(reader, block)?;
                match &node_btree {
                    RootBTreePage::Intermediate(page, ..) if page.level() + 1 != level => {
                        error!(
                            name: "PstUnexpectedBTreeIntermediatePage",
                            block = ?block.block(),
                            index = ?block.index(),
                            parent = level,
                            child = page.level(),
                            "Possible NBT page cycle detected, expected child == parent - 1"
                        );
                        return Err(PstError::InvalidBTreePage(block.index().index().into()).into());
                    }
                    RootBTreePage::Leaf(_) if level != 1 => {
                        error!(
                            name: "PstUnexpectedBTreeLeafPage",
                            block = ?block.block(),
                            index = ?block.index(),
                            parent = level,
                            child = page.level(),
                            "Corrupted NBT intermediate page detected, unexpected child leaf page"
                        );
                        return Err(PstError::InvalidBTreePage(block.index().index().into()).into());
                    }
                    _ => (),
                }
                Self::mark_node_btree_allocations(reader, block.index(), &node_btree, amap_pages)?;
            }
        }

        Ok(())
    }

    /// Recursively mark all of the pages and blocks in the [`Block BTree`](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-pst/7d759bcb-7864-480c-8746-f6af913ab085).
    ///
    /// See also [Crash Recovery and AMap Rebuilding](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-pst/d9bcc1fd-c66a-41b3-b6d7-ed09d2a25ced).
    #[instrument(skip_all)]
    fn mark_block_btree_allocations<R: PstReader>(
        reader: &mut R,
        page_index: Pst::ByteIndex,
        block_btree: &PstFileReadWriteBlockBTree<Pst>,
        amap_pages: &mut Vec<AllocationMapPageInfo<Pst>>,
    ) -> io::Result<()> {
        Self::mark_page_allocation(page_index.index().into(), amap_pages)?;

        match block_btree {
            RootBTreePage::Intermediate(page, ..) => {
                let level = page.level();
                for entry in page.entries() {
                    let block = entry.block();
                    let block_btree = <Pst::BlockBTree as RootBTreeReadWrite>::read(reader, block)?;
                    match &block_btree {
                        RootBTreePage::Intermediate(page, ..) if page.level() + 1 != level => {
                            error!(
                                name: "PstUnexpectedBTreeIntermediatePage",
                                block = ?block.block(),
                                index = ?block.index(),
                                parent = level,
                                child = page.level(),
                                "Possible BBT page cycle detected, expected child == parent - 1"
                            );
                            return Err(
                                PstError::InvalidBTreePage(block.index().index().into()).into()
                            );
                        }
                        RootBTreePage::Leaf(_) if level != 1 => {
                            error!(
                                name: "PstUnexpectedBTreeLeafPage",
                                block = ?block.block(),
                                index = ?block.index(),
                                parent = level,
                                child = page.level(),
                                "Corrupted BBT intermediate page detected, unexpected child leaf page"
                            );
                            return Err(
                                PstError::InvalidBTreePage(block.index().index().into()).into()
                            );
                        }
                        _ => (),
                    }
                    Self::mark_block_btree_allocations(
                        reader,
                        block.index(),
                        &block_btree,
                        amap_pages,
                    )?;
                }
            }
            RootBTreePage::Leaf(page) => {
                for entry in page.entries() {
                    Self::mark_block_allocation(
                        entry.block().index().index().into(),
                        entry.size(),
                        amap_pages,
                    )?;
                }
            }
        }
        Ok(())
    }

    /// Mark a page at the given file offset as allocated.
    fn mark_page_allocation(
        index: u64,
        amap_pages: &mut [AllocationMapPageInfo<Pst>],
    ) -> io::Result<()> {
        Self::mark_allocation(index, PAGE_SIZE as u64, amap_pages)
    }

    /// Mark a block at the given file offset and size as allocated.
    fn mark_block_allocation(
        index: u64,
        size: u16,
        amap_pages: &mut [AllocationMapPageInfo<Pst>],
    ) -> io::Result<()> {
        // MailCraft: in u32, where a block size near u16::MAX overflowed.
        let size = u32::from(size)
            + u32::from(<<Pst as PstFile>::BlockTrailer as BlockTrailerReadWrite>::SIZE);
        let size = u64::from(size).next_multiple_of(64);
        Self::mark_allocation(index, size, amap_pages)
    }

    /// MailCraft: mark `size` bytes at file offset `index` as allocated, one bit per 64 bytes.
    /// An offset before the first allocation map, a range that runs past its map, or a map's
    /// free space going below zero is an error, where each one overflowed or indexed out of
    /// bounds; a damaged file's B-tree can name any of them.
    fn mark_allocation(
        index: u64,
        size: u64,
        amap_pages: &mut [AllocationMapPageInfo<Pst>],
    ) -> io::Result<()> {
        let index = index
            .checked_sub(AMAP_FIRST_OFFSET)
            .ok_or(PstError::InvalidBTreePage(index))?;
        let amap_index =
            usize::try_from(index / AMAP_DATA_SIZE).map_err(|_| PstError::IntegerConversion)?;
        let entry = amap_pages
            .get_mut(amap_index)
            .ok_or(PstError::AllocationMapPageNotFound(amap_index))?;
        entry.free_space = entry
            .free_space
            .checked_sub(size)
            .ok_or(PstError::AllocationMapPageNotFound(amap_index))?;

        let bytes = entry.amap_page.map_bits_mut();
        let bit_start = usize::try_from((index % AMAP_DATA_SIZE) / 64)
            .map_err(|_| PstError::IntegerConversion)?;
        let bit_end = usize::try_from(size / 64)
            .ok()
            .and_then(|bits| bit_start.checked_add(bits))
            .ok_or(PstError::IntegerConversion)?;
        if bit_end > bytes.len() * 8 {
            return Err(PstError::AllocationMapPageNotFound(amap_index.saturating_add(1)).into());
        }
        let mut bit = bit_start;
        while bit < bit_end {
            if bit % 8 == 0 && bit + 8 <= bit_end {
                bytes[bit / 8] = 0xFF;
                bit += 8;
            } else {
                bytes[bit / 8] |= 0x80 >> (bit % 8);
                bit += 1;
            }
        }

        Ok(())
    }

    /// Initialize the density list at the beginning of a transaction if it is missing, corrupt, or
    /// the page ID doesn't match the next page ID in the header.
    fn ensure_density_list(&mut self) -> PstResult<()> {
        if let Ok(density_list) = self.density_list.as_ref() {
            if density_list.trailer().block_id() == self.header.next_page() {
                return Ok(());
            }
        }

        let current_page = u32::try_from(
            (self.header.root().amap_last_index().index().into() - AMAP_FIRST_OFFSET)
                / AMAP_DATA_SIZE,
        )
        .map_err(|_| PstError::IntegerConversion)?;
        let block_id = self.header.next_page();
        let signature = PageType::DensityList
            .signature(ndb::page::DENSITY_LIST_FILE_OFFSET, block_id.into_u64());
        let trailer = <<Pst as PstFile>::PageTrailer as PageTrailerReadWrite>::new(
            PageType::DensityList,
            signature,
            block_id,
            0,
        );
        let density_list =
            <<Pst as PstFile>::DensityListPage as DensityListPageReadWrite<Pst>>::new(
                false,
                current_page,
                &[],
                trailer,
            )?;

        self.density_list = Ok(density_list);
        Ok(())
    }

    /// Similar to [`Self::ensure_density_list`], but instead of resetting the density list, it
    /// assumes that it's already initialized and only updates the page ID if it doesn't match.
    fn update_density_list_page_id(&mut self) -> PstResult<()> {
        let Ok(density_list) = self.density_list.as_ref() else {
            return Ok(());
        };

        let next_page = self.header.next_page();
        if density_list.trailer().block_id() == next_page {
            return Ok(());
        }

        let signature = PageType::DensityList
            .signature(ndb::page::DENSITY_LIST_FILE_OFFSET, next_page.into_u64());
        let trailer = <<Pst as PstFile>::PageTrailer as PageTrailerReadWrite>::new(
            PageType::DensityList,
            signature,
            next_page,
            0,
        );

        let density_list =
            <<Pst as PstFile>::DensityListPage as DensityListPageReadWrite<Pst>>::new(
                density_list.backfill_complete(),
                density_list.current_page(),
                density_list.entries(),
                trailer,
            )?;

        self.density_list = Ok(density_list);
        Ok(())
    }

    fn read_node(&self, node: NodeId) -> io::Result<<Pst as PstFile>::NodeBTreeEntry> {
        let node_btree = *self.header.root().node_btree();
        let mut reader = self.reader.lock().map_err(|_| PstError::LockError)?;
        let reader = &mut *reader;
        let node_btree =
            <<Pst as PstFile>::NodeBTree as RootBTreeReadWrite>::read(reader, node_btree)?;
        let mut page_cache = self.node_cache.borrow_mut();
        let node_id: <Pst as PstFile>::BTreeKey = u32::from(node).into();
        let node = node_btree.find_entry(reader, node_id, &mut page_cache)?;
        Ok(node)
    }

    fn read_block(&self, block: <Pst as PstFile>::BlockId) -> io::Result<Vec<u8>> {
        let encoding = self.header.crypt_method();
        let block_btree = *self.header.root().block_btree();
        let mut reader = self.reader.lock().map_err(|_| PstError::LockError)?;
        let reader = &mut *reader;
        let block_btree =
            <<Pst as PstFile>::BlockBTree as RootBTreeReadWrite>::read(reader, block_btree)?;
        let mut page_cache = self.block_cache.borrow_mut();
        let block = block_btree.find_entry(reader, block.search_key(), &mut page_cache)?;
        let block = DataTree::<Pst>::read(reader, encoding, &block)?;
        let mut block_cache = Default::default();
        // MailCraft: no more than the tree declares, nor than the file's limit.
        block.read_capped(
            reader,
            encoding,
            &block_btree,
            &mut page_cache,
            &mut block_cache,
            self.max_value_size.load(Ordering::Relaxed),
        )
    }
}

pub fn open_store(path: impl AsRef<Path>) -> io::Result<Rc<dyn Store>> {
    Ok(if let Ok(pst_file) = UnicodePstFile::open(path.as_ref()) {
        UnicodeStore::read(Rc::new(pst_file))?
    } else {
        let pst_file = AnsiPstFile::open(path.as_ref())?;
        AnsiStore::read(Rc::new(pst_file))?
    })
}

/// MailCraft: the writer reads a file it adds to the way the reader reads one, so a damaged
/// file is an error rather than a panic, a stack overflow or a walk that never ends.
#[cfg(test)]
mod existing_file_tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn scratch_file(label: &str) -> (std::path::PathBuf, File) {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "outlook-pst-{label}-{}-{stamp}.pst",
            std::process::id()
        ));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        (path, file)
    }

    /// Write `block` at its offset and return its block B-tree entry.
    fn put(file: &mut File, block: NewBlock) -> UnicodeBlockBTreeEntry {
        let entry = UnicodeBlockBTreeEntry::new(
            UnicodeBlockRef::new(block.id, UnicodeByteIndex::new(block.offset)),
            block.size,
        );
        write_new_block(file, NdbCryptMethod::None, block).unwrap();
        entry
    }

    fn xblock(
        id: UnicodeBlockId,
        offset: u64,
        children: &[UnicodeBlockId],
        total: u32,
    ) -> NewBlock {
        NewBlock {
            id,
            offset,
            size: u16::try_from(8 + 8 * children.len()).unwrap(),
            data: NewBlockData::DataTree(
                children
                    .iter()
                    .map(|id| UnicodeDataTreeEntry::from(*id))
                    .collect(),
                1,
                total,
            ),
        }
    }

    #[test]
    fn a_data_tree_that_names_itself_is_an_error() {
        let (path, mut file) = scratch_file("xblock-self");
        let id = UnicodeBlockId::new(true, 2).unwrap();
        let entry = put(&mut file, xblock(id, 0, &[id], 100));
        // It used to recurse until the stack overflowed.
        assert!(data_tree_leaves(
            &mut file,
            Format::Unicode,
            NdbCryptMethod::None,
            &[entry],
            id
        )
        .is_err());
        drop(file);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn a_leaf_named_over_and_over_counts_up_to_the_declared_size() {
        let (path, mut file) = scratch_file("xblock-repeat");
        let leaf = UnicodeBlockId::new(false, 4).unwrap();
        let leaf_entry = put(
            &mut file,
            NewBlock {
                id: leaf,
                offset: 0,
                size: 100,
                data: NewBlockData::Data(vec![7; 100]),
            },
        );
        let tree = UnicodeBlockId::new(true, 2).unwrap();
        let tree_entry = put(&mut file, xblock(tree, 192, &[leaf; 1000], 250));
        let mut entries = vec![leaf_entry, tree_entry];
        entries.sort_by_key(BTreeEntry::key);
        let leaves = data_tree_leaves(
            &mut file,
            Format::Unicode,
            NdbCryptMethod::None,
            &entries,
            tree,
        )
        .unwrap();
        // 250 bytes declared: three 100-byte blocks reach it, where all 1,000 were listed.
        assert_eq!(leaves.len(), 3);
        drop(file);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn a_block_size_no_block_can_have_is_an_error() {
        let (path, mut file) = scratch_file("block-size");
        file.write_all(&[0; 16384]).unwrap();
        let id = UnicodeBlockId::new(false, 4).unwrap();
        let entry =
            UnicodeBlockBTreeEntry::new(UnicodeBlockRef::new(id, UnicodeByteIndex::new(0)), 9000);
        // `block_size` asserted on it.
        assert!(UnicodeDataTree::read(&mut file, NdbCryptMethod::None, &entry).is_err());
        assert!(UnicodeSubNodeTree::read(&mut file, &entry).is_err());
        drop(file);
        std::fs::remove_file(path).unwrap();
    }

    fn page(offset: u64) -> UnicodePageRef {
        UnicodePageRef::new(UnicodePageId::from(offset), UnicodeByteIndex::new(offset))
    }

    fn btree_with_repeated_leaf(
        _: &mut File,
        page_ref: UnicodePageRef,
    ) -> io::Result<BTreePageItems<UnicodeBlockBTreeEntry>> {
        Ok(match page_ref.index().index() {
            // The root names one leaf twice.
            0x1000 => BTreePageItems::Intermediate(1, vec![(1, page(0x2000)), (2, page(0x2000))]),
            _ => BTreePageItems::Leaf(Vec::new()),
        })
    }

    fn btree_with_a_page_one_level_off(
        _: &mut File,
        page_ref: UnicodePageRef,
    ) -> io::Result<BTreePageItems<UnicodeBlockBTreeEntry>> {
        // Each page says level 1 and names 500 more pages that say the same.
        let offset = page_ref.index().index();
        Ok(BTreePageItems::Intermediate(
            1,
            (1..=500).map(|n| (n, page(offset + n * 512))).collect(),
        ))
    }

    #[test]
    fn a_btree_page_named_twice_or_at_its_own_level_is_an_error() {
        let (path, mut file) = scratch_file("btree-pages");
        // It used to list the leaf's entries twice.
        assert!(BTreeLayout::read(&mut file, page(0x1000), btree_with_repeated_leaf).is_err());
        // It used to read 500^8 pages before its depth limit stopped it.
        assert!(
            BTreeLayout::read(&mut file, page(0x1000), btree_with_a_page_one_level_off).is_err()
        );
        drop(file);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn a_row_too_short_for_its_id_is_an_error() {
        let context = TableContextInfo::new(0, 0, 0, 0, HeapId::default(), None, vec![]).unwrap();
        // It used to panic slicing the row ID into an empty row.
        assert!(encode_table_row(&context, &mut HeapBuilder::new(), 1_u32, 0, vec![]).is_err());
    }

    #[test]
    fn an_allocation_outside_the_maps_is_an_error() {
        let mut pages = Vec::new();
        // Before the first map: it used to overflow.
        assert!(
            PstFileInner::<UnicodePstFile>::mark_block_allocation(0x200, 64, &mut pages).is_err()
        );
        let trailer = <UnicodePageTrailer as PageTrailerReadWrite>::new(
            PageType::AllocationMap,
            0,
            UnicodePageId::from(AMAP_FIRST_OFFSET),
            0,
        );
        pages.push(AllocationMapPageInfo::<UnicodePstFile> {
            amap_page:
                <<UnicodePstFile as PstFile>::AllocationMapPage as AllocationMapPageReadWrite<
                    UnicodePstFile,
                >>::new([0; mem::size_of::<MapBits>()], trailer)
                .unwrap(),
            free_space: AMAP_DATA_SIZE,
        });
        // A block that runs past the map's last bit: it used to index out of bounds.
        let last = AMAP_FIRST_OFFSET + AMAP_DATA_SIZE - 64;
        assert!(
            PstFileInner::<UnicodePstFile>::mark_block_allocation(last, 8000, &mut pages).is_err()
        );
        assert!(PstFileInner::<UnicodePstFile>::mark_block_allocation(
            AMAP_FIRST_OFFSET + 512,
            100,
            &mut pages
        )
        .is_ok());
    }
}
