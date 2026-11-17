// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: a search's hit opened at its message (R38): `teams.open` with a message asks the
//! worker to find it; what it found is the window, newer messages left out when it stops short
//! of the newest; opened as any chat is (marked read, its news waiting not announced, its
//! messages refused kept); and the params checked first.

use quatrelle_model::pim::teams::{News, Op, OutKind, OutState, Outgoing, Unsent};

use super::*;
use crate::mime::teams_compose;

const CHAT: &str = "19:chat1@thread.v2";
const OTHER: &str = "19:chat2@thread.v2";

fn find(s: &mut Session, chat: &str, message: &str) {
    s.run("teams.open", &json!({"chat": chat, "message": message})).unwrap();
}

fn placed(s: &mut Session, chat: &str, message: &str, newest: bool) {
    s.apply(teams_event(teams::Event::Placed { chat: chat.into(), message: message.into(), newest }));
}

fn fresh(s: &mut Session, chat: &str, i: i64) {
    s.apply(teams_event(teams::Event::Messages { chat: chat.into(), messages: vec![msg(chat, i)], fresh: true, older: false }));
}

/// The worker's answer to a find: messages `range` (said as an opening says them).
fn found(s: &mut Session, chat: &str, range: std::ops::Range<i64>) {
    let all: Vec<ChatMessage> = range.map(|i| msg(chat, i)).collect();
    for part in all.chunks(teams::EVENT_ITEMS) {
        s.apply(teams_event(teams::Event::Messages { chat: chat.into(), messages: part.to_vec(), fresh: false, older: false }));
    }
}

#[test]
fn a_hit_opens_its_chat_at_the_message() {
    let mut s = signed_in(vec![row(1, 10), row(2, 20)]);
    find(&mut s, CHAT, "m340");
    assert_eq!(teams_jobs(&mut s), [teams::Job::Find { chat: CHAT.into(), message: "m340".into() }]);
    assert_eq!(s.teams.open.as_deref(), Some(CHAT));
    assert_eq!(s.teams.go_to.as_deref(), Some("m340"), "to scroll to once the window holds it");
    // Its page and the next, short of the newest: newer ones aren't taken (they'd follow a gap).
    found(&mut s, CHAT, 300..500);
    placed(&mut s, CHAT, "m340", false);
    assert!(s.teams.newer_left_out);
    fresh(&mut s, CHAT, 1000);
    assert_eq!(window_ids(&s), (300..500).collect::<Vec<_>>(), "a fresh message after it isn't taken");
    // Older pages come in above it as the user scrolls up.
    assert!(s.run("teams.older", &json!({"chat": CHAT})).unwrap()["asked"].as_bool().unwrap());
    assert_eq!(teams_jobs(&mut s), [teams::Job::Page { chat: CHAT.into(), before: msg(CHAT, 300).created }]);
    // Opened again as usual: at its newest, nothing to go to.
    open(&mut s, CHAT);
    assert_eq!(teams_jobs(&mut s), [teams::Job::Open { chat: Some(CHAT.into()) }]);
    assert!(!s.teams.newer_left_out && s.teams.go_to.is_none() && s.teams.window.is_empty());
}

#[test]
fn a_hit_that_reaches_the_newest_takes_what_comes_after() {
    let mut s = signed_in(vec![row(1, 10)]);
    find(&mut s, CHAT, "m950");
    found(&mut s, CHAT, 900..1000);
    placed(&mut s, CHAT, "m950", true);
    assert!(!s.teams.newer_left_out);
    fresh(&mut s, CHAT, 1000);
    assert_eq!(window_ids(&s).last(), Some(&1000));
    // Past the window's room, the newest end goes: the user is back in the history.
    found(&mut s, CHAT, 1001..1001 + WINDOW_MAX as i64);
    assert_eq!(s.teams.window.front().map(|m| m.id.as_str()), Some("m900"), "the message found stays");
    assert!(s.teams.newer_left_out);
}

