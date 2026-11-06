//! ## [Attachment Objects](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-pst/46eb4828-c6a5-420d-a137-9ee36df317c1)

use std::{collections::BTreeMap, io, rc::Rc};

use super::{message::*, read_write::*, *};
use crate::{
    ltp::{
        heap::HeapNode,
        prop_context::{BinaryValue, PropertyContext, PropertyValue},
        prop_type::PropertyType,
        read_write::*,
    },
    ndb::{
        block::{DataTree, IntermediateTreeBlock, LeafSubNodeTreeEntry, SubNodeTree},
        block_id::BlockId,
        header::Header,
        node_id::{NodeId, NodeIdType},
        page::{BTreePage, NodeBTreeEntry, RootBTree},
        read_write::*,
        root::Root,
    },
    AnsiPstFile, PstFile, PstFileLock, PstReader, UnicodePstFile,
};

#[derive(Default, Debug)]
pub struct AttachmentProperties {
    properties: BTreeMap<u16, PropertyValue>,
}

impl AttachmentProperties {
    pub fn get(&self, id: u16) -> Option<&PropertyValue> {
        self.properties.get(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&u16, &PropertyValue)> {
        self.properties.iter()
    }

    pub fn attachment_size(&self) -> io::Result<i32> {
        let attachment_size = self
            .properties
            .get(&0x0E20)
            .ok_or(MessagingError::AttachmentSizeNotFound)?;

        match attachment_size {
            PropertyValue::Integer32(value) => Ok(*value),
            invalid => {
                Err(MessagingError::InvalidAttachmentSize(PropertyType::from(invalid)).into())
            }
        }
    }

    pub fn attachment_method(&self) -> io::Result<i32> {
        let attachment_method = self
            .properties
            .get(&0x3705)
            .ok_or(MessagingError::AttachmentMethodNotFound)?;

        match attachment_method {
            PropertyValue::Integer32(value) => Ok(*value),
            invalid => {
                Err(MessagingError::InvalidAttachmentMethod(PropertyType::from(invalid)).into())
            }
        }
    }

    pub fn rendering_position(&self) -> io::Result<i32> {
        let rendering_position = self
            .properties
            .get(&0x370B)
            .ok_or(MessagingError::AttachmentRenderingPositionNotFound)?;

        match rendering_position {
            PropertyValue::Integer32(value) => Ok(*value),
            invalid => Err(
                MessagingError::InvalidAttachmentRenderingPosition(PropertyType::from(invalid))
                    .into(),
            ),
        }
    }
}

/// [PidTagAttachMethod](https://learn.microsoft.com/en-us/openspecs/exchange_server_protocols/ms-oxcmsg/252923d6-dd41-468b-9c57-d3f68051a516)
#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum AttachmentMethod {
    /// `afNone`: The attachment has just been created.
    #[default]
    None = 0x00000000,
    /// `afByValue`: The `PidTagAttachDataBinary` property (section [2.2.2.7](https://learn.microsoft.com/en-us/openspecs/exchange_server_protocols/ms-oxcmsg/42dfb62b-2ff5-4ffc-ae25-bfdd2db3d8e0))
    /// contains the attachment data.
    ByValue = 0x00000001,
    /// `afByReference`: The `PidTagAttachLongPathname` property (section [2.2.2.13](https://learn.microsoft.com/en-us/openspecs/exchange_server_protocols/ms-oxcmsg/74b1b39e-1cb4-48ad-b28e-405a261e556c))
    /// contains a fully qualified path identifying the attachment To recipients with access to a
    /// common file server.
    ByReference = 0x00000002,
    /// `afByReferenceOnly`: The `PidTagAttachLongPathname` property contains a fully qualified
    /// path identifying the attachment.
    ByReferenceOnly = 0x00000004,
    /// `afEmbeddedMessage`: The attachment is an embedded message that is accessed via the `RopOpenEmbeddedMessage` ROP ([MS-OXCROPS](https://learn.microsoft.com/en-us/openspecs/exchange_server_protocols/ms-oxcrops/13af6911-27e5-4aa0-bb75-637b02d4f2ef)
    /// section [2.2.6.16](https://learn.microsoft.com/en-us/openspecs/exchange_server_protocols/ms-oxcrops/bce79473-e082-4452-822c-ab8cb055dee6)).
    EmbeddedMessage = 0x00000005,
    /// `afStorage`: The `PidTagAttachDataObject` property (section [2.2.2.8](https://learn.microsoft.com/en-us/openspecs/exchange_server_protocols/ms-oxcmsg/0691206f-0082-463a-a12f-58cb7cb7875f))
    /// contains data in an application-specific format.
    Storage = 0x00000006,
    /// `afByWebReference`: The `PidTagAttachLongPathname` property contains a fully qualified path
    /// identifying the attachment. The `PidNameAttachmentProviderType` defines the web service API
    /// manipulating the attachment.
    ByWebReference = 0x00000007,
}

