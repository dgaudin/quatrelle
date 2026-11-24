// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: an invitation whose uid holds characters a content line can't (a server stores
//! it without them, `ical::stored_uid`): the server's copy is read back under the uid kept here,
//! only for the item it lives in, an answer names the organizer's uid byte for byte, and one
//! deleted here never comes back.

use quatrelle_model::ical::{Attendee, CalEvent, stored_uid};
use quatrelle_model::pim::{CalendarInfo, StoredEvent};

use super::deleting::{DAV_CALENDAR, Server, as_saved, asked, result, session};
use super::*;

/// An invitation's uid with a control character in it.
const ODD: &str = "odd\u{1}uid\u{1b}-7@studio.example";

/// The other calendar of the fake CalDAV server.
const OTHER: &str = "https://dav.example.org/cal/other/";

/// Ana's invitation, to the account's user, under an odd uid.
fn invitation() -> CalEvent {
    CalEvent {
        uid: ODD.into(),
        summary: "Mix review".into(),
        start: NOW + 86_400,
        end: NOW + 86_400 + 3600,
        organizer: Some(Attendee { email: "ana@contoso.example".into(), name: "Ana Lima".into(), ..Default::default() }),
        attendees: vec![Attendee {
            email: "zoe@contoso.example".into(),
            name: "Zoe Park".into(),
            partstat: "NEEDS-ACTION".into(),
            rsvp: true,
            ..Default::default()
        }],
        sequence: 1,
        ..Default::default()
    }
}

/// The sync that makes `invitation()` on the CalDAV server: what it pushed (under the uid kept
/// here, at the address it made), and what it read back (the server stored it under `stored_uid`).
fn made_odd(href: &str) -> (StoredEvent, StoredEvent) {
    let pushed = StoredEvent {
        calendar: DAV_CALENDAR.into(),
        href: href.into(),
        etag: "ck-1".into(),
        event: invitation(),
        dirty: false,
        ..Default::default()
    };
    let read = StoredEvent {
        calendar: DAV_CALENDAR.into(),
        href: href.into(),
        etag: "ck-1".into(),
        event: CalEvent { uid: stored_uid(ODD), ..invitation() },
        ..Default::default()
    };
    (pushed, read)
}

/// The raw mails the session asked of the network since it was last looked at.
fn sent(s: &mut Session) -> Vec<String> {
    s.take_jobs()
        .iter()
        .filter_map(|j| match j {
            Job::Send { raw, .. } => Some(String::from_utf8_lossy(raw).into_owned()),
            _ => None,
        })
        .collect()
}

/// The account's events kept, by their address on the server.
fn at<'a>(s: &'a Session, id: &str, href: &str) -> Vec<&'a StoredEvent> {
    s.pim[id].events.iter().filter(|e| e.href == href).collect()
}

#[test]
fn an_odd_uid_invitation_made_on_the_server_is_read_back_under_it_and_answered_with_it() {
    let (mut s, id) = session("odd-uid-read", Server::CalDav);
    // The invitation, as a REQUEST put it in the calendar, never sent to the server.
    s.pim_mut(&id).events.push(StoredEvent { calendar: DAV_CALENDAR.into(), event: invitation(), dirty: true, ..Default::default() });
    s.save_pim(&id);
    // The sync makes it on the server and reads the calendar back.
    let href = format!("{DAV_CALENDAR}odd.ics");
    let (pushed, read) = made_odd(&href);
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(result(Server::CalDav, "state-2", vec![read], vec![pushed], Vec::new())) });
    // One event, under the uid kept here, at the address the server made it.
    let kept = at(&s, &id, &href);
    assert_eq!(kept.len(), 1, "one event at the address made: {:?}", s.pim[&id].events);
    assert_eq!(kept[0].event.uid, ODD, "under the uid kept here, not the server's stored one");
    assert!(!kept[0].dirty);
    // The answer names the uid the organizer looks for, byte for byte.
    s.run("event.respond", &json!({"uid": ODD, "answer": "accepted"})).unwrap();
    let mails = sent(&mut s);
    assert!(mails.iter().any(|m| m.contains(&format!("UID:{ODD}"))), "no answer carried the uid byte for byte: {mails:?}");
    // The answer goes to the server (the sync pushes it), and a later read has the meeting moved
    // to another place: still one event, the new place, the odd uid.
    let mut pushed_answer = s.pim[&id].events[0].clone();
    pushed_answer.dirty = false;
    pushed_answer.etag = "ck-2".into();
    let moved = StoredEvent {
        calendar: DAV_CALENDAR.into(),
        href: href.clone(),
        etag: "ck-2".into(),
        event: CalEvent { uid: stored_uid(ODD), location: "Room 2".into(), ..invitation() },
        ..Default::default()
    };
    s.apply(Event::DavSynced {
        account: id.clone(),
        sync: Box::new(result(Server::CalDav, "state-3", vec![moved], vec![pushed_answer], Vec::new())),
    });
    let kept = at(&s, &id, &href);
    assert_eq!(kept.len(), 1, "still one event: {:?}", s.pim[&id].events);
    assert_eq!(kept[0].event.uid, ODD);
    assert_eq!(kept[0].event.location, "Room 2", "the new place");
    done(s);
}

