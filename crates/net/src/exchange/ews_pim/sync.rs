// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! EWS calendars and contacts: the sync — what changed here sent first, then every calendar,
//! address book, the notes and the tasks read by their changes ([`changes`](super::changes)).

use quatrelle_model::pim::Tombstone;

use super::changes::{Public, notes_read, public_calendar, public_contacts, read_contacts, read_own_calendar, shared_calendar};
use super::*;

/// Send what changed here, then read the calendar and contacts again.
pub fn sync(c: &Client, pim: &Pim, now: i64) -> Result<DavSync> {
    sync_as(c, pim, now, false)
}

/// What the server told of in the account's own calendar or contacts: what changed here sent,
/// then those two read by their changes, the other calendars, the public folders, the notes and
/// the tasks left as they are (`partial`). A few requests, where the whole sync read every
/// appointment again (06/11).
pub fn sync_changes(c: &Client, pim: &Pim, now: i64) -> Result<DavSync> {
    let mut out = DavSync { caldav_home: Some("ews".into()), carddav_home: Some("ews".into()), checked: now, partial: true, ..Default::default() };
    let refused = delete_gone(c, pim.deleted.iter().filter(|t| !t.href.starts_with("task:") && !t.href.starts_with("note:")), &mut out);
    push_events(c, pim, &mut out);
    push_cards(c, pim, &mut out);
    let state = read_own_calendar(c, pim, now, &again_in(&refused, CALENDAR), &mut out)?;
    answer_pending(c, pim, &mut out);
    reminders::answer_all(c, pim, &mut out);
    // Named as it was: the server isn't asked again.
    let own = pim.calendars.iter().find(|k| k.id == CALENDAR).cloned();
    let name =
        own.as_ref().map(|k| k.name.clone()).filter(|n| !n.is_empty()).or_else(|| folder_name(c, "calendar")).unwrap_or_else(|| "Calendar".into());
    out.calendars = Some(vec![CalendarInfo { id: CALENDAR.into(), name, ctag: state, main: true, ..own.unwrap_or_default() }]);
    let book = pim.books.iter().find(|b| b.id == CONTACTS).map(|b| b.name.clone()).filter(|n| !n.is_empty());
    let book = book.or_else(|| folder_name(c, "contacts")).unwrap_or_else(|| "Contacts".into());
    read_contacts(c, pim, book, &hrefs(&refused), &mut out)?;
    quatrelle_model::pim::separate_shared_uids(&mut out.events);
    Ok(out)
}

