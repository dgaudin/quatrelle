// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: Teams, what is announced of the new messages the worker reports: who and where,
//! the chats and settings that announce nothing, the open chat marked read instead while it's
//! looked at, one chat's news grouped, a burst summed up, each message once, and the bounds.

use quatrelle_model::pim::teams::{NEWS_CHARS, News, Op};

use super::*;
use crate::TeamsAnnouncement;

/// The clock at the start of each test, in seconds.
const T: i64 = 1_763_200_000;
const CHAT: &str = "19:chat1@thread.v2";

fn chat(i: usize) -> String {
    format!("19:chat{i}@thread.v2")
}

/// A new message `message` in `chat`, from someone of the organisation, in a group.
fn news(chat: &str, message: &str) -> News {
    News {
        chat: chat.into(),
        message: message.into(),
        sender: "Marie Durand".into(),
        external: false,
        app: false,
        mentions_me: false,
        chat_name: Some("Support team".into()),
        direct: false,
        hidden: false,
        text: "Lunch at noon?".into(),
        at: T * 1_000,
    }
}

/// The worker's news, in events of at most [`teams::EVENT_ITEMS`].
fn arrive(s: &mut Session, news: Vec<News>) {
    for part in news.chunks(teams::EVENT_ITEMS) {
        s.apply(teams_event(teams::Event::News { news: part.to_vec() }));
    }
}

/// Teams on and signed in with chats 1 to `n` listed, at [`T`].
fn at_t(n: usize) -> Session {
    let mut s = signed_in((1..=n).map(|i| row(i, 10)).collect());
    s.set_clock(T, 0);
    s
}

/// `chat` open in Conversations, its jobs taken.
fn looking_at(s: &mut Session, chat: &str) {
    s.module = Module::Conversations;
    open(s, chat);
    s.take_jobs();
}

/// The announcements taken at `now`.
fn taken_at(s: &mut Session, now: i64) -> Vec<TeamsAnnouncement> {
    s.set_clock(now, 0);
    s.take_teams_announcements()
}

fn mark_read(chat: &str) -> teams::Job {
    teams::Job::Op { op: Op::MarkRead { chat: chat.into() } }
}

/// The chats announced, in order (empty for a summary).
fn chats_of(a: &[TeamsAnnouncement]) -> Vec<&str> {
    a.iter().map(|a| a.chat.as_str()).collect()
}

#[test]
fn a_new_message_is_announced_with_who_and_where() {
    let mut s = at_t(3);
    let mut first = news(CHAT, "m1");
    first.external = true;
    first.mentions_me = true;
    arrive(&mut s, vec![first]);
    assert_eq!(
        s.take_teams_announcements(),
        [TeamsAnnouncement {
            chat: CHAT.into(),
            sender: "Marie Durand".into(),
            chat_name: Some("Support team".into()),
            external: true,
            app: false,
            mentioned: true,
            text: "Lunch at noon?".into(),
            count: 1,
            chats: 1,
        }]
    );
    assert!(s.take_teams_announcements().is_empty(), "taken once");
    assert!(teams_jobs(&mut s).is_empty(), "nothing marked read");
    assert_eq!(s.teams_next_announcement(), None, "nothing held");
    // An application in a one-to-one chat: no chat name.
    let bot = News { sender: "Build bot".into(), app: true, chat_name: None, direct: true, ..news(&chat(2), "m2") };
    arrive(&mut s, vec![bot]);
    let got = s.take_teams_announcements();
    assert_eq!(got.iter().map(|a| (a.sender.as_str(), a.app, a.chat_name.clone())).collect::<Vec<_>>(), [("Build bot", true, None)]);
    // What the worker says is bounded again: a name on one line without bidi controls, the first
    // line of the text at most NEWS_CHARS characters, ids within MAX_ID.
    let hostile = News {
        sender: "Marie\u{202E} Durand\nadmin".into(),
        chat_name: Some("Sup\u{2066}port\tteam".into()),
        text: "é".repeat(5_000),
        ..news(&chat(3), "m3")
    };
    let too_long = News { message: "m".repeat(teams::MAX_ID + 1), ..news(&chat(3), "m4") };
    let no_chat = news("", "m5");
    arrive(&mut s, vec![hostile, too_long, no_chat]);
    let got = s.take_teams_announcements();
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].sender, "Marie Durand admin");
    assert_eq!(got[0].chat_name.as_deref(), Some("Support team"));
    assert_eq!(got[0].text.chars().count(), NEWS_CHARS);
}

