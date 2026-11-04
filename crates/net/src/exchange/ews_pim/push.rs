// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! EWS calendars: the appointments changed here, sent to the server — new ones created, changed
//! ones updated field by field, an occurrence changed on its own through the occurrence's own
//! item — and the guests told when the change asks for it; invitations answered as Outlook
//! answers them, by the server.

use super::events::guests_written;
use super::*;

/// Send every appointment changed here, noting each one's item on the server in `out`.
pub(super) fn push_events(c: &Client, pim: &Pim, out: &mut DavSync) {
    for e in pim.events.iter().filter(|e| e.dirty) {
        // The series of an occurrence found gone already: it goes with it, said once.
        if e.event.recurrence_id.is_some()
            && e.href.is_empty()
            && series_of(pim, out, &e.event.uid).is_some_and(|(h, _)| out.gone_events.contains(&h))
        {
            continue;
        }
        // An answer to an invitation: given by the server on the meeting it placed in the
        // calendar; one not in the calendar here yet waits for it to be read
        // ([`answer_pending`]). Never created as an appointment of the user's own: that made a
        // second copy of the meeting, with the user as its organizer.
        if let Some(answer) = &e.respond {
            if !e.href.is_empty() && e.event.recurrence_id.is_none() {
                note_answer(out, e, answer, respond_item(c, &e.href, answer));
            }
            continue;
        }
        let owner = quatrelle_model::pim::shared_calendar_owner(&e.calendar);
        // The server is asked to tell the guests only when the appointment goes with some: asked
        // to send to no one (the user alone, as Exchange lists the organizer), it refuses.
        let guests = guests_written(&e.event, c.email());
        let notify = e.notify && !guests.is_empty();
        // The item a change goes to, if there's one already: the appointment's, or its series'.
        let mut item = e.href.clone();
        let r = match e.event.recurrence_id.filter(|_| e.href.is_empty()) {
            // One occurrence of a series, changed on its own: the occurrence's item is asked for
            // by its place in the series, then changed as any appointment is.
            Some(at) => match series_of(pim, out, &e.event.uid) {
                Some((master_id, master)) => {
                    item.clone_from(&master_id);
                    occurrence_item_id(c, &master_id, &master, at).and_then(|(id, key)| update_item(c, &id, &key, &e.event, notify))
                }
                None => Err(NetError::No("the series isn't on the server yet".into())),
            },
            None if e.href.is_empty() => create_item(c, owner, &e.event, notify),
            None => update_item(c, &e.href, &e.etag, &e.event, notify),
        };
        match r {
            // Deleted on the server meanwhile (a "test" appointment, 03/11, refused at every sync):
            // the server wins; the engine lets it go here too, and says so.
            Err(NetError::Gone(why)) if !item.is_empty() => {
                log::info!("{}: deleted on the server ({why}); the change made here is dropped", e.event.summary);
                if !out.gone_events.contains(&item) {
                    out.gone_events.push(item);
                }
            }
            // Someone else's event stays in their calendar.
            Ok(Saved { href, etag, untold }) => {
                // Saved, but the server wouldn't send to its guests: said once, not tried again.
                if let Some(why) = untold {
                    out.errors.push(format!("{}: saved, but the server wouldn't tell the guests ({}): {why}", e.event.summary, some_names(&guests)));
                }
                out.pushed_events.push(StoredEvent {
                    calendar: if owner.is_some() { e.calendar.clone() } else { CALENDAR.into() },
                    href,
                    etag,
                    event: e.event.clone(),
                    dirty: false,
                    notify: false,
                    respond: None,
                });
            }
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
            note_answer(out, e, answer, respond_item(c, &href, answer));
        }
    }
}

