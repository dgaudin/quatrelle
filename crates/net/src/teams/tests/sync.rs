// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The synchronisation against the fake Graph server, the worker driven by hand at the times the
//! test makes: the cadences (the open chat, the list, the week's chats), incremental reads and
//! their watermark, read state, the chats Graph refuses and those the user left, names when the
//! members are refused, and a 429.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use quatrelle_model::pim::teams::{self as model, ChatMessage, ChatRow, Job, NO_NAME};

use super::worker::{Kept, Seen, account, by_hand, grant, teams_of, turn_until};
use super::{CHAT, MINUTE, T0, TENANT, chat, hits, me, serve_teams, teams_with};
use crate::fake_graph::{FakeChat, FakeMessage, FakeTeams, FakeUser, Server, rfc3339};
use crate::teams::limits::Cadence;
use crate::teams::store::ChatRecord;
use crate::teams::worker::Worker;

/// The chats, their reads and the Active pass.
mod passes;

const DAY: i64 = 24 * 60 * MINUTE;
/// The fake's clock: 40 days after the fixtures' messages (none of them recent).
const NOW: i64 = T0 + 40 * DAY;
const OTHER: &str = "19:c3d4@thread.v2";
const GROUP: &str = "19:g5h6@thread.v2";
/// The step the test's clock moves by between two turns.
const STEP: Duration = Duration::from_millis(100);

fn ana() -> FakeUser {
    FakeUser::new("u-ana", "Ana Lima")
}

/// A group chat of the user, Ana and Bruno, with one message of Ana's at `at`.
fn group(id: &str, topic: Option<&str>, at: i64) -> FakeChat {
    let mut c = FakeChat::new(id, "group", topic, vec![me(), ana(), FakeUser::new("u-bruno", "Bruno Rossi")]);
    c.messages.push(FakeMessage::new(&at.to_string(), Some("u-ana"), &rfc3339(at), "<p>Agenda</p>"));
    c
}

/// A worker signed in by hand to a fake Graph whose clock stands at [`NOW`], and what it said.
struct Run {
    w: Worker,
    server: Server,
    seen: Seen,
    t0: Instant,
    /// How far past `t0` the last turn was.
    at: Duration,
    _creds: Arc<Kept>,
}

/// [`Run`], signed in and its first pass over (five seconds of turns).
fn run(mut teams: FakeTeams) -> Run {
    teams.clock.get_or_insert(NOW);
    let server = serve_teams(teams);
    let (mut w, _, seen, creds) = by_hand(&server, grant(Some(TENANT)));
    let t0 = Instant::now();
    w.job(Job::SignIn { client_id: super::worker::CLIENT.into(), tenant: "organizations".into() });
    turn_until(&mut w, t0, "the account", |_| account(&teams_of(&seen)).is_some());
    let mut r = Run { w, server, seen, t0, at: Duration::ZERO, _creds: creds };
    r.turns(Duration::from_secs(5));
    r
}

impl Run {
    /// Turns, a [`STEP`] apart, for `span` of the test's clock.
    fn turns(&mut self, span: Duration) {
        self.turns_every(span, STEP);
    }

    fn turns_every(&mut self, span: Duration, step: Duration) {
        let end = self.at + span;
        while self.at < end {
            self.at += step;
            self.w.turn(self.t0 + self.at);
        }
    }

    fn fake<T>(&self, change: impl FnOnce(&mut FakeTeams) -> T) -> T {
        change(self.server.data.lock().unwrap().teams.as_mut().unwrap())
    }

    /// The reads of a chat's messages the fake had.
    fn reads(&self, chat: &str) -> usize {
        self.fake(|t| t.message_reads.iter().filter(|(c, _)| c == chat).count())
    }

    fn hits(&self, route: &str) -> u32 {
        hits(&self.server, route)
    }

    fn all_hits(&self) -> u32 {
        self.fake(|t| t.hits.values().sum())
    }

    fn events(&self) -> Vec<model::Event> {
        teams_of(&self.seen)
    }

    /// The last row said of each chat.
    fn rows(&self) -> HashMap<String, ChatRow> {
        let mut rows = HashMap::new();
        for e in self.events() {
            if let model::Event::Chats { rows: said } = e {
                for row in said {
                    rows.insert(row.id.clone(), row);
                }
            }
        }
        rows
    }

