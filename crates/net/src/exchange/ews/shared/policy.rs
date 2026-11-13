// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! EWS: the archive and retention policy tags. The server offers its personal retention tags
//! (`GetUserRetentionPolicyTags`), some of which move mail to the archive and some delete it. A
//! message carries an archive tag in `item:ArchiveTag` (the day the server plans to move it in
//! `PidTagArchiveDate`) and a retention tag in `item:PolicyTag` (the day the server plans to
//! delete it in `item:RetentionDate`); a folder in `folder:ArchiveTag` and `folder:PolicyTag`.
//! All of it is read as hostile: ids are GUIDs, names are bounded and on one line, the list is
//! capped.

use quatrelle_model::job::is_day;
use quatrelle_model::{ArchiveTag, MAX_ARCHIVE_TAGS, MAX_TAG_DAYS, PolicyKind, is_tag_id};

use super::*;

/// How many elements of a tag list are looked at, kept or not.
const SCANNED: usize = 5_000;

/// `PidTagArchiveDate`: the day the server plans to move a message to the archive.
const ARCHIVE_DATE: &str = "<t:ExtendedFieldURI PropertyTag=\"0x301F\" PropertyType=\"SystemTime\"/>";

/// The personal tags a `GetUserRetentionPolicyTags` answer lists, those of type `Personal`:
/// archive tags, which `IsArchive` says whatever their action, or whose action is
/// `MoveToArchive`; retention tags, whose action deletes the mail (`DeleteAndAllowRecovery`,
/// `PermanentlyDelete`); and the disabled ones (action `None`), which never act: Exchange's
/// "never" tags, an archive one when `IsArchive` says so, a retention one when it says not.
/// "Never" is also a tag with no period, or a period of 0; a disabled one has no other. Others
/// (the mailbox's default tags, those that only mark mail past its limit) are left out, as are
/// tags without a GUID for an id or without a name, hidden ones, an id seen before, an enabled
/// one whose period isn't a number of days up to [`MAX_TAG_DAYS`], and what goes past
/// [`MAX_ARCHIVE_TAGS`].
pub fn tags_listed(root: &El) -> Vec<ArchiveTag> {
    tags_read(root).0
}

/// How many of the personal tags left out are told in the log, each with its reason; the rest
/// are counted.
const LEFT_OUT_TOLD: usize = 20;

/// A personal tag left out of the list, and why: what the server says of it, cleaned as names
/// are and bounded (nothing else of it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LeftOut {
    pub(crate) reason: &'static str,
    pub(crate) name: String,
    pub(crate) action: String,
    pub(crate) period: String,
    pub(crate) is_archive: String,
    pub(crate) visible: String,
}

impl std::fmt::Display for LeftOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "archive policy: the personal tag {:?} is left out ({}): RetentionAction {:?}, RetentionPeriod {:?}, IsArchive {:?}, IsVisible {:?}",
            self.name, self.reason, self.action, self.period, self.is_archive, self.visible
        )
    }
}

/// [`tags_listed`], with the personal tags left out and why (the first [`LEFT_OUT_TOLD`]) and
/// how many more were.
pub(crate) fn tags_read(root: &El) -> (Vec<ArchiveTag>, Vec<LeftOut>, usize) {
    let mut all = Vec::new();
    root.all("RetentionPolicyTag", &mut all);
    let (mut out, mut left, mut more) = (Vec::new(), Vec::new(), 0usize);
    for t in all.into_iter().take(SCANNED) {
        // The mailbox's default tags aren't the user's to choose: not told.
        if !t.text_of("Type").is_some_and(|v| v.trim().eq_ignore_ascii_case("Personal")) {
            continue;
        }
        match personal_tag(t, &out) {
            Ok(tag) => out.push(tag),
            Err(reason) if left.len() < LEFT_OUT_TOLD => left.push(LeftOut {
                reason,
                name: tag_name(t.text_of("DisplayName").unwrap_or_default()),
                action: told(t, "RetentionAction"),
                period: told(t, "RetentionPeriod"),
                is_archive: told(t, "IsArchive"),
                visible: told(t, "IsVisible"),
            }),
            Err(_) => more += 1,
        }
    }
    (out, left, more)
}

/// A field of a tag as the log tells it: cleaned as names are, a few characters at most.
fn told(t: &El, name: &str) -> String {
    match t.text_of(name) {
        Some(v) => tag_name(v).chars().take(40).collect(),
        None => "(none)".into(),
    }
}

/// A personal tag as it's listed, or why it isn't (`listed`: the ones kept before it).
fn personal_tag(t: &El, listed: &[ArchiveTag]) -> std::result::Result<ArchiveTag, &'static str> {
    let is = |name: &str, want: &str| t.text_of(name).is_some_and(|v| v.trim().eq_ignore_ascii_case(want));
    if listed.len() >= MAX_ARCHIVE_TAGS {
        return Err("past the most tags listed");
    }
    if is("RetentionAction", "MarkAsPastRetentionLimit") {
        return Err("it only marks mail past its limit");
    }
    let disabled = is("RetentionAction", "None");
    let kind = if is("IsArchive", "true") || is("RetentionAction", "MoveToArchive") {
        PolicyKind::Archive
    } else if is("RetentionAction", "DeleteAndAllowRecovery") || is("RetentionAction", "PermanentlyDelete") || (disabled && is("IsArchive", "false"))
    {
        PolicyKind::Retention
    } else if disabled {
        return Err("disabled, and IsArchive doesn't say whether it archives");
    } else {
        return Err("its action neither archives nor deletes");
    };
    if is("IsVisible", "false") {
        return Err("hidden from the user");
    }
    let id = t.text_of("RetentionId").map(|i| i.trim().to_ascii_lowercase()).unwrap_or_default();
    if !is_tag_id(&id) {
        return Err("its id isn't a GUID");
    }
    let name = tag_name(t.text_of("DisplayName").unwrap_or_default());
    if name.is_empty() {
        return Err("it has no name");
    }
    if listed.iter().any(|o| o.id == id) {
        return Err("its id is listed already");
    }
    // A disabled tag never acts, whatever period it says. Otherwise no period, or none that
    // counts: the tag that keeps mail where it is (out of the archive, or never deleted).
    let period = match t.text_of("RetentionPeriod").map(str::trim).filter(|p| !p.is_empty() && !disabled) {
        None => None,
        Some(p) => match p.parse::<u32>() {
            Ok(days) if days <= MAX_TAG_DAYS => Some(days),
            _ => return Err("its period isn't a number of days the list takes"),
        },
    };
    let never = period.is_none_or(|d| d == 0);
    Ok(ArchiveTag { id, name, days: period.filter(|d| *d > 0), never, kind })
}

