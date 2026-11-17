// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The worker's outbox against the fake Graph, the worker driven by hand: a message posted as
//! escaped HTML, once; each chat's in order, a second apart, and one chat never held behind
//! another; Graph asking to wait; a lost answer checked rather than posted again; a post not made
//! posted again three times at most; "Not sent" never sent again unasked; a run cut off checked
//! when it starts again; Delete and Retry by the state; a left chat; the caps; working offline;
//! signing out; another identity's outbox; marking read; and what the log says.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use quatrelle_mime::teams_compose;
use quatrelle_model::pim::teams::{self as model, ChatMessage, ChatText, Job, Op, OutKind, OutState, Outgoing, SEND_MAX, Unsent};
use serde_json::json;

use super::sync::{NOW, OTHER, Run, STEP, run, run_with};
use super::worker::{AtDesk, Desk, Seen, desk, emit_into, grant, settings};
use super::{CHAT, ME, MINUTE, T0, TENANT, chat, teams_with};
use crate::NetError;
use crate::teams::chat_of;
use crate::teams::graph::filter_time;
use crate::teams::limits::Cadence;
use crate::teams::outbox::answer_code;
use crate::teams::store::{ChatRecord, Copy, Identity, OpRecord, OpState, Opened};
use crate::teams::sync::rfc3339;
use crate::teams::worker::Worker;

fn body(text: &str) -> ChatText {
    teams_compose(text, &[], &[])
}

fn not_sent(reason: Unsent, code: Option<&str>) -> OpState {
    OpState::NotSent { reason, code: code.map(str::to_string) }
}

impl Run {
    /// The user sends `text` to `chat`, the engine calling the message `local`.
    pub(super) fn send(&mut self, chat: &str, local: &str, text: &str) {
        self.w.job(Job::Op { op: Op::Send { chat: chat.into(), local: local.into(), body: body(text), mentions: Vec::new() } });
    }

    /// The outbox as the copy holds it.
    pub(super) fn ops(&self) -> Vec<OpRecord> {
        self.w.copy.as_ref().map(Copy::ops).unwrap_or_default()
    }

    /// The outboxes said of a chat, in order: (items, took).
    pub(super) fn outboxes(&self, chat: &str) -> Vec<(Vec<Outgoing>, Option<String>)> {
        self.events()
            .into_iter()
            .filter_map(|e| match e {
                model::Event::Outbox { chat: c, items, took } if c == chat => Some((items, took)),
                _ => None,
            })
            .collect()
    }

    /// The items of the last outbox said of a chat.
    pub(super) fn outbox_said(&self, chat: &str) -> Vec<Outgoing> {
        self.outboxes(chat).pop().map(|(items, _)| items).unwrap_or_else(|| panic!("no outbox said of {chat}: {:?}", self.events()))
    }

    /// Every message said of a chat.
    pub(super) fn messages_said(&self, chat: &str) -> Vec<ChatMessage> {
        self.messages(chat, 0).into_iter().flat_map(|(m, _, _)| m).collect()
    }

    /// The last count of the outbox said.
    pub(super) fn unsent(&self) -> Option<u32> {
        self.events().into_iter().rev().find_map(|e| if let model::Event::Unsent { count } = e { Some(count) } else { None })
    }

    /// How many messages of a chat of the fake have `text` in their HTML.
    pub(super) fn held(&self, chat: &str, text: &str) -> usize {
        self.fake(|t| t.chat(chat).map_or(0, |c| c.messages.iter().filter(|m| m.body_html.contains(text)).count()))
    }

    /// Turns, a [`STEP`] apart, until `done`, for `span` at most: whether it came.
    fn turns_until(&mut self, span: Duration, done: impl Fn(&Run) -> bool) -> bool {
        let end = self.at + span;
        while self.at < end {
            if done(self) {
                return true;
            }
            self.turns(STEP);
        }
        done(self)
    }
}

/// Quatrelle started again: a new worker on `r`'s cache and keychain, going on from the sign-in
/// kept (the one before gone); `offline`: the network isn't there yet.
fn restart(r: Run, offline: bool) -> (Run, Arc<Desk>) {
    let Run { w, server, creds, .. } = r;
    let store = w.store.clone();
    drop(w);
    let (d, seen) = (desk(grant(Some(TENANT))), Seen::default());
    d.offline.store(offline, Ordering::SeqCst);
    let mut w = Worker::new(store, creds.clone(), emit_into(&seen), server.base.clone(), Box::new(AtDesk(d.clone()))).unwrap();
    w.job(Job::Start { settings: settings(ME) });
    (Run { w, server, seen, t0: Instant::now(), at: Duration::ZERO, creds }, d)
}

