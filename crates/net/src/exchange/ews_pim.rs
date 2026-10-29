//! On-premises Exchange calendars and contacts through Exchange Web Services, synced into the
//! same calendars and address books as CalDAV and CardDAV ([`DavSync`], which the engine merges).
//!
//! The mailbox's Calendar and Contacts folders: appointments (with their recurrence, read as the
//! pattern and range Graph also uses, so [`crate::graph_pim::rrule_of`] makes the RRULE) and
//! contacts. What changed here goes first: new appointments and contacts are created, changed
//! appointments updated field by field (an occurrence changed on its own through the
//! occurrence's own item), changed contacts made again, deletions done; the server mails the
//! guests of a meeting only when the change asks it to ([`StoredEvent::notify`]). Then both
//! folders are read again, a series' changed occurrences with it. Notes and tasks go with them
//! (the Notes and Tasks folders).

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use quatrelle_model::ical::{Attendee, CalEvent, ShowAs};
use quatrelle_model::pim::{BookInfo, CalendarInfo, DavSync, Note, Pim, StoredCard, StoredEvent, StoredNote};
use quatrelle_model::vcard::{Card, PostalAddress, Typed};
use quatrelle_model::zone::Zone;
use serde_json::{Value, json};

use crate::ews::{Client, El, esc};
use crate::graph_pim::{recurrence_of, rrule_of};
use crate::{NetError, Result};

/// The calendar and address book Exchange's folders go into.
pub const CALENDAR: &str = "ews:calendar";
pub const CONTACTS: &str = "ews:contacts";
/// Items read from a folder at most, and per `GetItem`.
const MAX_ITEMS: usize = 5000;
const BATCH: usize = 50;
/// Connections a read of many items is shared among (see [`get_items`]).
const LANES: usize = 4;
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

pub(crate) fn utc(t: i64) -> String {
    let (y, m, d) = quatrelle_model::dates::civil_from_days(t.div_euclid(86_400));
    let s = t.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", s / 3600, s / 60 % 60, s % 60)
}

fn text(e: &El, name: &str) -> String {
    e.text_of(name).unwrap_or_default().trim().to_string()
}

/// Public contact folders and calendars read, at most.
const MAX_PUBLIC_PIM: usize = 20;

/// How far back calendars are read: Outlook's own cached mode keeps about this much, and
/// reading every meeting of every colleague's calendar since it began never ended.
pub(crate) const CALENDAR_BACK: i64 = 183 * 86_400;

/// A calendar's items that matter from `since` on: those ending then or later, and every
/// recurring series (its first meeting may be long before; [MS-OXOCAL] PidLidRecurring, which
/// Exchange lets a search ask about, where it refuses CalendarItemType) — not the years before.
/// `strict`: the ones ending then or later alone, for a server that refuses the other.
fn calendar_restriction(since: i64, strict: bool) -> String {
    let ends = format!(
        "<t:IsGreaterThanOrEqualTo><t:FieldURI FieldURI=\"calendar:End\"/>\
         <t:FieldURIOrConstant><t:Constant Value=\"{}\"/></t:FieldURIOrConstant></t:IsGreaterThanOrEqualTo>",
        utc(since)
    );
    if strict {
        return format!("<m:Restriction>{ends}</m:Restriction>");
    }
    format!(
        "<m:Restriction><t:Or>{ends}<t:IsEqualTo><t:ExtendedFieldURI DistinguishedPropertySetId=\"Appointment\" PropertyId=\"33315\" PropertyType=\"Boolean\"/>\
         <t:FieldURIOrConstant><t:Constant Value=\"true\"/></t:FieldURIOrConstant></t:IsEqualTo></t:Or></m:Restriction>"
    )
}

/// Every item of a distinguished folder, with all its properties (bodies as text); a
/// calendar's from `since` on.
fn items(c: &Client, folder: &str, since: Option<i64>) -> Result<Vec<El>> {
    items_in(c, &format!("<t:DistinguishedFolderId Id=\"{folder}\"/>"), if folder == "notes" { NOTE_COLOR } else { "" }, since)
}

/// Every item of a folder (given as its EWS folder id element), with all its properties.
fn items_in(c: &Client, parent: &str, extra: &str, since: Option<i64>) -> Result<Vec<El>> {
    match (since, items_in_from(c, parent, extra, since.map(|t| calendar_restriction(t, false)))) {
        // A server that won't search so: what ends from then on, then everything.
        (Some(t), Err(NetError::No(e) | NetError::Protocol(e))) => {
            log::info!("calendar search refused ({e}): asking more simply");
            match items_in_from(c, parent, extra, Some(calendar_restriction(t, true))) {
                Err(NetError::No(e) | NetError::Protocol(e)) => {
                    log::info!("calendar search refused again ({e}): reading it all");
                    items_in_from(c, parent, extra, None)
                }
                r => r,
            }
        }
        (_, r) => r,
    }
}

