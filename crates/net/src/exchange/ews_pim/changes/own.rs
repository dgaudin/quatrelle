// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The account's own calendar and contacts read by their changes (R21).

use super::*;

/// The account's own calendar into `out`: by its changes when a state is kept, else whole. `again`:
/// appointments to read again whatever the server says (a deletion it refused: they come back).
/// The state to keep with it (empty: none, read whole next time too).
pub(in crate::exchange::ews_pim) fn read_own_calendar(c: &Client, pim: &Pim, now: i64, again: &[String], out: &mut DavSync) -> Result<String> {
    let since = now - CALENDAR_BACK;
    let folder = distinguished("calendar");
    let kept = calendar_state(pim.calendars.iter().find(|k| k.id == CALENDAR).map(|k| k.ctag.as_str()).unwrap_or(""));
    if let Some(read) = read_changes(c, &folder, "the calendar", kept, again, Shape::Appointment)? {
        let (began, changed, deleted, given) = (read.began, read.changed, read.deleted, read.found.len());
        let state = read.state.clone();
        let kept = calendar_changes(c, pim, read, CALENDAR, c.email(), since, false, out);
        log::info!(
            "the calendar read by its changes in {:.1} s: {changed} made or changed ({given} given), {deleted} deleted, {kept} kept as they were",
            began.elapsed().as_secs_f32(),
        );
        return Ok(keep_calendar_state(state));
    }
    // Its state first: what changes while it's read comes next time.
    let state = state_now(c, &folder, "the calendar")?.unwrap_or_default();
    read_calendar(c, items(c, "calendar", Some(since))?, CALENDAR, c.email(), since, out);
    out.synced_calendars.push(CALENDAR.into());
    Ok(keep_calendar_state(state))
}

/// The account's contacts into `out`, under `name`: by their changes when a state is kept, else
/// whole. `again`: contacts to read again (a deletion the server refused).
pub(in crate::exchange::ews_pim) fn read_contacts(c: &Client, pim: &Pim, name: String, again: &[String], out: &mut DavSync) -> Result<()> {
    let folder = distinguished("contacts");
    let book = pim.books.iter().find(|b| b.id == CONTACTS).map(|b| b.ctag.as_str()).unwrap_or("");
    let again: Vec<String> = again.iter().filter_map(|h| h.strip_prefix("contact:")).map(str::to_string).collect();
    let state = match read_changes(c, &folder, "the contacts", book, &again, Shape::All(""))? {
        Some(read) => {
            out.cards.extend(stored_cards(&read.found, CONTACTS, "contact:"));
            let touched: HashSet<String> = read.touched.iter().map(|id| format!("contact:{id}")).collect();
            let kept = pim.cards.iter().filter(|k| k.book == CONTACTS && !k.href.is_empty() && !touched.contains(&k.href));
            out.kept.extend(kept.map(|k| k.href.clone()));
            log::info!("the contacts read by their changes: {} made or changed, {} deleted", read.changed, read.deleted);
            read.state
        }
        None => {
            let state = state_now(c, &folder, "the contacts")?.unwrap_or_default();
            let found = items(c, "contacts", None)?;
            out.cards.extend(stored_cards(&found, CONTACTS, "contact:"));
            state
        }
    };
    out.books = Some(vec![BookInfo { id: CONTACTS.into(), name, read_only: false, ctag: state }]);
    out.synced_books.push(CONTACTS.into());
    Ok(())
}

/// Contacts read into address book `book`, each kept under its item (`prefix` and the item's id)
/// with its change key (the item's `etag`).
pub(super) fn stored_cards<'a>(found: &'a [El], book: &'a str, prefix: &'a str) -> impl Iterator<Item = StoredCard> + 'a {
    found.iter().filter_map(move |it| {
        let card = card_of(it)?;
        let etag = it.child("ItemId").and_then(|i| i.attr("ChangeKey")).unwrap_or("").to_string();
        Some(StoredCard { book: book.into(), href: format!("{prefix}{}", card.uid), etag, card, dirty: false })
    })
}