#[test]
fn an_odd_uid_invitation_deleted_here_stays_deleted_when_a_sync_read_it_before() {
    let (mut s, id) = session("odd-uid-tombstone", Server::CalDav);
    s.pim_mut(&id).events.push(StoredEvent { calendar: DAV_CALENDAR.into(), event: invitation(), dirty: true, ..Default::default() });
    s.save_pim(&id);
    let href = format!("{DAV_CALENDAR}odd.ics");
    let (pushed, read) = made_odd(&href);
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(result(Server::CalDav, "state-2", vec![read], vec![pushed], Vec::new())) });
    assert_eq!(at(&s, &id, &href).len(), 1);
    // Deleted here (it has its address now), then a sync that read the calendar before it was
    // deleted comes back with it.
    s.run("event.delete", &json!({"uid": ODD})).unwrap();
    assert!(s.pim[&id].events.is_empty());
    assert_eq!(asked(&mut s, &id).0, 1, "its deletion goes to the server");
    s.apply(Event::DavSynced {
        account: id.clone(),
        sync: Box::new(result(Server::CalDav, "state-3", vec![made_odd(&href).1], Vec::new(), Vec::new())),
    });
    assert!(s.pim[&id].events.is_empty(), "not brought back by what the sync read before: {:?}", s.pim[&id].events);
    let saved = as_saved(&s, &id);
    assert_eq!(saved.deleted.iter().map(|t| t.href.as_str()).collect::<Vec<_>>(), [href.as_str()], "its deletion stays to go");
    done(s);
}

#[test]
fn an_odd_uid_invitation_deleted_while_it_went_up_is_not_taken_back_and_goes_from_the_server() {
    let (mut s, id) = session("odd-uid-held", Server::CalDav);
    s.pim_mut(&id).events.push(StoredEvent { calendar: DAV_CALENDAR.into(), event: invitation(), dirty: true, ..Default::default() });
    s.save_pim(&id);
    // Deleted meanwhile, before the sync that makes it says where it made it.
    s.run("event.delete", &json!({"uid": ODD})).unwrap();
    assert!(s.pim[&id].events.is_empty());
    let href = format!("{DAV_CALENDAR}odd.ics");
    let (pushed, read) = made_odd(&href);
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(result(Server::CalDav, "state-2", vec![read], vec![pushed], Vec::new())) });
    assert!(s.pim[&id].events.is_empty(), "the one deleted while it went up isn't taken back: {:?}", s.pim[&id].events);
    let saved = as_saved(&s, &id);
    assert_eq!(saved.deleted.iter().map(|t| t.href.as_str()).collect::<Vec<_>>(), [href.as_str()], "deleted there by the next sync, as any other");
    assert_eq!(asked(&mut s, &id).0, 1, "a sync goes to delete it on the server");
    // Nor with the result of another sync that read it before.
    s.apply(Event::DavSynced {
        account: id.clone(),
        sync: Box::new(result(Server::CalDav, "state-3", vec![made_odd(&href).1], Vec::new(), Vec::new())),
    });
    assert!(s.pim[&id].events.is_empty());
    done(s);
}

