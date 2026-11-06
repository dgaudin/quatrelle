//! MailCraft: what an attachment holds beyond a file's bytes given whole: an attached message
//! (`afEmbeddedMessage`, MS-OXCMSG 2.2.2.9), kept as MS-PST 2.4.6.2 keeps one, its own
//! recipients and attachments in its own subnode tree; or a file read from a source block by
//! block as it is written, so a big one is never held whole. And tagged properties of the
//! attachment's own (`PidTagAttachmentHidden`, `PidTagAttachmentFlags`…).

use super::*;

/// How deep attached messages go: a message attached to a message attached to the one given is
/// two deep.
pub const MAX_EMBEDDED_DEPTH: usize = 32;

/// The most a file read from a source can hold: its size is `PidTagAttachSize`, a 32-bit signed
/// integer, with the attachment's other properties counted in.
pub(crate) const MAX_SOURCE_SIZE: u64 = i32::MAX as u64 - (1 << 20);

/// The file an attachment holds, read when it is written.
pub trait UnicodePstAttachmentSource {
    /// The file's size: exactly what [`Self::open`] reads.
    fn size(&self) -> u64;
    /// The file's bytes from the start, read once as the file is written.
    fn open(&self) -> io::Result<Box<dyn Read + '_>>;
}

/// What an attachment holds.
#[derive(Default)]
pub enum UnicodePstAttachmentContent<'a> {
    /// The file's bytes, [`UnicodePstAttachment::data`].
    #[default]
    Data,
    /// A file read from a source as it is written; `data` is empty.
    Source(&'a dyn UnicodePstAttachmentSource),
    /// An attached message; `data` is empty and `filename` is its display name.
    Message(Box<UnicodePstEmbeddedMessage<'a>>),
}

/// A message attached to another one, with its own attachments.
pub struct UnicodePstEmbeddedMessage<'a> {
    pub message: UnicodePstMessage<'a>,
    pub attachments: Vec<UnicodePstAttachment<'a>>,
}

/// `PidTagAttachMethod`: `afByValue` and `afEmbeddedMessage`.
pub(crate) const ATTACH_BY_VALUE: i32 = 1;
pub(crate) const ATTACH_EMBEDDED_MESSAGE: i32 = 5;

/// The node IDs attached messages are given, as Outlook gives them: message IDs
/// (`NID_TYPE_NORMAL_MESSAGE`) no other node in the file has, after those of the messages a batch
/// adds; the file's next message ID is moved past them.
pub(crate) struct EmbeddedIds {
    next: u32,
    pub(crate) given: Vec<NodeId>,
}

impl EmbeddedIds {
    /// IDs from `first` on.
    pub(crate) fn new(first: u32) -> Self {
        Self {
            next: first,
            given: Vec::new(),
        }
    }

    /// The next one.
    pub(crate) fn take(&mut self) -> io::Result<NodeId> {
        let node = NodeId::new(NodeIdType::NormalMessage, self.next)?;
        self.next = self
            .next
            .checked_add(1)
            .ok_or(PstError::IntegerConversion)?;
        self.given.push(node);
        Ok(node)
    }
}

/// An attached message's node ID while its size is worked out: any takes the same room.
pub(crate) fn sizing_node() -> io::Result<NodeId> {
    Ok(NodeId::new(NodeIdType::NormalMessage, 1)?)
}

/// Properties of an attachment the writer works out itself: its size, its value and its method.
const ATTACHMENT_OWNED: [u16; 3] = [0x0E20, 0x3701, 0x3705];

/// The attached message, when that is what an attachment holds.
pub(crate) fn embedded<'b>(
    input: &'b UnicodePstAttachment<'_>,
) -> Option<&'b UnicodePstEmbeddedMessage<'b>> {
    match &input.content {
        UnicodePstAttachmentContent::Message(message) => Some(message),
        _ => None,
    }
}

/// The size of the file an attachment holds (0 for an attached message).
pub(crate) fn file_size(input: &UnicodePstAttachment<'_>) -> io::Result<usize> {
    match input.content {
        UnicodePstAttachmentContent::Data => Ok(input.data.len()),
        UnicodePstAttachmentContent::Source(source) => {
            usize::try_from(source.size()).map_err(|_| PstError::IntegerConversion.into())
        }
        UnicodePstAttachmentContent::Message(_) => Ok(0),
    }
}

