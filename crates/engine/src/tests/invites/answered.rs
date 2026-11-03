// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: an answered invitation, and the meeting's earlier invitations in the Inbox, go
//! to Deleted Items (behind `Prefs::tidy_answered_invites`); a newer update stays.

use super::*;

const ZOE: (&str, &str) = ("Zoe Park", "zoe@soundlab.example");

/// Ana's meeting, with the user among its guests, in version `sequence`.
fn meeting(summary: &str, sequence: u32) -> CalEvent {
    CalEvent {
        uid: "review@soundlab.example".into(),
        summary: summary.into(),
        start: NOW + 86_400,
        end: NOW + 86_400 + 3600,
        organizer: Some(Attendee { email: ANA.1.into(), name: ANA.0.into(), ..Default::default() }),
        attendees: vec![me("NEEDS-ACTION")],
        sequence,
        ..Default::default()
    }
}

/// The message put as `n` in a folder of the sample account.
fn key(s: &Session, folder: &str, n: u32) -> quatrelle_model::MessageKey {
    let validity = s.store.as_ref().unwrap().load_folder("sample", folder).unwrap().validity;
    quatrelle_model::MessageKey { account: "sample".into(), folder: folder.into(), validity, uid: 9000 + n }
}

fn trash(s: &Session) -> String {
    s.account("sample").and_then(|a| a.folder(FolderRole::Trash)).map(|f| f.path.clone()).unwrap()
}

/// The Message-IDs in a folder of the sample account (angle brackets aside).
fn ids(s: &mut Session, folder: &str) -> Vec<String> {
    s.folder_state("sample", folder).messages.iter().map(|m| m.message_id.trim_matches(['<', '>']).to_string()).collect()
}

/// Message `n` is in the list shown.
fn listed(s: &Session, n: u32) -> bool {
    s.list.iter().any(|m| m.message_id.trim_matches(['<', '>']) == format!("i{n}@x"))
}

#[test]
fn answered_in_the_message_takes_its_earlier_versions_and_leaves_a_newer_one() {
    let mut s = sample_session("answered-versions");
    let lunch = CalEvent { uid: "lunch@soundlab.example".into(), ..meeting("Lunch", 1) };
    put(&mut s, "INBOX", 0, ZOE, "Lunch", "REQUEST", &[lunch], NOW);
    put(&mut s, "INBOX", 1, ANA, "Mix review", "REQUEST", &[meeting("Mix review", 1)], NOW);
    put(&mut s, "INBOX", 3, ANA, "Updated: Mix review", "REQUEST", &[meeting("Mix review", 3)], NOW);
    deliver(&mut s, 2, ANA, "REQUEST", &[meeting("Mix review", 2)], NOW);
    let (k0, k1, k2, k3) = (key(&s, "INBOX", 0), key(&s, "INBOX", 1), key(&s, "INBOX", 2), key(&s, "INBOX", 3));
    // The next message is anchored at the one read, not after the lowest one going.
    assert_eq!(s.successor_of(&k3, &[k1.clone(), k3.clone()]), Some(k2.clone()));
    let next = s.successor_of(&k2, &[k1.clone(), k2.clone()]).unwrap();
    assert_eq!(next, k0);
    assert!(!s.list.iter().find(|m| m.key == next).unwrap().flags.seen);

    let r = s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    assert_eq!(r["moved"], 2, "{r}");
    let t = trash(&s);
    let gone = ids(&mut s, &t);
    assert!(gone.contains(&"i1@x".to_string()) && gone.contains(&"i2@x".to_string()), "{gone:?}");
    assert!(!listed(&s, 1) && !listed(&s, 2), "out of the Inbox");
    assert!(listed(&s, 3), "the newer update stays");
    assert_eq!(s.reading.as_ref().map(|r| r.key.clone()), Some(next.clone()));
    assert!(!s.list.iter().find(|m| m.key == next).unwrap().flags.seen, "not marked read");
    done(s);
}

#[test]
fn with_the_switch_off_nothing_moves() {
    let mut s = sample_session("answered-off");
    assert!(s.prefs.tidy_answered_invites, "on at first");
    assert_eq!(s.run("view.tidyAnsweredInvites", &json!({"on": false})).unwrap(), json!({"on": false}));
    assert!(!s.prefs.tidy_answered_invites);
    put(&mut s, "INBOX", 1, ANA, "Mix review", "REQUEST", &[meeting("Mix review", 1)], NOW);
    deliver(&mut s, 2, ANA, "REQUEST", &[meeting("Mix review", 2)], NOW);
    let k2 = key(&s, "INBOX", 2);
    s.pop_out(&k2).unwrap();
    assert_eq!(s.windows.len(), 1);
    let r = s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    assert_eq!(r["moved"], 0, "{r}");
    assert!(listed(&s, 1) && listed(&s, 2), "both stay in the Inbox");
    assert!(s.windows.is_empty(), "its window still closes");
    // Without `on`, it toggles.
    assert_eq!(s.run("view.tidyAnsweredInvites", &json!({})).unwrap(), json!({"on": true}));
    done(s);
}

