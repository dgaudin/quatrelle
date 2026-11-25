// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A meeting the user organizes, deleted here: the server tells its guests it's off, under the
//! meeting's own UID. The cancellation mailed from here carried a UID their calendars didn't know.

use quatrelle_model::pim::{Pim, Tombstone};

use super::*;

fn appointment(subject: &str, day: u32, uid: &str) -> El {
    parse(&format!(
        "<t:CalendarItem><t:Subject>{subject}</t:Subject><t:Start>2026-11-{day:02}T09:00:00Z</t:Start><t:End>2026-11-{day:02}T10:00:00Z</t:End>\
         <t:IsAllDayEvent>false</t:IsAllDayEvent><t:CalendarItemType>Single</t:CalendarItemType><t:UID>{uid}</t:UID></t:CalendarItem>"
    ))
    .kids[0]
        .clone()
}

/// The meeting's deletion marked for the server to tell its guests: deleted with the server's
/// cancellation to all of them, a copy kept, as Outlook's "Send cancellation"; another deletion
/// (an appointment of the user's alone, someone else's meeting) tells no one.
#[test]
fn a_meeting_deleted_here_is_cancelled_by_the_server() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let cal = d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    d.pim.push(("E1".into(), cal.clone(), appointment("Budget", 3, "budget-1")));
    d.pim.push(("E2".into(), cal, appointment("Dentist", 4, "dentist-1")));
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let calendar = crate::ews_pim::CALENDAR.to_string();
    let pim = Pim {
        deleted: vec![
            Tombstone { href: "E1".into(), etag: String::new(), calendar: calendar.clone(), notify: true },
            Tombstone { href: "E2".into(), etag: String::new(), calendar, notify: false },
        ],
        ..Default::default()
    };
    let sync = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    assert_eq!(sync.deleted, ["E1", "E2"]);
    let told = server.data.lock().unwrap().cancellations.clone();
    assert_eq!(told, [("E1".to_string(), "SendToAllAndSaveCopy".to_string()), ("E2".to_string(), "SendToNone".to_string())]);
}
