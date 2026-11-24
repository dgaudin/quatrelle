// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Outlook's Calendar pane groups, as navigation shortcuts ([MS-OXOCFG] 2.2.9): the hidden items
//! of the mailbox's Common Views folder. A group header names a group ([MS-OXOCFG] 2.2.9.3,
//! 2.2.9.5 type 4, 2.2.9.2 name); the entries under it point at a colleague's calendar by their
//! address book entry id ([MS-OXOCFG] 2.2.9.16) or at one of the account's own folders
//! ([MS-OXOCFG] 2.2.9.5 type 0); the order is the ordinal's ([MS-OXOCFG] 2.2.9.7).

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use crate::Result;
use crate::ews::{Client, El, esc};

mod write;

pub(crate) use write::{create_calendar_group, delete_calendar_group, move_calendar_group, move_calendar_to_group, rename_calendar_group};

/// A calendar Outlook keeps in a group of its Calendar pane (a colleague's in a team's group, a
/// room's…): whose it is, the name shown, and the group's name. One kept by the account itself
/// in a group has no address to open it by (`own`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CalendarLink {
    pub email: String,
    pub name: String,
    pub group: String,
}

/// What the Common Views folder says of the Calendar pane: its groups in Outlook's order, and the
/// calendars kept in them (the account's own ones have no address).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Shortcuts {
    pub groups: Vec<String>,
    pub links: Vec<CalendarLink>,
}

/// The shortcut's message class ([MS-OXOCFG] 2.2.9.1).
const WLINK_CLASS: &str = "IPM.Microsoft.WunderBar.Link";
/// The Calendar section of the pane ([MS-OXOCFG] 2.2.9.14).
const CALENDAR_SECTION: i64 = 3;
/// The shortcut points at a group header ([MS-OXOCFG] 2.2.9.5).
const WLINK_HEADER: i64 = 4;
/// The shortcut points at another user's folder ([MS-OXOCFG] 2.2.9.5).
const WLINK_SHARED: i64 = 2;
/// The shortcut points at a folder of the account's own ([MS-OXOCFG] 2.2.9.5).
const WLINK_OWN: i64 = 0;
/// How many shortcuts one `FindItem` asks for, and how many pages are read: a hostile server
/// can't make the read go on forever.
const PAGE: usize = 300;
const MAX_PAGES: usize = 10;
/// Most calendars kept from them.
const MAX_LINKS: usize = 300;
/// Most groups kept (a hostile or huge mailbox can't fill the pane).
const MAX_GROUPS: usize = 300;

/// A navigation shortcut that names a calendar in a group: its X.500 directory name, the name
/// Outlook shows, and the group. `None` for another shortcut, or one that isn't a calendar
/// another user owns. Read as hostile XML. For the fuzz target too.
fn shortcut_of(it: &El) -> Option<(String, String, String)> {
    let text = |tag: &str| ext_text(it, tag);
    let int = |tag: &str| text(tag).and_then(|v| v.trim().parse::<i64>().ok());
    // A calendar (section 3) another user owns (kind 2), in a group.
    if text("0x001A") != Some(WLINK_CLASS) || int("0x6852") != Some(CALENDAR_SECTION) || int("0x6849") != Some(WLINK_SHARED) {
        return None;
    }
    let dn = text("0x6854").and_then(|v| crate::ews::unb64(v.trim())).and_then(|b| address_book_dn(&b))?;
    let name = clean(text("0x0E1D").unwrap_or_default());
    let group = clean(text("0x6851").unwrap_or_default());
    Some((dn, name, group))
}

