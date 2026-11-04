// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The Webex rooms kept in the settings (`calendar.webexRoom`, the single room of 1.0 carried
//! over, a hand-edited file cut), and where to join an appointment online (`conference` in
//! `event.create` and `event.update`, an invitation by mail, a Webex link changed, a cached event's
//! kept line).

use std::collections::BTreeSet;

use quatrelle_model::ical::{Conference, is_webex_link};

use super::*;
use crate::{Prefs, WebexRoom};

const ANA: &str = "https://acme.webex.com/meet/ana";
const TEAM: &str = "https://acme.webex.com/meet/team";
const JITSI: &str = "https://jitsi.example.org/team";

fn room(name: &str, url: &str, details: &[&str]) -> WebexRoom {
    WebexRoom { name: name.into(), url: url.into(), details: details.iter().map(|d| d.to_string()).collect() }
}

/// The rooms an answer gives.
fn answered(r: &Value) -> Vec<WebexRoom> {
    serde_json::from_value(r["rooms"].clone()).unwrap()
}

#[test]
fn webex_rooms_are_added_changed_and_removed() {
    let mut s = sample_session("webex-rooms");
    let r = s
        .run(
            "calendar.webexRoom",
            &json!({"url": " https://acme.webex.com/meet/ana ", "name": "Ana", "details": "N° 123\nVidéo : ana@acme.webex.com"}),
        )
        .unwrap();
    assert_eq!(r["rooms"][0], json!({"name": "Ana", "url": ANA, "details": ["N° 123", "Vidéo : ana@acme.webex.com"]}));
    let ana = room("Ana", ANA, &["N° 123", "Vidéo : ana@acme.webex.com"]);
    // Blank lines left out, the others trimmed; a new room goes at the end.
    let r = s.run("calendar.webexRoom", &json!({"url": TEAM, "name": "Équipe", "details": ["  ", "N° 9 "]})).unwrap();
    assert_eq!(answered(&r), [ana.clone(), room("Équipe", TEAM, &["N° 9"])]);
    assert_eq!(s.prefs.webex_rooms, answered(&r));
    // Its lines taken off: its name stays.
    s.run("calendar.webexRoom", &json!({"url": ANA, "details": []})).unwrap();
    assert_eq!(s.prefs.webex_rooms[0], room("Ana", ANA, &[]));
    // Its link changed: the same room, in its place.
    let ana2 = "https://acme.webex.com/meet/ana2";
    let r = s.run("calendar.webexRoom", &json!({"url": ana2, "was": ANA})).unwrap();
    assert_eq!(answered(&r), [room("Ana", ana2, &[]), room("Équipe", TEAM, &["N° 9"])]);
    // Removed.
    let r = s.run("calendar.webexRoom", &json!({"url": TEAM, "remove": true})).unwrap();
    assert_eq!(answered(&r), [room("Ana", ana2, &[])]);
    let r = s.run("calendar.webexRoom", &json!({"url": TEAM, "remove": true}));
    assert!(matches!(r, Err(CmdError::Params(_))), "no room has it: {r:?}");
    assert_eq!(s.prefs.webex_rooms.len(), 1);
    done(s);
}

