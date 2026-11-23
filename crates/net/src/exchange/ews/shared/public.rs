// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! EWS: the public folders walked, a level at a time and each folder's own a page at a time:
//! their mail folders, and their contact folders and calendars; and what of them didn't list
//! whole.

use std::collections::{HashSet, VecDeque};

use super::*;
use crate::exchange::missed::PublicMissed;

/// A public folder's folders asked for in one page.
const PUBLIC_PAGE: usize = 200;
/// Pages asked for one public folder's folders at most: the most listed (`MAX_PUBLIC`) at a
/// page of ten, and an end for a server that says more follow without end.
const PUBLIC_PAGES_MAX: usize = 50;

/// The public folders walked: the root's id, the folders found, and those whose own didn't list
/// whole, so that what was under them isn't taken for gone.
pub(in crate::exchange::ews) struct PublicTree {
    root_id: String,
    folders: Vec<TreeFolder>,
    /// The folders (by id, the root's too) whose own didn't list whole: a page of them failed,
    /// their pages were given up (too many, or one bringing none new while more were to follow),
    /// or the cap on the folders listed or the depth walked stopped at them.
    incomplete: HashSet<String>,
    /// The cap on the folders listed (`MAX_PUBLIC`) left some out.
    capped: bool,
}

/// A walk of the public folders under way: the tree so far, the folders taken (each once,
/// however the server pages), and those whose own are still to be asked for, how deep.
struct Walk {
    tree: PublicTree,
    seen: HashSet<String>,
    queue: VecDeque<(String, usize)>,
}

impl Client {
    /// Exchange's public folders (the organization's shared folders): their mail folders, under
    /// a "Public Folders" folder, and what of them didn't list whole (the folders whose own
    /// didn't, by id). Public folders only list one level at a time, so the tree is walked, a few
    /// levels deep and up to a few hundred folders.
    pub(in crate::exchange::ews) fn public_folders(&self) -> Result<(Vec<Mailbox>, PublicMissed)> {
        let tree = self.public_tree()?;
        // Counts only: how many were listed, and whether the cap cut the walk.
        log::info!(
            "public folders: {} listed, {} whose own didn't list whole, cut by the cap of {MAX_PUBLIC}: {}",
            tree.folders.len(),
            tree.incomplete.len(),
            tree.capped
        );
        let missed = if tree.incomplete.is_empty() { PublicMissed::None } else { PublicMissed::Under(tree.incomplete) };
        // Calendars and contacts kept as public folders aren't mail; a folder holding others is
        // kept for them.
        let mut out: Vec<Mailbox> = tree
            .folders
            .into_iter()
            .filter(|t| t.class.is_empty() || t.class == "IPF.Note" || t.class.starts_with("IPF.Note.") || t.children)
            .map(|t| t.mailbox)
            .collect();
        if out.is_empty() {
            return Ok((out, missed));
        }
        out.insert(0, Mailbox { id: tree.root_id, name: PUBLIC_FOLDERS.into(), parent: None, role: FolderRole::Other, total: 0, unread: 0 });
        Ok((out, missed))
    }

