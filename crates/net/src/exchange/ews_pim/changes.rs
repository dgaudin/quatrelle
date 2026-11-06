// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The account's own calendar and contacts read by what changed in them, as Outlook's Cached
//! Exchange Mode reads its folders: `SyncFolderItems` from the state the last sync ended at, kept
//! with the folder's entry here (its `ctag`, which a CalDAV collection's version also fills), then
//! only the items created or changed are asked for, and the rest is kept as it is
//! ([`DavSync::kept`]). The owner's 239 appointments were read whole at each change, 3 to 6 s
//! holding the connection new mail comes on (06/11).
//!
//! Read whole, once, when there's no state yet (a first sync, a profile from before), or when the
//! server refuses the one kept (lost, too old) or tells of more changes than are worth reading so:
//! the folder's state as of then is taken first, so what changes while it's read comes next time.
//! The engine keeps a sync's state with the appointments it read, saved together, so the state
//! kept is never newer than what's kept here; a sync that starts from an older one only reads
//! again what it read before.

use std::collections::HashSet;

use super::*;

/// Changes asked for at a time (Exchange answers 512 at most), and pages read at most: past
/// that, the folder is read whole.
const PAGE: usize = 512;
const PAGES: usize = 40;

/// What changed in a folder since a state: the items made or changed, those deleted, and the
/// state to start from next time.
#[derive(Debug, Default)]
pub(super) struct Changed {
    pub changed: Vec<String>,
    pub deleted: Vec<String>,
    pub state: String,
}

/// One page of `SyncFolderItems` in a distinguished folder (`calendar`, `contacts`) from `state`
/// (empty: from nothing, every item a creation): the changes in `into`, the state to go on from,
/// and whether that was the last page.
fn page(c: &Client, folder: &str, state: &str, into: &mut Changed) -> Result<(String, bool)> {
    let since = if state.is_empty() { String::new() } else { format!("<m:SyncState>{}</m:SyncState>", esc(state)) };
    let m = c.call_one(&format!(
        "<m:SyncFolderItems><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape></m:ItemShape>\
         <m:SyncFolderId><t:DistinguishedFolderId Id=\"{folder}\"/></m:SyncFolderId>{since}\
         <m:MaxChangesReturned>{PAGE}</m:MaxChangesReturned><m:SyncScope>NormalItems</m:SyncScope></m:SyncFolderItems>"
    ))?;
    let next = m
        .text_of("SyncState")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| NetError::No("no sync state in the answer".into()))?;
    let last = m.text_of("IncludesLastItemInRange").is_none_or(|v| v.trim() == "true");
    for ch in m.child("Changes").map(|c| c.kids.as_slice()).unwrap_or_default() {
        let id = ch.find("ItemId").and_then(|i| i.attr("Id")).map(str::to_string);
        match (ch.name.as_str(), id) {
            ("Create" | "Update", Some(id)) => {
                into.deleted.retain(|d| *d != id);
                if !into.changed.contains(&id) {
                    into.changed.push(id);
                }
            }
            ("Delete", Some(id)) => {
                into.changed.retain(|d| *d != id);
                if !into.deleted.contains(&id) {
                    into.deleted.push(id);
                }
            }
            _ => {}
        }
    }
    Ok((next, last))
}

/// What changed in a distinguished folder since `state`. `None`: the server refused the state (or
/// a page of it), or told of more changes than [`PAGES`] hold, so the folder is to be read whole.
pub(super) fn changes(c: &Client, folder: &str, state: &str) -> Result<Option<Changed>> {
    let mut out = Changed::default();
    let mut at = state.to_string();
    for _ in 0..PAGES {
        match page(c, folder, &at, &mut out) {
            Ok((next, last)) => {
                at = next;
                if last {
                    out.state = at;
                    return Ok(Some(out));
                }
            }
            Err(NetError::No(e) | NetError::Protocol(e)) => {
                log::info!("{folder}: its changes weren't given ({e}): read whole");
                return Ok(None);
            }
            Err(e) => return Err(e),
        }
        if out.changed.len() > MAX_ITEMS {
            break;
        }
    }
    log::info!("{folder}: more changes than are worth reading one by one: read whole");
    Ok(None)
}

