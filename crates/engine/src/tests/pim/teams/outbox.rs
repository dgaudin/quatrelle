// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: Teams, writing: a message sent shown at once, then as the worker keeps it; the
//! send, retry and delete commands and what they refuse; a message not sent elsewhere said once;
//! a chat marked read when it's opened.

use quatrelle_model::pim::teams::{OUTBOX_CHAT_MAX, OUTBOX_MAX, Op, OutState, Outgoing, Unsent};

use super::*;
use crate::mime::teams_compose;

const CHAT: &str = "19:chat1@thread.v2";
const OTHER: &str = "19:chat2@thread.v2";
const NOT_SENT: &str = "A message to {chat} wasn't sent";

fn outbox(chat: &str, items: Vec<Outgoing>, took: Option<&str>) -> Event {
    teams_event(teams::Event::Outbox { chat: chat.into(), items, took: took.map(str::to_string) })
}

fn item(chat: &str, seq: u64, local: &str, state: OutState) -> Outgoing {
    Outgoing { seq, local: local.into(), chat: chat.into(), body: teams_compose("hi", &[], &[]), state }
}

fn refused() -> OutState {
    OutState::NotSent { reason: Unsent::Refused, code: Some("HTTP 403 Forbidden".into()) }
}

fn send(s: &mut Session, chat: &str, text: &str) -> Result<String, CmdError> {
    s.run("teams.send", &json!({"chat": chat, "text": text})).map(|v| v["local"].as_str().unwrap_or_default().to_string())
}

/// The notices saying a message to another chat wasn't sent.
fn said(s: &Session) -> Vec<String> {
    s.notices.iter().filter(|n| n.template == NOT_SENT).map(|n| n.text.clone()).collect()
}

/// The open chat with `rows`, on screen (Conversations shown), its jobs taken.
fn opened(rows: Vec<ChatRow>) -> Session {
    let mut s = signed_in(rows);
    s.module = Module::Conversations;
    open(&mut s, CHAT);
    s.take_jobs();
    s
}

#[test]
fn a_message_shows_at_once_then_as_the_worker_keeps_it() {
    let mut s = opened(vec![row(1, 10)]);
    let local = s.run("teams.send", &json!({"chat": CHAT, "text": "hello world", "bold": [[0, 5]]})).unwrap()["local"].as_str().unwrap().to_string();
    assert!(!local.is_empty() && local.len() <= teams::LOCAL_MAX, "{local}");
    let body = teams_compose("hello world", std::slice::from_ref(&(0..5)), &[]);
    assert_eq!(s.teams.outgoing, [Outgoing { seq: 0, local: local.clone(), chat: CHAT.into(), body: body.clone(), state: OutState::Waiting }]);
    assert_eq!(teams_jobs(&mut s), [teams::Job::Op { op: Op::Send { chat: CHAT.into(), local: local.clone(), body: body.clone() } }]);
    // The worker took it, as its seventh: shown once, as it keeps it.
    let kept = Outgoing { seq: 7, local: local.clone(), chat: CHAT.into(), body, state: OutState::Waiting };
    s.apply(outbox(CHAT, vec![kept.clone()], Some(&local)));
    assert_eq!(s.teams.outgoing, [kept]);
    // Teams has it: gone from the messages going out.
    s.apply(outbox(CHAT, vec![], Some(&local)));
    assert!(s.teams.outgoing.is_empty(), "{:?}", s.teams.outgoing);
    // Each message sent has an id of its own.
    let again = send(&mut s, CHAT, "hello again").unwrap();
    assert_ne!(again, local);
}

#[test]
fn a_snapshot_before_the_job_was_taken_keeps_the_provisional() {
    let mut s = opened(vec![row(1, 10)]);
    let local = send(&mut s, CHAT, "on its way").unwrap();
    s.apply(outbox(CHAT, vec![], None));
    assert_eq!(s.teams.outgoing.iter().map(|o| (o.seq, o.local.as_str())).collect::<Vec<_>>(), [(0, local.as_str())]);
    // The chat's others said: the one sent here after them, still.
    let waiting = item(CHAT, 3, "100-1", OutState::Waiting);
    s.apply(outbox(CHAT, vec![waiting.clone()], None));
    assert_eq!(s.teams.outgoing.iter().map(|o| (o.seq, o.local.as_str())).collect::<Vec<_>>(), [(3, "100-1"), (0, local.as_str())]);
    // Another chat's outbox doesn't touch the open one's.
    s.apply(outbox(OTHER, vec![item(OTHER, 9, "100-2", OutState::Waiting)], None));
    assert_eq!(s.teams.outgoing.len(), 2);
}

