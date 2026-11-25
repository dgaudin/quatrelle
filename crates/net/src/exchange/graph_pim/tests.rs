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
    // The nth of several weekdays, and the 31st or the month's last, as a data file's reader
    // writes them, are said to Exchange as they are (the last weekday went as the first, the
    // 31st as the day the series began), and read from it the same way.
    let r = recurrence_of("FREQ=MONTHLY;INTERVAL=1;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1", start).unwrap();
    assert_eq!((r["pattern"]["type"].as_str(), r["pattern"]["index"].as_str()), (Some("relativeMonthly"), Some("last")));
    assert_eq!(r["pattern"]["daysOfWeek"].as_array().map(Vec::len), Some(5));
    assert_eq!(rrule_of(&r).unwrap(), "FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1");
    let r = recurrence_of("FREQ=MONTHLY;BYMONTHDAY=28,29,30,31;BYSETPOS=-1", start).unwrap();
    assert_eq!((r["pattern"]["type"].as_str(), r["pattern"]["dayOfMonth"].as_u64()), (Some("absoluteMonthly"), Some(31)));
    assert_eq!(rrule_of(&r).unwrap(), "FREQ=MONTHLY;BYMONTHDAY=28,29,30,31;BYSETPOS=-1");
    // A list of several days other than that form keeps the start's day, as before; one day
    // alone is taken.
    let r = recurrence_of("FREQ=MONTHLY;BYMONTHDAY=1,15", start).unwrap();
    assert_eq!((r["pattern"]["type"].as_str(), r["pattern"]["dayOfMonth"].as_u64()), (Some("absoluteMonthly"), Some(5)));
    let r = recurrence_of("FREQ=MONTHLY;BYMONTHDAY=15", start).unwrap();
    assert_eq!(r["pattern"]["dayOfMonth"].as_u64(), Some(15));
    // The day weeks start on goes both ways; Monday needn't be said.
    let r = recurrence_of("FREQ=WEEKLY;INTERVAL=2;BYDAY=SA,SU;WKST=SU", start).unwrap();
    assert_eq!(r["pattern"]["firstDayOfWeek"], "sunday");
    assert_eq!(rrule_of(&r).unwrap(), "FREQ=WEEKLY;BYDAY=SA,SU;WKST=SU;INTERVAL=2");
    let r = recurrence_of("FREQ=WEEKLY;INTERVAL=2;BYDAY=MO", start).unwrap();
    assert_eq!(r["pattern"]["firstDayOfWeek"], "monday");
    assert_eq!(rrule_of(&r).unwrap(), "FREQ=WEEKLY;BYDAY=MO;INTERVAL=2");
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

    let c = json!({"id": "C1", "displayName": "Ana Exemple", "givenName": "Ana", "surname": "Lima",
        "emailAddresses": [{"name": "Ana", "address": "ana@contoso.example"}], "businessPhones": ["+1 555 0100"],
        "mobilePhone": "+1 555 0101", "companyName": "Sound Lab", "jobTitle": "Mixer"});
    let card = card_of(&c).unwrap();
    assert_eq!(
        (card.display(), card.emails[0].value.as_str(), card.phones.len(), card.org.as_str()),
        ("Ana Exemple".to_string(), "ana@contoso.example", 2, "Sound Lab")
    );
    let back = card_json(&card);
    assert_eq!((back["mobilePhone"].as_str(), back["businessPhones"][0].as_str()), (Some("+1 555 0101"), Some("+1 555 0100")));
    // The whole name: Graph's middle name, title (the prefix) and generation (the suffix).
    let named = card_of(&json!({"id": "C2", "displayName": "Dr. Zoe Marie Park Jr.", "givenName": "Zoe", "middleName": "Marie", "surname": "Park",
        "title": "Dr.", "generation": "Jr."}))
    .unwrap();
    assert_eq!(named.name_rest, "Marie;Dr.;Jr.");
    let sent = card_json(&named);
    assert_eq!((sent["middleName"].as_str(), sent["title"].as_str(), sent["generation"].as_str()), (Some("Marie"), Some("Dr."), Some("Jr.")));
    assert_eq!((sent["givenName"].as_str(), sent["surname"].as_str()), (Some("Zoe"), Some("Park")));
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

