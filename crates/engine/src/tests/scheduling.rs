// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: the rooms offered for a meeting (the Room Finder) and when everyone is free
//! (the Scheduling Assistant), from the calendars kept here and from an Exchange server.

use quatrelle_model::pim::{CalendarInfo, DavSync, shared_calendar_id};
use quatrelle_model::scheduling::{Availability, BusyKind, BusySlot, MailTip, Room, RoomList};

use super::*;

/// The sample's rooms: the calendars in Outlook's Rooms group and the room booked in a
/// meeting; a meeting made with one carries it as a resource and is held there; a room is
/// changed apart from the guests, and a place typed stays.
#[test]
fn the_sample_offers_its_rooms_and_books_one() {
    let mut s = sample_session("rooms-local");
    let r = s.run("calendar.rooms", &json!({})).unwrap();
    assert_eq!((r["state"].as_str(), r["fromCalendar"].as_bool()), (Some("ready"), Some(true)));
    let names: Vec<&str> = r["rooms"].as_array().unwrap().iter().filter_map(|x| x["name"].as_str()).collect();
    assert_eq!(names, ["Screening room", "Sound Lab"]);
    assert!(s.take_jobs().is_empty(), "nothing asked of a server");
    let uid = s
        .run(
            "event.create",
            &json!({"summary": "Foley review", "start": "2026-10-16T14:00", "attendees": "Zoe Park <zoe@studio.example>", "rooms": "Sound Lab <sound.lab@studio.example>"}),
        )
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    let e = s.find_event(&uid).unwrap().1.event.clone();
    assert_eq!(e.location, "Sound Lab", "held in the room");
    let room = e.attendees.iter().find(|a| a.is_resource()).unwrap();
    assert_eq!((room.email.as_str(), room.name.as_str(), room.cutype.as_str()), ("sound.lab@studio.example", "Sound Lab", "ROOM"));
    assert_eq!(e.attendees.iter().filter(|a| !a.is_resource()).map(|a| a.email.as_str()).collect::<Vec<_>>(), ["zoe@studio.example"]);
    s.run("event.update", &json!({"uid": uid, "rooms": "screening.room@studio.example", "location": "Online"})).unwrap();
    let e = s.find_event(&uid).unwrap().1.event.clone();
    assert_eq!(e.location, "Online", "a place given stays");
    assert_eq!(e.attendees.iter().filter(|a| a.is_resource()).map(|a| a.email.as_str()).collect::<Vec<_>>(), ["screening.room@studio.example"]);
    assert_eq!(
        e.attendees.iter().filter(|a| !a.is_resource()).map(|a| a.email.as_str()).collect::<Vec<_>>(),
        ["zoe@studio.example"],
        "the guests stay"
    );
    // Written and read as iCalendar, the room keeps its kind.
    let ics = quatrelle_model::ical::write(std::slice::from_ref(&e), None, NOW);
    assert!(ics.contains("CUTYPE=ROOM"), "{ics}");
    let back = quatrelle_model::ical::parse(&ics, 0);
    assert!(back.events[0].attendees.iter().any(|a| a.cutype == "ROOM"));
    // Now offered too, as a room the calendar knows.
    let r = s.run("calendar.rooms", &json!({"refresh": true})).unwrap();
    assert_eq!(r["rooms"].as_array().map(Vec::len), Some(2), "both were known already");
    assert!(s.run("event.create", &json!({"summary": "x", "start": "2026-10-16T14:00", "rooms": "not an address"})).is_err());
    done(s);
}