#[test]
fn too_long_is_refused_and_nothing_queued() {
    let mut s = opened(vec![row(1, 10)]);
    let too_long = Err(CmdError::Params("This message is too long for Teams".into()));
    assert_eq!(send(&mut s, CHAT, &"a".repeat(90 << 10)), too_long);
    // Escaped, `<` takes four bytes: 30,000 of them are past 80 KiB.
    assert_eq!(send(&mut s, CHAT, &"<".repeat(30_000)), too_long);
    assert!(teams_jobs(&mut s).is_empty());
    assert!(s.teams.outgoing.is_empty());
    // Under the cap, markup goes as text.
    send(&mut s, CHAT, &"<b>".repeat(1_000)).unwrap();
    assert_eq!(teams_jobs(&mut s).len(), 1);
}

#[test]
fn nothing_to_send_is_refused() {
    let mut s = opened(vec![row(1, 10)]);
    for blank in ["", "   ", "\n \n\t\n", "\u{1}\u{7}\u{1b}", "\r\n\r\n"] {
        assert!(matches!(send(&mut s, CHAT, blank), Err(CmdError::Params(_))), "{blank:?}");
    }
    assert!(matches!(s.run("teams.send", &json!({"chat": CHAT})), Err(CmdError::Params(_))), "no text");
    assert!(teams_jobs(&mut s).is_empty() && s.teams.outgoing.is_empty());
}

#[test]
fn only_a_listed_chat_that_can_be_written_takes_a_message() {
    let mut left = row(2, 20);
    left.left = true;
    let mut unreadable = row(3, 30);
    unreadable.unreadable = true;
    let mut s = signed_in(vec![row(1, 10), left, unreadable]);
    for chat in ["19:chat2@thread.v2", "19:chat3@thread.v2", "19:nope@thread.v2", "", &"x".repeat(teams::MAX_ID + 1)] {
        assert!(send(&mut s, chat, "hello").is_err(), "{chat}");
    }
    assert!(teams_jobs(&mut s).is_empty());
    // A listed chat not open: sent, nothing shown in the open one.
    send(&mut s, CHAT, "hello").unwrap();
    assert!(matches!(&teams_jobs(&mut s)[..], [teams::Job::Op { op: Op::Send { chat, .. } }] if chat == CHAT));
    assert!(s.teams.outgoing.is_empty());
}

#[test]
fn ranges_off_the_text_are_refused() {
    let mut s = opened(vec![row(1, 10)]);
    let with = |s: &mut Session, text: &str, bold: Value| s.run("teams.send", &json!({"chat": CHAT, "text": text, "bold": bold}));
    for bad in [json!([[0, 99]]), json!([[1]]), json!([["a", 2]]), json!([[2, 1]]), json!([[0, 1, 2]]), json!(5), json!([[-1, 1]]), json!([[0.5, 1]])]
    {
        assert!(matches!(with(&mut s, "hi", bad.clone()), Err(CmdError::Params(_))), "{bad}");
    }
    assert!(matches!(with(&mut s, "été", json!([[0, 1]])), Err(CmdError::Params(_))), "inside a character");
    let many: Vec<Value> = (0..1_001).map(|_| json!([0, 1])).collect();
    assert!(matches!(with(&mut s, "hi", json!(many)), Err(CmdError::Params(_))), "past 1,000 ranges");
    assert!(matches!(s.run("teams.send", &json!({"chat": CHAT, "text": "hi", "italic": [[0, 3]]})), Err(CmdError::Params(_))));
    assert!(teams_jobs(&mut s).is_empty() && s.teams.outgoing.is_empty());
    // On the text's characters: taken.
    with(&mut s, "été", json!([[0, 2], [3, 5]])).unwrap();
    s.run("teams.send", &json!({"chat": CHAT, "text": "hi", "bold": null, "italic": [[0, 2]]})).unwrap();
    assert_eq!(teams_jobs(&mut s).len(), 2);
}

