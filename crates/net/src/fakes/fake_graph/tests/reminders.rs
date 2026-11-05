// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Reminders answered here, told to the fake Graph (issue #40).

use quatrelle_model::pim::{Pim, ReminderAnswer};
use serde_json::json;

use super::*;

/// A single appointment snoozed here is snoozed on the event until its time; an occurrence of a
/// series dismissed here is dismissed on that occurrence (found among the series' instances,
/// nearest its start); one whose event is gone is let go. Each is said answered.
#[test]
fn reminders_answered_here_are_answered_on_graph() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"));
    d.calendars.push(("CAL1".into(), "Calendar".into()));
    d.events.push(json!({"id": "EV-one", "calendar": "CAL1", "subject": "Dentist"}));
    d.events.push(json!({"id": "EV-series", "calendar": "CAL1", "subject": "Stand-up"}));
    let at = |t: &str| json!({"dateTime": t, "timeZone": "UTC"});
    d.instances.push((
        "EV-series".into(),
        vec![
            json!({"id": "EV-series-3", "start": at("2026-11-03T09:00:00.0000000")}),
            json!({"id": "EV-series-4", "start": at("2026-11-04T09:00:00.0000000")}),
        ],
    ));
    let server = serve(d);
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let nov4 = 1_793_782_800; // 2026-11-04 09:00 UTC
    let answer = |uid: &str, href: &str, series: bool, snooze_until: Option<i64>| ReminderAnswer {
        calendar: "CAL1".into(),
        href: href.into(),
        uid: uid.into(),
        occurrence: nov4,
        series,
        snooze_until,
    };
    let pim = Pim {
        reminder_answers: vec![
            answer("one", "EV-one", false, Some(nov4 - 300)),
            answer("series", "EV-series", true, None),
            answer("gone", "EV-gone", false, None),
        ],
        ..Default::default()
    };
    let sync = crate::graph_pim::sync(&c, &pim, nov4 - 900).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    assert_eq!(sync.answered, pim.reminder_answers);
    let d = server.data.lock().unwrap();
    let done: Vec<(&str, &str)> = d.reminders.iter().map(|(a, id, _)| (a.as_str(), id.as_str())).collect();
    assert_eq!(done, [("snoozeReminder", "EV-one"), ("dismissReminder", "EV-series-4")]);
    assert_eq!(d.reminders[0].2["newReminderTime"], json!({"dateTime": "2026-11-04T08:55:00", "timeZone": "UTC"}));
}
