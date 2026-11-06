//! MailCraft: named properties (MS-PST 2.4.7): each one a message is given is looked up in the
//! file's name-to-ID map (`NID_NAME_TO_ID_MAP`), and added to it when it isn't there, so it has
//! a property ID (0x8000 and up) to be written with.

use super::*;

/// A named property's name in its property set: a number (a LID) or a string.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnicodePstName<'a> {
    Id(u32),
    String(&'a str),
}

/// `PidTagNameidBucketCount`, `PidTagNameidStreamGuid`, `PidTagNameidStreamEntry`,
/// `PidTagNameidStreamString`, and the first hash bucket's ID.
const BUCKET_COUNT: u16 = 0x0001;
const STREAM_GUID: u16 = 0x0002;
const STREAM_ENTRY: u16 = 0x0003;
const STREAM_STRING: u16 = 0x0004;
const FIRST_BUCKET: u16 = 0x1000;

/// A name-to-ID map entry's `wPropIdx` is at most this (NPID 0xFFFE).
const MAX_PROP_INDEX: u16 = 0x7FFE;
/// The longest string name, in UTF-16 code units.
pub(crate) const MAX_NAME: usize = 1024;

/// A GUID as the file keeps it: data1 to data3 little-endian, then data4.
pub(crate) fn guid_bytes(guid: &GuidValue) -> [u8; 16] {
    let mut bytes = [0; 16];
    bytes[0..4].copy_from_slice(&guid.data1().to_le_bytes());
    bytes[4..6].copy_from_slice(&guid.data2().to_le_bytes());
    bytes[6..8].copy_from_slice(&guid.data3().to_le_bytes());
    bytes[8..16].copy_from_slice(guid.data4());
    bytes
}

/// A named property, as the map is searched for it.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub(crate) enum NameKey {
    Id([u8; 16], u32),
    String([u8; 16], Vec<u16>),
}

impl NameKey {
    pub(crate) fn of(set: &GuidValue, name: UnicodePstName<'_>) -> Self {
        match name {
            UnicodePstName::Id(id) => Self::Id(guid_bytes(set), id),
            UnicodePstName::String(name) => {
                Self::String(guid_bytes(set), name.encode_utf16().collect())
            }
        }
    }

    fn set(&self) -> [u8; 16] {
        match self {
            Self::Id(set, _) | Self::String(set, _) => *set,
        }
    }
}

/// The property IDs of the named properties a batch of messages has, and the map's new
/// properties when names were added to it.
#[derive(Default)]
pub(crate) struct NamedIds {
    ids: HashMap<NameKey, u16>,
    pub(crate) new_map: Option<Vec<(u16, PropertyValue)>>,
}

