// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The channel check (`--channels`) against the fake Graph server: each line, what a refusal, a
//! tenant without teams and a 429 make of it, its bounds, that it reads only, and that it says
//! nothing private.

use std::cell::RefCell;
use std::collections::HashSet;

use serde_json::json;

use super::*;
use crate::fake_graph::{ChannelRules, FakeChannel, FakeMember, FakeTeam, FakeThread};
use crate::oauth::{TEAMS_CHANNELS_SCOPE, TEAMS_CHANNELS_WRITE_SCOPE, TEAMS_SCOPE};
use crate::teams::{ChannelCheck, run_channel_check};

thread_local! {
    /// What the check waited, in this test's thread.
    static SLEPT: RefCell<Vec<Duration>> = const { RefCell::new(Vec::new()) };
}

fn record(d: Duration) {
    SLEPT.with(|s| s.borrow_mut().push(d));
}

fn slept() -> Vec<Duration> {
    SLEPT.with(|s| s.borrow().clone())
}

/// Every permission granted, as Microsoft lists them (by address).
fn granted_all() -> String {
    format!("{TEAMS_SCOPE} {TEAMS_CHANNELS_SCOPE} {TEAMS_CHANNELS_WRITE_SCOPE}")
}

fn opts() -> ChannelCheck {
    ChannelCheck { tenant: Some(TENANT.into()), granted: granted_all(), sleep: record }
}

/// The channel check run on a fresh client: its result and the lines it said.
fn channel_check(server: &Server, opts: &ChannelCheck) -> (crate::Result<()>, Vec<String>) {
    SLEPT.with(|s| s.borrow_mut().clear());
    let (c, me) = TeamsClient::connect(&server.base, TOKEN).unwrap();
    let mut lines = Vec::new();
    let r = run_channel_check(&c, &me, opts, &mut |l| lines.push(l.to_string()));
    (r, lines)
}

const CH1: &str = "19:general@thread.tacv2";
const CH2: &str = "19:private@thread.tacv2";
const CH3: &str = "19:shared@thread.tacv2";

fn at(minutes: i64) -> String {
    rfc3339(T0 + minutes * MINUTE)
}

/// A message of a channel, its id its time in minutes after `T0`.
fn post(minutes: i64, from: Option<&str>, html: &str) -> FakeMessage {
    FakeMessage::new(&(T0 + minutes * MINUTE).to_string(), from, &at(minutes), html)
}

/// A picture of a channel's message, at the address Graph gives it (`reply`: a reply's).
fn picture(channel: &str, root: i64, reply: Option<i64>) -> String {
    let message = (T0 + root * MINUTE).to_string();
    let reply = reply.map(|r| format!("/replies/{}", T0 + r * MINUTE)).unwrap_or_default();
    let channel = crate::http::pct(channel);
    format!(
        "<p>Look <img src=\"https://graph.microsoft.com/v1.0/teams/t-1/channels/{channel}/messages/{message}{reply}/hostedContents/aW1n/$value\"></p>"
    )
}