/// The sync, or (`calendars_only`, the calendar just opened) the calendars alone: the address
/// books, notes and tasks left as they are (public address books of 1,400 contacts took longer
/// than every calendar ticked, 28/10).
fn sync_as(c: &Client, pim: &Pim, now: i64, calendars_only: bool) -> Result<DavSync> {
    let mut out = DavSync { caldav_home: Some("ews".into()), carddav_home: Some("ews".into()), checked: now, ..Default::default() };
    let others = |t: &&quatrelle_model::pim::Tombstone| ["contact:", "note:", "task:"].iter().any(|p| t.href.starts_with(p));
    let refused = delete_gone(c, pim.deleted.iter().filter(|t| !calendars_only || !others(t)), &mut out);
    push_events(c, pim, &mut out);
    if !calendars_only {
        push_cards(c, pim, &mut out);
    }
    // Then the account's own calendar, by its changes.
    let state = read_own_calendar(c, pim, now, &again_in(&refused, CALENDAR), &mut out)?;
    answer_pending(c, pim, &mut out);
    // Reminders dismissed or snoozed here (issue #40), once the server said its version.
    reminders::answer_all(c, pim, &mut out);
    // Named as the server names it, in the mailbox's language ("Agenda"), as Outlook shows it.
    let own = folder_name(c, "calendar").unwrap_or_else(|| "Calendar".into());
    let mut calendars = vec![CalendarInfo {
        id: CALENDAR.into(),
        name: own,
        color: 0,
        read_only: false,
        ctag: state,
        group: String::new(),
        main: true,
        not_own: false,
    }];
    // Other people's calendars: those opened here by their address, and those Outlook keeps in
    // the groups of its Calendar pane (under the group's name). Their mailbox's calendar folder
    // is read where the permissions they gave allow, and changed where they let this account.
    // Contacts first: People shows them, and a first sync's whole address book mustn't wait
    // behind a dozen colleagues' calendars.
    if !calendars_only {
        let name = folder_name(c, "contacts").unwrap_or_else(|| "Contacts".into());
        read_contacts(c, pim, name, &hrefs(&refused), &mut out)?;
    }
    if pim.own_only {
        // The calendar isn't shown: this account's own is enough (the others are read once it is).
        out.calendars = Some(calendars);
        out.partial = true;
        finish(c, pim, &refused, &mut out, calendars_only);
        return Ok(out);
    }
    // The mailbox's own other calendars, each read by its changes as its Calendar is.
    super::own_calendars::read_all(c, pim, now, &refused, &mut calendars, &mut out)?;
    // Opened here by their address, then the calendars of the shared mailboxes the account opens
    // (Outlook lists a shared mailbox's calendar with its mail), then those Outlook keeps in the
    // groups of its Calendar pane, under the group's name. `asked` says which were opened by hand:
    // theirs is the one that says why it's missing.
    let mut others: Vec<(String, String, String, bool)> = pim.shared.iter().map(|e| (e.clone(), e.clone(), String::new(), true)).collect();
    for m in c.shared_mailboxes() {
        if !others.iter().any(|(e, _, _, _)| e.eq_ignore_ascii_case(&m.address)) {
            others.push((m.address, m.name, String::new(), false));
        }
    }
    match calendar_links(c) {
        Ok(links) => {
            for l in links {
                match others.iter_mut().find(|(e, _, _, _)| *e == l.email) {
                    Some(o) => {
                        if !l.name.is_empty() {
                            o.1 = l.name.clone();
                        }
                        if o.2.is_empty() {
                            o.2 = l.group.clone();
                        }
                    }
                    None => others.push((l.email, l.name, l.group, false)),
                }
            }
        }
        Err(e) => log::info!("no calendar groups: {e}"),
    }
    // A profile that knows nothing of other people's calendars yet (a first sync): they're
    // listed, none read. Outlook leaves them unticked, and ticking is what reads one; the engine
    // records the ticks as soon as it sees the list, so the next sync reads the ones ticked.
    let known = pim.calendars.iter().any(|c| quatrelle_model::pim::shared_calendar_owner(&c.id).is_some());
    let ticked = others.iter().filter(|(e, _, _, _)| !known || !pim.leaves_unread(&quatrelle_model::pim::shared_calendar_id(e))).count();
    if !others.is_empty() {
        log::info!("{} other people's calendars, {ticked} ticked{}", others.len(), if known { " and read" } else { " (listed, none read yet)" });
    }
    for (n, (email, name, group, asked)) in others.iter().enumerate() {
        let id = quatrelle_model::pim::shared_calendar_id(email);
        // Its state and rights as kept, to go on from.
        let before = pim.calendars.iter().find(|k| k.id == id);
        let entry = CalendarInfo {
            name: name.clone(),
            color: u8::try_from((n + 1) % 8).unwrap_or(0),
            read_only: before.is_none_or(|k| k.read_only),
            ctag: before.map(|k| k.ctag.clone()).unwrap_or_default(),
            group: group.clone(),
            id,
            main: false,
            not_own: false,
        };
        if !*asked && !known {
            calendars.push(entry);
            continue;
        }
        // Unticked, or removed from the list: listed as it was, not read (the fork keeps
        // Outlook's ticks), whether this account may change it as known from before: asked of
        // the server only the first time (each of fifty colleagues' was, at every sync, 28/10).
        if pim.leaves_unread(&entry.id) {
            let read_only = match pim.calendars.iter().find(|k| k.id == entry.id) {
                Some(k) => k.read_only,
                None => !matches!(may_edit(c, &calendar_folder(Some(&c.address_of(email)))), Ok(true)),
            };
            calendars.push(CalendarInfo { read_only, ..entry });
            continue;
        }
        if let Some(info) = read_other(c, pim, email, *asked, entry, now, &refused, &mut out) {
            calendars.push(info);
        }
    }
    out.calendars = Some(calendars);
    let mut books = out.books.take().unwrap_or_default();
    // The organisation's shared contacts and calendars kept as public folders (on-premises
    // Exchange): read-only, as Outlook shows them under Public Folders. A server without public
    // folders has none.
    match c.public_pim_folders() {
        Ok(list) => {
            // Said in the log, found or not: what a "where are my shared contacts?" needs.
            let contacts = list.iter().filter(|f| f.class.starts_with("IPF.Contact")).count();
            log::info!("public folders: {contacts} address books, {} calendars", list.len() - contacts);
            for (n, f) in list.into_iter().take(MAX_PUBLIC_PIM).enumerate() {
                let (id, name, class) = (f.id, f.path, f.class);
                if calendars_only && class.starts_with("IPF.Contact") {
                    continue;
                }
                let key = format!("public:{id}");
                let color = u8::try_from((n + 3) % 8).unwrap_or(0);
                // A public calendar unticked or removed from the list: listed, not read (its state
                // kept with what's kept of it).
                if !class.starts_with("IPF.Contact") && pim.leaves_unread(&key) {
                    let ctag = pim.calendars.iter().find(|k| k.id == key).map(|k| k.ctag.clone()).unwrap_or_default();
                    if let Some(cals) = out.calendars.as_mut() {
                        cals.push(CalendarInfo { id: key, name, color, read_only: true, ctag, group: String::new(), main: false, not_own: false });
                    }
                    continue;
                }
                // A public calendar from six months back, as the others; contacts whole. Each by
                // its changes once read.
                let p = Public { id: &id, replica: f.replica.as_deref(), total: f.total, name: &name };
                if class.starts_with("IPF.Contact") {
                    match public_contacts(c, pim, &p, &key, &mut out) {
                        Ok(ctag) => books.push(BookInfo { id: key, name, read_only: true, ctag }),
                        Err(e) => out.errors.push(format!("{name}: {e}")),
                    }
                    continue;
                }
                match public_calendar(c, pim, &p, &key, now - CALENDAR_BACK, &mut out) {
                    Ok(ctag) => {
                        if let Some(cals) = out.calendars.as_mut() {
                            cals.push(CalendarInfo {
                                id: key,
                                name,
                                color,
                                read_only: true,
                                ctag,
                                group: String::new(),
                                main: false,
                                not_own: false,
                            });
                        }
                    }
                    Err(e) => out.errors.push(format!("{name}: {e}")),
                }
            }
        }
        // A server with public folders that won't list them says why; one without has none.
        Err(e) if c.public_hierarchy_mailbox().is_some() => out.errors.push(format!("Public folders: {e}")),
        Err(e) => log::info!("no public contacts or calendars: {e}"),
    }
    if !calendars_only {
        out.books = Some(books);
    }
    finish(c, pim, &refused, &mut out, calendars_only);
    Ok(out)
}