impl TryFrom<i32> for AttachmentMethod {
    type Error = MessagingError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0x00000000 => Ok(Self::None),
            0x00000001 => Ok(Self::ByValue),
            0x00000002 => Ok(Self::ByReference),
            0x00000004 => Ok(Self::ByReferenceOnly),
            0x00000005 => Ok(Self::EmbeddedMessage),
            0x00000006 => Ok(Self::Storage),
            0x00000007 => Ok(Self::ByWebReference),
            _ => Err(MessagingError::UnknownAttachmentMethod(value)),
        }
    }
}

pub enum AttachmentData {
    Binary(BinaryValue),
    Message(Rc<dyn Message>),
}

pub trait Attachment {
    fn message(&self) -> Rc<dyn Message>;
    fn properties(&self) -> &AttachmentProperties;
    fn data(&self) -> Option<&AttachmentData>;
}

struct AttachmentInner<Pst>
where
    Pst: PstFile,
{
    message: Rc<Pst::Message>,
    properties: AttachmentProperties,
    data: Option<AttachmentData>,
    /// MailCraft: the attached message as the file's own type, so its attachments can be read
    /// in turn (`data` holds the same message as `dyn Message`).
    embedded: Option<Rc<Pst::Message>>,
}

impl<Pst> AttachmentInner<Pst>
where
    Pst: PstFile + PstFileLock<Pst>,
    <Pst as PstFile>::BTreeKey: BTreePageKeyReadWrite,
    <Pst as PstFile>::NodeBTreeEntry: NodeBTreeEntryReadWrite,
    <Pst as PstFile>::NodeBTree: RootBTreeReadWrite,
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
    <Pst as PstFile>::BlockBTree: RootBTreeReadWrite,
    <<Pst as PstFile>::BlockBTree as RootBTree>::Entry: BTreeEntryReadWrite,
    <<Pst as PstFile>::BlockBTree as RootBTree>::IntermediatePage:
        RootBTreeIntermediatePageReadWrite<
            Pst,
            <<Pst as PstFile>::BlockBTree as RootBTree>::Entry,
            <<Pst as PstFile>::BlockBTree as RootBTree>::LeafPage,
        >,
    <<Pst as PstFile>::BlockBTree as RootBTree>::LeafPage:
        RootBTreeLeafPageReadWrite<Pst> + BTreePageReadWrite,
    <Pst as PstFile>::BlockTrailer: BlockTrailerReadWrite,
    <Pst as PstFile>::DataTreeBlock: IntermediateTreeBlockReadWrite,
    <<Pst as PstFile>::DataTreeBlock as IntermediateTreeBlock>::Entry:
        IntermediateTreeEntryReadWrite,
    <Pst as PstFile>::DataBlock: BlockReadWrite + Clone,
    <Pst as PstFile>::SubNodeTreeBlockHeader: IntermediateTreeHeaderReadWrite,
    <Pst as PstFile>::SubNodeTreeBlock: IntermediateTreeBlockReadWrite,
    <<Pst as PstFile>::SubNodeTreeBlock as IntermediateTreeBlock>::Entry:
        IntermediateTreeEntryReadWrite,
    <Pst as PstFile>::SubNodeBlock: IntermediateTreeBlockReadWrite,
    <<Pst as PstFile>::SubNodeBlock as IntermediateTreeBlock>::Entry:
        IntermediateTreeEntryReadWrite,
    <Pst as PstFile>::HeapNode: HeapNodeReadWrite<Pst>,
    <Pst as PstFile>::PropertyTree: HeapTreeReadWrite<Pst>,
    <Pst as PstFile>::PropertyContext: PropertyContextReadWrite<Pst>,
    <Pst as PstFile>::Store: StoreReadWrite<Pst>,
    <Pst as PstFile>::Message: MessageReadWrite<Pst> + 'static,
{
    fn read(
        message: Rc<<Pst as PstFile>::Message>,
        sub_node: NodeId,
        prop_ids: Option<&[u16]>,
    ) -> io::Result<Self> {
        let node_id_type = sub_node.id_type()?;
        match node_id_type {
            NodeIdType::Attachment => {}
            _ => {
                return Err(MessagingError::InvalidAttachmentNodeIdType(node_id_type).into());
            }
        }

        let store = message.pst_store();
        let pst = store.pst();
        let header = pst.header();
        let root = header.root();
        let limit = pst.max_value_size();

        let (properties, data, embedded_node) = {
            let mut file = pst
                .reader()
                .lock()
                .map_err(|_| MessagingError::FailedToLockFile)?;
            let file = &mut *file;

            let encoding = header.crypt_method();
            let block_btree = <<Pst as PstFile>::BlockBTree as RootBTreeReadWrite>::read(
                file,
                *root.block_btree(),
            )?;

            let node = message
                .sub_nodes()
                .get(&sub_node)
                .ok_or(MessagingError::AttachmentSubNodeNotFound(sub_node))?;
            let node = <<Pst as PstFile>::NodeBTreeEntry as NodeBTreeEntryReadWrite>::new(
                node.node(),
                node.block(),
                node.sub_node(),
                None,
            );
            // MailCraft: the attachment's own subnode tree, where MS-PST keeps the object it
            // holds (an attached message, or OLE storage).
            let own_sub_nodes = node.sub_node();

            let mut page_cache = pst.block_cache();
            let data = node.data();
            let heap = <<Pst as PstFile>::HeapNode as HeapNodeReadWrite<Pst>>::read(
                file,
                &block_btree,
                &mut page_cache,
                encoding,
                data.search_key(),
            )?;
            let header = heap.header()?;

            let tree = <Pst as PstFile>::PropertyTree::new(heap, header.user_root());
            let prop_context = <<Pst as PstFile>::PropertyContext as PropertyContextReadWrite<
                Pst,
            >>::new(node, tree);
            let properties = prop_context
                .properties()?
                .into_iter()
                .map(|(prop_id, record)| {
                    // MailCraft: under the file's limit.
                    prop_context
                        .read_property_capped(
                            file,
                            encoding,
                            &block_btree,
                            &mut page_cache,
                            record,
                            limit,
                        )
                        .map(|value| (prop_id, value))
                })
                .collect::<io::Result<BTreeMap<_, _>>>()?;
            let properties = AttachmentProperties { properties };

            let attachment_method = AttachmentMethod::try_from(properties.attachment_method()?)?;
            let mut embedded_node = None;
            let data = match attachment_method {
                AttachmentMethod::ByValue => {
                    let binary_data = match properties
                        .get(0x3701)
                        .ok_or(MessagingError::AttachmentMessageObjectDataNotFound)?
                    {
                        PropertyValue::Binary(value) => value.clone(),
                        // MailCraft: a zero-byte file is kept with no value at all (HNID 0).
                        PropertyValue::Null => BinaryValue::new(Vec::new()),
                        invalid => {
                            return Err(MessagingError::InvalidMessageObjectData(
                                PropertyType::from(invalid),
                            )
                            .into())
                        }
                    };
                    Some(AttachmentData::Binary(binary_data))
                }
                AttachmentMethod::EmbeddedMessage => {
                    let object_data = match properties
                        .get(0x3701)
                        .ok_or(MessagingError::AttachmentMessageObjectDataNotFound)?
                    {
                        PropertyValue::Object(value) => value,
                        invalid => {
                            return Err(MessagingError::InvalidMessageObjectData(
                                PropertyType::from(invalid),
                            )
                            .into())
                        }
                    };

                    // MailCraft: the message is read below, once the file's lock is let go.
                    embedded_node = Some(Self::object_node(
                        file,
                        &block_btree,
                        &mut page_cache,
                        own_sub_nodes,
                        &message,
                        object_data.node(),
                    )?);
                    None
                }
                AttachmentMethod::Storage => {
                    let object_data = match properties
                        .get(0x3701)
                        .ok_or(MessagingError::AttachmentMessageObjectDataNotFound)?
                    {
                        PropertyValue::Object(value) => value,
                        invalid => {
                            return Err(MessagingError::InvalidMessageObjectData(
                                PropertyType::from(invalid),
                            )
                            .into())
                        }
                    };
                    let node = Self::object_node(
                        file,
                        &block_btree,
                        &mut page_cache,
                        own_sub_nodes,
                        &message,
                        object_data.node(),
                    )?;
                    let block =
                        block_btree.find_entry(file, node.data().search_key(), &mut page_cache)?;
                    let block = DataTree::read(file, encoding, &block)?;
                    // MailCraft: no more than the tree declares, nor than the file's limit.
                    let data = block.read_capped(
                        file,
                        encoding,
                        &block_btree,
                        &mut page_cache,
                        &mut Default::default(),
                        limit,
                    )?;
                    Some(AttachmentData::Binary(BinaryValue::new(data)))
                }
                _ => None,
            };

            (properties, data, embedded_node)
        };

        // MailCraft: reading the attached message locks the file again, and `std::sync::Mutex`
        // isn't re-entrant: read inside the block above, the first attached message hung.
        let (data, embedded) = match embedded_node {
            Some(node) => {
                let embedded = <<Pst as PstFile>::Message as MessageReadWrite<Pst>>::read_embedded(
                    store.clone(),
                    node,
                    prop_ids,
                )?;
                let data: Rc<dyn Message> = embedded.clone();
                (Some(AttachmentData::Message(data)), Some(embedded))
            }
            None => (data, None),
        };

        Ok(Self {
            message,
            properties,
            data,
            embedded,
        })
    }

    /// MailCraft: the node of the object an attachment holds, looked up in the attachment's own
    /// subnode tree (MS-PST 2.4.6.2), then in the message's, where this crate looked before. An
    /// own tree that can't be read is passed over, so a damaged one costs nothing that the
    /// message's tree holds.
    fn object_node<R: PstReader>(
        file: &mut R,
        block_btree: &crate::PstFileReadWriteBlockBTree<Pst>,
        page_cache: &mut RootBTreePageCache<<Pst as PstFile>::BlockBTree>,
        own_sub_nodes: Option<<Pst as PstFile>::BlockId>,
        message: &<Pst as PstFile>::Message,
        object: NodeId,
    ) -> io::Result<<Pst as PstFile>::NodeBTreeEntry> {
        let own = match own_sub_nodes {
            Some(sub_nodes) => {
                let found = block_btree
                    .find_entry(file, sub_nodes.search_key(), page_cache)
                    .and_then(|block| SubNodeTree::<Pst>::read(file, &block))
                    .and_then(|tree| {
                        Ok(tree
                            .entries(file, block_btree, page_cache)?
                            .find(|entry| u32::from(entry.node()) == u32::from(object)))
                    });
                found.unwrap_or_else(|e| {
                    tracing::warn!("an attachment's own subnode tree could not be read: {e}");
                    None
                })
            }
            None => None,
        };
        let entry = match own {
            Some(entry) => entry,
            None => message
                .sub_nodes()
                .get(&object)
                .map(|entry| LeafSubNodeTreeEntry::new(entry.node(), entry.block(), entry.sub_node()))
                .ok_or(MessagingError::AttachmentSubNodeNotFound(object))?,
        };
        Ok(
            <<Pst as PstFile>::NodeBTreeEntry as NodeBTreeEntryReadWrite>::new(
                entry.node(),
                entry.block(),
                entry.sub_node(),
                None,
            ),
        )
    }
}