/// Two teams, three channels (standard in threads, private in chat layout, shared from another
/// organisation without a layout or a link), five threads, three of them with replies.
fn two_teams() -> FakeTeams {
    let ana = FakeUser::new("u-ana", "Ana Lima");
    let gus = FakeUser::new("u-gus", "Gus Gale");
    let olga = FakeUser::new("u-olga", "Olga Other");
    let mine = || FakeMember::new(me());
    // The standard channel: a thread of two replies with a picture, mentions of me and of the
    // channel, a subject and high importance, its last reply after the next thread's root; a
    // thread of one reply from an app, with a picture in the reply's form; a system message.
    let mut th1 =
        FakeThread::new(post(0, Some("u-ana"), &picture(CH1, 0, None)), vec![post(1, Some(ME), "<p>a</p>"), post(15, Some("u-ana"), "<p>b</p>")]);
    th1.root.mentions = vec![ME.into()];
    th1.other_mentions = vec![json!({"conversation": {"id": CH1, "displayName": "General", "conversationIdentityType": "channel"}})];
    th1.subject = Some("Hello".into());
    th1.importance = "high";
    let mut bot = post(11, None, &picture(CH1, 10, Some(11)));
    bot.from_json =
        Some(json!({"application": {"id": "app-1", "displayName": "Workflows", "applicationIdentityType": "bot"}, "device": null, "user": null}));
    let th2 = FakeThread::new(post(10, Some(ME), "<p>c</p>"), vec![bot]);
    let mut event = post(20, None, "<systemEventMessage/>");
    event.system = true;
    let th3 = FakeThread::new(event, Vec::new());
    let members = vec![
        mine(),
        FakeMember::new(ana.clone()),
        FakeMember { guest: true, ..FakeMember::new(gus) },
        FakeMember { tenant: Some("tenant-2".into()), ..FakeMember::new(olga) },
    ];
    let mut general = FakeChannel::new(CH1, "General", members);
    general.threads = vec![th1, th2, th3];
    // The private channel: a thread of two replies mentioning a team and a tag.
    let mut th4 = FakeThread::new(post(5, Some("u-ana"), "<p>d</p>"), vec![post(6, Some(ME), "<p>e</p>"), post(7, Some("u-ana"), "<p>f</p>")]);
    th4.other_mentions = vec![
        json!({"conversation": {"id": "t-1", "displayName": "Design", "conversationIdentityType": "team"}}),
        json!({"tag": {"id": "tag-1", "displayName": "Designers"}}),
    ];
    let mut private =
        FakeChannel { membership: "private", layout: Some("chat"), ..FakeChannel::new(CH2, "Private", vec![mine(), FakeMember::new(ana)]) };
    private.threads = vec![th4];
    // The shared channel, of another organisation, in an archived team.
    let mut th5 = FakeThread::new(post(30, Some(ME), "<p>g</p>"), Vec::new());
    th5.importance = "urgent";
    let skype =
        FakeMember { kind: Some("#microsoft.graph.skypeForBusinessUserConversationMember"), ..FakeMember::new(FakeUser::new("u-sky", "Sky Pe")) };
    let mut shared = FakeChannel {
        membership: "shared",
        layout: None,
        tenant: Some("tenant-2".into()),
        link: Some(None),
        ..FakeChannel::new(CH3, "Shared", vec![mine(), skype])
    };
    shared.threads = vec![th5];
    let design = FakeTeam::new("t-1", "Design", vec![general, private]);
    let old = FakeTeam { archived: true, ..FakeTeam::new("t-2", "Old", vec![shared]) };
    FakeTeams { teams: vec![design, old], ..teams_with(Vec::new()) }
}

/// The routes that only read: everything else the check must never ask.
const READS: [&str; 7] = ["me", "joinedTeams", "channels", "channelMessages", "channelReplies", "channelMembers", "channelDelta"];

