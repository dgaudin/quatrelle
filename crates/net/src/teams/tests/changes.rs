// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The worker's changes to messages already there, against the fake Graph, the worker driven by
//! hand: an edit as a PATCH of escaped HTML with its mentions, a deletion by the user's own path,
//! a reaction as its emoji, each written in the copy on Graph's answer; what waits merged (a
//! reaction taken back, a later edit, a deletion); in order with the sends; refused, unanswered,
//! cut off; let go when Teams shows the message deleted, or the chat is left; the caps; the
//! search; and what the log says.

use std::time::{Duration, Instant};

use quatrelle_mime::{Typed, teams_compose, teams_compose_typed};
use quatrelle_model::pim::teams::{self as model, ChatMessage, ChatText, Job, Op, OutKind, OutState, Outgoing, Reaction, Unsent};
use serde_json::json;

use super::sync::{NOW, OTHER, Run, STEP, run};
use super::worker::{AtDesk, Seen, desk, emit_into, grant, settings};
use super::{CHAT, ME, MINUTE, T0, TENANT, chat, teams_with};
use crate::fake_graph::rfc3339;
use crate::teams::outbox;
use crate::teams::store::{OpRecord, OpState};
use crate::teams::worker::Worker;

/// The user's first message of `chat(CHAT, T0, 4)`: `(T0 + MINUTE).to_string()`.
const OWN: &str = "1790812860000";
/// The user's second (`T0 + 3 min`).
const OWN_2: &str = "1790812980000";
/// Ana's first (`T0`).
const ANAS: &str = "1790812800000";
const THUMBS: &str = "\u{1F44D}";

fn text(t: &str) -> ChatText {
    teams_compose(t, &[], &[])
}

fn edit(chat: &str, local: &str, message: &str, created: i64, t: &str) -> Op {
    Op::Edit { chat: chat.into(), local: local.into(), message: message.into(), created, body: text(t), mentions: Vec::new() }
}

fn delete(chat: &str, local: &str, message: &str, created: i64) -> Op {
    Op::Delete { chat: chat.into(), local: local.into(), message: message.into(), created }
}

fn react(chat: &str, local: &str, message: &str, created: i64, emoji: &str, on: bool) -> Op {
    Op::React { chat: chat.into(), local: local.into(), message: message.into(), created, emoji: emoji.into(), on }
}

fn refused(code: &str) -> OpState {
    OpState::NotSent { reason: Unsent::Refused, code: Some(code.into()) }
}

impl Run {
    fn op(&mut self, op: Op) {
        self.w.job(Job::Op { op });
    }

    /// The routes of what was written to the fake, in order.
    fn routes(&self) -> Vec<String> {
        self.fake(|t| t.writes.iter().map(|w| w.0.clone()).collect())
    }

    /// A message as the copy holds it.
    fn kept(&self, chat: &str, id: &str, created: i64) -> ChatMessage {
        self.w.copy.as_ref().unwrap().message_at(chat, id, created).unwrap_or_else(|| panic!("{id} isn't in the copy"))
    }

    /// What a search of the copy finds: (chat, message).
    fn found(&mut self, query: &str) -> Vec<(String, String)> {
        self.w.job(Job::Search { query: query.into() });
        let found = self.events().into_iter().rev().find_map(|e| match e {
            model::Event::Found { hits, indexing, .. } => Some((hits, indexing)),
            _ => None,
        });
        let (hits, indexing) = found.unwrap_or_else(|| panic!("no search answered"));
        assert!(!indexing, "the index is loaded");
        hits
    }

    /// The reactions let go said in `chat`'s outboxes (`seq` 0, "Not sent"): one per outbox that
    /// holds one.
    fn let_go(&self, chat: &str) -> Vec<Outgoing> {
        let gone = |o: &Outgoing| o.seq == 0 && matches!(o.what, OutKind::React { .. }) && matches!(o.state, OutState::NotSent { .. });
        self.outboxes(chat).into_iter().filter_map(|(items, _)| items.into_iter().find(gone)).collect()
    }
}