    fn row(&self, chat: &str) -> ChatRow {
        self.rows().remove(chat).unwrap_or_else(|| panic!("no row for {chat}: {:?}", self.events()))
    }

    /// The messages said of a chat after event number `from`: (messages, fresh, older).
    fn messages(&self, chat: &str, from: usize) -> Vec<(Vec<ChatMessage>, bool, bool)> {
        self.events()
            .into_iter()
            .skip(from)
            .filter_map(|e| match e {
                model::Event::Messages { chat: c, messages, fresh, older } if c == chat => Some((messages, fresh, older)),
                _ => None,
            })
            .collect()
    }

    fn record(&self, chat: &str) -> ChatRecord {
        self.w.copy.as_ref().unwrap().chat(chat).unwrap()
    }

    /// A message of `from` added to a chat of the fake at `at`, the fake's clock moved past it.
    fn post(&self, chat: &str, from: &str, at: i64, text: &str) {
        self.fake(|t| {
            t.clock = Some(t.clock.unwrap_or(NOW).max(at + 1000));
            t.chat_mut(chat).unwrap().messages.push(FakeMessage::new(&at.to_string(), Some(from), &rfc3339(at), &format!("<p>{text}</p>")));
        });
    }
}

fn ids(messages: &[(Vec<ChatMessage>, bool, bool)]) -> Vec<String> {
    messages.iter().flat_map(|(m, _, _)| m.iter().map(|m| m.id.clone())).collect()
}

#[test]
fn a_new_message_arrives_within_one_list_pass() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 2)]));
    let before = r.row(CHAT).unread;
    let from = r.events().len();
    let at = NOW + MINUTE;
    r.post(CHAT, "u-ana", at, "Lunch at noon?");
    r.turns(Duration::from_secs(11));
    let said = r.messages(CHAT, from);
    assert_eq!(ids(&said), [at.to_string()], "{said:?}");
    assert!(said.iter().all(|(_, fresh, older)| *fresh && !older), "news, to be announced: {said:?}");
    let row = r.row(CHAT);
    assert_eq!(row.unread, before + 1, "{row:?}");
    assert!(row.last_line.contains("Lunch at noon?"), "{row:?}");
    // In the copy, and read from the watermark on.
    assert!(r.w.copy.as_ref().unwrap().last_pages(CHAT, 2).iter().any(|m| m.id == at.to_string()));
    let (_, filter) = r.fake(|t| t.message_reads.last().cloned()).unwrap();
    assert!(filter.starts_with("lastModifiedDateTime gt "), "{filter}");

    // Read elsewhere (the read state moved): nothing unread.
    r.fake(|t| t.chat_mut(CHAT).unwrap().read_time = rfc3339(at + 500));
    r.turns(Duration::from_secs(11));
    assert_eq!(r.row(CHAT).unread, 0);
}

#[test]
fn the_open_chat_is_read_every_three_seconds_and_only_it() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 2), chat(OTHER, T0 + MINUTE, 2)]));
    let at = NOW + MINUTE;
    r.post(CHAT, "u-ana", at, "Ready");
    r.turns(Duration::from_secs(11));
    let (open, other) = (r.reads(CHAT), r.reads(OTHER));

    // Opened: its copy at once, before anything is asked.
    let (from, asked) = (r.events().len(), r.all_hits());
    r.w.job(Job::Open { chat: Some(CHAT.into()) });
    let said = r.messages(CHAT, from);
    assert_eq!(r.all_hits(), asked);
    assert!(ids(&said).contains(&at.to_string()) && said.iter().all(|(_, fresh, older)| !fresh && !older), "{said:?}");

    // Then every three seconds, and no other chat any more often.
    r.turns(Duration::from_secs(30));
    let reads = r.reads(CHAT) - open;
    assert!((10..=11).contains(&reads), "{reads} reads of the open chat in 30 s");
    assert_eq!(r.reads(OTHER), other, "the other chat isn't read for being listed");

    // Closed: no more.
    r.w.job(Job::Open { chat: None });
    let now = r.reads(CHAT);
    r.turns(Duration::from_secs(12));
    assert_eq!(r.reads(CHAT), now);
}