#[test]
fn a_message_is_sent_as_escaped_html_once() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 3)]));
    r.w.job(Job::Op {
        op: Op::Send {
            chat: CHAT.into(),
            local: "1-1".into(),
            body: teams_compose("<img src=\"https://evil.example/x.png\"> & hi", &[], &[]),
            mentions: Vec::new(),
        },
    });
    // Taken at once: said waiting, with the engine's id.
    let (items, took) = r.outboxes(CHAT).pop().unwrap();
    assert_eq!(took.as_deref(), Some("1-1"));
    assert!(matches!(items.as_slice(), [o] if o.seq >= 1 && o.local == "1-1" && o.state == OutState::Waiting), "{items:?}");
    assert_eq!(r.unsent(), Some(1));
    r.turns(Duration::from_secs(3));
    let posts = r.fake(|t| t.posts.clone());
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].1["body"]["content"], "<p>&lt;img src=&quot;https://evil.example/x.png&quot;&gt; &amp; hi</p>");
    assert!(r.outbox_said(CHAT).is_empty(), "the last snapshot holds nothing");
    assert!(r.messages_said(CHAT).iter().any(|m| m.body.plain().ends_with("& hi")));
    assert!(r.ops().is_empty());
    assert_eq!(r.unsent(), Some(0));
    // In the copy, as Graph made it.
    assert!(r.w.copy.as_ref().unwrap().last_pages(CHAT, 1).iter().any(|m| m.body.plain().ends_with("& hi")));
}

#[test]
fn sends_of_one_chat_go_in_order_a_second_apart() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 3), chat(OTHER, T0, 3)]));
    for i in 1..=3 {
        r.send(CHAT, &format!("1-{i}"), &format!("In order {i}"));
    }
    r.send(OTHER, "1-4", "Elsewhere");
    // Each post as it came: (the test's clock, the chat, the HTML).
    let mut seen: Vec<(Duration, String, String)> = Vec::new();
    for _ in 0..60 {
        r.turns(STEP);
        let posts = r.fake(|t| t.posts.clone());
        for (chat, body) in posts.iter().skip(seen.len()) {
            seen.push((r.at, chat.clone(), body["body"]["content"].as_str().unwrap_or_default().to_string()));
        }
    }
    let ours: Vec<&(Duration, String, String)> = seen.iter().filter(|(_, c, _)| c == CHAT).collect();
    let html: Vec<&str> = ours.iter().map(|(_, _, h)| h.as_str()).collect();
    assert_eq!(html, ["<p>In order 1</p>", "<p>In order 2</p>", "<p>In order 3</p>"], "{seen:?}");
    assert!(ours.windows(2).all(|w| w[1].0 - w[0].0 >= Duration::from_secs(1)), "a second apart: {seen:?}");
    let other = seen.iter().find(|(_, c, _)| c == OTHER).unwrap();
    assert!(other.0 < ours[1].0, "another chat's isn't held behind them: {seen:?}");
    assert!(r.ops().is_empty());
}

#[test]
fn a_429_on_a_send_waits_retry_after_and_asks_nothing_meanwhile() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 3)]));
    r.fake(|t| t.throttle_once = Some("post"));
    r.send(CHAT, "1-1", "Busy now");
    assert!(r.turns_until(Duration::from_secs(2), |r| r.hits("post") == 1));
    assert_eq!(r.ops()[0].state, OpState::Waiting, "not taken by Graph: it waits again");
    let asked = r.all_hits();
    r.turns(Duration::from_millis(900));
    assert_eq!(r.all_hits(), asked, "nothing at all asked of Teams during the Retry-After");
    r.turns(Duration::from_millis(500));
    assert_eq!(r.hits("post"), 2);
    r.turns(Duration::from_secs(10));
    assert_eq!(r.hits("post"), 2, "then exactly one more post");
    assert!(r.ops().is_empty());
    assert_eq!(r.held(CHAT, "Busy now"), 1);
}

/// I1: a 503 on a post (made, its answer lost) holds every request for its `Retry-After`, as a
/// 429 does — not the 10 s of a failure.
#[test]
fn a_503_on_a_send_honours_retry_after() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 3)]));
    r.fake(|t| {
        t.answer_lost.insert("post".into(), 1);
        t.lost_retry_after = Some(30);
    });
    r.send(CHAT, "1-1", "Busy for a while");
    assert!(r.turns_until(Duration::from_secs(2), |r| r.hits("post") == 1));
    assert_eq!(r.ops()[0].state, OpState::Unknown, "it may have gone");
    let asked = r.all_hits();
    r.turns(Duration::from_secs(29));
    assert_eq!(r.all_hits(), asked, "nothing at all asked of Teams for its Retry-After");
    assert!(r.turns_until(Duration::from_secs(3), |r| r.ops().is_empty()), "then checked: {:?}", r.ops());
    assert_eq!((r.hits("post"), r.held(CHAT, "Busy for a while")), (1, 1));
}

