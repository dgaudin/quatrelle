//! ## [BTree-on-Heap (BTH)](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-pst/2dd1a95a-c8b1-4ac5-87d1-10cb8de64053)

use byteorder::{ReadBytesExt, WriteBytesExt};
use core::mem;
use std::{
    io::{self, Cursor, Read, Write},
    marker::PhantomData,
};

use super::{heap::*, read_write::*, *};
use crate::{AnsiPstFile, PstFile, UnicodePstFile};

/// [BTHHEADER](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-pst/8e4ae05c-3c24-4103-b7e5-ffef6f244834)
#[derive(Clone, Copy, Debug)]
pub struct HeapTreeHeader {
    key_size: u8,
    entry_size: u8,
    levels: u8,
    root: HeapId,
}

impl HeapTreeHeader {
    pub fn new(key_size: u8, entry_size: u8, levels: u8, root: HeapId) -> LtpResult<Self> {
        match key_size {
            2 | 4 | 8 | 16 => {}
            invalid => {
                return Err(LtpError::InvalidHeapTreeKeySize(invalid));
            }
        }

        match entry_size {
            1..=32 => {}
            invalid => {
                return Err(LtpError::InvalidHeapTreeDataSize(invalid));
            }
        }

        Ok(Self {
            key_size,
            entry_size,
            levels,
            root,
        })
    }

    pub fn key_size(&self) -> u8 {
        self.key_size
    }

    pub fn entry_size(&self) -> u8 {
        self.entry_size
    }

    pub fn levels(&self) -> u8 {
        self.levels
    }

    pub fn root(&self) -> HeapId {
        self.root
    }
}

pub trait HeapTreeEntryKey: Copy + Sized + PartialEq + PartialOrd {
    const SIZE: u8;
}

pub trait HeapTreeEntryValue: Copy + Sized {
    const SIZE: u8;
}

impl HeapNodePageReadWrite for HeapTreeHeader {
    fn read(f: &mut dyn Read) -> io::Result<Self> {
        let heap_type = HeapNodeType::try_from(f.read_u8()?)?;
        if heap_type != HeapNodeType::Tree {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                LtpError::InvalidHeapTreeNodeType(heap_type),
            ));
        }

        let key_size = f.read_u8()?;
        let entry_size = f.read_u8()?;
        let levels = f.read_u8()?;
        let root = HeapId::read(f)?;

        Ok(Self::new(key_size, entry_size, levels, root)?)
    }

    fn write(&self, f: &mut dyn Write) -> io::Result<()> {
        f.write_u8(HeapNodeType::Tree as u8)?;
        f.write_u8(self.key_size)?;
        f.write_u8(self.entry_size)?;
        f.write_u8(self.levels)?;
        self.root.write(f)
    }
}

/// [Intermediate BTH (Index) Records](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-pst/2c992ac1-1b21-4167-b111-f76cf609005f)
#[derive(Clone, Copy, Debug)]
pub struct HeapTreeIntermediateEntry<K>
where
    K: HeapTreeEntryKey,
{
    key: K,
    next_level: HeapId,
}

impl<K> HeapTreeIntermediateEntry<K>
where
    K: HeapTreeEntryKey,
{
    pub fn new(key: K, next_level: HeapId) -> Self {
        Self { key, next_level }
    }

    pub fn key(&self) -> K {
        self.key
    }

    pub fn next_level(&self) -> HeapId {
        self.next_level
    }
}

impl<K> HeapNodePageReadWrite for HeapTreeIntermediateEntry<K>
where
    K: HeapNodePageReadWrite + HeapTreeEntryKey,
{
    fn read(f: &mut dyn Read) -> io::Result<Self> {
        let key = K::read(f)?;
        let next_level = HeapId::read(f)?;

        Ok(Self::new(key, next_level))
    }

    fn write(&self, f: &mut dyn Write) -> io::Result<()> {
        self.key.write(f)?;
        self.next_level.write(f)
    }
}

/// [Leaf BTH (Data) Records](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-pst/660db569-c8f7-4516-82ad-44709b1c667f)
#[derive(Clone, Copy, Debug)]
pub struct HeapTreeLeafEntry<K, V>
where
    K: HeapTreeEntryKey,
    V: Copy + Sized,
{
    key: K,
    data: V,
}