#[test]
fn a_chat_refused_from_its_first_read_is_listed_unreadable_and_not_read_again() {
    // Recent, so the week's pass would read it too.
    let mut teams = teams_with(vec![chat(CHAT, NOW - 10 * MINUTE, 3), chat(OTHER, T0, 2)]);
    teams.forbid_reads.push(CHAT.into());
    let mut r = run(teams);
    let row = r.row(CHAT);
    assert!(row.unreadable && !row.left, "{row:?}");
    assert!(row.last_line.contains("Message 2"), "its preview on its last line: {row:?}");
    assert_eq!(row.unread, 1, "unread, by its preview: Ana's, after it was last read");
    assert!(!r.row(OTHER).unreadable);
    assert_eq!(r.reads(CHAT), 1);

    // Ten list passes, the week's pass, and a minute of the open chat's cadence on it.
    r.w.job(Job::Open { chat: Some(CHAT.into()) });
    r.turns(Duration::from_secs(101));
    assert_eq!(r.reads(CHAT), 1, "not read again the same day");
    assert!(r.hits("chats") >= 10);

    // A day later by the server's clock: tried once more.
    r.fake(|t| t.clock = Some(NOW + DAY));
    r.turns(Duration::from_secs(11));
    assert_eq!(r.reads(CHAT), 2);
    assert!(r.row(CHAT).unreadable, "still refused");

    // Served at last: the first page that comes clears it.
    r.w.job(Job::Open { chat: None });
    r.fake(|t| {
        t.forbid_reads.clear();
        t.clock = Some(NOW + 2 * DAY + MINUTE);
    });
    r.turns(Duration::from_secs(11));
    assert_eq!(r.reads(CHAT), 3);
    assert!(!r.row(CHAT).unreadable);
}

#[test]
fn a_chat_read_before_then_refused_is_left() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 2)]));
    assert!(!r.row(CHAT).left && r.record(CHAT).watermark.is_some());
    let from = r.events().len();
    r.fake(|t| t.forbid_reads.push(CHAT.into()));
    r.post(CHAT, "u-ana", NOW + MINUTE, "Bye");
    r.turns(Duration::from_secs(11));
    let removed = r.events().into_iter().skip(from).filter(|e| matches!(e, model::Event::Removed { chat } if chat == CHAT)).count();
    assert_eq!(removed, 1, "{:?}", r.events());
    let row = r.row(CHAT);
    assert!(row.left && !row.unreadable, "kept, read-only: {row:?}");
    // Not asked again for nothing.
    let reads = r.reads(CHAT);
    r.w.job(Job::Open { chat: Some(CHAT.into()) });
    r.turns(Duration::from_secs(31));
    assert_eq!(r.reads(CHAT), reads);
}

#[test]
fn a_rename_reads_the_members_again() {
    let mut r = run(teams_with(vec![group(GROUP, None, T0)]));
    assert_eq!(r.hits("members"), 1);
    assert_eq!(r.row(GROUP).name, "Ana, Bruno");
    // Nothing moved: not read again.
    r.turns(Duration::from_secs(21));
    assert_eq!(r.hits("members"), 1);

    // A member added (`lastUpdatedDateTime` moves with it).
    r.fake(|t| {
        let c = t.chat_mut(GROUP).unwrap();
        c.members.push(FakeUser::new("u-chloe", "Chloé Martin"));
        c.updated = Some(rfc3339(NOW + MINUTE));
    });
    r.turns(Duration::from_secs(11));
    assert_eq!(r.hits("members"), 2);
    assert_eq!((r.row(GROUP).name.as_str(), r.row(GROUP).members), ("Ana, Bruno, Chloé", 4));

    // Renamed.
    r.fake(|t| t.rename(GROUP, "Budget 2027"));
    r.turns(Duration::from_secs(11));
    assert_eq!(r.hits("members"), 3);
    assert_eq!(r.row(GROUP).name, "Budget 2027");
}

