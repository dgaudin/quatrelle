// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Exporting an account's mail to an Outlook data file (`.pst`).
//!
//! The file is written with the MIT-licensed `outlook-pst-rw` crate (MS-PST, Unicode format):
//! each folder kept on this computer becomes a folder of the same name, and the account's Sent,
//! Trash, Drafts and Junk folders go to the file's own Sent Items, Deleted Items, Drafts and Junk
//! Email ([`place`]). Each message whose body is here goes in with its addresses (Bcc too), text
//! and HTML bodies, attachments and header block, and with what the cache keeps of it: read or
//! not, a draft, flagged (and when it's due), answered or forwarded, its categories, importance
//! and sensitivity ([`item`]). Messages only known by their headers (never opened, on an account
//! that keeps headers only) are counted as skipped. The writer makes mail items only, so the
//! account's calendar and contacts go beside the file in the standard formats every calendar and
//! address book reads (`… Calendar.ics`, `… Contacts.vcf`).
//!
//! A data file that's already there is added to, as Outlook's export does: its mail stays, the
//! account's goes into folders of the same names, and messages already in it (by `Message-ID`)
//! aren't written twice. The file is written as a copy that replaces it at the end, so a failed
//! export leaves it as it was.

use std::collections::HashSet;
use std::io;
use std::path::Path;
use std::rc::Rc;

use outlook_pst::messaging::message::{Message as _, UnicodeMessage};
use outlook_pst::messaging::store::{Store as PstStore, UnicodeStore};
use outlook_pst::ndb::node_id::NodeId;
use outlook_pst::{
    UnicodePstAttachment, UnicodePstBatchMessage, UnicodePstFile, UnicodePstMessage, UnicodePstProperty, UnicodePstPropertyKey, UnicodePstRecipient,
    UnicodePstRecipientType,
};
use quatrelle_mime as mime;
use quatrelle_model::organize::ExportMarks;
use quatrelle_model::{FolderRole, MessageKey, utf7};
use quatrelle_store::Store;

/// Messages written per commit to the file, at most: the writer rewrites the folder's table of
/// contents with each, so a big folder written a hundred at a time took longer with every batch.
const BATCH: usize = 2000;
/// …and their bytes (attachments and all), at most.
const BATCH_BYTES: usize = 48 << 20;
/// Attachments larger than this are left out.
const MAX_ATTACHMENT: usize = 100 * 1024 * 1024;
/// How deep folders in a data file already there are looked through.
const MAX_DEPTH: usize = 64;

/// What an export did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Exported {
    pub messages: usize,
    /// Messages whose body isn't on this computer, or that couldn't be read.
    pub skipped: usize,
    /// Messages a data file already there had (by `Message-ID`), not written again.
    pub already: usize,
    /// Events and people written beside the data file.
    pub events: usize,
    pub contacts: usize,
}

mod item;
mod place;
#[cfg(test)]
mod tests;
mod writing;

use item::{Owned, owned};
use writing::write;

/// The `Message-ID`s in an existing data file's folders (a file the writer can add to: the Unicode
/// format, not Outlook's offline copy of a server account). The file is read as the import reads
/// one, within its limit for a value, and a panic in the reader is an error.
fn ids_in(path: &Path) -> Result<HashSet<String>, String> {
    let mut head = [0u8; 12];
    std::fs::File::open(path).and_then(|mut f| io::Read::read_exact(&mut f, &mut head)).map_err(|e| e.to_string())?;
    if !head.starts_with(b"!BDN") || head.get(8..10) != Some(b"SM") {
        return Err("it isn't an Outlook data file: choose another name to make a new one".into());
    }
    let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> io::Result<HashSet<String>> {
        let pst = outlook_pst::UnicodePstFile::open(path)?;
        pst.set_max_value_size(crate::pst::MAX_VALUE);
        let store = UnicodeStore::read(Rc::new(pst))?;
        let mut ids = HashSet::new();
        let any: Rc<dyn PstStore> = store.clone();
        let top = any.open_folder(&any.properties().ipm_sub_tree_entry_id()?)?;
        let mut stack = vec![(top, 0usize)];
        while let Some((folder, depth)) = stack.pop() {
            if let Some(contents) = folder.contents_table() {
                for row in contents.rows_matrix() {
                    let Ok(entry) = any.properties().make_entry_id(NodeId::from(u32::from(row.id()))) else { continue };
                    let Ok(m) = UnicodeMessage::read(store.clone(), &entry, Some(&[0x1035])) else { continue };
                    if let Some(id) = m.properties().get(0x1035).and_then(crate::pst::text_of) {
                        ids.insert(quatrelle_model::thread::norm_id(&id));
                    }
                }
            }
            if depth < MAX_DEPTH
                && let Some(table) = folder.hierarchy_table()
            {
                for row in table.rows_matrix() {
                    if let Ok(sub) = any.properties().make_entry_id(NodeId::from(u32::from(row.id()))).and_then(|e| any.open_folder(&e)) {
                        stack.push((sub, depth + 1));
                    }
                }
            }
        }
        Ok(ids)
    }));
    match read {
        Ok(Ok(ids)) => Ok(ids),
        // ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION.
        Ok(Err(e)) if matches!(e.raw_os_error(), Some(32 | 33)) => {
            Err("Outlook (or another program) has this data file open: close Outlook, including from the notification area, and try again".into())
        }
        Ok(Err(_)) | Err(_) => Err("this data file can't be read, so mail can't be added to it: choose another name to make a new one".into()),
    }
}