/// A search's hit: (chat, message).
fn at(chat: &str, message: &str) -> (String, String) {
    (chat.to_string(), message.to_string())
}

/// Quatrelle started again: a new worker on `r`'s cache and keychain, going on from the sign-in
/// kept (the one before gone).
fn restart(r: Run) -> Run {
    let Run { w, server, creds, .. } = r;
    let store = w.store.clone();
    drop(w);
    let seen = Seen::default();
    let mut w = Worker::new(store, creds.clone(), emit_into(&seen), server.base.clone(), Box::new(AtDesk(desk(grant(Some(TENANT)))))).unwrap();
    w.job(Job::Start { settings: settings(ME) });
    Run { w, server, seen, t0: Instant::now(), at: Duration::ZERO, creds }
}

#[test]
fn an_edit_goes_as_a_patch_of_escaped_html_with_its_mentions() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    let typed = Typed { text: "<b>x</b> & Ana Lima".into(), mentions: vec![(11..19, "u-ana".into())], ..Typed::default() };
    let (body, mentions) = teams_compose_typed(&typed);
    r.w.job(Job::Op { op: Op::Edit { chat: CHAT.into(), local: "1-1".into(), message: OWN.into(), created: T0 + MINUTE, body, mentions } });
    r.turns(Duration::from_secs(3));
    let writes = r.fake(|t| t.writes.clone());
    assert_eq!(writes.len(), 1);
    assert_eq!((writes[0].0.as_str(), writes[0].2.as_str()), ("edit", OWN));
    assert_eq!(writes[0].3["body"]["content"], "<p>&lt;b&gt;x&lt;/b&gt; &amp; <at id=\"0\">Ana Lima</at></p>");
    assert_eq!(writes[0].3["mentions"][0]["mentioned"]["user"]["id"], "u-ana");
    assert!(r.outbox_said(CHAT).is_empty());
    let m = r.messages_said(CHAT).into_iter().rfind(|m| m.id == OWN).unwrap();
    assert!(m.edited.is_some());
    assert_eq!(m.body.plain(), "<b>x</b> & Ana Lima");
    // In the copy too, with its mention, at Graph's time.
    let kept = r.kept(CHAT, OWN, T0 + MINUTE);
    assert_eq!((kept.body.plain(), kept.mentions.len()), ("<b>x</b> & Ana Lima".to_string(), 1));
    assert!(kept.edited.is_some_and(|t| t >= NOW) && kept.modified >= NOW, "{kept:?}");
    assert!(r.ops().is_empty());
}

#[test]
fn a_deletion_goes_by_the_users_own_path_and_the_copy_keeps_nothing_of_it() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    assert!(r.found("Message 1").contains(&at(CHAT, OWN)), "its words are found before");
    r.op(delete(CHAT, "1-1", OWN, T0 + MINUTE));
    r.turns(Duration::from_secs(3));
    assert_eq!(r.routes(), ["softDelete"]);
    assert_eq!(r.fake(|t| (t.writes[0].1.clone(), t.writes[0].2.clone())), (CHAT.to_string(), OWN.to_string()));
    assert!(r.fake(|t| t.chat(CHAT).unwrap().messages.iter().any(|m| m.id == OWN && m.deleted.is_some())), "Teams deleted it");
    assert!(r.ops().is_empty());
    let kept = r.kept(CHAT, OWN, T0 + MINUTE);
    assert!(kept.deleted.is_some() && kept.body.blocks.is_empty(), "{kept:?}");
    assert!(!r.found("Message 1").contains(&at(CHAT, OWN)), "its words left the index");
    assert!(r.messages_said(CHAT).iter().any(|m| m.id == OWN && m.deleted.is_some()), "said deleted");
    // By the signed-in user's own id, not `/me`: under another's, Graph refuses it.
    r.w.settings.object_id = "u-1".into();
    r.op(delete(CHAT, "1-2", OWN_2, T0 + 3 * MINUTE));
    r.turns(Duration::from_secs(3));
    assert_eq!(r.routes(), ["softDelete", "softDelete"]);
    assert_eq!(r.ops()[0].state, refused("HTTP 403 Forbidden"));
}