impl<K, V> HeapTreeLeafEntry<K, V>
where
    K: HeapTreeEntryKey,
    V: Copy + Sized,
{
    pub fn new(key: K, data: V) -> Self {
        Self { key, data }
    }

    pub fn key(&self) -> K {
        self.key
    }

    pub fn data(&self) -> V {
        self.data
    }
}

impl<K, V> HeapNodePageReadWrite for HeapTreeLeafEntry<K, V>
where
    K: HeapNodePageReadWrite + HeapTreeEntryKey,
    V: HeapNodePageReadWrite + Copy,
{
    fn read(f: &mut dyn Read) -> io::Result<Self> {
        let key = K::read(f)?;
        let data = V::read(f)?;

        Ok(Self::new(key, data))
    }

    fn write(&self, f: &mut dyn Write) -> io::Result<()> {
        self.key.write(f)?;
        self.data.write(f)
    }
}

pub trait HeapTree {
    type Key: HeapTreeEntryKey;
    type Value: HeapTreeEntryValue;

    fn heap(&self) -> &dyn HeapNode;
    fn user_root(&self) -> HeapId;
    fn header(&self) -> io::Result<HeapTreeHeader>;
    fn entries(&self) -> io::Result<Vec<HeapTreeLeafEntry<Self::Key, Self::Value>>>;
}

struct HeapTreeInner<Pst, K, V>
where
    Pst: PstFile,
    K: HeapTreeEntryKey + HeapNodePageReadWrite,
    V: HeapTreeEntryValue + HeapNodePageReadWrite,
{
    heap: <Pst as PstFile>::HeapNode,
    user_root: HeapId,
    _phantom: PhantomData<(K, V)>,
}

impl<Pst, K, V> HeapTreeInner<Pst, K, V>
where
    Pst: PstFile,
    K: HeapTreeEntryKey + HeapNodePageReadWrite,
    V: HeapTreeEntryValue + HeapNodePageReadWrite,
{
    fn new(heap: <Pst as PstFile>::HeapNode, user_root: HeapId) -> Self {
        Self {
            heap,
            user_root,
            _phantom: PhantomData,
        }
    }

    fn header(&self) -> io::Result<HeapTreeHeader> {
        let mut cursor = Cursor::new(self.heap.find_entry(self.user_root)?);
        HeapTreeHeader::read(&mut cursor)
    }

    fn entries(&self) -> io::Result<Vec<HeapTreeLeafEntry<K, V>>> {
        let header = self.header()?;
        if header.key_size() != K::SIZE {
            return Err(LtpError::InvalidHeapTreeKeySize(header.key_size()).into());
        }
        if header.entry_size() != V::SIZE {
            return Err(LtpError::InvalidHeapTreeDataSize(header.entry_size()).into());
        }

        if u32::from(header.root()) == 0 {
            return Ok(Default::default());
        }

        let leaves = leaves::<K>(header.root(), header.levels(), |heap_id| {
            self.heap.find_entry(heap_id)
        })?;

        let mut results = Vec::new();
        for heap_id in leaves {
            let records = leaf_records::<K, V>(self.heap.find_entry(heap_id)?);
            if results.len().saturating_add(records.len()) > max_records::<K>() {
                return Err(LtpError::HeapTreeTooLarge(max_records::<K>()).into());
            }
            results.extend(records);
        }

        Ok(results)
    }
}

/// MailCraft: the leaves under a BTH's `levels` index levels. Index records that point back up
/// the tree multiply at every level, and the header can declare up to 255 levels, so every index
/// record read, at any level, counts against one budget: a real tree never comes near it, and a
/// damaged one can't keep the walk going level after level.
fn leaves<'a, K>(
    root: HeapId,
    levels: u8,
    find: impl Fn(HeapId) -> io::Result<&'a [u8]>,
) -> io::Result<Vec<HeapId>>
where
    K: HeapTreeEntryKey + HeapNodePageReadWrite,
{
    let mut next_level = vec![root];
    let mut read = 0_usize;
    for _ in 0..levels {
        for heap_id in mem::take(&mut next_level) {
            let mut cursor = Cursor::new(find(heap_id)?);
            while let Ok(row) = HeapTreeIntermediateEntry::<K>::read(&mut cursor) {
                if read >= max_records::<K>() {
                    return Err(LtpError::HeapTreeTooLarge(max_records::<K>()).into());
                }
                read += 1;
                next_level.push(row.next_level());
            }
        }
    }
    Ok(next_level)
}