/// Line by line, in the order they come: the permissions, the teams, the channels, the messages
/// of each, what a reply does to its root, the order and the size of a page, the replies route,
/// pictures, mentions, members and the old delta.
#[test]
fn a_granted_account_is_said_line_by_line() {
    let server = serve_teams(two_teams());
    let (r, lines) = channel_check(&server, &opts());
    r.unwrap();
    let starts = [
        "channels, permissions granted:",
        "teams:",
        "channels:",
        "channel messages read:",
        "messages:",
        "from.user:",
        "roots with replies:",
        "the listing is in lastModifiedDateTime order:",
        "a page with $expand=replies:",
        "the replies route:",
        "hosted images:",
        "mentions:",
        "channel members:",
        "the old delta:",
        "in all:",
    ];
    assert_eq!(lines.len(), starts.len(), "{lines:#?}");
    for (l, s) in lines.iter().zip(starts) {
        assert!(l.starts_with(s), "{s:?} expected, {l:?} said: {lines:#?}");
    }
    for said in [
        "channels, permissions granted: Team.ReadBasic.All yes, Channel.ReadBasic.All yes, ChannelMessage.Read.All yes, ChannelMember.Read.All yes, ChannelMessage.Send yes, ChannelMessage.ReadWrite yes",
        "teams: 2 (archived 1)",
        "channels: 3 (standard 1, private 1, shared 1, other 0; of another organisation 1; layout post 1, chat 1, other 0, none 1; webUrl l/channel 2)",
        "channel messages read: 3 of 3 channels (refused 0; why: -)",
        "messages: roots 5, replies expanded 5, system 1, from apps 1; subject 1, importance high 1, urgent 1; webUrl l/message 10",
        "from.user: tenantId present 8, absent 0; the user's organisation 8, another 0; userIdentityType aadUser 8, other 0",
        "roots with replies: 3; whose lastModifiedDateTime is at or after their newest reply: 3",
        "the listing is in lastModifiedDateTime order: yes (out of order 0)",
        "the replies route: newest first 2, oldest first 0, mixed 0 (threads read 2, refused 0)",
        "hosted images: channel form 1, channel reply form 1, reply as a message 0, chat form 0, other 0",
        "mentions: person 1 (the user 1), channel 1, team 1, tag 1, other 0",
        "channel members: read 3 of 3 (refused 0; why: -); organisation 5, guests 1, another organisation 1, other kinds 1; largest channel 4",
        "the old delta: 400 BadRequest",
        // The user, the teams, two channel lists, three pages with replies and one without, two
        // threads' replies, three channels' members and the delta.
        "in all: requests 14, throttled 0",
    ] {
        let start = said.split(':').next().unwrap();
        assert_eq!(line(&lines, &format!("{start}:")), said, "{lines:#?}");
    }
    let page = line(&lines, "a page with $expand=replies:");
    assert!(page.ends_with("KiB; replies expanded per root at most 2; roots whose replies were cut 0"), "{page}");
    let words: Vec<&str> = page.split(' ').collect();
    let kib: Vec<u32> = words.windows(2).filter(|w| w[1].starts_with("KiB")).filter_map(|w| w[0].parse().ok()).collect();
    assert!(kib.len() == 3 && kib.iter().all(|k| (1..100).contains(k)) && kib[0] >= kib[1] && kib[0] >= kib[2], "{page}");
    // `shared` is asked for as it is.
    assert!(headers(&server, "channels").contains(&("prefer".into(), "include-unknown-enum-members".into())));
}

#[test]
fn permissions_are_said_one_by_one() {
    let server = serve_teams(two_teams());
    let short = "User.Read Chat.ReadWrite team.readbasic.all ChannelMessage.Read.All ChannelMessage.Send offline_access";
    let (_, lines) = channel_check(&server, &ChannelCheck { granted: short.into(), ..opts() });
    assert_eq!(
        line(&lines, "channels, permissions granted:"),
        "channels, permissions granted: Team.ReadBasic.All yes, Channel.ReadBasic.All no, ChannelMessage.Read.All yes, ChannelMember.Read.All no, ChannelMessage.Send yes, ChannelMessage.ReadWrite no"
    );
    // A part of a name isn't the name.
    let (_, lines) = channel_check(&server, &ChannelCheck { granted: "ChannelMessage.Read ChannelMessage.ReadWrite.All".into(), ..opts() });
    assert!(
        line(&lines, "channels, permissions granted:")
            .contains("ChannelMessage.Read.All no, ChannelMember.Read.All no, ChannelMessage.Send no, ChannelMessage.ReadWrite no")
    );
    let (_, lines) = channel_check(&server, &ChannelCheck { granted: " ".into(), ..opts() });
    assert_eq!(line(&lines, "channels, permissions granted:"), "channels, permissions granted: not said by the sign-in");
}