#[test]
fn undo_puts_them_back() {
    let mut s = sample_session("answered-undo");
    put(&mut s, "INBOX", 1, ANA, "Mix review", "REQUEST", &[meeting("Mix review", 1)], NOW);
    deliver(&mut s, 2, ANA, "REQUEST", &[meeting("Mix review", 2)], NOW);
    let r = s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    assert_eq!(r["moved"], 2, "{r}");
    assert!(!listed(&s, 1) && !listed(&s, 2));
    s.run("edit.undo", &json!({})).unwrap();
    assert!(listed(&s, 1) && listed(&s, 2), "back in the Inbox");
    let t = trash(&s);
    let left = ids(&mut s, &t);
    assert!(!left.contains(&"i1@x".to_string()) && !left.contains(&"i2@x".to_string()), "{left:?}");
    done(s);
}

#[test]
fn only_messages_named_by_title_or_organizer_are_looked_at() {
    let mut s = sample_session("answered-filter");
    // From the organizer under another subject: found by its sender.
    put(&mut s, "INBOX", 1, ANA, "New time", "REQUEST", &[meeting("Mix review", 1)], NOW);
    // Neither the title nor the organizer: not looked at, though it carries the meeting.
    put(&mut s, "INBOX", 3, ZOE, "Agenda", "REQUEST", &[meeting("Mix review", 1)], NOW);
    deliver(&mut s, 2, ANA, "REQUEST", &[meeting("Mix review", 2)], NOW);
    let r = s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    assert_eq!(r["moved"], 2, "{r}");
    assert!(!listed(&s, 1), "the organizer's goes");
    assert!(listed(&s, 3), "the one not looked at stays");
    done(s);
}

#[test]
fn a_message_not_downloaded_stays_and_isnt_fetched() {
    let mut s = sample_session("answered-not-here");
    // Numbered above the one answered, so that it isn't the next one shown (which is fetched).
    let k3 = put(&mut s, "INBOX", 3, ANA, "Mix review", "REQUEST", &[meeting("Mix review", 1)], NOW);
    s.store.as_ref().unwrap().remove_body(&k3);
    deliver(&mut s, 2, ANA, "REQUEST", &[meeting("Mix review", 2)], NOW);
    s.take_jobs();
    let r = s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    assert_eq!(r["moved"], 1, "{r}");
    assert!(listed(&s, 3), "not downloaded: it stays");
    let jobs = s.take_jobs();
    assert!(!jobs.iter().any(|j| matches!(j, Job::FetchBody { key } if *key == k3)), "{jobs:?}");
    done(s);
}

#[test]
fn a_short_title_matches_no_subject() {
    let mut s = sample_session("answered-short");
    put(&mut s, "INBOX", 1, ZOE, "ab ab", "REQUEST", &[meeting("ab", 1)], NOW);
    put(&mut s, "INBOX", 3, ANA, "Something", "REQUEST", &[meeting("ab", 1)], NOW);
    deliver(&mut s, 2, ANA, "REQUEST", &[meeting("ab", 2)], NOW);
    let r = s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    assert_eq!(r["moved"], 2, "{r}");
    assert!(listed(&s, 1), "a two-letter title names nothing");
    assert!(!listed(&s, 3), "the organizer's goes");
    done(s);
}

#[test]
fn answered_in_deleted_items_it_stays_and_the_inbox_ones_go() {
    let mut s = sample_session("answered-in-trash");
    let t = trash(&s);
    put(&mut s, "INBOX", 1, ANA, "Mix review", "REQUEST", &[meeting("Mix review", 1)], NOW);
    let k2 = put(&mut s, &t, 2, ANA, "Mix review", "REQUEST", &[meeting("Mix review", 2)], NOW);
    s.run("folder.open", &json!({"account": "sample", "folder": t})).unwrap();
    s.run("list.open", &json!({"key": k2.to_id()})).unwrap();
    let r = s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    assert_eq!(r["moved"], 1, "{r}");
    let in_trash = ids(&mut s, &t);
    assert!(in_trash.contains(&"i1@x".to_string()), "the Inbox one is in Deleted Items: {in_trash:?}");
    assert_eq!(in_trash.iter().filter(|m| *m == "i2@x").count(), 1, "the answered one is there once");
    assert!(!ids(&mut s, "INBOX").contains(&"i1@x".to_string()));
    done(s);
}