#[test]
fn a_server_copy_is_renamed_to_the_odd_uid_only_for_the_item_it_lives_in() {
    let (mut s, id) = session("odd-uid-beside", Server::CalDav);
    // A real meeting on the server at address A, and an invitation never sent whose stored uid
    // is the real meeting's uid.
    let real = "abc@corp.example";
    let odd = "abc@corp.example\u{1}";
    assert_eq!(stored_uid(odd), real, "the invitation's stored uid is the real meeting's uid");
    let a = format!("{DAV_CALENDAR}real.ics");
    s.pim_mut(&id).events.push(StoredEvent {
        calendar: DAV_CALENDAR.into(),
        href: a.clone(),
        etag: "\"1\"".into(),
        event: CalEvent {
            uid: real.into(),
            summary: "Standup".into(),
            location: "Room 1".into(),
            start: NOW + 86_400,
            end: NOW + 86_400 + 1800,
            ..Default::default()
        },
        ..Default::default()
    });
    s.pim_mut(&id).events.push(StoredEvent {
        calendar: DAV_CALENDAR.into(),
        event: CalEvent { uid: odd.into(), summary: "Standup (invited)".into(), start: NOW + 86_400, end: NOW + 86_400 + 1800, ..Default::default() },
        dirty: true,
        ..Default::default()
    });
    s.save_pim(&id);
    // The sync makes the invitation in the other calendar, at address B, and reads both back
    // (the invitation under its stored uid, the real meeting's).
    let b = format!("{OTHER}inv.ics");
    let pushed = StoredEvent {
        calendar: OTHER.into(),
        href: b.clone(),
        etag: "ck-1".into(),
        event: CalEvent { uid: odd.into(), summary: "Standup (invited)".into(), start: NOW + 86_400, end: NOW + 86_400 + 1800, ..Default::default() },
        dirty: false,
        ..Default::default()
    };
    let read_b = StoredEvent {
        calendar: OTHER.into(),
        href: b.clone(),
        etag: "ck-1".into(),
        event: CalEvent {
            uid: stored_uid(odd),
            summary: "Standup (invited)".into(),
            start: NOW + 86_400,
            end: NOW + 86_400 + 1800,
            ..Default::default()
        },
        ..Default::default()
    };
    let read_a = StoredEvent {
        calendar: DAV_CALENDAR.into(),
        href: a.clone(),
        etag: "\"1\"".into(),
        event: CalEvent {
            uid: real.into(),
            summary: "Standup".into(),
            location: "Room 1".into(),
            start: NOW + 86_400,
            end: NOW + 86_400 + 1800,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut sync = result(Server::CalDav, "state-2", vec![read_a, read_b], vec![pushed], Vec::new());
    sync.calendars = Some(vec![
        CalendarInfo { id: DAV_CALENDAR.into(), name: "Calendar".into(), ctag: "state-2".into(), main: true, ..Default::default() },
        CalendarInfo { id: OTHER.into(), name: "Other".into(), ctag: "state-2".into(), ..Default::default() },
    ]);
    sync.synced_calendars = vec![DAV_CALENDAR.into(), OTHER.into()];
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(sync) });
    assert_eq!(at(&s, &id, &a).len(), 1);
    assert_eq!(at(&s, &id, &a)[0].event.uid, real);
    assert_eq!(at(&s, &id, &b).len(), 1);
    assert_eq!(at(&s, &id, &b)[0].event.uid, odd, "the invitation's copy under the uid kept here");
    // A reread of A with the meeting moved to another place: the real meeting keeps its uid and
    // takes the new place, the invitation keeps its own.
    let read_a2 = StoredEvent {
        event: CalEvent {
            uid: real.into(),
            summary: "Standup".into(),
            location: "Room 2".into(),
            start: NOW + 86_400,
            end: NOW + 86_400 + 1800,
            ..Default::default()
        },
        etag: "\"2\"".into(),
        ..read_a0(&a)
    };
    let mut sync = result(Server::CalDav, "state-3", vec![read_a2, made_odd_b(&b, odd)], Vec::new(), Vec::new());
    sync.calendars = Some(vec![
        CalendarInfo { id: DAV_CALENDAR.into(), name: "Calendar".into(), ctag: "state-3".into(), main: true, ..Default::default() },
        CalendarInfo { id: OTHER.into(), name: "Other".into(), ctag: "state-3".into(), ..Default::default() },
    ]);
    sync.synced_calendars = vec![DAV_CALENDAR.into(), OTHER.into()];
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(sync) });
    let events = &s.pim[&id].events;
    assert_eq!(events.len(), 2, "{events:?}");
    let at_a = at(&s, &id, &a);
    assert_eq!(at_a.len(), 1);
    assert_eq!(at_a[0].event.uid, real, "the real meeting's copy is not renamed to the invitation's uid");
    assert_eq!(at_a[0].event.location, "Room 2", "the new place");
    let at_b = at(&s, &id, &b);
    assert_eq!(at_b.len(), 1);
    assert_eq!(at_b[0].event.uid, odd, "the invitation keeps its own");
    done(s);
}

/// A stored event of calendar `DAV_CALENDAR` at `href`, with no event: to fill in.
fn read_a0(href: &str) -> StoredEvent {
    StoredEvent { calendar: DAV_CALENDAR.into(), href: href.into(), ..Default::default() }
}

/// The other calendar's copy of the invitation, as the server stored it.
fn made_odd_b(href: &str, odd: &str) -> StoredEvent {
    StoredEvent {
        calendar: OTHER.into(),
        href: href.into(),
        etag: "ck-1".into(),
        event: CalEvent {
            uid: stored_uid(odd),
            summary: "Standup (invited)".into(),
            start: NOW + 86_400,
            end: NOW + 86_400 + 1800,
            ..Default::default()
        },
        ..Default::default()
    }
}