/// A 403, wherever Graph says it: on a channel's messages, its members, a team's channels, and
/// on the teams themselves (nothing more is asked then).
#[test]
fn refusals_are_said_by_their_status() {
    let mut t = two_teams();
    t.teams[0].channels[0].forbid_messages = true;
    t.teams[0].channels[1].forbid_members = true;
    t.teams.push(FakeTeam { forbid_channels: true, ..FakeTeam::new("t-3", "Closed", Vec::new()) });
    let server = serve_teams(t);
    let (r, lines) = channel_check(&server, &opts());
    r.unwrap();
    assert_eq!(line(&lines, "teams:"), "teams: 3 (archived 1)");
    assert_eq!(line(&lines, "channel lists:"), "channel lists: asked of 3 teams, refused 1 (why: 403 Forbidden 1)");
    assert!(line(&lines, "channels:").starts_with("channels: at least 3 (standard 1, private 1, shared 1"), "{lines:#?}");
    assert_eq!(line(&lines, "channel messages read:"), "channel messages read: 2 of 3 channels (refused 1; why: 403 Forbidden 1)");
    assert_eq!(
        line(&lines, "messages:"),
        "messages: roots 2, replies expanded 2, system 0, from apps 0; subject 0, importance high 0, urgent 1; webUrl l/message 4"
    );
    assert_eq!(
        line(&lines, "channel members:"),
        "channel members: read 2 of 3 (refused 1; why: 403 Forbidden 1); organisation 3, guests 1, another organisation 1, other kinds 1; largest channel 4"
    );
    // The delta is asked of the first channel read (taken in turn: the shared one).
    assert_eq!(line(&lines, "the old delta:"), "the old delta: 400 BadRequest");
    let asks = teams_of(&server).channel_asks;
    assert!(asks.contains(&("channelDelta".into(), CH3.into())), "{asks:?}");

    let mut t = two_teams();
    t.channel_rules = ChannelRules { forbid_teams: true, ..ChannelRules::default() };
    let server = serve_teams(t);
    let (r, lines) = channel_check(&server, &opts());
    r.unwrap();
    assert_eq!(lines.len(), 3, "{lines:#?}");
    assert_eq!(lines[1], "teams: no (403 Forbidden)");
    assert_eq!(lines[2], "in all: requests 2, throttled 0");
}

#[test]
fn a_tenant_without_teams_says_so() {
    let server = serve_teams(teams_with(Vec::new()));
    let (r, lines) = channel_check(&server, &ChannelCheck { tenant: None, ..opts() });
    r.unwrap();
    for said in [
        "teams: 0 (archived 0)",
        "channels: 0 (standard 0, private 0, shared 0, other 0; of another organisation not known; layout post 0, chat 0, other 0, none 0; webUrl l/channel 0)",
        "channel messages read: 0 of 0 channels (refused 0; why: -)",
        "messages: roots 0, replies expanded 0, system 0, from apps 0; subject 0, importance high 0, urgent 0; webUrl l/message 0",
        "from.user: tenantId present 0, absent 0; the user's organisation not known; userIdentityType aadUser 0, other 0",
        "roots with replies: 0; whose lastModifiedDateTime is at or after their newest reply: 0",
        "the listing is in lastModifiedDateTime order: yes (out of order 0)",
        "a page with $expand=replies: none read",
        "the replies route: not tried (no thread with 2 replies or more)",
        "hosted images: channel form 0, channel reply form 0, reply as a message 0, chat form 0, other 0",
        "mentions: person 0 (the user 0), channel 0, team 0, tag 0, other 0",
        "channel members: read 0 of 0 (refused 0; why: -); organisation 0, guests 0, another organisation not known, other kinds 0; largest channel 0",
        "the old delta: not tried (no channel read)",
        "in all: requests 2, throttled 0",
    ] {
        let start = said.split(':').next().unwrap();
        assert_eq!(line(&lines, &format!("{start}:")), said, "{lines:#?}");
    }
}