/// A distinguished folder's state as of now (every page read, the items let go: only the state
/// matters). `None` when the server can't give one, or the folder is too big to follow so.
fn state_now(c: &Client, folder: &str) -> Result<Option<String>> {
    match changes(c, folder, "")? {
        Some(ch) => Ok(Some(ch.state)),
        None => Ok(None),
    }
}

/// Whether an appointment read belongs in the calendar kept here, as a whole read finds them: it
/// ends `since` or later, or it's a series.
fn in_window(it: &El, since: i64) -> bool {
    it.child("Recurrence").is_some()
        || it.text_of("CalendarItemType").is_some_and(|t| t.trim() == "RecurringMaster")
        || event_of(it).is_some_and(|e| e.end >= since)
}

/// The account's own calendar into `out`: by its changes when a state is kept, else whole. `again`:
/// appointments to read again whatever the server says (a deletion it refused: they come back).
/// The state to keep with it (empty: none, read whole next time too).
pub(super) fn read_own_calendar(c: &Client, pim: &Pim, now: i64, again: &[String], out: &mut DavSync) -> Result<String> {
    let since = now - CALENDAR_BACK;
    let kept = pim.calendars.iter().find(|k| k.id == CALENDAR).map(|k| k.ctag.as_str()).unwrap_or("");
    if !kept.is_empty()
        && let Some(ch) = changes(c, "calendar", kept)?
    {
        // Nothing changed: the state kept still says where to start (and the calendar kept here
        // isn't saved again for a new one).
        let state = if ch.changed.is_empty() && ch.deleted.is_empty() { kept.to_string() } else { ch.state };
        calendar_changes(c, pim, ch.changed, &ch.deleted, again, since, out)?;
        return Ok(state);
    }
    // Its state first: what changes while it's read comes next time.
    let state = state_now(c, "calendar")?.unwrap_or_default();
    read_calendar(c, items(c, "calendar", Some(since))?, CALENDAR, c.email(), since, out);
    out.synced_calendars.push(CALENDAR.into());
    Ok(state)
}

/// The calendar's changes made on what's kept here: the appointments made or changed read (a
/// series with its occurrences changed on their own), those deleted gone, the rest kept.
fn calendar_changes(
    c: &Client,
    pim: &Pim,
    mut changed: Vec<String>,
    deleted: &[String],
    again: &[String],
    since: i64,
    out: &mut DavSync,
) -> Result<()> {
    let began = std::time::Instant::now();
    for id in again {
        if !changed.contains(id) && !deleted.contains(id) {
            changed.push(id.clone());
        }
    }
    // One gone meanwhile isn't given: it goes as one deleted does.
    let found = if changed.is_empty() { Vec::new() } else { get_items(c, &changed, Shape::Appointment)? };
    let given = found.len();
    let wanted: Vec<El> = found.into_iter().filter(|it| in_window(it, since)).collect();
    let before = out.events.len();
    read_calendar(c, wanted, CALENDAR, c.email(), since, out);
    // The items touched, and the series among them: their occurrences kept here go with them (those
    // still changed on their own were just read again).
    let touched: HashSet<&str> = changed.iter().chain(deleted).map(String::as_str).collect();
    let mine = |e: &&StoredEvent| e.calendar == CALENDAR && !e.href.is_empty();
    let series: HashSet<&str> = pim
        .events
        .iter()
        .filter(mine)
        .filter(|e| e.event.recurrence_id.is_none() && touched.contains(e.href.as_str()))
        .map(|e| e.event.uid.as_str())
        .collect();
    let kept: Vec<&StoredEvent> = pim
        .events
        .iter()
        .filter(mine)
        .filter(|e| !(touched.contains(e.href.as_str()) || e.event.recurrence_id.is_some() && series.contains(e.event.uid.as_str())))
        .collect();
    if let Some(read) = out.events.get_mut(before..) {
        keep_apart(read, &kept);
    }
    out.kept.extend(kept.iter().map(|e| e.href.clone()));
    out.synced_calendars.push(CALENDAR.into());
    log::info!(
        "the calendar read by its changes in {:.1} s: {} made or changed ({given} given), {} deleted, {} kept as they were",
        began.elapsed().as_secs_f32(),
        changed.len(),
        deleted.len(),
        kept.len()
    );
    Ok(())
}

