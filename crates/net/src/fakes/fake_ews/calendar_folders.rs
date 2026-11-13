// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The fake EWS server's calendar folders under the account's Calendar, as Exchange keeps them:
//! listed (`FindFolder`, deep, under `calendar`), made (`CreateFolder` of a `CalendarFolder`, a
//! name taken refused), renamed (`UpdateFolder`), deleted into Deleted Items with their
//! appointments (`DeleteFolder`, the Calendar itself refused); and those the server refuses to
//! read (`Data::refused_folders`).

use super::*;

fn ok(inner: String) -> String {
    format!("<m:R ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode>{inner}</m:R>")
}

fn err(code: &str) -> String {
    format!("<m:R ResponseClass=\"Error\"><m:MessageText>{code}</m:MessageText><m:ResponseCode>{code}</m:ResponseCode></m:R>")
}

impl Data {
    /// The account's own Calendar folder.
    fn own_calendar(&self) -> Option<String> {
        self.folders.iter().find(|f| f.known.as_deref() == Some("calendar") && f.owner.is_empty()).map(|f| f.id.clone())
    }

    /// Every folder under `parent`, at any depth, in the order they were made.
    fn under(&self, parent: &str) -> Vec<Fo> {
        let mut ids = vec![parent.to_string()];
        let mut i = 0;
        while let Some(id) = ids.get(i).cloned() {
            ids.extend(self.folders.iter().filter(|f| f.parent == id).map(|f| f.id.clone()));
            i += 1;
        }
        self.folders.iter().filter(|f| f.id != parent && ids.contains(&f.id)).cloned().collect()
    }

    /// A folder of appointments (its class one of `IPF.Appointment`'s).
    fn is_calendar_folder(&self, id: &str) -> bool {
        self.folders.iter().any(|f| f.id == id && f.class.starts_with("IPF.Appointment"))
    }

    /// The operations on calendar folders, answered; `None` for any other, which `route` answers.
    pub(super) fn calendar_folders(&mut self, op: &El) -> Option<Vec<String>> {
        let first = |name: &str| op.child(name).and_then(|p| p.kids.first()).and_then(|f| self.folder_id(f));
        let calendar = self.own_calendar();
        match op.name.as_str() {
            "SyncFolderItems" | "FindItem" | "GetFolder" => {
                let named = first("SyncFolderId").or_else(|| first("ParentFolderIds")).or_else(|| first("FolderIds"))?;
                self.refused_folders.contains(&named).then(|| vec![err("ErrorAccessDenied")])
            }
            "FindFolder" if op.attr("Traversal") == Some("Deep") && calendar.is_some() && first("ParentFolderIds") == calendar => {
                let max: usize =
                    op.child("IndexedPageFolderView").and_then(|v| v.attr("MaxEntriesReturned")).and_then(|m| m.parse().ok()).unwrap_or(1000);
                let all = self.under(calendar.as_deref().unwrap_or_default());
                let listed: String = all
                    .iter()
                    .take(max)
                    .map(|f| {
                        let kind = if f.class.starts_with("IPF.Appointment") { "CalendarFolder" } else { "Folder" };
                        format!(
                            "<t:{kind}><t:FolderId Id=\"{}\"/><t:FolderClass>{}</t:FolderClass><t:DisplayName>{}</t:DisplayName></t:{kind}>",
                            f.id,
                            esc(&f.class),
                            esc(&f.name)
                        )
                    })
                    .collect();
                let last = all.len() <= max;
                Some(vec![ok(format!(
                    "<m:RootFolder TotalItemsInView=\"{}\" IncludesLastItemInRange=\"{last}\"><t:Folders>{listed}</t:Folders></m:RootFolder>",
                    all.len()
                ))])
            }
            "CreateFolder" if calendar.is_some() && first("ParentFolderId") == calendar => {
                let parent = calendar.unwrap_or_default();
                let mut out = Vec::new();
                for f in op.child("Folders").map(|f| f.kids.clone()).unwrap_or_default() {
                    let name = f.text_of("DisplayName").unwrap_or_default().to_string();
                    let class = f.text_of("FolderClass").map(str::to_string).unwrap_or_else(|| "IPF.Appointment".into());
                    if self.folders.iter().any(|x| x.parent == parent && x.name.eq_ignore_ascii_case(&name)) {
                        out.push(err("ErrorFolderExists"));
                        continue;
                    }
                    let id = self.folder(&name, Some(&parent), None, &class);
                    out.push(ok(format!("<m:Folders><t:CalendarFolder><t:FolderId Id=\"{id}\"/></t:CalendarFolder></m:Folders>")));
                }
                Some(out)
            }
            // A calendar folder renamed (its new name a `CalendarFolder`'s): one not there, or a
            // name taken beside it, refused.
            "UpdateFolder" if op.find("CalendarFolder").is_some() => {
                let mut out = Vec::new();
                for ch in op.child("FolderChanges").map(|c| c.kids.clone()).unwrap_or_default() {
                    let id = ch.child("FolderId").and_then(|i| i.attr("Id")).unwrap_or_default().to_string();
                    let name = ch.find("DisplayName").map(|d| d.text.clone()).unwrap_or_default();
                    let Some(parent) = self.folders.iter().find(|f| f.id == id).map(|f| f.parent.clone()) else {
                        out.push(err("ErrorFolderNotFound"));
                        continue;
                    };
                    if self.folders.iter().any(|x| x.parent == parent && x.id != id && x.name.eq_ignore_ascii_case(&name)) {
                        out.push(err("ErrorFolderExists"));
                        continue;
                    }
                    if let Some(f) = self.folders.iter_mut().find(|f| f.id == id) {
                        f.name = name;
                    }
                    out.push(ok(String::new()));
                }
                Some(out)
            }
            // A calendar folder deleted: into Deleted Items with its appointments (asked so), the
            // Calendar itself refused, one not there said so.
            "DeleteFolder" => {
                let ids: Vec<String> =
                    op.child("FolderIds").map(|f| f.kids.iter().filter_map(|k| k.attr("Id").map(str::to_string)).collect()).unwrap_or_default();
                if !ids.iter().any(|id| self.is_calendar_folder(id) || !self.folders.iter().any(|f| f.id == *id)) {
                    return None;
                }
                let bin = self.folders.iter().find(|f| f.known.as_deref() == Some("deleteditems") && f.owner.is_empty()).map(|f| f.id.clone());
                let moved = op.attr("DeleteType") == Some("MoveToDeletedItems");
                let mut out = Vec::new();
                for id in ids {
                    if Some(&id) == calendar.as_ref() {
                        out.push(err("ErrorDeleteDistinguishedFolder"));
                        continue;
                    }
                    if !self.folders.iter().any(|f| f.id == id) {
                        out.push(err("ErrorFolderNotFound"));
                        continue;
                    }
                    match (&bin, moved) {
                        (Some(bin), true) => {
                            if let Some(f) = self.folders.iter_mut().find(|f| f.id == id) {
                                f.parent = bin.clone();
                            }
                        }
                        _ => {
                            let gone: Vec<String> = self.under(&id).into_iter().map(|f| f.id).chain([id.clone()]).collect();
                            self.folders.retain(|f| !gone.contains(&f.id));
                            self.pim.retain(|(_, f, _)| !gone.contains(f));
                        }
                    }
                    out.push(ok(String::new()));
                }
                Some(out)
            }
            _ => None,
        }
    }
}