/// What a room can't keep is refused whole: the rooms stay as they were.
#[test]
fn webex_rooms_refuse_what_they_cannot_keep() {
    let mut s = sample_session("webex-rooms-refused");
    s.run("calendar.webexRoom", &json!({"url": ANA, "name": "Ana"})).unwrap();
    s.run("calendar.webexRoom", &json!({"url": TEAM, "name": "Équipe"})).unwrap();
    let refused = |s: &mut Session, params: Value| {
        let before = s.prefs.webex_rooms.clone();
        let r = s.run("calendar.webexRoom", &params);
        assert!(matches!(r, Err(CmdError::Params(_))), "{params}: {r:?}");
        assert_eq!(s.prefs.webex_rooms, before, "{params}");
    };
    let new = "https://acme.webex.com/meet/new";
    // A new room needs a name, of 80 characters at most.
    refused(&mut s, json!({"url": new}));
    refused(&mut s, json!({"url": new, "name": "  "}));
    refused(&mut s, json!({"url": new, "name": "n".repeat(81)}));
    s.run("calendar.webexRoom", &json!({"url": new, "name": "n".repeat(80)})).unwrap();
    // At most 20 lines of 200 characters.
    refused(&mut s, json!({"url": ANA, "details": vec!["ligne"; 21]}));
    s.run("calendar.webexRoom", &json!({"url": ANA, "details": vec!["ligne"; 20]})).unwrap();
    refused(&mut s, json!({"url": ANA, "details": ["x".repeat(201)]}));
    s.run("calendar.webexRoom", &json!({"url": ANA, "details": ["x".repeat(200)]})).unwrap();
    // Only a Webex personal room's link.
    refused(&mut s, json!({"url": "", "name": "Vide"}));
    for not in ["https://evil.example/meet/ana", "http://acme.webex.com/meet/ana", "acme.webex.com/meet/ana", "https://us02web.zoom.us/j/1"] {
        refused(&mut s, json!({"url": not, "name": "Ailleurs"}));
    }
    // Two rooms never share a link, whatever its case.
    refused(&mut s, json!({"url": "https://acme.webex.com/meet/ANA", "name": "Ana bis"}));
    refused(&mut s, json!({"url": TEAM, "was": ANA}));
    // A room named keeps a name.
    refused(&mut s, json!({"url": ANA, "name": ""}));
    // Twenty rooms at most.
    for i in s.prefs.webex_rooms.len()..20 {
        s.run("calendar.webexRoom", &json!({"url": format!("https://acme.webex.com/meet/r{i}"), "name": format!("Salle {i}")})).unwrap();
    }
    assert_eq!(s.prefs.webex_rooms.len(), 20);
    refused(&mut s, json!({"url": "https://acme.webex.com/meet/r21", "name": "Salle 21"}));
    // Wrong kinds of values.
    refused(&mut s, json!({"url": ANA, "details": {"a": 1}}));
    refused(&mut s, json!({"url": ANA, "details": ["N° 1", 2]}));
    refused(&mut s, json!({"url": ANA, "name": 5}));
    refused(&mut s, json!({"url": ANA, "remove": "yes"}));
    done(s);
}

/// A 1.0 settings file's single room becomes the first room, unnamed, and isn't written again:
/// read once more, it isn't added a second time.
#[test]
fn an_old_personal_room_becomes_the_first_room() {
    let rooms = |p: &Prefs| p.webex_rooms.iter().map(|r| (r.name.clone(), r.url.clone())).collect::<Vec<_>>();
    let mut p: Prefs = serde_json::from_value(json!({"webexRoom": ANA, "webexRooms": [{"name": "Équipe", "url": TEAM}]})).unwrap();
    p.carry_over_webex_rooms();
    let both = vec![(String::new(), ANA.to_string()), ("Équipe".to_string(), TEAM.to_string())];
    assert_eq!(rooms(&p), both);
    assert!(p.webex_room.is_empty());
    let saved = serde_json::to_value(&p).unwrap();
    assert!(saved.get("webexRoom").is_none(), "never written again: {saved}");
    let mut again: Prefs = serde_json::from_value(saved).unwrap();
    again.carry_over_webex_rooms();
    assert_eq!(rooms(&again), both, "not two rooms on the second read");
    // Not a Webex link: no room.
    let mut p: Prefs = serde_json::from_value(json!({"webexRoom": "https://evil.example/x"})).unwrap();
    p.carry_over_webex_rooms();
    assert!(p.webex_rooms.is_empty() && p.webex_room.is_empty());
    // Already among the rooms (whatever its case): not added twice.
    for old in [ANA, " https://acme.webex.com/meet/ANA "] {
        let mut p: Prefs = serde_json::from_value(json!({"webexRoom": old, "webexRooms": [{"name": "Ana", "url": ANA}]})).unwrap();
        p.carry_over_webex_rooms();
        assert_eq!(rooms(&p), [("Ana".to_string(), ANA.to_string())], "{old}");
    }
}

