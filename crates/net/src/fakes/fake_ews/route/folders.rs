// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The fake EWS server's folder operations: a folder asked for, the folders listed (the mailbox's
//! root, a public folder's, a shared mailbox's, the archive's, the whole tree in pages), and a
//! folder made, moved, renamed, deleted or emptied; and what a test scripts on those lists.

use super::*;

/// What a test scripts on the lists of folders beyond the account's own — a shared mailbox's, the
/// public folders', the online archive's — and on what finds them (Autodiscover, the directory):
/// a list refused, answered with no folder, in pages, not answered.
#[derive(Clone, Debug, Default)]
pub struct ZoneScript {
    /// Shared mailboxes (by address) whose folders' list is refused (`ErrorAccessDenied`): their
    /// folders and items are answered still.
    pub refused: Vec<String>,
    /// Shared mailboxes (by address) whose folders' list comes back with none.
    pub emptied: Vec<String>,
    /// A shared mailbox's folders come in pages of this many at most (as many as asked, without).
    pub shared_page: Option<usize>,
    /// Shared mailboxes (by address) whose list's pages after the first come back with no
    /// response message.
    pub unanswered_pages: Vec<String>,
    /// The public folders' root is refused (`ErrorAccessDenied`), routed or not.
    pub public_refused: bool,
    /// The public folders' root has no folder under it.
    pub public_emptied: bool,
    /// The online archive's list comes back with no folder (refused: `ArchiveBox::refused`).
    pub archive_emptied: bool,
    /// The next Autodiscover requests about the account fail (HTTP 500), this many.
    pub discoveries_failing: usize,
    /// The Autodiscover requests about the account seen, failed or answered.
    pub discoveries: usize,
    /// `ResolveNames` goes unanswered (HTTP 500).
    pub directory_silent: bool,
    /// A public folder's folders come in pages of this many at most (as many as asked, without).
    pub public_page: Option<usize>,
    /// Public folders (by id) whose folders' list answers its first page at every `Offset`,
    /// never saying it's the last.
    pub public_repeated: Vec<String>,
    /// Public folders (by id) whose folders' list ignores `Offset`: every page is the first, said
    /// to be the last where the one asked for would be.
    pub public_offset_ignored: Vec<String>,
    /// Public folders (by id) whose folders' list is refused from its page `n` on (1: the first).
    pub public_failing: Vec<(String, usize)>,
    /// Public folders (by id) whose folders' list never ends: each page brings folders never
    /// seen, and says more follow.
    pub public_endless: Vec<String>,
    /// The pages of public folders' folders asked for: the folder (by id), and the `Offset`.
    pub public_asked: Vec<(String, usize)>,
}

impl ZoneScript {
    /// An Autodiscover request about the account, counted: whether it fails.
    pub(in crate::fakes::fake_ews) fn discovery_fails(&mut self) -> bool {
        self.discoveries = self.discoveries.saturating_add(1);
        let fails = self.discoveries_failing > 0;
        self.discoveries_failing = self.discoveries_failing.saturating_sub(1);
        fails
    }
}

