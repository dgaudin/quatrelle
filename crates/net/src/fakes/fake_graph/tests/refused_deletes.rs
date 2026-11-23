// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What Microsoft 365 says to a delete or a reminder answer: only "it isn't there" is done; a
//! refusal (access denied) is said and the change stays for the next sync.

use quatrelle_model::Op;
use quatrelle_model::pim::{Pim, ReminderAnswer, Tombstone};
use serde_json::json;

use super::*;

fn contact_deleted(href: &str) -> Pim {
    Pim { deleted: vec![Tombstone { href: format!("contact:{href}"), ..Default::default() }], ..Default::default() }
}

fn account() -> Data {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"));
    d.calendars.push(("CAL1".into(), "Calendar".into()));
    d.calendars.push(("CAL2".into(), "Trips".into()));
    d
}

/// A delete Graph refuses (403) isn't taken for done: it is said, kept to be asked again, and
/// the contact is still on the server.
#[test]
fn a_refused_delete_is_said_and_kept() {
    let mut d = account();
    d.contacts.push(json!({"id": "CT1", "displayName": "Zoe Park"}));
    d.forbid.push("DELETE /me/contacts".into());
    let server = serve(d);
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let pim = contact_deleted("CT1");
    let sync = crate::graph_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    assert!(sync.deleted.is_empty(), "still to delete: {:?}", sync.deleted);
    assert_eq!(sync.errors.len(), 1, "{:?}", sync.errors);
    assert_eq!(server.data.lock().unwrap().contacts.len(), 1);
    // Allowed again: it goes.
    server.data.lock().unwrap().forbid.clear();
    let sync = crate::graph_pim::sync(&c, &pim, 1_792_000_060).unwrap();
    assert_eq!(sync.deleted.len(), 1, "{:?} {:?}", sync.deleted, sync.errors);
}

/// A delete of what Graph no longer has (404) is done.
#[test]
fn a_delete_of_what_is_not_there_is_done() {
    let server = serve(account());
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let sync = crate::graph_pim::sync(&c, &contact_deleted("CT9"), 1_792_000_000).unwrap();
    assert_eq!(sync.deleted.len(), 1, "{:?}", sync.deleted);
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
}

/// A calendar already deleted on the server (404 to the `DELETE`) is done, not a change
/// that "wasn't kept"; one the server refuses (403) is said.
#[test]
fn a_calendar_already_deleted_on_the_server_is_done() {
    let server = serve(account());
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let st = crate::jmap::State::default();
    let gone = Op::DeleteCalendar { calendar: "NOPE".into() };
    assert!(crate::graph::apply_remote(&c, &st, &gone).is_ok(), "deleted already: done");
    server.data.lock().unwrap().forbid.push("DELETE /me/calendars".into());
    let refused = crate::graph::apply_remote(&c, &st, &Op::DeleteCalendar { calendar: "CAL2".into() });
    assert!(refused.is_err(), "refused: said");
    assert_eq!(server.data.lock().unwrap().calendars.len(), 2);
}

/// A reminder answer Graph refuses (403) isn't taken for given: it is said and kept; one for an
/// event that is gone (404) is let go.
#[test]
fn a_reminder_answer_graph_refuses_is_kept() {
    let mut d = account();
    d.events.push(json!({"id": "EV-one", "calendar": "CAL1", "subject": "Dentist"}));
    d.forbid.push("POST /me/events/EV-one/".into());
    let server = serve(d);
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let answer = |href: &str| ReminderAnswer {
        calendar: "CAL1".into(),
        href: href.into(),
        uid: href.into(),
        occurrence: 1_793_782_800,
        series: false,
        snooze_until: None,
    };
    let kept = Pim { reminder_answers: vec![answer("EV-one")], ..Default::default() };
    let sync = crate::graph_pim::sync(&c, &kept, 1_793_782_000).unwrap();
    assert!(sync.answered.is_empty(), "kept: {:?}", sync.answered);
    assert!(sync.errors.iter().any(|e| e.starts_with("a reminder")), "{:?}", sync.errors);
    let gone = Pim { reminder_answers: vec![answer("EV-gone")], ..Default::default() };
    let sync = crate::graph_pim::sync(&c, &gone, 1_793_782_000).unwrap();
    assert_eq!(sync.answered, gone.reminder_answers, "gone: let go");
}