    /// Exchange's public contact folders and calendars (shared address books and calendars), as
    /// far as the tree listed: what didn't list whole isn't told apart here.
    pub(crate) fn public_pim_folders(&self) -> Result<Vec<PublicFolder>> {
        let all = self.public_tree()?.folders;
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
    pub(in crate::exchange::ews) fn public_tree(&self) -> Result<PublicTree> {
        let plain = self.public_tree_walk();
        if plain.as_ref().is_ok_and(|t| !t.folders.is_empty()) {
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

    /// Walk the public folders a level at a time, a few levels deep, each folder's own a page at
    /// a time: the root's id, and each folder with its class, whether it holds others and where
    /// its content is kept; and the folders whose own didn't list whole. Only the root's first
    /// page failing fails the walk: then none can be told.
    pub(in crate::exchange::ews) fn public_tree_walk(&self) -> Result<PublicTree> {
        let root = self.call_one(
            "<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape></m:FolderShape>\
             <m:FolderIds><t:DistinguishedFolderId Id=\"publicfoldersroot\"/></m:FolderIds></m:GetFolder>",
        )?;
        let tree = |root_id: String| PublicTree { root_id, folders: Vec::new(), incomplete: HashSet::new(), capped: false };
        let Some(root_id) = root.find("FolderId").and_then(|f| f.attr("Id")).map(str::to_string) else { return Ok(tree(String::new())) };
        let mut walk = Walk { tree: tree(root_id.clone()), seen: HashSet::from([root_id.clone()]), queue: VecDeque::from([(root_id, 0)]) };
        while let Some((parent, depth)) = walk.queue.pop_front() {
            // Too deep, or the cap reached: its folders aren't asked for, and what's under it
            // isn't taken for gone.
            if depth >= PUBLIC_DEPTH || walk.tree.folders.len() >= MAX_PUBLIC {
                walk.tree.capped |= walk.tree.folders.len() >= MAX_PUBLIC;
                walk.tree.incomplete.insert(parent);
                continue;
            }
            if !self.public_children(&mut walk, &parent, depth)? {
                walk.tree.incomplete.insert(parent);
            }
        }
        Ok(walk.tree)
    }

    /// One public folder's folders (`parent`'s, `depth` down), a page at a time from where the
    /// last left off, into the walk: whether they listed whole. The pages read are kept when a
    /// later one fails. Only the root's first page failing is an error.
    fn public_children(&self, walk: &mut Walk, parent: &str, depth: usize) -> Result<bool> {
        let mut offset = 0usize;
        for page in 0..PUBLIC_PAGES_MAX {
            if walk.tree.folders.len() >= MAX_PUBLIC {
                walk.tree.capped = true;
                return Ok(false);
            }
            let m = self.call_one(&format!(
                "<m:FindFolder Traversal=\"Shallow\"><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>\
                 <t:FieldURI FieldURI=\"folder:DisplayName\"/><t:FieldURI FieldURI=\"folder:TotalCount\"/>\
                 <t:FieldURI FieldURI=\"folder:UnreadCount\"/><t:FieldURI FieldURI=\"folder:FolderClass\"/>\
                 <t:FieldURI FieldURI=\"folder:ChildFolderCount\"/>{REPLICA_LIST}</t:AdditionalProperties></m:FolderShape>\
                 <m:IndexedPageFolderView MaxEntriesReturned=\"{PUBLIC_PAGE}\" Offset=\"{offset}\" BasePoint=\"Beginning\"/>\
                 <m:ParentFolderIds><t:FolderId Id=\"{}\"/></m:ParentFolderIds></m:FindFolder>",
                esc(parent)
            ));
            let m = match m {
                Ok(m) => m,
                Err(e) if depth == 0 && page == 0 => return Err(e),
                Err(e) => {
                    log::info!("a public folder's folders, page {}: {e}", page.saturating_add(1));
                    return Ok(false);
                }
            };
            // An answer without its folders says nothing of them.
            let Some(rf) = m.child("RootFolder") else { return Ok(false) };
            let last = rf.attr("IncludesLastItemInRange").is_none_or(|v| v.trim() == "true");
            let folders = rf.folders();
            let mut new = 0usize;
            for f in &folders {
                let Some(id) = f.child("FolderId").and_then(|i| i.attr("Id")) else { continue };
                if walk.seen.contains(id) {
                    continue;
                }
                if walk.tree.folders.len() >= MAX_PUBLIC {
                    walk.tree.capped = true;
                    return Ok(false);
                }
                walk.seen.insert(id.to_string());
                new = new.saturating_add(1);
                self.take_public(walk, f, id, parent, depth);
            }
            // A page bringing none new ends the pages: whole only when it's an empty last one
            // (one repeating a page, or ignoring where to start, doesn't say the rest).
            if new == 0 {
                return Ok(folders.is_empty() && last);
            }
            if last {
                return Ok(true);
            }
            offset = offset.saturating_add(folders.len());
        }
        // More said to follow, page after page.
        Ok(false)
    }

    /// A public folder found (`f`, its `id`), under `parent`, `depth` down: into the walk's tree,
    /// and to be asked for its own when it holds others.
    fn take_public(&self, walk: &mut Walk, f: &El, id: &str, parent: &str, depth: usize) {
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
        let children = count("ChildFolderCount") > 0;
        walk.tree.folders.push(TreeFolder {
            mailbox: Mailbox {
                name: f.text_of("DisplayName").unwrap_or_default().chars().take(255).collect(),
                parent: Some(parent.to_string()),
                role: FolderRole::Other,
                total: count("TotalCount"),
                unread: count("UnreadCount"),
                id: id.to_string(),
            },
            class,
            children,
            replica,
        });
        if children {
            walk.queue.push_back((id.to_string(), depth.saturating_add(1)));
        }
    }
}