/// The notes and tasks read with the rest (`refused`: deletions refused, to read again); the
/// calendars alone, their appointments told apart.
fn finish(c: &Client, pim: &Pim, refused: &[Tombstone], out: &mut DavSync, calendars_only: bool) {
    if calendars_only {
        quatrelle_model::pim::separate_shared_uids(&mut out.events);
    } else {
        read_notes(c, pim, &hrefs(refused), out);
        read_tasks(c, pim, &hrefs(refused), out);
    }
}

/// The deletions the server refused in calendar `id` (the account's own: those of no other
/// calendar), to read again: they come back.
pub(super) fn again_in(refused: &[Tombstone], id: &str) -> Vec<String> {
    refused
        .iter()
        .filter(|t| !["contact:", "note:", "task:"].iter().any(|p| t.href.starts_with(p)))
        .filter(|t| if id == CALENDAR { t.calendar.is_empty() || t.calendar == CALENDAR } else { t.calendar == id })
        .map(|t| t.href.clone())
        .collect()
}

/// What the deletions the server refused named (contacts, notes and tasks are told by their
/// prefix).
fn hrefs(refused: &[Tombstone]) -> Vec<String> {
    refused.iter().map(|t| t.href.clone()).collect()
}

/// Other people's calendars alone (just ticked), not everything again: their appointments and
/// their entries in the list, the rest left as it is (`partial`). A few seconds, where a full
/// sync of a large account took half a minute. None named: every calendar ticked (the calendar just
/// opened), without the address books.
pub fn sync_calendars(c: &Client, pim: &Pim, ids: &[String], now: i64) -> Result<DavSync> {
    if ids.is_empty() {
        return sync_as(c, pim, now, true);
    }
    let mut out = DavSync { caldav_home: Some("ews".into()), carddav_home: Some("ews".into()), checked: now, partial: true, ..Default::default() };
    let mut calendars = Vec::new();
    for id in ids {
        let Some(email) = quatrelle_model::pim::shared_calendar_owner(id) else { continue };
        // As the list has it (its name, colour and group).
        let entry = pim.calendars.iter().find(|k| k.id == *id).cloned().unwrap_or_else(|| CalendarInfo {
            id: id.clone(),
            name: email.to_string(),
            read_only: true,
            ..Default::default()
        });
        let asked = pim.shared.iter().any(|e| e.eq_ignore_ascii_case(email));
        if let Some(info) = read_other(c, pim, email, asked, entry, now, &[], &mut out) {
            calendars.push(info);
        }
    }
    out.calendars = Some(calendars);
    quatrelle_model::pim::separate_shared_uids(&mut out.events);
    Ok(out)
}

