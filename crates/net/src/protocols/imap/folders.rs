// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The IMAP client's folders: listing them with their roles, creating, renaming, deleting and
//! emptying one.

use super::*;

impl<S: Read + Write> Client<S> {
    /// All folders, with special-use roles.
    pub fn list(&mut self, account: &str) -> Result<Vec<Folder>> {
        let cmd = if self.has("SPECIAL-USE") && self.has("LIST-EXTENDED") { "LIST \"\" \"*\" RETURN (SPECIAL-USE)" } else { "LIST \"\" \"*\"" };
        let rs = self.cmd(&[Part::Text(cmd)])?;
        let mut out: Vec<Folder> = Vec::new();
        // Which roles the server gave (RFC 6154), not guessed from a name.
        let mut flagged = HashSet::new();
        for r in rs {
            let Response::MailboxData(MailboxDatum::List(l)) = r else { continue };
            let delimiter = l.delimiter.as_deref().and_then(|d| d.chars().next());
            let path = l.name.to_string();
            let mut role = None;
            let mut selectable = true;
            for a in &l.name_attributes {
                match a {
                    NameAttribute::NoSelect => selectable = false,
                    NameAttribute::Extension(e) if e.eq_ignore_ascii_case("\\NonExistent") => selectable = false,
                    NameAttribute::Sent => role = Some(FolderRole::Sent),
                    NameAttribute::Drafts => role = Some(FolderRole::Drafts),
                    NameAttribute::Trash => role = Some(FolderRole::Trash),
                    NameAttribute::Junk => role = Some(FolderRole::Junk),
                    NameAttribute::Archive => role = Some(FolderRole::Archive),
                    NameAttribute::All => role = role.or(Some(FolderRole::Archive)),
                    NameAttribute::Extension(e) => role = role.or(FolderRole::from_attribute(e)),
                    _ => {}
                }
            }
            if out.iter().any(|f| f.path == path) {
                continue;
            }
            if role.is_some() && !path.eq_ignore_ascii_case("INBOX") {
                flagged.insert(path.clone());
            }
            let role =
                if path.eq_ignore_ascii_case("INBOX") { FolderRole::Inbox } else { role.unwrap_or_else(|| FolderRole::from_name(&path, delimiter)) };
            out.push(Folder { account: account.to_string(), path, delimiter, role, selectable, ..Default::default() });
        }
        one_per_role(&mut out, &flagged);
        Ok(out)
    }

    /// Delete every message in a folder for good.
    pub fn empty(&mut self, folder: &str) -> Result<()> {
        let info = self.select(folder, false)?;
        if info.exists == 0 {
            return Ok(());
        }
        self.uid_store("1:*", true, "\\Deleted")?;
        self.cmd(&[Part::Text("EXPUNGE")]).map(|_| ())
    }

    /// Create a folder and subscribe to it (some clients only show subscribed folders). A folder
    /// that already exists counts as created.
    pub fn create(&mut self, folder: &str) -> Result<()> {
        let cmd = format!("CREATE {}", mailbox_arg(folder));
        match self.cmd(&[Part::Text(&cmd)]) {
            Ok(_) => {}
            Err(NetError::No(m)) if m.to_ascii_uppercase().contains("ALREADYEXISTS") || m.to_ascii_lowercase().contains("exists") => {}
            Err(e) => return Err(e),
        }
        let _ = self.cmd(&[Part::Text(&format!("SUBSCRIBE {}", mailbox_arg(folder)))]);
        Ok(())
    }

    /// Rename a folder (folders inside it move with it, RFC 3501 §6.3.5).
    pub fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        if self.selected.as_deref().is_some_and(|s| s.starts_with(from)) {
            // Renaming the selected folder is allowed, but leaves the session in an odd state on
            // some servers: deselect first.
            self.unselect();
        }
        let cmd = format!("RENAME {} {}", mailbox_arg(from), mailbox_arg(to));
        self.cmd(&[Part::Text(&cmd)])?;
        let _ = self.cmd(&[Part::Text(&format!("UNSUBSCRIBE {}", mailbox_arg(from)))]);
        let _ = self.cmd(&[Part::Text(&format!("SUBSCRIBE {}", mailbox_arg(to)))]);
        Ok(())
    }

    /// Delete a folder. One that is already gone counts as deleted.
    pub fn delete(&mut self, folder: &str) -> Result<()> {
        if self.selected.as_deref().is_some_and(|s| s.starts_with(folder)) {
            self.unselect();
        }
        let _ = self.cmd(&[Part::Text(&format!("UNSUBSCRIBE {}", mailbox_arg(folder)))]);
        match self.cmd(&[Part::Text(&format!("DELETE {}", mailbox_arg(folder)))]) {
            Ok(_) => Ok(()),
            Err(NetError::No(m)) if m.to_ascii_uppercase().contains("NONEXISTENT") => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Leave the selected folder (`UNSELECT` when offered; `CLOSE` would expunge).
    fn unselect(&mut self) {
        if self.has("UNSELECT") {
            let _ = self.cmd(&[Part::Text("UNSELECT")]);
        } else if self.selected.is_some() {
            // A failed EXAMINE leaves the selected state without expunging (RFC 3501 §6.3.1).
            let _ = self.cmd(&[Part::Text("EXAMINE \"Quatrelle no such folder\"")]);
        }
        self.selected = None;
    }
}

/// One folder per special role, the rest ordinary folders: the one the server flags for it, else
/// one that can be opened, else the first listed. Servers don't list it first (Dovecot lists cPanel's
/// flagged `INBOX.Sent` after `INBOX.Sent Items` made by another program: issue #19, where sent
/// mail's copies went in the one nobody looks at).
fn one_per_role(folders: &mut [Folder], flagged: &HashSet<String>) {
    let mut order: Vec<usize> = (0..folders.len()).collect();
    order.sort_by_key(|&i| folders.get(i).map(|f| (!flagged.contains(&f.path), !f.selectable)));
    let mut seen = HashSet::new();
    for i in order {
        if let Some(f) = folders.get_mut(i)
            && f.role != FolderRole::Other
            && !seen.insert(f.role)
        {
            f.role = FolderRole::Other;
        }
    }
}
