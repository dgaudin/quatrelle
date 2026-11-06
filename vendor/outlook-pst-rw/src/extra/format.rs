//! MailCraft: the writer adds to Outlook 97–2002's ANSI files as it adds to Unicode ones.
//!
//! MS-PST 2.2.1.2: the two formats differ in the NDB layer, where an ANSI file has 32-bit BIDs
//! and IBs (so 8-byte BREFs, 12-byte block and page trailers, 8,180 bytes of data in a block,
//! smaller B-tree, XBLOCK and SLBLOCK entries and other header offsets), and in its strings,
//! which are all 8-bit (MS-PST 2.5: "for an ANSI PST, all string properties are stored in ANSI
//! encoding"); a table's row index keeps a 16-bit row number (MS-PST 2.3.4.3.1), so a table has
//! at most 2^16 rows, and the file ends before the first FPMap's 2 GB (MS-PST 1.3.2.8).
//!
//! The writer keeps one model of a file, in the Unicode types, whose IDs and offsets are the
//! wider: an ANSI file's header, B-trees and blocks are read with the reader's Ansi types and
//! widened ([`FileHeader`], [`read_block_btree`], [`read_tree_block`]…), and narrowed back, each
//! value checked, where they are written ([`write_ansi_block`], [`write_ansi_btree_page`]).
//! What the writer builds above the NDB layer (heaps, property contexts, tables) is the same in
//! both formats but for the strings ([`Text`]) and the row index ([`Format::row_index_size`]).

use super::*;

mod layout;

pub(crate) use layout::*;

/// Which of MS-PST's two formats a file is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Format {
    Unicode,
    Ansi,
}

/// The end of the space an ANSI file can use: the header's FPMap, the only one an ANSI file
/// has, covers 1,024 PMaps of eight AMaps each from the first AMap (MS-PST 1.3.2.8), about
/// 1.94 GB, the size Outlook 97–2002 stops at.
pub(crate) const ANSI_MAX_FILE_SIZE: u64 = AMAP_FIRST_OFFSET + FPMAP_FIRST_DATA_SIZE;

impl Format {
    /// The format of the file whose first bytes are `head` (MS-PST 2.2.2.6: `wVer` 14 or 15 is
    /// ANSI, 23 or more Unicode).
    pub(crate) fn of(head: &[u8]) -> io::Result<Self> {
        let version = head
            .get(10..12)
            .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
            .ok_or_else(|| invalid_data("the file is too short for a data file's header"))?;
        Ok(match NdbVersion::try_from(version)? {
            NdbVersion::Ansi => Self::Ansi,
            NdbVersion::Unicode => Self::Unicode,
        })
    }

    /// The format of the file at `path`.
    pub(crate) fn of_file(path: &Path) -> io::Result<Self> {
        let mut head = [0; 12];
        File::open(path)?.read_exact(&mut head)?;
        Self::of(&head)
    }

    /// The most data a block holds (MS-PST 2.2.2.8: 8,192 bytes less its trailer). A data tree's
    /// blocks but the last, and a table's row matrix blocks, are this size (MS-PST 2.3.4.4).
    pub(crate) fn data_block_size(self) -> usize {
        match self {
            Self::Unicode => MAX_DATA_BLOCK_SIZE,
            Self::Ansi => (ndb::block::MAX_BLOCK_SIZE - AnsiBlockTrailer::SIZE) as usize,
        }
    }

    pub(crate) fn block_trailer_size(self) -> u16 {
        match self {
            Self::Unicode => UnicodeBlockTrailer::SIZE,
            Self::Ansi => AnsiBlockTrailer::SIZE,
        }
    }

    /// The bytes of a block's entries after its header: a BID in an XBLOCK, an SLENTRY and an
    /// SIENTRY (MS-PST 2.2.2.8.3).
    fn data_tree_entry_size(self) -> usize {
        match self {
            Self::Unicode => usize::from(UnicodeDataTreeEntry::ENTRY_SIZE),
            Self::Ansi => usize::from(AnsiDataTreeEntry::ENTRY_SIZE),
        }
    }

    fn sub_node_header_size(self) -> usize {
        match self {
            Self::Unicode => usize::from(UnicodeSubNodeTreeBlockHeader::HEADER_SIZE),
            Self::Ansi => usize::from(AnsiSubNodeTreeBlockHeader::HEADER_SIZE),
        }
    }

    fn leaf_sub_node_entry_size(self) -> usize {
        match self {
            Self::Unicode => usize::from(UnicodeLeafSubNodeTreeEntry::ENTRY_SIZE),
            Self::Ansi => usize::from(AnsiLeafSubNodeTreeEntry::ENTRY_SIZE),
        }
    }