fn items_in_from(c: &Client, parent: &str, extra: &str, restriction: Option<String>) -> Result<Vec<El>> {
    let folder = parent;
    let began = Instant::now();
    let before = c.spent();
    let restriction = restriction.unwrap_or_default();
    let mut ids: Vec<String> = Vec::new();
    let mut offset = 0;
    loop {
        let m = c.call_one(&format!(
            "<m:FindItem Traversal=\"Shallow\"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape></m:ItemShape>\
             <m:IndexedPageItemView MaxEntriesReturned=\"500\" Offset=\"{offset}\" BasePoint=\"Beginning\"/>{restriction}\
             <m:ParentFolderIds>{folder}</m:ParentFolderIds></m:FindItem>"
        ))?;
        let Some(rf) = m.child("RootFolder") else { break };
        let page: Vec<String> =
            rf.child("Items").map(|i| i.kids.iter().filter_map(|k| k.child("ItemId")?.attr("Id").map(str::to_string)).collect()).unwrap_or_default();
        let n = page.len();
        ids.extend(page);
        offset += n;
        if n == 0 || rf.attr("IncludesLastItemInRange").is_none_or(|v| v == "true") || ids.len() >= MAX_ITEMS {
            break;
        }
    }
    let out = get_items(c, &ids, extra)?;
    // Where the time went, the connections' added up: the server's part, and Quatrelle's.
    let t = c.spent().since(before);
    log::info!(
        "{} items read in {:.1} s: {} requests over {} connections, {:.1} s waiting for the server, {:.1} s receiving {:.1} MB, {:.1} s reading it",
        out.len(),
        began.elapsed().as_secs_f32(),
        t.requests,
        batches(&ids).len().clamp(1, LANES),
        t.server.as_secs_f32(),
        t.receiving.as_secs_f32(),
        t.bytes as f64 / 1e6,
        t.reading.as_secs_f32()
    );
    Ok(out)
}

/// Items by id, with all their properties (bodies as text), in batches; one the server can't
/// give (gone meanwhile) is left out. An appointment's pattern and time zone come only when
/// asked for by name (`AllProperties` leaves them out).
///
/// Many batches are asked over [`LANES`] connections at once: asked one after the other, a
/// colleague's calendar (700 items) took 8 s, and a calendar just ticked showed a quarter of a
/// minute later (29/10). Each connection takes the next batch not yet asked, so a batch the
/// server is slow to answer holds back only its own (each taking every fourth, one held for ten
/// seconds kept four more waiting: 869 items in 15 s); and the batches are made alike, as many as
/// give each connection the same share (241 occurrences in five had one connection ask two, 6.5 s
/// where 4 do). Each answer is in the order asked, whatever the connection; a batch one of the
/// others couldn't ask (it didn't sign in, say) is asked on this one.
fn get_items(c: &Client, ids: &[String], extra: &str) -> Result<Vec<El>> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let batches = batches(ids);
    let lanes = batches.len().min(LANES);
    if lanes <= 1 {
        return get_batches(c, &batches, extra);
    }
    let next = AtomicUsize::new(0);
    let take = || {
        let i = next.fetch_add(1, Ordering::Relaxed);
        batches.get(i).map(|b| (i, *b))
    };
    let mut answers: Vec<Option<Vec<El>>> = vec![None; batches.len()];
    // How long each batch took, connection by connection (this one first), for the log.
    let mut took: Vec<Vec<Duration>> = Vec::with_capacity(lanes);
    std::thread::scope(|s| -> Result<()> {
        let others: Vec<_> = (1..lanes)
            .map(|n| {
                let (sibling, take) = (c.sibling(), &take);
                s.spawn(move || {
                    let (mut got, mut times) = (Vec::new(), Vec::new());
                    while let Some((i, b)) = take() {
                        let began = Instant::now();
                        let answer = get_batch(&sibling, b, extra);
                        times.push(began.elapsed());
                        match answer {
                            Ok(items) => got.push((i, items)),
                            // It stops there: the batch is asked again on the first.
                            Err(e) => {
                                log::info!("ews: connection {} of {lanes} couldn't read {} items ({e})", n + 1, b.len());
                                break;
                            }
                        }
                    }
                    (got, times, sibling.spent())
                })
            })
            .collect();
        let (mut mine, mut failed) = (Vec::new(), None);
        while let Some((i, b)) = take() {
            let began = Instant::now();
            let answer = get_batch(c, b, extra);
            mine.push(began.elapsed());
            match answer {
                Ok(items) => {
                    if let Some(slot) = answers.get_mut(i) {
                        *slot = Some(items);
                    }
                }
                // The others stop too: the read fails.
                Err(e) => {
                    next.store(batches.len(), Ordering::Relaxed);
                    failed = Some(e);
                    break;
                }
            }
        }
        took.push(mine);
        for h in others {
            let (got, times, spent) = h.join().unwrap_or_default();
            c.count_spent(spent);
            took.push(times);
            for (i, items) in got {
                if let Some(slot) = answers.get_mut(i) {
                    *slot = Some(items);
                }
            }
        }
        failed.map_or(Ok(()), Err)
    })?;
    let again = answers.iter().filter(|a| a.is_none()).count();
    log::info!("ews: {} batches over {lanes} connections: {}", batches.len(), lanes_said(&took, again));
    let mut out = Vec::new();
    for (a, b) in answers.into_iter().zip(&batches) {
        match a {
            Some(items) => out.extend(items),
            None => out.extend(get_batch(c, b, extra)?),
        }
    }
    Ok(out)
}

