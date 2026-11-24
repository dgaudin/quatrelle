// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Outlook's Calendar pane groups written back: the navigation shortcuts of the Common Views
//! folder made, renamed, deleted and moved ([MS-OXOCFG] 2.2.9). Only the Calendar section's
//! shortcuts this reads whole are touched: a group header ([MS-OXOCFG] 2.2.9.3, 2.2.9.5 type 4)
//! and the entries under it (types 0 and 2). Other sections' shortcuts, and any shortcut that
//! doesn't read as one, are left as they are.

use rand_core::RngCore as _;

use super::*;
use crate::NetError;

/// A Calendar-section shortcut as the server keeps it, with the item id it's changed by.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Shortcut {
    id: String,
    change_key: String,
    section: Option<i64>,
    kind: Option<i64>,
    name: String,
    group_name: String,
    group_id: Vec<u8>,
    header_id: Vec<u8>,
    ordinal: Vec<u8>,
    entry_id: Vec<u8>,
    store_id: Vec<u8>,
    folder_type: Vec<u8>,
    address_book: Vec<u8>,
}

/// The Calendar section's shortcuts, read page by page for a write. Only the shortcuts this
/// understands are returned; the others are left alone.
fn read_raw(c: &Client) -> Result<Vec<Shortcut>> {
    let Some(views) = super::common_views(c)? else {
        return Err(NetError::No("the server doesn't show its Common Views folder".into()));
    };
    let props: String = PROPS.iter().map(|(tag, kind)| format!("<t:ExtendedFieldURI PropertyTag=\"{tag}\" PropertyType=\"{kind}\"/>")).collect();
    let mut out = Vec::new();
    let mut complete = false;
    for page in 0..MAX_PAGES {
        let offset = page * PAGE;
        let m = c.call_one(&format!(
            "<m:FindItem Traversal=\"Associated\"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape>\
             <t:AdditionalProperties>{props}</t:AdditionalProperties></m:ItemShape>\
             <m:IndexedPageItemView MaxEntriesReturned=\"{PAGE}\" Offset=\"{offset}\" BasePoint=\"Beginning\"/>\
             <m:Restriction><t:IsEqualTo><t:FieldURIOrConstant><t:FieldURI FieldURI=\"message:MessageClass\"/></t:FieldURIOrConstant>\
             <t:FieldURIOrConstant><t:Constant Value=\"{WLINK_CLASS}\"/></t:FieldURIOrConstant></t:IsEqualTo></m:Restriction>\
             <m:ParentFolderIds><t:FolderId Id=\"{}\"/></m:ParentFolderIds></m:FindItem>",
            esc(&views)
        ))?;
        let batch: Vec<El> = m.find("Items").map(|i| i.kids.clone()).unwrap_or_default();
        let last = m.find("RootFolder").and_then(|r| r.attr("IncludesLastItemInRange")).is_some_and(|v| v.eq_ignore_ascii_case("true"));
        for it in &batch {
            if let Some(s) = shortcut_of(it) {
                out.push(s);
            }
        }
        if last {
            complete = true;
            break;
        }
    }
    if !complete {
        return Err(NetError::No("the server's navigation shortcuts weren't read whole".into()));
    }
    Ok(out)
}

/// One shortcut, when its message class is the navigation shortcut's.
fn shortcut_of(it: &El) -> Option<Shortcut> {
    let class = ext_text(it, "0x001A")?.trim();
    if !class.eq_ignore_ascii_case(WLINK_CLASS) {
        return None;
    }
    let id = it.find("ItemId")?.attr("Id")?.to_string();
    if id.is_empty() {
        return None;
    }
    let change_key = it.find("ItemId").and_then(|i| i.attr("ChangeKey")).unwrap_or_default().to_string();
    let section = int_prop(it, "0x6852");
    let kind = int_prop(it, "0x6849");
    let name = clean(ext_text(it, "0x0E1D").unwrap_or_default());
    let group_name = clean(ext_text(it, "0x6851").unwrap_or_default());
    let group_id = bin_prop(it, "0x6850").unwrap_or_default();
    let header_id = bin_prop(it, "0x6842").unwrap_or_default();
    let ordinal = bin_prop(it, "0x684B").unwrap_or_default();
    let address_book = bin_prop(it, "0x6854").unwrap_or_default();
    if ordinal.len() > 16 || (!group_id.is_empty() && group_id.len() != 16) {
        return None;
    }
    let calendar = section == Some(CALENDAR_SECTION);
    let usable = match kind {
        Some(WLINK_HEADER) => calendar && !name.is_empty() && header_id.len() == 16,
        Some(WLINK_OWN) => calendar && !name.is_empty(),
        Some(WLINK_SHARED) => calendar && address_book_dn(&address_book).is_some(),
        _ => false,
    };
    if !usable {
        return None;
    }
    Some(Shortcut {
        id,
        change_key,
        section,
        kind,
        name,
        group_name,
        group_id,
        header_id,
        ordinal,
        entry_id: bin_prop(it, "0x684C").unwrap_or_default(),
        store_id: bin_prop(it, "0x684E").unwrap_or_default(),
        folder_type: bin_prop(it, "0x684F").unwrap_or_default(),
        address_book,
    })
}

