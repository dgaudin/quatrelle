// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The fake EWS server's folder operations: a folder asked for, the folders listed (the mailbox's
//! root, a public folder's, a shared mailbox's, the archive's, the whole tree in pages), and a
//! folder made, moved, renamed, deleted or emptied.

use super::*;

impl Data {
    /// A folder operation answered; `None` for any other, which `route` answers.
    pub(super) fn folder_route(&mut self, op: &El) -> Option<Vec<String>> {
        let ok = |inner: String| format!("<m:R ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode>{inner}</m:R>");
        let err =
            |code: &str| format!("<m:R ResponseClass=\"Error\"><m:MessageText>{code}</m:MessageText><m:ResponseCode>{code}</m:ResponseCode></m:R>");
        let ids = |el: Option<&El>| -> Vec<String> {
            el.map(|e| e.kids.iter().filter_map(|k| k.attr("Id").map(str::to_string)).collect()).unwrap_or_default()
        };
        let mut out = Vec::new();
        match op.name.as_str() {
            "GetFolder" => {
                for f in op.child("FolderIds").map(|f| f.kids.clone()).unwrap_or_default() {
                    // Someone else's calendar: what this account may do there.
                    if let Some(who) = self.folder_id(&f).and_then(|id| id.strip_prefix("mailbox:").map(str::to_string)) {
                        let edit = self.editors.contains(&who) && !self.item_rights_only;
                        out.push(ok(format!(
                            "<m:Folders><t:CalendarFolder><t:FolderId Id=\"mailbox:{who}\"/><t:EffectiveRights><t:CreateAssociated>false</t:CreateAssociated>\
                             <t:CreateContents>{edit}</t:CreateContents><t:CreateHierarchy>false</t:CreateHierarchy><t:Delete>{edit}</t:Delete>\
                             <t:Modify>false</t:Modify><t:Read>true</t:Read></t:EffectiveRights></t:CalendarFolder></m:Folders>"
                        )));
                        continue;
                    }
                    if matches!(self.shared_of(&f), Some(Err(()))) {
                        out.push(err("ErrorAccessDenied"));
                        continue;
                    }
                    if let Some(code) = self.archive_error(&f) {
                        out.push(err(code));
                        continue;
                    }
                    let root = f.attr("Id") == Some("publicfoldersroot");
                    match self.folder_id(&f).and_then(|id| self.folders.iter().find(|x| x.id == id).cloned()) {
                        // The root is the hierarchy's; any other public folder, its content's.
                        Some(fo) if root && self.unrouted_hierarchy(&fo.id) => out.push(err("ErrorFolderNotFound")),
                        Some(fo) if !root && self.unrouted(&fo.id) => out.push(err("ErrorFolderNotFound")),
                        Some(fo) => out.push(ok(format!("<m:Folders>{}</m:Folders>", self.folder_xml(&fo)))),
                        None => out.push(err("ErrorFolderNotFound")),
                    }
                }
            }
            // The mailbox's own root (above the mail folders): its Common Views folder.
            "FindFolder"
                if op.attr("Traversal") == Some("Shallow")
                    && op.child("ParentFolderIds").and_then(|p| p.kids.first()).is_some_and(|f| f.attr("Id") == Some("root")) =>
            {
                out.push(ok("<m:RootFolder IncludesLastItemInRange=\"true\"><t:Folders><t:Folder><t:FolderId Id=\"COMMON-VIEWS\"/>\
                     <t:DisplayName>Common Views</t:DisplayName></t:Folder><t:Folder><t:FolderId Id=\"FINDER\"/><t:DisplayName>Finder</t:DisplayName>\
                     </t:Folder></t:Folders></m:RootFolder>"
                    .into()));
            }
            // Shallow (public folders): the folders right under the one asked for.
            "FindFolder" if op.attr("Traversal") == Some("Shallow") => {
                let parent = op.child("ParentFolderIds").and_then(|p| p.kids.first()).and_then(|f| self.folder_id(f)).unwrap_or_default();
                if self.unrouted_hierarchy(&parent) {
                    out.push(err("ErrorFolderNotFound"));
                    return Some(out);
                }
                let kids: Vec<String> = self
                    .folders
                    .iter()
                    .filter(|f| f.parent == parent)
                    .map(|f| {
                        let n = self.folders.iter().filter(|x| x.parent == f.id).count();
                        self.folder_xml(f)
                            .replace("<t:ChildFolderCount>0</t:ChildFolderCount>", &format!("<t:ChildFolderCount>{n}</t:ChildFolderCount>"))
                    })
                    .collect();
                out.push(ok(format!("<m:RootFolder IncludesLastItemInRange=\"true\"><t:Folders>{}</t:Folders></m:RootFolder>", kids.concat())));
            }
            "FindFolder" if op.child("ParentFolderIds").and_then(|p| p.kids.first()).and_then(|f| self.shared_of(f)).is_some() => {
                // Deep, in a shared mailbox: refused without access to it.
                let Some(Ok(owner)) = op.child("ParentFolderIds").and_then(|p| p.kids.first()).and_then(|f| self.shared_of(f)) else {
                    out.push(err("ErrorAccessDenied"));
                    return Some(out);
                };
                let all: Vec<String> = self
                    .folders
                    .iter()
                    .filter(|f| f.owner == owner && f.known.as_deref() != Some("msgfolderroot"))
                    .map(|f| self.folder_xml(f))
                    .collect();
                out.push(ok(format!("<m:RootFolder IncludesLastItemInRange=\"true\"><t:Folders>{}</t:Folders></m:RootFolder>", all.concat())));
            }
            // Deep, in the online archive: refused without one, or by one that refuses.
            "FindFolder"
                if op.child("ParentFolderIds").and_then(|p| p.kids.first()).and_then(|f| f.attr("Id")).is_some_and(super::archive::is_archive_id) =>
            {
                match op.child("ParentFolderIds").and_then(|p| p.kids.first()).and_then(|f| self.archive_error(f)) {
                    Some(code) => out.push(err(code)),
                    None => out.push(self.archive_tree(op)),
                }
            }
            "FindFolder" => {
                // Deep: everything under the root (the mailbox's, not the public folders), in pages.
                let view = op.child("IndexedPageFolderView");
                let offset: usize = view.and_then(|v| v.attr("Offset")).and_then(|o| o.parse().ok()).unwrap_or(0);
                let max: usize = view.and_then(|v| v.attr("MaxEntriesReturned")).and_then(|o| o.parse().ok()).unwrap_or(1000).min(3);
                let public = self.public_ids();
                let all: Vec<Fo> = self.folders.iter().filter(|f| f.id != "ROOT" && f.owner.is_empty() && !public.contains(&f.id)).cloned().collect();
                let page: Vec<String> = all.iter().skip(offset).take(max).map(|f| self.with_folder_policy(op, &f.id, self.folder_xml(f))).collect();
                let last = offset + max >= all.len();
                out.push(ok(format!(
                    "<m:RootFolder TotalItemsInView=\"{}\" IncludesLastItemInRange=\"{last}\"><t:Folders>{}</t:Folders></m:RootFolder>",
                    all.len(),
                    page.concat()
                )));
            }
            "CreateFolder" => {
                let parent = op.child("ParentFolderId").and_then(|p| p.kids.first()).and_then(|f| self.folder_id(f)).unwrap_or_else(|| "ROOT".into());
                let name = op.find("DisplayName").map(|d| d.text.clone()).unwrap_or_default();
                let id = self.folder(&name, Some(&parent), None, "IPF.Note");
                out.push(ok(format!("<m:Folders><t:Folder><t:FolderId Id=\"{id}\"/></t:Folder></m:Folders>")));
            }
            "MoveFolder" => {
                let to = op.child("ToFolderId").and_then(|t| t.kids.first()).and_then(|f| self.folder_id(f)).unwrap_or_else(|| "ROOT".into());
                for id in ids(op.child("FolderIds")) {
                    if let Some(f) = self.folders.iter_mut().find(|f| f.id == id) {
                        f.parent = to.clone();
                    }
                    out.push(ok(format!("<m:Folders><t:Folder><t:FolderId Id=\"{id}\"/></t:Folder></m:Folders>")));
                }
            }
            "UpdateFolder" => {
                for ch in op.child("FolderChanges").map(|c| c.kids.clone()).unwrap_or_default() {
                    let id = ch.child("FolderId").and_then(|i| i.attr("Id")).unwrap_or_default().to_string();
                    let name = ch.find("DisplayName").map(|d| d.text.clone()).unwrap_or_default();
                    if let Some(f) = self.folders.iter_mut().find(|f| f.id == id) {
                        f.name = name;
                    }
                    out.push(ok(String::new()));
                }
            }
            "DeleteFolder" => {
                for id in ids(op.child("FolderIds")) {
                    self.folders.retain(|f| f.id != id);
                    out.push(ok(String::new()));
                }
            }
            "EmptyFolder" => {
                for id in ids(op.child("FolderIds")) {
                    self.take_items(op, |i| i.folder == id);
                    out.push(ok(String::new()));
                }
            }
            _ => return None,
        }
        Some(out)
    }
}