/// `email`'s calendar read into `out` (from six months back), by its changes from the state its
/// entry keeps once read, and its entry in the list as `entry` has it, editable or not as the
/// server says. `refused`: deletions the server refused, those of this calendar read again.
/// `None`: a shared mailbox whose calendar isn't shared with this account (said in the log, then
/// not asked again this session); one opened by hand (`asked`) says why it's missing, and stays
/// listed with what's kept of it.
#[allow(clippy::too_many_arguments)]
fn read_other(
    c: &Client,
    pim: &Pim,
    email: &str,
    asked: bool,
    entry: CalendarInfo,
    now: i64,
    refused: &[Tombstone],
    out: &mut DavSync,
) -> Option<CalendarInfo> {
    if !asked && c.has_no_calendar(email) {
        return None;
    }
    let parent = calendar_folder(Some(&c.address_of(email)));
    // Editable when the folder's rights say so, or (some servers say nothing useful about
    // another mailbox's folder) when its events do, as Outlook finds by trying.
    let folder_rights = may_edit(c, &parent);
    let mut can_edit = matches!(&folder_rights, Ok(true));
    match shared_calendar(c, pim, &parent, &entry.id, email, &entry.ctag, now, &again_in(refused, &entry.id), out) {
        Ok(read) => {
            // Read by its changes, none of them read: as it was.
            can_edit |= read.may_change || (read.nothing_read && !entry.read_only);
            if !can_edit && let Err(e) = &folder_rights {
                log::info!("{email}'s calendar rights: {e}");
            }
            return Some(CalendarInfo { read_only: !can_edit, ctag: read.state, ..entry });
        }
        Err(e) if asked => out.errors.push(format!("{email}'s calendar: {e}")),
        Err(e) => {
            log::info!("shared mailbox {email}: no calendar ({e})");
            // Refused (not a server busy or out of reach): no calendar there this session.
            if matches!(e, NetError::No(_)) {
                c.note_no_calendar(email);
            }
            return None;
        }
    }
    Some(CalendarInfo { read_only: !can_edit, ..entry })
}

/// The tasks alone, as the server tells of a change in their folder: those deleted and changed
/// here sent, then the folder read again. The calendars and address books aren't in it (`None`):
/// they stay as they are.
pub fn sync_tasks(c: &Client, pim: &Pim, now: i64) -> DavSync {
    let mut out = DavSync { caldav_home: Some("ews".into()), carddav_home: Some("ews".into()), checked: now, ..Default::default() };
    let refused = delete_gone(c, pim.deleted.iter().filter(|t| t.href.starts_with("task:")), &mut out);
    read_tasks(c, pim, &hrefs(&refused), &mut out);
    out
}