/// The Calendar section's group headers, in Outlook's order.
fn headers(all: &[Shortcut]) -> Vec<&Shortcut> {
    let mut out: Vec<&Shortcut> =
        all.iter().filter(|s| s.section == Some(CALENDAR_SECTION) && s.kind == Some(WLINK_HEADER) && !s.name.is_empty()).collect();
    out.sort_by(|a, b| a.ordinal.cmp(&b.ordinal));
    out
}

/// The group a shortcut is in, as its header names it (else as the shortcut says).
fn group_of(s: &Shortcut, all: &[Shortcut]) -> String {
    if !s.group_id.is_empty()
        && let Some(h) = all
            .iter()
            .find(|h| h.section == Some(CALENDAR_SECTION) && h.kind == Some(WLINK_HEADER) && h.header_id == s.group_id && !h.name.is_empty())
    {
        return h.name.clone();
    }
    s.group_name.clone()
}

fn group_named<'a>(all: &'a [Shortcut], name: &str) -> Option<&'a Shortcut> {
    headers(all).into_iter().find(|h| h.name.eq_ignore_ascii_case(name))
}

fn members_of<'a>(all: &'a [Shortcut], group: &Shortcut) -> Vec<&'a Shortcut> {
    all.iter()
        .filter(|s| s.section == Some(CALENDAR_SECTION) && matches!(s.kind, Some(WLINK_OWN | WLINK_SHARED)))
        // The header's id is what Outlook points at; a member that carries no id falls back to
        // the header's exact name, never to a case-insensitive one that another group could
        // share.
        .filter(|s| if s.group_id.is_empty() { !s.group_name.is_empty() && s.group_name == group.name } else { s.group_id == group.header_id })
        .collect()
}

/// A new group's ordinal, after the last group's ([MS-OXOCFG] 2.2.9.7: a final byte other than
/// 0 or 255 keeps room to insert before and after).
fn after_last_ordinal(all: &[Shortcut]) -> Vec<u8> {
    let mut out = headers(all).last().map(|h| h.ordinal.clone()).unwrap_or_default();
    if out.len() >= 16 {
        out.truncate(15);
        out.push(0x7f);
    } else {
        out.push(0x80);
    }
    out
}

/// The ordinal for an entry added after `group`'s entries, `except` left out.
fn after_group(all: &[Shortcut], group: &Shortcut, except: Option<&str>) -> Vec<u8> {
    let mut last = Vec::new();
    for m in members_of(all, group) {
        if except.is_some_and(|id| id == m.id) {
            continue;
        }
        if m.ordinal > last {
            last = m.ordinal.clone();
        }
    }
    if last.len() >= 16 {
        last.truncate(15);
        last.push(0x7f);
    } else {
        last.push(0x80);
    }
    last
}

/// CLSID_CalendarFolder ([MS-OXOCFG] 2.2.9.11): the folder class a calendar shortcut points at.
const CALENDAR_FOLDER: [u8; 16] = [0x02, 0x78, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46];

/// A new GUID for a group header ([MS-OXOCFG] 2.2.9.3, PtypGuid).
fn new_guid() -> [u8; 16] {
    let mut out = [0u8; 16];
    rand_core::OsRng.fill_bytes(&mut out);
    out
}

/// An extended property of a new shortcut.
fn ext(tag: &str, kind: &str, value: &str) -> String {
    format!("<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"{tag}\" PropertyType=\"{kind}\"/><t:Value>{value}</t:Value></t:ExtendedProperty>")
}