/// A 429 is waited out as its `Retry-After` says, then asked again; when Graph goes on asking to
/// wait, or asks for longer than the check waits, the check stops asking and says so.
#[test]
fn a_429_is_waited_out_then_the_check_stops() {
    let mut t = two_teams();
    t.throttle_once = Some("channelMessages");
    t.throttle_retry_after = Some(7);
    let server = serve_teams(t);
    let (r, lines) = channel_check(&server, &opts());
    r.unwrap();
    assert_eq!(line(&lines, "channel messages read:"), "channel messages read: 3 of 3 channels (refused 0; why: -)");
    assert_eq!(line(&lines, "in all:"), "in all: requests 15, throttled 1");
    assert!(slept().contains(&Duration::from_secs(7)), "{:?}", slept());
    assert!(!lines.iter().any(|l| l.starts_with("stopped")), "{lines:#?}");

    // Every request from the fifth on is refused: three tries of the first channel's messages,
    // then nothing more is asked.
    let mut t = two_teams();
    (t.throttle_every, t.throttle_from, t.throttle_retry_after) = (Some(1), 4, Some(2));
    let server = serve_teams(t);
    let (r, lines) = channel_check(&server, &opts());
    r.unwrap();
    assert_eq!(line(&lines, "channel messages read:"), "channel messages read: 0 of 3 channels (refused 1; why: 429 TooManyRequests 1; not asked 2)");
    assert_eq!(
        line(&lines, "stopped early:"),
        "stopped early: Graph still asked to wait (429 TooManyRequests); the lines above count what was read before"
    );
    assert_eq!(line(&lines, "in all:"), "in all: requests 7, throttled 3");
    assert_eq!(slept(), [Duration::from_secs(2), Duration::from_secs(2)]);
    assert_eq!(teams_of(&server).channel_asks.len(), 3);

    // A wait longer than the check's: not waited, nothing more asked.
    let mut t = two_teams();
    (t.throttle_once, t.throttle_retry_after) = (Some("channelMessages"), Some(3600));
    let server = serve_teams(t);
    let (_, lines) = channel_check(&server, &opts());
    assert!(lines.iter().any(|l| l.starts_with("stopped early:")), "{lines:#?}");
    assert_eq!(line(&lines, "in all:"), "in all: requests 5, throttled 1");
    assert!(slept().is_empty(), "{:?}", slept());
}

/// Each of the fake's switches changes the line it is for.
#[test]
fn each_switch_of_graph_changes_its_line() {
    let run = |rules: ChannelRules| {
        let server = serve_teams(FakeTeams { channel_rules: rules, ..two_teams() });
        let (r, lines) = channel_check(&server, &opts());
        r.unwrap();
        lines
    };
    // A reply doesn't move its root: its thread still comes first, its root out of order.
    let lines = run(ChannelRules { root_moves: false, ..ChannelRules::default() });
    assert_eq!(line(&lines, "roots with replies:"), "roots with replies: 3; whose lastModifiedDateTime is at or after their newest reply: 0");
    assert_eq!(line(&lines, "the listing is in lastModifiedDateTime order:"), "the listing is in lastModifiedDateTime order: no (out of order 1)");
    // Replies cut past one.
    let lines = run(ChannelRules { expand_cap: 1, ..ChannelRules::default() });
    assert!(line(&lines, "a page with").ends_with("replies expanded per root at most 1; roots whose replies were cut 2"), "{lines:#?}");
    // The replies route lists the oldest first.
    let lines = run(ChannelRules { replies_oldest_first: true, ..ChannelRules::default() });
    assert_eq!(line(&lines, "the replies route:"), "the replies route: newest first 0, oldest first 2, mixed 0 (threads read 2, refused 0)");
    // The old delta answers.
    let lines = run(ChannelRules { delta_status: 200, ..ChannelRules::default() });
    assert_eq!(line(&lines, "the old delta:"), "the old delta: 200 (a link to go on: yes)");
}

/// An id Graph gives that can't be one segment of an address (a team's or a channel's) is never
/// sent: the client refuses it before any request, and the check says it as an unusable id.
#[test]
fn hostile_ids_are_refused_before_any_request() {
    let mut t = two_teams();
    t.teams[0].channels[0].id = "19:a/../b@thread.tacv2".into();
    t.teams[1].id = "t-2?x=1".into();
    let server = serve_teams(t);
    let (r, lines) = channel_check(&server, &opts());
    r.unwrap();
    assert_eq!(line(&lines, "channel lists:"), "channel lists: asked of 2 teams, refused 1 (why: unusable id 1)");
    assert_eq!(line(&lines, "channel messages read:"), "channel messages read: 1 of 2 channels (refused 1; why: unusable id 1)");
    let t = teams_of(&server);
    assert!(t.channel_asks.iter().all(|(_, c)| c == CH2), "{:?}", t.channel_asks);
    assert_eq!(t.hits.get("channels"), Some(&1));
    assert_eq!(t.hits.get("unknown"), None);

    let c = client(&server);
    let before = teams_of(&server).hits;
    for bad in ["", "..", "a/b", "a?b", "a#b", "a\\b", "a\nb", &"x".repeat(9000)] {
        let refused = |r: crate::Result<Page>| matches!(r, Err(NetError::Refused(_)));
        assert!(refused(c.channels(bad, None)), "{bad:?}");
        assert!(refused(c.channel_messages("t-1", bad, 50, true, None)), "{bad:?}");
        assert!(refused(c.channel_messages(bad, CH1, 50, true, None)), "{bad:?}");
        assert!(refused(c.replies("t-1", CH1, bad, None)), "{bad:?}");
        assert!(refused(c.channel_members(bad, CH1, None)), "{bad:?}");
        assert!(matches!(c.channel_delta_status("t-1", bad), Err(NetError::Refused(_))), "{bad:?}");
    }
    assert_eq!(teams_of(&server).hits, before, "nothing was asked");
    // A next page elsewhere than Graph's own address isn't followed.
    assert!(c.joined_teams(Some("https://elsewhere.example/v1.0/me/joinedTeams?$skiptoken=1")).is_err());
    assert!(c.channel_messages("t-1", CH1, 50, true, Some("https://elsewhere.example/v1.0/teams")).is_err());
    assert_eq!(teams_of(&server).hits, before);
}