impl NamedIds {
    pub(crate) fn id(&self, key: &NameKey) -> Option<u16> {
        self.ids.get(key).copied()
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn binary(properties: &BTreeMap<u16, PropertyValue>, id: u16) -> io::Result<Vec<u8>> {
    match properties.get(&id) {
        Some(PropertyValue::Binary(value)) => Ok(value.buffer().to_vec()),
        None => Ok(Vec::new()),
        Some(_) => Err(invalid("a name-to-ID map stream isn't binary")),
    }
}

/// The map's string stream entry at `offset`: its UTF-16 code units.
fn string_at(strings: &[u8], offset: u32) -> Option<Vec<u16>> {
    let start = usize::try_from(offset).ok()?;
    let size = strings.get(start..start.checked_add(4)?)?;
    let size = usize::try_from(u32::from_le_bytes(size.try_into().ok()?)).ok()?;
    let from = start.checked_add(4)?;
    let bytes = strings.get(from..from.checked_add(size)?)?;
    Some(
        bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect(),
    )
}

/// `wGuid` for a property set: 1 for PS_MAPI, 2 for PS_PUBLIC_STRINGS, else 3 and up for its
/// place in the GUID stream (added to it when it isn't there).
fn guid_index(set: [u8; 16], guids: &mut Vec<u8>) -> io::Result<u16> {
    if set == guid_bytes(&PS_MAPI) {
        return Ok(1);
    }
    if set == guid_bytes(&PS_PUBLIC_STRINGS) {
        return Ok(2);
    }
    let found = guids.chunks_exact(16).position(|guid| guid == set);
    let index = match found {
        Some(index) => index,
        None => {
            let index = guids.len() / 16;
            guids.extend_from_slice(&set);
            index
        }
    };
    u16::try_from(index)
        .ok()
        .and_then(|index| index.checked_add(3))
        .filter(|index| *index < 0x8000)
        .ok_or_else(|| invalid("the name-to-ID map has too many property sets"))
}

/// The ID of each named property `inputs` give, looked up in the store's name-to-ID map; those
/// it doesn't have are added to a new copy of its properties (`new_map`). The map's streams are
/// read as the reader reads them, within the file's limit.
pub(crate) fn resolve(
    store: &dyn Store,
    inputs: &[UnicodePstBatchMessage<'_>],
) -> io::Result<NamedIds> {
    let mut wanted = Vec::new();
    for input in inputs {
        // The messages attached to it too, at any depth.
        let mut messages = vec![&input.message];
        embedded_messages(input.attachments, &mut messages);
        for property in messages.iter().flat_map(|message| message.properties) {
            if let UnicodePstPropertyKey::Named(set, name) = property.key {
                let key = NameKey::of(&set, name);
                if !wanted.contains(&key) {
                    wanted.push(key);
                }
            }
        }
    }
    let mut named = NamedIds::default();
    if wanted.is_empty() {
        return Ok(named);
    }

    let map = store.named_property_map()?;
    let mut properties: BTreeMap<u16, PropertyValue> = map
        .properties()
        .iter()
        .map(|(id, value)| (*id, value.clone()))
        .collect();
    let buckets = match properties.get(&BUCKET_COUNT) {
        Some(PropertyValue::Integer32(count)) => u16::try_from(*count)
            .ok()
            .filter(|count| *count > 0 && *count <= u16::MAX - FIRST_BUCKET)
            .ok_or_else(|| invalid("the name-to-ID map's bucket count is out of range"))?,
        _ => return Err(invalid("the name-to-ID map has no bucket count")),
    };
    let mut guids = binary(&properties, STREAM_GUID)?;
    let mut entries = binary(&properties, STREAM_ENTRY)?;
    let mut strings = binary(&properties, STREAM_STRING)?;

    // The names the map has: each entry is dwPropertyID, then (wGuid << 1) | N, then wPropIdx.
    let mut next_index = 0_u16;
    for entry in entries.chunks_exact(8) {
        let id = u32::from_le_bytes([entry[0], entry[1], entry[2], entry[3]]);
        let guid = u16::from_le_bytes([entry[4], entry[5]]);
        let index = u16::from_le_bytes([entry[6], entry[7]]);
        next_index = next_index.max(index.saturating_add(1));
        let set = match guid >> 1 {
            1 => guid_bytes(&PS_MAPI),
            2 => guid_bytes(&PS_PUBLIC_STRINGS),
            0 => continue,
            n => {
                let at = usize::from(n - 3) * 16;
                match guids.get(at..at + 16).and_then(|set| set.try_into().ok()) {
                    Some(set) => set,
                    None => continue,
                }
            }
        };
        let key = if guid & 1 == 0 {
            NameKey::Id(set, id)
        } else {
            match string_at(&strings, id) {
                Some(name) => NameKey::String(set, name),
                None => continue,
            }
        };
        if index <= MAX_PROP_INDEX && wanted.contains(&key) {
            named.ids.entry(key).or_insert(0x8000 + index);
        }
    }
    let count = u16::try_from(entries.len() / 8).unwrap_or(u16::MAX);
    next_index = next_index.max(count);

    let mut added = false;
    let mut buckets_added: BTreeMap<u16, Vec<u8>> = BTreeMap::new();
    for key in wanted {
        if named.ids.contains_key(&key) {
            continue;
        }
        if next_index > MAX_PROP_INDEX {
            return Err(invalid("the name-to-ID map is full"));
        }
        let guid = guid_index(key.set(), &mut guids)?;
        let (id, kind, hash) = match &key {
            NameKey::Id(_, id) => (*id, 0, *id),
            NameKey::String(_, name) => {
                // Each string: its size in bytes, the UTF-16 code units, then padding to 4 bytes.
                let offset =
                    u32::try_from(strings.len()).map_err(|_| PstError::IntegerConversion)?;
                let bytes: Vec<u8> = name.iter().flat_map(|unit| unit.to_le_bytes()).collect();
                let size = u32::try_from(bytes.len()).map_err(|_| PstError::IntegerConversion)?;
                strings.extend_from_slice(&size.to_le_bytes());
                strings.extend_from_slice(&bytes);
                strings.resize(strings.len().next_multiple_of(4), 0);
                (offset, 1, crate::crc::compute_crc(0, &bytes))
            }
        };
        let guid_field = (guid << 1) | kind;
        entries.extend_from_slice(&id.to_le_bytes());
        entries.extend_from_slice(&guid_field.to_le_bytes());
        entries.extend_from_slice(&next_index.to_le_bytes());
        // A hash bucket's entry has the string's CRC in place of its offset (MS-PST 2.4.7.5).
        let bucket = (hash ^ u32::from(guid_field)) % u32::from(buckets);
        let bucket =
            FIRST_BUCKET + u16::try_from(bucket).map_err(|_| PstError::IntegerConversion)?;
        let record = buckets_added
            .entry(bucket)
            .or_insert_with(|| binary(&properties, bucket).unwrap_or_default());
        record.extend_from_slice(&hash.to_le_bytes());
        record.extend_from_slice(&guid_field.to_le_bytes());
        record.extend_from_slice(&next_index.to_le_bytes());
        named.ids.insert(key, 0x8000 + next_index);
        next_index += 1;
        added = true;
    }
    if added {
        for (id, value) in [
            (STREAM_GUID, guids),
            (STREAM_ENTRY, entries),
            (STREAM_STRING, strings),
        ]
        .into_iter()
        .chain(buckets_added)
        {
            properties.insert(id, PropertyValue::Binary(BinaryValue::new(value)));
        }
        named.new_map = Some(properties.into_iter().collect());
    }
    Ok(named)
}

/// Write the name-to-ID map's new properties: its node gets a new property context, with the
/// values too big for its heap in subnodes of their own (the map has no others).
#[allow(clippy::too_many_arguments)]
pub(crate) fn push_map(
    properties: Vec<(u16, PropertyValue)>,
    nbt_entries: &mut Vec<UnicodeNodeBTreeEntry>,
    blocks: &mut Vec<NewBlock>,
    next_bid: &mut u64,
    next_offset: &mut FileSpace,
) -> io::Result<()> {
    let entry = nbt_entries
        .iter()
        .find(|entry| entry.node() == NID_NAME_TO_ID_MAP)
        .copied()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "name-to-ID map not found"))?;
    // Its streams are binary, in an ANSI file too (MS-PST 2.4.7).
    let (pages, external) = property_context_pages(properties, 1, next_offset.format.text())?;
    let data = push_data_tree_chunks(blocks, next_bid, next_offset, pages)?;
    let mut subnodes = Vec::with_capacity(external.len());
    for (node, value) in external {
        let value = push_data_tree(blocks, next_bid, next_offset, &value)?;
        subnodes.push(UnicodeLeafSubNodeTreeEntry::new(node, value, None));
    }
    let subnode = if subnodes.is_empty() {
        None
    } else {
        Some(push_subnode_tree(blocks, next_bid, next_offset, subnodes)?)
    };
    nbt_entries.retain(|candidate| candidate.node() != NID_NAME_TO_ID_MAP);
    nbt_entries.push(UnicodeNodeBTreeEntry::new(
        NID_NAME_TO_ID_MAP,
        data,
        subnode,
        entry.parent(),
    ));
    Ok(())
}

#[cfg(test)]
mod tests;