/// A shortcut created in the Common Views folder: an associated item ([MS-OXOCFG] 2.2.9: a
/// group header, type 4, section 3).
fn create_group_xml(views: &str, name: &str, id: &[u8], ordinal: &[u8]) -> String {
    let guid = crate::imap::base64(id);
    let folder_type = crate::imap::base64(&CALENDAR_FOLDER);
    let ordinal = crate::imap::base64(ordinal);
    format!(
        "<m:CreateItem MessageDisposition=\"SaveOnly\"><m:SavedItemFolderId><t:FolderId Id=\"{}\"/></m:SavedItemFolderId>\
         <m:Items><t:Message><t:ItemClass>{WLINK_CLASS}</t:ItemClass><t:Subject>{}</t:Subject>{}{}{}{}{}\
         <t:IsAssociated>true</t:IsAssociated></t:Message></m:Items></m:CreateItem>",
        esc(views),
        esc(name),
        ext("0x6842", "Binary", &guid),
        ext("0x6849", "Integer", "4"),
        ext("0x684B", "Binary", &ordinal),
        ext("0x684F", "Binary", &folder_type),
        ext("0x6852", "Integer", "3"),
    )
}

/// A `SetItemField` setting the shortcut's name (its normalized subject, [MS-OXOCFG] 2.2.9.2).
fn set_subject(name: &str) -> String {
    format!("<t:SetItemField><t:FieldURI FieldURI=\"item:Subject\"/><t:Message><t:Subject>{}</t:Subject></t:Message></t:SetItemField>", esc(name))
}

/// A `SetItemField` setting an extended property.
fn set_ext(tag: &str, kind: &str, value: &str) -> String {
    let uri = format!("<t:ExtendedFieldURI PropertyTag=\"{tag}\" PropertyType=\"{kind}\"/>");
    format!("<t:SetItemField>{uri}<t:Message><t:ExtendedProperty>{uri}<t:Value>{value}</t:Value></t:ExtendedProperty></t:Message></t:SetItemField>")
}

fn update(c: &Client, s: &Shortcut, updates: &[String]) -> Result<()> {
    let updates = updates.concat();
    c.call_one(&format!(
        "<m:UpdateItem ConflictResolution=\"AutoResolve\"><m:ItemChanges><t:ItemChange>\
         <t:ItemId Id=\"{}\" ChangeKey=\"{}\"/><t:Updates>{updates}</t:Updates>\
         </t:ItemChange></m:ItemChanges></m:UpdateItem>",
        esc(&s.id),
        esc(&s.change_key)
    ))
    .map(drop)
}

/// A shortcut deleted with `SoftDelete` (into Deleted Items), as Outlook deletes one.
fn delete(c: &Client, s: &Shortcut) -> Result<()> {
    c.call_one(&format!(
        "<m:DeleteItem DeleteType=\"SoftDelete\"><m:ItemIds><t:ItemId Id=\"{}\" ChangeKey=\"{}\"/></m:ItemIds></m:DeleteItem>",
        esc(&s.id),
        esc(&s.change_key)
    ))
    .map(drop)
}

/// `groups_are` checks the read-back names, in Outlook's order.
fn groups_are(c: &Client, expected: &[String]) -> Result<()> {
    let after = read_raw(c)?;
    let mut seen: Vec<String> = Vec::new();
    for h in headers(&after) {
        if !seen.iter().any(|n| n.eq_ignore_ascii_case(&h.name)) {
            seen.push(h.name.clone());
        }
    }
    let same = seen.len() == expected.len() && seen.iter().zip(expected).all(|(a, b)| a.eq_ignore_ascii_case(b));
    if same { Ok(()) } else { Err(NetError::No("the server's calendar groups aren't the ones written".into())) }
}

/// Make a group called `name`.
pub(crate) fn create_calendar_group(c: &Client, name: &str) -> Result<()> {
    let name = clean(name);
    if name.is_empty() {
        return Err(NetError::No("a calendar group needs a name".into()));
    }
    let before = read_raw(c)?;
    if group_named(&before, &name).is_some() {
        return Err(NetError::No("a calendar group with that name is there already".into()));
    }
    let Some(views) = super::common_views(c)? else {
        return Err(NetError::No("the server doesn't show its Common Views folder".into()));
    };
    let id = new_guid();
    let ordinal = after_last_ordinal(&before);
    c.call_one(&create_group_xml(&views, &name, &id, &ordinal))?;
    let mut expected: Vec<String> = headers(&before).into_iter().map(|h| h.name.clone()).collect();
    expected.push(name);
    groups_are(c, &expected)
}