/// The calendars named by navigation shortcuts in a `FindItem` answer's items, each address book
/// entry once, at most [`MAX_LINKS`], before the directory resolves them. For the fuzz target.
#[doc(hidden)]
pub fn shortcuts_listed(doc: &El) -> Vec<(String, String, String)> {
    let items: Vec<El> = doc.find("Items").map(|i| i.kids.clone()).unwrap_or_default();
    let mut out: Vec<(String, String, String)> = Vec::new();
    for it in items.iter().take(MAX_LINKS) {
        let Some((dn, name, group)) = shortcut_of(it) else { continue };
        if out.iter().any(|(d, _, _)| *d == dn) {
            continue;
        }
        out.push((dn, name, group));
    }
    out
}
/// Most folders of the mailbox root probed for the Common Views folder.
const MAX_PROBED: usize = 24;
/// Most addresses looked up in the directory (a group has few people; a hostile server can't
/// make the read call it hundreds of times).
const MAX_LOOKUPS: usize = 100;
/// A binary shortcut property longer than this isn't read: an entry id is at most a few hundred
/// bytes, and the bound keeps a hostile answer from being decoded.
const MAX_BINARY: usize = 8192;
/// A displayed name's most characters.
const MAX_NAME: usize = 255;
/// The properties a shortcut carries that this reads: class, name, header id, type, ordinal,
/// entry and store ids, folder type, group id, group name, section and address book entry id
/// ([MS-OXOCFG] 2.2.9.1–2.2.9.16).
const PROPS: [(&str, &str); 12] = [
    ("0x001A", "String"),
    ("0x0E1D", "String"),
    ("0x6842", "Binary"),
    ("0x6849", "Integer"),
    ("0x684B", "Binary"),
    ("0x684C", "Binary"),
    ("0x684E", "Binary"),
    ("0x684F", "Binary"),
    ("0x6850", "Binary"),
    ("0x6851", "String"),
    ("0x6852", "Integer"),
    ("0x6854", "Binary"),
];

/// The calendars Outlook keeps in groups of its Calendar pane (a colleague's, a room's), each
/// with the group's name. The account's own calendars kept in a group come too, with no address
/// (`own`).
pub(crate) fn calendar_links(c: &Client) -> Result<Shortcuts> {
    let Some(views) = common_views(c)? else {
        log::info!("no calendar groups: the Common Views folder wasn't found");
        return Ok(Shortcuts::default());
    };
    read_shortcuts(c, &views)
}

/// The Common Views folder's id: the folder the navigation shortcuts are kept in
/// ([MS-OXOCFG] 2.2.9). Exchange gives it a name in the mailbox's language ([MS-OXOSFLD]
/// 3.1.4.1), and EWS has no distinguished id for it, so a mailbox without the English name is
/// asked which of its root's folders holds shortcuts. Kept for the session: the answer doesn't
/// change under us.
pub(super) fn common_views(c: &Client) -> Result<Option<String>> {
    if let Some(found) = c.common_views() {
        return Ok(found);
    }
    let found = match find_common_views(c) {
        Ok(found) => found,
        Err(e) => {
            // A probe that failed isn't made again this session: the folder isn't found by it,
            // and twenty-four probes at each sync would only say so again.
            log::info!("navigation shortcuts: the Common Views folder couldn't be read ({e})");
            c.keep_common_views(&None);
            return Err(e);
        }
    };
    c.keep_common_views(&found);
    Ok(found)
}

/// The folder among the mailbox root's own that holds navigation shortcuts. The English name
/// first (one answer), then each candidate asked for one shortcut.
fn find_common_views(c: &Client) -> Result<Option<String>> {
    let m = c.call_one(
        "<m:FindFolder Traversal=\"Shallow\"><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>\
         <t:FieldURI FieldURI=\"folder:DisplayName\"/></t:AdditionalProperties></m:FolderShape>\
         <m:IndexedPageFolderView MaxEntriesReturned=\"200\" Offset=\"0\" BasePoint=\"Beginning\"/>\
         <m:ParentFolderIds><t:DistinguishedFolderId Id=\"root\"/></m:ParentFolderIds></m:FindFolder>",
    )?;
    let mut folders = Vec::new();
    if let Some(rf) = m.child("RootFolder") {
        rf.all("Folder", &mut folders);
    }
    if let Some(id) = folders.iter().find(|f| f.text_of("DisplayName") == Some("Common Views")).and_then(|f| folder_id(f)) {
        log::info!("navigation shortcuts: the Common Views folder was found by its English name");
        return Ok(Some(id.to_string()));
    }
    // A mailbox in another language: the folder that holds the shortcuts says so itself. A
    // refused candidate doesn't stop the others; a real failure (the connection) is returned
    // when no folder was found.
    let mut refused = None;
    for f in folders.iter().take(MAX_PROBED) {
        let Some(id) = folder_id(f) else { continue };
        match holds_shortcuts(c, id) {
            Ok(true) => {
                log::info!("navigation shortcuts: the Common Views folder was found by the shortcut it holds");
                return Ok(Some(id.to_string()));
            }
            Ok(false) => {}
            Err(e) => {
                refused.get_or_insert(e);
            }
        }
    }
    match refused {
        Some(e) => Err(e),
        None => {
            log::info!("navigation shortcuts: no folder of the mailbox holds one");
            Ok(None)
        }
    }
}