/// A settings file edited by hand is cut to what a room keeps, never refused: the other settings
/// in it are still read.
#[test]
fn a_hand_edited_rooms_list_is_cut_when_read() {
    let mut list = vec![
        json!({"name": "é".repeat(500), "url": ANA, "details": vec!["x".repeat(300); 25]}),
        json!({"name": "Ailleurs", "url": "https://evil.example/meet/ana"}),
        json!({"name": "Majuscules", "url": ANA.to_uppercase()}),
    ];
    list.extend((3..30).map(|i| json!({"name": format!("Salle {i}"), "url": format!("https://acme.webex.com/meet/r{i}")})));
    let mut p: Prefs = serde_json::from_value(json!({"webexRooms": list, "externalTag": true})).unwrap();
    p.carry_over_webex_rooms();
    assert_eq!(p.webex_rooms.len(), 20);
    assert!(p.webex_rooms.iter().all(|r| is_webex_link(&r.url)));
    let links: BTreeSet<String> = p.webex_rooms.iter().map(|r| r.url.to_ascii_lowercase()).collect();
    assert_eq!(links.len(), 20, "each link once");
    let first = &p.webex_rooms[0];
    assert_eq!((first.url.as_str(), first.name.chars().count()), (ANA, 80));
    assert_eq!(first.details.len(), 20);
    assert!(first.details.iter().all(|l| l.chars().count() == 200));
    assert_eq!(p.webex_rooms[1].url, "https://acme.webex.com/meet/r3", "the bad link and the second copy left out");
    assert!(p.external_tag, "the other settings still read");
}

#[test]
fn an_event_takes_a_conference_and_lets_it_go() {
    let mut s = sample_session("event-conference");
    let made = s.run("event.create", &json!({"summary": "Point", "start": "2026-10-21T15:00", "conference": {"url": JITSI, "label": "Jitsi"}}));
    let uid = made.unwrap()["uid"].as_str().unwrap().to_string();
    let conference = |s: &Session| s.find_event(&uid).unwrap().1.event.conference.clone();
    let made_here =
        |url: &str, label: &str| Some(Conference { url: url.into(), label: label.into(), features: vec!["AUDIO".into(), "VIDEO".into()] });
    assert_eq!(conference(&s), made_here(JITSI, "Jitsi"));
    // As another calendar app could have said it.
    s.update_event(&uid, |e| {
        if let Some(c) = e.conference.as_mut() {
            c.features = vec!["VIDEO".into(), "SCREEN".into()];
        }
    })
    .unwrap();
    let read = conference(&s);
    s.run("event.update", &json!({"uid": uid, "summary": "Point d'équipe"})).unwrap();
    assert_eq!(conference(&s), read, "not given: left as it was");
    s.run("event.update", &json!({"uid": uid, "conference": {"url": JITSI}})).unwrap();
    assert_eq!(conference(&s), read, "the same link keeps its label and features");
    let other = "https://jitsi.example.org/other";
    s.run("event.update", &json!({"uid": uid, "conference": {"url": other, "label": "Autre"}})).unwrap();
    assert_eq!(conference(&s), made_here(other, "Autre"));
    for bad in [
        json!("x"),
        json!({"url": "javascript:alert(1)"}),
        json!({"url": 5}),
        json!({"label": "x"}),
        json!({"url": "https://x.example/", "label": 3}),
    ] {
        let before = s.find_event(&uid).unwrap().1.clone();
        let r = s.run("event.update", &json!({"uid": uid, "conference": bad}));
        assert!(matches!(r, Err(CmdError::Params(_))), "{bad}: {r:?}");
        assert_eq!(s.find_event(&uid).unwrap().1, &before, "{bad}");
    }
    s.run("event.update", &json!({"uid": uid, "conference": null})).unwrap();
    assert_eq!(conference(&s), None, "taken out");
    done(s);
}