#[test]
fn an_occurrence_answered_moves_only_its_invitations() {
    let mut s = sample_session("answered-occurrence");
    let master = weekly_series(&mut s);
    let week = |n: i64| master.start + n * WEEK;
    let one = |n: i64, seq: u32| CalEvent {
        recurrence_id: Some(week(n)),
        start: week(n) + 3600,
        end: week(n) + 7200,
        rrule: None,
        sequence: seq,
        attendees: vec![me("NEEDS-ACTION")],
        ..master.clone()
    };
    // An earlier version of week 3: goes with it.
    put(&mut s, "INBOX", 1, ANA, "Mix review (week 3)", "REQUEST", &[one(3, 1)], NOW);
    // The whole series, and another week: stay.
    let series = CalEvent { attendees: vec![me("NEEDS-ACTION")], ..master.clone() };
    put(&mut s, "INBOX", 3, ANA, "Mix review", "REQUEST", &[series], NOW);
    put(&mut s, "INBOX", 4, ANA, "Mix review (week 4)", "REQUEST", &[one(4, 1)], NOW);
    deliver(&mut s, 2, ANA, "REQUEST", &[one(3, 2)], NOW);
    let r = s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    assert_eq!(r["moved"], 2, "{r}");
    assert!(!listed(&s, 1) && !listed(&s, 2), "week 3's go");
    assert!(listed(&s, 3), "the series' stays");
    assert!(listed(&s, 4), "week 4's stays");
    done(s);
}

/// Put a meeting in a server calendar of the sample account (`calendar`), as a sync would.
fn in_calendar(s: &mut Session, calendar: &str, href: &str, event: CalEvent) {
    let sync = DavSync {
        calendars: Some(vec![CalendarInfo { id: calendar.into(), name: "Work".into(), ..Default::default() }]),
        events: vec![StoredEvent { calendar: calendar.into(), href: href.into(), etag: "\"1\"".into(), event, ..Default::default() }],
        synced_calendars: vec![calendar.into()],
        checked: NOW,
        ..Default::default()
    };
    s.apply(Event::DavSynced { account: "sample".into(), sync: Box::new(sync) });
}

const WORK: &str = "https://dav.example.org/cal/work/";

/// Ana's meeting at version 2 in the calendar, its invitations at versions 1 and 2 in the Inbox.
fn answered_from_the_calendar(name: &str) -> (Session, CalEvent) {
    let mut s = sample_session(name);
    let m = meeting("Mix review", 2);
    in_calendar(&mut s, WORK, &format!("{WORK}review.ics"), m.clone());
    put(&mut s, "INBOX", 1, ANA, "Mix review", "REQUEST", &[meeting("Mix review", 1)], NOW);
    put(&mut s, "INBOX", 2, ANA, "Updated: Mix review", "REQUEST", &[meeting("Mix review", 2)], NOW);
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    (s, m)
}

#[test]
fn answered_in_the_calendar_takes_the_inbox_invitations() {
    let (mut s, m) = answered_from_the_calendar("answered-calendar");
    let other = s.list.iter().find(|m| m.key.uid < 9000).map(|m| m.key.clone()).unwrap();
    s.run("list.open", &json!({"key": other.to_id()})).unwrap();
    let r = s.run("event.respond", &json!({"uid": m.uid, "answer": "accepted"})).unwrap();
    assert_eq!(r["moved"], 2, "{r}");
    let t = trash(&s);
    let gone = ids(&mut s, &t);
    assert!(gone.contains(&"i1@x".to_string()) && gone.contains(&"i2@x".to_string()), "{gone:?}");
    assert!(!listed(&s, 1) && !listed(&s, 2), "out of the Inbox");
    assert_eq!(s.reading.as_ref().map(|r| r.key.clone()), Some(other), "the message read stays");
    done(s);
}

#[test]
fn answered_in_the_calendar_while_its_invitation_is_read_goes_on() {
    let (mut s, m) = answered_from_the_calendar("answered-calendar-read");
    let (k1, k2) = (key(&s, "INBOX", 1), key(&s, "INBOX", 2));
    s.run("list.open", &json!({"key": k2.to_id()})).unwrap();
    let next = s.successor_of(&k2, &[k1, k2.clone()]).unwrap();
    let r = s.run("event.respond", &json!({"uid": m.uid, "answer": "tentative"})).unwrap();
    assert_eq!(r["moved"], 2, "{r}");
    assert!(!listed(&s, 1) && !listed(&s, 2), "out of the Inbox");
    assert_eq!(s.reading.as_ref().map(|r| r.key.clone()), Some(next), "the list goes on");
    done(s);
}