#[test]
fn a_reaction_goes_as_its_emoji_and_comes_back_as_the_users() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.op(react(CHAT, "1-1", ANAS, T0, THUMBS, true));
    r.turns(Duration::from_secs(3));
    let writes = r.fake(|t| t.writes.clone());
    assert_eq!(writes.len(), 1);
    assert_eq!((writes[0].0.as_str(), writes[0].2.as_str()), ("setReaction", ANAS));
    assert_eq!(writes[0].3, json!({"reactionType": "👍"}));
    assert!(r.ops().is_empty());
    let mine = [Reaction { emoji: THUMBS.into(), users: vec![ME.into()] }];
    assert_eq!(r.kept(CHAT, ANAS, T0).reactions, mine);
    assert!(r.messages_said(CHAT).into_iter().rfind(|m| m.id == ANAS).is_some_and(|m| m.reactions == mine));
    // Read back from Teams: the user's.
    let reads = r.reads(CHAT);
    r.w.job(Job::Open { chat: Some(CHAT.into()) });
    r.turns(Duration::from_secs(4));
    assert!(r.reads(CHAT) > reads);
    assert_eq!(r.kept(CHAT, ANAS, T0).reactions, mine);
    r.op(react(CHAT, "1-2", ANAS, T0, THUMBS, false));
    r.turns(Duration::from_secs(3));
    assert_eq!(r.routes(), ["setReaction", "unsetReaction"]);
    assert_eq!(r.fake(|t| t.writes[1].3.clone()), json!({"reactionType": "👍"}));
    assert!(r.kept(CHAT, ANAS, T0).reactions.is_empty(), "none left");
    assert!(r.fake(|t| t.chat(CHAT).unwrap().messages[0].reactions.is_empty()));
}

/// Review Focus 1: reactions toggled while offline, then the network back.
#[test]
fn a_reaction_taken_back_before_it_went_cancels_it() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.w.work_offline(true);
    r.op(react(CHAT, "1-1", ANAS, T0, THUMBS, true));
    r.op(react(CHAT, "1-2", ANAS, T0, THUMBS, false));
    assert!(r.ops().is_empty());
    let took: Vec<Option<String>> = r.outboxes(CHAT).into_iter().map(|(_, took)| took).collect();
    assert!(took.contains(&Some("1-1".into())) && took.contains(&Some("1-2".into())), "both answered: {took:?}");
    assert!(r.outbox_said(CHAT).is_empty());
    r.w.work_offline(false);
    r.turns(Duration::from_secs(10));
    assert_eq!((r.hits("setReaction"), r.hits("unsetReaction")), (0, 0));
    assert!(r.ops().is_empty());
}

#[test]
fn the_same_reaction_twice_is_kept_once() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.w.work_offline(true);
    // With and without its selector: one reaction.
    r.op(react(CHAT, "1-1", ANAS, T0, "\u{2764}\u{FE0F}", true));
    r.op(react(CHAT, "1-2", ANAS, T0, "\u{2764}", true));
    let locals: Vec<String> = r.ops().into_iter().map(|o| o.local).collect();
    assert_eq!(locals, ["1-1"]);
    let took: Vec<Option<String>> = r.outboxes(CHAT).into_iter().map(|(_, took)| took).collect();
    assert_eq!(took.last(), Some(&Some("1-2".to_string())), "answered all the same");
    r.w.work_offline(false);
    r.turns(Duration::from_secs(10));
    assert_eq!(r.routes(), ["setReaction"]);
    assert_eq!(r.fake(|t| t.writes[0].3.clone()), json!({"reactionType": "\u{2764}\u{FE0F}"}));
    assert!(r.ops().is_empty());
}