/// Write `account`'s mail to a data file at `path`, named `name` when it's new; a file already
/// there is added to. `marks`: what only this computer keeps of the mail (when flagged messages
/// are due, the categories' names). Calls `progress` with the number of messages written so far.
pub fn export(
    store: &Store,
    account: &str,
    name: &str,
    path: &Path,
    marks: &ExportMarks,
    progress: &mut dyn FnMut(usize),
) -> Result<Exported, String> {
    let folders = store.load_folders(account).map_err(|e| e.to_string())?;
    let mut done = Exported::default();
    let temp = path.with_extension("pst.part");
    let _ = std::fs::remove_file(&temp);
    // Added to: a copy of it, with the messages it has already left out.
    let existing = path.is_file();
    let had = if existing { ids_in(path)? } else { HashSet::new() };
    if existing {
        std::fs::copy(path, &temp).map_err(|e| e.to_string())?;
    }
    let mut created = existing;
    // The special folders a file added to records, else the names a new one gets.
    let targets = place::Targets::of(existing.then_some(temp.as_path()));
    let written = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<(), String> {
        for f in folders.iter().filter(|f| f.selectable) {
            let Ok(st) = store.load_folder(account, &f.path) else { continue };
            let folder = place::place(f, &folders, &targets);
            let folder: Vec<&str> = folder.iter().map(String::as_str).collect();
            let mut batch = Vec::new();
            let mut bytes = 0usize;
            for s in &st.messages {
                let key = MessageKey { account: account.to_string(), folder: f.path.clone(), validity: s.key.validity, uid: s.key.uid };
                let state = item::state(s, f.role, marks);
                match store.read_body(&key).and_then(|raw| owned(&raw, s.received, state)) {
                    Some(m) if !m.message_id.is_empty() && had.contains(&quatrelle_model::thread::norm_id(&m.message_id)) => done.already += 1,
                    Some(m) => {
                        let held = [m.body.len(), m.html.as_ref().map_or(0, String::len), m.headers.len()]
                            .into_iter()
                            .chain(m.files.iter().map(|f| f.3.len()));
                        bytes = held.fold(bytes, usize::saturating_add);
                        batch.push(m);
                    }
                    None => done.skipped += 1,
                }
                if batch.len() >= BATCH || bytes >= BATCH_BYTES {
                    let n = batch.len();
                    write(&temp, &folder, &std::mem::take(&mut batch), &mut created)?;
                    bytes = 0;
                    done.messages += n;
                    progress(done.messages);
                }
            }
            if !batch.is_empty() {
                write(&temp, &folder, &batch, &mut created)?;
                done.messages += batch.len();
                progress(done.messages);
            }
        }
        place::record_special(&temp, &folders, &targets, created);
        Ok(())
    }));
    let result = match written {
        Ok(Ok(())) if !created => Err("there is no mail on this computer to export".to_string()),
        // A file added to keeps its own name.
        Ok(Ok(())) if existing => Ok(()),
        Ok(Ok(())) => UnicodePstFile::set_store_display_name(&temp, name).map(drop).map_err(|e| e.to_string()),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("the data file could not be written".into()),
    };
    match result {
        Ok(()) => {
            // `rename` replaces a file already there (MoveFileExW with MOVEFILE_REPLACE_EXISTING on
            // Windows), so the old file stays until the new one takes its place; if it can't, the
            // mail is still in the `.pst.part` file, and the error says so.
            if let Err(e) = std::fs::rename(&temp, path) {
                return Err(format!("the data file couldn't be replaced ({e}): the mail is in {}", temp.display()));
            }
            let (events, contacts) = pim_beside(store, account, path).map_err(|e| e.to_string())?;
            Ok(Exported { events, contacts, ..done })
        }
        Err(e) => {
            let _ = std::fs::remove_file(&temp);
            Err(e)
        }
    }
}

/// The account's calendar and contacts beside the data file: (events, people) written.
fn pim_beside(store: &Store, account: &str, path: &Path) -> quatrelle_store::Result<(usize, usize)> {
    let pim: quatrelle_model::pim::Pim = store.load_account_doc(account, quatrelle_model::pim::PIM_DOC).unwrap_or_default();
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Mail".into());
    let dir = path.parent().unwrap_or(Path::new("."));
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| i64::try_from(d.as_secs()).unwrap_or(0)).unwrap_or(0);
    let events: Vec<quatrelle_model::ical::CalEvent> = pim.events.iter().map(|e| e.event.clone()).collect();
    if !events.is_empty() {
        quatrelle_store::write_atomic(&dir.join(format!("{stem} Calendar.ics")), quatrelle_model::ical::write(&events, None, now).as_bytes())?;
    }
    if !pim.cards.is_empty() {
        let cards: String = pim.cards.iter().map(|c| quatrelle_model::vcard::write(&c.card)).collect();
        quatrelle_store::write_atomic(&dir.join(format!("{stem} Contacts.vcf")), cards.as_bytes())?;
    }
    Ok((events.len(), pim.cards.len()))
}
