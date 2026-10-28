//! EWS calendars and contacts: the sync — what changed here sent first, then every folder read again.

use super::*;

/// Send what changed here, then read the calendar and contacts again.
pub fn sync(c: &Client, pim: &Pim, now: i64) -> Result<DavSync> {
    let mut out = DavSync { caldav_home: Some("ews".into()), carddav_home: Some("ews".into()), checked: now, ..Default::default() };
    delete_gone(c, pim.deleted.iter(), &mut out);
    push_events(c, pim, &mut out);
    for k in pim.cards.iter().filter(|k| k.dirty) {
        // Made again: a contact's addresses and numbers are simplest replaced whole.
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
    // Then both folders, read again.
    read_calendar(c, items(c, "calendar", Some(now - CALENDAR_BACK))?, CALENDAR, c.email(), &mut out);
    answer_pending(c, pim, &mut out);
    // Named as the server names it, in the mailbox's language ("Agenda"), as Outlook shows it.
    let own = folder_name(c, "calendar").unwrap_or_else(|| "Calendar".into());
    let mut calendars = vec![CalendarInfo { id: CALENDAR.into(), name: own, color: 0, read_only: false, ctag: String::new(), group: String::new() }];
    out.synced_calendars.push(CALENDAR.into());
    // Other people's calendars: those opened here by their address, and those Outlook keeps in
    // the groups of its Calendar pane (under the group's name). Their mailbox's calendar folder
    // is read where the permissions they gave allow, and changed where they let this account.
    // Contacts first: People shows them, and a first sync's whole address book mustn't wait
    // behind a dozen colleagues' calendars.
    read_contacts(c, &mut out)?;
    if pim.own_only {
        // The calendar isn't shown: this account's own is enough (the others are read once it is).
        out.calendars = Some(calendars);
        out.partial = true;
        read_notes(c, pim, &mut out);
        read_tasks(c, pim, &mut out);
        return Ok(out);
    }
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
    let ticked = others.iter().filter(|(e, _, _, _)| !known || !pim.not_read.contains(&quatrelle_model::pim::shared_calendar_id(e))).count();
    if !others.is_empty() {
        log::info!("{} other people's calendars, {ticked} ticked{}", others.len(), if known { " and read" } else { " (listed, none read yet)" });
    }
    for (n, (email, name, group, asked)) in others.iter().enumerate() {
        let id = quatrelle_model::pim::shared_calendar_id(email);
        if !*asked && !known {
            calendars.push(CalendarInfo {
                id,
                name: name.clone(),
                color: u8::try_from((n + 1) % 8).unwrap_or(0),
                read_only: true,
                ctag: String::new(),
                group: group.clone(),
            });
            continue;
        }
        let parent = calendar_folder(Some(&c.address_of(email)));
        // Editable when the folder's rights say so, or (some servers say nothing useful about
        // another mailbox's folder) when its events do, as Outlook finds by trying.
        let folder_rights = may_edit(c, &parent);
        let mut can_edit = matches!(&folder_rights, Ok(true));
        // Unticked: listed as it was, not read (the fork keeps Outlook's ticks).
        if pim.not_read.contains(&id) {
            calendars.push(CalendarInfo {
                id,
                name: name.clone(),
                color: u8::try_from((n + 1) % 8).unwrap_or(0),
                read_only: !can_edit,
                ctag: String::new(),
                group: group.clone(),
            });
            continue;
        }
        match items_in(c, &parent, "", Some(now - CALENDAR_BACK)) {
            Ok(list) => {
                can_edit |= list.iter().any(item_may_change);
                if !can_edit && let Err(e) = &folder_rights {
                    log::info!("{email}'s calendar rights: {e}");
                }
                read_calendar(c, list, &id, email, &mut out);
                out.synced_calendars.push(id.clone());
            }
            // One asked for by hand says why it's missing; a shared mailbox whose calendar
            // isn't shared with the account just has none here.
            Err(e) if *asked => out.errors.push(format!("{email}'s calendar: {e}")),
            Err(e) => {
                log::info!("shared mailbox {email}: no calendar ({e})");
                continue;
            }
        }
        calendars.push(CalendarInfo {
            id,
            name: name.clone(),
            color: u8::try_from((n + 1) % 8).unwrap_or(0),
            read_only: !can_edit,
            ctag: String::new(),
            group: group.clone(),
        });
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
                let key = format!("public:{id}");
                // An unticked public calendar: listed, not read.
                if !class.starts_with("IPF.Contact") && pim.not_read.contains(&key) {
                    if let Some(cals) = out.calendars.as_mut() {
                        cals.push(CalendarInfo {
                            id: key,
                            name,
                            color: u8::try_from((n + 3) % 8).unwrap_or(0),
                            read_only: true,
                            ctag: String::new(),
                            group: String::new(),
                        });
                    }
                    continue;
                }
                // A public calendar from six months back, as the others; contacts whole.
                let since = class.starts_with("IPF.Appointment").then_some(now - CALENDAR_BACK);
                let found = match public_items(c, &id, f.replica.as_deref(), f.total, since) {
                    Ok(found) => found,
                    Err(e) => {
                        out.errors.push(format!("{name}: {e}"));
                        continue;
                    }
                };
                if class.starts_with("IPF.Contact") {
                    for it in found {
                        if let Some(card) = card_of(&it) {
                            out.cards.push(StoredCard {
                                book: key.clone(),
                                href: format!("public:{}", card.uid),
                                etag: String::new(),
                                card,
                                dirty: false,
                            });
                        }
                    }
                    books.push(BookInfo { id: key.clone(), name, read_only: true, ctag: String::new() });
                    out.synced_books.push(key);
                } else {
                    for it in found {
                        if let Some(event) = event_of(&it) {
                            let href = it.child("ItemId").and_then(|i| i.attr("Id")).unwrap_or("").to_string();
                            out.events.push(StoredEvent {
                                calendar: key.clone(),
                                href,
                                etag: String::new(),
                                event,
                                dirty: false,
                                notify: false,
                                respond: None,
                            });
                        }
                    }
                    if let Some(cals) = out.calendars.as_mut() {
                        cals.push(CalendarInfo {
                            id: key.clone(),
                            name,
                            color: u8::try_from((n + 3) % 8).unwrap_or(0),
                            read_only: true,
                            ctag: String::new(),
                            group: String::new(),
                        });
                    }
                    out.synced_calendars.push(key);
                }
            }
        }
        // A server with public folders that won't list them says why; one without has none.
        Err(e) if c.public_hierarchy_mailbox().is_some() => out.errors.push(format!("Public folders: {e}")),
        Err(e) => log::info!("no public contacts or calendars: {e}"),
    }
    out.books = Some(books);
    read_notes(c, pim, &mut out);
    read_tasks(c, pim, &mut out);
    Ok(out)
}