#[test]
fn a_later_edit_replaces_one_waiting() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.w.work_offline(true);
    r.op(edit(CHAT, "1-1", OWN, T0 + MINUTE, "a"));
    r.op(edit(CHAT, "1-2", OWN, T0 + MINUTE, "b"));
    let ops = r.ops();
    assert_eq!(ops.len(), 1);
    assert_eq!((ops[0].local.as_str(), ops[0].body.plain()), ("1-2", "b".to_string()));
    r.w.work_offline(false);
    r.turns(Duration::from_secs(5));
    assert_eq!(r.routes(), ["edit"]);
    assert_eq!(r.fake(|t| t.writes[0].3["body"]["content"].clone()), "<p>b</p>");
}

#[test]
fn a_deletion_drops_the_waiting_changes_of_its_message() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.w.work_offline(true);
    r.op(edit(CHAT, "1-1", OWN, T0 + MINUTE, "Never seen"));
    r.op(react(CHAT, "1-2", OWN, T0 + MINUTE, THUMBS, true));
    r.op(delete(CHAT, "1-3", OWN, T0 + MINUTE));
    r.op(delete(CHAT, "1-4", OWN, T0 + MINUTE));
    let ops = r.ops();
    assert_eq!(ops.len(), 1, "{ops:?}");
    assert_eq!((ops[0].local.as_str(), &ops[0].what), ("1-3", &OutKind::Delete { message: OWN.into(), created: T0 + MINUTE }));
    let shown: Vec<String> = r.outbox_said(CHAT).into_iter().map(|o| o.local).collect();
    assert_eq!(shown, ["1-3"]);
    r.w.work_offline(false);
    r.turns(Duration::from_secs(5));
    assert_eq!(r.routes(), ["softDelete"]);
}

#[test]
fn changes_go_in_order_with_the_sends() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.send(CHAT, "1-1", "First");
    r.op(react(CHAT, "1-2", ANAS, T0, THUMBS, true));
    r.op(edit(CHAT, "1-3", OWN, T0 + MINUTE, "Second"));
    // Each write as it came: (the test's clock, its route).
    let mut seen: Vec<(Duration, String)> = Vec::new();
    for _ in 0..60 {
        r.turns(STEP);
        for w in r.fake(|t| t.writes.clone()).into_iter().skip(seen.len()) {
            seen.push((r.at, w.0));
        }
    }
    let routes: Vec<&str> = seen.iter().map(|(_, route)| route.as_str()).collect();
    assert_eq!(routes, ["post", "setReaction", "edit"], "{seen:?}");
    assert!(seen.windows(2).all(|w| w[1].0 - w[0].0 >= Duration::from_secs(1)), "a second apart: {seen:?}");
    assert!(r.ops().is_empty());
}

#[test]
fn a_refused_edit_is_not_edited_and_tried_again_only_when_asked() {
    let mut teams = teams_with(vec![chat(CHAT, T0, 4)]);
    teams.forbid_edit.push(CHAT.into());
    let mut r = run(teams);
    r.op(edit(CHAT, "1-1", OWN, T0 + MINUTE, "Not here"));
    r.turns(Duration::from_secs(60));
    assert_eq!(r.hits("edit"), 1);
    assert_eq!(r.ops()[0].state, refused("HTTP 403 Forbidden"));
    let shown = r.outbox_said(CHAT);
    assert_eq!(shown[0].state, OutState::NotSent { reason: Unsent::Refused, code: Some("HTTP 403 Forbidden".into()) });
    assert!(matches!(&shown[0].what, OutKind::Edit { message, .. } if message == OWN));
    assert_eq!(r.kept(CHAT, OWN, T0 + MINUTE).body.plain(), "Message 1", "not edited");
    let seq = r.ops()[0].seq;
    r.w.job(Job::Retry { seq });
    r.turns(Duration::from_secs(10));
    assert_eq!(r.hits("edit"), 2, "once more, as asked");
    assert_eq!(r.ops()[0].state, refused("HTTP 403 Forbidden"));
}