#[test]
fn a_send_whose_answer_is_lost_is_checked_not_sent_twice() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 3)]));
    r.fake(|t| {
        t.answer_lost.insert("post".into(), 1);
    });
    r.send(CHAT, "1-1", "Lost on the way");
    assert!(r.turns_until(Duration::from_secs(2), |r| r.hits("post") == 1));
    assert_eq!(r.ops()[0].state, OpState::Unknown);
    assert_eq!(r.outbox_said(CHAT)[0].state, OutState::Sending, "shown as being sent while it's checked");
    let reads = r.reads(CHAT);
    assert!(r.turns_until(Duration::from_secs(15), |r| r.ops().is_empty()), "checked: {:?}", r.ops());
    let checks: Vec<String> = r.fake(|t| t.message_reads.iter().filter(|(c, _)| c == CHAT).skip(reads).map(|(_, f)| f.clone()).collect());
    assert_eq!(checks.len(), 1, "{checks:?}");
    assert!(checks[0].starts_with("lastModifiedDateTime gt "), "{}", checks[0]);
    assert!(r.outbox_said(CHAT).is_empty());
    assert!(r.messages_said(CHAT).iter().any(|m| m.body.plain() == "Lost on the way"), "found, and shown");
    r.turns(Duration::from_secs(30));
    assert_eq!(r.hits("post"), 1);
    assert_eq!(r.held(CHAT, "Lost on the way"), 1);
}

/// A post answered 5xx (or not answered) may have been made: it is checked only after the
/// worker's pause, when Graph lists it more surely, not at the next turn.
#[test]
fn a_5xx_on_a_send_is_checked_after_a_wait() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 3)]));
    r.fake(|t| {
        t.fail_next.insert("post".into(), 1);
    });
    r.send(CHAT, "1-1", "After a wait");
    assert!(r.turns_until(Duration::from_secs(2), |r| r.hits("post") == 1));
    assert_eq!(r.ops()[0].state, OpState::Unknown);
    let (reads, asked) = (r.reads(CHAT), r.all_hits());
    r.turns(Duration::from_millis(9_500));
    assert_eq!((r.reads(CHAT), r.all_hits()), (reads, asked), "neither checked nor anything asked during the pause");
    assert!(r.turns_until(Duration::from_secs(3), |r| r.ops().is_empty()), "then checked, and posted again: {:?}", r.ops());
    assert_eq!((r.reads(CHAT) - reads, r.hits("post")), (1, 2));
    assert_eq!(r.held(CHAT, "After a wait"), 1);
}

/// Graph keeps a message's HTML its own way.
fn rewritten(html: &str) -> String {
    format!("<div>{}</div>", html.replace(" &amp; ", "&nbsp;&amp; "))
}

/// A message whose answer was lost is found by its text as Graph's reader reads it, whatever
/// HTML Graph made of it.
#[test]
fn a_check_finds_the_message_as_graph_rewrote_it() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 3)]));
    r.fake(|t| {
        t.answer_lost.insert("post".into(), 1);
        t.lost_retry_after = Some(1);
        t.rewrite_posts = Some(rewritten);
    });
    r.w.job(Job::Op {
        op: Op::Send {
            chat: CHAT.into(),
            local: "1-1".into(),
            body: teams_compose("a & b", std::slice::from_ref(&(4..5)), &[]),
            mentions: Vec::new(),
        },
    });
    assert!(r.turns_until(Duration::from_secs(2), |r| r.hits("post") == 1));
    assert_eq!(r.fake(|t| t.posts[0].1["body"]["content"].clone()), "<p>a &amp; <b>b</b></p>");
    let kept = r.fake(|t| t.chat(CHAT).and_then(|c| c.messages.last()).map(|m| m.body_html.clone()));
    assert_eq!(kept.as_deref(), Some("<div><p>a&nbsp;&amp; <b>b</b></p></div>"), "not the HTML sent");
    assert!(r.turns_until(Duration::from_secs(5), |r| r.ops().is_empty()), "found sent: {:?}", r.ops());
    r.turns(Duration::from_secs(10));
    assert_eq!(r.hits("post"), 1, "never posted again");
}

#[test]
fn a_send_not_made_is_sent_again_at_most_three_times() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 3)]));
    r.fake(|t| {
        t.fail_next.insert("post".into(), 5);
    });
    r.send(CHAT, "1-1", "Not made");
    // Each post not answered waits out the worker's pause before it's checked.
    r.turns(Duration::from_secs(40));
    assert_eq!(r.hits("post"), 3);
    let unconfirmed = not_sent(Unsent::Unconfirmed, None);
    assert_eq!(r.ops()[0].state, unconfirmed);
    assert_eq!(r.outbox_said(CHAT)[0].state, OutState::NotSent { reason: Unsent::Unconfirmed, code: None });
    r.turns(Duration::from_secs(60));
    assert_eq!(r.hits("post"), 3, "never again on its own");
    assert_eq!(r.held(CHAT, "Not made"), 0);
}

/// `n` messages of Ana's in a chat of the fake, a second apart, after the user's last post: what a
/// busy chat brings while a check waits (the computer asleep, say).
fn busy_after(r: &Run, chat: &str, n: i64) {
    for i in 1..=n {
        r.post(chat, "u-ana", NOW + i * 1_000, &format!("Meanwhile {i}"));
    }
}

/// The reads of a chat that check a message from `after` (its range begins 2 s before it).
fn check_reads(r: &Run, chat: &str, after: i64) -> usize {
    let filter = format!("lastModifiedDateTime gt {}", filter_time(&rfc3339(after - 2_000)).unwrap());
    r.fake(|t| t.message_reads.iter().filter(|(c, f)| c == chat && *f == filter).count())
}