/// The tasks alone, as the server tells of a change in their folder: those deleted and changed
/// here sent, then the folder read again. The calendars and address books aren't in it (`None`):
/// they stay as they are.
pub fn sync_tasks(c: &Client, pim: &Pim, now: i64) -> DavSync {
    let mut out = DavSync { caldav_home: Some("ews".into()), carddav_home: Some("ews".into()), checked: now, ..Default::default() };
    delete_gone(c, pim.deleted.iter().filter(|t| t.href.starts_with("task:")), &mut out);
    read_tasks(c, pim, &mut out);
    out
}

/// What was deleted here, deleted on the server.
fn delete_gone<'a>(c: &Client, gone: impl Iterator<Item = &'a quatrelle_model::pim::Tombstone>, out: &mut DavSync) {
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
            }
            // Not now (offline, the server busy): tried again next time, still deleted here.
            Err(e) => out.errors.push(e.to_string()),
        }
    }
}

/// A calendar's items, read, into `out`: each appointment and series as an event, and the
/// occurrences of a series changed on their own (moved, renamed) as instances of their own —
/// the series lists their items, which are read in a batch — so that a meeting moved in
/// Outlook, or dragged here, shows on its new day rather than its old one.
/// `owner`: the calendar's mailbox (the organizer of the meetings it organizes).
fn read_calendar(c: &Client, list: Vec<El>, calendar: &str, owner: &str, out: &mut DavSync) {
    let id_of = |it: &El| {
        it.child("ItemId").map(|i| (i.attr("Id").unwrap_or("").to_string(), i.attr("ChangeKey").unwrap_or("").to_string())).unwrap_or_default()
    };
    let mut changed: Vec<String> = Vec::new();
    for it in &list {
        if let Some(mut event) = event_of(it) {
            own_organizer(&mut event, it, owner);
            let (href, etag) = id_of(it);
            out.events.push(StoredEvent { calendar: calendar.into(), href, etag, event, dirty: false, notify: false, respond: None });
            changed.extend(modified_occurrence_ids(it));
        }
    }
    changed.truncate(MAX_ITEMS);
    match get_items(c, &changed, "") {
        Ok(found) => {
            for it in &found {
                if let Some(mut event) = exception_of(it) {
                    own_organizer(&mut event, it, owner);
                    let (href, etag) = id_of(it);
                    out.events.push(StoredEvent { calendar: calendar.into(), href, etag, event, dirty: false, notify: false, respond: None });
                }
            }
        }
        Err(e) => out.errors.push(format!("changed occurrences: {e}")),
    }
}

/// This account's contacts, read again: its address book in `out`.
fn read_contacts(c: &Client, out: &mut DavSync) -> Result<()> {
    for it in items(c, "contacts", None)? {
        if let Some(card) = card_of(&it) {
            let etag = it.child("ItemId").and_then(|i| i.attr("ChangeKey")).unwrap_or("").to_string();
            out.cards.push(StoredCard { book: CONTACTS.into(), href: format!("contact:{}", card.uid), etag, card, dirty: false });
        }
    }
    let own = folder_name(c, "contacts").unwrap_or_else(|| "Contacts".into());
    out.books = Some(vec![BookInfo { id: CONTACTS.into(), name: own, read_only: false, ctag: String::new() }]);
    out.synced_books.push(CONTACTS.into());
    Ok(())
}

/// Notes: what changed here goes up, then the folder is read again.
fn read_notes(c: &Client, pim: &Pim, out: &mut DavSync) {
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
        let r = c.call_one(&format!(
            "<m:CreateItem><m:SavedItemFolderId><t:DistinguishedFolderId Id=\"notes\"/></m:SavedItemFolderId><m:Items>{}</m:Items></m:CreateItem>",
            note_xml(&n.note)
        ));
        match r {
            Ok(m) => {
                let (id, etag) = new_id(&m);
                out.pushed_notes.push(StoredNote { href: format!("note:{id}"), etag, note: n.note.clone(), dirty: false });
            }
            Err(x) => out.errors.push(format!("{}: {x}", n.note.title())),
        }
    }
    match items(c, "notes", None) {
        Ok(list) => out.notes = Some(list.iter().filter_map(note_of).collect()),
        Err(e) => log::info!("no Notes folder: {e}"),
    }
    // A scheduling tool can send several plain appointments under one series' UID: the engine
    // keeps events by UID, so they would collapse into one.
    quatrelle_model::pim::separate_shared_uids(&mut out.events);
}
