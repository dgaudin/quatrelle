// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! EWS: the public folders walked, a level at a time: their mail folders, and their contact
//! folders and calendars.

use super::*;

impl Client {
    /// Exchange's public folders (the organization's shared folders): their mail folders, under
    /// a "Public Folders" folder. Public folders only list one level at a time, so the tree is
    /// walked, a few levels deep and up to a few hundred folders.
    pub(in crate::exchange::ews) fn public_folders(&self) -> Result<Vec<Mailbox>> {
        let (root_id, all) = self.public_tree()?;
        // Calendars and contacts kept as public folders aren't mail; a folder holding others is
        // kept for them.
        let mut out: Vec<Mailbox> = all
            .into_iter()
            .filter(|t| t.class.is_empty() || t.class == "IPF.Note" || t.class.starts_with("IPF.Note.") || t.children)
            .map(|t| t.mailbox)
            .collect();
        if out.is_empty() {
            return Ok(out);
        }
        out.insert(0, Mailbox { id: root_id, name: PUBLIC_FOLDERS.into(), parent: None, role: FolderRole::Other, total: 0, unread: 0 });
        Ok(out)
    }

    /// Exchange's public contact folders and calendars (shared address books and calendars).
    pub(crate) fn public_pim_folders(&self) -> Result<Vec<PublicFolder>> {
        let (_, all) = self.public_tree()?;
        let path = |m: &Mailbox| {
            let mut parts = vec![m.name.clone()];
            let mut up = m.parent.clone();
            while let Some(p) = up.take() {
                if let Some(t) = all.iter().find(|t| t.mailbox.id == p)
                    && parts.len() < PUBLIC_DEPTH + 1
                {
                    parts.push(t.mailbox.name.clone());
                    up = t.mailbox.parent.clone();
                }
            }
            parts.reverse();
            parts.join(" › ")
        };
        Ok(all
            .iter()
            .filter(|t| t.class.starts_with("IPF.Contact") || t.class.starts_with("IPF.Appointment"))
            .map(|t| PublicFolder {
                id: t.mailbox.id.clone(),
                path: path(&t.mailbox),
                class: t.class.clone(),
                total: t.mailbox.total,
                replica: t.replica.clone(),
            })
            .collect())
    }

    /// Every public folder: asked plainly (older servers, one public folder mailbox), else
    /// routed to the account's public folder hierarchy mailbox, as Exchange 2013 and later want.
    pub(in crate::exchange::ews) fn public_tree(&self) -> Result<(String, Vec<TreeFolder>)> {
        let plain = self.public_tree_walk();
        if plain.as_ref().is_ok_and(|(_, all)| !all.is_empty()) {
            return plain;
        }
        match self.public_hierarchy_mailbox() {
            Some(mailbox) => match self.routed_to(&mailbox, || self.public_tree_walk()) {
                Ok(t) => Ok(t),
                Err(e) => Err(NetError::No(format!("public folders ({mailbox}): {e}"))),
            },
            None => plain,
        }
    }

    /// Walk the public folders a level at a time, a few levels deep: the root's id, and each
    /// folder with its class, whether it holds others and where its content is kept. A folder
    /// that won't list its own is passed over.
    pub(in crate::exchange::ews) fn public_tree_walk(&self) -> Result<(String, Vec<TreeFolder>)> {
        let root = self.call_one(
            "<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape></m:FolderShape>\
             <m:FolderIds><t:DistinguishedFolderId Id=\"publicfoldersroot\"/></m:FolderIds></m:GetFolder>",
        )?;
        let Some(root_id) = root.find("FolderId").and_then(|f| f.attr("Id")).map(str::to_string) else { return Ok((String::new(), Vec::new())) };
        let mut out: Vec<TreeFolder> = Vec::new();
        let mut queue: std::collections::VecDeque<(String, usize)> = std::collections::VecDeque::from([(root_id.clone(), 0)]);
        while let Some((parent, depth)) = queue.pop_front() {
            if depth >= PUBLIC_DEPTH || out.len() >= MAX_PUBLIC {
                break;
            }
            let m = self.call_one(&format!(
                "<m:FindFolder Traversal=\"Shallow\"><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>\
                 <t:FieldURI FieldURI=\"folder:DisplayName\"/><t:FieldURI FieldURI=\"folder:TotalCount\"/>\
                 <t:FieldURI FieldURI=\"folder:UnreadCount\"/><t:FieldURI FieldURI=\"folder:FolderClass\"/>\
                 <t:FieldURI FieldURI=\"folder:ChildFolderCount\"/>{REPLICA_LIST}</t:AdditionalProperties></m:FolderShape>\
                 <m:IndexedPageFolderView MaxEntriesReturned=\"200\" Offset=\"0\" BasePoint=\"Beginning\"/>\
                 <m:ParentFolderIds><t:FolderId Id=\"{}\"/></m:ParentFolderIds></m:FindFolder>",
                esc(&parent)
            ));
            let m = match m {
                Ok(m) => m,
                Err(e) if depth == 0 => return Err(e),
                Err(e) => {
                    log::info!("a public folder's folders: {e}");
                    continue;
                }
            };
            let folders = m.child("RootFolder").map(El::folders).unwrap_or_default();
            for f in folders {
                let Some(id) = f.child("FolderId").and_then(|i| i.attr("Id")).map(str::to_string) else { continue };
                let count = |k: &str| f.text_of(k).and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(0);
                let class = f.text_of("FolderClass").unwrap_or("").to_string();
                let replica = f
                    .children("ExtendedProperty")
                    .find(|p| p.child("ExtendedFieldURI").and_then(|u| u.attr("PropertyTag")).is_some_and(|t| is_tag(t, "0x6698")))
                    .and_then(|p| p.text_of("Value"))
                    .and_then(replica_guid);
                if let Some(g) = &replica {
                    let mut known = self.pf_replicas.borrow_mut();
                    if !known.contains(g) && known.len() < 20 {
                        known.push(g.clone());
                    }
                }
                out.push(TreeFolder {
                    mailbox: Mailbox {
                        name: f.text_of("DisplayName").unwrap_or_default().chars().take(255).collect(),
                        parent: Some(parent.clone()),
                        role: FolderRole::Other,
                        total: count("TotalCount"),
                        unread: count("UnreadCount"),
                        id: id.clone(),
                    },
                    class,
                    children: count("ChildFolderCount") > 0,
                    replica,
                });
                if count("ChildFolderCount") > 0 {
                    queue.push_back((id, depth + 1));
                }
                if out.len() >= MAX_PUBLIC {
                    break;
                }
            }
        }
        Ok((root_id, out))
    }
}