#[test]
fn members_refused_name_falls_back() {
    let mut teams = teams_with(vec![group(GROUP, None, T0), group(OTHER, Some("Roadmap"), T0 + MINUTE), chat(CHAT, T0 + 2 * MINUTE, 2)]);
    teams.forbid_members = vec![GROUP.into(), OTHER.into(), CHAT.into()];
    let r = run(teams);
    assert_eq!(r.row(GROUP).name, NO_NAME, "an untitled group, its members refused");
    assert_eq!(r.row(OTHER).name, "Roadmap", "its topic");
    assert_eq!(r.row(CHAT).name, NO_NAME, "a one-to-one chat whose members are refused");
    // Nobody proven of the organisation: External, until the members say otherwise.
    assert!([GROUP, OTHER, CHAT].iter().all(|c| r.row(c).external), "{:?}", r.rows());
    assert_eq!(r.hits("members"), 3, "each asked once");
    // Read all the same.
    assert!(r.record(GROUP).watermark.is_some());
}

#[test]
fn the_watermark_never_passes_graphs_clock() {
    // A message dated two days after the server's own clock.
    let ahead = NOW + 2 * DAY;
    let mut c = chat(CHAT, T0, 1);
    c.messages.push(FakeMessage::new(&ahead.to_string(), Some("u-ana"), &rfc3339(ahead), "<p>From the future</p>"));
    let mut r = run(teams_with(vec![c]));
    let hour = 3_600_000;
    assert_eq!(r.record(CHAT).watermark, Some(NOW + hour), "Graph's Date plus an hour at most");
    // A watermark read back from a file that says more is held to the same.
    let mut kept = r.record(CHAT);
    kept.watermark = Some(NOW + 300 * DAY);
    r.w.copy.as_ref().unwrap().put_chat(&kept).unwrap();
    let reads = r.reads(CHAT);
    r.w.job(Job::Open { chat: Some(CHAT.into()) });
    r.turns(Duration::from_secs(1));
    assert_eq!(r.reads(CHAT), reads + 1);
    let (_, filter) = r.fake(|t| t.message_reads.last().cloned()).unwrap();
    assert!(filter.starts_with(&format!("lastModifiedDateTime gt {}", &rfc3339(NOW + hour - 2000)[..19])), "{filter}");
    assert_eq!(r.record(CHAT).watermark, Some(NOW + hour));
}

#[test]
fn a_429_slows_that_cadence_only() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 2)]));
    r.fake(|t| t.throttle_once = Some("chats"));
    // Up to the list's 429.
    let until = r.at + Duration::from_secs(15);
    while r.fake(|t| t.throttle_once.is_some()) {
        assert!(r.at < until, "the list was never asked for");
        r.turns(STEP);
    }
    // While its Retry-After (a second) runs, nothing at all is asked: not even the chat opened now.
    let asked = r.all_hits();
    r.w.job(Job::Open { chat: Some(CHAT.into()) });
    r.turns(Duration::from_millis(800));
    assert_eq!(r.all_hits(), asked, "nothing asked during the Retry-After");
    assert_eq!(r.w.limiter.interval(Cadence::List), Duration::from_secs(20));
    assert_eq!(r.w.limiter.interval(Cadence::Open), Duration::from_secs(3));

    // Then the list every twenty seconds, and the open chat still every three.
    r.turns(Duration::from_secs(1));
    let (list, open) = (r.hits("chats"), r.reads(CHAT));
    r.turns(Duration::from_secs(40));
    assert_eq!(r.hits("chats") - list, 2, "every twenty seconds");
    let reads = r.reads(CHAT) - open;
    assert!((13..=14).contains(&reads), "{reads} reads of the open chat in 40 s");
}

#[test]
fn listings_stop_on_a_next_link_seen_before() {
    // The chat list going round in circles (60 chats, its second page's link its own): two
    // pages, not 400, though the second brought chats.
    let mut teams = teams_with((0..60).map(|i| chat(&format!("19:c{i}@thread.v2"), T0 + i * MINUTE, 1)).collect());
    teams.loop_next = Some("chats");
    let mut r = run(teams);
    assert_eq!(r.hits("chats"), 2);
    // Each chat's members and messages: a request a turn.
    r.turns(Duration::from_secs(10));
    assert_eq!((r.hits("chats"), r.rows().len()), (2, 60));

    // A chat's messages going round: two pages, not 200.
    let mut r = run(teams_with(vec![chat(CHAT, T0, 2)]));
    let reads = r.reads(CHAT);
    r.fake(|t| t.loop_next = Some("messages"));
    for i in 0..60 {
        r.post(CHAT, "u-ana", NOW + i * 1000, &format!("Again {i}"));
    }
    r.turns(Duration::from_secs(11));
    assert_eq!(r.reads(CHAT) - reads, 2);
    assert_eq!(r.record(CHAT).watermark, Some(NOW + 59 * 1000), "and the read counts");
}

