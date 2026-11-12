// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The fake EWS server's archive and retention policies: the retention tags it offers
//! (`GetUserRetentionPolicyTags`), the archive tag each message and folder carries
//! (`item:ArchiveTag`, `folder:ArchiveTag`, the day it moves in `PidTagArchiveDate`) and the
//! retention tag (`item:PolicyTag`, `folder:PolicyTag`, the day it is deleted in
//! `item:RetentionDate`), written with `UpdateItem` / `UpdateFolder`.

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
/// Personal: delete after three years (a retention tag).
pub const TAG_DELETE: &str = "55555555-aaaa-4bbb-8ccc-000000000005";
/// Personal: delete for good after a week (a retention tag).
pub const TAG_WEEK: &str = "66666666-aaaa-4bbb-8ccc-000000000006";
/// Personal: delete after a month, hidden from the user.
pub const TAG_HIDDEN: &str = "77777777-aaaa-4bbb-8ccc-000000000007";
/// Personal: only marks mail past its limit (neither archives nor deletes).
pub const TAG_MARK: &str = "88888888-aaaa-4bbb-8ccc-000000000008";

/// The day the server plans to move a message given an archive tag with days.
pub const ARCHIVE_DAY: &str = "2027-11-10T00:00:00Z";
/// The day the server plans to delete a message given a retention tag with days.
pub const DELETE_DAY: &str = "2026-11-19T00:00:00Z";

/// `PidTagArchiveDate`, the day the server plans to move a message to the archive.
const ARCHIVE_DATE_TAG: &str = "0x301f";

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
    /// The archive tag a message carries of its own, and the day the server plans to move it,
    /// by item id.
    pub items: HashMap<String, (String, Option<String>)>,
    /// The archive tag a folder carries of its own, by folder id.
    pub folders: HashMap<String, String>,
    /// The retention tag a message carries of its own, and the day the server plans to delete
    /// it, by item id.
    pub retained: HashMap<String, (String, Option<String>)>,
    /// The retention tag a folder carries of its own, by folder id.
    pub retained_folders: HashMap<String, String>,
    /// Writes of a tag are refused (`ErrorAccessDenied`).
    pub refuse_writes: bool,
    /// Requests that named `item:ArchiveTag`, `item:PolicyTag` or `item:RetentionDate`, and
    /// listings of folders that named `folder:ArchiveTag` or `folder:PolicyTag`.
    pub item_reads: AtomicUsize,
    pub list_reads: AtomicUsize,
    /// Tag writes the server took in hand (accepted or not).
    pub writes: usize,
}

impl Data {
    /// The server offers the tags a default policy has: a default one, two personal archive
    /// tags (a year, five years), the personal "never", two personal delete tags (three years,
    /// a week for good), a hidden delete tag and one that only marks mail past its limit.
    pub fn offer_tags(&mut self) {
        let mut hidden = TagDef::new(TAG_HIDDEN, "1 Month Delete", "Personal", "DeleteAndAllowRecovery", Some(30));
        hidden.visible = false;
        self.policy.offered = Some(vec![
            TagDef::new(TAG_DEFAULT, "Default 2 years move to archive", "DefaultFolder", "MoveToArchive", Some(730)),
            TagDef::new(TAG_YEAR, "1 Year Move to Archive", "Personal", "MoveToArchive", Some(365)),
            TagDef::new(TAG_FIVE, "5 Year Move to Archive", "Personal", "MoveToArchive", Some(1825)),
            TagDef::new(TAG_NEVER, "Never Move to Archive", "Personal", "MoveToArchive", None),
            TagDef::new(TAG_DELETE, "3 Year Delete", "Personal", "DeleteAndAllowRecovery", Some(1095)),
            TagDef::new(TAG_WEEK, "1 Week Delete", "Personal", "PermanentlyDelete", Some(7)),
            hidden,
            TagDef::new(TAG_MARK, "1 Year Mark", "Personal", "MarkAsPastRetentionLimit", Some(365)),
        ]);
    }

    /// A message carrying an archive tag of its own, with the day the server plans to move it.
    pub fn tag_item(&mut self, id: &str, tag: &str, date: Option<&str>) {
        self.policy.items.insert(id.into(), (tag.into(), date.map(str::to_string)));
    }

