// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A search's hit opened (R38): the chat on screen from its message's page on, 200 messages at
//! most, then whether they reach its newest; a message the copy doesn't have opens it as usual;
//! its members and its messages going out said as for an opening; and a chat that failed asked
//! at once when the user opens it.

use quatrelle_mime::teams_compose;
use quatrelle_model::pim::teams::{Op, OutState, Sender, Unsent};

use super::*;

/// `n` messages of Ana's put in `chat`'s copy after the fake's own two, a second apart from a
/// day before [`NOW`], in Graph's pages of 50 as a read puts them; their ids, oldest first.
fn copied(r: &Run, chat: &str, n: i64) -> Vec<String> {
    let ana = Sender::User { id: "u-ana".into(), name: "Ana Exemple".into(), identity_type: Some("aadUser".into()), tenant_id: Some(TENANT.into()) };
    let all: Vec<ChatMessage> = (0..n)
        .map(|i| {
            let at = NOW - DAY + i * 1000;
            let body = teams_compose(&format!("Line {i}"), &[], &[]);
            ChatMessage { id: format!("c{i:04}"), chat: chat.into(), sender: ana.clone(), created: at, modified: at, body, ..ChatMessage::default() }
        })
        .collect();
    for part in all.chunks(50) {
        r.w.copy.as_ref().unwrap().upsert(chat, part).unwrap();
    }
    all.into_iter().map(|m| m.id).collect()
}

/// The `Placed` events said from event number `from` on: (chat, message, newest).
fn placed(r: &Run, from: usize) -> Vec<(String, String, bool)> {
    r.events()
        .into_iter()
        .skip(from)
        .filter_map(|e| match e {
            model::Event::Placed { chat, message, newest } => Some((chat, message, newest)),
            _ => None,
        })
        .collect()
}

/// What a hit opened said, in order: its messages (neither news nor a page before), where they
/// stop, its members, then its messages going out (C4), the last.
fn said_as_a_hit(r: &Run, chat: &str, from: usize) {
    let events: Vec<model::Event> = r.events().into_iter().skip(from).collect();
    assert!(r.messages(chat, from).iter().all(|(m, fresh, older)| m.len() <= 50 && !fresh && !older), "{events:?}");
    let at = |f: &dyn Fn(&model::Event) -> bool| events.iter().position(f);
    let last_messages = events.iter().rposition(|e| matches!(e, model::Event::Messages { .. }));
    let place = at(&|e| matches!(e, model::Event::Placed { .. }));
    let members = at(&|e| matches!(e, model::Event::Members { .. }));
    assert!(last_messages < place && place < members, "messages, then where they stop, then the members: {events:?}");
    assert!(matches!(events.last(), Some(model::Event::Outbox { chat: c, took: None, .. }) if c == chat), "its outbox, last: {events:?}");
}

#[test]
fn find_sends_the_found_messages_page_and_after() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 2)]));
    let ids = copied(&r, CHAT, 1000);
    let from = r.events().len();
    r.w.job(Job::Find { chat: CHAT.into(), message: ids[299].clone() });
    let said = super::ids(&r.messages(CHAT, from));
    // From the first message of the page that holds it, in creation order, none left out.
    let k = ids.iter().position(|id| Some(id) == said.first()).expect("copied messages said");
    assert_eq!(said, ids[k..k + said.len()], "in creation order, none left out");
    assert!(k <= 299 && 299 < k + said.len(), "the message found is among them");
    let first = NOW - DAY + k as i64 * 1000;
    assert!(r.record(CHAT).pages.contains(&first), "from its page's first message: {k}");
    assert_eq!(said.len(), 200, "its page and the next, 200 at most");
    assert_eq!(placed(&r, from), [(CHAT.to_string(), ids[299].clone(), false)], "short of the newest");
    said_as_a_hit(&r, CHAT, from);
    // On screen as an opening: read every three seconds.
    let reads = r.reads(CHAT);
    r.turns(Duration::from_secs(3));
    assert_eq!(r.reads(CHAT), reads + 1);
    assert_eq!(r.w.open.as_deref(), Some(CHAT));
}

