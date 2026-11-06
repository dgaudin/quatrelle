//! MailCraft: folders made with a container class, and the special folders a file records
//! (MS-OXOSFLD 2.2): Deleted Items, Sent Items and the Outbox on the store; Drafts, Junk Email,
//! the Calendar, Contacts, the Journal, Notes and Tasks on the root folder and the Inbox, found by
//! what it records whatever its name.

use super::*;

/// `PR_CONTAINER_CLASS` of a folder of mail.
pub(crate) const MAIL_CONTAINER_CLASS: &str = "IPF.Note";

/// How deep a special folder is looked for under the top of the file.
const MAX_DEPTH: usize = 64;

/// A special folder a data file records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnicodePstSpecialFolder {
    /// `PidTagIpmWastebasketEntryId` on the store.
    DeletedItems,
    /// `PidTagIpmSentMailEntryId` on the store.
    SentItems,
    /// `PidTagIpmOutboxEntryId` on the store.
    Outbox,
    /// `PidTagIpmDraftsEntryId` on the root folder and the Inbox.
    Drafts,
    /// `PidTagAdditionalRenEntryIds`' fifth value on the root folder and the Inbox.
    JunkEmail,
    /// MailCraft: `PidTagIpmAppointmentEntryId` on the root folder and the Inbox.
    Calendar,
    /// MailCraft: `PidTagIpmContactEntryId` on the root folder and the Inbox.
    Contacts,
    /// MailCraft: `PidTagIpmJournalEntryId` on the root folder and the Inbox.
    Journal,
    /// MailCraft: `PidTagIpmNoteEntryId` on the root folder and the Inbox.
    Notes,
    /// MailCraft: `PidTagIpmTaskEntryId` on the root folder and the Inbox.
    Tasks,
}

/// `PidTagAdditionalRenEntryIds`' value for Junk Email.
const JUNK_INDEX: usize = 4;

impl UnicodePstSpecialFolder {
    /// The folder's name in a new data file, as Outlook names it.
    pub fn default_name(self) -> &'static str {
        match self {
            Self::DeletedItems => "Deleted Items",
            Self::SentItems => "Sent Items",
            Self::Outbox => "Outbox",
            Self::Drafts => "Drafts",
            Self::JunkEmail => "Junk Email",
            Self::Calendar => "Calendar",
            Self::Contacts => "Contacts",
            Self::Journal => "Journal",
            Self::Notes => "Notes",
            Self::Tasks => "Tasks",
        }
    }

    /// MailCraft: the container class (`PR_CONTAINER_CLASS`) of its folder: what Outlook shows it
    /// as (MS-OXOSFLD 2.2.2).
    pub fn container_class(self) -> &'static str {
        match self {
            Self::Calendar => "IPF.Appointment",
            Self::Contacts => "IPF.Contact",
            Self::Journal => "IPF.Journal",
            Self::Notes => "IPF.StickyNote",
            Self::Tasks => "IPF.Task",
            _ => MAIL_CONTAINER_CLASS,
        }
    }

    /// The property that records it, and whether that is on the store (else on the root
    /// folder and the Inbox).
    fn property(self) -> (u16, bool) {
        match self {
            Self::DeletedItems => (0x35E3, true),
            Self::SentItems => (0x35E4, true),
            Self::Outbox => (0x35E2, true),
            Self::Drafts => (0x36D7, false),
            Self::JunkEmail => (0x36D8, false),
            Self::Calendar => (0x36D0, false),
            Self::Contacts => (0x36D1, false),
            Self::Journal => (0x36D2, false),
            Self::Notes => (0x36D3, false),
            Self::Tasks => (0x36D4, false),
        }
    }
}

impl UnicodePstFile {
    /// MailCraft: make the folders of `folder_path` that are missing, relative to the IPM
    /// subtree; the last gets `container_class` (`PR_CONTAINER_CLASS`, such as `IPF.Note`), any
    /// made on the way `IPF.Note`. Folders already there keep their own.
    pub fn create_folder(
        path: impl AsRef<Path>,
        folder_path: &[&str],
        container_class: &str,
    ) -> io::Result<Self> {
        expect_format(path.as_ref(), Format::Unicode)?;
        create_folder(path.as_ref(), folder_path, container_class)?;
        Self::open(path)
    }

    /// MailCraft: the folder the file records as `role`, as names from the IPM subtree, when it
    /// records one there that is still in the file.
    pub fn special_folder(
        path: impl AsRef<Path>,
        role: UnicodePstSpecialFolder,
    ) -> io::Result<Option<Vec<String>>> {
        special_folder(path.as_ref(), role)
    }

    /// MailCraft: record the folder at `folder_path` (relative to the IPM subtree, made when
    /// missing, as [`UnicodePstFile::create_folder`] makes it with the role's
    /// [`UnicodePstSpecialFolder::container_class`]) as `role`, in place of any folder recorded
    /// before. The roles not kept on the store are recorded on the root folder, the IPM subtree
    /// and the Inbox.
    pub fn set_special_folder(
        path: impl AsRef<Path>,
        role: UnicodePstSpecialFolder,
        folder_path: &[&str],
    ) -> io::Result<Self> {
        expect_format(path.as_ref(), Format::Unicode)?;
        set_special_folder(path.as_ref(), role, folder_path)?;
        Self::open(path)
    }
}