#[test]
fn a_placed_for_another_chat_or_message_or_not_found_changes_nothing() {
    let mut s = signed_in(vec![row(1, 10), row(2, 20)]);
    find(&mut s, CHAT, "m340");
    found(&mut s, CHAT, 300..500);
    placed(&mut s, OTHER, "m340", false);
    placed(&mut s, CHAT, "m341", false);
    assert!(!s.teams.newer_left_out, "not this find's answer");
    assert_eq!(s.teams.go_to.as_deref(), Some("m340"));
    // The copy didn't have it: the worker sent the last pages, the chat opened as usual.
    let mut s = signed_in(vec![row(1, 10)]);
    find(&mut s, CHAT, "m5");
    found(&mut s, CHAT, 800..1000);
    placed(&mut s, CHAT, "m5", true);
    assert!(s.teams.go_to.is_none() && !s.teams.newer_left_out);
    found(&mut s, CHAT, 1000..1000 + WINDOW_MAX as i64);
    assert_eq!(window_ids(&s).last(), Some(&(999 + WINDOW_MAX as i64)), "the oldest go, as for any opening");
    assert!(!s.teams.newer_left_out);
    // Another chat opened meanwhile: the late answer is another chat's.
    let mut s = signed_in(vec![row(1, 10), row(2, 20)]);
    find(&mut s, CHAT, "m340");
    open(&mut s, OTHER);
    placed(&mut s, CHAT, "m340", false);
    assert!(!s.teams.newer_left_out && s.teams.go_to.is_none());
}

/// Sending from a chat opened at an older message opens it again at its newest first.
#[test]
fn sending_from_a_hit_opens_the_newest_first() {
    let mut s = signed_in(vec![row(1, 10)]);
    find(&mut s, CHAT, "m340");
    found(&mut s, CHAT, 300..500);
    placed(&mut s, CHAT, "m340", false);
    teams_jobs(&mut s);
    s.run("teams.send", &json!({"chat": CHAT, "text": "hi"})).unwrap();
    let jobs = teams_jobs(&mut s);
    assert!(matches!(jobs.as_slice(), [teams::Job::Open { chat: Some(c) }, teams::Job::Op { op: Op::Send { .. } }] if c == CHAT), "{jobs:?}");
    assert!(s.teams.go_to.is_none() && !s.teams.newer_left_out);
}

