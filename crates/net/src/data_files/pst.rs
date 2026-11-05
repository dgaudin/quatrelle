// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Importing an Outlook data file (`.pst`) into an account kept on this computer.
//!
//! The file is read with Microsoft's MIT-licensed reader (MS-PST), from the fixed copy of
//! `outlook-pst-rw` the export writes with too (library `outlook_pst`): the folder tree
//! under the top of the mailbox becomes folders, and each mail item is written out again as a
//! standard message (RFC 5322 / MIME) from its properties (MS-OXCMSG / MS-OXPROPS), keeping the
//! original headers when the item has them. Contacts and calendar items go to the account's address
//! book and calendar (see `pst_pim`), notes to its notes, tasks to its tasks.
//!
//! A data file is hostile input like any other: the reader runs behind `catch_unwind`, items that
//! can't be read are skipped and counted, and sizes are capped.

use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::rc::Rc;

use outlook_pst::ltp::prop_context::PropertyValue;
use outlook_pst::ltp::table_context::TableContext;
use outlook_pst::messaging::attachment::{AnsiAttachment, Attachment, AttachmentData, UnicodeAttachment};
use outlook_pst::messaging::folder::Folder as PstFolder;
use outlook_pst::messaging::message::{AnsiMessage, Message, UnicodeMessage};
use outlook_pst::messaging::store::{AnsiStore, EntryId, Store as PstStore, UnicodeStore};
use outlook_pst::ndb::node_id::NodeId;
use outlook_pst::{AnsiPstFile, UnicodePstFile};
use quatrelle_mime::{self as mime, Draft, InlinePicture, OutAttachment};
use quatrelle_model::pim::{LOCAL, NOTE_YELLOW, Note, PIM_DOC, Pim, StoredCard, StoredEvent, StoredNote, StoredTask, Task};
use quatrelle_model::{EmailAddress, Flags, Folder, FolderRole, MessageKey, utf7};
use quatrelle_store::{FolderState, Store};

mod item;
mod out;
mod props;
mod rebuild;
#[cfg(test)]
mod tests;

use item::{Item, attachment, item, message_code_page, picture_of};
use out::Out;
pub(crate) use props::{Val, text_of};

/// Folders nested deeper than this are left out.
const MAX_DEPTH: usize = 32;
/// Attachments larger than this are left out of the imported message.
const MAX_ATTACHMENT: usize = 100 * 1024 * 1024;
/// A value (a body, an attachment's bytes, a table's rows, a store's, folder's or the name map's
/// properties) the file says is larger than this is an error before any of it is read: a damaged
/// size can't make the import run out of memory.
const MAX_VALUE: u64 = 256 * 1024 * 1024;
/// Original headers longer than this are not kept.
const MAX_HEADERS: usize = 256 * 1024;
/// Embedded messages inside embedded messages… stop here.
const MAX_NESTING: usize = 3;
/// Message ids kept from an item's References (as many as threading reads).
const MAX_IDS: usize = 256;

/// What an import did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Imported {
    pub folders: Vec<Folder>,
    pub messages: usize,
    /// Contacts, calendar items, notes and tasks.
    pub contacts: usize,
    pub events: usize,
    pub notes: usize,
    pub tasks: usize,
    /// Items that couldn't be read or written.
    pub skipped: usize,
    /// Why the folder list or the contacts, calendar and notes couldn't be saved at the end, when
    /// they couldn't: the messages above came in all the same.
    pub last_save: Option<String>,
    /// Messages with text (or an attached message with text) in a code page Quatrelle can't read
    /// (ISO-2022-KR): imported with only its ASCII. Logged, and not in `skipped`, since the
    /// message itself came in.
    pub charset_unsupported: usize,
}

