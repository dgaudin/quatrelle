// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin

use super::*;

#[test]
fn recurrences_both_ways() {
    let weekly = json!({"pattern": {"type": "weekly", "interval": 2, "daysOfWeek": ["monday", "wednesday"]}, "range": {"type": "endDate", "startDate": "2026-10-01", "endDate": "2026-12-31"}});
    assert_eq!(rrule_of(&weekly).as_deref(), Some("FREQ=WEEKLY;BYDAY=MO,WE;INTERVAL=2;UNTIL=20261231T235959Z"));
    let monthly = json!({"pattern": {"type": "relativeMonthly", "interval": 1, "daysOfWeek": ["friday"], "index": "last"}, "range": {"type": "numbered", "numberOfOccurrences": 6}});
    assert_eq!(rrule_of(&monthly).as_deref(), Some("FREQ=MONTHLY;BYDAY=-1FR;COUNT=6"));
    let yearly = json!({"pattern": {"type": "absoluteYearly", "month": 3, "dayOfMonth": 14}, "range": {"type": "noEnd"}});
    assert_eq!(rrule_of(&yearly).as_deref(), Some("FREQ=YEARLY;BYMONTH=3;BYMONTHDAY=14"));
    assert_eq!(rrule_of(&json!({"pattern": {"type": "hourly"}})), None);
    // And back.
    let start = quatrelle_jmap::utc_date("2026-10-05T09:00:00Z").unwrap();
    let r = recurrence_of("FREQ=WEEKLY;BYDAY=MO,WE;INTERVAL=2;UNTIL=20261231T235959Z", start).unwrap();
    assert_eq!(r["pattern"]["daysOfWeek"], json!(["monday", "wednesday"]));
    assert_eq!((r["pattern"]["interval"].as_u64(), r["range"]["endDate"].as_str()), (Some(2), Some("2026-12-31")));
    let r = recurrence_of("FREQ=MONTHLY;BYDAY=-1FR;COUNT=6", start).unwrap();
    assert_eq!(
        (r["pattern"]["type"].as_str(), r["pattern"]["index"].as_str(), r["range"]["numberOfOccurrences"].as_u64()),
        (Some("relativeMonthly"), Some("last"), Some(6))
    );
    let r = recurrence_of("FREQ=WEEKLY", start).unwrap();
    assert_eq!(r["pattern"]["daysOfWeek"], json!(["monday"]), "the start's own day");
    assert!(recurrence_of("FREQ=SECONDLY", start).is_none() && recurrence_of("junk", start).is_none());
}

/// An `UNTIL` that isn't a date (a calendar file's own text) ends nothing, and stops nothing.
#[test]
fn an_until_that_is_no_date_is_left_out() {
    let start = quatrelle_jmap::utc_date("2026-10-05T09:00:00Z").unwrap();
    for until in ["€€€", "2026123é", "2026é231", "ééééé", "2026-12-31", "<b>12345"] {
        let r = recurrence_of(&format!("FREQ=DAILY;UNTIL={until}"), start).unwrap();
        assert_eq!(r["range"]["type"], json!("noEnd"), "{until}");
    }
    for until in ["20261231", "20261231T235959Z", "20261231é"] {
        let r = recurrence_of(&format!("FREQ=DAILY;UNTIL={until}"), start).unwrap();
        assert_eq!(r["range"]["endDate"], json!("2026-12-31"), "{until}");
    }
}

fn v_start() -> Value {
    json!({"dateTime": "2026-10-16T14:00:00.0000000", "timeZone": "UTC"})
}

