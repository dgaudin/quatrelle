//! Engine tests: the calendar's search (`calendar.search`): every calendar shown, a series found
//! once, the order, the scopes, and how fast it goes through a big calendar.

use super::*;

fn search(s: &mut Session, p: Value) -> Vec<Value> {
    s.run("calendar.search", &p).unwrap().as_array().cloned().unwrap_or_default()
}

fn titles(list: &[Value]) -> Vec<String> {
    list.iter().filter_map(|e| e["summary"].as_str().map(str::to_string)).collect()
}

/// The sample mailbox, read in a zone `offset` seconds east of UTC.
fn zoned_session(name: &str, offset: i32) -> Session {
    let store = temp(name);
    assert!(sample::seed_if_empty(&store, NOW).unwrap());
    Session::open(Some(store), quatrelle_model::dates::Clock::new(NOW, offset))
}

#[test]
fn the_calendar_search_finds_events_in_every_calendar() {
    let mut s = sample_session("calendar-search");
    // Words from a subject, a place, a person; in both calendars (the personal and the studio's).
    assert_eq!(titles(&search(&mut s, json!({"query": "storyboard"}))), ["Storyboard review"]);
    assert_eq!(titles(&search(&mut s, json!({"query": "location:suite"}))), ["Edit session: act two"], "the studio calendar too");
    assert_eq!(titles(&search(&mut s, json!({"query": "attendee:sam"}))), ["Storyboard review"]);
    assert_eq!(titles(&search(&mut s, json!({"query": "organizer:ana"}))), ["Sound mix playback"]);
    assert_eq!(titles(&search(&mut s, json!({"query": "from:zoe@studio.example"}))), ["Storyboard review"]);
    assert_eq!(titles(&search(&mut s, json!({"query": "notes:scenes"}))), ["Storyboard review"]);
    let work = titles(&search(&mut s, json!({"query": "category:work"})));
    assert_eq!(work.len(), 2, "{work:?}");
    assert!(search(&mut s, json!({"query": "nothing-like-this"})).is_empty());
    // A recurring series is found once, at its next occurrence.
    let standup = search(&mut s, json!({"query": "stand-up"}));
    assert_eq!(standup.len(), 1, "{standup:?}");
    let next = standup[0]["start"].as_i64().unwrap();
    assert!(next > NOW && next - NOW < 3 * 86_400, "the next one: {next}");
    assert_eq!((standup[0]["recurring"].as_bool(), standup[0]["upcoming"].as_bool()), (Some(true), Some(true)));
    // Its occurrence opens it (named as the calendar names it).
    let week = s.run("calendar.events", &json!({"from": "2026-10-12", "to": "2026-10-25"})).unwrap();
    assert!(week.as_array().unwrap().iter().any(|e| e["uid"] == standup[0]["uid"] && e["occurrence"] == standup[0]["occurrence"]));
    // Past: the last one before now; with dates, the ones then.
    let past = search(&mut s, json!({"query": "stand-up", "when": "past"}));
    let last = past[0]["start"].as_i64().unwrap();
    assert!(last < NOW && NOW - last < 3 * 86_400 && !past[0]["upcoming"].as_bool().unwrap());
    let first = search(&mut s, json!({"query": "stand-up before:2026-10-01"}));
    assert_eq!(first.len(), 1);
    assert!(first[0]["start"].as_i64().unwrap() < crate::parse_local("2026-10-01", &s.clock).unwrap().0);
    let later = search(&mut s, json!({"query": "stand-up after:2026-12-01"}));
    assert!(later[0]["start"].as_i64().unwrap() >= crate::parse_local("2026-12-01", &s.clock).unwrap().0);
    assert!(search(&mut s, json!({"query": "stand-up after:2026-12-01", "when": "past"})).is_empty(), "nothing past after December");
    // Upcoming first (soonest first), then past (latest first).
    let all = search(&mut s, json!({"query": ""}));
    let upcoming: Vec<bool> = all.iter().map(|e| e["upcoming"].as_bool().unwrap()).collect();
    assert!(upcoming.windows(2).all(|w| w[0] || !w[1]), "{upcoming:?}");
    let starts: Vec<i64> = all.iter().filter(|e| e["upcoming"] == true).map(|e| e["start"].as_i64().unwrap()).collect();
    assert!(starts.windows(2).all(|w| w[0] <= w[1]));
    assert!(all.iter().all(|e| e["upcoming"] == true || e["end"].as_i64().unwrap() <= NOW));
    // Scope: one calendar (by name), or what's shown.
    assert_eq!(titles(&search(&mut s, json!({"query": "", "calendar": "Studio"}))).len(), 2);
    s.run("calendar.show", &json!({"account": "sample", "calendar": crate::sample::STUDIO_CALENDAR, "shown": false})).unwrap();
    assert!(search(&mut s, json!({"query": "location:suite"})).is_empty(), "a hidden calendar isn't searched");
    assert_eq!(titles(&search(&mut s, json!({"query": "suite", "calendar": crate::sample::STUDIO_CALENDAR}))), ["Edit session: act two"]);
    // Private ones: the lunch made so, and a new one.
    let lunch = s.pim["sample"].events.iter().find(|e| e.event.summary == "Lunch with Jun").unwrap().event.uid.clone();
    s.run("event.update", &json!({"uid": lunch, "private": true})).unwrap();
    s.run("event.create", &json!({"summary": "Dentist", "start": "2026-10-21T08:00", "private": true, "showAs": "outOfOffice"})).unwrap();
    let private = search(&mut s, json!({"query": "is:private"}));
    assert_eq!(titles(&private), ["Lunch with Jun", "Dentist"], "the lunch, then the new one");
    assert!(private.iter().all(|e| e["private"] == true));
    assert_eq!((private[0]["showAs"].as_str(), private[1]["showAs"].as_str()), (Some("busy"), Some("outOfOffice")));
    assert_eq!(titles(&search(&mut s, json!({"query": "-is:private lunch"}))), Vec::<String>::new());
    // Bad params are refused; a limit holds.
    assert!(matches!(s.run("calendar.search", &json!({"query": "x", "when": "soon"})), Err(CmdError::Params(_))));
    assert_eq!(search(&mut s, json!({"query": "", "limit": 2})).len(), 2);
    assert!(!search(&mut s, json!({"query": "", "limit": 0})).is_empty(), "at least one");
    done(s);
}

