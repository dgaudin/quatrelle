// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Exchange's calendars, address books, notes and tasks read by what changed in them, as
//! Outlook's Cached Exchange Mode reads its folders: `SyncFolderItems` from the state the last sync
//! ended at, kept with the folder's entry here (its `ctag`, which a CalDAV collection's version
//! also fills; [`Pim::notes_state`] and [`Pim::tasks_state`] for the notes and the tasks), then only
//! the items created or changed are asked for, and the rest is kept as it is. The owner's 239
//! appointments were read whole at each change, 3 to 6 s holding the connection new mail comes on
//! (06/11); a colleague's calendar, the public folders, the notes and the tasks at every full sync.
//!
//! Read whole, once, when there's no state yet (a first sync, a profile from before), or when the
//! server refuses the one kept (lost, too old) or tells of more changes than are worth reading so:
//! the folder's state as of then is taken first, so what changes while it's read comes next time.
//! The engine keeps a sync's state with what it read, saved together, so the state kept is never
//! newer than what's kept here; a sync that starts from an older one only reads again what it read
//! before. A folder whose changes the server won't give at all (public folders, on some servers)
//! is read whole, as before, and not asked about again that session.
//!
//! The account's own calendar and contacts ([`own`]), other people's calendars and the public
//! folders ([`others`]), the notes and tasks ([`lists`]).

use std::collections::{HashMap, HashSet};

use super::*;

mod lists;
mod others;
mod own;

pub(super) use lists::{notes_read, tasks_read};
pub(super) use others::{Public, public_calendar, public_contacts, shared_calendar};
pub(super) use own::{read_contacts, read_own_calendar};

/// Changes asked for at a time (Exchange answers 512 at most), and pages read at most: past
/// that, the folder is read whole.
const PAGE: usize = 512;
const PAGES: usize = 40;

/// The codes of a refusal that says nothing of whether the server gives a folder's changes: the
/// state kept refused (read whole, then followed again), the folder out of reach or not shared any
/// more (said as a whole read says it). Any other refusal of a state from nothing: the server
/// won't follow this folder so.
const PASSING: [&str; 6] = [
    "ErrorInvalidSyncStateData",
    "ErrorAccessDenied",
    "ErrorFolderNotFound",
    "ErrorItemNotFound",
    "ErrorNonExistentMailbox",
    "ErrorMailboxMoveInProgress",
];

/// What a calendar's state kept here begins with since its appointments' files come with them
/// (R25): a calendar followed before has its appointments without their files, so its state is
/// none, and it's read whole once, its files with it.
const WITH_FILES: &str = "f1:";

/// A calendar's state as kept (its entry's `ctag`) to go on from: none for one kept before its
/// appointments' files were read.
pub(super) fn calendar_state(ctag: &str) -> &str {
    ctag.strip_prefix(WITH_FILES).unwrap_or("")
}

/// A calendar's state to keep (none stays none).
pub(super) fn keep_calendar_state(state: String) -> String {
    if state.is_empty() { state } else { format!("{WITH_FILES}{state}") }
}

/// The element naming one of the account's own special folders (`calendar`, `tasks`…).
pub(super) fn distinguished(id: &str) -> String {
    format!("<t:DistinguishedFolderId Id=\"{id}\"/>")
}

/// What changed in a folder since a state: the items made or changed, those deleted, and the
/// state to start from next time.
#[derive(Debug, Default)]
pub(super) struct Changed {
    pub changed: Vec<String>,
    pub deleted: Vec<String>,
    pub state: String,
}

/// One page of `SyncFolderItems`: the state to go on from and whether it was the last, or the
/// server's refusal (its code).
enum Page {
    Next(String, bool),
    Refused(String),
}

