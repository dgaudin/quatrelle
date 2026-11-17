// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A change whose request is always cut without an answer while the network works (a proxy
//! cutting one PATCH, a request timing out each time): held back, the worker going on with the
//! rest (another chat's send, the list, the open chat's reads), and given up after three tries;
//! with the network away, never a try.

use quatrelle_model::pim::teams::Status;

use super::*;

/// The last status the worker said.
fn status(r: &Run) -> Option<Status> {
    r.events().into_iter().rev().find_map(|e| match e {
        model::Event::Status { status } => Some(status),
        _ => None,
    })
}

fn chats() -> Vec<crate::fake_graph::FakeChat> {
    vec![chat(CHAT, T0, 4), chat(OTHER, T0, 2)]
}

#[test]
fn a_request_always_cut_while_the_network_works_is_given_up_after_three_tries() {
    let mut r = run(teams_with(chats()));
    r.fake(|t| {
        t.cut_after.insert("edit".into(), 1_000);
    });
    let lists = r.hits("chats");
    r.op(edit(CHAT, "1-1", OWN, T0 + MINUTE, "Never answered"));
    r.send(OTHER, "1-9", "still going");
    r.turns(Duration::from_secs(300));
    assert_eq!(r.hits("edit"), 4, "the first, which never counts, then three tries");
    let unconfirmed = OpState::NotSent { reason: Unsent::Unconfirmed, code: None };
    let states: Vec<(String, OpState)> = r.ops().into_iter().map(|o| (o.local, o.state)).collect();
    assert_eq!(states, [("1-1".to_string(), unconfirmed)]);
    assert_eq!(r.outbox_said(CHAT)[0].state, OutState::NotSent { reason: Unsent::Unconfirmed, code: None });
    assert_eq!((r.hits("post"), r.held(OTHER, "still going")), (1, 1), "another chat's send went meanwhile");
    assert!(r.hits("chats") >= lists + 20, "the list read meanwhile: {lists}, then {}", r.hits("chats"));
    assert_ne!(status(&r), Some(Status::Offline), "the network works");
}

#[test]
fn a_reaction_always_cut_is_let_go_and_said_once() {
    let mut r = run(teams_with(chats()));
    r.fake(|t| {
        t.cut_after.insert("setReaction".into(), 1_000);
    });
    r.op(react(CHAT, "1-1", ANAS, T0, THUMBS, true));
    r.send(OTHER, "1-9", "still going");
    r.turns(Duration::from_secs(300));
    assert_eq!(r.hits("setReaction"), 4, "the first, which never counts, then three tries");
    assert!(r.ops().is_empty(), "no record: {:?}", r.ops());
    let gone = r.let_go(CHAT);
    assert_eq!(gone.len(), 1, "said once: {:?}", r.outboxes(CHAT));
    assert_eq!((gone[0].local.as_str(), &gone[0].state), ("1-1", &OutState::NotSent { reason: Unsent::Unconfirmed, code: None }));
    assert_eq!(r.held(OTHER, "still going"), 1);
}

#[test]
fn a_change_held_back_doesnt_hold_its_chats_reads() {
    let mut r = run(teams_with(chats()));
    r.fake(|t| {
        t.cut_after.insert("edit".into(), 1_000);
    });
    r.w.job(Job::Open { chat: Some(CHAT.into()) });
    r.turns(Duration::from_secs(1));
    let reads = r.reads(CHAT);
    r.op(edit(CHAT, "1-1", OWN, T0 + MINUTE, "Never answered"));
    r.turns(Duration::from_secs(60));
    assert!(r.hits("edit") >= 2, "tried again: {}", r.hits("edit"));
    assert!(r.reads(CHAT) >= reads + 10, "the open chat read meanwhile: {reads}, then {}", r.reads(CHAT));
}

#[test]
fn with_the_network_away_a_change_waits_untried() {
    let mut r = run(teams_with(chats()));
    r.fake(|t| t.cut_all = true);
    r.op(edit(CHAT, "1-1", OWN, T0 + MINUTE, "Written offline"));
    r.turns(Duration::from_secs(120));
    let cuts = r.hits("edit");
    assert!(cuts >= 2, "tried while the network is away: {cuts}");
    let states: Vec<(OpState, u8)> = r.ops().into_iter().map(|o| (o.state, o.tries)).collect();
    assert_eq!(states, [(OpState::Waiting, 0)], "never a try");
    assert_eq!(status(&r), Some(Status::Offline));
    // The network back: made, at most a hold later.
    r.fake(|t| t.cut_all = false);
    r.turns(Duration::from_secs(25));
    assert_eq!(r.hits("edit"), cuts + 1);
    assert!(r.ops().is_empty(), "no record: {:?}", r.ops());
    assert_eq!(r.kept(CHAT, OWN, T0 + MINUTE).body.plain(), "Written offline");
}