/// MailCraft: make the folders of `folder_path` that are missing, in a file of either format
/// ([`UnicodePstFile::create_folder`], [`AnsiPstFile::create_folder`]).
pub(crate) fn create_folder(
    path: &Path,
    folder_path: &[&str],
    container_class: &str,
) -> io::Result<()> {
    if container_class.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "container class must not be empty",
        ));
    }
    ensure_unicode_folder_path(path, folder_path, container_class)?;
    Ok(())
}

/// The folder a file of either format records as `role` ([`UnicodePstFile::special_folder`]).
pub(crate) fn special_folder(
    path: &Path,
    role: UnicodePstSpecialFolder,
) -> io::Result<Option<Vec<String>>> {
    let store = AddStore::open(path)?;
    let (id, on_store) = role.property();
    let entry_ids = if on_store {
        vec![store.properties().get(id).cloned()]
    } else {
        recording_folders(&*store)?
            .into_iter()
            .map(|node| {
                let folder = store.open_folder(&store.properties().make_entry_id(node)?)?;
                Ok::<_, io::Error>(folder.properties().get(id).cloned())
            })
            .collect::<io::Result<Vec<_>>>()?
    };
    for value in entry_ids.into_iter().flatten() {
        let entry_id = match (&value, role) {
            (PropertyValue::MultipleBinary(values), UnicodePstSpecialFolder::JunkEmail) => {
                values.get(JUNK_INDEX).map(|value| value.buffer().to_vec())
            }
            (PropertyValue::Binary(value), _) => Some(value.buffer().to_vec()),
            _ => None,
        };
        let Some(node) = entry_id
            .and_then(|bytes| EntryId::try_from(bytes.as_slice()).ok())
            .map(|entry_id| entry_id.node_id())
        else {
            continue;
        };
        if let Some(names) = folder_names(&*store, node)? {
            return Ok(Some(names));
        }
    }
    Ok(None)
}

/// Record a folder of a file of either format as `role` ([`UnicodePstFile::set_special_folder`]).
pub(crate) fn set_special_folder(
    path: &Path,
    role: UnicodePstSpecialFolder,
    folder_path: &[&str],
) -> io::Result<()> {
    let folder = ensure_unicode_folder_path(path, folder_path, role.container_class())?;
    let store = AddStore::open(path)?;
    let record_key = *store.properties().record_key()?.record_key();
    let entry_id = EntryId::new(StoreRecordKey::new(record_key), folder);
    let entry_id = Vec::try_from(&entry_id)?;
    let (id, on_store) = role.property();
    let set = |properties: &mut Vec<(u16, PropertyValue)>| {
        let value = if role == UnicodePstSpecialFolder::JunkEmail {
            let mut values = match properties.iter().find(|(existing, _)| *existing == id) {
                Some((_, PropertyValue::MultipleBinary(values))) => values.clone(),
                _ => Vec::new(),
            };
            if values.len() <= JUNK_INDEX {
                values.resize(JUNK_INDEX + 1, BinaryValue::new(Vec::new()));
            }
            if let Some(slot) = values.get_mut(JUNK_INDEX) {
                *slot = BinaryValue::new(entry_id.clone());
            }
            PropertyValue::MultipleBinary(values)
        } else {
            PropertyValue::Binary(BinaryValue::new(entry_id.clone()))
        };
        properties.retain(|(existing, _)| *existing != id);
        properties.push((id, value));
    };
    let mut updates = Vec::new();
    if on_store {
        let mut properties: Vec<_> = store
            .properties()
            .iter()
            .map(|(id, value)| (*id, value.clone()))
            .collect();
        set(&mut properties);
        updates.push((NID_MESSAGE_STORE, properties));
    } else {
        for node in recording_folders(&*store)? {
            let folder = store.open_folder(&store.properties().make_entry_id(node)?)?;
            let mut properties = writable_folder_properties(folder.as_ref());
            set(&mut properties);
            updates.push((node, properties));
        }
    }
    drop(store);
    rewrite_properties(path, updates)
}

/// The folders the roles not kept on the store are recorded on: the root folder, the IPM subtree
/// and, when the file has one, the Inbox.
fn recording_folders(store: &dyn Store) -> io::Result<Vec<NodeId>> {
    let top = store.properties().ipm_sub_tree_entry_id()?.node_id();
    let mut nodes = vec![NID_ROOT_FOLDER, top];
    if let Some(inbox) = find_inbox(store, top)? {
        nodes.push(inbox);
    }
    nodes.dedup();
    Ok(nodes)
}