fn teams_graph_event(extra: Value) -> Value {
    let mut v = json!({"id": "T1", "subject": "Sync", "start": v_start(), "end": v_start()});
    for (k, x) in extra.as_object().cloned().unwrap_or_default() {
        v[k.as_str()] = x;
    }
    v
}

#[test]
fn a_teams_event_is_asked_for_and_read() {
    assert!(EVENT_FIELDS.contains("isOnlineMeeting") && EVENT_FIELDS.contains("onlineMeetingProvider"));
    assert!(events_query().contains("isOnlineMeeting,onlineMeetingProvider"));
    let link = "https://teams.microsoft.com/l/meetup-join/19%3ameeting_x%40thread.v2/0";
    let e = event_of(&teams_graph_event(json!({
        "isOnlineMeeting": true, "onlineMeetingProvider": "teamsForBusiness", "onlineMeeting": {"joinUrl": link}
    })))
    .unwrap();
    assert!(e.teams_online);
    assert_eq!(e.join_url, link);
}

#[test]
fn another_providers_meeting_is_not_teams() {
    let link = "https://join.skype.com/abc";
    let e = event_of(&teams_graph_event(json!({
        "isOnlineMeeting": true, "onlineMeetingProvider": "skypeForBusiness", "onlineMeeting": {"joinUrl": link}
    })))
    .unwrap();
    assert!(!e.teams_online);
    assert_eq!(e.join_url, link);
    for v in [json!({}), json!({"isOnlineMeeting": false, "onlineMeetingProvider": "teamsForBusiness"}), json!({"isOnlineMeeting": true})] {
        assert!(!event_of(&teams_graph_event(v)).unwrap().teams_online);
    }
}

#[test]
fn a_join_link_that_cannot_be_opened_is_dropped() {
    let long = format!("https://teams.microsoft.com/l/meetup-join/{}", "a".repeat(8 * 1024));
    for bad in ["javascript:alert(1)", "https://x.example/a b", long.as_str(), "", "file:///etc/passwd"] {
        let e = event_of(&teams_graph_event(json!({"onlineMeeting": {"joinUrl": bad}}))).unwrap();
        assert_eq!(e.join_url, "", "{bad:.40}");
    }
    assert_eq!(event_of(&teams_graph_event(json!({"onlineMeeting": {"joinUrl": 7}}))).unwrap().join_url, "");
    assert_eq!(event_of(&teams_graph_event(json!({"onlineMeeting": "x"}))).unwrap().join_url, "");
}

#[test]
fn an_event_marked_is_written_as_a_teams_meeting() {
    let e = CalEvent { teams_online: true, start: 1_792_000_000, end: 1_792_003_600, ..Default::default() };
    let v = event_json(&e);
    assert_eq!(v["isOnlineMeeting"], json!(true));
    assert_eq!(v["onlineMeetingProvider"], json!("teamsForBusiness"));
}

#[test]
fn an_event_not_marked_says_nothing_of_it() {
    let e = CalEvent {
        teams_online: false,
        join_url: "https://teams.microsoft.com/l/meetup-join/x".into(),
        start: 1_792_000_000,
        end: 1_792_003_600,
        ..Default::default()
    };
    let v = event_json(&e);
    assert!(v.get("isOnlineMeeting").is_none() && v.get("onlineMeetingProvider").is_none() && v.get("onlineMeeting").is_none());
    assert!(!v.to_string().contains("isOnlineMeeting\":false"));
}

#[test]
fn join_takes_the_servers_link_first() {
    let server = "https://teams.microsoft.com/l/meetup-join/server";
    let e = event_of(&teams_graph_event(json!({
        "body": {"contentType": "text", "content": "Also https://zoom.us/j/123456789"},
        "onlineMeeting": {"joinUrl": server}
    })))
    .unwrap();
    assert_eq!(e.join_link().as_deref(), Some(server));
}