/// Opened by a hit as by the list: marked read when it has messages unread (not a chat left or
/// one Teams won't let be read), its news waiting not announced; and the same chat opened at a
/// message keeps the messages it refused.
#[test]
fn a_hit_opens_as_any_chat_does() {
    let mut unread = row(1, 10);
    unread.unread = 3;
    let mut left = row(3, 30);
    (left.unread, left.left) = (2, true);
    let mut closed = row(4, 40);
    (closed.unread, closed.unreadable) = (1, true);
    let mut s = signed_in(vec![unread, row(2, 20), left, closed]);
    s.module = Module::Conversations;
    find(&mut s, CHAT, "m1");
    assert_eq!(
        teams_jobs(&mut s),
        [teams::Job::Find { chat: CHAT.into(), message: "m1".into() }, teams::Job::Op { op: Op::MarkRead { chat: CHAT.into() } }]
    );
    for chat in ["19:chat3@thread.v2", "19:chat4@thread.v2"] {
        find(&mut s, chat, "m1");
        assert_eq!(teams_jobs(&mut s), [teams::Job::Find { chat: chat.into(), message: "m1".into() }], "{chat}: not marked read");
    }

    // A chat's news held, then opened by a hit: not announced.
    let t = 1_763_200_000;
    let mut s = signed_in(vec![row(1, 10), row(2, 20)]);
    s.set_clock(t, 0);
    s.module = Module::Conversations;
    open(&mut s, OTHER);
    let news = |message: &str| News {
        chat: CHAT.into(),
        message: message.into(),
        sender: "Marie Durand".into(),
        external: false,
        app: false,
        mentions_me: false,
        chat_name: Some("Support team".into()),
        direct: false,
        hidden: false,
        text: "Lunch at noon?".into(),
        at: t * 1_000,
    };
    s.apply(teams_event(teams::Event::News { news: vec![news("m1")] }));
    assert_eq!(s.take_teams_announcements().len(), 1);
    s.set_clock(t + 1, 0);
    s.apply(teams_event(teams::Event::News { news: vec![news("m2")] }));
    s.set_clock(t + 2, 0);
    find(&mut s, CHAT, "m2");
    s.set_clock(t + 5, 0);
    assert!(s.take_teams_announcements().is_empty(), "seen where it was");
    assert_eq!(s.teams_next_announcement(), None);

    // The chat open with a message it refused (known here alone), opened again at a message.
    let body = teams_compose("hi", &[], &[]);
    let refusal = Outgoing {
        seq: 0,
        local: "100-1".into(),
        chat: CHAT.into(),
        body,
        mentions: Vec::new(),
        what: OutKind::Send,
        state: OutState::NotSent { reason: Unsent::TooMany, code: None },
        parts: Vec::new(),
    };
    s.apply(teams_event(teams::Event::Outbox { chat: CHAT.into(), items: vec![refusal.clone()], took: Some("100-1".into()) }));
    assert_eq!(s.teams.outgoing, std::slice::from_ref(&refusal));
    find(&mut s, CHAT, "m1");
    s.apply(teams_event(teams::Event::Outbox { chat: CHAT.into(), items: vec![], took: None }));
    assert_eq!(s.teams.outgoing, [refusal], "kept");
}

#[test]
fn a_hits_params_are_checked_first() {
    let mut s = signed_in(vec![row(1, 10)]);
    open(&mut s, CHAT);
    teams_jobs(&mut s);
    let long = "m".repeat(teams::MAX_ID + 1);
    for bad in [
        json!({"chat": CHAT, "message": ""}),
        json!({"chat": CHAT, "message": long}),
        json!({"chat": CHAT, "message": 5}),
        json!({"chat": CHAT, "message": ["m1"]}),
        json!({"chat": null, "message": "m1"}),
        json!({"chat": "19:unlisted@thread.v2", "message": "m1"}),
    ] {
        assert!(matches!(s.run("teams.open", &bad), Err(CmdError::Params(_))), "{bad}");
        assert!(teams_jobs(&mut s).is_empty(), "{bad}: nothing asked");
        assert!(s.teams.go_to.is_none(), "{bad}");
    }
    // At most an id's length, and a message given as null is none.
    find(&mut s, CHAT, &"m".repeat(teams::MAX_ID));
    assert!(matches!(teams_jobs(&mut s).as_slice(), [teams::Job::Find { .. }]));
    s.run("teams.open", &json!({"chat": CHAT, "message": null})).unwrap();
    assert_eq!(teams_jobs(&mut s), [teams::Job::Open { chat: Some(CHAT.into()) }]);
}

/// Review (Task 12, Important 1): each opening at a message has a number of its own, the same hit
/// opened again included, never given twice in a run (Teams switched off between): the window
/// goes to the message once for each.
#[test]
fn each_opening_at_a_message_is_numbered_afresh() {
    let mut s = signed_in(vec![row(1, 10)]);
    let mut seen = Vec::new();
    for _ in 0..2 {
        find(&mut s, CHAT, "m340");
        seen.push(s.teams.go_to_opening);
    }
    s.run("teams.settings", &json!({"on": false})).unwrap();
    s.run("teams.settings", &json!({"on": true})).unwrap();
    s.apply(teams_event(teams::Event::Chats { rows: vec![row(1, 10)] }));
    find(&mut s, CHAT, "m340");
    seen.push(s.teams.go_to_opening);
    assert!(seen.windows(2).all(|w| w[0] < w[1]), "each afresh: {seen:?}");
}