    fn intermediate_sub_node_entry_size(self) -> usize {
        match self {
            Self::Unicode => usize::from(UnicodeIntermediateSubNodeTreeEntry::ENTRY_SIZE),
            Self::Ansi => usize::from(AnsiIntermediateSubNodeTreeEntry::ENTRY_SIZE),
        }
    }

    /// The BIDs an XBLOCK or XXBLOCK names at most.
    pub(crate) fn data_tree_capacity(self) -> usize {
        (self.data_block_size() - usize::from(DataTreeBlockHeader::HEADER_SIZE))
            / self.data_tree_entry_size()
    }

    /// The entries an SLBLOCK holds at most…
    pub(crate) fn leaf_sub_node_capacity(self) -> usize {
        (self.data_block_size() - self.sub_node_header_size()) / self.leaf_sub_node_entry_size()
    }

    /// …and an SIBLOCK.
    pub(crate) fn intermediate_sub_node_capacity(self) -> usize {
        (self.data_block_size() - self.sub_node_header_size())
            / self.intermediate_sub_node_entry_size()
    }

    /// The bytes of a block with these entries, its header counted.
    pub(crate) fn block_data_size(self, data: &NewBlockData) -> io::Result<u16> {
        let size = match data {
            NewBlockData::Data(data) => data.len(),
            NewBlockData::DataTree(entries, ..) => entries
                .len()
                .checked_mul(self.data_tree_entry_size())
                .and_then(|size| size.checked_add(usize::from(DataTreeBlockHeader::HEADER_SIZE)))
                .ok_or(PstError::IntegerConversion)?,
            NewBlockData::SubNodes(entries) => entries
                .len()
                .checked_mul(self.leaf_sub_node_entry_size())
                .and_then(|size| size.checked_add(self.sub_node_header_size()))
                .ok_or(PstError::IntegerConversion)?,
            NewBlockData::IntermediateSubNodes(entries, ..) => entries
                .len()
                .checked_mul(self.intermediate_sub_node_entry_size())
                .and_then(|size| size.checked_add(self.sub_node_header_size()))
                .ok_or(PstError::IntegerConversion)?,
        };
        Ok(u16::try_from(size).map_err(|_| PstError::IntegerConversion)?)
    }

    /// The most entries a B-tree page holds: a BBT leaf's, an NBT leaf's and an intermediate
    /// page's (MS-PST 2.2.2.7.7.1: 488 bytes of 24- and 32-byte entries, or 496 of 12 and 16).
    pub(crate) fn btree_capacity(self) -> (usize, usize, usize) {
        match self {
            Self::Unicode => (20, 15, 20),
            Self::Ansi => (41, 31, 41),
        }
    }

    /// The bytes of a row index record's row number (MS-PST 2.3.4.3.1).
    pub(crate) fn row_index_size(self) -> u8 {
        match self {
            Self::Unicode => 4,
            Self::Ansi => 2,
        }
    }

    /// How far a file in this format can grow.
    pub(crate) fn max_file_size(self) -> u64 {
        match self {
            Self::Unicode => u64::MAX,
            Self::Ansi => ANSI_MAX_FILE_SIZE,
        }
    }

    /// The text a file in this format keeps outside a message: folders' and the store's names,
    /// in Windows-1252 in an ANSI file, as no property says which code page they are in.
    pub(crate) fn text(self) -> Text {
        match self {
            Self::Unicode => Text::Unicode,
            Self::Ansi => Text::Ansi(CodePage::Windows1252),
        }
    }
}

/// An error unless the file at `path` is in `format`: a Unicode file is written through
/// [`UnicodePstFile`], an ANSI one through [`AnsiPstFile`], and each opens the file it wrote.
pub(crate) fn expect_format(path: &Path, format: Format) -> io::Result<()> {
    if Format::of_file(path)? == format {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        match format {
            Format::Unicode => "the file is in Outlook 97–2002's ANSI format (AnsiPstFile)",
            Format::Ansi => "the file is in Outlook's Unicode format (UnicodePstFile)",
        },
    ))
}

pub(crate) fn invalid_data(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

fn too_big() -> io::Error {
    io::Error::other("an ANSI data file (Outlook 97–2002's format) can't grow past 2 GB")
}

/// The 8-bit code page an ANSI file's strings are written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CodePage {
    /// Windows-1252, Outlook's for western languages: a character it hasn't is a `?`.
    Windows1252,
    /// UTF-8 (65001), for a message whose text Windows-1252 can't hold.
    Utf8,
}

impl CodePage {
    /// `PidTagMessageCodepage` (MS-OXCMSG 2.2.1.4: the code page of the message's non-Unicode
    /// strings).
    pub(crate) fn id(self) -> i32 {
        match self {
            Self::Windows1252 => 1252,
            Self::Utf8 => 65001,
        }
    }