/// A-I1: a check reads on past its first page, a page a step: 60 messages of others after the one
/// whose answer was lost, it's found on the second, and never posted again.
#[test]
fn a_check_reads_on_past_its_first_page() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 3)]));
    r.fake(|t| {
        t.answer_lost.insert("post".into(), 1);
        t.lost_retry_after = Some(1);
    });
    r.send(CHAT, "1-1", "Lost in a busy chat");
    assert!(r.turns_until(Duration::from_secs(2), |r| r.hits("post") == 1));
    busy_after(&r, CHAT, 60);
    assert!(r.turns_until(Duration::from_secs(15), |r| r.ops().is_empty()), "found sent: {:?}", r.ops());
    assert_eq!(check_reads(&r, CHAT, T0 + 2 * MINUTE), 2, "two pages read");
    r.turns(Duration::from_secs(10));
    assert_eq!((r.hits("post"), r.held(CHAT, "Lost in a busy chat")), (1, 1));
}

/// A-I1: more changed since than a check reads (250 messages, five pages): after its four pages
/// it can't tell, so the message is "Not sent" (couldn't be confirmed), never posted again.
#[test]
fn a_check_past_its_pages_is_not_sent_rather_than_posted_again() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 3)]));
    r.fake(|t| {
        t.answer_lost.insert("post".into(), 1);
        t.lost_retry_after = Some(1);
    });
    r.send(CHAT, "1-1", "Lost in a busier chat");
    assert!(r.turns_until(Duration::from_secs(2), |r| r.hits("post") == 1));
    busy_after(&r, CHAT, 250);
    let unconfirmed = not_sent(Unsent::Unconfirmed, None);
    assert!(r.turns_until(Duration::from_secs(20), |r| r.ops().first().is_some_and(|o| o.state == unconfirmed)), "{:?}", r.ops());
    assert_eq!(check_reads(&r, CHAT, T0 + 2 * MINUTE), 4, "four pages read, no more");
    assert_eq!(r.outbox_said(CHAT)[0].state, OutState::NotSent { reason: Unsent::Unconfirmed, code: None });
    r.turns(Duration::from_secs(30));
    assert_eq!((r.hits("post"), r.held(CHAT, "Lost in a busier chat")), (1, 1));
}

/// A-m2: a check Graph keeps failing (the chat's messages answer 500) waits 30 s, then twice as
/// long each time; the fifth failure running makes the message "Not sent" (couldn't be
/// confirmed), never posted again: the chat's next message goes, and Delete takes this one.
#[test]
fn a_check_that_keeps_failing_waits_longer_then_is_not_sent() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 3)]));
    r.fake(|t| {
        // Not made: the chat's last message stays, so only the checks read it.
        t.fail_next.insert("post".into(), 1);
        t.fail_reads.push(CHAT.into());
    });
    r.send(CHAT, "1-1", "Never confirmed");
    r.send(CHAT, "1-2", "Next in line");
    assert!(r.turns_until(Duration::from_secs(2), |r| r.hits("post") == 1));
    let unconfirmed = not_sent(Unsent::Unconfirmed, None);
    let (mut checks, mut reads) = (Vec::new(), r.reads(CHAT));
    while r.at < Duration::from_secs(600) && !r.ops().first().is_some_and(|o| o.state == unconfirmed) {
        r.turns(STEP);
        if r.reads(CHAT) > reads {
            reads = r.reads(CHAT);
            checks.push(r.at);
        }
    }
    let gaps: Vec<u64> = checks.windows(2).map(|w| (w[1] - w[0]).as_secs()).collect();
    assert_eq!(gaps, [30, 60, 120, 240], "five checks, each wait twice the last: {checks:?}");
    assert_eq!(r.ops()[0].state, unconfirmed);
    assert_eq!(r.outbox_said(CHAT)[0].state, OutState::NotSent { reason: Unsent::Unconfirmed, code: None });
    r.fake(|t| t.fail_reads.clear());
    assert!(r.turns_until(Duration::from_secs(3), |r| r.hits("post") == 2), "the chat's next goes");
    r.turns(Duration::from_secs(300));
    assert_eq!(r.hits("post"), 2, "and the one not sent never goes again on its own");
    assert_eq!((r.held(CHAT, "Never confirmed"), r.held(CHAT, "Next in line")), (0, 1));
    let seq = r.ops()[0].seq;
    r.w.job(Job::Discard { seq });
    assert!(r.ops().is_empty(), "Delete takes it");
}

/// A check never reads from past Graph's clock and an hour (a record read back with a time far
/// ahead: a damaged file, or a time Graph made up, chained). Nothing sent now can be told from
/// such a time: the message is "Not sent" (couldn't be confirmed), not posted again.
#[test]
fn a_check_from_a_time_ahead_of_graph_is_capped_and_posts_nothing_more() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 3)]));
    r.w.work_offline(true);
    r.send(CHAT, "1-1", "From ahead");
    let op = r.ops().remove(0);
    let ahead = NOW + 100 * 365 * 24 * 60 * MINUTE;
    r.w.copy.as_ref().unwrap().put_op(&OpRecord { after: Some(ahead), ..op }).unwrap();
    r.fake(|t| {
        t.answer_lost.insert("post".into(), 1);
        t.lost_retry_after = Some(1);
    });
    r.w.work_offline(false);
    let unconfirmed = not_sent(Unsent::Unconfirmed, None);
    assert!(r.turns_until(Duration::from_secs(15), |r| r.ops().first().is_some_and(|o| o.state == unconfirmed)), "{:?}", r.ops());
    assert_eq!(check_reads(&r, CHAT, NOW + 60 * MINUTE), 1, "read from Graph's time and an hour");
    r.turns(Duration::from_secs(20));
    assert_eq!((r.hits("post"), r.held(CHAT, "From ahead")), (1, 1));
}