#[test]
fn hidden_chats_and_the_setting_announce_nothing() {
    let mut s = at_t(3);
    // Hidden as the worker says it, or as the list has it.
    let mut hidden_row = row(2, 10);
    hidden_row.hidden = true;
    s.apply(teams_event(teams::Event::Chats { rows: vec![hidden_row] }));
    arrive(&mut s, vec![News { hidden: true, ..news(CHAT, "m1") }, news(&chat(2), "m2")]);
    assert!(taken_at(&mut s, T).is_empty());
    // Notifications off: nothing, and nothing held for later.
    s.run("teams.settings", &json!({"notify": false})).unwrap();
    arrive(&mut s, vec![news(&chat(3), "m3")]);
    assert!(taken_at(&mut s, T + 60).is_empty());
    assert_eq!(s.teams_next_announcement(), None);
    // Back on: the next is announced.
    s.run("teams.settings", &json!({"notify": true})).unwrap();
    arrive(&mut s, vec![news(&chat(3), "m4")]);
    assert_eq!(chats_of(&taken_at(&mut s, T + 60)), [chat(3)]);
    // Turned off while one waits: it isn't given when its time comes.
    arrive(&mut s, vec![news(&chat(3), "m5")]);
    assert!(s.teams_next_announcement().is_some());
    s.run("teams.settings", &json!({"notify": false})).unwrap();
    assert!(taken_at(&mut s, T + 70).is_empty());
    s.run("teams.settings", &json!({"notify": true})).unwrap();
    assert!(taken_at(&mut s, T + 80).is_empty(), "forgotten, not kept for later");
}

#[test]
fn the_open_chat_in_a_focused_window_is_marked_read_not_announced() {
    let mut s = at_t(3);
    looking_at(&mut s, CHAT);
    s.teams_focus(true);
    arrive(&mut s, vec![news(CHAT, "m1"), news(CHAT, "m2")]);
    assert!(s.take_teams_announcements().is_empty());
    assert_eq!(teams_jobs(&mut s), [mark_read(CHAT)], "once for the event");
    // Another chat's news: announced, and nothing is marked read.
    arrive(&mut s, vec![news(&chat(2), "m3")]);
    assert_eq!(chats_of(&s.take_teams_announcements()), [chat(2)]);
    assert!(teams_jobs(&mut s).is_empty());
    // Another module shown: the open chat isn't looked at, its news are announced.
    s.run("view.module", &json!({"name": "mail"})).unwrap();
    arrive(&mut s, vec![news(CHAT, "m4")]);
    assert_eq!(chats_of(&s.take_teams_announcements()), [CHAT]);
    assert!(teams_jobs(&mut s).is_empty());
    // A chat Teams doesn't let be read here, looked at: not announced, and never marked read.
    let mut unreadable = row(3, 10);
    unreadable.unreadable = true;
    s.apply(teams_event(teams::Event::Chats { rows: vec![unreadable] }));
    looking_at(&mut s, &chat(3));
    arrive(&mut s, vec![news(&chat(3), "")]);
    assert!(s.take_teams_announcements().is_empty());
    assert!(teams_jobs(&mut s).is_empty());
}

/// Review Focus 4: the chat is open, the window in the background (in the tray, another
/// application in front): its new messages are announced, and it isn't marked read.
#[test]
fn the_open_chat_unfocused_is_announced_and_not_marked_read() {
    let mut s = at_t(1);
    looking_at(&mut s, CHAT);
    s.teams_focus(false);
    arrive(&mut s, vec![news(CHAT, "m1")]);
    assert_eq!(chats_of(&s.take_teams_announcements()), [CHAT]);
    assert!(teams_jobs(&mut s).is_empty(), "not marked read until the user looks");
    s.teams_focus(false);
    assert!(teams_jobs(&mut s).is_empty());
}

