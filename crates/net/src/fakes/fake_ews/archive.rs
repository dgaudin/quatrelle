// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The fake EWS server's online archive: a second mailbox of the account, named by Autodiscover
//! and listed under `archivemsgfolderroot`.

use super::*;

/// The account's archive mailbox, as Autodiscover names it.
#[derive(Clone, Debug)]
pub struct ArchiveBox {
    /// Its display name (empty: Autodiscover gives none).
    pub name: String,
    /// Its address (`SmtpAddress`), when Autodiscover gives one.
    pub address: Option<String>,
    /// Listing it is refused (`ErrorAccessDenied`): turned off by an administrator, or its
    /// server down.
    pub refused: bool,
}

impl ArchiveBox {
    /// Whose its folders are, among the fake's.
    fn owner(&self) -> String {
        self.address.as_deref().unwrap_or("archive").to_ascii_lowercase()
    }
}

/// Whether a distinguished folder id names a folder of the archive (`archivemsgfolderroot`,
/// `archivedeleteditems`…; `archive` alone is the mailbox's own Archive folder).
pub(super) fn is_archive_id(known: &str) -> bool {
    known.len() > "archive".len() && known.starts_with("archive")
}

impl Data {
    /// Give the account an online archive named `name`, at archive@contoso.example: its root's id.
    pub fn archive_mailbox(&mut self, name: &str) -> String {
        self.archive = Some(ArchiveBox { name: name.into(), address: Some("archive@contoso.example".into()), refused: false });
        self.archive_make("Top of Information Store", "", Some("archivemsgfolderroot"))
    }

    /// A mail folder of the archive, under `parent` (the root's id, or a folder's): its id.
    pub fn archive_folder(&mut self, name: &str, parent: &str) -> String {
        self.archive_make(name, parent, None)
    }

    /// The archive's Deleted Items, under its root: its id.
    pub fn archive_deleted_items(&mut self, parent: &str) -> String {
        self.archive_make("Deleted Items", parent, Some("archivedeleteditems"))
    }

    fn archive_make(&mut self, name: &str, parent: &str, known: Option<&str>) -> String {
        let owner = self.archive.as_ref().map(ArchiveBox::owner).unwrap_or_default();
        let id = self.id("A");
        self.folders.push(Fo {
            id: id.clone(),
            name: name.into(),
            parent: parent.into(),
            class: "IPF.Note".into(),
            known: known.map(str::to_string),
            owner,
        });
        id
    }

    /// Whose a distinguished folder asked for without a mailbox is: the archive's, or the
    /// account's own (empty).
    pub(super) fn owner_of_known(&self, known: &str) -> String {
        if is_archive_id(known) { self.archive.as_ref().map(ArchiveBox::owner).unwrap_or_else(|| "no archive".into()) } else { String::new() }
    }

    /// Whether a folder is one of the archive's.
    pub fn in_archive(&self, folder: &str) -> bool {
        let Some(owner) = self.archive.as_ref().map(ArchiveBox::owner) else { return false };
        self.folders.iter().any(|f| f.id == folder && f.owner == owner)
    }

    /// The error a request naming a folder of the archive by its well-known id gets, if any: no
    /// archive, or one that refuses.
    pub(super) fn archive_error(&self, f: &El) -> Option<&'static str> {
        if f.name != "DistinguishedFolderId" || !f.attr("Id").is_some_and(is_archive_id) {
            return None;
        }
        match &self.archive {
            None => Some("ErrorArchiveMailboxNotEnabled"),
            Some(a) if a.refused => Some("ErrorAccessDenied"),
            Some(_) => None,
        }
    }

    /// `FindFolder` (deep) under the archive's root: its folders, in the pages asked for.
    pub(super) fn archive_tree(&self, op: &El) -> String {
        let owner = self.archive.as_ref().map(ArchiveBox::owner).unwrap_or_default();
        let view = op.child("IndexedPageFolderView");
        let offset: usize = view.and_then(|v| v.attr("Offset")).and_then(|o| o.parse().ok()).unwrap_or(0);
        let max: usize = view.and_then(|v| v.attr("MaxEntriesReturned")).and_then(|o| o.parse().ok()).unwrap_or(1000).min(1000);
        let all: Vec<&Fo> = self.folders.iter().filter(|f| f.owner == owner && f.known.as_deref() != Some("archivemsgfolderroot")).collect();
        // A big tree's counts aren't worked out folder by folder (ten thousand of them).
        let page: String = if all.len() > 1000 {
            all.iter()
                .skip(offset)
                .take(max)
                .map(|f| {
                    format!(
                        "<t:Folder><t:FolderId Id=\"{}\"/><t:ParentFolderId Id=\"{}\"/><t:FolderClass>{}</t:FolderClass><t:DisplayName>{}</t:DisplayName>\
                         <t:TotalCount>0</t:TotalCount><t:ChildFolderCount>0</t:ChildFolderCount><t:UnreadCount>0</t:UnreadCount></t:Folder>",
                        f.id,
                        f.parent,
                        f.class,
                        esc(&f.name)
                    )
                })
                .collect()
        } else {
            all.iter().skip(offset).take(max).map(|f| self.folder_xml(f)).collect()
        };
        let last = offset.saturating_add(max) >= all.len();
        format!(
            "<m:R ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode>\
             <m:RootFolder TotalItemsInView=\"{}\" IncludesLastItemInRange=\"{last}\"><t:Folders>{page}</t:Folders></m:RootFolder></m:R>",
            all.len()
        )
    }

    /// How many times the items of a folder were listed (`FindItem`), and the most asked for in
    /// one go.
    pub fn listings_of(&self, folder: &str) -> (usize, usize) {
        let asked: Vec<usize> = self.found_in.iter().filter(|(f, _, _)| f == folder).map(|(_, _, max)| *max).collect();
        (asked.len(), asked.into_iter().max().unwrap_or(0))
    }
}