    pub(crate) fn encode(self, value: &str) -> Vec<u8> {
        match self {
            Self::Windows1252 => value
                .chars()
                .map(|c| windows_1252(c).unwrap_or(b'?'))
                .collect(),
            Self::Utf8 => value.as_bytes().to_vec(),
        }
    }
}

/// Whether `stored`, a folder's 8-bit name in an ANSI file, is `name`: `name` as the writer wrote
/// it (in Windows-1252, a `?` for a character it hasn't), or as the reader reads the bytes back
/// (a byte to a character of its own, U+0080 to U+009F among them), ASCII letters in either case.
/// So the same name finds the same folder each time it is asked for, whatever Windows-1252 lacks;
/// names that are written alike (`Письма` and `Отчёты` both `??????`) are one folder.
pub(crate) fn same_ansi_folder_name(stored: &[u8], name: &str) -> bool {
    let stored = stored.strip_suffix(&[0]).unwrap_or(stored);
    let mut wanted = name.chars().map(|c| {
        windows_1252(c)
            .or_else(|| u8::try_from(u32::from(c)).ok())
            .unwrap_or(b'?')
    });
    stored
        .iter()
        .all(|&byte| wanted.next().is_some_and(|c| c.eq_ignore_ascii_case(&byte)))
        && wanted.next().is_none()
}

/// The Windows-1252 byte for `c`, if it has one (its five undefined bytes stand for the C1
/// controls they number, as readers read them).
fn windows_1252(c: char) -> Option<u8> {
    const HIGH: [u16; 32] = [
        0x20AC, 0x0081, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160,
        0x2039, 0x0152, 0x008D, 0x017D, 0x008F, 0x0090, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022,
        0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x009D, 0x017E, 0x0178,
    ];
    let code = u32::from(c);
    if code < 0x80 || (0xA0..=0xFF).contains(&code) {
        return u8::try_from(code).ok();
    }
    HIGH.iter()
        .position(|&high| u32::from(high) == code)
        .and_then(|index| u8::try_from(0x80 + index).ok())
}

/// How the writer writes a string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Text {
    /// `PtypString`, in a Unicode file.
    Unicode,
    /// `PtypString8` in this code page, in an ANSI file.
    Ansi(CodePage),
}

impl Text {
    /// How a message's strings are written in a file in `format`: in Windows-1252 when every one
    /// of `strings` is in it, else in UTF-8.
    pub(crate) fn for_message<'a>(
        format: Format,
        mut strings: impl Iterator<Item = &'a str>,
    ) -> Self {
        match format {
            Format::Unicode => Self::Unicode,
            Format::Ansi
                if strings.all(|value| value.chars().all(|c| windows_1252(c).is_some())) =>
            {
                Self::Ansi(CodePage::Windows1252)
            }
            Format::Ansi => Self::Ansi(CodePage::Utf8),
        }
    }

    /// The format whose strings these are.
    pub(crate) fn format(self) -> Format {
        match self {
            Self::Unicode => Format::Unicode,
            Self::Ansi(_) => Format::Ansi,
        }
    }

    /// The type a string column or property is written with.
    pub(crate) fn string_type(self) -> PropertyType {
        match self {
            Self::Unicode => PropertyType::Unicode,
            Self::Ansi(_) => PropertyType::String8,
        }
    }

    /// `value` as a property value of [`Text::string_type`].
    pub(crate) fn value(self, value: &str) -> PropertyValue {
        match self {
            Self::Unicode => {
                PropertyValue::Unicode(UnicodeValue::new(value.encode_utf16().collect()))
            }
            Self::Ansi(page) => PropertyValue::String8(String8Value::new(page.encode(value))),
        }
    }

    /// A string's bytes as a property context or a table keeps them, with its NUL.
    pub(crate) fn bytes(self, value: &str) -> Vec<u8> {
        match self {
            Self::Unicode => utf16_bytes(value),
            Self::Ansi(page) => {
                let mut bytes = page.encode(value);
                bytes.push(0);
                bytes
            }
        }
    }

    /// `value` written as this text is: a Unicode string, or several, become 8-bit ones in an
    /// ANSI file; anything else is as it was.
    pub(crate) fn narrow(self, value: PropertyValue) -> PropertyValue {
        let Self::Ansi(page) = self else {
            return value;
        };
        let text = |value: &UnicodeValue| {
            let mut text = value.to_string();
            // A multi-valued string's values end in a NUL, as Outlook writes them.
            if text.ends_with('\0') {
                text.pop();
            }
            text
        };
        match value {
            PropertyValue::Unicode(value) => {
                PropertyValue::String8(String8Value::new(page.encode(&text(&value))))
            }
            PropertyValue::MultipleUnicode(values) => PropertyValue::MultipleString8(
                values
                    .iter()
                    .map(|value| {
                        let mut bytes = page.encode(&text(value));
                        bytes.push(0);
                        String8Value::new(bytes)
                    })
                    .collect(),
            ),
            value => value,
        }
    }
}