/// The batches `ids` are asked in: at most [`BATCH`] items each and, more than one, as many as
/// give each of the [`LANES`] connections the same share, sized alike.
fn batches(ids: &[String]) -> Vec<&[String]> {
    let n = ids.len().div_ceil(BATCH);
    if n <= 1 {
        return ids.chunks(BATCH).collect();
    }
    ids.chunks(ids.len().div_ceil(n.next_multiple_of(LANES))).collect()
}

/// What each connection of [`get_items`] did, for the log: how long its batches took and how
/// many it asked, the slowest batch, and how many are asked again on the first.
fn lanes_said(took: &[Vec<Duration>], again: usize) -> String {
    let each: Vec<String> = took.iter().map(|t| format!("{:.1} s ({})", t.iter().sum::<Duration>().as_secs_f32(), t.len())).collect();
    let slowest = took.iter().flatten().max().copied().unwrap_or_default();
    let mut said = format!("each done in {} (batches in brackets), the slowest batch in {:.1} s", each.join(", "), slowest.as_secs_f32());
    if again > 0 {
        said.push_str(&format!("; {again} asked again on the first"));
    }
    said
}

/// Batches of item ids asked one after the other, on one connection.
fn get_batches(c: &Client, batches: &[&[String]], extra: &str) -> Result<Vec<El>> {
    let mut out = Vec::new();
    for b in batches {
        out.extend(get_batch(c, b, extra)?);
    }
    Ok(out)
}

/// One batch of items by id, with all their properties.
fn get_batch(c: &Client, chunk: &[String], extra: &str) -> Result<Vec<El>> {
    let mut out = Vec::new();
    let list: String = chunk.iter().map(|id| format!("<t:ItemId Id=\"{}\"/>", esc(id))).collect();
    let doc = c.call(&format!(
        "<m:GetItem><m:ItemShape><t:BaseShape>AllProperties</t:BaseShape><t:BodyType>Text</t:BodyType>\
         <t:AdditionalProperties><t:FieldURI FieldURI=\"calendar:Recurrence\"/><t:FieldURI FieldURI=\"calendar:StartTimeZone\"/>\
         <t:FieldURI FieldURI=\"item:EffectiveRights\"/>{}</t:AdditionalProperties></m:ItemShape>\
         <m:ItemIds>{list}</m:ItemIds></m:GetItem>",
        extra
    ))?;
    // What the server refused, by its code: said in the log (a colleague's occurrences were
    // all refused, without a word, 29/10).
    let mut refused: BTreeMap<String, usize> = BTreeMap::new();
    for m in crate::ews::messages(&doc) {
        if m.attr("ResponseClass") == Some("Error") {
            *refused.entry(m.text_of("ResponseCode").unwrap_or("no code").trim().to_string()).or_default() += 1;
            continue;
        }
        if let Some(items) = m.child("Items") {
            out.extend(items.kids.iter().cloned());
        }
    }
    if !refused.is_empty() {
        let said: Vec<String> = refused.iter().map(|(code, n)| format!("{code} ×{n}")).collect();
        log::info!("ews: {} of {} items not given: {}", refused.values().sum::<usize>(), chunk.len(), said.join(", "));
    }
    Ok(out)
}

