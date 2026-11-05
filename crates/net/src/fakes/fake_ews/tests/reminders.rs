// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Reminders answered here, told to the fake Exchange (issue #40): found among the server's (an
//! occurrence's by its series and start) and dismissed or snoozed there; an older Exchange,
//! which doesn't know those operations, isn't asked; one answered offline waits.

use quatrelle_model::pim::{Pim, ReminderAnswer};

use super::*;

/// 2026-11-04 09:00 UTC.
const NOV4: i64 = 1_793_782_800;

fn answer(uid: &str, href: &str, series: bool, snooze_until: Option<i64>) -> ReminderAnswer {
    ReminderAnswer { calendar: "calendar".into(), href: href.into(), uid: uid.into(), occurrence: NOV4, series, snooze_until }
}

/// A mailbox whose server lists a single appointment's reminder and two of a daily series'.
fn mailbox() -> Data {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    d.reminders.listed = vec![
        ("E-one".into(), String::new(), "2026-11-04T09:00:00Z".into()),
        ("E-series-3".into(), "E-series".into(), "2026-11-03T09:00:00Z".into()),
        ("E-series-4".into(), "E-series".into(), "2026-11-04T09:00:00Z".into()),
    ];
    d
}

fn connect(url: &str) -> crate::ews::Client {
    crate::ews::Client::connect(url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap()
}

/// Exchange 2013 and later: a snooze of a single appointment, and an occurrence of a series
/// dismissed or snoozed, are done on the server's own reminder (the occurrence's, by its series
/// and start); one the server no longer lists is let go. Each is said answered.
#[test]
fn reminders_answered_here_are_answered_on_exchange() {
    let server = serve(mailbox());
    let c = connect(&server.url);
    let pim = Pim {
        reminder_answers: vec![
            answer("one", "E-one", false, Some(NOV4 - 300)),
            answer("series", "E-series", true, None),
            answer("gone", "E-gone", false, Some(NOV4 - 300)),
        ],
        ..Default::default()
    };
    let sync = crate::ews_pim::sync(&c, &pim, NOV4 - 900).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    assert_eq!(sync.answered, pim.reminder_answers, "each one answered, or let go");
    let d = server.data.lock().unwrap();
    assert_eq!(
        d.reminders.done,
        [("Snooze".to_string(), "E-one".to_string(), "2026-11-04T08:55:00Z".to_string()), ("Dismiss".into(), "E-series-4".into(), String::new())]
    );
}

/// Exchange 2010 doesn't know the operations: it isn't asked, and nothing waits for it (the
/// reminder stays answered on this computer).
#[test]
fn an_exchange_older_than_2013_is_not_asked() {
    let mut d = mailbox();
    d.reminders.before_2013 = true;
    let server = serve(d);
    let c = connect(&server.url);
    let pim =
        Pim { reminder_answers: vec![answer("series", "E-series", true, None), answer("one", "E-one", false, Some(NOV4))], ..Default::default() };
    let sync = crate::ews_pim::sync(&c, &pim, NOV4).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    assert_eq!(sync.answered.len(), 2, "neither waits");
    let d = server.data.lock().unwrap();
    assert!(!d.calls.iter().any(|c| c == "GetReminders" || c == "PerformReminderAction"), "{:?}", d.calls);
    assert!(d.reminders.done.is_empty());
}

/// Out of reach when the reminders are asked: the answer waits for the next sync, which takes it.
#[test]
fn an_answer_given_offline_waits_for_the_next_sync() {
    let mut d = mailbox();
    d.reminders.unreachable = true;
    let server = serve(d);
    let c = connect(&server.url);
    let pim = Pim { reminder_answers: vec![answer("series", "E-series", true, Some(NOV4 - 60))], ..Default::default() };
    let sync = crate::ews_pim::sync(&c, &pim, NOV4 - 900).unwrap();
    assert!(sync.answered.is_empty(), "it waits");
    assert!(!sync.errors.is_empty(), "and says why");
    server.data.lock().unwrap().reminders.unreachable = false;
    let sync = crate::ews_pim::sync(&c, &pim, NOV4 - 900).unwrap();
    assert_eq!(sync.answered, pim.reminder_answers);
    assert_eq!(server.data.lock().unwrap().reminders.done, [("Snooze".to_string(), "E-series-4".to_string(), "2026-11-04T08:59:00Z".to_string())]);
}

/// The server's list is read as hostile: broken, huge or odd, nothing panics and the answer is let
/// go (nothing on the server matches it).
#[test]
fn a_hostile_reminders_list_is_survived() {
    let many =
        "<t:Reminder><t:StartDate>9999-99-99T99:99:99Z</t:StartDate><t:ItemId/><t:RecurringMasterItemId Id=\"E-series\"/></t:Reminder>".repeat(5000);
    for raw in [
        String::new(),
        "<m:Reminders><t:Reminder>".to_string(),
        format!("<m:Reminders>{many}</m:Reminders>"),
        "<m:Reminders><t:Reminder><t:StartDate>-999999999-01-01T00:00:00Z</t:StartDate><t:ItemId Id=\"E-one\"/></t:Reminder></m:Reminders>".into(),
    ] {
        let mut d = mailbox();
        d.reminders.raw = Some(raw);
        let server = serve(d);
        let c = connect(&server.url);
        let pim =
            Pim { reminder_answers: vec![answer("series", "E-series", true, None), answer("one", "E-one", false, Some(NOV4))], ..Default::default() };
        let sync = crate::ews_pim::sync(&c, &pim, NOV4).unwrap();
        assert_eq!(sync.answered.len(), 2, "{:?}", sync.errors);
        assert!(server.data.lock().unwrap().reminders.done.is_empty());
    }
}