/// What was deleted here, deleted on the server; those it refused, to read again (their item
/// there), as they stay on the server and so come back here.
fn delete_gone<'a>(c: &Client, gone: impl Iterator<Item = &'a Tombstone>, out: &mut DavSync) -> Vec<Tombstone> {
    let mut refused = Vec::new();
    for t in gone {
        let task = t.href.starts_with("task:");
        match delete(c, t.href.trim_start_matches("contact:").trim_start_matches("note:").trim_start_matches("task:"), task) {
            // Deleted, or gone already (deleted in Outlook too).
            Ok(()) => out.deleted.push(t.href.clone()),
            Err(NetError::No(why)) if why.starts_with("not on the server") => out.deleted.push(t.href.clone()),
            // Refused: it stays on the server, so it comes back here, and why is said.
            Err(NetError::No(why)) => {
                out.deleted.push(t.href.clone());
                out.errors.push(format!("not deleted on the server: {why}"));
                refused.push(t.clone());
            }
            // Not now (offline, the server busy): tried again next time, still deleted here.
            Err(e) => out.errors.push(e.to_string()),
        }
    }
    refused
}

/// The contacts changed here, made again on the server (a contact's addresses and numbers are
/// simplest replaced whole).
fn push_cards(c: &Client, pim: &Pim, out: &mut DavSync) {
    for k in pim.cards.iter().filter(|k| k.dirty) {
        if let Some(old) = k.href.strip_prefix("contact:")
            && let Err(e) = delete(c, old, false)
            && !matches!(e, NetError::No(_))
        {
            out.errors.push(format!("{}: {e}", k.card.display()));
            continue;
        }
        let r = c.call_one(&format!(
            "<m:CreateItem><m:SavedItemFolderId><t:DistinguishedFolderId Id=\"contacts\"/></m:SavedItemFolderId><m:Items>{}</m:Items></m:CreateItem>",
            contact_xml(&k.card)
        ));
        match r {
            Ok(m) => {
                let (id, etag) = new_id(&m);
                out.pushed_cards.push(StoredCard { book: CONTACTS.into(), href: format!("contact:{id}"), etag, card: k.card.clone(), dirty: false });
            }
            Err(x) => out.errors.push(format!("{}: {x}", k.card.display())),
        }
    }
}

/// A calendar's items, read, into `out`: each appointment and series as an event, and the
/// occurrences of a series changed on their own (moved, renamed) as instances of their own —
/// the series lists their items, which are read in a batch — so that a meeting moved in
/// Outlook, or dragged here, shows on its new day rather than its old one. One the server
/// doesn't give is made from what its series says of it (its own times).
/// `owner`: the calendar's mailbox (the organizer of the meetings it organizes); `since`: what
/// the calendar is read from (the occurrences changed before then aren't).
/// Each appointment comes with its files ([`super::files`]): kept for the account's own calendar,
/// listed for the others.
pub(super) fn read_calendar(c: &Client, list: Vec<El>, calendar: &str, owner: &str, since: i64, out: &mut DavSync) {
    let mut files = Vec::new();
    read_appointments(c, list, calendar, owner, since, out, &mut files);
    super::files::fill(c, out, files, own_mailbox_calendar(calendar));
}