fn new_id(m: &El) -> (String, String) {
    let id = m.find("ItemId");
    (id.and_then(|i| i.attr("Id")).unwrap_or("").to_string(), id.and_then(|i| i.attr("ChangeKey")).unwrap_or("").to_string())
}

/// An item deleted (to Deleted Items, as Outlook deletes). Exchange won't delete a task without
/// being told which of its occurrences go: all of them, as Outlook deletes a task.
fn delete(c: &Client, id: &str, task: bool) -> Result<()> {
    let occurrences = if task { " AffectedTaskOccurrences=\"AllOccurrences\"" } else { "" };
    c.call_one(&format!(
        "<m:DeleteItem DeleteType=\"MoveToDeletedItems\" SendMeetingCancellations=\"SendToNone\"{occurrences}><m:ItemIds><t:ItemId Id=\"{}\"/></m:ItemIds></m:DeleteItem>",
        esc(id)
    ))
    .map(drop)
}

/// A calendar Outlook keeps in a group of its Calendar pane (a colleague's in a team's group, a
/// room's…): whose it is, the name shown, and the group's name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CalendarLink {
    pub email: String,
    pub name: String,
    pub group: String,
}

/// Navigation shortcuts read at most.
const MAX_LINKS: usize = 300;

/// The calendars Outlook keeps in groups of its Calendar pane. Outlook stores its groups as
/// navigation shortcuts ([MS-OXOCFG] 2.2.9): hidden items of the mailbox's Common Views folder,
/// each naming its group (`PidTagWlinkGroupName`), its section (3: Calendar), its kind (2: a
/// folder another user owns) and that user's address book entry (`PidTagWlinkAddressBookEID`),
/// whose X.500 name Exchange resolves to an address.
pub(crate) fn calendar_links(c: &Client) -> Result<Vec<CalendarLink>> {
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
    let Some(views) = folders.iter().find(|f| f.text_of("DisplayName") == Some("Common Views")).and_then(|f| f.child("FolderId")?.attr("Id")) else {
        return Ok(Vec::new());
    };
    let prop = |tag: &str, kind: &str| format!("<t:ExtendedFieldURI PropertyTag=\"{tag}\" PropertyType=\"{kind}\"/>");
    let m = c.call_one(&format!(
        "<m:FindItem Traversal=\"Associated\"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>{}{}{}{}{}{}\
         </t:AdditionalProperties></m:ItemShape><m:IndexedPageItemView MaxEntriesReturned=\"{MAX_LINKS}\" Offset=\"0\" BasePoint=\"Beginning\"/>\
         <m:ParentFolderIds><t:FolderId Id=\"{}\"/></m:ParentFolderIds></m:FindItem>",
        prop("0x001A", "String"),
        prop("0x0E1D", "String"),
        prop("0x6849", "Integer"),
        prop("0x6851", "String"),
        prop("0x6852", "Integer"),
        prop("0x6854", "Binary"),
        esc(views)
    ))?;
    let items: Vec<El> = m.find("Items").map(|i| i.kids.clone()).unwrap_or_default();
    let mut resolved: HashMap<String, Option<String>> = HashMap::new();
    let mut out: Vec<CalendarLink> = Vec::new();
    for it in items.iter().take(MAX_LINKS) {
        let text = |tag: &str| ext_text(it, tag);
        let int = |tag: &str| text(tag).and_then(|v| v.trim().parse::<i64>().ok());
        // A calendar (section 3) another user owns (kind 2), in a group.
        if text("0x001A") != Some("IPM.Microsoft.WunderBar.Link") || int("0x6852") != Some(3) || int("0x6849") != Some(2) {
            continue;
        }
        let Some(dn) = text("0x6854").and_then(|v| crate::ews::unb64(v.trim())).and_then(|b| address_book_dn(&b)) else { continue };
        let email = match resolved.get(&dn) {
            Some(e) => e.clone(),
            None => {
                let e = resolve(c, &dn);
                resolved.insert(dn.clone(), e.clone());
                e
            }
        };
        let Some(email) = email.filter(|e| !e.eq_ignore_ascii_case(c.email())) else { continue };
        if out.iter().any(|l| l.email == email) {
            continue;
        }
        let name = text("0x0E1D").map(str::trim).filter(|n| !n.is_empty()).unwrap_or(&email).to_string();
        out.push(CalendarLink { email, name, group: text("0x6851").unwrap_or_default().trim().to_string() });
    }
    Ok(out)
}