/// Plain appointments just read that share their UID with others kept here (a scheduling tool's
/// dates sent under its series' UID): named as [`quatrelle_model::pim::separate_shared_uids`]
/// named those, so each stays apart.
fn keep_apart(read: &mut [StoredEvent], kept: &[&StoredEvent]) {
    let mut taken: HashSet<String> = kept.iter().map(|e| e.event.uid.clone()).collect();
    for e in read.iter_mut().filter(|e| e.event.recurrence_id.is_none() && e.event.rrule.is_none()) {
        let base = e.event.uid.clone();
        let prefix = format!("{base}/");
        if !kept.iter().any(|k| k.event.recurrence_id.is_none() && (k.event.uid == base || k.event.uid.starts_with(&prefix))) {
            continue;
        }
        let mut uid = format!("{base}/{}", e.event.start);
        let mut n = 1;
        while taken.contains(&uid) && n < 1000 {
            n += 1;
            uid = format!("{base}/{}-{n}", e.event.start);
        }
        taken.insert(uid.clone());
        e.event.uid = uid;
    }
}

/// The account's contacts into `out`, under `name`: by their changes when a state is kept, else
/// whole. `again`: contacts to read again (a deletion the server refused).
pub(super) fn read_contacts(c: &Client, pim: &Pim, name: String, again: &[String], out: &mut DavSync) -> Result<()> {
    let book = pim.books.iter().find(|b| b.id == CONTACTS).map(|b| b.ctag.as_str()).unwrap_or("");
    let state = match (!book.is_empty()).then(|| changes(c, "contacts", book)).transpose()?.flatten() {
        Some(ch) => {
            let unchanged = ch.changed.is_empty() && ch.deleted.is_empty();
            let mut ids = ch.changed;
            for id in again.iter().filter_map(|h| h.strip_prefix("contact:")) {
                if !ids.iter().any(|i| i == id) && !ch.deleted.iter().any(|d| d == id) {
                    ids.push(id.to_string());
                }
            }
            let found = if ids.is_empty() { Vec::new() } else { get_items(c, &ids, Shape::All(""))? };
            out.cards.extend(stored_cards(&found));
            let touched: HashSet<String> = ids.iter().chain(&ch.deleted).map(|id| format!("contact:{id}")).collect();
            let kept = pim.cards.iter().filter(|k| k.book == CONTACTS && !k.href.is_empty() && !touched.contains(&k.href));
            out.kept.extend(kept.map(|k| k.href.clone()));
            log::info!("the contacts read by their changes: {} made or changed, {} deleted", ids.len(), ch.deleted.len());
            if unchanged { book.to_string() } else { ch.state }
        }
        None => {
            let state = state_now(c, "contacts")?.unwrap_or_default();
            let found = items(c, "contacts", None)?;
            out.cards.extend(stored_cards(&found));
            state
        }
    };
    out.books = Some(vec![BookInfo { id: CONTACTS.into(), name, read_only: false, ctag: state }]);
    out.synced_books.push(CONTACTS.into());
    Ok(())
}

/// Contacts read, each kept under its item with its change key (the item's `etag`).
fn stored_cards(found: &[El]) -> impl Iterator<Item = StoredCard> + '_ {
    found.iter().filter_map(|it| {
        let card = card_of(it)?;
        let etag = it.child("ItemId").and_then(|i| i.attr("ChangeKey")).unwrap_or("").to_string();
        Some(StoredCard { book: CONTACTS.into(), href: format!("contact:{}", card.uid), etag, card, dirty: false })
    })
}