#[test]
fn a_page_that_brings_nothing_new_ends_the_listing() {
    // Each link new, each page the first again.
    let mut teams = teams_with(vec![chat(CHAT, T0, 2), chat(OTHER, T0 + MINUTE, 2)]);
    teams.repeat_page = Some("chats");
    let r = run(teams);
    assert_eq!(r.hits("chats"), 2);

    let mut r = run(teams_with(vec![chat(CHAT, T0, 2)]));
    let reads = r.reads(CHAT);
    r.fake(|t| t.repeat_page = Some("messages"));
    r.post(CHAT, "u-ana", NOW + MINUTE, "Once");
    r.turns(Duration::from_secs(11));
    assert_eq!(r.reads(CHAT) - reads, 2);
}

#[test]
fn rows_wait_for_the_users_own_id() {
    // Without it the user would be among a chat's others: named by their own name, say.
    let mut r = run(teams_with(vec![group(GROUP, None, T0)]));
    let from = r.events().len();
    let me = std::mem::take(&mut r.w.settings.object_id);
    r.fake(|t| t.rename(GROUP, "Retro"));
    r.turns(Duration::from_secs(11));
    assert!(!r.events().iter().skip(from).any(|e| matches!(e, model::Event::Chats { .. })), "{:?}", r.events());
    r.w.settings.object_id = me;
    r.fake(|t| t.chat_mut(GROUP).unwrap().read_time = rfc3339(NOW));
    r.turns(Duration::from_secs(11));
    assert_eq!(r.row(GROUP).name, "Retro");
}

#[test]
fn nothing_is_kept_or_said_under_an_empty_id() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 2)]));
    r.post(CHAT, "u-ana", NOW + MINUTE, "Kept");
    r.fake(|t| {
        let at = NOW + 2 * MINUTE;
        t.chat_mut(CHAT).unwrap().messages.push(FakeMessage::new("", Some("u-ana"), &rfc3339(at), "<p>Nameless</p>"));
    });
    let from = r.events().len();
    r.turns(Duration::from_secs(11));
    let said = r.messages(CHAT, from);
    assert_eq!(ids(&said), [(NOW + MINUTE).to_string()], "{said:?}");
    assert!(r.w.copy.as_ref().unwrap().last_pages(CHAT, 2).iter().all(|m| !m.id.is_empty()));
    assert!(r.record(CHAT).unread.iter().all(|(id, _)| !id.is_empty()));
    assert!(r.rows().keys().all(|id| !id.is_empty()));
}

#[test]
fn hidden_chats_are_synced_not_listed() {
    let mut c = chat(CHAT, T0, 2);
    c.hidden = true;
    let mut r = run(teams_with(vec![c, chat(OTHER, T0, 2)]));
    assert!(r.row(CHAT).hidden && !r.row(OTHER).hidden);
    let from = r.events().len();
    let at = NOW + MINUTE;
    r.post(CHAT, "u-ana", at, "Still here");
    r.turns(Duration::from_secs(11));
    assert_eq!(ids(&r.messages(CHAT, from)), [at.to_string()]);
    assert!(r.w.copy.as_ref().unwrap().last_pages(CHAT, 2).iter().any(|m| m.id == at.to_string()), "copied");
    let row = r.row(CHAT);
    assert!(row.hidden && row.unread == 1, "{row:?}");
}

