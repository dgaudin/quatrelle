// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The fake EWS server's archive policy: the retention tags it offers (`GetUserRetentionPolicyTags`)
//! and the archive tag each message and folder carries (`item:ArchiveTag`, `folder:ArchiveTag`,
//! written with `UpdateItem` / `UpdateFolder`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

/// A default tag of the mailbox's policy (not a personal one).
pub const TAG_DEFAULT: &str = "11111111-aaaa-4bbb-8ccc-000000000001";
/// Personal: move to the archive after a year.
pub const TAG_YEAR: &str = "22222222-aaaa-4bbb-8ccc-000000000002";
/// Personal: move to the archive after five years.
pub const TAG_FIVE: &str = "33333333-aaaa-4bbb-8ccc-000000000003";
/// Personal: never move to the archive.
pub const TAG_NEVER: &str = "44444444-aaaa-4bbb-8ccc-000000000004";
/// Personal: delete after three years (not an archive tag).
pub const TAG_DELETE: &str = "55555555-aaaa-4bbb-8ccc-000000000005";

/// One retention policy tag as the server lists it.
#[derive(Clone, Debug)]
pub struct TagDef {
    pub id: String,
    pub name: String,
    /// `Personal`, `DefaultFolder`…
    pub kind: String,
    /// `MoveToArchive`, `DeleteAndAllowRecovery`…
    pub action: String,
    /// `RetentionPeriod` in days (none: unlimited).
    pub period: Option<u32>,
    pub visible: bool,
}

impl TagDef {
    pub fn new(id: &str, name: &str, kind: &str, action: &str, period: Option<u32>) -> TagDef {
        TagDef { id: id.into(), name: name.into(), kind: kind.into(), action: action.into(), period, visible: true }
    }
}

/// What the server holds of the archive policy.
#[derive(Default)]
pub struct Policy {
    /// The tags `GetUserRetentionPolicyTags` lists; none: the server doesn't know the operation
    /// (older than Exchange 2013) and refuses it.
    pub offered: Option<Vec<TagDef>>,
    /// An answer written by hand instead (a hostile server): the content of the response element.
    pub raw: Option<String>,
    /// The tag a message carries of its own, and the day the server plans to move it, by item id.
    pub items: HashMap<String, (String, Option<String>)>,
    /// The tag a folder carries of its own, by folder id.
    pub folders: HashMap<String, String>,
    /// Writes of a tag are refused (`ErrorAccessDenied`).
    pub refuse_writes: bool,
    /// Requests that named `item:ArchiveTag` or `item:RetentionDate`, and listings of folders
    /// that named `folder:ArchiveTag`.
    pub item_reads: AtomicUsize,
    pub list_reads: AtomicUsize,
    /// Tag writes the server took in hand (accepted or not).
    pub writes: usize,
}

impl Data {
    /// The server offers the tags Outlook's default policy has: a default one, two personal
    /// archive tags (a year, five years), the personal "never", and a personal delete tag.
    pub fn offer_tags(&mut self) {
        self.policy.offered = Some(vec![
            TagDef::new(TAG_DEFAULT, "Default 2 years move to archive", "DefaultFolder", "MoveToArchive", Some(730)),
            TagDef::new(TAG_YEAR, "1 Year Move to Archive", "Personal", "MoveToArchive", Some(365)),
            TagDef::new(TAG_FIVE, "5 Year Move to Archive", "Personal", "MoveToArchive", Some(1825)),
            TagDef::new(TAG_NEVER, "Never Move to Archive", "Personal", "MoveToArchive", None),
            TagDef::new(TAG_DELETE, "3 Year Delete", "Personal", "DeleteAndAllowRecovery", Some(1095)),
        ]);
    }

    /// A message carrying a tag of its own, with the day the server plans to move it.
    pub fn tag_item(&mut self, id: &str, tag: &str, date: Option<&str>) {
        self.policy.items.insert(id.into(), (tag.into(), date.map(str::to_string)));
    }

    /// The server moves a message to a folder of the archive: it has another id there, and
    /// the id it had is gone.
    pub fn move_to_archive(&mut self, id: &str, folder: &str) -> Option<String> {
        let new = self.id("I");
        let it = self.items.iter_mut().find(|i| i.id == id)?;
        it.id = new.clone();
        it.folder = folder.into();
        self.policy.items.remove(id);
        Some(new)
    }

