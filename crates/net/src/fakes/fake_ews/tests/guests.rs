// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A change to a meeting asks the server to tell its guests only when it has guests, and a
//! refusal for its recipients doesn't keep the change unsent forever. On 03/11 an appointment
//! the user made, with only themselves as attendee, was sent with `SendToAllAndSaveCopy` at each
//! edit: Exchange refused it ("A message can't be sent because it contains no recipients."), the
//! appointment stayed changed here and was tried again at every sync, 61 times in an hour.

use quatrelle_model::ical::{Attendee, CalEvent};
use quatrelle_model::pim::{Pim, StoredEvent};

use super::*;

const ME: &str = "riley@contoso.example";

fn person(email: &str, partstat: &str) -> Attendee {
    Attendee { email: email.into(), name: email.into(), partstat: partstat.into(), ..Default::default() }
}

/// A server whose calendar holds `test` (item E1, no one invited), and a client signed in to it.
fn calendar() -> (super::server::Server, crate::ews::Client, String) {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let cal = d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let test = parse(
        "<t:CalendarItem><t:Subject>test</t:Subject><t:Start>2026-11-03T09:00:00Z</t:Start><t:End>2026-11-03T10:00:00Z</t:End>\
         <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>test-1</t:UID><t:CalendarItemType>Single</t:CalendarItemType></t:CalendarItem>",
    );
    d.pim.push(("E1".into(), cal.clone(), test.kids[0].clone()));
    d.refused_guests.push("gone@contoso.example".into());
    d.refused_guests.push("odd-gone@contoso.example".into());
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, ME).unwrap();
    (server, c, cal)
}

/// The appointment as changed here: on the server already (`href`) or new, with these
/// attendees, organized by the user, the guests to be told.
fn changed(href: &str, uid: &str, attendees: Vec<Attendee>) -> StoredEvent {
    StoredEvent {
        calendar: crate::ews_pim::CALENDAR.into(),
        href: href.into(),
        etag: if href.is_empty() { String::new() } else { "ck".into() },
        event: CalEvent {
            uid: uid.into(),
            summary: "test 2".into(),
            start: 1_793_696_400,
            end: 1_793_700_000,
            organizer: Some(person(ME, "")),
            attendees,
            ..Default::default()
        },
        dirty: true,
        notify: true,
        respond: None,
    }
}

fn subject_of(server: &super::server::Server, id: &str) -> String {
    let data = server.data.lock().unwrap();
    data.pim.iter().find(|(i, _, _)| i == id).and_then(|(_, _, el)| el.text_of("Subject")).unwrap_or_default().to_string()
}

/// The user alone as attendee (Exchange lists the organizer among them): no one to tell. The
/// change is saved telling no one, changed or new, and nothing is left to send.
#[test]
fn an_appointment_with_only_the_user_is_saved_telling_no_one() {
    let (server, c, _) = calendar();
    let pim = Pim {
        events: vec![changed("E1", "test-1", vec![person(ME, "ACCEPTED")]), changed("", "new-1", vec![person(ME, "ACCEPTED")])],
        ..Default::default()
    };
    let out = crate::ews_pim::sync(&c, &pim, 1_793_600_000).unwrap();
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    assert_eq!(out.pushed_events.len(), 2, "both saved: {:?}", out.pushed_events);
    assert!(out.pushed_events.iter().all(|e| !e.dirty && !e.notify));
    assert_eq!(subject_of(&server, "E1"), "test 2");
    assert_eq!(server.data.lock().unwrap().meeting_sends, ["SendToNone", "SendToNone"]);
}

