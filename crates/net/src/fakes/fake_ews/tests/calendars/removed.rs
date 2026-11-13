// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Calendars removed from the list on this computer (`Pim::removed`): listed as they were and
//! never asked of the server — one of the mailbox's own other calendars, its Birthdays, and a
//! colleague's kept in one of Outlook's groups — while the others are read as before.

use quatrelle_model::pim::{DavSync, Pim, shared_calendar_id};

use super::*;
use crate::fakes::fake_ews::server::Server;

const NOW: i64 = 1_792_000_000;

fn appointment(subject: &str, day: u32, uid: &str) -> El {
    parse(&format!(
        "<t:CalendarItem><t:Subject>{subject}</t:Subject><t:Start>2026-11-{day:02}T09:00:00Z</t:Start><t:End>2026-11-{day:02}T10:00:00Z</t:End>\
         <t:IsAllDayEvent>false</t:IsAllDayEvent><t:CalendarItemType>Single</t:CalendarItemType><t:UID>{uid}</t:UID></t:CalendarItem>"
    ))
    .kids[0]
        .clone()
}

/// A mailbox with its Calendar, Trips and Birthdays under it, and Ana's calendar in one of
/// Outlook's groups; an appointment in each. Their ids here: (Trips, Birthdays, Ana's).
fn mailbox() -> (Data, String, String, String) {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let cal = d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    let trips = d.folder("Trips", Some(&cal), None, "IPF.Appointment");
    let birthdays = d.folder("Birthdays", Some(&cal), None, "IPF.Appointment.Birthday");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    d.pim.push(("M1".into(), cal, appointment("Stand-up", 16, "standup")));
    d.pim.push(("T1".into(), trips.clone(), appointment("Train to Lyon", 20, "train")));
    d.pim.push(("B1".into(), birthdays.clone(), appointment("Zoe's birthday", 21, "zoe-birthday")));
    d.pim.push(("S1".into(), "mailbox:ana@contoso.example".into(), appointment("Budget review", 18, "budget")));
    let dn = "/o=Contoso/ou=Exchange Administrative Group (FYDIBOHF23SPDLT)/cn=Recipients/cn=ana";
    d.directory = vec![(dn.into(), "ana@contoso.example".into())];
    d.wlinks = vec![wlink("IPM.Microsoft.WunderBar.Link", 3, 2, "Équipe : Studio", "Ana Lima", dn)];
    (d, format!("ews:folder:{trips}"), format!("ews:folder:{birthdays}"), shared_calendar_id("ana@contoso.example"))
}

fn client(server: &Server) -> crate::ews::Client {
    crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap()
}

fn calls(server: &Server) -> usize {
    server.data.lock().unwrap().calls.len()
}

/// A sync from `pim`, and the operations it asked of the server.
fn sync(server: &Server, pim: &Pim) -> (DavSync, usize) {
    let before = calls(server);
    let out = crate::ews_pim::sync(&client(server), pim, NOW).unwrap();
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    (out, calls(server) - before)
}

fn read(sync: &DavSync, calendar: &str) -> bool {
    sync.synced_calendars.iter().any(|c| c == calendar) || sync.events.iter().any(|e| e.calendar == calendar)
}

/// Each removed: still listed as it was (its state kept, to go on from once it's brought back),
/// nothing of it read, one operation fewer asked of the server; the others read as before.
#[test]
fn a_removed_calendar_is_listed_as_it_was_and_never_asked_for() {
    let (d, trips, birthdays, ana) = mailbox();
    let server = serve(d);
    // The first sync lists them all (Ana's not read yet: a first sync only lists other people's).
    let (first, _) = sync(&server, &Pim::default());
    let pim = Pim { calendars: first.calendars.clone().unwrap_or_default(), events: first.events.clone(), ..Default::default() };
    let (all, asked_all) = sync(&server, &pim);
    for c in [&trips, &birthdays, &ana] {
        assert!(read(&all, c), "{c} read when not removed: {:?}", all.synced_calendars);
    }
    for c in [&trips, &birthdays, &ana] {
        let removed = Pim { removed: vec![c.clone()], ..pim.clone() };
        let (out, asked) = sync(&server, &removed);
        assert!(!read(&out, c), "{c} not read");
        let kept = pim.calendars.iter().find(|k| k.id == *c).map(|k| k.ctag.clone());
        assert_eq!(out.calendars.as_ref().and_then(|l| l.iter().find(|k| k.id == *c)).map(|k| k.ctag.clone()), kept, "{c} listed as it was");
        assert!(asked < asked_all, "{c}: {asked} operations asked, {asked_all} with it");
        for other in [&trips, &birthdays, &ana].into_iter().filter(|o| *o != c) {
            assert!(read(&out, other), "{other} still read with {c} removed");
        }
        assert!(read(&out, crate::ews_pim::CALENDAR));
    }
}
