// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The local archive: one message copied into the Archives account on this computer and read back.
//! Nothing is deleted on the server unless this says the copy is there and identical.

use std::sync::Mutex;

use quatrelle_mime as mime;
use quatrelle_model::{Flags, Folder, FolderRole, MessageKey, sort_newest_first};
use quatrelle_store::{FolderState, Store};
use ring::digest;

use crate::{NetError, Result};

/// The delimiter of the Archives account's folders.
pub(crate) const DELIMITER: char = '/';

/// Writers of the Archives account's folder lists, one after the other: a list is read, changed
/// and saved whole, so two workers filing into one at once would lose a message's entry.
static WRITING: Mutex<()> = Mutex::new(());

fn sha256(raw: &[u8]) -> Vec<u8> {
    digest::digest(&digest::SHA256, raw).as_ref().to_vec()
}

/// The folder and its parents listed in the Archives account's folders.
fn list_folder(store: &Store, account: &str, path: &str) -> Result<()> {
    let mut list = store.load_folders(account)?;
    let mut at = String::new();
    let mut changed = false;
    for part in path.split(DELIMITER).filter(|p| !p.is_empty()) {
        if !at.is_empty() {
            at.push(DELIMITER);
        }
        at.push_str(part);
        if !list.iter().any(|f| f.path == at) {
            list.push(Folder {
                account: account.to_string(),
                path: at.clone(),
                delimiter: Some(DELIMITER),
                role: FolderRole::Other,
                selectable: true,
                ..Default::default()
            });
            changed = true;
        }
    }
    if changed {
        quatrelle_model::sort_folders(&mut list);
        store.save_folders(account, &list)?;
    }
    Ok(())
}

/// The folder's counts in the account's list, after a message came in.
fn count_folder(store: &Store, account: &str, path: &str, total: usize, unread: u32) -> Result<()> {
    let mut list = store.load_folders(account)?;
    for f in list.iter_mut().filter(|f| f.path == path) {
        f.total = u32::try_from(total).unwrap_or(u32::MAX);
        f.unread = unread;
    }
    store.save_folders(account, &list)?;
    Ok(())
}

/// A message already filed in `fs` that is this one: the same Message-ID and size, or, with no
/// Message-ID, the same size and the same SHA-256. The copy's source is read to say so.
fn already_there(store: &Store, fs: &FolderState, id: &str, raw: &[u8]) -> Option<MessageKey> {
    let size = u32::try_from(raw.len()).unwrap_or(u32::MAX);
    let sum = sha256(raw);
    fs.messages
        .iter()
        .filter(|m| m.size == size && m.message_id == id)
        .find(|m| matches!(store.load_body(&m.key), Ok(Some(have)) if sha256(&have) == sum))
        .map(|m| m.key.clone())
}