/// When everyone is free, from the calendars kept here: the organizer's own calendars, a
/// colleague's and a room's opened here; someone whose calendar isn't here has no information;
/// nothing is asked of a server, and bad windows are refused.
#[test]
fn availability_comes_from_the_calendars_kept_here() {
    let mut s = sample_session("availability-local");
    let r = s
        .run(
            "calendar.availability",
            &json!({"emails": "riley@studio.example, zoe@studio.example, Sound Lab <sound.lab@studio.example>, nobody@elsewhere.example", "from": "2026-10-14"}),
        )
        .unwrap();
    let people = r["people"].as_array().unwrap();
    assert_eq!(people.len(), 4);
    assert!(people.iter().all(|p| p["state"] == "ready"), "{people:?}");
    let busy = |i: usize| {
        people[i]["busy"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| (b["subject"].as_str().unwrap_or("").to_string(), b["kind"].as_str().unwrap_or("").to_string()))
            .collect::<Vec<_>>()
    };
    let mine = busy(0);
    for what in ["Team stand-up", "Edit session: act two", "Lunch with Jun"] {
        assert!(mine.iter().any(|(s, k)| s == what && k == "busy"), "{what} in {mine:?}");
    }
    assert_eq!(busy(1), [("Producers' call".to_string(), "busy".to_string()), ("Casting session".to_string(), "tentative".to_string())]);
    assert_eq!(busy(2), [("Foley session".to_string(), "busy".to_string())]);
    assert!(people[3]["error"].as_str().is_some_and(|e| e.contains("calendar")), "{:?}", people[3]);
    assert!(s.take_jobs().is_empty(), "nothing asked of a server");
    assert!(s.run("calendar.availability", &json!({"emails": "a@x", "from": "2026-10-14", "to": "2026-10-13"})).is_err());
    assert!(s.run("calendar.availability", &json!({"emails": "a@x", "from": "2026-10-14", "to": "2027-10-14"})).is_err());
    assert!(s.run("calendar.availability", &json!({"from": "2026-10-14"})).is_err());
    assert!(s.run("calendar.availability", &json!({"emails": "a@x"})).is_err());
    done(s);
}