#[test]
fn an_edit_unanswered_is_sent_again_three_times_at_most() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.fake(|t| {
        t.fail_next.insert("edit".into(), 5);
    });
    r.op(edit(CHAT, "1-1", OWN, T0 + MINUTE, "Unanswered"));
    r.turns(Duration::from_secs(40));
    assert_eq!(r.hits("edit"), 3);
    assert_eq!(r.ops()[0].state, OpState::NotSent { reason: Unsent::Unconfirmed, code: None });
    assert_eq!(r.outbox_said(CHAT)[0].state, OutState::NotSent { reason: Unsent::Unconfirmed, code: None });
    r.turns(Duration::from_secs(60));
    assert_eq!(r.hits("edit"), 3, "never again on its own");

    // Made, its answer lost: made again, and done.
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.fake(|t| {
        t.answer_lost.insert("edit".into(), 1);
    });
    r.op(edit(CHAT, "1-1", OWN, T0 + MINUTE, "Answer lost"));
    r.turns(Duration::from_secs(2));
    assert_eq!((r.hits("edit"), r.ops()[0].state.clone(), r.ops()[0].tries), (1, OpState::Waiting, 1));
    r.turns(Duration::from_secs(13));
    assert_eq!(r.hits("edit"), 2);
    assert!(r.ops().is_empty());
    assert_eq!(r.kept(CHAT, OWN, T0 + MINUTE).body.plain(), "Answer lost");
}

/// R16, R17: Graph asking to wait holds a change as it holds a send: nothing at all asked of
/// Teams for its `Retry-After` (a 429: not taken; a 503: maybe made), then it goes once more.
#[test]
fn graph_asking_to_wait_holds_a_change_for_its_retry_after() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.fake(|t| t.throttle_once = Some("setReaction"));
    r.op(react(CHAT, "1-1", ANAS, T0, THUMBS, true));
    for _ in 0..20 {
        if r.hits("setReaction") > 0 {
            break;
        }
        r.turns(STEP);
    }
    assert_eq!(r.hits("setReaction"), 1);
    assert_eq!((r.ops()[0].state.clone(), r.ops()[0].tries), (OpState::Waiting, 0), "not taken by Graph: it waits again");
    let asked = r.all_hits();
    r.turns(Duration::from_millis(900));
    assert_eq!(r.all_hits(), asked, "nothing at all asked during the Retry-After");
    r.turns(Duration::from_millis(500));
    assert_eq!(r.hits("setReaction"), 2);
    assert!(r.ops().is_empty());

    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.fake(|t| {
        t.answer_lost.insert("edit".into(), 1);
        t.lost_retry_after = Some(30);
    });
    r.op(edit(CHAT, "1-1", OWN, T0 + MINUTE, "Busy for a while"));
    r.turns(Duration::from_secs(1));
    assert_eq!((r.hits("edit"), r.ops()[0].tries), (1, 1));
    let asked = r.all_hits();
    r.turns(Duration::from_secs(28));
    assert_eq!(r.all_hits(), asked, "nothing at all asked for its Retry-After, not the worker's 10 s");
    r.turns(Duration::from_secs(3));
    assert_eq!(r.hits("edit"), 2);
    assert!(r.ops().is_empty());
}

#[test]
fn a_deletion_answered_404_is_done() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.fake(|t| t.chat_mut(CHAT).unwrap().messages.retain(|m| m.id != OWN));
    r.op(delete(CHAT, "1-1", OWN, T0 + MINUTE));
    r.turns(Duration::from_secs(3));
    assert_eq!(r.hits("softDelete"), 1);
    assert!(r.ops().is_empty());
    assert!(r.outbox_said(CHAT).is_empty());
    assert!(r.kept(CHAT, OWN, T0 + MINUTE).deleted.is_some());
    r.turns(Duration::from_secs(30));
    assert_eq!(r.hits("softDelete"), 1);
}

