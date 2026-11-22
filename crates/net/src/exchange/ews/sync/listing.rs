// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! EWS: the folder list — the account's own folders, and those of the mailboxes it opens — and
//! what of it couldn't be listed.

use super::*;
use crate::exchange::missed::Listed;

/// Refresh the folder list (and the folder ids behind it), with what of it couldn't be listed
/// this time ([`Listed::missed`]: a shared mailbox, the public folders, the online archive).
pub fn list_folders(c: &Client, st: &mut State, account: &str) -> Result<Listed> {
    let (boxes, missed) = c.mailboxes()?;
    let mut found = jmap::folders(account, &boxes);
    st.mailboxes = found.iter().map(|(f, id)| (f.path.clone(), id.clone())).collect();
    jmap::name_special(&mut found, &boxes);
    // A shared mailbox's folders say whose they are, and what each is there.
    let shared = c.shared_roles.borrow();
    for (f, id) in &mut found {
        if let Some((address, role)) = shared.get(id.as_str()) {
            f.mailbox = Some(address.clone());
            f.mailbox_role = *role;
        }
    }
    // The archive and retention policy tags a folder carries of its own.
    let tagged = c.folder_tags.borrow();
    for (f, id) in &mut found {
        (f.archive_tag, f.retention_tag) = tagged.get(id.as_str()).cloned().unwrap_or_default();
    }
    // The online archive's folders say so: read on the server, never synced nor kept.
    let archive = c.archive.borrow();
    for (f, id) in &mut found {
        if let Some(role) = archive.roles.get(id.as_str()) {
            f.mailbox = Some(archive.mailbox.clone());
            f.mailbox_role = *role;
            f.role = FolderRole::Other;
            f.archive = true;
        }
    }
    Ok(Listed { folders: found.into_iter().map(|(f, _)| f).collect(), missed })
}