/// A guest the server won't send to: the change is saved all the same, telling no one, and
/// one error says who wasn't told, in the server's words. Nothing is left to send: the next
/// sync sends nothing.
#[test]
fn a_guest_the_server_refuses_leaves_the_change_saved_and_says_so() {
    let (server, c, _) = calendar();
    let pim = Pim {
        events: vec![changed("E1", "test-1", vec![person(ME, "ACCEPTED"), person("gone@contoso.example", "NEEDS-ACTION")])],
        ..Default::default()
    };
    let out = crate::ews_pim::sync(&c, &pim, 1_793_600_000).unwrap();
    assert_eq!(out.errors.len(), 1, "{:?}", out.errors);
    let error = &out.errors[0];
    assert!(error.contains("gone@contoso.example") && error.contains("At least one recipient isn't valid."), "{error}");
    assert_eq!(out.pushed_events.len(), 1, "saved: {:?}", out.pushed_events);
    assert!(!out.pushed_events[0].dirty && !out.pushed_events[0].notify);
    assert_eq!(subject_of(&server, "E1"), "test 2");
    assert_eq!(server.data.lock().unwrap().meeting_sends, ["SendToAllAndSaveCopy", "SendToNone"]);
    // As the engine keeps it: what was pushed, clean.
    let next = Pim { events: out.pushed_events.clone(), ..Default::default() };
    let again = crate::ews_pim::sync(&c, &next, 1_793_600_100).unwrap();
    assert!(again.errors.is_empty(), "{:?}", again.errors);
    assert_eq!(server.data.lock().unwrap().meeting_sends.len(), 2, "nothing sent again");
}

/// The on-premises server's code was never seen, only its words: a refusal of a send that
/// speaks of recipients is one whatever its code, and is saved telling no one just the same.
#[test]
fn a_refusal_for_recipients_under_another_code_is_treated_the_same() {
    let (server, c, _) = calendar();
    let pim = Pim {
        events: vec![changed("E1", "test-1", vec![person(ME, "ACCEPTED"), person("odd-gone@contoso.example", "NEEDS-ACTION")])],
        ..Default::default()
    };
    let out = crate::ews_pim::sync(&c, &pim, 1_793_600_000).unwrap();
    assert_eq!(out.errors.len(), 1, "{:?}", out.errors);
    assert!(out.errors[0].contains("At least one recipient isn't valid."), "{}", out.errors[0]);
    assert_eq!(out.pushed_events.len(), 1);
    assert!(!out.pushed_events[0].dirty && !out.pushed_events[0].notify);
    assert_eq!(subject_of(&server, "E1"), "test 2");
    assert_eq!(server.data.lock().unwrap().meeting_sends, ["SendToAllAndSaveCopy", "SendToNone"]);
}

/// A new meeting with a guest the server refuses is created once, telling no one, with one error.
#[test]
fn a_new_meeting_with_a_refused_guest_is_created_telling_no_one() {
    let (server, c, _) = calendar();
    let pim = Pim {
        events: vec![changed("", "new-1", vec![person(ME, "ACCEPTED"), person("gone@contoso.example", "NEEDS-ACTION")])],
        ..Default::default()
    };
    let out = crate::ews_pim::sync(&c, &pim, 1_793_600_000).unwrap();
    assert_eq!(out.errors.len(), 1, "{:?}", out.errors);
    assert!(out.errors[0].contains("gone@contoso.example"), "{}", out.errors[0]);
    assert_eq!(out.pushed_events.len(), 1);
    let e = &out.pushed_events[0];
    assert!(!e.href.is_empty() && !e.dirty && !e.notify, "{e:?}");
    assert_eq!(subject_of(&server, &e.href), "test 2");
    let d = server.data.lock().unwrap();
    assert_eq!(d.meeting_sends, ["SendToAllAndSaveCopy", "SendToNone"]);
    assert_eq!(d.pim.iter().filter(|(_, _, el)| el.text_of("Subject") == Some("test 2")).count(), 1, "created once");
}

/// The error names a few guests, then how many more, not all of up to 500.
#[test]
fn the_error_names_a_few_refused_guests_and_counts_the_others() {
    let (_server, c, _) = calendar();
    let mut guests = vec![person(ME, "ACCEPTED"), person("gone@contoso.example", "NEEDS-ACTION")];
    guests.extend((0..40).map(|i| person(&format!("g{i}@contoso.example"), "NEEDS-ACTION")));
    let pim = Pim { events: vec![changed("E1", "test-1", guests)], ..Default::default() };
    let out = crate::ews_pim::sync(&c, &pim, 1_793_600_000).unwrap();
    assert_eq!(out.errors.len(), 1, "{:?}", out.errors);
    let error = &out.errors[0];
    assert!(error.contains("and 39 others") || error.contains("others"), "{error}");
    assert!(error.len() < 400, "{} bytes: {error}", error.len());
}