/// An Exchange account asks its server: rooms once (the answer kept, the calendar's rooms after
/// the server's; a server with no lists set up still offers the calendar's, one that refused
/// says why when there's nothing else), free/busy once per window, each answer finding its
/// entry, a failure reaching those still waiting.
#[test]
fn an_exchange_account_asks_its_server_once() {
    let mut s = Session::open(Some(temp("rooms-ews")), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s
        .run("account.add", &json!({"email": "riley@contoso.example", "ews": "mail.contoso.example", "username": "CONTOSO\\riley", "password": "pw"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    // Its calendars: a "Salles" group, as a French Outlook keeps room calendars.
    let room = shared_calendar_id("room.lyon@contoso.example");
    let cal = |id: &str, name: &str, group: &str| CalendarInfo {
        id: id.into(),
        name: name.into(),
        group: group.into(),
        read_only: true,
        ..Default::default()
    };
    s.apply_dav(
        &id,
        &DavSync {
            calendars: Some(vec![cal("ews:calendar", "Calendar", ""), cal(&room, "Salle Lyon", "Salles")]),
            checked: NOW,
            ..Default::default()
        },
    );
    s.take_jobs();
    assert_eq!(s.run("calendar.rooms", &json!({})).unwrap()["state"], "loading");
    assert!(matches!(s.take_jobs().as_slice(), [Job::GetRooms { account }] if *account == id));
    assert_eq!(s.run("calendar.rooms", &json!({})).unwrap()["state"], "loading");
    assert!(s.take_jobs().is_empty(), "asked once");
    let names = |r: &Value| r["rooms"].as_array().unwrap().iter().filter_map(|x| x["name"].as_str().map(str::to_string)).collect::<Vec<_>>();
    // The server has lists: its rooms first, the calendar's after them.
    s.apply(Event::Rooms {
        account: id.clone(),
        lists: vec![RoomList { email: "rooms.paris@contoso.example".into(), name: "Paris".into() }],
        rooms: vec![Room { email: "room.seine@contoso.example".into(), name: "Salle Seine".into(), list: "Paris".into() }],
        error: None,
    });
    let r = s.run("calendar.rooms", &json!({})).unwrap();
    assert_eq!((r["state"].as_str(), r["fromCalendar"].as_bool()), (Some("ready"), Some(false)));
    assert_eq!(names(&r), ["Salle Seine", "Salle Lyon"]);
    assert_eq!(r["lists"][0]["name"], "Paris");
    // No lists set up: the calendar's rooms alone, and that said.
    s.run("calendar.rooms", &json!({"refresh": true})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::GetRooms { .. }]));
    s.apply(Event::Rooms { account: id.clone(), lists: Vec::new(), rooms: Vec::new(), error: None });
    let r = s.run("calendar.rooms", &json!({})).unwrap();
    assert_eq!((r["fromCalendar"].as_bool(), names(&r)), (Some(true), vec!["Salle Lyon".to_string()]));
    // Refused, with a room still known: offered all the same; with none: the refusal.
    s.run("calendar.rooms", &json!({"refresh": true})).unwrap();
    s.take_jobs();
    s.apply(Event::Rooms { account: id.clone(), lists: Vec::new(), rooms: Vec::new(), error: Some("not on this server".into()) });
    assert_eq!(names(&s.run("calendar.rooms", &json!({})).unwrap()), ["Salle Lyon"]);
    s.pim_mut(&id).calendars.retain(|c| c.id != room);
    s.run("calendar.rooms", &json!({"refresh": true})).unwrap();
    s.take_jobs();
    s.apply(Event::Rooms { account: id.clone(), lists: Vec::new(), rooms: Vec::new(), error: Some("not on this server".into()) });
    let r = s.run("calendar.rooms", &json!({})).unwrap();
    assert_eq!((r["state"].as_str(), r["error"].as_str()), (Some("failed"), Some("not on this server")));
    // Free/busy: asked for the window once; someone added later is asked for alone.
    let from = crate::parse_local("2026-10-22", &quatrelle_model::dates::Clock::new(0, 0)).unwrap().0;
    let ask = |s: &mut Session, emails: &[&str]| {
        s.run("calendar.availability", &json!({"account": id, "emails": emails, "from": from, "to": from + 86_400})).unwrap()
    };
    let r = ask(&mut s, &["riley@contoso.example", "room.seine@contoso.example"]);
    assert!(r["people"].as_array().unwrap().iter().all(|p| p["state"] == "loading"), "{r}");
    let jobs = s.take_jobs();
    assert!(
        matches!(jobs.as_slice(), [Job::GetAvailability { emails, from: f, to, .. }] if emails.len() == 2 && *f == from && *to == from + 86_400),
        "{jobs:?}"
    );
    ask(&mut s, &["riley@contoso.example", "room.seine@contoso.example"]);
    assert!(s.take_jobs().is_empty(), "asked once per window");
    ask(&mut s, &["riley@contoso.example", "room.seine@contoso.example", "ana@contoso.example"]);
    let jobs = s.take_jobs();
    assert!(matches!(jobs.as_slice(), [Job::GetAvailability { emails, .. }] if emails == &["ana@contoso.example".to_string()]), "{jobs:?}");
    s.apply(Event::Availability {
        account: id.clone(),
        from,
        to: from + 86_400,
        people: vec![
            Availability {
                email: "riley@contoso.example".into(),
                slots: vec![BusySlot { start: from + 9 * 3600, end: from + 10 * 3600, kind: BusyKind::Busy, subject: "Budget".into() }],
                error: None,
            },
            Availability { email: "ROOM.seine@contoso.example".into(), slots: Vec::new(), error: Some("no access".into()) },
        ],
        error: None,
    });
    let r = ask(&mut s, &["riley@contoso.example", "room.seine@contoso.example", "ana@contoso.example"]);
    let p = r["people"].as_array().unwrap();
    assert_eq!(
        (p[0]["state"].as_str(), p[0]["busy"][0]["kind"].as_str(), p[0]["busy"][0]["subject"].as_str()),
        (Some("ready"), Some("busy"), Some("Budget"))
    );
    assert_eq!((p[1]["state"].as_str(), p[1]["error"].as_str()), (Some("ready"), Some("no access")));
    assert_eq!(p[2]["state"], "loading");
    // The second request failed as a whole: the one it was about hears why (the worker names
    // everyone it asked about), the others keep their answers.
    s.apply(Event::Availability {
        account: id.clone(),
        from,
        to: from + 86_400,
        people: vec![Availability { email: "ana@contoso.example".into(), slots: Vec::new(), error: Some("the account is offline".into()) }],
        error: Some("the account is offline".into()),
    });
    let r = ask(&mut s, &["ana@contoso.example"]);
    assert_eq!((r["people"][0]["state"].as_str(), r["people"][0]["error"].as_str()), (Some("failed"), Some("the account is offline")));
    assert!(s.take_jobs().is_empty(), "a failure isn't asked again by itself");
    assert_eq!(ask(&mut s, &["riley@contoso.example"])["people"][0]["state"], "ready");
    // An answer naming no one that failed: everyone still waiting on the window hears why.
    ask(&mut s, &["ben@contoso.example"]);
    s.take_jobs();
    s.apply(Event::Availability { account: id.clone(), from, to: from + 86_400, people: Vec::new(), error: Some("no connection".into()) });
    assert_eq!(ask(&mut s, &["ben@contoso.example"])["people"][0]["error"], "no connection");
    assert_eq!(ask(&mut s, &["riley@contoso.example"])["people"][0]["state"], "ready", "an answer already here stays");
    // Another day: asked again.
    s.run("calendar.availability", &json!({"account": id, "emails": ["riley@contoso.example"], "from": from + 86_400})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::GetAvailability { .. }]));
    done(s);
}

