// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! An answer to an invitation the server refuses for good is said once, naming what Exchange
//! named, rather than sent again at every sync; and someone else's meeting changed here goes
//! without the organizer's zone, which Exchange refuses on a guest's copy.

use quatrelle_model::ical::CalEvent;
use quatrelle_model::pim::{DavSync, Pim, StoredEvent, same_meeting};

use super::*;

/// How a server refuses: its code, its text, and what its `MessageXml` holds.
type Refusal = (String, String, String);

/// Exchange's words for a property it won't set, the field named taken for the organizer.
fn invalid_set() -> Refusal {
    ("ErrorInvalidPropertySet".into(), "Set action is invalid for property.".into(), "<t:FieldURI FieldURI=\"calendar:Organizer\"/>".into())
}

/// A server whose calendar holds Ana's meeting `mtg-1` (item E1, set in Paris time, with a
/// reminder), refusing answers so, and a client signed in to it.
fn refusing(how: Refusal) -> (super::server::Server, crate::ews::Client) {
    serving(Some(how))
}

/// The same server, refusing answers or not.
fn serving(how: Option<Refusal>) -> (super::server::Server, crate::ews::Client) {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let cal = d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let meeting = parse(
        "<t:CalendarItem><t:Subject>Review</t:Subject><t:ReminderIsSet>true</t:ReminderIsSet>\
         <t:ReminderMinutesBeforeStart>15</t:ReminderMinutesBeforeStart><t:Start>2026-10-27T09:00:00Z</t:Start><t:End>2026-10-27T10:00:00Z</t:End>\
         <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>mtg-1</t:UID><t:CalendarItemType>Single</t:CalendarItemType>\
         <t:Organizer><t:Mailbox><t:Name>Ana</t:Name><t:EmailAddress>ana@sfr.example</t:EmailAddress></t:Mailbox></t:Organizer>\
         <t:RequiredAttendees><t:Attendee><t:Mailbox><t:EmailAddress>riley@contoso.example</t:EmailAddress></t:Mailbox><t:ResponseType>Unknown</t:ResponseType></t:Attendee></t:RequiredAttendees>\
         <t:MyResponseType>NoResponseReceived</t:MyResponseType><t:StartTimeZone Id=\"Romance Standard Time\"/></t:CalendarItem>",
    );
    d.pim.push(("E1".into(), cal, meeting.kids[0].clone()));
    d.refuse_answers = how;
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    (server, c)
}

/// Someone else's meeting changed here (its reminder dismissed, which turns it off as Outlook
/// does) goes to the server without the organizer's zone: Exchange refuses it written on a
/// guest's copy, and the whole change with it ("Set action is invalid for property.", naming
/// `calendar:StartTimeZone`). The owner's two outside meetings were refused so at every sync on
/// 04/11, which was taken at first for a refused answer.
#[test]
fn someone_elses_meeting_changed_here_is_saved_without_its_zone() {
    let (server, c) = serving(None);
    let first = crate::ews_pim::sync(&c, &Pim::default(), 1_792_000_000).unwrap();
    let mut pim = Pim { events: first.events, ..Default::default() };
    let e = pim.events.iter_mut().find(|e| same_meeting(&e.event.uid, "mtg-1")).expect("the server's meeting");
    assert_eq!(e.event.tzid, "Romance Standard Time");
    assert_eq!(e.event.reminder, Some(15));
    e.event.reminder = None;
    e.dirty = true;
    let out = crate::ews_pim::sync(&c, &pim, 1_792_000_100).unwrap();
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    assert_eq!(out.pushed_events.len(), 1, "saved");
    let d = server.data.lock().unwrap();
    let item = &d.pim.iter().find(|(id, _, _)| id == "E1").expect("the meeting").2;
    assert_eq!(item.text_of("ReminderIsSet"), Some("false"), "the reminder off on the server");
    assert_eq!(item.child("StartTimeZone").and_then(|z| z.attr("Id")), Some("Romance Standard Time"), "Ana's zone as it was");
}

/// The meeting as read, answered `ACCEPTED` here: the next sync answers it on its item.
fn accepted_on_its_item(c: &crate::ews::Client) -> (Pim, String) {
    let first = crate::ews_pim::sync(c, &Pim::default(), 1_792_000_000).unwrap();
    let mut pim = Pim { events: first.events, ..Default::default() };
    let e = pim.events.iter_mut().find(|e| same_meeting(&e.event.uid, "mtg-1")).expect("the server's meeting");
    e.respond = Some("ACCEPTED".into());
    e.dirty = true;
    let uid = e.event.uid.clone();
    (pim, uid)
}

/// Answered `TENTATIVE` before the calendar here had the meeting: answered once it's read.
fn answered_before_it_was_here() -> Pim {
    let placeholder = StoredEvent {
        calendar: crate::ews_pim::CALENDAR.into(),
        event: CalEvent { uid: "mtg-1".into(), summary: "Review".into(), ..Default::default() },
        dirty: true,
        respond: Some("TENTATIVE".into()),
        ..Default::default()
    };
    Pim { events: vec![placeholder], ..Default::default() }
}