/// A meeting's guests invited by mail from an IMAP account are told where to join it.
#[test]
fn an_invitation_by_mail_carries_the_conference() {
    let mut s = sample_session("conference-invitation");
    s.run(
        "event.create",
        &json!({"summary": "Online review", "start": "2026-10-21T15:00", "attendees": "Zoe Park <zoe@studio.example>",
                "conference": {"url": JITSI, "label": "Jitsi"}}),
    )
    .unwrap();
    let (method, raw) = sent_calendar(&s, "Sent", "Invitation: Online review");
    assert_eq!(method, "REQUEST");
    let unfolded = raw.replace("\r\n ", "").replace("\n ", "");
    assert_eq!(unfolded.matches("CONFERENCE;").count(), 1, "{unfolded}");
    assert!(unfolded.contains(&format!("CONFERENCE;VALUE=URI;FEATURE=AUDIO,VIDEO;LABEL=Jitsi:{JITSI}")), "{unfolded}");
    done(s);
}

/// Webex gives a scheduled meeting another link: the appointment's conference follows, its label
/// kept.
#[test]
fn a_webex_link_changed_moves_the_conference() {
    use super::webex::{NEW_LINK, OLD_LINK, appointment, meeting};
    let mut s = sample_session("webex-link-conference");
    let uid = appointment(&mut s, OLD_LINK, &format!("Join the Webex meeting: {OLD_LINK}"));
    let e = s.pim_mut("sample").events.iter_mut().find(|e| e.event.uid == uid).unwrap();
    e.event.conference = Conference::new(OLD_LINK, "Webex");
    s.apply(Event::WebexUpdated { id: "m1".into(), meeting: Some(meeting(NEW_LINK)), error: None });
    let c = s.find_event(&uid).unwrap().1.event.conference.clone().unwrap();
    assert_eq!((c.url.as_str(), c.label.as_str()), (NEW_LINK, "Webex"));
    done(s);
}

/// An event cached before Quatrelle read `CONFERENCE` holds the line among its kept ones: read
/// from the cache, it's the event's conference, and the line isn't kept twice. Other people's
/// appointments set aside on disk are adopted as they come back.
#[test]
fn a_cached_events_kept_conference_is_adopted() {
    use quatrelle_model::pim::StoredEvent;
    let account = sample::SAMPLE_ACCOUNT;
    let old = "CONFERENCE;VALUE=URI;LABEL=Old:https://old.example/";
    let mut s = sample_session("conference-adopted");
    let made = s.run("event.create", &json!({"summary": "Cached", "start": "2026-10-21T15:00"}));
    let uid = made.unwrap()["uid"].as_str().unwrap().to_string();
    let e = s.pim_mut(account).events.iter_mut().find(|e| e.event.uid == uid).unwrap();
    e.event.extra = vec![old.into()];
    e.event.conference = None;
    s.save_pim(account);
    let store = s.store.clone().unwrap();
    // Set aside: one of other people's appointments with the line too.
    let mut aside: Vec<StoredEvent> = store.load_account_doc(account, "pim-parked.json").unwrap();
    let theirs = aside.first().map(|e| (e.calendar.clone(), e.event.uid.clone(), e.event.recurrence_id)).unwrap();
    aside[0].event.extra.push(old.into());
    store.save_account_doc(account, "pim-parked.json", &aside).unwrap();
    drop(s);
    let mut s = Session::open(Some(store), quatrelle_model::dates::Clock::new(NOW, 0));
    let adopted = |e: &quatrelle_model::ical::CalEvent| {
        e.conference.as_ref().map(|c| (c.url.as_str(), c.label.as_str())) == Some(("https://old.example/", "Old"))
            && !e.extra.iter().any(|l| l == old)
    };
    let e = &s.find_event(&uid).unwrap().1.event;
    assert!(adopted(e), "{e:?}");
    assert!(e.extra.is_empty());
    s.run("view.calendar", &json!({})).unwrap();
    s.take_jobs();
    let back = s.pim[account].events.iter().find(|e| (e.calendar.clone(), e.event.uid.clone(), e.event.recurrence_id) == theirs).unwrap();
    assert!(adopted(&back.event), "{:?}", back.event);
    done(s);
}