/// MailTips for a message being written: an Exchange account is asked about its recipients once
/// per set, in To, Cc and Bcc order; an answer to a set since changed is let go; closing the
/// message forgets them; an account with no server for them isn't asked.
#[test]
fn mail_tips_follow_the_recipients_of_a_message_being_written() {
    let mut s = sample_session("mail-tips");
    let account = s
        .run("account.add", &json!({"email": "riley@contoso.example", "ews": "mail.contoso.example", "username": "CONTOSO\\riley", "password": "pw"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    let id = s.run("mail.new", &json!({"account": account})).unwrap()["compose"].as_u64().unwrap();
    // Nothing to ask about yet.
    assert!(!s.mail_tips_wanted());
    assert!(s.run("compose.mailTips", &json!({})).is_err(), "no recipient");
    s.run("compose.update", &json!({"to": "Ana <ana@contoso.example>; bob@contoso.example", "cc": "carol@contoso.example"})).unwrap();
    assert!(s.mail_tips_wanted());
    assert_eq!(s.run("compose.mailTips", &json!({})).unwrap()["state"], "loading");
    assert!(!s.mail_tips_wanted(), "one request per set, while it's on its way");
    let (ask, recipients) = match s.take_jobs().as_slice() {
        [Job::GetMailTips { from, recipients, ask, .. }] => {
            assert_eq!(from, "riley@contoso.example");
            (*ask, recipients.clone())
        }
        jobs => panic!("{jobs:?}"),
    };
    assert_eq!(recipients, ["ana@contoso.example", "bob@contoso.example", "carol@contoso.example"], "in To then Cc, lower case, once each");
    // One answer worth showing, one ordinary: only the first stays.
    s.apply(Event::MailTips {
        account: account.clone(),
        ask,
        tips: vec![
            MailTip { email: "ana@contoso.example".into(), automatic_replies: "Away until Monday".into(), ..Default::default() },
            MailTip { email: "bob@contoso.example".into(), ..Default::default() },
        ],
        error: None,
    });
    let r = s.run("compose.mailTips", &json!({})).unwrap();
    assert_eq!((r["state"].as_str(), r["tips"].as_array().map(Vec::len)), (Some("ready"), Some(1)));
    assert_eq!((r["tips"][0]["email"].as_str(), r["tips"][0]["automaticReplies"].as_str()), (Some("ana@contoso.example"), Some("Away until Monday")));
    // A recipient added: a new question, and the old answer is let go.
    s.run("compose.update", &json!({"bcc": "dave@contoso.example"})).unwrap();
    assert!(s.mail_tips_wanted());
    s.run("compose.mailTips", &json!({})).unwrap();
    let again = match s.take_jobs().as_slice() {
        [Job::GetMailTips { ask, recipients, .. }] => {
            assert_eq!(recipients.last().map(String::as_str), Some("dave@contoso.example"));
            *ask
        }
        jobs => panic!("{jobs:?}"),
    };
    assert_ne!(again, ask);
    s.apply(Event::MailTips {
        account: account.clone(),
        ask,
        tips: vec![MailTip { email: "bob@contoso.example".into(), mailbox_full: true, ..Default::default() }],
        error: None,
    });
    assert_eq!(s.run("compose.mailTips", &json!({})).unwrap()["state"], "loading", "an answer for the set from before is let go");
    s.apply(Event::MailTips {
        account: account.clone(),
        ask: again,
        tips: vec![MailTip { email: "dave@contoso.example".into(), custom: "Ring first".into(), ..Default::default() }],
        error: None,
    });
    let r = s.run("compose.mailTips", &json!({})).unwrap();
    assert_eq!((r["state"].as_str(), r["tips"][0]["custom"].as_str()), (Some("ready"), Some("Ring first")));
    // Closing the message forgets them.
    s.run("compose.close", &json!({"save": false})).unwrap();
    assert!(s.mail_tips_now(id).is_none());
    // The sample mailbox has no server that answers MailTips.
    s.run("mail.new", &json!({"account": sample::SAMPLE_ACCOUNT})).unwrap();
    s.run("compose.update", &json!({"to": "ana@soundlab.example"})).unwrap();
    assert!(!s.mail_tips_wanted());
    assert!(s.run("compose.mailTips", &json!({})).is_err());
    // Another account's server is asked in turn, even with the same recipients: the tips were
    // the first server's.
    let other = s
        .run("account.add", &json!({"email": "sam@fourthcoffee.example", "ews": "mail.fourthcoffee.example", "username": "sam", "password": "pw"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    s.run("mail.new", &json!({"account": account})).unwrap();
    s.run("compose.update", &json!({"to": "ana@contoso.example"})).unwrap();
    s.run("compose.mailTips", &json!({})).unwrap();
    let (ask, from) = match s.take_jobs().as_slice() {
        [Job::GetMailTips { ask, from, .. }] => (*ask, from.clone()),
        jobs => panic!("{jobs:?}"),
    };
    assert_eq!(from, "riley@contoso.example");
    s.apply(Event::MailTips {
        account: account.clone(),
        ask,
        tips: vec![MailTip { email: "ana@contoso.example".into(), custom: "Ring first".into(), ..Default::default() }],
        error: None,
    });
    s.run("compose.update", &json!({"account": other})).unwrap();
    assert!(s.mail_tips_wanted(), "the other server hasn't been asked about these recipients");
    assert_eq!(s.run("compose.mailTips", &json!({})).unwrap()["state"], "loading");
    match s.take_jobs().as_slice() {
        [Job::GetMailTips { from, .. }] => assert_eq!(from, "sam@fourthcoffee.example"),
        jobs => panic!("{jobs:?}"),
    }
    done(s);
}

/// One MailTips question on its way per message: the recipients changed while it is out wait for
/// its answer, the newest set in place of any that waited, and only then is the server asked
/// again; back to the set on its way, nothing waits.
#[test]
fn one_mail_tips_question_is_on_its_way_at_a_time() {
    let mut s = sample_session("mail-tips-one");
    let account = s
        .run("account.add", &json!({"email": "riley@contoso.example", "ews": "mail.contoso.example", "username": "CONTOSO\\riley", "password": "pw"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    s.run("mail.new", &json!({"account": account})).unwrap();
    let asked = |s: &mut Session| -> Vec<(u64, Vec<String>)> {
        s.take_jobs()
            .into_iter()
            .filter_map(|j| match j {
                Job::GetMailTips { ask, recipients, .. } => Some((ask, recipients)),
                _ => None,
            })
            .collect()
    };
    let ask_now = |s: &mut Session, update: Value| {
        s.run("compose.update", &update).unwrap();
        if s.mail_tips_wanted() {
            s.run("compose.mailTips", &json!({})).unwrap();
        }
    };
    ask_now(&mut s, json!({"to": "ana@contoso.example"}));
    let first = asked(&mut s);
    assert_eq!(first.len(), 1, "{first:?}");
    // Two more sets while it is on its way: nothing more is asked yet.
    ask_now(&mut s, json!({"cc": "bo@contoso.example"}));
    ask_now(&mut s, json!({"cc": "bo@contoso.example; cy@contoso.example"}));
    assert!(!s.mail_tips_wanted(), "the newest set waits for the answer");
    assert!(asked(&mut s).is_empty(), "one question on its way");
    // Its answer: the newest set is asked about, once.
    s.apply(Event::MailTips {
        account: account.clone(),
        ask: first[0].0,
        tips: vec![MailTip { email: "ana@contoso.example".into(), custom: "Ring first".into(), ..Default::default() }],
        error: None,
    });
    let next = asked(&mut s);
    let sets: Vec<&Vec<String>> = next.iter().map(|(_, r)| r).collect();
    assert_eq!(sets, [&vec!["ana@contoso.example".to_string(), "bo@contoso.example".into(), "cy@contoso.example".into()]], "the newest set, once");
    // A set changed and changed back while that one is out: nothing waits.
    ask_now(&mut s, json!({"cc": "bo@contoso.example"}));
    ask_now(&mut s, json!({"cc": "bo@contoso.example; cy@contoso.example"}));
    assert!(!s.mail_tips_wanted());
    s.apply(Event::MailTips { account: account.clone(), ask: next[0].0, tips: Vec::new(), error: None });
    assert!(asked(&mut s).is_empty(), "nothing waited");
    done(s);
}

/// An answer naming an address that wasn't asked about keeps nothing of it.
#[test]
fn mail_tips_keep_only_the_recipients_asked_about() {
    let mut s = sample_session("mail-tips-asked");
    let account = s
        .run("account.add", &json!({"email": "riley@contoso.example", "ews": "mail.contoso.example", "username": "CONTOSO\\riley", "password": "pw"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    s.run("mail.new", &json!({"account": account})).unwrap();
    s.run("compose.update", &json!({"to": "cy@contoso.example"})).unwrap();
    s.run("compose.mailTips", &json!({})).unwrap();
    let ask = match s.take_jobs().as_slice() {
        [Job::GetMailTips { ask, .. }] => *ask,
        jobs => panic!("{jobs:?}"),
    };
    s.apply(Event::MailTips {
        account,
        ask,
        tips: vec![
            MailTip { email: "zed@contoso.example".into(), custom: "Not asked".into(), ..Default::default() },
            MailTip { email: "cy@contoso.example".into(), mailbox_full: true, ..Default::default() },
        ],
        error: None,
    });
    let r = s.run("compose.mailTips", &json!({})).unwrap();
    let emails: Vec<&str> = r["tips"].as_array().into_iter().flatten().filter_map(|t| t["email"].as_str()).collect();
    assert_eq!(emails, ["cy@contoso.example"], "only who was asked about: {r}");
    done(s);
}