pub struct UnicodeAttachment {
    inner: AttachmentInner<UnicodePstFile>,
}

impl UnicodeAttachment {
    pub fn read(
        message: Rc<UnicodeMessage>,
        sub_node: NodeId,
        prop_ids: Option<&[u16]>,
    ) -> io::Result<Rc<Self>> {
        <Self as AttachmentReadWrite<UnicodePstFile>>::read(message, sub_node, prop_ids)
    }
    /// MailCraft: the attached message this attachment holds, as a [`UnicodeMessage`], whose own
    /// attachments read with [`UnicodeAttachment::read`]. `None` for any other attachment.
    pub fn embedded_message(&self) -> Option<Rc<UnicodeMessage>> {
        self.inner.embedded.clone()
    }
}

impl Attachment for UnicodeAttachment {
    fn message(&self) -> Rc<dyn Message> {
        self.inner.message.clone()
    }

    fn properties(&self) -> &AttachmentProperties {
        &self.inner.properties
    }

    fn data(&self) -> Option<&AttachmentData> {
        self.inner.data.as_ref()
    }
}

impl AttachmentReadWrite<UnicodePstFile> for UnicodeAttachment {
    fn read(
        message: Rc<UnicodeMessage>,
        sub_node: NodeId,
        prop_ids: Option<&[u16]>,
    ) -> io::Result<Rc<Self>> {
        let inner = AttachmentInner::read(message, sub_node, prop_ids)?;
        Ok(Rc::new(Self { inner }))
    }
}

