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

#[test]
fn one_chat_failing_slows_only_itself() {
    // One chat's messages fail (500) from its first read; another's members fail once.
    let mut teams = teams_with(vec![chat(CHAT, NOW - DAY, 2), chat(OTHER, NOW - DAY + MINUTE, 2), group(GROUP, None, T0)]);
    teams.fail_reads.push(CHAT.into());
    teams.fail_next.insert("members".into(), 1);
    let mut r = run(teams);
    assert!(r.record(OTHER).watermark.is_some(), "read though its members failed");
    r.w.job(Job::Open { chat: Some(GROUP.into()) });
    // The failing chat's incremental reads: its history waits on its own too.
    let incremental = |r: &Run| r.fake(|t| t.message_reads.iter().filter(|(c, f)| c == CHAT && f.contains(" gt ")).count());
    let (open, week, failing) = (r.reads(GROUP), r.reads(OTHER), incremental(&r));
    r.turns(Duration::from_secs(125));
    let open = r.reads(GROUP) - open;
    assert!(open >= 38, "the open chat every three seconds: {open} reads in 125 s");
    assert!(r.reads(OTHER) - week >= 2, "the week's pass every minute");
    assert!(incremental(&r) - failing <= 3, "the failing chat waits on its own: {} reads", incremental(&r) - failing);
    let history = r.fake(|t| t.message_reads.iter().filter(|(c, f)| c == CHAT && f.contains(" lt ")).count());
    assert!(history <= 3, "and so does its history: {history} reads in 125 s");
    assert!(!r.events().contains(&model::Event::Status { status: model::Status::Offline }));
}

#[test]
fn the_hourly_refresh_lets_the_other_cadences_go() {
    // 120 chats of the month (not of the week), one of the week, one older.
    let mut chats: Vec<FakeChat> = (0..120).map(|i| chat(&format!("19:m{i}@thread.v2"), NOW - 10 * DAY - i * MINUTE, 1)).collect();
    chats.push(chat(OTHER, NOW - DAY, 2));
    chats.push(chat(CHAT, T0, 2));
    let mut r = run(teams_with(chats));
    r.turns(Duration::from_secs(30));
    let members = r.hits("members");
    // The hour comes, and the minute's pass with it.
    r.w.limiter.prompt(Cadence::Hourly);
    r.w.limiter.prompt(Cadence::Active);
    let pages = r.hits("chats");
    while r.hits("chats") < pages + 3 {
        r.turns(STEP);
    }
    r.turns(Duration::from_secs(4));
    assert!(!r.w.limiter.due(Cadence::Active, r.t0 + r.at), "the minute's pass ran while the hour's refresh runs");
    // A new message during the refresh: seen within one list pass.
    let from = r.events().len();
    r.post(CHAT, "u-ana", NOW + MINUTE, "Meanwhile");
    r.turns(Duration::from_secs(11));
    assert_eq!(ids(&r.messages(CHAT, from)), [(NOW + MINUTE).to_string()]);
    // And the refresh gets done: every chat of the month has its members read again.
    r.turns(Duration::from_secs(40));
    assert!(r.hits("members") >= members + 120, "{} members reads", r.hits("members") - members);
}

#[test]
fn a_list_refused_says_so_and_is_tried_again_hourly() {
    // An account without Teams: the chat list itself is refused.
    let mut teams = teams_with(vec![chat(CHAT, T0, 2)]);
    teams.forbid_list = true;
    let mut r = run(teams);
    let why = "Teams didn't give this account's chats (403): sign in with the account you use Teams with";
    assert!(r.events().contains(&model::Event::Status { status: model::Status::Failed { why: why.into() } }), "{:?}", r.events());
    r.turns_every(Duration::from_secs(3500), Duration::from_secs(10));
    assert_eq!(r.hits("chats"), 1, "not asked again every ten seconds");
    r.fake(|t| t.forbid_list = false);
    r.turns_every(Duration::from_secs(120), Duration::from_secs(10));
    assert_eq!(r.hits("chats"), 2, "an hour later");
    assert_eq!(
        r.events().iter().rev().find(|e| matches!(e, model::Event::Status { .. })),
        Some(&model::Event::Status { status: model::Status::UpToDate })
    );
}