/// Import the data file at `path` into `account`'s cache (an account with no server). Calls
/// `progress` with the number of messages imported so far, every so often.
pub fn import(path: &Path, store: &Store, account: &str, progress: &mut dyn FnMut(usize)) -> Result<Imported, String> {
    let mut out = Out {
        store,
        account,
        folders: Vec::new(),
        states: HashMap::new(),
        imported: 0,
        skipped: 0,
        charset_unsupported: 0,
        progress,
        pim: Pim::default(),
        pending: Vec::new(),
        pending_bytes: 0,
    };
    // `!BDN` then the client magic: `SM` a data file, `SO` Outlook's offline copy of a server
    // account (MS-PST 2.2.2.6).
    let mut head = [0u8; 10];
    if std::fs::File::open(path).and_then(|mut f| io::Read::read_exact(&mut f, &mut head)).is_ok()
        && head.starts_with(b"!BDN")
        && head.get(8..10) == Some(b"SO")
    {
        return Err("this is Outlook's offline copy of a server account (.ost), not a data file: add the account itself to see its mail".into());
    }
    let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> io::Result<()> {
        match UnicodePstFile::open(path) {
            Ok(pst) => read_unicode(pst, &mut out),
            // Locked by Outlook: say so, rather than trying it as the older format.
            Err(e) if matches!(e.raw_os_error(), Some(32 | 33)) => Err(e),
            Err(_) => read_ansi(AnsiPstFile::open(path)?, &mut out),
        }
    }));
    match read {
        Ok(Ok(())) => {}
        // Windows' sharing and lock violations: Outlook (or another program) has it open.
        Ok(Err(e)) if matches!(e.raw_os_error(), Some(32 | 33)) => {
            return Err(
                "Outlook (or another program) has this data file open: close Outlook, including from the notification area, and try again".into()
            );
        }
        // Nothing found at all: say why; otherwise keep what was read.
        Ok(Err(e)) if out.folders.is_empty() => return Err(format!("not an Outlook data file, or it is damaged ({e})")),
        Ok(Err(e)) => log::warn!("{}: the rest of the data file could not be read: {e}", path.display()),
        Err(_) if out.folders.is_empty() => return Err("the data file is damaged".into()),
        Err(_) => log::warn!("{}: the data file is damaged; stopped reading", path.display()),
    }
    Ok(out.finish())
}

macro_rules! reader {
    ($name:ident, $file:ty, $store:ty, $message:ty, $attachment:ty) => {
        fn $name(pst: $file, out: &mut Out<'_>) -> io::Result<()> {
            pst.set_max_value_size(MAX_VALUE);
            let store = <$store>::read(Rc::new(pst))?;
            let any: Rc<dyn PstStore> = store.clone();
            let named = crate::pst_pim::Named::of(&*any);
            let read = |entry: &EntryId| -> io::Result<Item> {
                let m = <$message>::read(store.clone(), entry, None)?;
                let cp = message_code_page(&*m);
                let mut files = Vec::new();
                let mut photo = None;
                if let Some(table) = m.attachment_table() {
                    for row in table.rows_matrix() {
                        let node = NodeId::from(u32::from(row.id()));
                        match <$attachment>::read(m.clone(), node, None) {
                            Ok(a) => {
                                // A contact's picture is one of its attachments, hidden.
                                photo = photo.or_else(|| picture_of(&*a));
                                files.extend(attachment(&*a, 0, cp, &named));
                            }
                            Err(e) => log::info!("an attachment could not be read: {e}"),
                        }
                    }
                }
                Ok(item(&*m, files, &named, photo))
            };
            walk(&*any, &read, out)
        }
    };
}

/// Bytes from a data file as the import reads them as text: an HTML body (`html`) in its internet
/// code page, or an 8-bit string in its message's. For the fuzz target.
#[doc(hidden)]
pub fn text_from_bytes(bytes: &[u8], code_page: u32, html: bool) -> String {
    props::decode(bytes, code_page, html)
}

reader!(read_unicode, UnicodePstFile, UnicodeStore, UnicodeMessage, UnicodeAttachment);
reader!(read_ansi, AnsiPstFile, AnsiStore, AnsiMessage, AnsiAttachment);

type ReadItem<'a> = dyn Fn(&EntryId) -> io::Result<Item> + 'a;

fn walk(store: &dyn PstStore, read: &ReadItem<'_>, out: &mut Out<'_>) -> io::Result<()> {
    let top = store.open_folder(&store.properties().ipm_sub_tree_entry_id()?)?;
    visit(store, &*top, None, 0, read, out);
    Ok(())
}