/// The one answer refused, by its invitation's UID: why, which names the field.
fn refused_once(out: &DavSync, uid: &str) {
    assert!(out.errors.is_empty(), "not an error to try again: {:?}", out.errors);
    assert!(out.responded.is_empty(), "{:?}", out.responded);
    let [(refused, why)] = out.refused_answers.as_slice() else { panic!("one refused answer: {:?}", out.refused_answers) };
    assert_eq!(refused, uid);
    assert!(why.contains("Set action is invalid for property") && why.contains("calendar:Organizer"), "{why}");
}

#[test]
fn a_refused_answer_is_reported_once_as_refused() {
    let (server, c) = refusing(invalid_set());
    // On the meeting's item.
    let (pim, uid) = accepted_on_its_item(&c);
    let out = crate::ews_pim::sync(&c, &pim, 1_792_000_100).unwrap();
    refused_once(&out, &uid);
    // Answered before the meeting was here: refused the same way once it's read.
    let out = crate::ews_pim::sync(&c, &answered_before_it_was_here(), 1_792_000_200).unwrap();
    refused_once(&out, "mtg-1");
    assert!(server.data.lock().unwrap().answers.is_empty(), "never given");
}

/// A server too busy to answer now hasn't refused: the answer stays an error, tried again.
#[test]
fn an_answer_refused_while_the_server_is_busy_is_tried_again() {
    let busy = (
        "ErrorServerBusy".to_string(),
        "The server cannot service this request right now. Try again later.".to_string(),
        "<t:Value Name=\"BackOffMilliseconds\">10</t:Value>".to_string(),
    );
    let (_server, c) = refusing(busy);
    let (pim, _) = accepted_on_its_item(&c);
    for (pim, at) in [(pim, 1_792_000_100), (answered_before_it_was_here(), 1_792_000_200)] {
        let out = crate::ews_pim::sync(&c, &pim, at).unwrap();
        assert!(out.refused_answers.is_empty(), "{:?}", out.refused_answers);
        assert!(out.responded.is_empty(), "{:?}", out.responded);
        assert_eq!(out.errors.len(), 1, "{:?}", out.errors);
        assert!(out.errors[0].starts_with("Review: "), "{:?}", out.errors);
    }
}

/// What the organizer meant the meeting to show as once accepted (`PidLidIntendedBusyStatus`,
/// on the guest's copy as on the invitation) is read with it: accepting shows it at once, as
/// Outlook does, not Busy until the next read.
#[test]
fn the_status_the_organizer_meant_is_read() {
    use quatrelle_model::ical::{ShowAs, intended_show_as};
    for (value, meant) in [("0", Some(ShowAs::Free)), ("3", Some(ShowAs::OutOfOffice)), ("-1", None), ("junk", None)] {
        let mut d = Data::default();
        d.folder("Inbox", None, Some("inbox"), "IPF.Note");
        let cal = d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
        d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
        let meeting = parse(&format!(
            "<t:CalendarItem><t:Subject>All hands</t:Subject><t:Start>2026-10-27T09:00:00Z</t:Start><t:End>2026-10-27T10:00:00Z</t:End>\
             <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>mtg-2</t:UID><t:CalendarItemType>Single</t:CalendarItemType>\
             <t:LegacyFreeBusyStatus>Tentative</t:LegacyFreeBusyStatus>\
             <t:Organizer><t:Mailbox><t:Name>Ana</t:Name><t:EmailAddress>ana@contoso.example</t:EmailAddress></t:Mailbox></t:Organizer>\
             <t:MyResponseType>NoResponseReceived</t:MyResponseType>\
             <t:ExtendedProperty><t:ExtendedFieldURI DistinguishedPropertySetId=\"Appointment\" PropertyId=\"33316\" PropertyType=\"Integer\"/>\
             <t:Value>{value}</t:Value></t:ExtendedProperty></t:CalendarItem>"
        ));
        d.pim.push(("E2".into(), cal, meeting.kids[0].clone()));
        let server = serve(d);
        let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
        let out = crate::ews_pim::sync(&c, &Pim::default(), 1_792_000_000).unwrap();
        let e = out.events.iter().find(|e| same_meeting(&e.event.uid, "mtg-2")).expect("the meeting");
        assert_eq!(e.event.show_as, ShowAs::Tentative, "shown as the server has it until answered");
        assert_eq!(intended_show_as(&e.event), meant, "meant: {value}");
        let mut answered = e.event.clone();
        quatrelle_model::ical::show_as_answered(&mut answered, "ACCEPTED");
        assert_eq!(answered.show_as, meant.unwrap_or(ShowAs::Busy), "accepted: {value}");
    }
}
