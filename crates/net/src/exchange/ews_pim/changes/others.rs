// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Other people's calendars ticked and the public folders followed (calendars and address books)
//! read by their changes, each by its own route: a colleague's calendar asked for in their mailbox
//! (named in the request, as a whole read names it), a public folder routed to the public folder
//! mailbox holding it once that's known (Exchange 2013 and later answer only there).
//!
//! What's kept of a calendar is given whole, as a whole read gives it: other people's appointments
//! are set aside while the calendar isn't shown, and the engine then takes what a sync gives in
//! place of what it set aside.

use super::*;

/// How a calendar of someone else's was read.
pub(in crate::exchange::ews_pim) struct Followed {
    /// The state to keep with it (empty: none, read whole next time too).
    pub state: String,
    /// Some appointment read says this account may change it.
    pub may_change: bool,
    /// Read by its changes, none read: nothing tells what this account may do with it.
    pub nothing_read: bool,
}

/// Someone else's calendar (`folder`, `owner`'s mailbox), kept here as `id`, into `out` from six
/// months back: by its changes from `kept` (its entry's state) when that's kept and the
/// appointments it stands for are here (set aside or not: a calendar lost from this computer is
/// read whole again), else whole. `again`: appointments to read again (a deletion the server
/// refused).
pub(in crate::exchange::ews_pim) fn shared_calendar(
    c: &Client,
    pim: &Pim,
    folder: &str,
    id: &str,
    owner: &str,
    kept: &str,
    now: i64,
    again: &[String],
    out: &mut DavSync,
) -> Result<Followed> {
    let since = now - CALENDAR_BACK;
    let label = format!("{owner}'s calendar");
    let here = pim.events.iter().any(|e| e.calendar == id && !e.href.is_empty());
    if let Some(read) = read_changes(c, folder, &label, if here { kept } else { "" }, again, Shape::Appointment)? {
        let (began, changed, deleted, state) = (read.began, read.changed, read.deleted, read.state.clone());
        let (may_change, nothing_read) = (read.found.iter().any(item_may_change), read.found.is_empty());
        let kept = calendar_changes(c, pim, read, id, owner, since, true, out);
        log::info!(
            "{label} read by its changes in {:.1} s: {changed} made or changed, {deleted} deleted, {kept} kept as they were",
            began.elapsed().as_secs_f32()
        );
        return Ok(Followed { state, may_change, nothing_read });
    }
    let state = state_now(c, folder, &label)?.unwrap_or_default();
    let list = items_in(c, folder, "", Some(since))?;
    let may_change = list.iter().any(item_may_change);
    read_calendar(c, list, id, owner, since, out);
    out.synced_calendars.push(id.into());
    Ok(Followed { state, may_change, nothing_read: false })
}

/// A public folder as listed: its id, where its content is kept (the GUID of its public folder
/// mailbox, Exchange 2013 and later), how many items it holds, and how it's named.
pub(in crate::exchange::ews_pim) struct Public<'a> {
    pub id: &'a str,
    pub replica: Option<&'a str>,
    pub total: u32,
    pub name: &'a str,
}

/// `f` run routed to the public folder mailbox holding `p`'s content, when that's known this
/// session (found by the whole read before, or a request not found elsewhere); else asked plainly,
/// and a request the server doesn't find so is tried there ([`Client::call`]). Asked plainly
/// first, each batch of items was asked twice on Exchange 2013 and later.
fn routed<T>(c: &Client, p: &Public, f: impl FnOnce() -> Result<T>) -> Result<T> {
    match p.replica.and_then(|g| c.known_public_folder_mailbox(g)) {
        Some(mailbox) => c.routed_to(&mailbox, f),
        None => f(),
    }
}

/// A public calendar, kept here as `key`, into `out` from `since` on: by its changes from its
/// entry's state when that's kept and its appointments are here, else whole. Its appointments are
/// read as a whole read reads them (each item an event, no occurrence of its own). The state to
/// keep.
pub(in crate::exchange::ews_pim) fn public_calendar(c: &Client, pim: &Pim, p: &Public, key: &str, since: i64, out: &mut DavSync) -> Result<String> {
    let folder = format!("<t:FolderId Id=\"{}\"/>", esc(p.id));
    let kept = pim.calendars.iter().find(|k| k.id == key).map(|k| k.ctag.as_str()).unwrap_or("");
    let here = pim.events.iter().any(|e| e.calendar == key && !e.href.is_empty());
    let read = routed(c, p, || read_changes(c, &folder, p.name, if here { kept } else { "" }, &[], Shape::Appointment))?;
    let (found, state) = match read {
        Some(read) => {
            let kept: Vec<StoredEvent> =
                pim.events.iter().filter(|e| e.calendar == key && !e.href.is_empty() && !read.touched.contains(&e.href)).cloned().collect();
            log::info!(
                "{} read by its changes: {} made or changed, {} deleted, {} kept as they were",
                p.name,
                read.changed,
                read.deleted,
                kept.len()
            );
            out.events.extend(kept);
            (read.found.into_iter().filter(|it| in_window(it, since)).collect(), read.state)
        }
        None => {
            let state = routed(c, p, || state_now(c, &folder, p.name))?.unwrap_or_default();
            (routed(c, p, || public_items(c, p.id, p.replica, p.total, Some(since)))?, state)
        }
    };
    for it in found {
        if let Some(event) = event_of(&it) {
            let href = it.child("ItemId").and_then(|i| i.attr("Id")).unwrap_or("").to_string();
            out.events.push(StoredEvent {
                calendar: key.into(),
                href,
                etag: String::new(),
                event,
                dirty: false,
                notify: false,
                respond: None,
                changed_too: false,
            });
        }
    }
    out.synced_calendars.push(key.into());
    Ok(state)
}

/// A public address book, kept here as `key`, into `out`: by its changes from its entry's state
/// when that's kept and its contacts are here (those unchanged named, `kept`: contacts aren't set
/// aside), else whole. The state to keep.
pub(in crate::exchange::ews_pim) fn public_contacts(c: &Client, pim: &Pim, p: &Public, key: &str, out: &mut DavSync) -> Result<String> {
    let folder = format!("<t:FolderId Id=\"{}\"/>", esc(p.id));
    let kept = pim.books.iter().find(|k| k.id == key).map(|k| k.ctag.as_str()).unwrap_or("");
    let here = pim.cards.iter().any(|k| k.book == key && !k.href.is_empty());
    let read = routed(c, p, || read_changes(c, &folder, p.name, if here { kept } else { "" }, &[], Shape::All("")))?;
    let (found, state) = match read {
        Some(read) => {
            let touched: HashSet<String> = read.touched.iter().map(|id| format!("public:{id}")).collect();
            let kept = pim.cards.iter().filter(|k| k.book == key && !k.href.is_empty() && !touched.contains(&k.href));
            out.kept.extend(kept.map(|k| k.href.clone()));
            log::info!("{} read by its changes: {} made or changed, {} deleted", p.name, read.changed, read.deleted);
            (read.found, read.state)
        }
        None => {
            let state = routed(c, p, || state_now(c, &folder, p.name))?.unwrap_or_default();
            (routed(c, p, || public_items(c, p.id, p.replica, p.total, None))?, state)
        }
    };
    for it in found {
        if let Some(card) = card_of(&it) {
            out.cards.push(StoredCard { book: key.into(), href: format!("public:{}", card.uid), etag: String::new(), card, dirty: false });
        }
    }
    out.synced_books.push(key.into());
    Ok(state)
}