/// One page of `SyncFolderItems` in a folder (its element) from `state` (empty: from nothing,
/// every item a creation): each item told of in `into`, in order, with whether it was deleted
/// (`None`: only the state matters).
fn page(c: &Client, folder: &str, state: &str, into: Option<&mut Vec<(String, bool)>>) -> Result<Page> {
    let since = if state.is_empty() { String::new() } else { format!("<m:SyncState>{}</m:SyncState>", esc(state)) };
    let doc = c.call(&format!(
        "<m:SyncFolderItems><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape></m:ItemShape>\
         <m:SyncFolderId>{folder}</m:SyncFolderId>{since}\
         <m:MaxChangesReturned>{PAGE}</m:MaxChangesReturned><m:SyncScope>NormalItems</m:SyncScope></m:SyncFolderItems>"
    ))?;
    let m = crate::ews::messages(&doc).first().map(|m| (*m).clone()).ok_or_else(|| NetError::Protocol("an empty answer".into()))?;
    if m.attr("ResponseClass") == Some("Error") {
        // Busy and the like: for later, as any request.
        if let Err(e @ NetError::Later(_)) = crate::ews::check_message(&m) {
            return Err(e);
        }
        return Ok(Page::Refused(m.text_of("ResponseCode").unwrap_or("no code").trim().to_string()));
    }
    let Some(next) = m.text_of("SyncState").map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) else {
        return Ok(Page::Refused("no sync state in the answer".into()));
    };
    let last = m.text_of("IncludesLastItemInRange").is_none_or(|v| v.trim() == "true");
    if let Some(into) = into {
        for ch in m.child("Changes").map(|c| c.kids.as_slice()).unwrap_or_default() {
            let Some(id) = ch.find("ItemId").and_then(|i| i.attr("Id")) else { continue };
            match ch.name.as_str() {
                "Create" | "Update" => into.push((id.to_string(), false)),
                "Delete" => into.push((id.to_string(), true)),
                _ => {}
            }
        }
    }
    Ok(Page::Next(next, last))
}

/// Each item told of once, in the order first told, as it was last told of: made or changed, or
/// deleted.
fn settle(told: Vec<(String, bool)>, state: String) -> Changed {
    let last: HashMap<&str, bool> = told.iter().map(|(id, gone)| (id.as_str(), *gone)).collect();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut out = Changed { state, ..Default::default() };
    for (id, _) in &told {
        if seen.insert(id) {
            if last.get(id.as_str()).copied().unwrap_or(false) { out.deleted.push(id.clone()) } else { out.changed.push(id.clone()) }
        }
    }
    out
}

/// What changed in a folder (its element; `label` names it in the log) since `state` (empty: its
/// state as of now alone, the items let go). `None`: the server refused the state (or a page of
/// it), or told of more changes than [`PAGES`] hold, so the folder is to be read whole.
pub(super) fn changes(c: &Client, folder: &str, label: &str, state: &str) -> Result<Option<Changed>> {
    if c.changes_refused(folder) {
        return Ok(None);
    }
    let record = !state.is_empty();
    let mut told: Vec<(String, bool)> = Vec::new();
    let mut at = state.to_string();
    for _ in 0..PAGES {
        let refused = match page(c, folder, &at, record.then_some(&mut told)) {
            Ok(Page::Next(next, last)) => {
                at = next;
                if last {
                    return Ok(Some(settle(told, at)));
                }
                None
            }
            Ok(Page::Refused(code)) => Some(code),
            // A fault (a server whose schema doesn't take the request).
            Err(NetError::No(e) | NetError::Protocol(e)) => Some(e),
            Err(e) => return Err(e),
        };
        if let Some(why) = refused {
            if !record && !PASSING.contains(&why.as_str()) {
                log::info!("{label}: the server won't give its changes ({why}): read whole, as before, this session");
                c.note_changes_refused(folder);
            } else {
                log::info!("{label}: its changes weren't given ({why}): read whole");
            }
            return Ok(None);
        }
        if told.len() > MAX_ITEMS {
            break;
        }
    }
    if record {
        log::info!("{label}: more changes than are worth reading one by one: read whole");
    } else {
        // Every sync would page through it again, then read it whole.
        log::info!("{label}: too big to follow by its changes: read whole, as before, this session");
        c.note_changes_refused(folder);
    }
    Ok(None)
}

/// A folder's state as of now (every page read, the items let go: only the state matters). `None`
/// when the server can't give one, or the folder is too big to follow so.
pub(super) fn state_now(c: &Client, folder: &str, label: &str) -> Result<Option<String>> {
    Ok(changes(c, folder, label, "")?.map(|ch| ch.state))
}

