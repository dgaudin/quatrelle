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
    flags: i64,
    save_stamp: Option<i64>,
    entry_id: Vec<u8>,
    store_id: Vec<u8>,
    folder_type: Vec<u8>,
    address_book: Vec<u8>,
}

/// The Calendar section's shortcuts' items, page by page, read whole.
fn read_pages(c: &Client) -> Result<Vec<El>> {
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
        out.extend(batch);
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

/// The Calendar section's shortcuts, read page by page for a write. Only the shortcuts this
/// understands are returned; the others are left alone.
fn read_raw(c: &Client) -> Result<Vec<Shortcut>> {
    Ok(read_pages(c)?.iter().filter_map(shortcut_of).collect())
}

/// Whether the production write parser reads this item as a shortcut; for the fuzz target.
pub(super) fn shortcut_checked(it: &El) -> bool {
    shortcut_of(it).is_some()
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
        flags: int_prop(it, "0x684A").unwrap_or(0),
        save_stamp: int_prop(it, "0x6847"),
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

/// An item the read can't parse but that still points at `header`: deleting the header would
/// leave it behind, pointing at a group that no longer exists.
fn unreadable_member(pages: &[El], header: &Shortcut) -> Option<String> {
    pages.iter().find_map(|it| {
        if shortcut_of(it).is_some() || int_prop(it, "0x6852") != Some(CALENDAR_SECTION) {
            return None;
        }
        if !matches!(int_prop(it, "0x6849"), Some(WLINK_OWN | WLINK_SHARED)) {
            return None;
        }
        let id = bin_prop(it, "0x6850").unwrap_or_default();
        let group = ext_text(it, "0x6851").map(clean).unwrap_or_default();
        let mine = if id.is_empty() { !group.is_empty() && group == header.name } else { id == header.header_id };
        if !mine {
            return None;
        }
        let shown = ext_text(it, "0x0E1D").map(clean).unwrap_or_default();
        Some(if shown.is_empty() { "a calendar this can't read".to_string() } else { shown })
    })
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

/// CLSID_CalendarFolder ([MS-OXOCFG] 2.2.9.11, the value [MS-OXCDATA] 2.2.5.2 gives) as 16 bytes, its
/// first three fields little-endian ([MS-DTYP] 2.3.4); the property is PtypBinary: the
/// folder class a calendar shortcut points at.
const CALENDAR_FOLDER: [u8; 16] = [0x02, 0x78, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46];

/// A new GUID for a group header ([MS-OXOCFG] 2.2.9.3).
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
fn create_group_xml(views: &str, name: &str, id: &[u8], ordinal: &[u8], stamp: u32) -> String {
    let guid = crate::imap::base64(id);
    let folder_type = crate::imap::base64(&CALENDAR_FOLDER);
    let ordinal = crate::imap::base64(ordinal);
    format!(
        "<m:CreateItem MessageDisposition=\"SaveOnly\"><m:SavedItemFolderId><t:FolderId Id=\"{}\"/></m:SavedItemFolderId>\
         <m:Items><t:Message><t:ItemClass>{WLINK_CLASS}</t:ItemClass><t:Subject>{}</t:Subject>{}{}{}{}{}{}{}\
         <t:IsAssociated>true</t:IsAssociated></t:Message></m:Items></m:CreateItem>",
        esc(views),
        esc(name),
        ext("0x6842", "Binary", &guid),
        ext("0x6847", "Integer", &stamp.to_string()),
        ext("0x6849", "Integer", "4"),
        ext("0x684A", "Integer", "0"),
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

/// A shortcut deleted with `SoftDelete` (into Recoverable Items, where Exchange keeps a deleted
/// associated item), as Outlook deletes one.
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
    let mut wanted: Vec<&String> = Vec::new();
    for name in expected {
        if !wanted.iter().any(|n| n.eq_ignore_ascii_case(name)) {
            wanted.push(name);
        }
    }
    let same = seen.len() == wanted.len() && seen.iter().zip(wanted).all(|(a, b)| a.eq_ignore_ascii_case(b));
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
    let made = c.call_one(&create_group_xml(&views, &name, &id, &ordinal, crate::ews::group_stamp(&id)))?;
    let created = made.find("ItemId").and_then(|i| i.attr("Id")).filter(|i| !i.is_empty()).map(str::to_string);
    let mut expected: Vec<String> = headers(&before).into_iter().map(|h| h.name.clone()).collect();
    expected.push(name);
    if let Err(e) = groups_are(c, &expected) {
        // The server made the header but its read-back doesn't show it: take it out again, so a
        // failed make leaves nothing behind.
        if let Some(id) = created {
            let _ =
                c.call_one(&format!("<m:DeleteItem DeleteType=\"HardDelete\"><m:ItemIds><t:ItemId Id=\"{}\"/></m:ItemIds></m:DeleteItem>", esc(&id)));
        }
        return Err(e);
    }
    Ok(())
}

/// Whether Quatrelle made the header: its save stamp is the one its own id gives.
fn ours(header: &Shortcut) -> bool {
    header.save_stamp == Some(i64::from(crate::ews::group_stamp(&header.header_id)))
}

/// Rename the group `group` to `name`, its entries with it.
pub(crate) fn rename_calendar_group(c: &Client, group: &str, name: &str) -> Result<()> {
    let name = clean(name);
    if name.is_empty() {
        return Err(NetError::No("a calendar group needs a name".into()));
    }
    let before = read_raw(c)?;
    let header = group_named(&before, group).cloned().ok_or_else(|| NetError::No("no calendar group with that name".into()))?;
    if !ours(&header) {
        return Err(NetError::No("Outlook's own calendar group can't be renamed here".into()));
    }
    if header.name.eq_ignore_ascii_case(&name) {
        return Ok(());
    }
    if group_named(&before, &name).is_some() {
        return Err(NetError::No("a calendar group with that name is there already".into()));
    }
    let members: Vec<Shortcut> = members_of(&before, &header).into_iter().cloned().collect();
    let flags = header.flags | 0x2000;
    update(
        c,
        &header,
        &[
            set_subject(&name),
            set_ext("0x6847", "Integer", &crate::ews::group_stamp(&header.header_id).to_string()),
            set_ext("0x684A", "Integer", &flags.to_string()),
        ],
    )?;
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
    // A header with the same name isn't the one renamed unless it is the same header; names
    // aren't unique in Outlook's pane.
    let expected: Vec<String> =
        headers(&before).into_iter().map(|h| if h.header_id == header.header_id { name.clone() } else { h.name.clone() }).collect();
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
    let pages = read_pages(c)?;
    let before: Vec<Shortcut> = pages.iter().filter_map(shortcut_of).collect();
    let header = group_named(&before, group).cloned().ok_or_else(|| NetError::No("no calendar group with that name".into()))?;
    let members: Vec<Shortcut> = members_of(&before, &header).into_iter().cloned().collect();
    if !ours(&header) && !members.is_empty() {
        return Err(NetError::No("a calendar group Outlook made can't be deleted while it holds calendars".into()));
    }
    if let Some(name) = unreadable_member(&pages, &header) {
        return Err(NetError::No(format!("the calendar group still holds {name}, which this can't read")));
    }
    let mut deleted: Vec<String> = Vec::new();
    for m in &members {
        match delete(c, m) {
            Ok(()) => deleted.push(m.owner_said().to_string()),
            Err(e) if deleted.is_empty() => return Err(e),
            Err(e) => return Err(NetError::No(format!("{} was already deleted; {e}", deleted.join(", ")))),
        }
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
    if !ours(list[at]) {
        return Err(NetError::No("Outlook's own calendar group can't be moved here".into()));
    }
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

/// The shortcut of the calendar `calendar` (its id), by the name the account gives it. Someone
/// else's is found by their address (the entry id's X.500 name, resolved once a session), a
/// shared name only when it is the only one; an own calendar's EWS folder id
/// is not the shortcut's MAPI entry id, so it is found by its name, and only when only one
/// shortcut has it.
fn find_member<'a>(c: &Client, all: &'a [Shortcut], calendar: &str, name: &str) -> Option<&'a Shortcut> {
    if let Some(owner) = calendar.strip_prefix("shared:") {
        let mut named: Vec<&Shortcut> = Vec::new();
        for s in all.iter().filter(|s| s.section == Some(CALENDAR_SECTION) && s.kind == Some(WLINK_SHARED)) {
            let mut known_other = false;
            if let Some(dn) = address_book_dn(&s.address_book) {
                let email = resolve(c, &dn);
                if email.as_ref().is_some_and(|e| e.eq_ignore_ascii_case(owner)) {
                    return Some(s);
                }
                // An address that resolved to someone else is never a name fallback: that
                // would move the other person's calendar.
                known_other = email.is_some();
            }
            if !known_other && s.name.eq_ignore_ascii_case(name) {
                named.push(s);
            }
        }
        return if named.len() == 1 { named.first().copied() } else { None };
    }
    let mut named =
        all.iter().filter(|s| s.section == Some(CALENDAR_SECTION) && s.kind == Some(WLINK_OWN)).filter(|s| s.name.eq_ignore_ascii_case(name));
    let first = named.next()?;
    if named.next().is_some() { None } else { Some(first) }
}

/// Put the calendar `calendar` in the group `group` (empty: My Calendars): its shortcut's group
/// id, name and ordinal ([MS-OXOCFG] 2.2.9.7, 2.2.9.12, 2.2.9.13) change; its folder is untouched.
pub(crate) fn move_calendar_to_group(c: &Client, calendar: &str, name: &str, group: &str) -> Result<()> {
    let before = read_raw(c)?;
    let target = group_named(&before, group).cloned().ok_or_else(|| NetError::No("no calendar group with that name".into()))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The folder type is binary ([MS-OXOCFG] 2.2.9.11, PtypBinary), read and written as such: the
    /// 16 bytes of CLSID_CalendarFolder, little-endian ([MS-DTYP] 2.3.4), in base64.
    #[test]
    fn the_folder_type_is_read_and_written_as_binary() {
        let read = PROPS.iter().find(|(tag, _)| *tag == "0x684F").map(|(_, kind)| *kind);
        let written = create_group_xml("F", "G", &[0u8; 16], &[0x80], 1);
        assert_eq!(read, Some("Binary"), "the read asks the type the write uses");
        let value = crate::imap::base64(&CALENDAR_FOLDER);
        assert_eq!(value, "AngGAAAAAADAAAAAAAAARg==");
        assert!(written.contains(&format!("PropertyTag=\"0x684F\" PropertyType=\"Binary\"/><t:Value>{value}</t:Value>")), "{written}");
    }
}