/// [`read_calendar`] without the files: those of each event read noted in `files`, by its place
/// in `out.events`.
fn read_appointments(
    c: &Client,
    list: Vec<El>,
    calendar: &str,
    owner: &str,
    since: i64,
    out: &mut DavSync,
    files: &mut Vec<(usize, Vec<super::files::Listed>)>,
) {
    let mut note = |out: &DavSync, it: &El| {
        let listed = super::files::listed(it);
        if !listed.is_empty() {
            files.push((out.events.len(), listed));
        }
    };
    let id_of = |it: &El| {
        it.child("ItemId").map(|i| (i.attr("Id").unwrap_or("").to_string(), i.attr("ChangeKey").unwrap_or("").to_string())).unwrap_or_default()
    };
    let stored = |href: String, etag: String, event: CalEvent| StoredEvent {
        calendar: calendar.into(),
        href,
        etag,
        event,
        dirty: false,
        notify: false,
        respond: None,
        changed_too: false,
    };
    let mut changed: Vec<String> = Vec::new();
    // Each of those as its series says of it, for one the server doesn't give.
    let mut listed: Vec<(String, String, CalEvent)> = Vec::new();
    for it in &list {
        if let Some(mut event) = event_of(it) {
            own_organizer(&mut event, it, owner);
            for o in modified_occurrences(it, Some(since)) {
                let (href, etag) = id_of(o);
                if let Some(e) = listed_exception(&event, o) {
                    listed.push((href.clone(), etag, e));
                }
                changed.push(href);
            }
            let (href, etag) = id_of(it);
            note(out, it);
            out.events.push(stored(href, etag, event));
        }
    }
    changed.truncate(MAX_ITEMS);
    if changed.is_empty() {
        return;
    }
    // Said in the log with what they cost, as the calendar's own items are.
    let all: usize = list.iter().filter_map(|it| it.child("ModifiedOccurrences")).map(|m| m.children("Occurrence").count()).sum();
    let (began, before) = (std::time::Instant::now(), c.spent());
    let found = get_items(c, &changed, Shape::Appointment).unwrap_or_else(|e| {
        out.errors.push(format!("changed occurrences: {e}"));
        Vec::new()
    });
    // The occurrences given, by series and place in it.
    let mut given: std::collections::HashSet<(String, i64)> = std::collections::HashSet::new();
    for it in &found {
        if let Some(mut event) = exception_of(it) {
            own_organizer(&mut event, it, owner);
            given.extend(event.recurrence_id.map(|r| (event.uid.clone(), r)));
            let (href, etag) = id_of(it);
            note(out, it);
            out.events.push(stored(href, etag, event));
        }
    }
    let mut from_series = 0;
    for (href, etag, event) in listed {
        if event.recurrence_id.is_some_and(|r| !given.contains(&(event.uid.clone(), r))) {
            from_series += 1;
            out.events.push(stored(href, etag, event));
        }
    }
    let t = c.spent().since(before);
    log::info!(
        "{} occurrences changed on their own read in {:.1} s ({} older left out{}): {:.1} s waiting for the server",
        changed.len(),
        began.elapsed().as_secs_f32(),
        all.saturating_sub(changed.len()),
        if from_series > 0 { format!("; {from_series} not given, shown from their series") } else { String::new() },
        t.server.as_secs_f32()
    );
}

/// Notes: what changed here goes up, then the folder is read by its changes (`again`: deletions
/// refused, to read again).
fn read_notes(c: &Client, pim: &Pim, again: &[String], out: &mut DavSync) {
    // Notes (Outlook's sticky notes): what changed here goes up (made again, as contacts are),
    // then the folder is read again. A mailbox without one keeps notes on this computer.
    for n in pim.notes.iter().filter(|n| n.dirty) {
        if let Some(old) = n.href.strip_prefix("note:")
            && let Err(e) = delete(c, old, false)
            && !matches!(e, NetError::No(_))
        {
            out.errors.push(format!("{}: {e}", n.note.title()));
            continue;
        }
        // A note is a message to Exchange, which wants it said that it is only saved (it refused
        // with "MessageDisposition attribute is required").
        let create = |sized: bool| {
            c.call_one(&format!(
                "<m:CreateItem MessageDisposition=\"SaveOnly\"><m:SavedItemFolderId><t:DistinguishedFolderId Id=\"notes\"/></m:SavedItemFolderId><m:Items>{}</m:Items></m:CreateItem>",
                note_xml(&n.note, sized)
            ))
        };
        let mut sent = n.note.clone();
        let r = match create(true) {
            // A server that won't take the window's size: the note goes without it, and its size
            // goes back to the server's (none), said with the rest.
            Err(NetError::No(e)) if n.note.size().is_some() => {
                log::info!("ews: a note's size refused ({e}): made without it");
                let r = create(false);
                if r.is_ok() {
                    (sent.width, sent.height) = (0, 0);
                    out.notes_size_refused.push(n.note.uid.clone());
                    out.errors.push(format!("{}: the server didn't keep its size ({e})", n.note.title()));
                }
                r
            }
            r => r,
        };
        match r {
            Ok(m) => {
                let (id, etag) = new_id(&m);
                out.pushed_notes.push(StoredNote { href: format!("note:{id}"), etag, note: sent, dirty: false });
            }
            Err(x) => out.errors.push(format!("{}: {x}", n.note.title())),
        }
    }
    match notes_read(c, pim, again) {
        Ok((list, state)) => {
            out.notes = Some(list);
            out.notes_state = state;
        }
        Err(e) => log::info!("no Notes folder: {e}"),
    }
    // A scheduling tool can send several plain appointments under one series' UID: the engine
    // keeps events by UID, so they would collapse into one.
    quatrelle_model::pim::separate_shared_uids(&mut out.events);
}