/// Rename the group `group` to `name`, its entries with it.
pub(crate) fn rename_calendar_group(c: &Client, group: &str, name: &str) -> Result<()> {
    let name = clean(name);
    if name.is_empty() {
        return Err(NetError::No("a calendar group needs a name".into()));
    }
    let before = read_raw(c)?;
    let header = group_named(&before, group).cloned().ok_or_else(|| NetError::No("no calendar group with that name".into()))?;
    if header.name.eq_ignore_ascii_case(&name) {
        return Ok(());
    }
    if group_named(&before, &name).is_some() {
        return Err(NetError::No("a calendar group with that name is there already".into()));
    }
    let members: Vec<Shortcut> = members_of(&before, &header).into_iter().cloned().collect();
    update(c, &header, &[set_subject(&name)])?;
    let mut done: Vec<Shortcut> = Vec::new();
    for m in &members {
        if let Err(e) = update(c, m, &[set_ext("0x6851", "String", &esc(&name))]) {
            // Put back what was changed, best effort, so the server isn't left half renamed.
            let _ = update(c, &header, &[set_subject(&header.name)]);
            for d in &done {
                let _ = update(c, d, &[set_ext("0x6851", "String", &esc(&header.name))]);
            }
            return Err(e);
        }
        done.push(m.clone());
    }
    let mut expected: Vec<String> = headers(&before).into_iter().map(|h| h.name.clone()).collect();
    for n in &mut expected {
        if n.eq_ignore_ascii_case(&header.name) {
            *n = name.clone();
        }
    }
    groups_are(c, &expected)?;
    // The entries read back under the new name (the header's name is what the read uses).
    let after = read_raw(c)?;
    let Some(after_header) = group_named(&after, &name) else {
        return Err(NetError::No("the server didn't keep the renamed group".into()));
    };
    let still_old = after.iter().any(|s| {
        s.section == Some(CALENDAR_SECTION)
            && matches!(s.kind, Some(WLINK_OWN | WLINK_SHARED))
            && s.group_id == after_header.header_id
            && !group_of(s, &after).eq_ignore_ascii_case(&name)
    });
    if still_old {
        return Err(NetError::No("the server kept some of the group's entries in the old group".into()));
    }
    Ok(())
}

/// Delete the group `group`: its entries go with it (the account's own calendars stay, and come
/// back under My Calendars; other people's leave the list).
pub(crate) fn delete_calendar_group(c: &Client, group: &str) -> Result<()> {
    let before = read_raw(c)?;
    let header = group_named(&before, group).cloned().ok_or_else(|| NetError::No("no calendar group with that name".into()))?;
    let members: Vec<Shortcut> = members_of(&before, &header).into_iter().cloned().collect();
    for m in &members {
        delete(c, m)?;
    }
    delete(c, &header)?;
    if group_named(&read_raw(c)?, group).is_some() {
        return Err(NetError::No("the server kept the calendar group".into()));
    }
    Ok(())
}

/// Move the group `group` up or down among the groups, by its ordinal ([MS-OXOCFG] 2.2.9.7).
pub(crate) fn move_calendar_group(c: &Client, group: &str, up: bool) -> Result<()> {
    let before = read_raw(c)?;
    let list = headers(&before);
    let at = list.iter().position(|h| h.name.eq_ignore_ascii_case(group)).ok_or_else(|| NetError::No("no calendar group with that name".into()))?;
    let there = if up {
        at.checked_sub(1).ok_or_else(|| NetError::No("it's the first calendar group already".into()))?
    } else {
        at.checked_add(1).filter(|n| *n < list.len()).ok_or_else(|| NetError::No("it's the last calendar group already".into()))?
    };
    let mut order: Vec<(String, Vec<u8>)> = list.iter().map(|h| (h.name.clone(), h.ordinal.clone())).collect();
    order.swap(at, there);
    // Each group's new ordinal: the ordinal of the place it takes, when they're all distinct
    // one-byte ordinals; else a fresh one-byte ordinal per group (groups are few, and Outlook's
    // own are one byte).
    let distinct =
        list.iter().map(|h| &h.ordinal).collect::<std::collections::BTreeSet<_>>().len() == list.len() && list.iter().all(|h| !h.ordinal.is_empty());
    let fresh: Vec<Vec<u8>> = if distinct {
        list.iter().map(|h| h.ordinal.clone()).collect()
    } else {
        (1..=order.len()).map(|n| vec![u8::try_from(n).unwrap_or(1)]).collect()
    };
    let mut changed: Vec<(Shortcut, Vec<u8>)> = Vec::new();
    for (i, (name, _)) in order.iter().enumerate() {
        let Some(shortcut) = list.iter().find(|h| h.name.eq_ignore_ascii_case(name)) else { continue };
        let new = fresh.get(i).cloned().unwrap_or_default();
        if new != shortcut.ordinal
            && let Err(e) = update(c, shortcut, &[set_ext("0x684B", "Binary", &crate::imap::base64(&new))])
        {
            for (s, old) in &changed {
                let _ = update(c, s, &[set_ext("0x684B", "Binary", &crate::imap::base64(old))]);
            }
            return Err(e);
        }
        if new != shortcut.ordinal {
            changed.push(((*shortcut).clone(), shortcut.ordinal.clone()));
        }
    }
    let expected: Vec<String> = order.into_iter().map(|(n, _)| n).collect();
    groups_are(c, &expected)
}