/// The id of a folder in a `FindFolder` answer.
fn folder_id(f: &El) -> Option<&str> {
    f.child("FolderId").and_then(|id| id.attr("Id"))
}

/// Whether a folder holds at least one navigation shortcut: one asked for, by message class
/// ([MS-OXOCFG] 2.2.9.1).
fn holds_shortcuts(c: &Client, folder: &str) -> Result<bool> {
    let m = c.call_one(&format!(
        "<m:FindItem Traversal=\"Associated\"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape></m:ItemShape>\
         <m:IndexedPageItemView MaxEntriesReturned=\"1\" Offset=\"0\" BasePoint=\"Beginning\"/>\
         <m:Restriction><t:IsEqualTo><t:FieldURIOrConstant><t:FieldURI FieldURI=\"message:MessageClass\"/></t:FieldURIOrConstant>\
         <t:FieldURIOrConstant><t:Constant Value=\"{WLINK_CLASS}\"/></t:FieldURIOrConstant></t:IsEqualTo></m:Restriction>\
         <m:ParentFolderIds><t:FolderId Id=\"{}\"/></m:ParentFolderIds></m:FindItem>",
        esc(folder)
    ))?;
    Ok(m.find("Items").is_some_and(|items| !items.kids.is_empty()))
}

/// One shortcut as read, before it becomes one of the pane's calendars.
struct Wlink {
    section: Option<i64>,
    kind: Option<i64>,
    name: String,
    group_name: String,
    group_id: Option<String>,
    header_id: Option<String>,
    ordinal: Vec<u8>,
    address_book_eid: Option<Vec<u8>>,
}

/// The shortcuts, page after page, as the server gives them: the calendar ones kept, in the
/// order their ordinals give.
fn read_shortcuts(c: &Client, views: &str) -> Result<Shortcuts> {
    let props: String = PROPS.iter().map(|(tag, kind)| format!("<t:ExtendedFieldURI PropertyTag=\"{tag}\" PropertyType=\"{kind}\"/>")).collect();
    let mut items: Vec<El> = Vec::new();
    let mut pages = 0usize;
    for page in 0..MAX_PAGES {
        let offset = page * PAGE;
        let m = c.call_one(&format!(
            "<m:FindItem Traversal=\"Associated\"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape>\
             <t:AdditionalProperties>{props}</t:AdditionalProperties></m:ItemShape>\
             <m:IndexedPageItemView MaxEntriesReturned=\"{PAGE}\" Offset=\"{offset}\" BasePoint=\"Beginning\"/>\
             <m:Restriction><t:IsEqualTo><t:FieldURIOrConstant><t:FieldURI FieldURI=\"message:MessageClass\"/></t:FieldURIOrConstant>\
             <t:FieldURIOrConstant><t:Constant Value=\"{WLINK_CLASS}\"/></t:FieldURIOrConstant></t:IsEqualTo></m:Restriction>\
             <m:ParentFolderIds><t:FolderId Id=\"{}\"/></m:ParentFolderIds></m:FindItem>",
            esc(views)
        ))?;
        let batch: Vec<El> = m.find("Items").map(|i| i.kids.clone()).unwrap_or_default();
        let last = m.find("RootFolder").and_then(|r| r.attr("IncludesLastItemInRange")).is_some_and(|v| v.eq_ignore_ascii_case("true"));
        let n = batch.len();
        items.extend(batch);
        pages += 1;
        // A page shorter than asked for, or the server's own "no more", ends the read; so does
        // the page budget.
        if last || n < PAGE {
            break;
        }
    }
    Ok(shape(c, &items, pages))
}