#[test]
fn opening_a_chat_with_unread_marks_it_read() {
    let mut unread = row(1, 10);
    unread.unread = 3;
    let mut left = row(3, 30);
    left.unread = 2;
    left.left = true;
    let mut s = signed_in(vec![unread, row(2, 20), left]);
    open(&mut s, CHAT);
    assert_eq!(teams_jobs(&mut s), [teams::Job::Open { chat: Some(CHAT.into()) }, teams::Job::Op { op: Op::MarkRead { chat: CHAT.into() } }]);
    assert_eq!(s.teams.row(CHAT).map(|r| r.unread), Some(3), "the row stays unread until the worker says so");
    open(&mut s, OTHER);
    assert_eq!(teams_jobs(&mut s), [teams::Job::Open { chat: Some(OTHER.into()) }], "nothing unread");
    open(&mut s, "19:chat3@thread.v2");
    assert_eq!(teams_jobs(&mut s), [teams::Job::Open { chat: Some("19:chat3@thread.v2".into()) }], "left: not marked");
    s.run("teams.open", &json!({"chat": null})).unwrap();
    assert_eq!(teams_jobs(&mut s), [teams::Job::Open { chat: None }]);
}

#[test]
fn delete_is_refused_while_sending() {
    let mut s = opened(vec![row(1, 10)]);
    s.apply(outbox(CHAT, vec![item(CHAT, 4, "100-1", OutState::Sending), item(CHAT, 5, "100-2", refused())], None));
    assert_eq!(s.run("teams.discard", &json!({"seq": 4})), Err(CmdError::Disabled("This message may already be in Teams".into())));
    assert!(teams_jobs(&mut s).is_empty());
    assert_eq!(s.teams.outgoing.len(), 2);
    s.run("teams.discard", &json!({"seq": 5})).unwrap();
    assert_eq!(teams_jobs(&mut s), [teams::Job::Discard { seq: 5 }]);
    assert_eq!(s.teams.outgoing.iter().map(|o| o.seq).collect::<Vec<_>>(), [4], "gone at once");
    // One that isn't going out in the open chat: refused.
    for junk in [json!({"seq": 99}), json!({"seq": "5"}), json!({"seq": -1}), json!({"seq": 0}), json!({"seq": 0, "local": "100-1"})] {
        assert!(matches!(s.run("teams.discard", &junk), Err(CmdError::Params(_))), "{junk}");
    }
    // One sent from here the worker hasn't answered for yet: refused too, it's on its way.
    let local = send(&mut s, CHAT, "on its way").unwrap();
    teams_jobs(&mut s);
    assert!(matches!(s.run("teams.discard", &json!({"seq": 0, "local": local})), Err(CmdError::Disabled(_))));
    assert_eq!(s.teams.outgoing.len(), 2);
}

/// A message the worker refused to take (`seq` 0, nothing kept on disk) stays shown "Not sent"
/// over the snapshots after it: Delete takes it away here alone, Retry sends its text again.
#[test]
fn a_message_refused_by_the_outbox_stays_until_deleted_or_sent_again() {
    let mut s = opened(vec![row(1, 10)]);
    let local = send(&mut s, CHAT, "hi").unwrap();
    let Some(teams::Job::Op { op: Op::Send { body, .. } }) = teams_jobs(&mut s).pop() else { panic!("no send") };
    let too_many = OutState::NotSent { reason: Unsent::TooMany, code: None };
    let refusal = Outgoing { seq: 0, local: local.clone(), chat: CHAT.into(), body: body.clone(), state: too_many.clone() };
    s.apply(outbox(CHAT, vec![item(CHAT, 3, "100-1", OutState::Waiting), refusal.clone()], Some(&local)));
    assert_eq!(s.teams.outgoing.last(), Some(&refusal));
    s.apply(outbox(CHAT, vec![], None));
    assert_eq!(s.teams.outgoing, std::slice::from_ref(&refusal), "kept: the worker holds nothing of it");
    // Deleted: here alone.
    s.run("teams.discard", &json!({"seq": 0, "local": local})).unwrap();
    assert!(teams_jobs(&mut s).is_empty() && s.teams.outgoing.is_empty());
    // Sent again: its text, as a new message.
    s.apply(outbox(CHAT, vec![refusal.clone()], Some(&local)));
    let again = s.run("teams.retry", &json!({"seq": 0, "local": local})).unwrap()["local"].as_str().unwrap().to_string();
    assert_ne!(again, local);
    assert_eq!(teams_jobs(&mut s), [teams::Job::Op { op: Op::Send { chat: CHAT.into(), local: again.clone(), body: body.clone() } }]);
    assert_eq!(s.teams.outgoing, [Outgoing { seq: 0, local: again, chat: CHAT.into(), body, state: OutState::Waiting }]);
}

