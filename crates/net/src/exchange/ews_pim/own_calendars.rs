// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The mailbox's own other calendars: the folders of appointments under its Calendar (at any
//! depth), as Outlook lists them under My Calendars. Listed with one `FindFolder` per sync, each
//! read by its changes as the Calendar is ([`read_own_folder`]), one more `SyncFolderItems` each;
//! read only while the calendar is shown, as other people's are, and one unticked listed as it
//! was, not read. One the server refuses is passed over, listed as it was.

use quatrelle_model::pim::{Tombstone, plain_name};

use super::changes::read_own_folder;
use super::sync::again_in;
use super::*;

/// The account's other calendars read at most: past this, the rest is left out (said in the log).
pub(super) const MAX_OWN_CALENDARS: usize = 64;

/// Folders under the Calendar asked for at most (calendars and anything else kept there).
const LISTED: usize = 4 * MAX_OWN_CALENDARS;

/// The longest calendar name kept, in characters (as the Calendar's own: [`folder_name`]).
const NAME_CHARS: usize = 120;

/// One of the account's other calendars, as listed: its folder's id and its name made plain.
pub(super) struct OwnFolder {
    pub id: String,
    pub name: String,
}

/// The folders of appointments under the account's Calendar, at most [`MAX_OWN_CALENDARS`].
pub(super) fn list(c: &Client) -> Result<Vec<OwnFolder>> {
    let m = c.call_one(&format!(
        "<m:FindFolder Traversal=\"Deep\"><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>\
         <t:FieldURI FieldURI=\"folder:DisplayName\"/><t:FieldURI FieldURI=\"folder:FolderClass\"/></t:AdditionalProperties></m:FolderShape>\
         <m:IndexedPageFolderView MaxEntriesReturned=\"{LISTED}\" Offset=\"0\" BasePoint=\"Beginning\"/>\
         <m:ParentFolderIds><t:DistinguishedFolderId Id=\"calendar\"/></m:ParentFolderIds></m:FindFolder>"
    ))?;
    let found = m.child("RootFolder").and_then(|r| r.child("Folders")).map(|f| f.kids.as_slice()).unwrap_or_default();
    let mut out = Vec::new();
    let mut more = 0usize;
    for f in found {
        let class = f.text_of("FolderClass").unwrap_or("").trim();
        let Some(id) = f.child("FolderId").and_then(|i| i.attr("Id")).filter(|id| !id.is_empty()) else { continue };
        if !class.starts_with("IPF.Appointment") {
            continue;
        }
        if out.len() >= MAX_OWN_CALENDARS {
            more += 1;
            continue;
        }
        let name: String = plain_name(f.text_of("DisplayName").unwrap_or("")).chars().take(NAME_CHARS).collect();
        out.push(OwnFolder { id: id.to_string(), name: if name.is_empty() { "Calendar".into() } else { name } });
    }
    if more > 0 {
        log::info!("{} other calendars under the calendar: the first {MAX_OWN_CALENDARS} read, {more} left out", MAX_OWN_CALENDARS + more);
    }
    Ok(out)
}

/// The account's other calendars into `calendars` (their entries) and `out` (their appointments):
/// each read by its changes, one unticked (`Pim::not_read`) listed as it was; one the server
/// refuses passed over, listed as it was. A listing that fails keeps them all as they were: none
/// goes for it. Only what stops the whole sync (the sign-in refused) is returned.
pub(super) fn read_all(c: &Client, pim: &Pim, now: i64, refused: &[Tombstone], calendars: &mut Vec<CalendarInfo>, out: &mut DavSync) -> Result<()> {
    let listed = match list(c) {
        Ok(l) => l,
        Err(e) if e.is_fatal() => return Err(e),
        Err(e) => {
            log::info!("the account's other calendars weren't listed ({e}): kept as they were");
            calendars.extend(pim.calendars.iter().filter(|k| own_folder(&k.id).is_some()).cloned());
            return Ok(());
        }
    };
    for (n, f) in listed.into_iter().enumerate() {
        let id = format!("{FOLDER}{}", f.id);
        let before = pim.calendars.iter().find(|k| k.id == id);
        let entry = CalendarInfo {
            id: id.clone(),
            name: f.name,
            color: before.map_or_else(|| u8::try_from((n + 2) % 8).unwrap_or(0), |k| k.color),
            read_only: false,
            ctag: before.map(|k| k.ctag.clone()).unwrap_or_default(),
            group: String::new(),
            main: false,
            not_own: false,
        };
        if pim.not_read.contains(&id) {
            calendars.push(entry);
            continue;
        }
        let folder = format!("<t:FolderId Id=\"{}\"/>", esc(&f.id));
        match read_own_folder(c, pim, now, &again_in(refused, &id), &folder, &id, "one of the account's other calendars", out) {
            Ok(state) => calendars.push(CalendarInfo { ctag: state, ..entry }),
            Err(e) if e.is_fatal() => return Err(e),
            Err(e) => {
                log::info!("one of the account's other calendars wasn't read: {e}");
                calendars.push(entry);
            }
        }
    }
    Ok(())
}