/// MailCraft: the most records read from one BTH, at any level.
const MAX_HEAP_TREE_RECORDS: usize = 1 << 24;

/// MailCraft: the most records a BTH keyed by `K` can hold: one per key (a property context's
/// u16 ids stop at 65,536), and never more than `MAX_HEAP_TREE_RECORDS`.
fn max_records<K: HeapTreeEntryKey>() -> usize {
    1_usize
        .checked_shl(8 * u32::from(K::SIZE))
        .map_or(MAX_HEAP_TREE_RECORDS, |keys| keys.min(MAX_HEAP_TREE_RECORDS))
}

/// MailCraft: the records of one leaf, each read from its own fixed-size slot. A record that
/// doesn't parse (a property type this crate doesn't know) is skipped: it used to end the scan,
/// so every record after it was lost.
fn leaf_records<K, V>(data: &[u8]) -> Vec<HeapTreeLeafEntry<K, V>>
where
    K: HeapTreeEntryKey + HeapNodePageReadWrite,
    V: HeapTreeEntryValue + HeapNodePageReadWrite,
{
    let size = usize::from(K::SIZE) + usize::from(V::SIZE);
    data.chunks_exact(size.max(1))
        .filter_map(|record| {
            match HeapTreeLeafEntry::<K, V>::read(&mut Cursor::new(record)) {
                Ok(row) => Some(row),
                Err(err) => {
                    tracing::warn!(
                        name: "PstSkippedHeapTreeRecord",
                        ?err,
                        "A BTH record that could not be read was skipped"
                    );
                    None
                }
            }
        })
        .collect()
}

pub struct UnicodeHeapTree<K, V>
where
    K: HeapTreeEntryKey + HeapNodePageReadWrite,
    V: HeapTreeEntryValue + HeapNodePageReadWrite,
{
    inner: HeapTreeInner<UnicodePstFile, K, V>,
}

impl<K, V> HeapTree for UnicodeHeapTree<K, V>
where
    K: HeapTreeEntryKey + HeapNodePageReadWrite,
    V: HeapTreeEntryValue + HeapNodePageReadWrite,
{
    type Key = K;
    type Value = V;

    fn heap(&self) -> &dyn HeapNode {
        &self.inner.heap
    }

    fn user_root(&self) -> HeapId {
        self.inner.user_root
    }

    fn header(&self) -> io::Result<HeapTreeHeader> {
        self.inner.header()
    }

    fn entries(&self) -> io::Result<Vec<HeapTreeLeafEntry<K, V>>> {
        self.inner.entries()
    }
}

impl<K, V> HeapTreeReadWrite<UnicodePstFile> for UnicodeHeapTree<K, V>
where
    K: HeapTreeEntryKey + HeapNodePageReadWrite,
    V: HeapTreeEntryValue + HeapNodePageReadWrite,
{
    fn new(heap: UnicodeHeapNode, user_root: HeapId) -> Self {
        let inner = HeapTreeInner::new(heap, user_root);
        Self { inner }
    }
}

impl<K, V> From<UnicodeHeapTree<K, V>> for UnicodeHeapNode
where
    K: HeapTreeEntryKey + HeapNodePageReadWrite,
    V: HeapTreeEntryValue + HeapNodePageReadWrite,
{
    fn from(value: UnicodeHeapTree<K, V>) -> Self {
        value.inner.heap
    }
}

pub struct AnsiHeapTree<K, V>
where
    K: HeapTreeEntryKey + HeapNodePageReadWrite,
    V: HeapTreeEntryValue + HeapNodePageReadWrite,
{
    inner: HeapTreeInner<AnsiPstFile, K, V>,
}

impl<K, V> HeapTree for AnsiHeapTree<K, V>
where
    K: HeapTreeEntryKey + HeapNodePageReadWrite,
    V: HeapTreeEntryValue + HeapNodePageReadWrite,
{
    type Key = K;
    type Value = V;

    fn heap(&self) -> &dyn HeapNode {
        &self.inner.heap
    }

    fn user_root(&self) -> HeapId {
        self.inner.user_root
    }

    fn header(&self) -> io::Result<HeapTreeHeader> {
        self.inner.header()
    }

    fn entries(&self) -> io::Result<Vec<HeapTreeLeafEntry<K, V>>> {
        self.inner.entries()
    }
}

