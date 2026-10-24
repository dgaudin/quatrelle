//! EWS calendars: the appointments changed here, sent to the server — new ones created, changed
//! ones updated field by field, an occurrence changed on its own through the occurrence's own
//! item — and the guests told when the change asks for it; invitations answered as Outlook
//! answers them, by the server.

use super::*;

/// Send every appointment changed here, noting each one's item on the server in `out`.
pub(super) fn push_events(c: &Client, pim: &Pim, out: &mut DavSync) {
    for e in pim.events.iter().filter(|e| e.dirty) {
        // An answer to an invitation: given by the server on the meeting it placed in the
        // calendar; one not in the calendar here yet waits for it to be read
        // ([`answer_pending`]). Never created as an appointment of the user's own: that made a
        // second copy of the meeting, with the user as its organizer.
        if let Some(answer) = &e.respond {
            if !e.href.is_empty() && e.event.recurrence_id.is_none() {
                match respond_item(c, &e.href, answer) {
                    Ok(()) => out.responded.push((e.event.uid.clone(), answer.clone())),
                    Err(x) => out.errors.push(format!("{}: {x}", e.event.summary)),
                }
            }
            continue;
        }
        let owner = quatrelle_model::pim::shared_calendar_owner(&e.calendar);
        let r = match e.event.recurrence_id.filter(|_| e.href.is_empty()) {
            // One occurrence of a series, changed on its own: the occurrence's item is asked for
            // by its place in the series, then changed as any appointment is.
            Some(at) => match series_of(pim, out, &e.event.uid) {
                Some((master_id, master)) => {
                    occurrence_item_id(c, &master_id, &master, at).and_then(|(id, key)| update_item(c, &id, &key, &e.event, e.notify))
                }
                None => Err(NetError::No("the series isn't on the server yet".into())),
            },
            None if e.href.is_empty() => create_item(c, owner, &e.event, e.notify),
            None => update_item(c, &e.href, &e.etag, &e.event, e.notify),
        };
        match r {
            // Someone else's event stays in their calendar.
            Ok((href, etag)) => out.pushed_events.push(StoredEvent {
                calendar: if owner.is_some() { e.calendar.clone() } else { CALENDAR.into() },
                href,
                etag,
                event: e.event.clone(),
                dirty: false,
                notify: false,
                respond: None,
            }),
            Err(x) => out.errors.push(format!("{}: {x}", e.event.summary)),
        }
    }
}

/// Answers to invitations whose meeting wasn't in the calendar here yet: found in the calendar
/// just read (by the invitation's UID, or Exchange's `UID#start` for a single one), and
/// answered there. Not found yet: tried again at the next sync.
pub(super) fn answer_pending(c: &Client, pim: &Pim, out: &mut DavSync) {
    for e in pim.events.iter().filter(|e| e.dirty && e.href.is_empty() && e.event.recurrence_id.is_none()) {
        let Some(answer) = &e.respond else { continue };
        let found = out
            .events
            .iter()
            .find(|x| x.calendar == CALENDAR && x.event.recurrence_id.is_none() && quatrelle_model::pim::same_meeting(&x.event.uid, &e.event.uid))
            .map(|x| x.href.clone());
        if let Some(href) = found {
            match respond_item(c, &href, answer) {
                Ok(()) => out.responded.push((e.event.uid.clone(), answer.clone())),
                Err(x) => out.errors.push(format!("{}: {x}", e.event.summary)),
            }
        }
    }
}

/// Answer an invitation (`ACCEPTED`, `TENTATIVE`, `DECLINED`) on the meeting's item: the server
/// updates the meeting in the calendar (a declined one leaves it) and sends the organizer the
/// answer, as Outlook's buttons do.
fn respond_item(c: &Client, id: &str, answer: &str) -> Result<()> {
    let kind = match answer {
        "ACCEPTED" => "AcceptItem",
        "TENTATIVE" => "TentativelyAcceptItem",
        "DECLINED" => "DeclineItem",
        other => return Err(NetError::No(format!("no answer `{other}`"))),
    };
    c.call_one(&format!(
        "<m:CreateItem MessageDisposition=\"SendAndSaveCopy\"><m:Items><t:{kind}><t:ReferenceItemId Id=\"{}\"/></t:{kind}></m:Items></m:CreateItem>",
        esc(id)
    ))
    .map(drop)
}