#[test]
fn retry_acts_only_on_a_message_not_sent() {
    let mut s = opened(vec![row(1, 10)]);
    s.apply(outbox(CHAT, vec![item(CHAT, 4, "100-1", OutState::Sending), item(CHAT, 5, "100-2", refused())], None));
    assert!(matches!(s.run("teams.retry", &json!({"seq": 4})), Err(CmdError::Disabled(_))), "being sent");
    s.run("teams.retry", &json!({"seq": 5})).unwrap();
    assert_eq!(teams_jobs(&mut s), [teams::Job::Retry { seq: 5 }]);
    assert_eq!(s.teams.outgoing.get(1).map(|o| &o.state), Some(&OutState::Waiting), "shown waiting at once");
    // Clicked twice: asked once.
    assert!(s.run("teams.retry", &json!({"seq": 5})).is_err());
    assert!(teams_jobs(&mut s).is_empty());
}

#[test]
fn a_message_not_sent_elsewhere_is_said_once() {
    let mut s = signed_in(vec![row(1, 10), row(2, 20)]);
    s.module = Module::Conversations;
    open(&mut s, OTHER);
    s.apply(outbox(CHAT, vec![item(CHAT, 3, "100-1", refused())], None));
    s.apply(outbox(CHAT, vec![item(CHAT, 3, "100-1", refused()), item(CHAT, 4, "100-2", OutState::Waiting)], None));
    assert_eq!(said(&s), ["A message to Chat 1 wasn't sent"]);
    assert!(s.teams.outgoing.is_empty(), "another chat's messages aren't shown in the open one");
    // Its number used again by another message (the outbox numbers from its highest kept): said.
    s.apply(outbox(CHAT, vec![item(CHAT, 3, "100-3", refused())], None));
    assert_eq!(said(&s).len(), 2);
    // Sent again and not sent again: said again.
    s.apply(outbox(CHAT, vec![item(CHAT, 3, "100-3", OutState::Waiting)], None));
    s.apply(outbox(CHAT, vec![item(CHAT, 3, "100-3", refused())], None));
    assert_eq!(said(&s).len(), 3);
    // One the worker refused to take: said too.
    s.apply(outbox(CHAT, vec![item(CHAT, 0, "100-4", OutState::NotSent { reason: Unsent::Left, code: None })], Some("100-4")));
    assert_eq!(said(&s).len(), 4);
    // Seen in the open chat: not said when it comes up again from elsewhere.
    s.apply(outbox(OTHER, vec![item(OTHER, 8, "100-5", refused())], None));
    open(&mut s, CHAT);
    s.apply(outbox(OTHER, vec![item(OTHER, 8, "100-5", refused())], None));
    assert_eq!(said(&s).len(), 4);
}

/// A message not sent stays in its chat: shown again when the chat is opened again, the worker
/// saying its outbox at each opening.
#[test]
fn a_chat_opened_again_shows_its_unsent() {
    let mut s = opened(vec![row(1, 10), row(2, 20)]);
    let unsent = item(CHAT, 3, "100-1", refused());
    s.apply(outbox(CHAT, vec![unsent.clone()], None));
    assert_eq!(s.teams.outgoing, std::slice::from_ref(&unsent));
    open(&mut s, OTHER);
    assert!(s.teams.outgoing.is_empty(), "another chat's messages going out aren't shown");
    s.apply(outbox(OTHER, vec![], None));
    open(&mut s, CHAT);
    s.apply(outbox(CHAT, vec![unsent.clone()], None));
    assert_eq!(s.teams.outgoing, [unsent]);
    assert!(said(&s).is_empty(), "seen where it was: no notice");
}