/// What came of answering an invitation, in `out`: given; refused (the engine sends it again at
/// the next sync, and refused again, says so once and lets it go, rather than send it at every
/// sync); or not now, tried again at the next sync.
fn note_answer(out: &mut DavSync, e: &StoredEvent, answer: &str, r: Result<()>) {
    match r {
        Ok(()) => out.responded.push((e.event.uid.clone(), answer.to_string())),
        Err(NetError::No(why)) => {
            log::warn!("{}: the server refused the answer: {why}", e.event.summary);
            out.refused_answers.push((e.event.uid.clone(), why));
        }
        Err(x) => out.errors.push(format!("{}: {x}", e.event.summary)),
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

/// Whether an answer's code says the item itself isn't on the server (deleted there): not a
/// folder not found (`ErrorFolderNotFound`: a colleague's calendar no longer shared), which
/// stays an error.
pub(crate) fn item_gone_code(code: &str) -> bool {
    code.trim() == "ErrorItemNotFound"
}

/// An answer about an item the server no longer has: `NetError::Gone`.
fn gone(m: &El) -> Result<()> {
    let code = m.text_of("ResponseCode").unwrap_or_default();
    if m.attr("ResponseClass") == Some("Error") && item_gone_code(code) {
        let text = m.text_of("MessageText").unwrap_or_default().trim();
        return Err(NetError::Gone(if text.is_empty() { code.trim() } else { text }.to_string()));
    }
    Ok(())
}

/// One operation on one item ([`Client::call_one`]), the item not found said as
/// `NetError::Gone`.
pub(super) fn call_item(c: &Client, body: &str) -> Result<El> {
    let doc = c.call(body)?;
    let m = crate::ews::messages(&doc).first().map(|m| (*m).clone()).ok_or_else(|| NetError::Protocol("an empty answer".into()))?;
    gone(&m)?;
    crate::ews::check_message(&m)?;
    Ok(m)
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

/// An appointment saved on the server: its item and change key, and the server's words when it
/// wouldn't send to the guests (the change saved telling no one instead).
struct Saved {
    href: String,
    etag: String,
    untold: Option<String>,
}

/// What Exchange answers a meeting it won't send for its recipients: none left, or one it won't
/// take.
const RECIPIENT_ERRORS: [&str; 2] = ["ErrorInvalidRecipients", "ErrorMissingRecipients"];

/// Most guests named in a message.
const NAMES_SHOWN: usize = 3;

/// A few of the names, then how many more.
fn some_names(names: &[&str]) -> String {
    let shown = names.iter().take(NAMES_SHOWN).copied().collect::<Vec<_>>().join(", ");
    match names.len().saturating_sub(NAMES_SHOWN) {
        0 => shown,
        n => format!("{shown} and {n} others"),
    }
}

/// Whether a refusal is for the recipients: by its code, or (the on-premises server's code was
/// never seen, only its words) by its text speaking of recipients.
fn refuses_recipients(code: &str, text: &str) -> bool {
    RECIPIENT_ERRORS.contains(&code.trim()) || text.to_ascii_lowercase().contains("recipient")
}

/// One request that may ask the server to send a meeting (`sending`): a refusal for its
/// recipients comes back as `Ok(Err(the server's words))`, for the change to be saved telling no
/// one. Not asked to send, no refusal is one for recipients.
fn call_sending(c: &Client, body: &str, sending: bool) -> Result<std::result::Result<El, String>> {
    let doc = c.call(body)?;
    let m = crate::ews::messages(&doc).first().map(|m| (*m).clone()).ok_or_else(|| NetError::Protocol("an empty answer".into()))?;
    let (code, text) = (m.text_of("ResponseCode").unwrap_or_default(), m.text_of("MessageText").unwrap_or_default());
    gone(&m)?;
    if sending && m.attr("ResponseClass") == Some("Error") && refuses_recipients(code, text) {
        return Ok(Err(if text.trim().is_empty() { code } else { text }.trim().to_string()));
    }
    crate::ews::check_message(&m)?;
    Ok(Ok(m))
}

/// A refusal for recipients when the guests were to be told: logged, and the change made
/// again (`again`, telling no one); otherwise the answer as it came.
fn or_untold(
    summary: &str,
    first: Result<std::result::Result<El, String>>,
    again: impl FnOnce() -> Result<std::result::Result<El, String>>,
) -> Result<(El, Option<String>)> {
    match first? {
        Ok(m) => Ok((m, None)),
        Err(why) => {
            log::info!("{summary}: {why}; saved without telling the guests");
            let m = again()?.map_err(NetError::No)?;
            Ok((m, Some(why)))
        }
    }
}

/// A new appointment, in this mailbox's calendar or someone else's; the guests invited by the
/// server when asked (`notify`, as Outlook's Send), else told nothing. One the server won't
/// send for its recipients is made all the same, telling no one.
fn create_item(c: &Client, owner: Option<&str>, e: &CalEvent, notify: bool) -> Result<Saved> {
    let body: String = event_fields(e, c.email()).iter().map(|(_, x)| x.as_str()).collect();
    let folder = calendar_folder(owner.map(|o| c.address_of(o)).as_deref());
    let call = |send: &str| {
        call_sending(
            c,
            &format!(
                "<m:CreateItem SendMeetingInvitations=\"{send}\"><m:SavedItemFolderId>{folder}</m:SavedItemFolderId>\
                 <m:Items><t:CalendarItem>{body}</t:CalendarItem></m:Items></m:CreateItem>"
            ),
            send != "SendToNone",
        )
    };
    let first = call(if notify { "SendToAllAndSaveCopy" } else { "SendToNone" });
    let (m, untold) = or_untold(&e.summary, first, || call("SendToNone"))?;
    let (href, etag) = new_id(&m);
    Ok(Saved { href, etag, untold })
}

/// An appointment changed, field by field. Asked to tell the guests (`notify`), the server
/// sends the update to all of them and keeps a copy, as Outlook's "Send update" does — on the
/// item as last seen (its change key), so a change made meanwhile isn't overwritten unseen;
/// otherwise the change goes in over whatever is there, telling no one. One the server won't
/// send for its recipients goes in all the same, telling no one.
fn update_item(c: &Client, id: &str, change_key: &str, e: &CalEvent, notify: bool) -> Result<Saved> {
    let updates = updates(e, c.email());
    let call = |resolution: &str, key: &str, send: &str| {
        call_sending(
            c,
            &format!(
                "<m:UpdateItem ConflictResolution=\"{resolution}\" SendMeetingInvitationsOrCancellations=\"{send}\"><m:ItemChanges>\
                 <t:ItemChange><t:ItemId Id=\"{}\"{key}/><t:Updates>{updates}</t:Updates></t:ItemChange></m:ItemChanges></m:UpdateItem>",
                esc(id)
            ),
            send != "SendToNone",
        )
    };
    let send = if notify { "SendToAllAndSaveCopy" } else { "SendToNone" };
    let mut r = if notify && !change_key.is_empty() {
        call("AutoResolve", &format!(" ChangeKey=\"{}\"", esc(change_key)), send)
    } else {
        call("AlwaysOverwrite", "", send)
    };
    // The item changed on the server since it was last read and the two can't be reconciled:
    // this change goes in over it, as it always did (a change kept here forever helps no one).
    if let Err(NetError::No(why) | NetError::Protocol(why)) = &r
        && (why.contains("Conflict") || why.contains("ChangeKey"))
    {
        log::info!("{}: {why}; written over", e.summary);
        r = call("AlwaysOverwrite", "", send);
    }
    // Refused for its recipients: saved telling no one, over whatever is there (as above).
    let (m, untold) = or_untold(&e.summary, r, || call("AlwaysOverwrite", "", "SendToNone"))?;
    let (new, key) = new_id(&m);
    Ok(Saved { href: if new.is_empty() { id.to_string() } else { new }, etag: key, untold })
}

/// What only a meeting's organizer sets: its pattern and the zone it's set in.
const ORGANIZERS_OWN: [&str; 3] = ["calendar:Recurrence", "calendar:StartTimeZone", "calendar:EndTimeZone"];

/// The changes [`update_item`] makes, field by field: each field of the appointment set, and
/// those it no longer has taken off. `me`: this account's address.
///
/// The user's copy of someone else's meeting (the server keeps the user's answer to it, where
/// it says "Organizer" of the user's own) keeps the organizer's pattern and zone, neither set nor
/// taken off: Exchange refuses the zone written on a guest's copy, and the whole change with it
/// ("Set action is invalid for property.", naming `calendar:StartTimeZone`). Two outside
/// meetings changed here were refused so at every sync on 04/11, which was taken at
/// first for a refused answer.
pub(super) fn updates(e: &CalEvent, me: &str) -> String {
    let guests_copy = !e.my_answer.is_empty();
    let mut updates: String = event_fields(e, me)
        .iter()
        .filter(|(uri, _)| !(guests_copy && ORGANIZERS_OWN.contains(uri)))
        .map(|(uri, x)| format!("<t:SetItemField>{}<t:CalendarItem>{x}</t:CalendarItem></t:SetItemField>", tasks::path(uri)))
        .collect();
    // An appointment that no longer repeats loses its pattern (an occurrence never had one).
    if e.rrule.is_none() && e.recurrence_id.is_none() && !guests_copy {
        updates.push_str("<t:DeleteItemField><t:FieldURI FieldURI=\"calendar:Recurrence\"/></t:DeleteItemField>");
    }
    // A meeting of the user's whose room was taken off loses its resources too (a plain
    // appointment, with no one invited, is left alone: nothing to take off).
    let mine = e.organizer.as_ref().is_none_or(|o| o.email.eq_ignore_ascii_case(me));
    if mine && !e.attendees.is_empty() && !e.attendees.iter().any(|a| a.is_resource()) {
        updates.push_str("<t:DeleteItemField><t:FieldURI FieldURI=\"calendar:Resources\"/></t:DeleteItemField>");
    }
    // So with its optional guests, all taken off (or all made required).
    if mine && !e.attendees.is_empty() && !e.attendees.iter().any(|a| a.is_optional()) {
        updates.push_str("<t:DeleteItemField><t:FieldURI FieldURI=\"calendar:OptionalAttendees\"/></t:DeleteItemField>");
    }
    updates
}