#[test]
fn a_refused_send_is_not_sent_and_never_tried_again_on_its_own() {
    let mut teams = teams_with(vec![chat(CHAT, T0, 3)]);
    teams.forbid_post.push(CHAT.into());
    let mut r = run(teams);
    r.send(CHAT, "1-1", "Refused here");
    r.turns(Duration::from_secs(60));
    assert_eq!(r.hits("post"), 1);
    let forbidden = not_sent(Unsent::Refused, Some("HTTP 403 Forbidden"));
    assert_eq!(r.ops()[0].state, forbidden);
    assert_eq!(r.outbox_said(CHAT)[0].state, OutState::NotSent { reason: Unsent::Refused, code: Some("HTTP 403 Forbidden".into()) });
    let seq = r.ops()[0].seq;
    r.w.job(Job::Retry { seq });
    r.turns(Duration::from_secs(10));
    assert_eq!(r.hits("post"), 2, "once more, as asked");
    assert_eq!(r.ops()[0].state, forbidden);
}

/// Review Focus 1: the network drops while a message goes, Quatrelle is closed, then opened
/// offline: shown as being sent, checked once back online, in Teams exactly once.
#[test]
fn a_send_cut_off_then_a_restart_offline_is_checked_once_back_online() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 3)]));
    r.fake(|t| {
        t.cut_after.insert("post".into(), 1);
    });
    r.send(CHAT, "1-1", "Cut off on the way");
    assert!(r.turns_until(Duration::from_secs(2), |r| r.hits("post") == 1));
    // The record as a run cut off during the request leaves it.
    let op = r.ops().remove(0);
    r.w.copy.as_ref().unwrap().put_op(&OpRecord { state: OpState::Sending, ..op }).unwrap();
    let (mut r, network) = restart(r, true);
    assert_eq!(r.unsent(), Some(1));
    assert_eq!(r.ops()[0].state, OpState::Unknown, "a request cut off: to be checked");
    r.w.job(Job::Open { chat: Some(CHAT.into()) });
    let shown: Vec<OutState> = r.outbox_said(CHAT).into_iter().map(|o| o.state).collect();
    assert_eq!(shown, [OutState::Sending], "neither lost nor doubled");
    r.w.job(Job::Open { chat: None });
    let (reads, posts) = (r.reads(CHAT), r.hits("post"));
    r.turns(Duration::from_secs(5));
    assert_eq!((r.reads(CHAT), r.hits("post")), (reads, posts), "nothing asked while the network is away");
    network.offline.store(false, Ordering::SeqCst);
    assert!(r.turns_until(Duration::from_secs(15), |r| r.ops().is_empty()), "checked once back: {:?}", r.ops());
    assert_eq!(r.reads(CHAT) - reads, 1, "one check read");
    assert_eq!(r.hits("post"), 1);
    assert_eq!(r.held(CHAT, "Cut off on the way"), 1, "in Teams exactly once");
    assert!(r.outbox_said(CHAT).is_empty());
}

/// C4: a chat opened says its outbox, even empty: "Not sent" stays through a change of chat and a
/// restart.
#[test]
fn opening_a_chat_says_its_outbox() {
    let mut teams = teams_with(vec![chat(CHAT, T0, 3), chat(OTHER, T0, 3)]);
    teams.forbid_post.push(CHAT.into());
    let mut r = run(teams);
    r.send(CHAT, "1-1", "Stays not sent");
    r.turns(Duration::from_secs(3));
    let refused = OutState::NotSent { reason: Unsent::Refused, code: Some("HTTP 403 Forbidden".into()) };
    r.w.job(Job::Open { chat: Some(OTHER.into()) });
    assert!(matches!(r.events().last(), Some(model::Event::Outbox { chat, items, took: None }) if chat == OTHER && items.is_empty()));
    r.w.job(Job::Open { chat: Some(CHAT.into()) });
    assert!(matches!(r.events().last(), Some(model::Event::Outbox { chat, took: None, .. }) if chat == CHAT), "{:?}", r.events().last());
    let shown: Vec<OutState> = r.outbox_said(CHAT).into_iter().map(|o| o.state).collect();
    assert_eq!(shown, std::slice::from_ref(&refused));
    let (mut r, _) = restart(r, false);
    r.w.job(Job::Open { chat: Some(CHAT.into()) });
    let shown: Vec<(String, OutState)> = r.outbox_said(CHAT).into_iter().map(|o| (o.local, o.state)).collect();
    assert_eq!(shown, [("1-1".to_string(), refused)], "after a restart too");
}