#[test]
fn the_unsent_count_is_kept_and_forgotten_on_sign_out() {
    let mut s = opened(vec![row(1, 10)]);
    s.apply(teams_event(teams::Event::Unsent { count: 7 }));
    assert_eq!(s.teams.unsent, 7);
    s.apply(outbox(CHAT, vec![item(CHAT, 3, "100-1", refused())], None));
    s.run("teams.signOut", &json!({})).unwrap();
    assert_eq!(s.teams.unsent, 0);
    assert!(s.teams.outgoing.is_empty());
    s.apply(teams_event(teams::Event::Unsent { count: 3 }));
    assert_eq!(s.teams.unsent, 0, "not taken signed out");
}

#[test]
fn past_the_outbox_caps_nothing_is_queued() {
    let mut s = opened(vec![row(1, 10), row(2, 20)]);
    let too_many = Err(CmdError::Disabled("Too many messages are waiting".into()));
    s.apply(teams_event(teams::Event::Unsent { count: OUTBOX_MAX as u32 }));
    assert_eq!(send(&mut s, OTHER, "hi"), too_many, "the outbox is full");
    s.apply(teams_event(teams::Event::Unsent { count: 0 }));
    let full: Vec<Outgoing> = (1..=OUTBOX_CHAT_MAX as u64).map(|i| item(CHAT, i, &format!("100-{i}"), OutState::Waiting)).collect();
    s.apply(outbox(CHAT, full, None));
    assert_eq!(send(&mut s, CHAT, "hi"), too_many, "the chat's share is full");
    assert!(teams_jobs(&mut s).is_empty());
    assert_eq!(s.teams.outgoing.len(), OUTBOX_CHAT_MAX);
    // More than a chat's share said: the newest kept.
    let more: Vec<Outgoing> = (1..=OUTBOX_CHAT_MAX as u64 + 5).map(|i| item(CHAT, i, &format!("100-{i}"), OutState::Waiting)).collect();
    s.apply(outbox(CHAT, more, None));
    assert_eq!(s.teams.outgoing.len(), OUTBOX_CHAT_MAX);
    assert_eq!(s.teams.outgoing.last().map(|o| o.seq), Some(OUTBOX_CHAT_MAX as u64 + 5));
}

#[test]
fn sending_from_an_older_view_goes_back_to_the_newest() {
    let mut s = opened(vec![row(1, 10)]);
    s.teams.newer_left_out = true;
    let local = send(&mut s, CHAT, "hello").unwrap();
    let jobs = teams_jobs(&mut s);
    assert!(
        matches!(&jobs[..], [teams::Job::Open { chat: Some(c) }, teams::Job::Op { op: Op::Send { chat, local: l, .. } }] if c == CHAT && chat == CHAT && *l == local),
        "{jobs:?}"
    );
    assert!(!s.teams.newer_left_out);
    assert_eq!(s.teams.outgoing.iter().map(|o| o.local.as_str()).collect::<Vec<_>>(), [local.as_str()]);
}

/// The worker's refusal of `local` (it kept nothing of it), as its answer to the send.
fn refusal(chat: &str, local: &str) -> Event {
    let state = OutState::NotSent { reason: Unsent::Refused, code: None };
    outbox(chat, vec![item(chat, 0, local, state)], Some(local))
}

/// The messages going out shown, by local id.
fn shown(s: &Session) -> Vec<String> {
    s.teams.outgoing.iter().map(|o| o.local.clone()).collect()
}

/// Those of `shown` from `from` on, in any order (refusals' order is left for later).
fn shown_set(s: &Session, from: usize) -> std::collections::BTreeSet<String> {
    shown(s).into_iter().skip(from).collect()
}

