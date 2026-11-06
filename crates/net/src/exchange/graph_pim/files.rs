// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! An event's files on Microsoft 365 (R25), as the EWS calendar has them ([`crate::ews_pim`]'s
//! `files`): an event that says it has some (`hasAttachments`) lists them
//! (`…/events/{id}/attachments`); those of the account's own calendars are kept with it within the
//! same bounds, their bytes read one by one (`$value`); the others, and other people's, are listed
//! and fetched when opened ([`attachment`]).
//!
//! Graph's calendars are read whole at each sync: an event the server has as it was (its change
//! key) keeps the files it has here, without asking again; one changed is listed again, so a file
//! taken off on the server goes from here.

use quatrelle_model::pim::files::{event_file_lines, event_has_files, server_file_line};

use super::*;
use crate::ews_pim::{MAX_EVENT_FILES, MAX_FILE};

/// The files of the events just read from a calendar (`at`: where its events are; `list`: them as
/// Graph gave them; `events`: those made of them), written into them: kept (`keep`, the account's
/// own calendars) within [`MAX_FILE`] and [`MAX_EVENT_FILES`], listed otherwise.
pub(super) fn fill(c: &Client, pim: &Pim, at: &str, list: &[Value], events: &mut [StoredEvent], keep: bool) {
    let with: std::collections::HashSet<&str> =
        list.iter().filter(|v| v.get("hasAttachments").and_then(Value::as_bool) == Some(true)).filter_map(|v| v.get("id")?.as_str()).collect();
    let (mut kept, mut asked) = (0usize, 0usize);
    for e in events.iter_mut().filter(|e| with.contains(e.href.as_str())) {
        // As it was: what's kept of its files stays.
        let before = pim.events.iter().find(|k| k.calendar == e.calendar && k.href == e.href && k.event.recurrence_id.is_none());
        let lines = match before.filter(|k| !k.etag.is_empty() && k.etag == e.etag && event_has_files(&k.event)) {
            Some(k) => {
                kept += 1;
                event_file_lines(&k.event)
            }
            None => {
                asked += 1;
                match read(c, &format!("{at}/{}/attachments", seg(&e.href)), keep) {
                    Ok(lines) => lines,
                    Err(err) => {
                        log::info!("{}: its files weren't listed ({err})", e.event.summary);
                        continue;
                    }
                }
            }
        };
        e.event.extra.extend(lines);
    }
    if kept + asked > 0 {
        log::info!("events' files: {kept} events' kept as they were, {asked} listed again");
    }
}

/// An event's files (`path`: its attachments) as its lines.
fn read(c: &Client, path: &str, keep: bool) -> Result<Vec<String>> {
    let list = c.all(&format!("{path}?$select=id,name,contentType,size,isInline"), quatrelle_model::pim::files::MAX_FILES)?;
    let mut room = MAX_EVENT_FILES;
    let mut lines = Vec::new();
    for a in &list {
        let kind = s(a, "@odata.type").to_ascii_lowercase();
        let item = kind.ends_with("itemattachment");
        // A link to a file elsewhere, a picture inside the text: no file of the event's.
        if kind.ends_with("referenceattachment") || a.get("isInline").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let id = s(a, "id");
        if id.is_empty() {
            continue;
        }
        let name = s(a, "name").trim().to_string();
        let (name, mime) = if item {
            (if name.is_empty() { "message.eml".to_string() } else { format!("{name}.eml") }, "message/rfc822".to_string())
        } else {
            (name, s(a, "contentType"))
        };
        let size = a.get("size").and_then(Value::as_u64).unwrap_or(0);
        let reference = format!("{path}/{}", seg(&id));
        let small = usize::try_from(size).is_ok_and(|n| n <= MAX_FILE && n <= room);
        let data = if keep && small {
            c.attachment_value(&reference).map_err(|e| log::info!("{name}: not given ({e}): left on the server")).ok()
        } else {
            None
        };
        match data.filter(|d| d.len() <= MAX_FILE && d.len() <= room) {
            Some(d) => {
                room -= d.len();
                lines.push(crate::pst_pim::event_file(&name, &mime, &d));
            }
            None => lines.extend(server_file_line(&name, &mime, size, &reference)),
        }
    }
    Ok(lines)
}

/// One of an event's files, fetched when it's opened (`reference`: its attachment's path, as
/// [`fill`] wrote it): only an event's attachment of this account's mailbox or another's.
pub fn attachment(c: &Client, reference: &str) -> Result<Vec<u8>> {
    let r = reference.trim();
    let shaped = (r.starts_with("/me/") || r.starts_with("/users/"))
        && r.contains("/events/")
        && r.contains("/attachments/")
        && !r.contains("..")
        && r.chars().all(|c| c.is_ascii_alphanumeric() || "/%-_.=+~:".contains(c));
    if !shaped {
        return Err(NetError::No("not an appointment's file on this server".into()));
    }
    c.attachment_value(r)
}