impl<K, V> HeapTreeReadWrite<AnsiPstFile> for AnsiHeapTree<K, V>
where
    K: HeapTreeEntryKey + HeapNodePageReadWrite,
    V: HeapTreeEntryValue + HeapNodePageReadWrite,
{
    fn new(heap: AnsiHeapNode, user_root: HeapId) -> Self {
        let inner = HeapTreeInner::new(heap, user_root);
        Self { inner }
    }
}

impl<K, V> From<AnsiHeapTree<K, V>> for AnsiHeapNode
where
    K: HeapTreeEntryKey + HeapNodePageReadWrite,
    V: HeapTreeEntryValue + HeapNodePageReadWrite,
{
    fn from(value: AnsiHeapTree<K, V>) -> Self {
        value.inner.heap
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ltp::prop_context::{
        PropertyTreeRecordKey, PropertyTreeRecordValue, PropertyValueRecord,
    };
    use crate::ltp::prop_type::PropertyType;

    fn record(key: u16, prop_type: u16, value: u32) -> Vec<u8> {
        [&key.to_le_bytes()[..], &prop_type.to_le_bytes(), &value.to_le_bytes()].concat()
    }

    /// MailCraft: an attachment's properties, sorted by id as they are kept, read past a record
    /// of a type no one knows and past `PtypObject`, which used to end the scan after 0x3001.
    #[test]
    fn a_record_that_does_not_parse_is_skipped() {
        let data = [
            record(0x0E20, 0x0003, 5),
            record(0x3001, 0x0099, 0),
            record(0x3701, 0x000D, 0x20),
            record(0x3705, 0x0003, 5),
            vec![0x0B, 0x37, 0x03], // a partial record at the end
        ]
        .concat();
        let rows = leaf_records::<PropertyTreeRecordKey, PropertyTreeRecordValue>(&data);
        let keys: Vec<u16> = rows.iter().map(|r| r.key()).collect();
        assert_eq!(keys, [0x0E20, 0x3701, 0x3705]);
        let object = rows[1].data();
        assert_eq!(object.prop_type(), PropertyType::Object);
        assert!(matches!(object.value(), PropertyValueRecord::Heap(h) if u32::from(h) == 0x20));
    }

    /// MailCraft: a property context's records stop at one per u16 id, so index records that
    /// point back at one leaf end there, not at 2^24; a table's row index can be larger.
    #[test]
    fn a_tree_holds_no_more_records_than_it_has_keys() {
        assert_eq!(max_records::<PropertyTreeRecordKey>(), 1 << 16);
        assert_eq!(max_records::<crate::ltp::table_context::TableRowId>(), MAX_HEAP_TREE_RECORDS);
    }

    /// An index record: a u16 key, then the HID of the level below.
    fn index_record(key: u16, hid: u32) -> Vec<u8> {
        [&key.to_le_bytes()[..], &hid.to_le_bytes()].concat()
    }

    /// MailCraft: 255 index levels whose records never multiply (300 at every level, each
    /// naming an allocation whose one record names itself) once read 300 records a level for
    /// every level the header declared; they now count against one budget for the whole walk.
    #[test]
    fn index_levels_that_point_back_at_themselves_stop_at_one_budget() {
        let (root, looping, leaf) = (0x20_u32, 0x40_u32, 0x60_u32);
        let wide: Vec<u8> = (0..300).flat_map(|k| index_record(k, looping)).collect();
        let back = index_record(0, looping);
        let find = |hid: HeapId| -> io::Result<&[u8]> {
            match u32::from(hid) {
                0x20 => Ok(&wide),
                0x40 => Ok(&back),
                _ => Ok(&[]),
            }
        };
        let walk = leaves::<PropertyTreeRecordKey>(HeapId::from(root), 255, find);
        assert!(walk.is_err_and(|e| e.to_string().contains("more than 65536")));
        // A real two-level tree: the root's records name the leaves.
        let two = [index_record(0, leaf), index_record(9, leaf)].concat();
        let find = |_: HeapId| -> io::Result<&[u8]> { Ok(&two) };
        let walk = leaves::<PropertyTreeRecordKey>(HeapId::from(root), 1, find).unwrap();
        assert_eq!(walk.iter().map(|h| u32::from(*h)).collect::<Vec<_>>(), [leaf, leaf]);
    }
}