#[test]
fn a_stranger_sender_reads_the_members_again_before_the_row_says_who() {
    let mut r = run(teams_with(vec![group(GROUP, Some("Ops"), T0)]));
    assert_eq!(r.hits("members"), 1);
    assert!(!r.row(GROUP).external);
    // A guest added to the chat, the list not saying so yet; their message says they're of the
    // organisation, as a guest account's does.
    r.fake(|t| {
        t.guests.push("u-gale".into());
        t.chat_mut(GROUP).unwrap().members.push(FakeUser::new("u-gale", "Gale Stone"));
    });
    let from = r.events().len();
    r.post(GROUP, "u-gale", NOW + MINUTE, "Hi all");
    r.turns(Duration::from_secs(11));
    assert_eq!(r.hits("members"), 2, "the members read again");
    let row = r.row(GROUP);
    assert!(row.last_external && row.external, "a guest, as the members say: {row:?}");
    assert!(row.last_line.starts_with("Gale"), "{row:?}");
    assert_eq!(ids(&r.messages(GROUP, from)), [(NOW + MINUTE).to_string()]);

    // Someone not among the members at all: read again once, then judged by the message.
    for (i, text) in ["Hello", "Hello again"].iter().enumerate() {
        r.post(GROUP, "u-gone", NOW + (2 + i as i64) * MINUTE, text);
        r.turns(Duration::from_secs(11));
    }
    assert_eq!(r.hits("members"), 3, "once for that sender");
    assert!(!r.row(GROUP).last_external, "an account of the organisation, by its message: {:?}", r.row(GROUP));
    assert_eq!(r.fake(|t| t.chat(GROUP).map(|c| c.members.len())), Some(4), "they never were");

    // A stranger's message that isn't the last: the members are read again before it goes.
    r.fake(|t| {
        t.guests.push("u-hal".into());
        t.chat_mut(GROUP).unwrap().members.push(FakeUser::new("u-hal", "Hal Brook"));
    });
    let (members, from, theirs) = (r.hits("members"), r.events().len(), (NOW + 4 * MINUTE).to_string());
    r.post(GROUP, "u-hal", NOW + 4 * MINUTE, "Psst");
    r.post(GROUP, "u-ana", NOW + 5 * MINUTE, "Noted");
    let mut when = None;
    for _ in 0..110 {
        r.turns(STEP);
        if when.is_none() && ids(&r.messages(GROUP, from)).contains(&theirs) {
            when = Some(r.hits("members"));
        }
    }
    assert_eq!(when, Some(members + 1), "the members read before the message went");
}

#[test]
fn a_stranger_last_in_a_chat_graph_refuses_reads_its_members_again() {
    // Its messages refused: only the list's preview says who wrote last.
    let mut teams = teams_with(vec![group(GROUP, Some("Ops"), T0)]);
    teams.forbid_reads.push(GROUP.into());
    let mut r = run(teams);
    assert!(r.row(GROUP).unreadable && !r.row(GROUP).last_external);
    r.fake(|t| {
        t.guests.push("u-gale".into());
        t.chat_mut(GROUP).unwrap().members.push(FakeUser::new("u-gale", "Gale Stone"));
    });
    r.post(GROUP, "u-gale", NOW + MINUTE, "Hi all");
    r.turns(Duration::from_secs(11));
    assert_eq!(r.hits("members"), 2);
    let row = r.row(GROUP);
    assert!(row.last_external && row.unreadable && row.last_line == "Gale: Hi all", "{row:?}");
}

#[test]
fn incremental_reads_a_chat_a_page_a_step_to_its_end() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 2)]));
    for i in 0..120 {
        r.post(CHAT, "u-ana", NOW + i * 1000, &format!("Line {i}"));
    }
    let (reads, from) = (r.reads(CHAT), r.events().len());
    let mut over = Vec::new();
    for second in 0..4 {
        let at = r.t0 + r.at + Duration::from_secs(second);
        over.push(crate::teams::sync::incremental(&mut r.w, CHAT, at).unwrap());
    }
    assert_eq!(over, [false, false, true, true], "three pages of 50, a second apart; then a read of its own");
    assert_eq!(r.reads(CHAT) - reads, 4);
    assert_eq!(ids(&r.messages(CHAT, from)).len(), 120);
    assert!(r.messages(CHAT, from).iter().all(|(m, fresh, _)| m.len() <= 50 && *fresh));
    assert_eq!(r.record(CHAT).watermark, Some(NOW + 119 * 1000));
    assert_eq!(r.record(CHAT).unread.len(), 120);
}