#[test]
fn a_refused_reaction_leaves_and_is_said_once() {
    let mut teams = teams_with(vec![chat(CHAT, T0, 4)]);
    teams.forbid_react.push(CHAT.into());
    let mut r = run(teams);
    r.op(react(CHAT, "1-1", ANAS, T0, THUMBS, true));
    r.turns(Duration::from_secs(30));
    assert_eq!(r.hits("setReaction"), 1);
    assert!(r.ops().is_empty());
    let gone = r.let_go(CHAT);
    assert_eq!(gone.len(), 1, "said once: {:?}", r.outboxes(CHAT));
    assert_eq!(gone[0].local, "1-1");
    assert_eq!(gone[0].state, OutState::NotSent { reason: Unsent::Refused, code: Some("HTTP 403 Forbidden".into()) });
    assert!(r.outbox_said(CHAT).iter().all(|o| o.seq == 0), "never kept");
    assert!(r.kept(CHAT, ANAS, T0).reactions.is_empty());
}

#[test]
fn a_change_left_sending_by_a_run_cut_off_goes_again() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.w.work_offline(true);
    r.op(edit(CHAT, "1-1", OWN, T0 + MINUTE, "Cut off"));
    r.op(react(CHAT, "1-2", ANAS, T0, THUMBS, true));
    let ops = r.ops();
    let copy = r.w.copy.as_ref().unwrap();
    copy.put_op(&OpRecord { state: OpState::Sending, ..ops[0].clone() }).unwrap();
    copy.put_op(&OpRecord { state: OpState::Sending, tries: 2, ..ops[1].clone() }).unwrap();
    let mut r = restart(r);
    let ops = r.ops();
    assert_eq!(ops.len(), 1, "{ops:?}");
    assert!(matches!(ops[0].what, OutKind::Edit { .. }));
    assert_eq!((ops[0].state.clone(), ops[0].tries), (OpState::Waiting, 1));
    let gone = r.let_go(CHAT);
    assert_eq!(gone.len(), 1, "the reaction, its third try cut off, said once");
    assert_eq!(gone[0].state, OutState::NotSent { reason: Unsent::Unconfirmed, code: None });
    assert_eq!(r.unsent(), Some(0), "changes aren't messages to send");
    r.turns(Duration::from_secs(15));
    assert_eq!((r.hits("edit"), r.hits("setReaction")), (1, 0));
    assert!(r.ops().is_empty());
    assert_eq!(r.kept(CHAT, OWN, T0 + MINUTE).body.plain(), "Cut off");

    // Cut off with no answer at all: it goes again after a wait.
    r.fake(|t| {
        t.cut_after.insert("edit".into(), 1);
    });
    r.op(edit(CHAT, "1-3", OWN, T0 + MINUTE, "Cut again"));
    r.turns(Duration::from_secs(2));
    assert_eq!(r.hits("edit"), 2);
    let ops = r.ops();
    assert_eq!((ops[0].state.clone(), ops[0].tries), (OpState::Waiting, 1));
    r.turns(Duration::from_secs(13));
    assert_eq!(r.hits("edit"), 3);
    assert!(r.ops().is_empty());
}