/// Review Focus 5: Delete or Retry clicked while the message goes, or clicked twice.
#[test]
fn delete_and_retry_follow_the_state() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 3)]));
    r.w.work_offline(true);
    r.send(CHAT, "1-1", "First");
    let first = r.ops()[0].seq;
    // Retry of one waiting: nothing changes.
    let before = r.ops();
    r.w.job(Job::Retry { seq: first });
    assert_eq!(r.ops(), before);
    // Delete of one waiting: gone; again: nothing more.
    r.w.job(Job::Discard { seq: first });
    assert!(r.ops().is_empty());
    assert!(r.outbox_said(CHAT).is_empty());
    r.w.job(Job::Discard { seq: first });
    assert!(r.ops().is_empty());
    // Being checked (it may be in Teams already): Delete is refused.
    r.send(CHAT, "1-2", "Second");
    let put = |r: &Run, op: &OpRecord| r.w.copy.as_ref().unwrap().put_op(op).unwrap();
    let checked = OpRecord { state: OpState::Unknown, tries: 1, ..r.ops().remove(0) };
    put(&r, &checked);
    r.w.job(Job::Discard { seq: checked.seq });
    assert_eq!(r.ops(), std::slice::from_ref(&checked));
    assert_eq!(r.outbox_said(CHAT)[0].state, OutState::Sending, "still there, being sent");
    for state in [OpState::Unknown, OpState::Sending] {
        r.w.job(Job::Retry { seq: checked.seq });
        assert_eq!(r.ops()[0].state, state);
        put(&r, &OpRecord { state: OpState::Sending, ..checked.clone() });
    }
    // Not sent: Retry makes it wait again, its tries from none; twice: the same.
    put(&r, &OpRecord { state: not_sent(Unsent::Unconfirmed, None), tries: 3, ..checked.clone() });
    for _ in 0..2 {
        r.w.job(Job::Retry { seq: checked.seq });
        let again = r.ops().remove(0);
        assert_eq!((again.state, again.tries), (OpState::Waiting, 0));
    }
    // Not sent: Delete takes it.
    put(&r, &OpRecord { state: not_sent(Unsent::Refused, Some("HTTP 403 Forbidden")), ..checked.clone() });
    r.w.job(Job::Discard { seq: checked.seq });
    assert!(r.ops().is_empty());
    assert_eq!(r.hits("post"), 0, "nothing posted all along");
}

/// C3: a message being checked when its chat goes (gone from Teams, then from two whole lists):
/// not sent, said as left.
#[test]
fn ops_of_a_left_chat_are_not_sent() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 3), chat(OTHER, T0 + MINUTE, 2)]));
    r.fake(|t| {
        t.fail_next.insert("post".into(), 1);
    });
    r.send(CHAT, "1-1", "To a chat about to go");
    assert!(r.turns_until(Duration::from_secs(2), |r| r.hits("post") == 1));
    // Gone from Teams: its check fails, and the message stays to be checked.
    r.fake(|t| t.chats.retain(|c| c.id != CHAT));
    let reads = r.reads(CHAT);
    r.turns(Duration::from_secs(3));
    assert_eq!(r.ops()[0].state, OpState::Unknown);
    // Absent from two whole lists running: left.
    for _ in 0..2 {
        r.w.limiter.prompt(Cadence::Hourly);
        r.turns(Duration::from_secs(15));
    }
    assert!(r.row(CHAT).left);
    assert!(r.reads(CHAT) - reads <= 2, "a check that fails waits longer each time: {} reads", r.reads(CHAT) - reads);
    assert_eq!(r.ops()[0].state, not_sent(Unsent::Left, None));
    assert_eq!(r.outbox_said(CHAT)[0].state, OutState::NotSent { reason: Unsent::Left, code: None });
    assert_eq!(r.hits("post"), 1);
}

/// I2: a message that can't be taken is answered "Not sent", nothing of it kept.
#[test]
fn a_message_that_cant_be_taken_is_answered_not_sent() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 3), chat(OTHER, T0, 3)]));
    r.w.work_offline(true);
    let mut left = r.record(OTHER);
    left.chat.left = true;
    r.w.copy.as_ref().unwrap().put_chat(&left).unwrap();
    let long = "<".repeat(SEND_MAX / 4 + 1);
    for (chat, local, text, reason) in [
        (CHAT, "1-1", long.as_str(), Unsent::TooLong),
        (OTHER, "1-2", "To a chat left", Unsent::Left),
        ("19:unknown@thread.v2", "1-3", "To a chat not in the copy", Unsent::Left),
    ] {
        r.send(chat, local, text);
        let (items, took) = r.outboxes(chat).pop().unwrap();
        assert_eq!(took.as_deref(), Some(local));
        let item = Outgoing {
            seq: 0,
            local: local.into(),
            chat: chat.into(),
            body: body(text),
            mentions: Vec::new(),
            what: OutKind::Send,
            state: OutState::NotSent { reason, code: None },
        };
        assert_eq!(items, [item]);
    }
    assert!(r.ops().is_empty());
    assert_eq!(r.unsent(), Some(0));
    // Ids no engine makes: dropped, nothing said.
    let said = r.events().len();
    r.send("", "1-4", "No chat");
    r.send(CHAT, "Not-A-Local", "No local");
    r.send(CHAT, &"1".repeat(65), "Too long a local");
    assert_eq!(r.events().len(), said);
    assert!(r.ops().is_empty());
}