/// Whether `list` names this mailbox.
fn names(list: &[String], address: &str) -> bool {
    list.iter().any(|a| a.eq_ignore_ascii_case(address))
}

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
                    if root && self.zones.public_refused {
                        out.push(err("ErrorAccessDenied"));
                        continue;
                    }
                    match self.folder_id(&f).and_then(|id| self.folders.iter().find(|x| x.id == id).cloned()) {
                        // The root is the hierarchy's; any other public folder, its content's.
                        Some(fo) if root && self.unrouted_hierarchy(&fo.id) => out.push(err("ErrorFolderNotFound")),
                        Some(fo) if !root && self.unrouted(&fo.id) => out.push(err("ErrorFolderNotFound")),
                        Some(fo) => out.push(ok(format!("<m:Folders>{}</m:Folders>", self.folder_xml(&fo)))),
                        None => out.push(err("ErrorFolderNotFound")),
                    }
                }
            }
            // The mailbox's own root (above the mail folders): its Common Views folder, named in
            // the mailbox's language.
            "FindFolder"
                if op.attr("Traversal") == Some("Shallow")
                    && op.child("ParentFolderIds").and_then(|p| p.kids.first()).is_some_and(|f| f.attr("Id") == Some("root")) =>
            {
                let name = if self.common_views_name.is_empty() { "Common Views" } else { &self.common_views_name };
                out.push(ok(format!(
                    "<m:RootFolder IncludesLastItemInRange=\"true\"><t:Folders><t:Folder><t:FolderId Id=\"COMMON-VIEWS\"/>\
                     <t:DisplayName>{}</t:DisplayName></t:Folder><t:Folder><t:FolderId Id=\"FINDER\"/><t:DisplayName>Finder</t:DisplayName>\
                     </t:Folder></t:Folders></m:RootFolder>",
                    esc(name)
                )));
            }
            // Shallow (public folders): the folders right under the one asked for, in the pages
            // asked for (or scripted).
            "FindFolder" if op.attr("Traversal") == Some("Shallow") => {
                let parent = op.child("ParentFolderIds").and_then(|p| p.kids.first()).and_then(|f| self.folder_id(f)).unwrap_or_default();
                if self.unrouted_hierarchy(&parent) {
                    out.push(err("ErrorFolderNotFound"));
                    return Some(out);
                }
                let view = op.child("IndexedPageFolderView");
                let offset: usize = view.and_then(|v| v.attr("Offset")).and_then(|o| o.parse().ok()).unwrap_or(0);
                let asked: usize = view.and_then(|v| v.attr("MaxEntriesReturned")).and_then(|o| o.parse().ok()).unwrap_or(1000);
                match self.public_page(&parent, offset, asked) {
                    Ok(page) => out.push(ok(page)),
                    Err(code) => out.push(err(code)),
                }
            }
            "FindFolder" if op.child("ParentFolderIds").and_then(|p| p.kids.first()).and_then(|f| self.shared_of(f)).is_some() => {
                // Deep, in a shared mailbox: refused without access to it.
                let Some(Ok(owner)) = op.child("ParentFolderIds").and_then(|p| p.kids.first()).and_then(|f| self.shared_of(f)) else {
                    out.push(err("ErrorAccessDenied"));
                    return Some(out);
                };
                if names(&self.zones.refused, &owner) {
                    out.push(err("ErrorAccessDenied"));
                    return Some(out);
                }
                // In the pages asked for (or scripted); a later one scripted unanswered has no
                // response message at all.
                let view = op.child("IndexedPageFolderView");
                let offset: usize = view.and_then(|v| v.attr("Offset")).and_then(|o| o.parse().ok()).unwrap_or(0);
                if offset > 0 && names(&self.zones.unanswered_pages, &owner) {
                    return Some(out);
                }
                let asked: usize = view.and_then(|v| v.attr("MaxEntriesReturned")).and_then(|o| o.parse().ok()).unwrap_or(1000);
                let max = asked.min(self.zones.shared_page.unwrap_or(usize::MAX)).max(1);
                let emptied = names(&self.zones.emptied, &owner);
                let all: Vec<&Fo> =
                    self.folders.iter().filter(|f| !emptied && f.owner == owner && f.known.as_deref() != Some("msgfolderroot")).collect();
                let page: String = all.iter().skip(offset).take(max).map(|f| self.folder_xml(f)).collect();
                let last = offset.saturating_add(max) >= all.len();
                out.push(ok(format!("<m:RootFolder IncludesLastItemInRange=\"{last}\"><t:Folders>{page}</t:Folders></m:RootFolder>")));
            }
            // Deep, in the online archive: refused without one, or by one that refuses.
            "FindFolder"
                if op.child("ParentFolderIds").and_then(|p| p.kids.first()).and_then(|f| f.attr("Id")).is_some_and(super::archive::is_archive_id) =>
            {
                match op.child("ParentFolderIds").and_then(|p| p.kids.first()).and_then(|f| self.archive_error(f)) {
                    Some(code) => out.push(err(code)),
                    None if self.zones.archive_emptied => {
                        out.push(ok("<m:RootFolder IncludesLastItemInRange=\"true\"><t:Folders></t:Folders></m:RootFolder>".into()))
                    }
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

impl Data {
    /// A page of the folders right under a public folder (`parent`, by id), from `offset`, `asked`
    /// at most: the answer's `RootFolder`, or the code it's refused with. As scripted: pages of
    /// [`ZoneScript::public_page`], one repeated, `Offset` ignored, a page refused, no end.
    fn public_page(&mut self, parent: &str, offset: usize, asked: usize) -> std::result::Result<String, &'static str> {
        self.zones.public_asked.push((parent.to_string(), offset));
        let max = asked.min(self.zones.public_page.unwrap_or(usize::MAX)).max(1);
        let page_number = (offset / max).saturating_add(1);
        if self.zones.public_failing.iter().any(|(p, from)| p == parent && page_number >= *from) {
            return Err("ErrorAccessDenied");
        }
        let folders =
            |page: String, last: bool| format!("<m:RootFolder IncludesLastItemInRange=\"{last}\"><t:Folders>{page}</t:Folders></m:RootFolder>");
        if self.zones.public_endless.iter().any(|p| p == parent) {
            let page: String = (offset..offset.saturating_add(max))
                .map(|n| {
                    format!(
                        "<t:Folder><t:FolderId Id=\"{parent}-E{n}\"/><t:ParentFolderId Id=\"{parent}\"/><t:FolderClass>IPF.Note</t:FolderClass>\
                         <t:DisplayName>Endless {n}</t:DisplayName><t:TotalCount>0</t:TotalCount><t:ChildFolderCount>0</t:ChildFolderCount>\
                         <t:UnreadCount>0</t:UnreadCount></t:Folder>"
                    )
                })
                .collect();
            return Ok(folders(page, false));
        }
        let emptied = self.zones.public_emptied && self.folders.iter().any(|f| f.id == parent && f.known.as_deref() == Some("publicfoldersroot"));
        let kids: Vec<&Fo> = self.folders.iter().filter(|f| !emptied && f.parent == parent).collect();
        let repeated = self.zones.public_repeated.iter().any(|p| p == parent);
        let from = if repeated || self.zones.public_offset_ignored.iter().any(|p| p == parent) { 0 } else { offset };
        let page: String = kids
            .iter()
            .skip(from)
            .take(max)
            .map(|f| {
                let n = self.folders.iter().filter(|x| x.parent == f.id).count();
                self.folder_xml(f).replace("<t:ChildFolderCount>0</t:ChildFolderCount>", &format!("<t:ChildFolderCount>{n}</t:ChildFolderCount>"))
            })
            .collect();
        let last = !repeated && offset.saturating_add(max) >= kids.len();
        Ok(folders(page, last))
    }
}