#[test]
fn events_and_contacts_both_ways() {
    let v = json!({
        "id": "E1", "iCalUId": "uid-1", "subject": "Mix review", "changeKey": "ck1",
        "body": {"contentType": "html", "content": "<p>Bring the <b>stems</b></p>"},
        "start": {"dateTime": "2026-10-16T14:00:00.0000000", "timeZone": "UTC"},
        "end": {"dateTime": "2026-10-16T15:00:00.0000000", "timeZone": "UTC"},
        "location": {"displayName": "Lab"}, "categories": ["Clients", " ", "Mix"],
        "organizer": {"emailAddress": {"name": "Ana", "address": "ana@contoso.example"}},
        "attendees": [{"type": "required", "status": {"response": "tentativelyAccepted"}, "emailAddress": {"name": "Me", "address": "me@contoso.example"}}],
    });
    let e = event_of(&v).unwrap();
    assert_eq!((e.uid.as_str(), e.summary.as_str(), e.location.as_str()), ("uid-1", "Mix review", "Lab"));
    assert_eq!(e.end - e.start, 3600);
    assert!(e.description.contains("stems"));
    assert_eq!(e.attendees[0].partstat, "TENTATIVE");
    assert_eq!(e.organizer.as_ref().unwrap().email, "ana@contoso.example");
    assert_eq!(e.categories, ["Clients", "Mix"]);
    let back = event_json(&e);
    assert_eq!(back["start"]["dateTime"], "2026-10-16T14:00:00");
    assert_eq!(back["categories"], json!(["Clients", "Mix"]));
    assert_eq!(back["subject"], "Mix review");
    assert!(back.get("singleValueExtendedProperties").is_none(), "no id, nothing written");
    let with_id = CalEvent { webex_id: Some("abc_123".into()), ..e.clone() };
    let mut sent = event_json(&with_id);
    assert_eq!(
        sent["singleValueExtendedProperties"],
        json!([{"id": "String {5b7a1c3e-8f44-4f0e-9c1d-3e2a7d6b4c11} Name WebexId", "value": "abc_123"}])
    );
    sent["id"] = json!("E1");
    assert_eq!(event_of(&sent).and_then(|x| x.webex_id).as_deref(), Some("abc_123"), "read back");
    // The name's case is Graph's to change; an id that isn't one is none.
    let theirs = |id: &str, v: &str| json!({"id": "E2", "start": v_start(), "singleValueExtendedProperties": [{"id": id, "value": v}]});
    let upper = "string {5B7A1C3E-8F44-4F0E-9C1D-3E2A7D6B4C11} name webexid";
    assert_eq!(event_of(&theirs(upper, "z9")).and_then(|x| x.webex_id).as_deref(), Some("z9"));
    assert_eq!(event_of(&theirs(upper, "a b")).and_then(|x| x.webex_id), None);
    assert_eq!(event_of(&theirs("String {other} Name WebexId", "z9")).and_then(|x| x.webex_id), None);
    let all_day = CalEvent { all_day: true, start: e.start - 14 * 3600, end: e.start - 14 * 3600 + 86_400, ..e.clone() };
    assert_eq!(event_json(&all_day)["end"]["dateTime"], "2026-10-17T00:00:00");
    assert!(event_of(&json!({"id": "x"})).is_none(), "no start, no event");

    let c = json!({"id": "C1", "displayName": "Ana Lima", "givenName": "Ana", "surname": "Lima",
        "emailAddresses": [{"name": "Ana", "address": "ana@contoso.example"}], "businessPhones": ["+1 555 0100"],
        "mobilePhone": "+1 555 0101", "companyName": "Sound Lab", "jobTitle": "Mixer"});
    let card = card_of(&c).unwrap();
    assert_eq!(
        (card.display(), card.emails[0].value.as_str(), card.phones.len(), card.org.as_str()),
        ("Ana Lima".to_string(), "ana@contoso.example", 2, "Sound Lab")
    );
    let back = card_json(&card);
    assert_eq!((back["mobilePhone"].as_str(), back["businessPhones"][0].as_str()), (Some("+1 555 0101"), Some("+1 555 0100")));
}

#[test]
fn exchange_gets_no_conference() {
    let plain = CalEvent {
        uid: "u".into(),
        summary: "Call".into(),
        location: "Lab".into(),
        description: "Notes".into(),
        start: 1_792_000_000,
        end: 1_792_003_600,
        ..Default::default()
    };
    let with = CalEvent { conference: quatrelle_model::ical::Conference::new("https://acme.webex.com/meet/ana", "Webex"), ..plain.clone() };
    assert!(with.conference.is_some());
    assert_eq!(event_json(&plain), event_json(&with));
}