/// Copy a message into `to_folder` of the Archives account `to_account` (the folder and its parents
/// made when missing), with its marks, keywords and arrival time, and read it back: the same
/// SHA-256 or an error, and then nothing of it is kept. A message already there (the same
/// Message-ID and size, or without a Message-ID the same size and SHA-256) is not written twice:
/// its key is returned.
pub(crate) fn copy_checked(store: &Store, to_account: &str, to_folder: &str, raw: &[u8], flags: Flags, received: i64) -> Result<MessageKey> {
    let _one_at_a_time = WRITING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    list_folder(store, to_account, to_folder)?;
    let mut fs = store.load_folder(to_account, to_folder)?;
    let id = mime::summary_from_header(raw, MessageKey::default(), Flags::default(), 0, 0).message_id;
    if let Some(key) = already_there(store, &fs, &id, raw) {
        return Ok(key);
    }
    if fs.validity == 0 {
        fs.validity = 1;
    }
    let mut next = fs.free_uid(to_account, to_folder)?;
    let uid = FolderState::take_uid(&mut next, to_account, to_folder)?;
    let key = MessageKey { account: to_account.to_string(), folder: to_folder.to_string(), validity: fs.validity, uid };
    let size = u32::try_from(raw.len()).unwrap_or(u32::MAX);
    let received = if received > 0 { received } else { crate::worker::now() };
    let mut summary = mime::summary_from_full(raw, key.clone(), flags, size, received);
    summary.received = received;
    // The source first, then the list that names it: a copy that can't be written is not listed.
    store.write_body(&key, raw).inspect_err(|_| store.remove_body(&key))?;
    fs.messages.push(summary);
    fs.uid_next = next;
    sort_newest_first(&mut fs.messages);
    let listed = fs.messages.len();
    let unread = fs.unread();
    if let Err(e) = store.save_folder(to_account, to_folder, &fs) {
        store.remove_body(&key);
        return Err(e.into());
    }
    // Read back from the Archives account, as it is kept now.
    let same = matches!(store.load_body(&key), Ok(Some(have)) if sha256(&have) == sha256(raw));
    if !same {
        fs.messages.retain(|m| m.key != key);
        let _ = store.save_folder(to_account, to_folder, &fs);
        store.remove_body(&key);
        return Err(NetError::No("the archived copy didn't read back the same".into()));
    }
    count_folder(store, to_account, to_folder, listed, unread)?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARCHIVES: &str = "a9";

    fn mail(id: &str, body: &str) -> Vec<u8> {
        format!(
            "From: Zoe <zoe@studio.example>\r\nTo: me@studio.example\r\nSubject: Plans\r\n{id}Date: Wed, 14 Oct 2026 09:30:00 +0000\r\n\r\n{body}\r\n"
        )
        .into_bytes()
    }

    #[test]
    fn a_copy_is_written_once_and_reads_back_the_same() {
        let store = Store::memory();
        let raw = mail("Message-ID: <plans-1@studio.example>\r\n", "Lunch at noon.");
        let flags = Flags { seen: true, flagged: true, answered: true, keywords: vec!["work".into()], ..Default::default() };
        let key = copy_checked(&store, ARCHIVES, "Gmail/INBOX/2025", &raw, flags.clone(), 1_700_000_000).unwrap();
        assert_eq!(store.load_body(&key).unwrap().as_deref(), Some(raw.as_slice()));
        // The same source again: the same message, nothing more.
        let again = copy_checked(&store, ARCHIVES, "Gmail/INBOX/2025", &raw, flags.clone(), 1_700_000_000).unwrap();
        assert_eq!(again, key);
        let fs = store.load_folder(ARCHIVES, "Gmail/INBOX/2025").unwrap();
        assert_eq!(fs.messages.len(), 1);
        let m = &fs.messages[0];
        assert_eq!((m.flags.seen, m.flags.flagged, m.flags.answered), (true, true, true));
        assert_eq!(m.flags.keywords, ["work"]);
        assert_eq!(m.received, 1_700_000_000);
        assert_eq!(m.message_id, "plans-1@studio.example");
        // The folder and its parents are listed, counted.
        let list = store.load_folders(ARCHIVES).unwrap();
        let paths: Vec<&str> = list.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["Gmail", "Gmail/INBOX", "Gmail/INBOX/2025"]);
        assert_eq!(list.iter().find(|f| f.path == "Gmail/INBOX/2025").map(|f| (f.total, f.unread)), Some((1, 0)));
        // Without a Message-ID: the same size and bytes are the same message; other bytes are not.
        let bare = mail("", "No id here.");
        let first = copy_checked(&store, ARCHIVES, "Gmail/INBOX", &bare, Flags::default(), 0).unwrap();
        assert_eq!(copy_checked(&store, ARCHIVES, "Gmail/INBOX", &bare, Flags::default(), 0).unwrap(), first);
        let other = mail("", "No id hera.");
        assert_ne!(copy_checked(&store, ARCHIVES, "Gmail/INBOX", &other, Flags::default(), 0).unwrap(), first);
        assert_eq!(store.load_folder(ARCHIVES, "Gmail/INBOX").unwrap().messages.len(), 2);
    }

    #[test]
    fn a_copy_that_reads_back_different_is_refused() {
        let raw = mail("Message-ID: <plans-2@studio.example>\r\n", "Lunch at noon.");
        // The source can't be written.
        let store = Store::failing();
        store.fail(None, Some(".eml"));
        assert!(copy_checked(&store, ARCHIVES, "Gmail/INBOX", &raw, Flags::default(), 0).is_err());
        store.fail(None, None);
        assert!(store.load_folder(ARCHIVES, "Gmail/INBOX").unwrap().messages.is_empty());
        // Written, but read back damaged: refused, and nothing is left listed.
        store.fail(Some("!.eml"), None);
        let e = copy_checked(&store, ARCHIVES, "Gmail/INBOX", &raw, Flags::default(), 0).unwrap_err();
        assert!(e.to_string().contains("read back"), "{e}");
        store.fail(None, None);
        assert!(store.load_folder(ARCHIVES, "Gmail/INBOX").unwrap().messages.is_empty());
        // And a good disk takes it.
        assert!(copy_checked(&store, ARCHIVES, "Gmail/INBOX", &raw, Flags::default(), 0).is_ok());
    }
}