/// An extended property's value, by its tag (`0x6851`, as EWS gives it back).
fn ext_text<'a>(it: &'a El, tag: &str) -> Option<&'a str> {
    it.children("ExtendedProperty")
        .find(|p| p.child("ExtendedFieldURI").and_then(|u| u.attr("PropertyTag")).is_some_and(|t| crate::ews::is_tag(t, tag)))
        .and_then(|p| p.text_of("Value"))
}

/// The X.500 name in an Exchange address book entry id ([MS-OXCDATA] 2.2.5.2: flags, the
/// provider's UID, version and type, then the name, ending in a zero byte).
fn address_book_dn(b: &[u8]) -> Option<String> {
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

/// The calendar folder: this mailbox's, or someone else's by their address.
/// The name the server gives one of the account's own special folders (`calendar`, `contacts`):
/// the mailbox's language's ("Agenda", "Contactpersonen"). `None` when it doesn't say.
fn folder_name(c: &Client, distinguished: &str) -> Option<String> {
    let m = c
        .call_one(&format!(
            "<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>\
             <t:FieldURI FieldURI=\"folder:DisplayName\"/></t:AdditionalProperties></m:FolderShape>\
             <m:FolderIds><t:DistinguishedFolderId Id=\"{distinguished}\"/></m:FolderIds></m:GetFolder>"
        ))
        .ok()?;
    let name: String = m.find("DisplayName")?.text.trim().chars().take(120).collect();
    (!name.is_empty()).then_some(name)
}

fn calendar_folder(owner: Option<&str>) -> String {
    match owner {
        Some(email) => format!(
            "<t:DistinguishedFolderId Id=\"calendar\"><t:Mailbox><t:EmailAddress>{}</t:EmailAddress></t:Mailbox></t:DistinguishedFolderId>",
            esc(email)
        ),
        None => "<t:DistinguishedFolderId Id=\"calendar\"/>".into(),
    }
}

/// Whether this account may add items to a folder (its EffectiveRights): `None` when the server
/// doesn't say. A folder's `Modify` is about the folder itself (its name and permissions, the
/// owner's alone), so an editor is known by `CreateContents`.
fn may_edit(c: &Client, folder: &str) -> Result<bool> {
    let m = c.call_one(&format!(
        "<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>\
         <t:FieldURI FieldURI=\"folder:EffectiveRights\"/></t:AdditionalProperties></m:FolderShape>\
         <m:FolderIds>{folder}</m:FolderIds></m:GetFolder>"
    ))?;
    Ok(m.find("CreateContents").is_some_and(|e| e.text.trim().eq_ignore_ascii_case("true")))
}

/// Whether an item says this account may change it (its own EffectiveRights).
fn item_may_change(it: &El) -> bool {
    it.child("EffectiveRights").and_then(|r| r.child("Modify")).is_some_and(|m| m.text.trim().eq_ignore_ascii_case("true"))
}

/// A public folder's items. On Exchange 2013 and later its content is kept in a public folder
/// mailbox, and reading it is routed there (asked for by the GUID the folder names); a server
/// that answers without that (older ones, a single public folder mailbox) is asked plainly
/// first.
fn public_items(c: &Client, id: &str, replica: Option<&str>, total: u32, since: Option<i64>) -> Result<Vec<El>> {
    let folder = format!("<t:FolderId Id=\"{}\"/>", esc(id));
    let plain = items_in(c, &folder, "", since);
    let Some(guid) = replica else { return plain };
    match plain {
        Ok(list) if !list.is_empty() || total == 0 => Ok(list),
        plain => match c.public_folder_mailbox(guid) {
            Ok(mailbox) => c.routed_to(&mailbox, || items_in(c, &folder, "", since)),
            Err(e) => {
                log::info!("public folder mailbox {guid}: {e}");
                plain
            }
        },
    }
}

mod categories;
mod contacts;
pub(crate) mod events;
mod push;
pub mod scheduling;
mod sync;
mod tasks;

pub use categories::master_categories;
use contacts::{NOTE_COLOR, contact_xml, note_xml};
pub use contacts::{card_of, note_of};
use events::{event_fields, listed_exception, modified_occurrences, occurrence_item_id};
pub use events::{event_of, exception_of, own_organizer};
use push::{answer_pending, push_events};
pub use sync::{sync, sync_calendars, sync_tasks};
use tasks::read_tasks;

#[cfg(test)]
mod tests;