impl Shortcut {
    /// The item's name or address, for an error (never logged).
    fn owner_said(&self) -> &str {
        if self.name.is_empty() { self.group_name.as_str() } else { self.name.as_str() }
    }
}

/// The shortcut of the calendar `calendar` (its id), by the name the account gives it.
fn find_member<'a>(c: &Client, all: &'a [Shortcut], calendar: &str, name: &str) -> Option<&'a Shortcut> {
    if let Some(owner) = calendar.strip_prefix("shared:") {
        for s in all.iter().filter(|s| s.section == Some(CALENDAR_SECTION) && s.kind == Some(WLINK_SHARED)) {
            if s.name.eq_ignore_ascii_case(name) {
                return Some(s);
            }
            if let Some(dn) = address_book_dn(&s.address_book)
                && resolve(c, &dn).is_some_and(|e| e.eq_ignore_ascii_case(owner))
            {
                return Some(s);
            }
        }
        return None;
    }
    let own = all.iter().filter(|s| s.section == Some(CALENDAR_SECTION) && s.kind == Some(WLINK_OWN));
    if let Some(id) = calendar.strip_prefix(crate::ews_pim::FOLDER)
        && let Some(bytes) = crate::ews::unb64(id)
        && let Some(s) = own.clone().find(|s| s.entry_id == bytes)
    {
        return Some(s);
    }
    own.into_iter().find(|s| s.name.eq_ignore_ascii_case(name))
}

/// Put the calendar `calendar` in the group `group` (empty: My Calendars): its shortcut's group
/// id, name and ordinal ([MS-OXOCFG] 2.2.9.7, 2.2.9.12, 2.2.9.13) change; its folder is untouched.
pub(crate) fn move_calendar_to_group(c: &Client, calendar: &str, name: &str, group: &str) -> Result<()> {
    let before = read_raw(c)?;
    let target = if group.is_empty() {
        before
            .iter()
            .find(|s| s.section == Some(CALENDAR_SECTION) && s.kind == Some(WLINK_OWN) && s.name.eq_ignore_ascii_case(name))
            .map(|s| group_of(s, &before))
            .filter(|g| !g.is_empty())
            .and_then(|g| group_named(&before, &g).cloned())
            .or_else(|| group_named(&before, "My Calendars").cloned())
            .ok_or_else(|| NetError::No("the account's My Calendars group isn't in its shortcuts".into()))?
    } else {
        group_named(&before, group).cloned().ok_or_else(|| NetError::No("no calendar group with that name".into()))?
    };
    let member = find_member(c, &before, calendar, name)
        .cloned()
        .ok_or_else(|| NetError::No("this calendar isn't in Outlook's calendar groups yet".into()))?;
    if group_of(&member, &before).eq_ignore_ascii_case(&target.name) {
        return Ok(());
    }
    if target.header_id.is_empty() {
        return Err(NetError::No("the calendar group has no id to put a calendar under".into()));
    }
    let ordinal = after_group(&before, &target, Some(&member.id));
    update(
        c,
        &member,
        &[
            set_ext("0x6850", "Binary", &crate::imap::base64(&target.header_id)),
            set_ext("0x6851", "String", &esc(&target.name)),
            set_ext("0x684B", "Binary", &crate::imap::base64(&ordinal)),
        ],
    )?;
    let after = read_raw(c)?;
    let moved = after.iter().find(|s| s.id == member.id).ok_or_else(|| NetError::No("the server lost the calendar's shortcut".into()))?;
    if !group_of(moved, &after).eq_ignore_ascii_case(&target.name) {
        return Err(NetError::No(format!("the server kept {} in another group", member.owner_said())));
    }
    Ok(())
}