/// The shortcuts of a section, grouped and ordered. The counts the log line says are taken from
/// all of them, the calendar ones only kept.
fn shape(c: &Client, items: &[El], pages: usize) -> Shortcuts {
    let mut by_section: BTreeMap<(i64, i64), usize> = BTreeMap::new();
    let mut wlinks: Vec<Wlink> = Vec::new();
    for it in items {
        let section = int_prop(it, "0x6852");
        let kind = int_prop(it, "0x6849");
        if let (Some(s), Some(k)) = (section, kind) {
            *by_section.entry((s, k)).or_default() += 1;
        }
        let is_link = ext_text(it, "0x001A").is_some_and(|class| class.trim().eq_ignore_ascii_case(WLINK_CLASS));
        if !is_link {
            continue;
        }
        wlinks.push(Wlink {
            section,
            kind,
            name: clean(ext_text(it, "0x0E1D").unwrap_or_default()),
            group_name: clean(ext_text(it, "0x6851").unwrap_or_default()),
            group_id: guid_prop(it, "0x6850"),
            header_id: guid_prop(it, "0x6842"),
            ordinal: bin_prop(it, "0x684B").unwrap_or_default(),
            address_book_eid: bin_prop(it, "0x6854"),
        });
    }
    // The group headers name their groups, whatever the members carry ([MS-OXOCFG] 2.2.9.3:
    // the members point at the header by its id, `PidTagWlinkGroupClsid`).
    let mut headers: HashMap<String, String> = HashMap::new();
    for w in &wlinks {
        if w.section == Some(CALENDAR_SECTION)
            && w.kind == Some(WLINK_HEADER)
            && let (Some(id), false) = (&w.header_id, w.name.is_empty())
        {
            headers.entry(id.clone()).or_insert_with(|| w.name.clone());
        }
    }
    // The ordinals order the shortcuts, lexicographically ([MS-OXOCFG] 2.2.9.7).
    wlinks.sort_by(|a, b| a.ordinal.cmp(&b.ordinal));
    // The groups, in Outlook's order: the headers by their ordinals, then a group a member
    // names without a header (it may hold only an entry that doesn't resolve).
    let group_of = |w: &Wlink| w.group_id.as_ref().and_then(|id| headers.get(id)).cloned().unwrap_or_else(|| w.group_name.clone());
    let mut group_headers: Vec<(String, Vec<u8>)> = wlinks
        .iter()
        .filter(|w| w.section == Some(CALENDAR_SECTION) && w.kind == Some(WLINK_HEADER) && !w.name.is_empty())
        .map(|w| (w.name.clone(), w.ordinal.clone()))
        .collect();
    group_headers.sort_by(|a, b| a.1.cmp(&b.1));
    let mut groups: Vec<String> = Vec::new();
    for (name, _) in group_headers {
        if !groups.iter().any(|g| g.eq_ignore_ascii_case(&name)) {
            groups.push(name);
        }
    }
    for w in &wlinks {
        if w.section != Some(CALENDAR_SECTION) || !matches!(w.kind, Some(WLINK_OWN | WLINK_SHARED)) {
            continue;
        }
        let group = group_of(w);
        if !group.is_empty() && !groups.iter().any(|g| g.eq_ignore_ascii_case(&group)) {
            groups.push(group);
        }
    }
    if groups.len() > MAX_GROUPS {
        groups.truncate(MAX_GROUPS);
    }
    let mut out: Vec<CalendarLink> = Vec::new();
    let mut resolved: HashMap<String, Option<String>> = HashMap::new();
    let (mut found, mut unresolved, mut not_looked) = (0usize, 0usize, 0usize);
    for w in &wlinks {
        if w.section != Some(CALENDAR_SECTION) {
            continue;
        }
        let own = match w.kind {
            Some(WLINK_OWN) => true,
            Some(WLINK_SHARED) => false,
            _ => continue,
        };
        let email = if own {
            String::new()
        } else {
            let Some(dn) = w.address_book_eid.as_deref().and_then(address_book_dn) else { continue };
            let email = match resolved.get(&dn) {
                Some(e) => e.clone(),
                None => {
                    let e = if resolved.len() < MAX_LOOKUPS {
                        match resolve(c, &dn) {
                            Some(e) => {
                                found += 1;
                                Some(e)
                            }
                            None => {
                                unresolved += 1;
                                None
                            }
                        }
                    } else {
                        not_looked += 1;
                        None
                    };
                    resolved.insert(dn, e.clone());
                    e
                }
            };
            let Some(email) = email.filter(|e| !e.eq_ignore_ascii_case(c.email())) else { continue };
            email
        };
        let group = group_of(w);
        let name = if w.name.is_empty() { email.clone() } else { w.name.clone() };
        let duplicate = if own {
            out.iter().any(|l| l.email.is_empty() && l.name.eq_ignore_ascii_case(&name) && l.group == group)
        } else {
            out.iter().any(|l| l.email.eq_ignore_ascii_case(&email))
        };
        if duplicate || out.len() >= MAX_LINKS {
            continue;
        }
        out.push(CalendarLink { email, name, group });
    }
    log::info!(
        "navigation shortcuts: {} item(s) in {pages} page(s) [{}], {} shortcut(s), {} group(s), {} calendar(s) kept, {found} address(es) resolved, {unresolved} not, {not_looked} not looked up",
        items.len(),
        sections(&by_section),
        wlinks.len(),
        groups.len(),
        out.len(),
    );
    Shortcuts { groups, links: out }
}