/// The store of a file the writer adds to, in either format.
pub(crate) struct AddStore {
    store: Rc<dyn Store>,
    kind: StoreKind,
}

enum StoreKind {
    Unicode(Rc<UnicodeStore>),
    Ansi(Rc<AnsiStore>),
}

impl std::ops::Deref for AddStore {
    type Target = dyn Store;

    fn deref(&self) -> &Self::Target {
        self.store.as_ref()
    }
}

impl AddStore {
    /// The store of the file at `path`, read within [`ADD_MAX_VALUE_SIZE`].
    pub(crate) fn open(path: &Path) -> io::Result<Self> {
        let limit = add_max_value_size();
        Ok(match Format::of_file(path)? {
            Format::Unicode => {
                let pst = UnicodePstFile::open(path)?;
                pst.set_max_value_size(limit);
                let store = UnicodeStore::read(Rc::new(pst))?;
                Self {
                    store: store.clone(),
                    kind: StoreKind::Unicode(store),
                }
            }
            Format::Ansi => {
                let pst = AnsiPstFile::open(path)?;
                pst.set_max_value_size(limit);
                let store = AnsiStore::read(Rc::new(pst))?;
                Self {
                    store: store.clone(),
                    kind: StoreKind::Ansi(store),
                }
            }
        })
    }

    /// The table `entry` names.
    pub(crate) fn table(&self, entry: UnicodeNodeBTreeEntry) -> io::Result<Rc<dyn TableContext>> {
        match &self.kind {
            StoreKind::Unicode(store) => UnicodeTableContext::read(store.clone(), entry),
            StoreKind::Ansi(store) => {
                AnsiTableContext::read(store.clone(), ansi_nbt_entry(&entry)?)
            }
        }
    }
}

/// Rebuild the allocation map of the file at `path` (once its header's B-trees are the new
/// ones), its density list too, as the reader's own writer does.
pub(crate) fn finish_metadata(path: &Path, blocks: &[(u64, u16)], pages: &[u64]) -> io::Result<()> {
    let limit = add_max_value_size();
    match Format::of_file(path)? {
        Format::Unicode => {
            let mut pst = UnicodePstFile::open(path)?;
            pst.set_max_value_size(limit);
            pst.inner
                .rebuild_allocation_map_from(Some((blocks, pages)))?;
            pst.inner.ensure_density_list()?;
            pst.inner.finish_write()?;
        }
        Format::Ansi => {
            let mut pst = AnsiPstFile::open(path)?;
            pst.set_max_value_size(limit);
            pst.inner
                .rebuild_allocation_map_from(Some((blocks, pages)))?;
            pst.inner.ensure_density_list()?;
            pst.inner.finish_write()?;
        }
    }
    Ok(())
}

/// How the strings of `message` and its attachments, attached messages and all, are written in a
/// file in `format` ([`Text::for_message`]).
pub(crate) fn message_text(
    format: Format,
    message: &UnicodePstMessage<'_>,
    attachments: &[UnicodePstAttachment<'_>],
) -> Text {
    if format == Format::Unicode {
        return Text::Unicode;
    }
    let mut strings = Vec::new();
    message_strings(message, attachments, &mut strings);
    Text::for_message(format, strings.into_iter())
}

fn message_strings<'a>(
    message: &'a UnicodePstMessage<'a>,
    attachments: &'a [UnicodePstAttachment<'a>],
    out: &mut Vec<&'a str>,
) {
    out.extend([
        message.subject,
        message.sender_name,
        message.sender_email,
        message.body,
        message.message_id,
        message.message_class,
    ]);
    for recipient in message.recipients {
        out.extend([recipient.name, recipient.email]);
    }
    for property in message.properties {
        property_strings(property, out);
    }
    for attachment in attachments {
        out.extend([attachment.filename, attachment.mime_type]);
        out.extend(attachment.content_id);
        for property in attachment.properties {
            property_strings(property, out);
        }
        if let UnicodePstAttachmentContent::Message(inner) = &attachment.content {
            message_strings(&inner.message, &inner.attachments, out);
        }
    }
}

fn property_strings<'a>(property: &UnicodePstProperty<'a>, out: &mut Vec<&'a str>) {
    match property.value {
        UnicodePstValue::Unicode(value) => out.push(value),
        UnicodePstValue::MultipleUnicode(values) => out.extend(values.iter().copied()),
        _ => {}
    }
}
