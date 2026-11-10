// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! EWS: the archive policy tags. The server offers its personal retention tags
//! (`GetUserRetentionPolicyTags`); a message carries one in `item:ArchiveTag` (with the day the
//! server plans to move it in `item:RetentionDate`), a folder in `folder:ArchiveTag`. All of it
//! is read as hostile: ids are GUIDs, names are bounded, the list is capped.

use quatrelle_model::job::is_day;
use quatrelle_model::{ArchiveTag, MAX_ARCHIVE_TAGS, MAX_TAG_DAYS, is_tag_id};

use super::*;

/// How many elements of a tag list are looked at, kept or not.
const SCANNED: usize = 5_000;

/// The personal archive tags a `GetUserRetentionPolicyTags` answer lists: those of type
/// `Personal` whose action is `MoveToArchive`, the one that keeps mail out of the archive among
/// them (no period). Others (the mailbox's default tags, delete tags) are left out, as are tags
/// without a GUID for an id or without a name, hidden ones, and what goes past
/// [`MAX_ARCHIVE_TAGS`].
pub fn tags_listed(root: &El) -> Vec<ArchiveTag> {
    let mut all = Vec::new();
    root.all("RetentionPolicyTag", &mut all);
    let mut out: Vec<ArchiveTag> = Vec::new();
    for t in all.into_iter().take(SCANNED) {
        if out.len() >= MAX_ARCHIVE_TAGS {
            break;
        }
        let is = |name: &str, want: &str| t.text_of(name).is_some_and(|v| v.trim().eq_ignore_ascii_case(want));
        if !is("Type", "Personal") || !is("RetentionAction", "MoveToArchive") || is("IsVisible", "false") {
            continue;
        }
        let id = t.text_of("RetentionId").map(|i| i.trim().to_ascii_lowercase()).unwrap_or_default();
        let name = clean_name(t.text_of("DisplayName").unwrap_or_default());
        if !is_tag_id(&id) || name.is_empty() || out.iter().any(|o| o.id == id) {
            continue;
        }
        // No period, or none that counts: the tag that keeps mail where it is.
        let period = match t.text_of("RetentionPeriod").map(str::trim).filter(|p| !p.is_empty()) {
            None => None,
            Some(p) => match p.parse::<u32>() {
                Ok(days) if days <= MAX_TAG_DAYS => Some(days),
                _ => continue,
            },
        };
        let never = period.is_none_or(|d| d == 0);
        out.push(ArchiveTag { id, name, days: period.filter(|d| *d > 0), never });
    }
    out
}

/// The archive tag a message or a folder carries of its own (`t:ArchiveTag`): its id, when it is
/// a GUID and was set explicitly rather than inherited.
pub(in super::super) fn own_tag(item: &El) -> Option<String> {
    let tag = item.child("ArchiveTag")?;
    if tag.attr("IsExplicit").is_some_and(|v| v.trim().eq_ignore_ascii_case("false")) {
        return None;
    }
    let id = tag.text.trim().to_ascii_lowercase();
    is_tag_id(&id).then_some(id)
}

/// The day (`YYYY-MM-DD`) in a `RetentionDate` value, when it is a real one.
fn planned_day(item: &El) -> Option<String> {
    let day: String = item.text_of("RetentionDate")?.trim().chars().take(10).collect();
    is_day(&day).then_some(day)
}

impl Client {
    /// The server's personal archive tags, asked once per session: none from a server that has
    /// none or won't say (Exchange 2010 doesn't know the operation); a connection that fails is
    /// an error, and asked again.
    pub fn archive_tags(&self) -> Result<Vec<ArchiveTag>> {
        if let Some(known) = self.tags.borrow().clone() {
            return Ok(known);
        }
        let tags = match self.call("<m:GetUserRetentionPolicyTags/>") {
            Ok(doc) => tags_listed(&doc),
            Err(NetError::No(e) | NetError::Protocol(e)) => {
                log::info!("no archive policy tags: {e}");
                Vec::new()
            }
            Err(e) => return Err(e),
        };
        *self.tags.borrow_mut() = Some(tags.clone());
        Ok(tags)
    }

    /// A message's own archive tag and the day the server plans to move it, as `YYYY-MM-DD`.
    pub fn item_archive_tag(&self, id: &str) -> Result<(Option<String>, Option<String>)> {
        let m = self.call_one(&format!(
            "<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties><t:FieldURI FieldURI=\"item:ArchiveTag\"/>\
             <t:FieldURI FieldURI=\"item:RetentionDate\"/></t:AdditionalProperties></m:ItemShape>\
             <m:ItemIds><t:ItemId Id=\"{}\"/></m:ItemIds></m:GetItem>",
            esc(id)
        ))?;
        let item = m.find("Items").and_then(|i| i.kids.first());
        Ok(match item {
            Some(it) => (own_tag(it), planned_day(it)),
            None => (None, None),
        })
    }

    /// Whether the account's folder listing should ask each folder's tag: only a server that
    /// offers tags (Exchange 2013 and later) knows the field.
    pub(in super::super) fn folder_tags_asked(&self) -> bool {
        self.archive_tags().is_ok_and(|t| !t.is_empty())
    }
}