    /// `GetUserRetentionPolicyTags`, `UpdateItem` and `UpdateFolder` when they carry an archive
    /// tag; anything else is left to the other operations.
    pub(super) fn policy_route(&mut self, op: &El) -> Option<(String, Vec<String>)> {
        let ok = |inner: String| format!("<m:R ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode>{inner}</m:R>");
        let err =
            |code: &str| format!("<m:R ResponseClass=\"Error\"><m:MessageText>{code}</m:MessageText><m:ResponseCode>{code}</m:ResponseCode></m:R>");
        let mut out = Vec::new();
        match op.name.as_str() {
            "GetUserRetentionPolicyTags" => {
                let inner = match (&self.policy.raw, &self.policy.offered) {
                    (Some(raw), _) => raw.clone(),
                    (None, None) => {
                        "<m:ResponseCode>ErrorInvalidServerVersion</m:ResponseCode><m:MessageText>not on this server</m:MessageText>".into()
                    }
                    (None, Some(tags)) => {
                        let list: String = tags
                            .iter()
                            .map(|t| {
                                let period = t.period.map(|p| format!("<t:RetentionPeriod>{p}</t:RetentionPeriod>")).unwrap_or_default();
                                format!(
                                    "<t:RetentionPolicyTag><t:DisplayName>{}</t:DisplayName><t:RetentionId>{}</t:RetentionId>{period}\
                                     <t:Type>{}</t:Type><t:RetentionAction>{}</t:RetentionAction><t:Description>d</t:Description>\
                                     <t:IsVisible>{}</t:IsVisible><t:OptedInto>true</t:OptedInto><t:IsArchive>{}</t:IsArchive></t:RetentionPolicyTag>",
                                    esc(&t.name),
                                    esc(&t.id),
                                    esc(&t.kind),
                                    esc(&t.action),
                                    t.visible,
                                    t.action == "MoveToArchive"
                                )
                            })
                            .collect();
                        format!("<m:ResponseCode>NoError</m:ResponseCode><m:RetentionPolicyTags>{list}</m:RetentionPolicyTags>")
                    }
                };
                return Some(("GetUserRetentionPolicyTagsResponse".into(), vec![inner]));
            }
            "UpdateItem" | "UpdateFolder" => {}
            _ => return None,
        }
        let folder = op.name == "UpdateFolder";
        let changes = op.child(if folder { "FolderChanges" } else { "ItemChanges" })?;
        let uri = if folder { "folder:ArchiveTag" } else { "item:ArchiveTag" };
        let tags = |ch: &El| -> Vec<El> {
            ch.child("Updates")
                .map(|u| u.kids.iter().filter(|u| u.find("FieldURI").and_then(|f| f.attr("FieldURI")) == Some(uri)).cloned().collect())
                .unwrap_or_default()
        };
        if !changes.kids.iter().any(|ch| !tags(ch).is_empty()) {
            return None;
        }
        for ch in &changes.kids {
            let id = ch.child(if folder { "FolderId" } else { "ItemId" }).and_then(|i| i.attr("Id")).unwrap_or_default().to_string();
            let known = if folder { self.folders.iter().any(|f| f.id == id) } else { self.items.iter().any(|i| i.id == id) };
            if !known {
                out.push(err(if folder { "ErrorFolderNotFound" } else { "ErrorItemNotFound" }));
                continue;
            }
            if self.policy.refuse_writes {
                out.push(err("ErrorAccessDenied"));
                continue;
            }
            for u in tags(ch) {
                self.policy.writes += 1;
                if u.name == "DeleteItemField" || u.name == "DeleteFolderField" {
                    if folder {
                        self.policy.folders.remove(&id);
                    } else {
                        self.policy.items.remove(&id);
                    }
                    continue;
                }
                let Some(tag) = u.find("ArchiveTag") else { continue };
                let wanted = tag.text.trim().to_string();
                // The tag must be one the server offers, written as an explicit one.
                let def = self.policy.offered.as_ref().and_then(|t| t.iter().find(|t| t.id == wanted)).cloned();
                if tag.attr("IsExplicit") != Some("true") || def.is_none() {
                    out.push(err("ErrorInvalidRequest"));
                    continue;
                }
                if folder {
                    self.policy.folders.insert(id.clone(), wanted);
                } else {
                    let date = def.and_then(|d| d.period).map(|_| "2027-11-10T00:00:00Z".to_string());
                    self.policy.items.insert(id.clone(), (wanted, date));
                }
            }
            let (kind, id_el) = if folder { ("Folder", "FolderId") } else { ("Message", "ItemId") };
            out.push(ok(format!(
                "<m:{}s><t:{kind}><t:{id_el} Id=\"{id}\" ChangeKey=\"ck2\"/></t:{kind}></m:{}s>",
                if folder { "Folder" } else { "Item" },
                if folder { "Folder" } else { "Item" }
            )));
        }
        Some((format!("{}Response", op.name), out))
    }

    /// An item's answer with its archive tag and planned day, when the request named them.
    pub(super) fn with_item_policy(&self, op: &El, id: &str, xml: String) -> String {
        let named = fields_named(op);
        if !named.iter().any(|f| matches!(*f, "item:ArchiveTag" | "item:RetentionDate")) {
            return xml;
        }
        self.policy.item_reads.fetch_add(1, Ordering::Relaxed);
        let Some((tag, date)) = self.policy.items.get(id) else { return xml };
        let date = date.as_ref().map(|d| format!("<t:RetentionDate>{}</t:RetentionDate>", esc(d))).unwrap_or_default();
        match xml.rsplit_once("</t:Message>") {
            Some((head, tail)) => format!("{head}<t:ArchiveTag IsExplicit=\"true\">{}</t:ArchiveTag>{date}</t:Message>{tail}", esc(tag)),
            None => xml,
        }
    }

    /// A folder's answer with its archive tag, when the request named it.
    pub(super) fn with_folder_policy(&self, op: &El, id: &str, xml: String) -> String {
        if !fields_named(op).contains(&"folder:ArchiveTag") {
            return xml;
        }
        self.policy.list_reads.fetch_add(1, Ordering::Relaxed);
        let Some(tag) = self.policy.folders.get(id) else { return xml };
        match xml.rfind("</t:") {
            Some(at) => format!("{}<t:ArchiveTag IsExplicit=\"true\">{}</t:ArchiveTag>{}", &xml[..at], esc(tag), &xml[at..]),
            None => xml,
        }
    }
}
