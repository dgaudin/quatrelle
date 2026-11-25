// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! An occurrence of a series deleted here goes to the server: the series' day taken out was sent
//! with the series' other fields, which say nothing of it, and the occurrence came back at the
//! next read.

use quatrelle_model::pim::Pim;

use super::*;

/// A weekly stand-up the user organizes, with a guest, from 5 October 2026.
fn standup() -> El {
    parse(
        "<t:CalendarItem><t:Subject>Stand-up</t:Subject><t:Start>2026-10-05T09:00:00Z</t:Start><t:End>2026-10-05T09:15:00Z</t:End>\
         <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>standup-2</t:UID><t:CalendarItemType>RecurringMaster</t:CalendarItemType>\
         <t:Organizer><t:Mailbox><t:Name>Riley</t:Name><t:EmailAddress>riley@contoso.example</t:EmailAddress></t:Mailbox></t:Organizer>\
         <t:RequiredAttendees><t:Attendee><t:Mailbox><t:Name>Ana Lima</t:Name><t:EmailAddress>ana@contoso.example</t:EmailAddress></t:Mailbox></t:Attendee></t:RequiredAttendees>\
         <t:Recurrence><t:WeeklyRecurrence><t:Interval>1</t:Interval><t:DaysOfWeek>Monday</t:DaysOfWeek></t:WeeklyRecurrence>\
         <t:NoEndRecurrence><t:StartDate>2026-10-05Z</t:StartDate></t:NoEndRecurrence></t:Recurrence></t:CalendarItem>",
    )
    .kids[0]
        .clone()
}

/// The third Monday deleted here (the series' day taken out, the series to send): the server
/// deletes that occurrence (asked by its place in the series, its guests not told by the server:
/// the cancellation is mailed as before), the series read back keeps the day out, and a series
/// sent again with a day the server took out already asks nothing more of it.
#[test]
fn an_occurrence_deleted_here_is_deleted_on_the_server() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let cal = d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    d.pim.push(("E1".into(), cal, standup()));
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let third = quatrelle_jmap::utc_date("2026-10-19T09:00:00Z").unwrap();
    let first = crate::ews_pim::sync(&c, &Pim::default(), 1_792_000_000).unwrap();
    let mut pim = Pim { events: first.events.clone(), ..Default::default() };
    for e in pim.events.iter_mut().filter(|e| e.event.uid == "standup-2") {
        e.event.exdates.push(third);
        e.dirty = true;
    }
    let second = crate::ews_pim::sync(&c, &pim, 1_792_000_100).unwrap();
    assert!(second.errors.is_empty(), "{:?}", second.errors);
    assert_eq!(server.data.lock().unwrap().cancellations, [("E1:occ3".to_string(), "SendToNone".to_string())]);
    let master = |s: &quatrelle_model::pim::DavSync| s.events.iter().find(|e| e.event.uid == "standup-2").map(|e| e.event.exdates.clone());
    assert_eq!(master(&second), Some(vec![third]), "the series read back has the occurrence again");
    // Sent again (changed again here): the day out is the server's already.
    let mut pim = Pim { events: second.events.clone(), ..Default::default() };
    for e in pim.events.iter_mut().filter(|e| e.event.uid == "standup-2") {
        e.event.location = "Room 2".into();
        e.dirty = true;
    }
    let third_sync = crate::ews_pim::sync(&c, &pim, 1_792_000_200).unwrap();
    assert!(third_sync.errors.is_empty(), "{:?}", third_sync.errors);
    assert_eq!(server.data.lock().unwrap().cancellations.len(), 1, "deleted again");
    assert_eq!(master(&third_sync), Some(vec![third]));
}