/// The section and type counts, as `section/type: n`, for the log line (never a name or an id).
fn sections(by: &BTreeMap<(i64, i64), usize>) -> String {
    let mut out = String::new();
    for ((section, kind), n) in by {
        if !out.is_empty() {
            out.push_str(", ");
        }
        let _ = write!(out, "{section}/{kind}: {n}");
    }
    out
}

/// An extended property's text, by its tag (as EWS gives it back).
fn ext_text<'a>(it: &'a El, tag: &str) -> Option<&'a str> {
    it.children("ExtendedProperty")
        .find(|p| p.child("ExtendedFieldURI").and_then(|u| u.attr("PropertyTag")).is_some_and(|t| crate::ews::is_tag(t, tag)))
        .and_then(|p| p.text_of("Value"))
}

/// An integer extended property, read only when it is one.
fn int_prop(it: &El, tag: &str) -> Option<i64> {
    let v = ext_text(it, tag)?.trim();
    if v.is_empty() || v.len() > 32 {
        return None;
    }
    v.parse::<i64>().ok()
}

/// A binary extended property, decoded only when short enough.
fn bin_prop(it: &El, tag: &str) -> Option<Vec<u8>> {
    let v = ext_text(it, tag)?.trim();
    if v.len() > MAX_BINARY {
        return None;
    }
    crate::ews::unb64(v)
}

/// A binary id's key: its 16 bytes in hex (EWS gives a `Binary` property as base64), or its text
/// lowercased when it isn't one.
fn guid_prop(it: &El, tag: &str) -> Option<String> {
    let v = ext_text(it, tag)?.trim();
    if v.is_empty() || v.len() > MAX_BINARY {
        return None;
    }
    Some(match crate::ews::unb64(v).filter(|b| b.len() == 16) {
        Some(b) => b.iter().fold(String::with_capacity(32), |mut s, byte| {
            let _ = write!(s, "{byte:02x}");
            s
        }),
        None => v.to_ascii_lowercase(),
    })
}

/// A name as the pane shows it: control and bidirectional characters out (a server's name may
/// carry either), trimmed, and bounded.
fn clean(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() && !matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'))
        .take(MAX_NAME)
        .collect::<String>()
        .trim()
        .to_string()
}

/// The X.500 name in an Exchange address book entry id ([MS-OXCDATA] 2.2.5.2: flags, the
/// provider's UID, version and type, then the name, ending in a zero byte).
pub(crate) fn address_book_dn(b: &[u8]) -> Option<String> {
    const PROVIDER: [u8; 16] = [0xDC, 0xA7, 0x40, 0xC8, 0xC0, 0x42, 0x10, 0x1A, 0xB4, 0xB9, 0x08, 0x00, 0x2B, 0x2F, 0xE1, 0x82];
    if b.get(4..20)? != PROVIDER {
        return None;
    }
    let rest = b.get(28..)?;
    let end = rest.iter().position(|c| *c == 0).unwrap_or(rest.len()).min(1024);
    let dn = std::str::from_utf8(rest.get(..end)?).ok()?.trim();
    (dn.starts_with('/') && dn.is_ascii()).then(|| dn.to_string())
}

/// The address behind an X.500 name, as Exchange's directory has it.
fn resolve(c: &Client, dn: &str) -> Option<String> {
    let m = c
        .call_one(&format!(
            "<m:ResolveNames ReturnFullContactData=\"false\" SearchScope=\"ActiveDirectory\"><m:UnresolvedEntry>{}</m:UnresolvedEntry></m:ResolveNames>",
            esc(dn)
        ))
        .ok()?;
    let email = m.find("Mailbox").and_then(|b| b.text_of("EmailAddress"))?.trim().to_ascii_lowercase();
    email.contains('@').then_some(email)
}