/// The check reads, and only reads: every request a `GET` of a reading route, nothing posted.
#[test]
fn the_check_writes_nothing() {
    let server = serve_teams(two_teams());
    let (r, _) = channel_check(&server, &opts());
    r.unwrap();
    let t = teams_of(&server);
    assert_eq!(t.methods.keys().collect::<Vec<_>>(), ["GET"], "{:?}", t.methods);
    assert_eq!(t.methods.get("GET"), Some(&14), "every request of the check, each a read");
    assert!(t.hits.keys().all(|k| READS.contains(&k.as_str())), "{:?}", t.hits);
    assert!(t.writes.is_empty() && t.posts.is_empty());
}

/// At most: twenty channels, taken in turn from the teams; 25 pages of a channel's members; ten
/// pages of teams, the channels of 200 teams. Two requests about one channel are a second apart.
#[test]
fn the_check_keeps_to_its_bounds() {
    let channel = |team: usize, i: usize| {
        let mut c = FakeChannel::new(&format!("19:t{team}c{i}@thread.tacv2"), "C", vec![FakeMember::new(me())]);
        c.threads = vec![FakeThread::new(post(i as i64, Some(ME), "<p>x</p>"), vec![post(i as i64 + 1, Some(ME), "<p>y</p>"); 2])];
        c
    };
    let mut teams: Vec<FakeTeam> = (0..3).map(|t| FakeTeam::new(&format!("t-{t}"), "T", (0..10).map(|i| channel(t, i)).collect())).collect();
    teams[0].channels[0].members = (0..40).map(|i| FakeMember::new(FakeUser::new(&format!("u-{i}"), "P"))).collect();
    let rules = ChannelRules { members_page: 1, ..ChannelRules::default() };
    let server = serve_teams(FakeTeams { teams, channel_rules: rules, ..teams_with(Vec::new()) });
    let (r, lines) = channel_check(&server, &opts());
    r.unwrap();
    let t = teams_of(&server);
    let read: Vec<&str> = t.channel_asks.iter().filter(|(r, _)| r == "channelMessages").map(|(_, c)| c.as_str()).collect();
    let turns: Vec<String> = (0..7).flat_map(|i| (0..3).map(move |t| format!("19:t{t}c{i}@thread.tacv2"))).take(20).collect();
    assert_eq!(read[..20], turns.iter().map(String::as_str).collect::<Vec<_>>()[..], "{read:?}");
    assert_eq!(read.len(), 21, "twenty, and the largest again without its replies");
    assert_eq!(t.channel_asks.iter().filter(|(r, c)| r == "channelMembers" && c == "19:t0c0@thread.tacv2").count(), 25);
    assert_eq!(t.channel_asks.iter().filter(|(r, _)| r == "channelReplies").count(), 3);
    assert!(line(&lines, "channel members:").ends_with("largest channel at least 25"), "{lines:#?}");
    // Every request about a channel already asked waited its second first.
    let distinct: HashSet<&String> = t.channel_asks.iter().map(|(_, c)| c).collect();
    let paced = slept().iter().filter(|d| **d == Duration::from_secs(1)).count();
    assert_eq!(paced, t.channel_asks.len() - distinct.len());

    // Teams: ten pages at most, then the channels of 200 teams at most.
    let teams: Vec<FakeTeam> = (0..205).map(|t| FakeTeam::new(&format!("t-{t}"), "T", Vec::new())).collect();
    let rules = ChannelRules { teams_page: 30, ..ChannelRules::default() };
    let server = serve_teams(FakeTeams { teams, channel_rules: rules, ..teams_with(Vec::new()) });
    let (_, lines) = channel_check(&server, &opts());
    assert_eq!(line(&lines, "teams:"), "teams: 205 (archived 0)");
    assert_eq!(line(&lines, "channel lists:"), "channel lists: asked of 200 teams of 205, refused 0 (why: -)");
    assert_eq!(teams_of(&server).hits.get("channels"), Some(&200));
    let rules = ChannelRules { teams_page: 1, ..ChannelRules::default() };
    let teams: Vec<FakeTeam> = (0..12).map(|t| FakeTeam::new(&format!("t-{t}"), "T", Vec::new())).collect();
    let server = serve_teams(FakeTeams { teams, channel_rules: rules, ..teams_with(Vec::new()) });
    let (_, lines) = channel_check(&server, &opts());
    assert_eq!(line(&lines, "teams:"), "teams: at least 10 (archived 0)");
    assert_eq!(teams_of(&server).hits.get("joinedTeams"), Some(&10));
}