/// An all-day appointment is found on its own day, whatever the zone: from that day's local
/// midnight, as the calendar draws it.
#[test]
fn an_all_day_appointment_is_found_on_its_own_day() {
    for (name, offset) in [("calendar-search-east", 10 * 3600), ("calendar-search-west", -8 * 3600)] {
        let mut s = zoned_session(name, offset);
        s.run("event.create", &json!({"summary": "Festival", "start": "2026-10-22", "allDay": true})).unwrap();
        let day = quatrelle_model::dates::days_from_civil(2026, 10, 22);
        for query in ["festival", "festival after:2026-10-22 before:2026-10-23"] {
            let hits = search(&mut s, json!({"query": query}));
            assert_eq!(hits.len(), 1, "{name} {query}: {hits:?}");
            assert_eq!(hits[0]["start"].as_i64(), Some(s.local_midnight(day)), "{name} {query}");
            assert_eq!(hits[0]["allDay"], true);
        }
        assert!(search(&mut s, json!({"query": "festival after:2026-10-23"})).is_empty(), "{name}: not on the next day");
        assert!(search(&mut s, json!({"query": "festival before:2026-10-22"})).is_empty(), "{name}: not on the day before");
        done(s);
    }
}

/// One occurrence of a weekly series moved and renamed on its own is found at its new place,
/// under the name its rule gave it (which opens it), and the series is not found for it.
#[test]
fn an_occurrence_moved_on_its_own_is_found_at_its_place() {
    let mut s = sample_session("calendar-search-moved");
    let day = crate::parse_local("2026-10-19", &s.clock).unwrap().0;
    let made = s
        .run("event.create", &json!({"summary": "Weekly review", "start": "2026-10-19T09:00", "end": "2026-10-19T10:00", "repeat": "weekly"}))
        .unwrap();
    let uid = made["uid"].as_str().unwrap().to_string();
    let named = day + 9 * 3600 + 7 * 86_400;
    let new_start = named + 5 * 3600;
    s.run("event.move", &json!({"uid": uid, "occurrence": named, "start": new_start})).unwrap();
    for e in s.pim_mut("sample").events.iter_mut().filter(|e| e.event.uid == uid && e.event.recurrence_id == Some(named)) {
        e.event.summary = "Moved stand-up".into();
    }
    let hits = search(&mut s, json!({"query": "moved"}));
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["occurrence"].as_i64(), Some(named));
    assert_eq!(hits[0]["start"].as_i64(), Some(new_start));
    assert_eq!(hits[0]["summary"], "Moved stand-up");
    // The series itself is still found once, by its own name.
    assert_eq!(titles(&search(&mut s, json!({"query": "weekly"}))), ["Weekly review"]);
    done(s);
}

/// A big calendar (a second account's, 25,000 appointments, some of them daily series) is searched
/// in well under a second, by words and by a field.
#[test]
fn searching_25000_appointments_stays_quick() {
    use quatrelle_model::ical::CalEvent;
    use quatrelle_model::pim::{LOCAL, StoredEvent};
    let mut s = sample_session("calendar-search-big");
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let p = s.pim_mut(&id);
    p.events.extend((0..25_000_i64).map(|i| {
        let start = NOW - 200 * 86_400 + i * 1_500;
        let event = CalEvent {
            uid: format!("big-{i}"),
            summary: format!("Appointment {i}"),
            location: format!("Room {}", i % 50),
            start,
            end: start + 900,
            rrule: (i % 125 == 0).then(|| "FREQ=DAILY".to_string()),
            ..Default::default()
        };
        StoredEvent { calendar: LOCAL.into(), event, ..Default::default() }
    }));
    for (query, wanted) in [("a", 500), ("location:room", 500)] {
        let started = std::time::Instant::now();
        let hits = s.find_events(query, crate::EventWhen::All, None, None, 500);
        let took = started.elapsed();
        assert_eq!(hits.len(), wanted, "{query}");
        assert!(took < std::time::Duration::from_secs(1), "{query}: {took:?}");
        eprintln!("calendar search `{query}` over 25,000 appointments: {took:?}");
    }
    done(s);
}
