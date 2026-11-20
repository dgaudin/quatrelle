// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What Exchange is asked of a Teams meeting's id, which EWS keeps in a property of Quatrelle's
//! own: the property is taken off only an appointment that had a meeting, once its meeting is taken
//! out, never at every change (F2).

use quatrelle_model::pim::DavSync;

use super::follow::{LOCAL_CALENDAR, calendars};
use super::*;

/// Whether the change waiting for `uid` takes the Teams meeting's property off.
fn taken_off(s: &Session, uid: &str) -> bool {
    s.find_event(uid).is_some_and(|(_, e)| e.teams_meeting_taken_off)
}

/// The Exchange calendar's id.
const EXCHANGE: &str = "AAMkEXCHANGE";

/// An appointment on the Exchange calendar as a sync leaves it (its item there, nothing waiting),
/// with the Teams meeting `id` if any; its uid.
fn on_exchange(s: &mut Session, id: Option<&str>) -> String {
    let mut p = json!({"summary": "Budget", "start": "2026-10-21T15:00", "end": "2026-10-21T16:00", "account": "sample", "calendar": EXCHANGE});
    if let Some(id) = id {
        p["teamsMeetingId"] = json!(id);
    }
    let uid = s.run("event.create", &p).unwrap()["uid"].as_str().unwrap().to_string();
    let stored = s.pim_mut("sample").events.iter_mut().find(|e| e.event.uid == uid).unwrap();
    assert_eq!((stored.calendar.as_str(), stored.event.teams_meeting_id.as_deref()), (EXCHANGE, id));
    (stored.href, stored.dirty) = (format!("item-{uid}"), false);
    uid
}

#[test]
fn the_property_is_taken_off_only_where_a_meeting_was() {
    let mut s = signed_in("teams-taken-off");
    calendars(&mut s, "ews", &[(EXCHANGE, false)]);
    let plain = on_exchange(&mut s, None);
    s.run("event.update", &json!({"uid": plain, "summary": "Budget 2"})).unwrap();
    assert!(!taken_off(&s, &plain), "an appointment that never had a meeting");
    let with = on_exchange(&mut s, Some("m1"));
    s.run("event.update", &json!({"uid": with, "teamsMeetingId": ""})).unwrap();
    assert!(taken_off(&s, &with), "its meeting taken out");
    s.run("event.update", &json!({"uid": with, "summary": "Budget 3"})).unwrap();
    assert!(taken_off(&s, &with), "until the server has the change");
    s.run("event.update", &json!({"uid": with, "teamsMeetingId": "m2"})).unwrap();
    assert!(!taken_off(&s, &with), "another meeting put in: set, not taken off");
    s.run("event.update", &json!({"uid": with, "teamsMeetingId": ""})).unwrap();
    assert!(taken_off(&s, &with));
    // The change on the server: nothing more to take off.
    let pushed = s.find_event(&with).map(|(_, e)| e.clone()).unwrap();
    s.apply_dav("sample", &DavSync { pushed_events: vec![pushed], ..Default::default() });
    assert!(!taken_off(&s, &with) && s.find_event(&with).is_some_and(|(_, e)| !e.dirty));
    done(s);
}

#[test]
fn nothing_is_taken_off_on_this_computers_calendar() {
    let mut s = signed_in("teams-taken-off-local");
    let p = json!({"summary": "Budget", "start": "2026-10-21T15:00", "calendar": LOCAL_CALENDAR, "teamsMeetingId": "m1"});
    let uid = s.run("event.create", &p).unwrap()["uid"].as_str().unwrap().to_string();
    s.run("event.update", &json!({"uid": uid, "teamsMeetingId": ""})).unwrap();
    assert!(!taken_off(&s, &uid), "no server to tell");
    done(s);
}