/// Nothing the fake answers reaches a line: no name, description, subject, text, address, id or
/// organisation, whatever it is.
#[test]
fn the_check_says_nothing_private() {
    let secret = |s: &str| format!("zq-{s}");
    let me_secret =
        FakeUser { id: secret("oid-me"), display_name: "Zebulon Quixote".into(), mail: "zq@secret.example".into(), upn: "zq@secret.example".into() };
    let other = FakeUser::new(&secret("oid-other"), "Philippa Nightjar");
    let channel_id = "19:zq-nightingale@thread.tacv2";
    let mut root = post(0, Some(&other.id), &format!("<p>secret-banana {}</p>", picture(channel_id, 0, None)));
    root.mentions = vec![me_secret.id.clone(), other.id.clone()];
    let mut th = FakeThread::new(root, vec![post(1, Some(&me_secret.id), "<p>secret-banana again</p>"); 2]);
    th.subject = Some("Project Nightingale".into());
    th.other_mentions = vec![json!({"tag": {"id": "zq-tag", "displayName": "Nightingale crew"}})];
    let members =
        vec![FakeMember::new(me_secret.clone()), FakeMember { tenant: Some(secret("tenant-other")), guest: true, ..FakeMember::new(other.clone()) }];
    let mut ch = FakeChannel::new(channel_id, "Nightingale secrets", members);
    ch.threads = vec![th];
    ch.link = Some(Some("https://teams.microsoft.com.evil.example/l/channel/zq-secret".into()));
    let team = FakeTeam { description: "secret-banana team".into(), ..FakeTeam::new(&secret("team"), "Nightingale", vec![ch]) };
    let t = FakeTeams {
        teams: vec![team],
        channel_rules: ChannelRules { delta_status: 200, ..ChannelRules::default() },
        ..FakeTeams::new(me_secret, &secret("tenant"))
    };
    let server = serve_teams(t);
    let (r, mut lines) = channel_check(&server, &ChannelCheck { tenant: Some(secret("tenant")), ..opts() });
    r.unwrap();
    let (r, more) = channel_check(&server, &ChannelCheck { tenant: None, granted: String::new(), ..opts() });
    r.unwrap();
    lines.extend(more);
    assert!(lines.iter().any(|l| l.starts_with("mentions: person 2 (the user 1)")), "{lines:#?}");
    for l in &lines {
        for p in ["zq", "Zebulon", "Quixote", "secret", "Nightingale", "Nightjar", "Philippa", "banana", "evil", "Project", "crew", "aW1n", "tacv2"] {
            assert!(!l.contains(p), "{p:?} in {l:?}");
        }
        assert!(l.is_ascii() && !l.contains('@') && !l.contains("//") && !l.contains(&(T0 / 1000).to_string()), "{l:?}");
    }
}
