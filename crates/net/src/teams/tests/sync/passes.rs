// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The passes besides the list's: the week's chats every minute, the hourly pass and the chats it
//! no longer finds, and the copy's pages answered for the chat on screen.

use super::super::ME;
use super::*;

#[test]
fn the_chats_of_the_last_seven_days_are_read_every_minute() {
    let mut r = run(teams_with(vec![chat(CHAT, NOW - DAY, 2), chat(OTHER, T0, 2)]));
    let (recent, old) = (r.reads(CHAT), r.reads(OTHER));
    r.turns(Duration::from_secs(121));
    assert_eq!(r.reads(CHAT) - recent, 2, "once a minute");
    assert_eq!(r.reads(OTHER), old, "an older chat isn't");
}

#[test]
fn a_chat_gone_from_the_hourly_list_is_left() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 2), chat(OTHER, T0 + MINUTE, 2)]));
    r.fake(|t| t.chats.retain(|c| c.id != OTHER));
    // The ten-second passes read only the list's newest: they say nothing of what's gone.
    r.turns_every(Duration::from_secs(3500), Duration::from_secs(1));
    assert!(!r.row(OTHER).left);
    let from = r.events().len();
    r.turns_every(Duration::from_secs(120), Duration::from_secs(1));
    let removed: Vec<model::Event> = r.events().into_iter().skip(from).filter(|e| matches!(e, model::Event::Removed { .. })).collect();
    assert_eq!(removed, [model::Event::Removed { chat: OTHER.into() }]);
    assert!(r.row(OTHER).left && !r.row(CHAT).left);
    assert!(r.record(OTHER).chat.left, "kept, to be read");
}

#[test]
fn the_copy_answers_the_chat_opened_and_its_older_pages() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 2)]));
    for i in 0..250 {
        r.post(CHAT, if i % 2 == 0 { "u-ana" } else { ME }, NOW + i * 1000, &format!("Line {i}"));
    }
    r.turns(Duration::from_secs(20));
    let copy = r.w.copy.as_ref().unwrap();
    let window = copy.last_pages(CHAT, 2);
    assert!(window.len() < 250 && window.len() >= 100, "{}", window.len());
    let first = window.first().map(|m| m.created).unwrap();
    let before = copy.before(CHAT, first);
    assert!(!before.is_empty());

    let from = r.events().len();
    r.w.job(Job::Open { chat: Some(CHAT.into()) });
    let said = r.messages(CHAT, from);
    assert!(said.iter().all(|(m, fresh, older)| m.len() <= 50 && !fresh && !older), "{} events", said.len());
    assert_eq!(ids(&said), window.iter().map(|m| m.id.clone()).collect::<Vec<_>>(), "its last two pages, oldest first");

    let from = r.events().len();
    r.w.job(Job::Page { chat: CHAT.into(), before: first });
    let said = r.messages(CHAT, from);
    assert!(said.iter().all(|(m, fresh, older)| m.len() <= 50 && !fresh && *older));
    assert_eq!(ids(&said), before.iter().map(|m| m.id.clone()).collect::<Vec<_>>());
    // Not a chat's id: nothing.
    let from = r.events().len();
    r.w.job(Job::Page { chat: String::new(), before: first });
    r.w.job(Job::Open { chat: Some(String::new()) });
    assert_eq!(r.events().len(), from);
}
