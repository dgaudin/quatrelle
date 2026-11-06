// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Exporting an account's mail to an Outlook data file (`.pst`).
//!
//! The file is written with the MIT-licensed `outlook-pst-rw` crate (MS-PST, Unicode format):
//! each folder kept on this computer becomes a folder of the same name, and each message whose
//! body is here goes in with its addresses, text and HTML bodies and attachments. Messages only
//! known by their headers (never opened, on an account that keeps headers only) are counted as
//! skipped. The writer makes mail items only, so the account's calendar and contacts go beside
//! the file in the standard formats every calendar and address book reads (`… Calendar.ics`,
//! `… Contacts.vcf`).

use std::path::Path;

use outlook_pst::{UnicodePstAttachment, UnicodePstBatchMessage, UnicodePstFile, UnicodePstMessage, UnicodePstRecipient, UnicodePstRecipientType};
use quatrelle_mime as mime;
use quatrelle_model::{FolderRole, MessageKey, utf7};
use quatrelle_store::Store;

/// Messages written per commit to the file, at most: the writer rewrites the folder's table of
/// contents with each, so a big folder written a hundred at a time took longer with every batch.
const BATCH: usize = 2000;
/// …and their bytes (attachments and all), at most.
const BATCH_BYTES: usize = 48 << 20;
/// Attachments larger than this are left out.
const MAX_ATTACHMENT: usize = 100 * 1024 * 1024;

/// What an export did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Exported {
    pub messages: usize,
    /// Messages whose body isn't on this computer, or that couldn't be read.
    pub skipped: usize,
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
use place::names;
use writing::write;

/// Write `account`'s mail to a new data file at `path` (replacing a file already there), named
/// `name` when opened. Calls `progress` with the number of messages written so far.
pub fn export(store: &Store, account: &str, name: &str, path: &Path, progress: &mut dyn FnMut(usize)) -> Result<Exported, String> {
    let folders = store.load_folders(account).map_err(|e| e.to_string())?;
    let mut done = Exported::default();
    let mut created = false;
    let temp = path.with_extension("pst.part");
    let _ = std::fs::remove_file(&temp);
    let written = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<(), String> {
        for f in folders.iter().filter(|f| f.selectable) {
            let Ok(st) = store.load_folder(account, &f.path) else { continue };
            let folder = names(&f.path, f.delimiter, f.role);
            let folder: Vec<&str> = folder.iter().map(String::as_str).collect();
            let mut batch = Vec::new();
            let mut bytes = 0usize;
            for s in &st.messages {
                let key = MessageKey { account: account.to_string(), folder: f.path.clone(), validity: s.key.validity, uid: s.key.uid };
                match store.read_body(&key).and_then(|raw| owned(&raw)) {
                    Some(m) => {
                        bytes += m.body.len() + m.html.as_ref().map_or(0, String::len) + m.files.iter().map(|f| f.3.len()).sum::<usize>();
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
        Ok(())
    }));
    let result = match written {
        Ok(Ok(())) if !created => Err("there is no mail on this computer to export".to_string()),
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