/// A folder's changes since the state kept, read: the items made or changed (and those to read
/// again whatever the server says, `again`: a deletion it refused) as the server gives them (one
/// gone meanwhile isn't), the items touched (those, and those deleted), and the state to keep.
pub(super) struct Read {
    pub found: Vec<El>,
    pub touched: HashSet<String>,
    pub changed: usize,
    pub deleted: usize,
    pub state: String,
    pub began: Instant,
}

/// A folder read by its changes since `kept`, its items asked as `shape` says. `None`: no state
/// kept, or it's to be read whole ([`changes`]).
pub(super) fn read_changes(c: &Client, folder: &str, label: &str, kept: &str, again: &[String], shape: Shape) -> Result<Option<Read>> {
    let began = Instant::now();
    if kept.is_empty() {
        return Ok(None);
    }
    let Some(ch) = changes(c, folder, label, kept)? else { return Ok(None) };
    // Nothing changed: the state kept still says where to start (and what's kept here isn't saved
    // again for a new one).
    let unchanged = ch.changed.is_empty() && ch.deleted.is_empty();
    let mut ids = ch.changed;
    for id in again {
        if !ids.contains(id) && !ch.deleted.contains(id) {
            ids.push(id.clone());
        }
    }
    let found = if ids.is_empty() { Vec::new() } else { get_items(c, &ids, shape)? };
    let touched: HashSet<String> = ids.iter().chain(&ch.deleted).cloned().collect();
    Ok(Some(Read {
        found,
        touched,
        changed: ids.len(),
        deleted: ch.deleted.len(),
        state: if unchanged { kept.to_string() } else { ch.state },
        began,
    }))
}

/// Whether an appointment read belongs in a calendar kept here, as a whole read finds them: it
/// ends `since` or later, or it's a series.
fn in_window(it: &El, since: i64) -> bool {
    it.child("Recurrence").is_some()
        || it.text_of("CalendarItemType").is_some_and(|t| t.trim() == "RecurringMaster")
        || event_of(it).is_some_and(|e| e.end >= since)
}

/// A calendar read by its changes into `out` (`read`, the appointments `calendar` keeps here in
/// `pim`): those made or changed (a series with its occurrences changed on their own) as
/// [`read_calendar`] reads them, those deleted gone, the rest kept — by their items (`kept`) for
/// the account's own calendar, which stays in memory, or (`copies`) as they are, for one whose
/// appointments may be set aside while the calendar isn't shown (other people's): what's kept is
/// then given whole, as a whole read gives it. The appointments kept, for the log.
fn calendar_changes(c: &Client, pim: &Pim, read: Read, calendar: &str, owner: &str, since: i64, copies: bool, out: &mut DavSync) -> usize {
    let wanted: Vec<El> = read.found.into_iter().filter(|it| in_window(it, since)).collect();
    let before = out.events.len();
    read_calendar(c, wanted, calendar, owner, since, out);
    // The items touched, and the series among them: their occurrences kept here go with them
    // (those still changed on their own were just read again).
    let touched = &read.touched;
    let mine = |e: &&StoredEvent| e.calendar == calendar && !e.href.is_empty();
    let series: HashSet<&str> = pim
        .events
        .iter()
        .filter(mine)
        .filter(|e| e.event.recurrence_id.is_none() && touched.contains(&e.href))
        .map(|e| e.event.uid.as_str())
        .collect();
    let kept: Vec<&StoredEvent> = pim
        .events
        .iter()
        .filter(mine)
        // One changed here and not yet sent is the engine's: it keeps it.
        .filter(|e| !(copies && e.dirty))
        .filter(|e| !(touched.contains(&e.href) || e.event.recurrence_id.is_some() && series.contains(e.event.uid.as_str())))
        .collect();
    if let Some(read) = out.events.get_mut(before..) {
        keep_apart(read, &kept);
    }
    if copies {
        out.events.extend(kept.iter().map(|e| (*e).clone()));
    } else {
        out.kept.extend(kept.iter().map(|e| e.href.clone()));
    }
    out.synced_calendars.push(calendar.into());
    kept.len()
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