#[test]
fn the_outbox_is_bounded() {
    let id = |i: usize| format!("19:box{i}@thread.v2");
    let mut r = run(teams_with((0..11).map(|i| chat(&id(i), T0 + i as i64 * MINUTE, 1)).collect()));
    r.w.work_offline(true);
    for i in 0..60 {
        r.send(&id(0), &format!("1-{i}"), "Again");
    }
    assert_eq!(r.ops().len(), 50);
    // Each one past the cap answered, its item not sent, nothing kept.
    let answers = r.outboxes(&id(0));
    let refused: Vec<&(Vec<Outgoing>, Option<String>)> = answers
        .iter()
        .filter(|(items, _)| items.last().is_some_and(|o| o.seq == 0 && o.state == OutState::NotSent { reason: Unsent::TooMany, code: None }))
        .collect();
    assert_eq!(refused.len(), 10);
    assert!(refused.iter().all(|(items, took)| items.len() == 51 && took.as_deref() == items.last().map(|o| o.local.as_str())));
    // 520 in all: 500.
    for c in 1..11 {
        for i in 0..47 {
            r.send(&id(c), &format!("{c}-{i}"), "More");
        }
    }
    assert_eq!(r.ops().len(), 500);
    assert_eq!(r.unsent(), Some(500));
    let last = r.outboxes(&id(10)).pop().unwrap().0;
    assert!(last.last().is_some_and(|o| o.seq == 0 && o.state == OutState::NotSent { reason: Unsent::TooMany, code: None }));
}

#[test]
fn nothing_is_sent_while_working_offline() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 3)]));
    r.w.work_offline(true);
    let asked = r.all_hits();
    r.send(CHAT, "1-1", "Later");
    r.turns(Duration::from_secs(10));
    assert_eq!(r.all_hits(), asked);
    assert_eq!(r.ops().len(), 1, "kept");
    r.w.work_offline(false);
    r.turns(Duration::from_secs(2));
    assert_eq!(r.hits("post"), 1);
    assert!(r.ops().is_empty());
}

#[test]
fn sign_out_empties_the_outbox_whatever_the_copy() {
    let who = Identity { tenant_id: TENANT.into(), object_id: ME.into() };
    // Signed in, the copy open.
    let mut r = run(teams_with(vec![chat(CHAT, T0, 3)]));
    r.w.work_offline(true);
    r.send(CHAT, "1-1", "Never sent");
    r.w.job(Job::SignOut { delete_copy: false });
    let (copy, opened) = Copy::open(r.w.store.clone(), &who).unwrap();
    assert_eq!(opened, Opened::Same, "the copy kept");
    assert!(!copy.chats().is_empty());
    assert!(copy.ops().is_empty());
    assert_eq!(r.unsent(), Some(0));

    // Switched off first: the outbox stays; signed out, the copy closed: emptied all the same.
    let mut r = run(teams_with(vec![chat(CHAT, T0, 3)]));
    r.w.work_offline(true);
    r.send(CHAT, "1-1", "Never sent either");
    r.w.job(Job::Stop);
    let (copy, _) = Copy::open(r.w.store.clone(), &who).unwrap();
    assert_eq!(copy.ops().len(), 1, "switched off keeps it");
    r.w.job(Job::SignOut { delete_copy: false });
    assert!(copy.ops().is_empty());
    assert!(!copy.chats().is_empty());
    assert_eq!(r.unsent(), Some(0));
}

#[test]
fn an_outbox_of_another_identity_is_never_sent() {
    let mut r = run_with(teams_with(vec![chat(CHAT, T0, 3)]), |w| {
        let (copy, _) = Copy::open(w.store.clone(), &Identity { tenant_id: TENANT.into(), object_id: "u-1".into() }).unwrap();
        let theirs = chat_of(&json!({"id": CHAT, "chatType": "oneOnOne"})).unwrap();
        copy.put_chat(&ChatRecord::new(theirs)).unwrap();
        for seq in 1..=2 {
            let op = OpRecord {
                seq,
                local: format!("1-{seq}"),
                chat: CHAT.into(),
                body: body("Theirs"),
                mentions: Vec::new(),
                what: OutKind::Send,
                state: OpState::Waiting,
                tries: 0,
                after: None,
            };
            copy.put_op(&op).unwrap();
        }
        assert_eq!(copy.ops().len(), 2);
    });
    r.turns(Duration::from_secs(10));
    assert_eq!(r.hits("post"), 0);
    assert!(r.ops().is_empty());
    assert_eq!(r.unsent(), Some(0));
}