/// Decision 4 (I4): the open chat looked at is marked read whatever the notifications' settings.
#[test]
fn with_notifications_off_the_open_chat_is_still_marked_read() {
    let mut s = at_t(1);
    s.run("teams.settings", &json!({"notify": false, "notifyOnlyDirect": true})).unwrap();
    looking_at(&mut s, CHAT);
    arrive(&mut s, vec![news(CHAT, "m1")]);
    assert_eq!(teams_jobs(&mut s), [mark_read(CHAT)]);
    assert!(s.take_teams_announcements().is_empty());
}

/// I5: back to the window on the open chat with messages unread (announced while it was in the
/// background): marked read, as the user now sees them.
#[test]
fn the_open_chat_is_marked_read_when_the_window_is_focused_again() {
    let mut s = at_t(2);
    looking_at(&mut s, CHAT);
    s.teams_focus(false);
    arrive(&mut s, vec![news(CHAT, "m1")]);
    assert_eq!(s.take_teams_announcements().len(), 1);
    let mut unread = row(1, 20);
    unread.unread = 1;
    s.apply(teams_event(teams::Event::Chats { rows: vec![unread.clone()] }));
    s.teams_focus(false);
    assert!(teams_jobs(&mut s).is_empty());
    s.teams_focus(true);
    assert_eq!(teams_jobs(&mut s), [mark_read(CHAT)]);
    // Focused still: said each frame, asked once.
    s.teams_focus(true);
    assert!(teams_jobs(&mut s).is_empty());
    // Back while another module is shown: not looked at, not marked read.
    s.teams_focus(false);
    s.run("view.module", &json!({"name": "mail"})).unwrap();
    s.teams_focus(true);
    assert!(teams_jobs(&mut s).is_empty());
    // Nothing unread: nothing asked.
    s.run("view.module", &json!({"name": "conversations"})).unwrap();
    s.apply(teams_event(teams::Event::Chats { rows: vec![row(1, 20)] }));
    s.teams_focus(false);
    s.teams_focus(true);
    assert!(teams_jobs(&mut s).is_empty());
    // A chat Teams doesn't let be read here: never marked read.
    unread.unreadable = true;
    s.apply(teams_event(teams::Event::Chats { rows: vec![unread] }));
    s.teams_focus(false);
    s.teams_focus(true);
    assert!(teams_jobs(&mut s).is_empty());
}

/// News waiting their chat's turn aren't announced once the chat has been on screen: the user
/// came back to the window, or opened the chat, and saw them (even if they looked away again).
#[test]
fn news_held_for_a_chat_then_looked_at_are_not_announced() {
    // Back to the window: announced in the background at T, one more held at T + 1.
    let mut s = at_t(2);
    looking_at(&mut s, CHAT);
    s.teams_focus(false);
    arrive(&mut s, vec![news(CHAT, "m1")]);
    assert_eq!(taken_at(&mut s, T).len(), 1);
    s.set_clock(T + 1, 0);
    arrive(&mut s, vec![news(CHAT, "m2")]);
    assert_eq!(s.teams_next_announcement(), Some(T + 5));
    s.set_clock(T + 2, 0);
    s.teams_focus(true);
    assert!(taken_at(&mut s, T + 5).is_empty());
    assert_eq!(s.teams_next_announcement(), None);
    // Looked away again before the turn came: still not announced.
    s.teams_focus(false);
    s.set_clock(T + 6, 0);
    arrive(&mut s, vec![news(CHAT, "m3")]);
    assert_eq!(chats_of(&s.take_teams_announcements()), [CHAT], "a new one is");
    s.set_clock(T + 7, 0);
    arrive(&mut s, vec![news(CHAT, "m4")]);
    s.teams_focus(true);
    s.teams_focus(false);
    assert!(taken_at(&mut s, T + 11).is_empty());

    // The chat opened: announced while another was open at T, one more held at T + 1, opened at
    // T + 2, the other opened again at T + 3.
    let mut s = at_t(2);
    looking_at(&mut s, &chat(2));
    arrive(&mut s, vec![news(CHAT, "m1")]);
    assert_eq!(taken_at(&mut s, T).len(), 1);
    s.set_clock(T + 1, 0);
    arrive(&mut s, vec![news(CHAT, "m2")]);
    s.set_clock(T + 2, 0);
    open(&mut s, CHAT);
    s.set_clock(T + 3, 0);
    open(&mut s, &chat(2));
    assert!(taken_at(&mut s, T + 5).is_empty());
    assert_eq!(s.teams_next_announcement(), None);
}