/// Review Focus 2: the message deleted in Teams (on the phone) while an edit of it was refused, or
/// a reaction of it waits.
#[test]
fn a_message_deleted_in_teams_takes_its_changes() {
    let mut teams = teams_with(vec![chat(CHAT, T0, 4)]);
    teams.forbid_edit.push(CHAT.into());
    let mut r = run(teams);
    r.op(edit(CHAT, "1-1", OWN, T0 + MINUTE, "Refused"));
    r.turns(Duration::from_secs(3));
    assert_eq!(r.ops()[0].state, refused("HTTP 403 Forbidden"));
    r.fake(|t| {
        let at = NOW + 10_000;
        t.clock = Some(at + 1_000);
        let m = t.chat_mut(CHAT).unwrap().messages.iter_mut().find(|m| m.id == OWN).unwrap();
        m.deleted = Some(rfc3339(at));
        m.modified = rfc3339(at);
    });
    let edits = r.hits("edit");
    r.w.job(Job::Open { chat: Some(CHAT.into()) });
    assert_eq!(r.outbox_said(CHAT).len(), 1, "shown not edited as the chat opens");
    r.turns(Duration::from_secs(5));
    assert!(r.kept(CHAT, OWN, T0 + MINUTE).deleted.is_some(), "read deleted");
    assert!(r.ops().is_empty());
    assert!(r.outbox_said(CHAT).is_empty(), "nothing lingers under it");
    assert_eq!(r.hits("edit"), edits, "no request for it");

    // Waiting offline, as the read lets go of it; one being made stays.
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.w.work_offline(true);
    r.op(react(CHAT, "1-1", OWN, T0 + MINUTE, THUMBS, true));
    r.op(react(CHAT, "1-2", ANAS, T0, THUMBS, true));
    r.op(edit(CHAT, "1-3", OWN_2, T0 + 3 * MINUTE, "Being made"));
    let making = OpRecord { state: OpState::Sending, ..r.ops()[2].clone() };
    r.w.copy.as_ref().unwrap().put_op(&making).unwrap();
    let said = r.outboxes(CHAT).len();
    outbox::gone(&mut r.w, CHAT, &[OWN, OWN_2]);
    let locals: Vec<String> = r.ops().into_iter().map(|o| o.local).collect();
    assert_eq!(locals, ["1-2", "1-3"]);
    assert_eq!(r.outboxes(CHAT).len(), said + 1);
    let shown: Vec<String> = r.outbox_said(CHAT).into_iter().map(|o| o.local).collect();
    assert_eq!(shown, ["1-2", "1-3"]);
    // Nothing to let go: nothing said.
    outbox::gone(&mut r.w, CHAT, &["1790812800001"]);
    outbox::gone(&mut r.w, OTHER, &[ANAS]);
    assert_eq!(r.outboxes(CHAT).len(), said + 1);
    assert_eq!(r.ops().len(), 2);
}

/// Review Focus 3: a reaction Teams lists by name (`like`), made in Teams.
#[test]
fn a_reaction_read_by_name_is_the_users_quick_reaction() {
    let mut c = chat(CHAT, T0, 4);
    c.messages[0].reactions.push((THUMBS.into(), ME.into()));
    let mut teams = teams_with(vec![c]);
    teams.reactions_by_name = true;
    let mut r = run(teams);
    assert_eq!(r.kept(CHAT, ANAS, T0).reactions, [Reaction { emoji: THUMBS.into(), users: vec![ME.into()] }]);
    r.op(react(CHAT, "1-1", ANAS, T0, THUMBS, false));
    r.turns(Duration::from_secs(3));
    let writes = r.fake(|t| t.writes.clone());
    assert_eq!(writes.len(), 1);
    assert_eq!((writes[0].0.as_str(), &writes[0].3), ("unsetReaction", &json!({"reactionType": "👍"})));
    assert!(r.fake(|t| t.chat(CHAT).unwrap().messages[0].reactions.is_empty()), "the fake holds none");
    assert!(r.kept(CHAT, ANAS, T0).reactions.is_empty());
}

#[test]
fn changes_of_a_left_chat_are_not_made() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    r.w.work_offline(true);
    r.op(edit(CHAT, "1-1", OWN, T0 + MINUTE, "Too late"));
    r.op(react(CHAT, "1-2", ANAS, T0, THUMBS, true));
    outbox::left(&mut r.w, CHAT);
    let ops = r.ops();
    assert_eq!(ops.len(), 1);
    assert_eq!((ops[0].local.as_str(), &ops[0].state), ("1-1", &OpState::NotSent { reason: Unsent::Left, code: None }));
    let gone = r.let_go(CHAT);
    assert_eq!(gone.len(), 1, "the reaction said once");
    assert_eq!((gone[0].local.as_str(), &gone[0].state), ("1-2", &OutState::NotSent { reason: Unsent::Left, code: None }));
    r.w.work_offline(false);
    r.turns(Duration::from_secs(10));
    assert_eq!((r.hits("edit"), r.hits("setReaction")), (0, 0));
}