/// A message the worker refused to take (nothing of it kept on disk) stays in its conversation:
/// another chat opened, then this one again, it's there, with Retry and Delete.
#[test]
fn a_refused_message_survives_another_chat_opened() {
    let mut s = opened(vec![row(1, 10), row(2, 20)]);
    let (a, b) = (send(&mut s, CHAT, "first").unwrap(), send(&mut s, CHAT, "second").unwrap());
    s.apply(refusal(CHAT, &a));
    s.apply(refusal(CHAT, &b));
    s.take_jobs();
    open(&mut s, OTHER);
    s.apply(outbox(OTHER, vec![], None));
    assert!(s.teams.outgoing.is_empty());
    open(&mut s, CHAT);
    let both: std::collections::BTreeSet<String> = [a.clone(), b.clone()].into();
    assert_eq!(shown_set(&s, 0), both, "shown again as the chat opens");
    // The worker says the chat's outbox (it holds nothing of them): they stay.
    s.apply(outbox(CHAT, vec![item(CHAT, 5, "100-1", OutState::Waiting)], None));
    assert_eq!((shown(&s).first().cloned(), shown_set(&s, 1)), (Some("100-1".to_string()), both));
    s.take_jobs();
    s.run("teams.discard", &json!({"seq": 0, "local": a})).unwrap();
    assert!(teams_jobs(&mut s).is_empty());
    let again = s.run("teams.retry", &json!({"seq": 0, "local": b})).unwrap()["local"].as_str().unwrap().to_string();
    assert!(matches!(&teams_jobs(&mut s)[..], [teams::Job::Op { op: Op::Send { local, .. } }] if *local == again));
    assert_eq!(shown(&s), ["100-1".to_string(), again]);
    // Deleted or sent again: not shown again at the next opening.
    open(&mut s, OTHER);
    open(&mut s, CHAT);
    assert!(s.teams.outgoing.is_empty(), "{:?}", shown(&s));
}

/// A refusal that comes while another chat is open is said, and kept for its chat: shown when
/// that chat is opened.
#[test]
fn a_refusal_for_a_chat_not_open_is_kept_for_it() {
    let mut s = opened(vec![row(1, 10), row(2, 20)]);
    let local = send(&mut s, CHAT, "hi").unwrap();
    open(&mut s, OTHER);
    s.apply(refusal(CHAT, &local));
    assert_eq!(said(&s), ["A message to Chat 1 wasn't sent"]);
    assert!(s.teams.outgoing.is_empty());
    open(&mut s, CHAT);
    s.apply(outbox(CHAT, vec![], None));
    assert_eq!(shown(&s), [local]);
    assert_eq!(said(&s).len(), 1, "said once");
}

/// The refusals kept for chats not open: at most a chat's share each, and 1,000 in all, the
/// oldest going first.
#[test]
fn refusals_kept_for_closed_chats_are_bounded() {
    let rows: Vec<ChatRow> = (1..=22).map(|i| row(i, 100 - i as i64)).collect();
    let mut s = opened(rows);
    open(&mut s, "19:chat22@thread.v2");
    for i in 0..OUTBOX_CHAT_MAX + 10 {
        s.apply(refusal(CHAT, &format!("1-{i}")));
    }
    for chat in 2..=21 {
        for i in 0..OUTBOX_CHAT_MAX {
            s.apply(refusal(&format!("19:chat{chat}@thread.v2"), &format!("{chat}-{i}")));
        }
    }
    // 21 chats of 50: the first chat's, the oldest, went for the 1,000 cap.
    open(&mut s, CHAT);
    assert!(s.teams.outgoing.is_empty(), "{}", s.teams.outgoing.len());
    open(&mut s, "19:chat2@thread.v2");
    assert_eq!(s.teams.outgoing.len(), OUTBOX_CHAT_MAX);
    // A chat's share: its newest kept.
    let mut s = opened(vec![row(1, 10), row(2, 20)]);
    open(&mut s, OTHER);
    for i in 0..OUTBOX_CHAT_MAX + 10 {
        s.apply(refusal(CHAT, &format!("1-{i}")));
    }
    open(&mut s, CHAT);
    assert_eq!(s.teams.outgoing.len(), OUTBOX_CHAT_MAX);
    assert_eq!(s.teams.outgoing.first().map(|o| o.local.as_str()), Some("1-10"));
}