/// P10: coming back to Conversations (by the rail, Ctrl+6…) on the open chat with messages
/// unread marks it read, as coming back to the window does: once each time it comes on screen.
#[test]
fn coming_back_to_conversations_marks_the_open_chat_read() {
    let mut s = at_t(1);
    looking_at(&mut s, CHAT);
    s.run("view.module", &json!({"name": "mail"})).unwrap();
    s.teams_focus(true);
    arrive(&mut s, vec![news(CHAT, "m1")]);
    assert_eq!(chats_of(&taken_at(&mut s, T)), [CHAT], "not on screen in Mail: announced");
    s.set_clock(T + 1, 0);
    arrive(&mut s, vec![news(CHAT, "m2")]);
    let mut unread = row(1, 20);
    unread.unread = 2;
    s.apply(teams_event(teams::Event::Chats { rows: vec![unread] }));
    s.teams_focus(true);
    assert!(teams_jobs(&mut s).is_empty(), "still in Mail");
    s.set_clock(T + 2, 0);
    s.run("view.module", &json!({"name": "conversations"})).unwrap();
    s.teams_focus(true);
    assert_eq!(teams_jobs(&mut s), [mark_read(CHAT)]);
    s.teams_focus(true);
    assert!(teams_jobs(&mut s).is_empty(), "asked once while it stays on screen");
    assert!(taken_at(&mut s, T + 5).is_empty(), "what waited was seen");
    // A new message while it's on screen: marked read for it, once (not also for coming back).
    arrive(&mut s, vec![news(CHAT, "m3")]);
    assert_eq!(teams_jobs(&mut s), [mark_read(CHAT)]);
    // Away and back again, still unread as far as the list knows: asked again, once.
    s.run("view.module", &json!({"name": "mail"})).unwrap();
    s.teams_focus(true);
    s.run("view.module", &json!({"name": "conversations"})).unwrap();
    s.teams_focus(true);
    s.teams_focus(true);
    assert_eq!(teams_jobs(&mut s), [mark_read(CHAT)]);
    // Come back to with nothing unread: nothing asked.
    s.apply(teams_event(teams::Event::Chats { rows: vec![row(1, 20)] }));
    s.run("view.module", &json!({"name": "mail"})).unwrap();
    s.teams_focus(true);
    s.run("view.module", &json!({"name": "conversations"})).unwrap();
    s.teams_focus(true);
    assert!(teams_jobs(&mut s).is_empty());
    // Opened with messages unread: marked read once, by the opening.
    let mut unread = row(1, 20);
    unread.unread = 1;
    s.apply(teams_event(teams::Event::Chats { rows: vec![unread] }));
    open(&mut s, CHAT);
    s.teams_focus(true);
    assert_eq!(teams_jobs(&mut s), [teams::Job::Open { chat: Some(CHAT.into()) }, mark_read(CHAT)]);
}

#[test]
fn only_direct_or_mentions_when_set() {
    let mut s = at_t(3);
    s.run("teams.settings", &json!({"notifyOnlyDirect": true})).unwrap();
    let group = news(CHAT, "m1");
    let direct = News { direct: true, chat_name: None, ..news(&chat(2), "m2") };
    let mention = News { mentions_me: true, ..news(&chat(3), "m3") };
    arrive(&mut s, vec![group, direct, mention]);
    assert_eq!(chats_of(&s.take_teams_announcements()), [chat(2), chat(3)]);
}