#[test]
fn find_near_the_end_reaches_the_newest() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 2)]));
    let ids = copied(&r, CHAT, 1000);
    let from = r.events().len();
    r.w.job(Job::Find { chat: CHAT.into(), message: ids[950].clone() });
    let said = super::ids(&r.messages(CHAT, from));
    assert!(said.contains(&ids[950]) && said.len() <= 200, "{} said", said.len());
    assert_eq!(said.last(), ids.last(), "up to the newest");
    assert_eq!(placed(&r, from), [(CHAT.to_string(), ids[950].clone(), true)]);
    said_as_a_hit(&r, CHAT, from);
}

#[test]
fn a_message_not_in_the_copy_opens_as_usual() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 2)]));
    copied(&r, CHAT, 300);
    let window: Vec<String> = r.w.copy.as_ref().unwrap().last_pages(CHAT, 2).into_iter().map(|m| m.id).collect();
    for message in ["c9999", "", "c0001\u{7}", &"c".repeat(600)] {
        let from = r.events().len();
        r.w.job(Job::Find { chat: CHAT.into(), message: message.to_string() });
        assert_eq!(super::ids(&r.messages(CHAT, from)), window, "{message:?}: its last two pages");
        assert_eq!(placed(&r, from), [(CHAT.to_string(), message.to_string(), true)], "{message:?}");
        said_as_a_hit(&r, CHAT, from);
    }
    // Not a chat's id: nothing said, nothing open.
    let from = r.events().len();
    r.w.job(Job::Find { chat: String::new(), message: "c0001".into() });
    assert_eq!(r.events().len(), from);
    assert_eq!(r.w.open, None);
}

/// C4: a chat opened by a hit says its messages going out, a "Not sent" one with them.
#[test]
fn a_hit_opened_says_its_outbox() {
    let mut teams = teams_with(vec![chat(CHAT, T0, 3), chat(OTHER, T0, 3)]);
    teams.forbid_post.push(CHAT.into());
    let mut r = run(teams);
    r.w.job(Job::Op {
        op: Op::Send {
            chat: CHAT.into(),
            local: "1-1".into(),
            body: teams_compose("Stays not sent", &[], &[]),
            mentions: Vec::new(),
            parts: Vec::new(),
        },
    });
    r.turns(Duration::from_secs(3));
    let refused = OutState::NotSent { reason: Unsent::Refused, code: Some("HTTP 403 Forbidden".into()) };
    r.w.job(Job::Find { chat: OTHER.into(), message: T0.to_string() });
    assert!(matches!(r.events().last(), Some(model::Event::Outbox { chat, items, took: None }) if chat == OTHER && items.is_empty()), "none as well");
    r.w.job(Job::Find { chat: CHAT.into(), message: T0.to_string() });
    let Some(model::Event::Outbox { chat, items, took: None }) = r.events().pop() else { panic!("no outbox last: {:?}", r.events()) };
    assert_eq!(chat, CHAT);
    assert_eq!(items.into_iter().map(|o| (o.local, o.state)).collect::<Vec<_>>(), [("1-1".to_string(), refused)]);
}

/// Minor (Task 7): a chat whose reads failed waits on its own; the user opening it (from the
/// list or from a hit) forgives that, and it is read within the open chat's three seconds.
#[test]
fn opening_forgives_its_own_back_off() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 2), chat(OTHER, T0 + MINUTE, 2)]));
    for job in [Job::Open { chat: Some(CHAT.into()) }, Job::Find { chat: CHAT.into(), message: T0.to_string() }] {
        // Its reads fail twice: it now waits a minute.
        r.fake(|t| t.fail_reads.push(CHAT.into()));
        r.w.job(Job::Open { chat: Some(CHAT.into()) });
        let reads = r.reads(CHAT);
        r.turns(Duration::from_secs(34));
        assert_eq!(r.reads(CHAT) - reads, 2, "{job:?}: the second after thirty seconds");
        r.fake(|t| t.fail_reads.clear());
        r.turns(Duration::from_secs(4));
        assert_eq!(r.reads(CHAT) - reads, 2, "{job:?}: waiting on its own");
        // Elsewhere, then opened again by the user: read at once.
        r.w.job(Job::Open { chat: Some(OTHER.into()) });
        r.w.job(job.clone());
        r.turns(Duration::from_secs(3));
        assert_eq!(r.reads(CHAT) - reads, 3, "{job:?}: read within three seconds");
        assert!(r.record(CHAT).watermark.is_some());
    }
}