/// The open chat isn't on screen while another module is shown: a message not sent there is
/// said, once.
#[test]
fn a_message_not_sent_while_mail_is_shown_is_said() {
    let mut s = opened(vec![row(1, 10)]);
    let local = send(&mut s, CHAT, "hi").unwrap();
    s.apply(outbox(CHAT, vec![item(CHAT, 4, &local, OutState::Waiting)], Some(&local)));
    s.run("view.module", &json!({"name": "mail"})).unwrap();
    s.apply(outbox(CHAT, vec![item(CHAT, 4, &local, refused())], None));
    assert_eq!(said(&s), ["A message to Chat 1 wasn't sent"]);
    assert_eq!(s.teams.outgoing.iter().map(|o| o.seq).collect::<Vec<_>>(), [4], "still shown in its chat");
    s.apply(outbox(CHAT, vec![item(CHAT, 4, &local, refused())], None));
    s.run("view.module", &json!({"name": "conversations"})).unwrap();
    s.apply(outbox(CHAT, vec![item(CHAT, 4, &local, refused())], None));
    assert_eq!(said(&s).len(), 1, "said once");
    // On screen: shown, not said.
    s.apply(outbox(CHAT, vec![item(CHAT, 4, &local, refused()), item(CHAT, 5, "100-9", refused())], None));
    assert_eq!(said(&s).len(), 1);
}

/// Nor is it on screen while the window is in the background (in the tray, another application in
/// front): a message not sent there is said, as the announcements of its news are.
#[test]
fn a_message_not_sent_while_the_window_is_unfocused_is_said() {
    let mut s = opened(vec![row(1, 10)]);
    s.teams_focus(false);
    s.apply(outbox(CHAT, vec![item(CHAT, 4, "100-1", refused())], None));
    assert_eq!(said(&s), ["A message to Chat 1 wasn't sent"]);
    assert_eq!(s.teams.outgoing.iter().map(|o| o.seq).collect::<Vec<_>>(), [4], "still shown in its chat");
    // Focused again: the next one is seen where it is, not said.
    s.teams_focus(true);
    s.apply(outbox(CHAT, vec![item(CHAT, 4, "100-1", refused()), item(CHAT, 5, "100-2", refused())], None));
    assert_eq!(said(&s).len(), 1);
}

/// Teams doesn't let an unreadable chat be read here: opening it doesn't mark it read there.
#[test]
fn an_unreadable_chat_is_not_marked_read_on_opening() {
    let mut unreadable = row(1, 10);
    unreadable.unread = 1;
    unreadable.unreadable = true;
    let mut s = signed_in(vec![unreadable]);
    open(&mut s, CHAT);
    assert_eq!(teams_jobs(&mut s), [teams::Job::Open { chat: Some(CHAT.into()) }]);
}

/// A Session launched at `now`, Teams on and signed in, with chat 1 listed.
fn launched(now: i64) -> Session {
    let mut s = Session::open(None, quatrelle_model::dates::Clock::new(now, 0));
    s.prefs.teams.on = true;
    s.prefs.teams.signed_in = true;
    s.apply(teams_event(teams::Event::Chats { rows: vec![row(1, 10)] }));
    s.take_jobs();
    s
}

/// The local ids count on from the launch's second × 1,000, as drafts' ids do: two runs sending
/// in the same second (the app and the command line, say) never give two messages the same id.
#[test]
fn two_launches_never_give_a_message_the_same_local_id() {
    const T: i64 = 1_763_200_000;
    let (mut first, mut second) = (launched(T), launched(T + 5));
    first.clock.now = T + 5;
    let (a, b) = (send(&mut first, CHAT, "hi").unwrap(), send(&mut second, CHAT, "hi").unwrap());
    assert_ne!(a, b);
    let n = |local: &str| local.rsplit('-').next().and_then(|n| n.parse::<u64>().ok()).unwrap_or(0);
    assert!(n(&a) > T as u64 * 1_000 && n(&b) > (T as u64 + 5) * 1_000, "{a} {b}");
}
