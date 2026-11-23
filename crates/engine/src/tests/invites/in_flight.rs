// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: a meeting added to the calendar by accepting an invitation, then taken out by
//! the organizer's cancellation (or declined again) while the sync that sends it to the server is
//! on its way. The server makes it, the sync reads it back: it stays out, and goes from the server
//! as any other. One cancelled before any sync read it never goes.

use quatrelle_model::pim::{PIM_DOC, Pim};

use super::*;

const WORK: &str = "https://dav.example.org/cal/work/";
const MADE: &str = "https://dav.example.org/cal/work/made.ics";

fn invitation(sequence: u32) -> CalEvent {
    CalEvent {
        uid: "review@soundlab.example".into(),
        summary: "Mix review".into(),
        start: NOW + 86_400,
        end: NOW + 86_400 + 3600,
        organizer: Some(Attendee { email: ANA.1.into(), name: ANA.0.into(), ..Default::default() }),
        attendees: vec![me("NEEDS-ACTION")],
        sequence,
        ..Default::default()
    }
}

/// A sync's result: the calendar read with `read` in it, and `made` made on the server.
fn result(state: &str, read: Vec<StoredEvent>, made: Vec<StoredEvent>, deleted: Vec<String>) -> DavSync {
    DavSync {
        caldav_home: Some("https://dav.example.org/cal/".into()),
        calendars: Some(vec![CalendarInfo { id: WORK.into(), name: "Work".into(), ctag: state.into(), ..Default::default() }]),
        synced_calendars: vec![WORK.into()],
        events: read,
        pushed_events: made,
        deleted,
        checked: NOW,
        ..Default::default()
    }
}

fn as_saved(s: &Session) -> Pim {
    s.store.as_ref().unwrap().load_account_doc("sample", PIM_DOC).unwrap()
}

/// The invitation accepted, in the server's calendar and still to send; what a sync that starts
/// now makes on the server and reads back.
fn accepted(s: &mut Session) -> (Vec<StoredEvent>, Vec<StoredEvent>) {
    s.apply(Event::DavSynced { account: "sample".into(), sync: Box::new(result("state-1", Vec::new(), Vec::new(), Vec::new())) });
    deliver(s, 1, ANA, "REQUEST", &[invitation(1)], NOW);
    s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    let saved = as_saved(s);
    let mine: Vec<&StoredEvent> = saved.events.iter().filter(|e| e.event.uid == invitation(1).uid).collect();
    assert!(mine.len() == 1 && mine[0].dirty && mine[0].href.is_empty() && mine[0].calendar == WORK, "{mine:?}");
    let made = StoredEvent { href: MADE.into(), etag: "\"1\"".into(), dirty: false, ..mine[0].clone() };
    let read = StoredEvent { respond: None, ..made.clone() };
    (vec![made], vec![read])
}

fn kept(s: &Session) -> usize {
    parts(s, &invitation(1).uid).len()
}

/// Cancelled by the organizer, or declined from a newer invitation: either takes the meeting out.
fn taken_out(s: &mut Session, how: &str) {
    match how {
        "cancel" => {
            let cancel = CalEvent { status: "CANCELLED".into(), ..invitation(2) };
            deliver(s, 2, ANA, "CANCEL", &[cancel], NOW);
            s.run("invite.apply", &json!({})).unwrap();
        }
        _ => {
            deliver(s, 2, ANA, "REQUEST", &[invitation(2)], NOW);
            s.run("invite.respond", &json!({"answer": "declined"})).unwrap();
        }
    }
}

fn taken_out_while_it_went_up(how: &str, name: &str) {
    let mut s = sample_session(name);
    let (made, read) = accepted(&mut s);
    // The sync starts; the meeting is taken out meanwhile.
    taken_out(&mut s, how);
    assert_eq!(kept(&s), 0);
    s.take_jobs();
    s.apply(Event::DavSynced { account: "sample".into(), sync: Box::new(result("state-2", read.clone(), made.clone(), Vec::new())) });
    assert_eq!(kept(&s), 0, "{how}: the meeting taken out here doesn't come back");
    // Nor with the result of another sync that read it before.
    s.apply(Event::DavSynced { account: "sample".into(), sync: Box::new(result("state-2", read, made, Vec::new())) });
    assert_eq!(kept(&s), 0, "{how}: nor with a later result");
    // Deleted on the server by the next sync.
    assert_eq!(as_saved(&s).deleted.iter().map(|t| t.href.as_str()).collect::<Vec<_>>(), [MADE], "{how}: its deletion goes");
    s.apply(Event::DavSynced { account: "sample".into(), sync: Box::new(result("state-3", Vec::new(), Vec::new(), vec![MADE.into()])) });
    assert!(kept(&s) == 0 && s.pim["sample"].deleted.is_empty() && s.pim["sample"].held.is_empty(), "{how}");
    done(s);
}

#[test]
fn a_meeting_cancelled_while_a_sync_made_it_stays_out_and_goes_from_the_server() {
    taken_out_while_it_went_up("cancel", "in-flight-cancel");
}

#[test]
fn a_meeting_declined_again_while_a_sync_made_it_stays_out_and_goes_from_the_server() {
    taken_out_while_it_went_up("decline", "in-flight-decline");
}

/// Taken out before any sync read it: it never goes, nothing comes back, nothing is deleted.
#[test]
fn a_meeting_cancelled_before_a_sync_read_it_never_goes() {
    let mut s = sample_session("in-flight-before");
    accepted(&mut s);
    taken_out(&mut s, "cancel");
    let saved = as_saved(&s);
    assert!(saved.events.iter().all(|e| e.event.uid != invitation(1).uid) && saved.deleted.is_empty());
    s.apply(Event::DavSynced { account: "sample".into(), sync: Box::new(result("state-2", Vec::new(), Vec::new(), Vec::new())) });
    assert!(kept(&s) == 0 && s.pim["sample"].deleted.is_empty());
    done(s);
}