#[test]
fn one_chats_news_are_grouped_five_seconds_apart() {
    let mut s = at_t(1);
    arrive(&mut s, vec![news(CHAT, "m1")]);
    assert_eq!(taken_at(&mut s, T).iter().map(|a| a.count).collect::<Vec<_>>(), [1]);
    s.set_clock(T + 1, 0);
    arrive(&mut s, vec![News { mentions_me: true, text: "@you look".into(), ..news(CHAT, "m2") }]);
    assert!(s.take_teams_announcements().is_empty());
    s.set_clock(T + 2, 0);
    arrive(&mut s, vec![News { text: "third".into(), ..news(CHAT, "m3") }]);
    assert!(s.take_teams_announcements().is_empty());
    assert_eq!(s.teams_next_announcement(), Some(T + 5));
    assert!(taken_at(&mut s, T + 4).is_empty());
    let grouped = taken_at(&mut s, T + 5);
    assert_eq!(grouped.iter().map(|a| (a.chat.as_str(), a.count, a.chats)).collect::<Vec<_>>(), [(CHAT, 2, 1)]);
    // A mention among them isn't lost behind a later message: the one shown is the mention.
    assert!(grouped[0].mentioned);
    assert_eq!(grouped[0].text, "@you look");
    assert_eq!(s.teams_next_announcement(), None);
    // The next one 5 s after that.
    s.set_clock(T + 6, 0);
    arrive(&mut s, vec![news(CHAT, "m4")]);
    assert!(s.take_teams_announcements().is_empty());
    assert_eq!(s.teams_next_announcement(), Some(T + 10));
    assert_eq!(taken_at(&mut s, T + 10).iter().map(|a| a.count).collect::<Vec<_>>(), [1]);
    // Several in one event, after a quiet while: the first alone, the others 5 s later.
    s.set_clock(T + 30, 0);
    arrive(&mut s, vec![news(CHAT, "m5"), news(CHAT, "m6"), news(CHAT, "m7")]);
    assert_eq!(s.take_teams_announcements().iter().map(|a| a.count).collect::<Vec<_>>(), [1]);
    assert_eq!(taken_at(&mut s, T + 35).iter().map(|a| a.count).collect::<Vec<_>>(), [2]);
}

/// Review Focus 3: a burst (the computer waking after a night) gives at most 4 announcements per
/// 5 s, the rest summed up at the start of the next 5 s, among its 4.
#[test]
fn a_burst_is_four_at_most_then_a_summary() {
    let mut s = at_t(14);
    arrive(&mut s, (1..=10).map(|i| news(&chat(i), "m1")).collect());
    let first = taken_at(&mut s, T);
    assert_eq!(chats_of(&first), [chat(1), chat(2), chat(3), chat(4)]);
    assert!(first.iter().all(|a| a.count == 1 && a.chats == 1));
    assert_eq!(s.teams_next_announcement(), Some(T + 5));
    for now in T + 1..T + 5 {
        assert!(taken_at(&mut s, now).is_empty(), "at {now}");
    }
    s.set_clock(T + 5, 0);
    // Four more chats at the start of the next 5 s: the summary counts among its 4.
    arrive(&mut s, (11..=14).map(|i| news(&chat(i), "m1")).collect());
    let next = s.take_teams_announcements();
    assert_eq!(next.len(), 4, "{next:?}");
    assert_eq!((next[0].chat.as_str(), next[0].chats, next[0].count), ("", 6, 6), "the summary first");
    assert_eq!(chats_of(&next[1..]), [chat(11), chat(12), chat(13)]);
    assert!(taken_at(&mut s, T + 9).is_empty());
    // A summary of one chat is that chat's own announcement.
    let last = taken_at(&mut s, T + 10);
    assert_eq!(
        last.iter().map(|a| (a.chat.as_str(), a.count, a.chats, a.sender.as_str())).collect::<Vec<_>>(),
        [(chat(14).as_str(), 1, 1, "Marie Durand")]
    );
    assert_eq!(s.teams_next_announcement(), None);
}