/// Attachments are an error unless they can be written: bytes given once (in `data` or by a
/// source no bigger than [`MAX_SOURCE_SIZE`]), attached messages no deeper than
/// [`MAX_EMBEDDED_DEPTH`] and each a message [`check_message`] takes, added properties tagged,
/// not the writer's own, given once, each small enough for the attachment's one heap page.
/// `depth`: how deep the message they're attached to is (0 for the one given).
pub(crate) fn check_attachments(
    attachments: &[UnicodePstAttachment<'_>],
    depth: usize,
) -> io::Result<()> {
    let invalid = |message: &str| Err(io::Error::new(io::ErrorKind::InvalidInput, message));
    for attachment in attachments {
        match &attachment.content {
            UnicodePstAttachmentContent::Data => {}
            _ if !attachment.data.is_empty() => {
                return invalid("an attachment's bytes are given twice");
            }
            UnicodePstAttachmentContent::Source(source) => {
                if source.size() > MAX_SOURCE_SIZE {
                    return invalid("an attachment is too big for a data file");
                }
            }
            UnicodePstAttachmentContent::Message(message) => {
                if depth + 1 > MAX_EMBEDDED_DEPTH {
                    return invalid("attached messages are nested too deep");
                }
                check_message(&message.message)?;
                check_attachments(&message.attachments, depth + 1)?;
            }
        }
        let mut tags = HashSet::new();
        for property in attachment.properties {
            let UnicodePstPropertyKey::Tag(id) = property.key else {
                return invalid("an attachment's added properties are tagged ones");
            };
            if id == 0 || id >= 0x8000 {
                return invalid("a tagged property's ID must be from 0x0001 to 0x7FFF");
            }
            if ATTACHMENT_OWNED.contains(&id) {
                return invalid("the writer sets this property itself");
            }
            if !tags.insert(id) {
                return invalid("a property is given twice");
            }
            let value = property.value.property_value();
            if small_property_record(&value).is_none()
                && property_value_bytes(&value)?.len() > MAX_HN_ALLOCATION_SIZE
            {
                return invalid("an attachment's added property is too big");
            }
        }
    }
    Ok(())
}

/// Every message attached to `attachments`, at any depth, in order.
pub(crate) fn embedded_messages<'b>(
    attachments: &'b [UnicodePstAttachment<'b>],
    out: &mut Vec<&'b UnicodePstMessage<'b>>,
) {
    for attachment in attachments {
        if let UnicodePstAttachmentContent::Message(message) = &attachment.content {
            out.push(&message.message);
            embedded_messages(&message.attachments, out);
        }
    }
}

/// An attachment's added properties, as the attachment's property context takes them: in place
/// of the writer's own of the same ID.
pub(crate) fn with_attachment_properties(
    input: &UnicodePstAttachment<'_>,
    properties: &mut Vec<(u16, PropertyType, Option<Vec<u8>>, u32)>,
    text: Text,
) -> io::Result<()> {
    for property in input.properties {
        let UnicodePstPropertyKey::Tag(id) = property.key else {
            continue;
        };
        // MailCraft: a string is 8-bit in an ANSI file.
        let value = text.narrow(property.value.property_value());
        let prop_type = PropertyType::from(&value);
        let record = match small_property_record(&value) {
            Some(small) => (id, prop_type, None, small),
            None => (id, prop_type, Some(property_value_bytes(&value)?), 0),
        };
        properties.retain(|(existing, ..)| *existing != id);
        properties.push(record);
    }
    properties.sort_by_key(|(id, ..)| *id);
    Ok(())
}

/// How many leaf blocks of a file read from a source are written to the file at once.
const LEAVES_PER_WRITE: usize = 256;

/// A file of `size` bytes from `source` as a data tree, its leaves sent on to the file
/// (`flush`) [`LEAVES_PER_WRITE`] at a time, so no more than that is held. The source must give
/// exactly `size` bytes.
pub(crate) fn push_data_tree_source(
    blocks: &mut Vec<NewBlock>,
    next_bid: &mut u64,
    next_offset: &mut FileSpace,
    source: &dyn UnicodePstAttachmentSource,
    size: usize,
    flush: &mut dyn FnMut(&mut Vec<NewBlock>) -> io::Result<()>,
) -> io::Result<UnicodeBlockId> {
    let mut reader = source.open()?;
    // MailCraft: the blocks but the last are as big as the file's format makes a block.
    let block_size = next_offset.format.data_block_size();
    let mut leaves = Vec::with_capacity(size.div_ceil(block_size));
    let mut left = size;
    while left > 0 {
        let mut chunk = vec![0; left.min(block_size)];
        reader.read_exact(&mut chunk).map_err(|err| {
            io::Error::new(
                err.kind(),
                format!("an attachment's source ended before its size: {err}"),
            )
        })?;
        left -= chunk.len();
        let length = u32::try_from(chunk.len()).map_err(|_| PstError::IntegerConversion)?;
        let id = push_block(
            blocks,
            next_bid,
            next_offset,
            false,
            NewBlockData::Data(chunk),
        )?;
        leaves.push((id, length));
        if leaves.len() % LEAVES_PER_WRITE == 0 {
            flush(blocks)?;
        }
    }
    let mut more = [0; 1];
    if reader.read(&mut more)? > 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "an attachment's source holds more than its size",
        ));
    }
    let tree = push_data_tree_leaves(blocks, next_bid, next_offset, leaves)?;
    flush(blocks)?;
    Ok(tree)
}

#[cfg(test)]
thread_local! {
    /// The most bytes of blocks that waited to be written at once, on this thread.
    pub(crate) static MOST_PENDING: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Note how many bytes of blocks wait to be written (a test's measure).
#[cfg(test)]
pub(crate) fn note_pending(blocks: &[NewBlock]) {
    let pending = blocks
        .iter()
        .map(|block| usize::from(block.size))
        .sum::<usize>();
    MOST_PENDING.with(|most| most.set(most.get().max(pending)));
}

#[cfg(test)]
mod tests;