pub struct AnsiAttachment {
    inner: AttachmentInner<AnsiPstFile>,
}

impl AnsiAttachment {
    pub fn read(
        message: Rc<AnsiMessage>,
        sub_node: NodeId,
        prop_ids: Option<&[u16]>,
    ) -> io::Result<Rc<Self>> {
        <Self as AttachmentReadWrite<AnsiPstFile>>::read(message, sub_node, prop_ids)
    }
    /// MailCraft: the attached message this attachment holds, as a [`AnsiMessage`], whose own
    /// attachments read with [`AnsiAttachment::read`]. `None` for any other attachment.
    pub fn embedded_message(&self) -> Option<Rc<AnsiMessage>> {
        self.inner.embedded.clone()
    }
}

impl Attachment for AnsiAttachment {
    fn message(&self) -> Rc<dyn Message> {
        self.inner.message.clone()
    }

    fn properties(&self) -> &AttachmentProperties {
        &self.inner.properties
    }

    fn data(&self) -> Option<&AttachmentData> {
        self.inner.data.as_ref()
    }
}

impl AttachmentReadWrite<AnsiPstFile> for AnsiAttachment {
    fn read(
        message: Rc<AnsiMessage>,
        sub_node: NodeId,
        prop_ids: Option<&[u16]>,
    ) -> io::Result<Rc<Self>> {
        let inner = AttachmentInner::read(message, sub_node, prop_ids)?;
        Ok(Rc::new(Self { inner }))
    }
}