#[test]
fn the_same_message_is_announced_once() {
    let mut s = at_t(3);
    arrive(&mut s, vec![news(CHAT, "m1")]);
    assert_eq!(taken_at(&mut s, T).len(), 1);
    s.set_clock(T + 10, 0);
    arrive(&mut s, vec![news(CHAT, "m1")]);
    assert!(taken_at(&mut s, T + 20).is_empty());
    // A chat whose messages can't be read: its new last message is known by its time.
    let preview = |at: i64| News { message: String::new(), at, ..news(&chat(2), "") };
    arrive(&mut s, vec![preview(5_000), preview(5_000)]);
    assert_eq!(taken_at(&mut s, T + 30).iter().map(|a| a.count).collect::<Vec<_>>(), [1]);
    assert!(taken_at(&mut s, T + 40).is_empty(), "the second was the same");
    arrive(&mut s, vec![preview(6_000)]);
    assert_eq!(taken_at(&mut s, T + 40).len(), 1);
    // A message id that reads like a preview's time isn't taken for it.
    arrive(&mut s, vec![news(&chat(2), "6000")]);
    assert_eq!(taken_at(&mut s, T + 50).len(), 1);
    // At most 2,000 remembered: past them, the first is forgotten.
    s.set_clock(T + 60, 0);
    arrive(&mut s, (0..2_000).map(|i| news(&chat(3), &format!("x{i}"))).collect());
    assert_eq!(taken_at(&mut s, T + 70).iter().map(|a| a.count).sum::<u32>(), 2_000);
    arrive(&mut s, vec![news(CHAT, "m1")]);
    assert_eq!(chats_of(&taken_at(&mut s, T + 80)), [CHAT]);
}

#[test]
fn switched_off_or_signed_out_forgets_what_was_held() {
    let mut s = at_t(10);
    arrive(&mut s, (1..=10).map(|i| news(&chat(i), "m1")).collect());
    s.set_clock(T + 1, 0);
    arrive(&mut s, vec![news(CHAT, "m2")]);
    assert_eq!(s.take_teams_announcements().len(), 4);
    assert!(s.teams_next_announcement().is_some(), "a summary and a group held");
    s.run("teams.settings", &json!({"on": false})).unwrap();
    assert_eq!(s.teams_next_announcement(), None);
    assert!(taken_at(&mut s, T + 5).is_empty());
    s.run("teams.settings", &json!({"on": true})).unwrap();
    assert!(taken_at(&mut s, T + 10).is_empty(), "forgotten while off");
    // On again: what comes is announced; then signed out, what was held is forgotten.
    arrive(&mut s, vec![news(CHAT, "m3")]);
    s.set_clock(T + 11, 0);
    arrive(&mut s, vec![news(CHAT, "m4")]);
    assert_eq!(s.take_teams_announcements().len(), 1);
    assert!(s.teams_next_announcement().is_some());
    s.apply(teams_event(teams::Event::Status { status: Status::SignedOut }));
    assert_eq!(s.teams_next_announcement(), None);
    assert!(taken_at(&mut s, T + 20).is_empty());
}

#[test]
fn bounded() {
    let mut s = at_t(0);
    arrive(&mut s, (0..5_000).map(|i| news(&chat(i), "m1")).collect());
    assert!(s.teams.announcements_held() <= 1_000, "{}", s.teams.announcements_held());
    assert_eq!(taken_at(&mut s, T).len(), 4);
    // None lost to the bound: the summary counts every one.
    let summary = taken_at(&mut s, T + 5);
    assert_eq!(summary.iter().map(|a| (a.chats, a.count)).collect::<Vec<_>>(), [(4_996, 4_996)]);
    // A window that takes nothing for a long while (news coming all along): at most 20 wait.
    for k in 0..200 {
        s.set_clock(T + 10 + 5 * k, 0);
        arrive(&mut s, (0..6).map(|i| news(&chat(10_000 + 6 * k as usize + i), "m1")).collect());
    }
    let taken = taken_at(&mut s, T + 10 + 5 * 200);
    assert!(!taken.is_empty() && taken.len() <= 20, "{}", taken.len());
    assert!(s.teams.announcements_held() <= 1_000);
}