#[test]
fn marking_read_is_once_per_chat_and_its_row_comes_at_once() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 3)]));
    assert!(r.row(CHAT).unread > 0);
    for _ in 0..3 {
        r.w.job(Job::Op { op: Op::MarkRead { chat: CHAT.into() } });
    }
    let from = r.events().len();
    r.turns(STEP);
    assert_eq!(r.hits("markChatReadForUser"), 1);
    let rows: Vec<model::ChatRow> = r
        .events()
        .into_iter()
        .skip(from)
        .filter_map(|e| if let model::Event::Chats { rows } = e { Some(rows) } else { None })
        .flatten()
        .filter(|row| row.id == CHAT)
        .collect();
    assert!(rows.last().is_some_and(|row| row.unread == 0), "in the same turn: {rows:?}");
    // The fake marks it only for the signed-in user's own ids.
    assert_ne!(r.fake(|t| t.chat(CHAT).unwrap().read_time.clone()), "2026-01-01T00:00:00.000Z");
    assert!(r.record(CHAT).unread.is_empty());
    r.turns(Duration::from_secs(30));
    assert_eq!(r.hits("markChatReadForUser"), 1);
    assert_eq!(r.row(CHAT).unread, 0);
}

#[test]
fn a_refused_mark_is_dropped() {
    let mut teams = teams_with(vec![chat(CHAT, T0, 3)]);
    teams.forbid_mark.push(CHAT.into());
    let mut r = run(teams);
    r.w.job(Job::Op { op: Op::MarkRead { chat: CHAT.into() } });
    r.turns(Duration::from_secs(30));
    assert_eq!(r.hits("markChatReadForUser"), 1);
    assert!(r.row(CHAT).unread > 0);
}

/// M1: two messages of the same text are two: the second, its post failing, isn't taken for the
/// first when it's checked.
#[test]
fn two_of_the_same_text_are_two_messages() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 3)]));
    r.send(CHAT, "1-1", "Same again");
    r.send(CHAT, "1-2", "Same again");
    assert!(r.turns_until(Duration::from_secs(2), |r| r.hits("post") == 1));
    r.fake(|t| {
        t.fail_next.insert("post".into(), 1);
    });
    r.turns(Duration::from_secs(15));
    assert_eq!(r.hits("post"), 3, "the second posted, failed, checked, and posted again");
    assert!(r.ops().is_empty());
    assert_eq!(r.held(CHAT, "Same again"), 2);
}

/// A refusal's code is kept within the model's 64 characters, and the outbox takes it.
#[test]
fn a_refusals_code_is_kept_short() {
    let (copy, _) = Copy::open(quatrelle_store::Store::memory(), &Identity { tenant_id: TENANT.into(), object_id: ME.into() }).unwrap();
    let fits = format!("HTTP 403 {}", "A".repeat(55));
    let cases = [
        ("HTTP 403 Forbidden".to_string(), Some("HTTP 403 Forbidden".to_string())),
        (fits.clone(), Some(fits)),
        (format!("HTTP 403 {}", "A".repeat(56)), Some("HTTP 403".to_string())),
        ("HTTP 413 RequestEntityTooLarge".to_string(), Some("HTTP 413 RequestEntityTooLarge".to_string())),
    ];
    for (said, kept) in cases {
        let code = answer_code(&NetError::Protocol(said));
        assert_eq!(code, kept);
        let op = OpRecord {
            seq: 1,
            local: "1-1".into(),
            chat: CHAT.into(),
            body: body("x"),
            mentions: Vec::new(),
            what: OutKind::Send,
            state: OpState::NotSent { reason: Unsent::Refused, code },
            tries: 0,
            after: None,
        };
        copy.put_op(&op).unwrap();
    }
    for no_code in [NetError::Closed("HTTP 503".into()), NetError::Protocol("Graph: an answer".into()), NetError::Refused("not a chat's id".into())] {
        assert_eq!(answer_code(&no_code), None, "{no_code:?}");
    }
}

#[test]
fn logs_carry_no_text_or_id_of_a_message() {
    const HIDDEN: &str = "19:quokka5520@thread.v2";
    const LOCAL: &str = "1790000000-77";
    let mut r = run(teams_with(vec![chat(HIDDEN, T0, 3)]));
    let tap = crate::fakes::logs::tap(|l| l.starts_with("teams: ") || l.starts_with("Teams copy: "));
    r.fake(|t| {
        t.fail_next.insert("post".into(), 1);
    });
    r.send(HIDDEN, LOCAL, "Wallaby bilby totals");
    r.turns(Duration::from_secs(15));
    r.send(HIDDEN, "1790000000-78", &"<".repeat(SEND_MAX));
    r.w.job(Job::Op { op: Op::MarkRead { chat: HIDDEN.into() } });
    r.turns(Duration::from_secs(2));
    assert!(r.ops().is_empty() && r.held(HIDDEN, "Wallaby") == 1);
    let lines = tap.lines();
    assert!(lines.iter().any(|l| l.contains("HTTP 500")), "a failure is said by its status: {lines:?}");
    for line in &lines {
        let lower = line.to_lowercase();
        for private in ["quokka", "wallaby", "bilby", "1790000000", "@thread.v2", "&lt;"] {
            assert!(!lower.contains(private), "{private:?} in the log: {line}");
        }
    }
}