#[test]
fn changes_count_in_the_caps_not_in_unsent() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 60), chat(OTHER, T0, 3)]));
    r.w.work_offline(true);
    for i in 0..60 {
        let at = T0 + i * MINUTE;
        r.op(react(CHAT, &format!("1-{i}"), &at.to_string(), at, THUMBS, true));
    }
    assert_eq!(r.ops().len(), 50);
    let too_many = OutState::NotSent { reason: Unsent::TooMany, code: None };
    let refused: Vec<(Vec<Outgoing>, Option<String>)> = r
        .outboxes(CHAT)
        .into_iter()
        .filter(|(items, _)| items.last().is_some_and(|o| o.seq == 0 && o.state == too_many && matches!(o.what, OutKind::React { .. })))
        .collect();
    assert_eq!(refused.len(), 10);
    assert!(refused.iter().all(|(items, took)| items.len() == 51 && took.as_deref() == items.last().map(|o| o.local.as_str())));
    assert_eq!(r.unsent(), Some(0));
    r.send(OTHER, "2-1", "Counted");
    assert_eq!(r.unsent(), Some(1));
    r.send(CHAT, "2-2", "Held by the changes");
    let (items, took) = r.outboxes(CHAT).pop().unwrap();
    assert_eq!(took.as_deref(), Some("2-2"));
    assert!(items.last().is_some_and(|o| o.seq == 0 && o.what == OutKind::Send && o.state == too_many), "{:?}", items.last());
    assert_eq!(r.unsent(), Some(1));
}

#[test]
fn an_edit_replaces_the_words_found() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 4)]));
    assert!(r.found("Message 1").contains(&at(CHAT, OWN)));
    r.op(edit(CHAT, "1-1", OWN, T0 + MINUTE, "zebra"));
    r.turns(Duration::from_secs(3));
    assert_eq!(r.found("zebra"), [at(CHAT, OWN)]);
    assert!(!r.found("Message 1").contains(&at(CHAT, OWN)));
}

#[test]
fn logs_carry_no_text_id_or_emoji_of_a_change() {
    const HIDDEN: &str = "19:quokka5520@thread.v2";
    let mut r = run(teams_with(vec![chat(HIDDEN, T0, 4)]));
    let tap = crate::fakes::logs::tap(|l| l.starts_with("teams: ") || l.starts_with("Teams copy: "));
    r.fake(|t| {
        t.fail_next.insert("edit".into(), 1);
        t.forbid_react.push(HIDDEN.into());
    });
    r.op(edit(HIDDEN, "1790000000-77", OWN, T0 + MINUTE, "Wallaby bilby totals"));
    r.op(react(HIDDEN, "1790000000-78", ANAS, T0, THUMBS, true));
    r.op(delete(HIDDEN, "1790000000-79", OWN_2, T0 + 3 * MINUTE));
    r.turns(Duration::from_secs(20));
    assert!(r.ops().is_empty(), "{:?}", r.ops());
    assert_eq!(r.routes(), ["edit", "edit", "setReaction", "softDelete"]);
    let lines = tap.lines();
    for (said, what) in [("HTTP 500", "a failure by its status"), ("HTTP 403", "a refusal by its status")] {
        assert!(lines.iter().any(|l| l.contains(said)), "{what}: {lines:?}");
    }
    for kind in ["an edit", "a reaction", "a deletion"] {
        assert!(lines.iter().any(|l| l.contains(kind)), "{kind} said: {lines:?}");
    }
    for line in &lines {
        let lower = line.to_lowercase();
        for private in ["quokka", "wallaby", "bilby", "1790000000", "@thread.v2", OWN, OWN_2, ANAS, THUMBS] {
            assert!(!lower.contains(private), "{private:?} in the log: {line}");
        }
    }
}
