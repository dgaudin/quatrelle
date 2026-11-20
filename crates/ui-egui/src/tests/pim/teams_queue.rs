// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: the queue that takes Teams' events a few a frame, bounded, keeping what the worker
//! doesn't say again; a meeting's answers never wait in it (D17).

use quatrelle_engine::model::pim::teams::{self, Status};

use super::conversations::{chats, row};
use super::*;

/// The queue never holds more than 10,000: the oldest rows and messages go (the worker sends
/// them again), a status stays.
#[test]
fn the_teams_queue_is_bounded_and_keeps_a_status() {
    let mut q = crate::conversations::TeamsQueue::default();
    let mut events = vec![MailEvent::Teams { event: teams::Event::Status { status: Status::UpToDate } }];
    events.extend((0..12_000).map(|i| chats(vec![row(&format!("19:q{i}@thread.v2"), 0)])));
    events.push(MailEvent::Quota { account: "sample".into(), used: 1, limit: 2 });
    q.take(&mut events);
    assert_eq!(events.len(), 1, "the mail's event stays to be applied");
    assert_eq!(q.len(), 10_000);
    assert!(q.holds(|e| matches!(e, teams::Event::Status { status: Status::UpToDate })), "the status is kept");
    assert!(q.holds(|e| matches!(e, teams::Event::Chats { rows } if rows[0].id == "19:q11999@thread.v2")), "the newest kept");
    assert!(!q.holds(|e| matches!(e, teams::Event::Chats { rows } if rows[0].id == "19:q0@thread.v2")), "the oldest gone");
}

/// Past 10,000 waiting, who signed in, a copy set aside, the open chat's members and a chat left
/// are never dropped: the worker doesn't say them again.
#[test]
fn the_teams_queue_never_drops_what_isnt_said_again() {
    let mut q = crate::conversations::TeamsQueue::default();
    let account = teams::Event::Account {
        name: "Riley Moreno".into(),
        address: "riley@contoso.example".into(),
        object_id: "00000000-0000-0000-0000-00000000000b".into(),
        tenant_id: "contoso.example".into(),
        drive_host: None,
    };
    let mut events: Vec<MailEvent> = [
        account,
        teams::Event::SetAside { path: "/data/Teams-0a.quatrelle.old".into() },
        teams::Event::Members { chat: "19:g@thread.v2".into(), members: vec![], web_url: None, first: true },
        teams::Event::Removed { chat: "19:g@thread.v2".into() },
    ]
    .into_iter()
    .map(|event| MailEvent::Teams { event })
    .collect();
    events.extend(
        (0..12_000).map(|i| MailEvent::Teams { event: teams::Event::Presence { presences: vec![(format!("u-{i}"), teams::Presence::Busy)] } }),
    );
    q.take(&mut events);
    assert!(q.len() <= 10_004, "{}", q.len());
    assert!(q.holds(|e| matches!(e, teams::Event::Account { .. })), "who signed in");
    assert!(q.holds(|e| matches!(e, teams::Event::SetAside { .. })), "the copy set aside");
    assert!(q.holds(|e| matches!(e, teams::Event::Members { .. })), "the members");
    assert!(q.holds(|e| matches!(e, teams::Event::Removed { .. })), "the chat left");
}

/// A Teams meeting's answers never wait behind a first copy, and are never dropped: they stay
/// with the mail's events, applied at once. Whether meetings are allowed waits in its place, and
/// stays past 10,000.
#[test]
fn a_meeting_answer_skips_the_teams_queue() {
    let mut q = crate::conversations::TeamsQueue::default();
    let mut events: Vec<MailEvent> = (0..12_000).map(|i| chats(vec![row(&format!("19:q{i}@thread.v2"), 0)])).collect();
    events.extend(
        [
            teams::Event::MeetingCreated { ask: 7, meeting: None, error: Some(teams::TeamsMeetingError::Offline) },
            teams::Event::MeetingUpdated { id: "MSpkYzE3Njc0Yy04MWQ5".into(), meeting: None, error: None },
            teams::Event::MeetingDeleted { id: "MSpkYzE3Njc0Yy04MWQ5".into(), error: None },
            teams::Event::Meetings { allowed: true },
        ]
        .into_iter()
        .map(|event| MailEvent::Teams { event }),
    );
    events.push(MailEvent::Quota { account: "sample".into(), used: 1, limit: 2 });
    q.take(&mut events);
    assert_eq!(events.len(), 4, "the three answers and the mail's event stay to be applied");
    assert!(matches!(&events[0], MailEvent::Teams { event: teams::Event::MeetingCreated { ask: 7, .. } }));
    assert!(matches!(&events[1], MailEvent::Teams { event: teams::Event::MeetingUpdated { .. } }));
    assert!(matches!(&events[2], MailEvent::Teams { event: teams::Event::MeetingDeleted { .. } }));
    assert!(matches!(&events[3], MailEvent::Quota { .. }));
    assert_eq!(q.len(), 10_000);
    assert!(q.holds(|e| matches!(e, teams::Event::Meetings { allowed: true })), "whether meetings are allowed is kept");
    let answer = |e: &teams::Event| {
        matches!(e, teams::Event::MeetingCreated { .. } | teams::Event::MeetingUpdated { .. } | teams::Event::MeetingDeleted { .. })
    };
    assert!(!q.holds(answer));
}