/// A series' item on the server and the series itself: as kept here, or as just created.
fn series_of(pim: &Pim, out: &DavSync, uid: &str) -> Option<(String, CalEvent)> {
    pim.events
        .iter()
        .chain(out.pushed_events.iter())
        .filter(|m| m.event.uid == uid && m.event.recurrence_id.is_none() && !m.href.is_empty())
        .map(|m| (m.href.clone(), m.event.clone()))
        .next_back()
}

/// A new appointment, in this mailbox's calendar or someone else's; the guests invited by the
/// server when asked (`notify`, as Outlook's Send), else told nothing.
fn create_item(c: &Client, owner: Option<&str>, e: &CalEvent, notify: bool) -> Result<(String, String)> {
    let body: String = event_fields(e, c.email()).iter().map(|(_, x)| x.as_str()).collect();
    let send = if notify { "SendToAllAndSaveCopy" } else { "SendToNone" };
    c.call_one(&format!(
        "<m:CreateItem SendMeetingInvitations=\"{send}\"><m:SavedItemFolderId>{}</m:SavedItemFolderId>\
         <m:Items><t:CalendarItem>{body}</t:CalendarItem></m:Items></m:CreateItem>",
        calendar_folder(owner.map(|o| c.address_of(o)).as_deref())
    ))
    .map(|m| new_id(&m))
}

/// An appointment changed, field by field. Asked to tell the guests (`notify`), the server
/// sends the update to all of them and keeps a copy, as Outlook's "Send update" does — on the
/// item as last seen (its change key), so a change made meanwhile isn't overwritten unseen;
/// otherwise the change goes in over whatever is there, telling no one.
fn update_item(c: &Client, id: &str, change_key: &str, e: &CalEvent, notify: bool) -> Result<(String, String)> {
    let mut updates: String = event_fields(e, c.email())
        .iter()
        .map(|(uri, x)| format!("<t:SetItemField><t:FieldURI FieldURI=\"{uri}\"/><t:CalendarItem>{x}</t:CalendarItem></t:SetItemField>"))
        .collect();
    // An appointment that no longer repeats loses its pattern (an occurrence never had one).
    if e.rrule.is_none() && e.recurrence_id.is_none() {
        updates.push_str("<t:DeleteItemField><t:FieldURI FieldURI=\"calendar:Recurrence\"/></t:DeleteItemField>");
    }
    // A meeting of the user's whose room was taken off loses its resources too (a plain
    // appointment, with no one invited, is left alone: nothing to take off).
    let mine = e.organizer.as_ref().is_none_or(|o| o.email.eq_ignore_ascii_case(c.email()));
    if mine && !e.attendees.is_empty() && !e.attendees.iter().any(|a| a.is_resource()) {
        updates.push_str("<t:DeleteItemField><t:FieldURI FieldURI=\"calendar:Resources\"/></t:DeleteItemField>");
    }
    // So with its optional guests, all taken off (or all made required).
    if mine && !e.attendees.is_empty() && !e.attendees.iter().any(|a| a.is_optional()) {
        updates.push_str("<t:DeleteItemField><t:FieldURI FieldURI=\"calendar:OptionalAttendees\"/></t:DeleteItemField>");
    }
    let send = if notify { "SendToAllAndSaveCopy" } else { "SendToNone" };
    let call = |resolution: &str, key: &str| {
        c.call_one(&format!(
            "<m:UpdateItem ConflictResolution=\"{resolution}\" SendMeetingInvitationsOrCancellations=\"{send}\"><m:ItemChanges>\
             <t:ItemChange><t:ItemId Id=\"{}\"{key}/><t:Updates>{updates}</t:Updates></t:ItemChange></m:ItemChanges></m:UpdateItem>",
            esc(id)
        ))
    };
    let mut r = if notify && !change_key.is_empty() {
        call("AutoResolve", &format!(" ChangeKey=\"{}\"", esc(change_key)))
    } else {
        call("AlwaysOverwrite", "")
    };
    // The item changed on the server since it was last read and the two can't be reconciled:
    // this change goes in over it, as it always did (a change kept here forever helps no one).
    if let Err(NetError::No(why) | NetError::Protocol(why)) = &r
        && (why.contains("Conflict") || why.contains("ChangeKey"))
    {
        log::info!("{}: {why}; written over", e.summary);
        r = call("AlwaysOverwrite", "");
    }
    r.map(|m| {
        let (new, key) = new_id(&m);
        (if new.is_empty() { id.to_string() } else { new }, key)
    })
}