fn visit(store: &dyn PstStore, folder: &dyn PstFolder, parent: Option<&str>, depth: usize, read: &ReadItem<'_>, out: &mut Out<'_>) {
    if depth >= MAX_DEPTH {
        return;
    }
    // The reader may panic on a damaged table (it runs behind `catch_unwind` in `import`, which
    // stopped the whole import there): here a panic costs one folder, and the import goes on.
    let listed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        folder.hierarchy_table().map(|t| t.rows_matrix().map(|row| u32::from(row.id())).collect::<Vec<_>>())
    }));
    let nodes = match listed {
        Ok(nodes) => nodes.unwrap_or_default(),
        Err(_) => {
            log::warn!("a folder's subfolders could not be read: the reader failed on them");
            out.skipped += 1;
            return;
        }
    };
    for node in nodes {
        let mut at = Visited::default();
        let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| visit_one(store, node, parent, read, out, &mut at)));
        if done.is_err() {
            // What came in of it stays, in its folder's list; the folders under it are read.
            log::warn!("a folder could not be read to its end: the reader failed on it");
            out.skipped += 1;
            if let Some(path) = &at.path {
                out.save_folder(path);
            }
        }
        if let Some(sub) = at.sub {
            let under = if at.mail { at.path.as_deref() } else { parent };
            visit(store, &*sub, under, depth + 1, read, out);
        }
    }
}

/// How far [`visit_one`] got with a folder, kept when the reader panics part-way.
#[derive(Default)]
struct Visited {
    /// The folder, opened (its subfolders are read next), unless it's left out.
    sub: Option<Rc<dyn PstFolder>>,
    /// A mail folder: the subfolders go in it (others' go beside it).
    mail: bool,
    /// The folder made for it, once made.
    path: Option<String>,
}

/// One folder of the file read in: its mail, or its contacts and calendar items.
fn visit_one(store: &dyn PstStore, node: u32, parent: Option<&str>, read: &ReadItem<'_>, out: &mut Out<'_>, at: &mut Visited) {
    let opened = store.properties().make_entry_id(NodeId::from(node)).and_then(|e| store.open_folder(&e));
    let sub = match opened {
        Ok(f) => f,
        Err(e) => {
            log::info!("a folder could not be read: {e}");
            return;
        }
    };
    // Mail folders (no class means mail, in old files), contacts and calendars.
    let class = sub.properties().get(0x3613).and_then(text_of).unwrap_or_default();
    let mail = (class.is_empty() || class.starts_with("IPF.Note") || class.starts_with("IPF.Imap"))
        // Outlook's RSS Feeds folder is its own bookkeeping, not mail.
        && !class.starts_with("IPF.Note.OutlookHomepage");
    // Outlook's own Outbox, when it has nothing waiting (Quatrelle has an Outbox of its own).
    let empty = sub.contents_table().is_none_or(|t| t.rows_matrix().next().is_none());
    if empty && special_folder(store, 0x35E2) == Some(node) {
        return;
    }
    if !mail {
        if class.starts_with("IPF.Contact")
            || class.starts_with("IPF.Appointment")
            || class.starts_with("IPF.Task")
            || class.starts_with("IPF.StickyNote")
        {
            at.sub = Some(sub.clone());
            read_items(store, &*sub, read, out);
        }
        return;
    }
    let name = sub.properties().get(0x3001).and_then(text_of).unwrap_or_default();
    let path = out.add_folder(parent, &name);
    (at.sub, at.mail, at.path) = (Some(sub.clone()), true, Some(path.clone()));
    if let Some(contents) = sub.contents_table() {
        for row in contents.rows_matrix() {
            #[cfg(test)]
            tests::fault(&name);
            let item = store.properties().make_entry_id(NodeId::from(u32::from(row.id()))).and_then(|e| read(&e));
            match item {
                Ok(i) => out.add_message(&path, i),
                Err(e) => {
                    log::info!("a message could not be read: {e}");
                    out.skipped += 1;
                }
            }
        }
    }
    out.save_folder(&path);
}

/// The node of one of the store's special folders (`0x35E2`: the Outbox), from its entry id
/// (flags, the store's 16-byte key, then the node id).
fn special_folder(store: &dyn PstStore, prop: u16) -> Option<u32> {
    let (_, v) = store.properties().iter().find(|(id, _)| **id == prop)?;
    let PropertyValue::Binary(b) = v else { return None };
    let id = b.buffer().get(20..24)?;
    Some(u32::from_le_bytes([*id.first()?, *id.get(1)?, *id.get(2)?, *id.get(3)?]))
}

/// A contacts or calendar folder's items.
fn read_items(store: &dyn PstStore, folder: &dyn PstFolder, read: &ReadItem<'_>, out: &mut Out<'_>) {
    let Some(contents) = folder.contents_table() else { return };
    for row in contents.rows_matrix() {
        match store.properties().make_entry_id(NodeId::from(u32::from(row.id()))).and_then(|e| read(&e)) {
            Ok(i) => out.add_pim(i),
            Err(e) => {
                log::info!("an item could not be read: {e}");
                out.skipped += 1;
            }
        }
    }
}