    /// A message carrying a retention tag of its own, with the day the server plans to delete it.
    pub fn retain_item(&mut self, id: &str, tag: &str, date: Option<&str>) {
        self.policy.retained.insert(id.into(), (tag.into(), date.map(str::to_string)));
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
        let uris: [&str; 2] = if folder { ["folder:ArchiveTag", "folder:PolicyTag"] } else { ["item:ArchiveTag", "item:PolicyTag"] };
        let field = |u: &El| u.find("FieldURI").and_then(|f| f.attr("FieldURI")).and_then(|f| uris.iter().position(|x| *x == f));
        let tags = |ch: &El| -> Vec<(El, usize)> {
            ch.child("Updates").map(|u| u.kids.iter().filter_map(|u| Some((u.clone(), field(u)?))).collect()).unwrap_or_default()
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
            for (u, which) in tags(ch) {
                self.policy.writes += 1;
                let retention = which == 1;
                if u.name == "DeleteItemField" || u.name == "DeleteFolderField" {
                    match (folder, retention) {
                        (true, false) => drop(self.policy.folders.remove(&id)),
                        (true, true) => drop(self.policy.retained_folders.remove(&id)),
                        (false, false) => drop(self.policy.items.remove(&id)),
                        (false, true) => drop(self.policy.retained.remove(&id)),
                    }
                    continue;
                }
                let Some(tag) = u.find(if retention { "PolicyTag" } else { "ArchiveTag" }) else { continue };
                let wanted = tag.text.trim().to_string();
                // The tag must be one the server offers, of the field's kind, written as an
                // explicit one.
                let def = self.policy.offered.as_ref().and_then(|t| t.iter().find(|t| t.id == wanted)).cloned();
                let fits = def.as_ref().is_some_and(|d| (d.action == "MoveToArchive") != retention);
                if tag.attr("IsExplicit") != Some("true") || !fits {
                    out.push(err("ErrorInvalidRequest"));
                    continue;
                }
                let day = if retention { DELETE_DAY } else { ARCHIVE_DAY };
                let date = def.and_then(|d| d.period).map(|_| day.to_string());
                match (folder, retention) {
                    (true, false) => drop(self.policy.folders.insert(id.clone(), wanted)),
                    (true, true) => drop(self.policy.retained_folders.insert(id.clone(), wanted)),
                    (false, false) => drop(self.policy.items.insert(id.clone(), (wanted, date))),
                    (false, true) => drop(self.policy.retained.insert(id.clone(), (wanted, date))),
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

    /// An item's answer with its policy tags and planned days, when the request named them: the
    /// archive tag, its day (`PidTagArchiveDate`, asked as an extended property), the retention
    /// tag and its day (`item:RetentionDate`).
    pub(super) fn with_item_policy(&self, op: &El, id: &str, xml: String) -> String {
        let named = fields_named(op);
        let archive_day = asks_archive_date(op);
        if !archive_day && !named.iter().any(|f| matches!(*f, "item:ArchiveTag" | "item:RetentionDate" | "item:PolicyTag")) {
            return xml;
        }
        self.policy.item_reads.fetch_add(1, Ordering::Relaxed);
        let mut add = String::new();
        if let Some((tag, date)) = self.policy.retained.get(id) {
            if named.contains(&"item:PolicyTag") {
                add += &format!("<t:PolicyTag IsExplicit=\"true\">{}</t:PolicyTag>", esc(tag));
            }
            if let Some(d) = date.as_ref().filter(|_| named.contains(&"item:RetentionDate")) {
                add += &format!("<t:RetentionDate>{}</t:RetentionDate>", esc(d));
            }
        }
        if let Some((tag, date)) = self.policy.items.get(id) {
            if named.contains(&"item:ArchiveTag") {
                add += &format!("<t:ArchiveTag IsExplicit=\"true\">{}</t:ArchiveTag>", esc(tag));
            }
            if let Some(d) = date.as_ref().filter(|_| archive_day) {
                add += &format!(
                    "<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"{ARCHIVE_DATE_TAG}\" PropertyType=\"SystemTime\"/>\
                     <t:Value>{}</t:Value></t:ExtendedProperty>",
                    esc(d)
                );
            }
        }
        match xml.rsplit_once("</t:Message>") {
            Some((head, tail)) => format!("{head}{add}</t:Message>{tail}"),
            None => xml,
        }
    }

    /// A folder's answer with its policy tags, when the request named them.
    pub(super) fn with_folder_policy(&self, op: &El, id: &str, xml: String) -> String {
        let named = fields_named(op);
        let (archive, retention) = (named.contains(&"folder:ArchiveTag"), named.contains(&"folder:PolicyTag"));
        if !archive && !retention {
            return xml;
        }
        self.policy.list_reads.fetch_add(1, Ordering::Relaxed);
        let mut add = String::new();
        if let Some(tag) = self.policy.retained_folders.get(id).filter(|_| retention) {
            add += &format!("<t:PolicyTag IsExplicit=\"true\">{}</t:PolicyTag>", esc(tag));
        }
        if let Some(tag) = self.policy.folders.get(id).filter(|_| archive) {
            add += &format!("<t:ArchiveTag IsExplicit=\"true\">{}</t:ArchiveTag>", esc(tag));
        }
        match xml.rfind("</t:") {
            Some(at) => format!("{}{add}{}", &xml[..at], &xml[at..]),
            None => xml,
        }
    }
}

/// Whether a `GetItem` asks for `PidTagArchiveDate` (an extended property, by its tag).
fn asks_archive_date(op: &El) -> bool {
    op.find("AdditionalProperties").is_some_and(|a| {
        a.kids.iter().any(|f| f.name == "ExtendedFieldURI" && f.attr("PropertyTag").is_some_and(|t| t.eq_ignore_ascii_case(ARCHIVE_DATE_TAG)))
    })
}