/// The special folders MS-OXOSFLD 2.2 records on the Inbox (and the root folder) alone:
/// `PidTagIpmAppointmentEntryId`, `PidTagIpmContactEntryId`, `PidTagIpmJournalEntryId`,
/// `PidTagIpmNoteEntryId`, `PidTagIpmTaskEntryId`, `PidTagIpmDraftsEntryId` and
/// `PidTagAdditionalRenEntryIds`.
const INBOX_RECORDS: [u16; 7] = [0x36D0, 0x36D1, 0x36D2, 0x36D3, 0x36D4, 0x36D7, 0x36D8];

/// MailCraft: the file's Inbox, among the folders at the top of the IPM subtree: the one that
/// records the special folders the Inbox records, as Outlook's Inbox does in a file of any
/// language ("Posteingang", "Boîte de réception"), else the one named Inbox. A folder that doesn't
/// open is passed over.
fn find_inbox(store: &dyn Store, top: NodeId) -> io::Result<Option<NodeId>> {
    let parent = store.open_folder(&store.properties().make_entry_id(top)?)?;
    let Some(hierarchy) = parent.hierarchy_table() else {
        return Ok(None);
    };
    let mut named = None;
    for row in hierarchy.rows_matrix() {
        let child = NodeId::from(u32::from(row.id()));
        let Ok(folder) = store.open_folder(&store.properties().make_entry_id(child)?) else {
            continue;
        };
        let properties = folder.properties();
        if INBOX_RECORDS.iter().any(|id| properties.get(*id).is_some()) {
            return Ok(Some(child));
        }
        if named.is_none()
            && properties
                .display_name()
                .is_ok_and(|name| name.eq_ignore_ascii_case("Inbox"))
        {
            named = Some(child);
        }
    }
    Ok(named)
}

/// A folder's names from the IPM subtree down to it; `None` when it isn't a folder under it (the
/// subtree itself isn't), or is deeper than [`MAX_DEPTH`].
///
/// MailCraft: each folder is looked through once, so a damaged hierarchy that lists a folder
/// under itself, or twice, costs one look per folder, not one per path to it.
fn folder_names(store: &dyn Store, node: NodeId) -> io::Result<Option<Vec<String>>> {
    let top = store.properties().ipm_sub_tree_entry_id()?.node_id();
    if node == top {
        return Ok(None);
    }
    // Down from the top, as names in the hierarchy tables, so the folder's place is the one
    // the file shows.
    let mut stack = vec![(top, Vec::<String>::new())];
    let mut seen = HashSet::new();
    while let Some((parent, names)) = stack.pop() {
        if names.len() >= MAX_DEPTH || !seen.insert(u32::from(parent)) {
            continue;
        }
        let Ok(folder) = store.open_folder(&store.properties().make_entry_id(parent)?) else {
            continue;
        };
        let Some(hierarchy) = folder.hierarchy_table() else {
            continue;
        };
        for row in hierarchy.rows_matrix() {
            let child = NodeId::from(u32::from(row.id()));
            let Ok(child_folder) = store.open_folder(&store.properties().make_entry_id(child)?)
            else {
                continue;
            };
            let Ok(name) = child_folder.properties().display_name() else {
                continue;
            };
            let mut path = names.clone();
            path.push(name);
            if child == node {
                return Ok(Some(path));
            }
            stack.push((child, path));
        }
    }
    Ok(None)
}

/// Give each node (the store or a folder) a new property context with `properties`, in one
/// commit; values too big for its heap go in subnodes after the ones it has.
fn rewrite_properties(
    path: &Path,
    updates: Vec<(NodeId, Vec<(u16, PropertyValue)>)>,
) -> io::Result<()> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let header = FileHeader::read(&mut file)?;
    // MailCraft: an ANSI file's too, its values widened.
    let format = header.format();
    let old_bbt = read_block_btree(&mut file, format, header.block_btree())?;
    let old_nbt = read_node_btree(&mut file, format, header.node_btree())?;
    let bbt_entries = old_bbt.entries();
    let mut nbt_entries = old_nbt.entries();
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
    for (node, properties) in updates {
        let entry = nbt_entries
            .iter()
            .find(|entry| entry.node() == node)
            .copied()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "folder node not found"))?;
        let mut subnodes =
            collect_subnode_entries(&mut file, format, header.block_btree(), entry.sub_node())?;
        let next_subnode = subnodes
            .iter()
            .filter(|entry| matches!(entry.node().id_type(), Ok(NodeIdType::Internal)))
            .map(|entry| entry.node().index())
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(PstError::IntegerConversion)?;
        let (pages, external) = property_context_pages(properties, next_subnode, format.text())?;
        let data = push_data_tree_chunks(&mut blocks, &mut next_bid, &mut next_offset, pages)?;
        for (subnode, value) in external {
            let value = push_data_tree(&mut blocks, &mut next_bid, &mut next_offset, &value)?;
            subnodes.push(UnicodeLeafSubNodeTreeEntry::new(subnode, value, None));
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
        nbt_entries.retain(|candidate| candidate.node() != node);
        nbt_entries.push(UnicodeNodeBTreeEntry::new(
            node,
            data,
            subnode,
            entry.parent(),
        ));
    }
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

#[cfg(test)]
mod tests;