/// A tag's name as it's shown: on one line (each run of whitespace one space), without control
/// or bidi characters, at most [`quatrelle_model::MAX_TAG_NAME`] characters.
fn tag_name(raw: &str) -> String {
    let words =
        raw.split_whitespace().map(|w| w.chars().filter(|c| !c.is_control() && !quatrelle_model::pim::teams::is_bidi(*c)).collect::<String>());
    let line = words.filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ");
    line.chars().take(quatrelle_model::MAX_TAG_NAME).collect::<String>().trim_end().to_string()
}

/// The element a message or a folder carries a tag of this kind in.
fn element(kind: PolicyKind) -> &'static str {
    match kind {
        PolicyKind::Archive => "ArchiveTag",
        PolicyKind::Retention => "PolicyTag",
    }
}

/// The tag of this kind a message or a folder carries of its own (`t:ArchiveTag`,
/// `t:PolicyTag`): its id, when it is a GUID and was set explicitly rather than inherited.
pub(in super::super) fn own_tag(item: &El, kind: PolicyKind) -> Option<String> {
    let tag = item.child(element(kind))?;
    if tag.attr("IsExplicit").is_some_and(|v| v.trim().eq_ignore_ascii_case("false")) {
        return None;
    }
    let id = tag.text.trim().to_ascii_lowercase();
    is_tag_id(&id).then_some(id)
}

/// The day (`YYYY-MM-DD`) in a date the server gives, when it is a real one.
fn day_of(text: &str) -> Option<String> {
    let day: String = text.trim().chars().take(10).collect();
    is_day(&day).then_some(day)
}

/// The day the server plans to move a message to the archive (`PidTagArchiveDate`).
fn archive_day(item: &El) -> Option<String> {
    let prop = item
        .kids
        .iter()
        .filter(|p| p.name == "ExtendedProperty")
        .find(|p| p.child("ExtendedFieldURI").and_then(|u| u.attr("PropertyTag")).is_some_and(|t| crate::ews::is_tag(t, "0x301F")))?;
    day_of(prop.text_of("Value")?)
}

/// What a message carries of its own: its archive tag and the day the server plans to move it,
/// its retention tag and the day the server plans to delete it (`YYYY-MM-DD`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ItemPolicy {
    pub archive: Option<String>,
    pub archive_day: Option<String>,
    pub retention: Option<String>,
    pub retention_day: Option<String>,
}

impl Client {
    /// The server's personal archive and retention tags, asked once per session: none from a
    /// server that has none or won't say (Exchange 2010 doesn't know the operation); a
    /// connection that fails is an error, and asked again.
    pub fn archive_tags(&self) -> Result<Vec<ArchiveTag>> {
        if let Some(known) = self.tags.borrow().clone() {
            return Ok(known);
        }
        let tags = match self.call("<m:GetUserRetentionPolicyTags/>") {
            // What's left out is told once a session, so the log says why a tag doesn't show.
            Ok(doc) => {
                let (tags, left, more) = tags_read(&doc);
                for l in &left {
                    log::info!("{l}");
                }
                if more > 0 {
                    log::info!("archive policy: {more} more personal tags left out");
                }
                tags
            }
            Err(NetError::No(e) | NetError::Protocol(e)) => {
                log::info!("no archive policy tags: {e}");
                Vec::new()
            }
            Err(e) => return Err(e),
        };
        *self.tags.borrow_mut() = Some(tags.clone());
        Ok(tags)
    }

    /// A message's own archive and retention tags and the days the server plans to move and to
    /// delete it.
    pub fn item_policy(&self, id: &str) -> Result<ItemPolicy> {
        let m = self.call_one(&format!(
            "<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties><t:FieldURI FieldURI=\"item:ArchiveTag\"/>\
             <t:FieldURI FieldURI=\"item:PolicyTag\"/><t:FieldURI FieldURI=\"item:RetentionDate\"/>{ARCHIVE_DATE}</t:AdditionalProperties></m:ItemShape>\
             <m:ItemIds><t:ItemId Id=\"{}\"/></m:ItemIds></m:GetItem>",
            esc(id)
        ))?;
        let Some(it) = m.find("Items").and_then(|i| i.kids.first()) else { return Ok(ItemPolicy::default()) };
        Ok(ItemPolicy {
            archive: own_tag(it, PolicyKind::Archive),
            archive_day: archive_day(it),
            retention: own_tag(it, PolicyKind::Retention),
            retention_day: it.text_of("RetentionDate").and_then(day_of),
        })
    }

    /// Whether the account's folder listing should ask each folder's tags: only a server that
    /// offers tags (Exchange 2013 and later) knows the fields.
    pub(in super::super) fn folder_tags_asked(&self) -> bool {
        self.archive_tags().is_ok_and(|t| !t.is_empty())
    }
}