#[test]
fn ones_own_meeting_answered_in_the_calendar_moves_nothing() {
    let mut s = sample_session("answered-own");
    let riley = Some(Attendee { email: "riley@studio.example".into(), name: "Riley".into(), ..Default::default() });
    let own = |sequence: u32| CalEvent { organizer: riley.clone(), ..meeting("Mix review", sequence) };
    in_calendar(&mut s, WORK, &format!("{WORK}review.ics"), own(2));
    put(&mut s, "INBOX", 1, ("Riley", "riley@studio.example"), "Mix review", "REQUEST", &[own(1)], NOW);
    put(&mut s, "INBOX", 2, ("Riley", "riley@studio.example"), "Mix review", "REQUEST", &[own(2)], NOW);
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    let r = s.run("event.respond", &json!({"uid": own(2).uid, "answer": "accepted"})).unwrap();
    assert_eq!(r["moved"], 0, "{r}");
    assert!(listed(&s, 1) && listed(&s, 2), "the user's own meeting is left alone");
    done(s);
}

/// On Exchange a colleague's meeting comes with no organizer (an internal address), and its
/// appointment is kept as `UID#start`: its invitations are found by the title.
#[test]
fn an_exchange_colleagues_meeting_is_tidied_by_its_title() {
    let mut s = sample_session("answered-exchange");
    if let Some(a) = s.accounts.iter_mut().find(|a| a.config.id == "sample") {
        a.config.ews = Some("https://mail.studio.example/EWS/Exchange.asmx".into());
    }
    const PAT: (&str, &str) = ("Pat Kim", "pat@contoso.example");
    let budget = |sequence: u32| CalEvent {
        uid: "budget@contoso.example".into(),
        summary: "Budget review".into(),
        start: NOW + 86_400,
        end: NOW + 86_400 + 3600,
        organizer: Some(Attendee { email: PAT.1.into(), name: PAT.0.into(), ..Default::default() }),
        attendees: vec![me("NEEDS-ACTION")],
        sequence,
        ..Default::default()
    };
    let appointment = CalEvent { uid: format!("budget@contoso.example#{}", NOW + 86_400), organizer: None, ..budget(2) };
    in_calendar(&mut s, "calendar", "AAMk-budget", appointment.clone());
    put(&mut s, "INBOX", 1, PAT, "Budget review", "REQUEST", &[budget(1)], NOW);
    put(&mut s, "INBOX", 2, PAT, "Budget review", "REQUEST", &[budget(2)], NOW);
    put(&mut s, "INBOX", 3, PAT, "Budget review", "REQUEST", &[budget(3)], NOW);
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    let r = s.run("event.respond", &json!({"uid": appointment.uid, "answer": "accepted"})).unwrap();
    assert_eq!(r["byServer"], true, "{r}");
    assert_eq!(r["moved"], 2, "{r}");
    assert!(!listed(&s, 1) && !listed(&s, 2), "out of the Inbox");
    assert!(listed(&s, 3), "the newer update stays");
    done(s);
}

#[test]
fn removing_a_cancelled_meeting_takes_the_cancellation_and_its_invitations() {
    let mut s = sample_session("answered-cancelled");
    in_calendar(&mut s, WORK, &format!("{WORK}review.ics"), meeting("Mix review", 1));
    put(&mut s, "INBOX", 1, ANA, "Mix review", "REQUEST", &[meeting("Mix review", 1)], NOW);
    // Sent again after the cancellation: stays, to be answered.
    put(&mut s, "INBOX", 3, ANA, "Mix review", "REQUEST", &[meeting("Mix review", 3)], NOW);
    let cancel = CalEvent { status: "CANCELLED".into(), ..meeting("Mix review", 2) };
    deliver(&mut s, 2, ANA, "CANCEL", &[cancel], NOW);
    let r = s.run("invite.apply", &json!({})).unwrap();
    assert_eq!(r["removed"], true, "{r}");
    assert_eq!(r["moved"], 2, "{r}");
    let t = trash(&s);
    let gone = ids(&mut s, &t);
    assert!(gone.contains(&"i1@x".to_string()) && gone.contains(&"i2@x".to_string()), "{gone:?}");
    assert!(!listed(&s, 1) && !listed(&s, 2), "the cancellation and the first invitation go");
    assert!(listed(&s, 3), "the later invitation stays");
    done(s);
}
