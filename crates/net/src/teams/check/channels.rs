// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The step 0 check of Teams' teams and channels (`quatrelle-cli teams check --channels`): what
//! Graph answers about them on the user's own account, before anything is built on it.
//!
//! It reads, and at most: the teams the user joined ([`TEAM_PAGES`] pages); the channels of each
//! ([`MAX_TEAMS`] teams, a page each); for [`READ_CHANNELS`] channels taken in turn from the teams,
//! a page of root messages with their replies, and its members ([`MEMBER_PAGES`] pages); a page
//! without the replies for the channel whose page was the largest; the replies route for
//! [`REPLY_THREADS`] threads of two replies or more; and, for the first channel read, the old
//! `…/messages/delta` (its status only, nothing followed). About a hundred requests in all on a
//! usual account, and a few minutes: two requests about one channel are [`PACE`] apart, and a
//! 429 or a 503 is waited out as its `Retry-After` says ([`MAX_WAIT`] at most, [`ROUNDS`] tries);
//! when Graph still asks to wait, the check asks nothing more and says so.
//!
//! It says only counts, Graph's names for kinds, yes or no, and HTTP statuses with Graph's error
//! codes: never a team's, a channel's or a person's name, an id, a message's text or an address.
//! It writes nothing, here or at Microsoft (the write permissions are asked for only to see
//! whether they are granted): stopped at any moment, it leaves nothing behind.
//!
//! Each line is said in the design's order, and each settles a question of the design: its
//! method says which.

use std::collections::HashSet;
use std::time::Duration;

use serde_json::Value;

use super::{ByName, Me, TeamsClient, id_of, ms_of, text_of, why};
use crate::graph::backoff_secs;
use crate::oauth::{TEAMS_CHANNELS_SCOPE, TEAMS_CHANNELS_WRITE_SCOPE};
use crate::{NetError, Result};

/// Pages of the joined teams followed at most.
const TEAM_PAGES: usize = 10;
/// Teams whose channels are listed at most (a page each).
const MAX_TEAMS: usize = 200;
/// Channels whose messages and members are read, taken in turn from the teams.
const READ_CHANNELS: usize = 20;
/// Root messages asked for a page (the most Graph gives).
const TOP: u32 = 50;
/// Pages of a channel's members followed at most (999 a page: past 20,000 members).
const MEMBER_PAGES: usize = 25;
/// Threads whose replies are read through the replies route.
const REPLY_THREADS: usize = 3;
/// Between two requests about one channel (Graph takes one a second per channel).
const PACE: Duration = Duration::from_secs(1);
/// Tries of a request Graph asks to wait for (a 429 or a 503), and the longest wait taken (s).
const ROUNDS: u32 = 3;
const MAX_WAIT: u64 = 60;
/// Pictures looked at per message, and parts of a picture's address, at most.
const MAX_PICTURES: usize = 50;
const MAX_SEGMENTS: usize = 16;
/// Teams' two web hosts: a link of Teams' is on one of them exactly, in `https`, without a port.
const TEAMS_HOSTS: [&str; 2] = ["https://teams.microsoft.com/", "https://teams.cloud.microsoft/"];

/// What the channel check is told of the sign-in.
pub struct ChannelCheck {
    /// The signed-in user's organisation (the sign-in's `tid`): what "another organisation" is
    /// measured against; `None`: not known.
    pub tenant: Option<String>,
    /// The permissions the sign-in says it granted (`Tokens::granted`; empty: not said).
    pub granted: String,
    /// How to wait: `std::thread::sleep`, or something shorter in tests.
    pub sleep: fn(Duration),
}

/// Run the channel check, saying what it finds one line at a time. Refusals are said and the
/// check goes on; it fails only when the token is refused from the start. The client's own
/// waits are off meanwhile: the check waits a `Retry-After` out itself.
pub fn run_channel_check(client: &TeamsClient, me: &Me, check: &ChannelCheck, say: &mut dyn FnMut(&str)) -> Result<()> {
    client.set_waiting(false);
    let mut run = Run { client, me, opts: check, say, asked: HashSet::new(), stopped: None };
    let r = run.all();
    client.set_waiting(true);
    r
}

struct Run<'a> {
    client: &'a TeamsClient,
    me: &'a Me,
    opts: &'a ChannelCheck,
    say: &'a mut dyn FnMut(&str),
    /// The channels asked about so far (their places among those picked).
    asked: HashSet<usize>,
    /// Why the check stopped asking: Graph still asked to wait.
    stopped: Option<String>,
}

/// A channel whose messages are read: its team's id and its own.
struct Picked {
    team: String,
    channel: String,
}

/// What reading the channels' messages gave.
#[derive(Default)]
struct Read {
    tally: Tally,
    /// The threads whose replies the replies route reads: (the channel's place, the root's id).
    threads: Vec<(usize, String)>,
    /// The first channel read.
    first: Option<usize>,
}

impl Run<'_> {
    fn say(&mut self, line: String) {
        (self.say)(&line);
    }

    fn all(&mut self) -> Result<()> {
        self.permissions();
        if let Some(teams) = self.teams()? {
            let lists = self.channels(&teams);
            let picked = pick(&lists);
            let read = self.messages(&picked);
            self.replies_route(&picked, &read.threads);
            let (pictures, mentions) = (read.tally.pictures_line(), read.tally.mentions_line());
            self.say(pictures);
            self.say(mentions);
            self.members(&picked);
            self.delta(&picked, read.first);
        }
        if let Some(why) = self.stopped.clone() {
            self.say(format!("stopped early: Graph still asked to wait ({why}); the lines above count what was read before"));
        }
        let s = self.client.stats();
        self.say(format!("in all: requests {}, throttled {}", s.requests, s.throttled));
        Ok(())
    }

    /// A request about `channel` (its place among those picked), when there is one, after
    /// [`PACE`] if that channel was asked about before; a 429 or a 503 waited out as its
    /// `Retry-After` says, [`ROUNDS`] tries at most. When Graph still asks to wait, or for longer
    /// than [`MAX_WAIT`], the check stops: nothing more is asked.
    fn ask<T>(&mut self, channel: Option<usize>, f: impl Fn(&TeamsClient) -> Result<T>) -> Result<T> {
        if let Some(why) = &self.stopped {
            return Err(NetError::Closed(why.clone()));
        }
        if let Some(i) = channel
            && !self.asked.insert(i)
        {
            (self.opts.sleep)(PACE);
        }
        let mut round = 0;
        loop {
            round += 1;
            let r = f(self.client);
            let Err(e) = &r else { return r };
            if !busy(e) {
                return r;
            }
            let wait = self.client.retry_after().unwrap_or_else(|| backoff_secs(None, round));
            if round >= ROUNDS || wait > MAX_WAIT {
                self.stopped = Some(why(e));
                return r;
            }
            (self.opts.sleep)(Duration::from_secs(wait));
        }
    }

    /// Line 1: the six permissions, granted or not, as the sign-in says. Settles: `ChannelMessage.Read.All`
    /// refused, no channel can be read (the design is amended); `ChannelMember.Read.All` refused,
    /// "External" by the channel's summary; a write permission refused, the second delivery cut.
    fn permissions(&mut self) {
        let granted: Vec<&str> = self.opts.granted.split_whitespace().collect();
        if granted.is_empty() {
            self.say("channels, permissions granted: not said by the sign-in".into());
            return;
        }
        let said: Vec<String> = TEAMS_CHANNELS_SCOPE
            .split(' ')
            .chain(TEAMS_CHANNELS_WRITE_SCOPE.split(' '))
            .map(|scope| {
                let short = scope.rsplit('/').next().unwrap_or(scope);
                let has = granted.iter().any(|g| g.eq_ignore_ascii_case(scope) || g.eq_ignore_ascii_case(short));
                format!("{short} {}", yes(has))
            })
            .collect();
        self.say(format!("channels, permissions granted: {}", said.join(", ")));
    }

    /// Line 2: the teams the user joined, and how many are archived (their ids, for the channels).
    /// Settles: the cap on teams. `None` when Graph refuses them: nothing more is asked.
    fn teams(&mut self) -> Result<Option<Vec<String>>> {
        let (mut ids, mut total, mut archived) = (Vec::new(), 0usize, 0usize);
        let (mut next, mut capped, mut end): (Option<String>, bool, String) = (None, true, String::new());
        for i in 0..TEAM_PAGES {
            let page = match self.ask(None, |c| c.joined_teams(next.as_deref())) {
                Ok(p) => p,
                Err(e @ NetError::Auth(_)) if i == 0 => return Err(e),
                Err(e) if i == 0 => {
                    self.say(format!("teams: no ({})", why(&e)));
                    return Ok(None);
                }
                Err(e) => {
                    end = format!(", the next page: no ({})", why(&e));
                    break;
                }
            };
            for t in &page.items {
                total += 1;
                archived += usize::from(t.get("isArchived").and_then(Value::as_bool) == Some(true));
                ids.extend(id_of(t));
            }
            next = page.next;
            if next.is_none() {
                capped = false;
                break;
            }
        }
        let least = if capped { "at least " } else { "" };
        self.say(format!("teams: {least}{total} (archived {archived}){end}"));
        Ok(Some(ids))
    }

    /// Line 3: the channels of each team (of the first [`MAX_TEAMS`]): by kind of membership, made in
    /// another organisation, by layout, and with Teams' own link. Settles: the cap on channels,
    /// whether shared channels and the chat layout are there, and the rule for a channel's link.
    /// The teams and their channels' ids, in Graph's order.
    fn channels(&mut self, teams: &[String]) -> Vec<(String, Vec<String>)> {
        let mine = self.opts.tenant.clone();
        let mut lists = Vec::new();
        let (mut total, mut kinds, mut foreign, mut layouts, mut links) = (0usize, [0usize; 4], 0usize, [0usize; 4], 0usize);
        let (mut asked, mut refused, mut more) = (0usize, 0usize, false);
        let mut why_not = ByName::default();
        for team in teams.iter().take(MAX_TEAMS) {
            if self.stopped.is_some() {
                break;
            }
            asked += 1;
            let page = match self.ask(None, |c| c.channels(team, None)) {
                Ok(p) => p,
                Err(e) => {
                    refused += 1;
                    why_not.add(why_id(&e));
                    continue;
                }
            };
            more |= page.next.is_some();
            let mut ids = Vec::new();
            for ch in &page.items {
                total += 1;
                let kind = match ch.get("membershipType").and_then(Value::as_str) {
                    Some("standard") => 0,
                    Some("private") => 1,
                    Some("shared") => 2,
                    _ => 3,
                };
                let layout = match ch.get("layoutType").and_then(Value::as_str) {
                    Some("post") => 0,
                    Some("chat") => 1,
                    Some(_) => 2,
                    None => 3,
                };
                for (counts, k) in [(&mut kinds, kind), (&mut layouts, layout)] {
                    if let Some(n) = counts.get_mut(k) {
                        *n += 1;
                    }
                }
                foreign += usize::from(mine.as_deref().is_some_and(|m| text_of(ch, "tenantId").is_some_and(|t| !t.eq_ignore_ascii_case(m))));
                links += usize::from(teams_link(ch.get("webUrl").and_then(Value::as_str), "l/channel/"));
                ids.extend(id_of(ch));
            }
            lists.push((team.clone(), ids));
        }
        let least = if more || refused > 0 || asked < teams.len() { "at least " } else { "" };
        let foreign = if mine.is_some() { foreign.to_string() } else { "not known".into() };
        let ([standard, private, shared, other], [post, chat, other_layout, none]) = (kinds, layouts);
        self.say(format!(
            "channels: {least}{total} (standard {standard}, private {private}, shared {shared}, other {other}; of another organisation {foreign}; \
             layout post {post}, chat {chat}, other {other_layout}, none {none}; webUrl l/channel {links})"
        ));
        if refused > 0 || asked < teams.len() {
            let of = if asked < teams.len() { format!(" of {}", teams.len()) } else { String::new() };
            self.say(format!("channel lists: asked of {asked} teams{of}, refused {refused} (why: {})", said(&why_not, refused)));
        }
        lists
    }

    /// Lines 4 to 9: a page of each channel's root messages with their replies, and one without them
    /// for the channel whose page was the largest. Settles: whether Graph serves channel messages
    /// at all (4); what they carry (5, 6: the rule for an outside sender stays the members'); whether
    /// a reply moves its root (7) and the listing follows the roots' changes (8), which decide
    /// whether the head is always read with its replies; how many roots a page can take (9).
    fn messages(&mut self, picked: &[Picked]) -> Read {
        let mut read = Read::default();
        let (mut ok, mut refused, mut not_asked) = (0usize, 0usize, 0usize);
        let mut why_not = ByName::default();
        let mut sizes: Vec<(usize, usize)> = Vec::new();
        for (i, p) in picked.iter().enumerate() {
            if self.stopped.is_some() {
                not_asked += 1;
                continue;
            }
            match self.ask(Some(i), |c| c.channel_messages(&p.team, &p.channel, TOP, true, None)) {
                Ok(page) => {
                    ok += 1;
                    read.first.get_or_insert(i);
                    sizes.push((self.client.last_answer_bytes(), i));
                    read.tally.page(&page.items, &self.me.id, self.opts.tenant.as_deref());
                    for root in &page.items {
                        let replies = root.get("replies").and_then(Value::as_array).map_or(0, Vec::len);
                        let cut = root.get("replies@odata.nextLink").is_some_and(Value::is_string);
                        if read.threads.len() < REPLY_THREADS
                            && (replies >= 2 || cut)
                            && let Some(id) = id_of(root)
                        {
                            read.threads.push((i, id));
                        }
                    }
                }
                Err(e) => {
                    refused += 1;
                    why_not.add(why_id(&e));
                }
            }
        }
        let not_asked = if not_asked > 0 { format!("; not asked {not_asked}") } else { String::new() };
        self.say(format!(
            "channel messages read: {ok} of {} channels (refused {refused}; why: {}{not_asked})",
            picked.len(),
            said(&why_not, refused)
        ));
        let t = &read.tally;
        let lines = [t.messages_line(), t.senders_line(self.opts.tenant.is_some()), t.moved_line(), t.order_line()];
        for line in lines {
            self.say(line);
        }
        let line = self.sizes(picked, &sizes, read.tally.most_expanded, read.tally.cut);
        self.say(line);
        read
    }

    /// Line 9: the pages' sizes with the replies (the largest, the median), and the largest channel's
    /// page asked again without them.
    fn sizes(&mut self, picked: &[Picked], sizes: &[(usize, usize)], most: usize, cut: usize) -> String {
        let Some(&(largest, at)) = sizes.iter().max_by_key(|(bytes, _)| *bytes) else {
            return "a page with $expand=replies: none read".into();
        };
        let mut sorted: Vec<usize> = sizes.iter().map(|(b, _)| *b).collect();
        sorted.sort_unstable();
        let median = sorted.get(sorted.len().saturating_sub(1) / 2).copied().unwrap_or_default();
        let without = match picked.get(at) {
            Some(_) if self.stopped.is_some() => "not asked".into(),
            None => "not asked".into(),
            Some(p) => match self.ask(Some(at), |c| c.channel_messages(&p.team, &p.channel, TOP, false, None)) {
                Ok(_) => format!("{} KiB", kib(self.client.last_answer_bytes())),
                Err(e) => format!("no ({})", why_id(&e)),
            },
        };
        format!(
            "a page with $expand=replies: largest {} KiB, median {} KiB; without: {without}; replies expanded per root at most {most}; roots whose replies were cut {cut}",
            kib(largest),
            kib(median)
        )
    }

    /// Line 10: the replies route, for [`REPLY_THREADS`] threads: whether it lists the newest first.
    /// Settles how a thread's replies are read: from the newest, stopping at the first known, or
    /// to the end, within the bound.
    fn replies_route(&mut self, picked: &[Picked], threads: &[(usize, String)]) {
        if threads.is_empty() {
            self.say("the replies route: not tried (no thread with 2 replies or more)".into());
            return;
        }
        let ([mut newest, mut oldest, mut mixed], mut read, mut refused) = ([0usize; 3], 0usize, 0usize);
        let mut why_not = ByName::default();
        for (i, root) in threads {
            let Some(p) = picked.get(*i) else { continue };
            if self.stopped.is_some() {
                break;
            }
            match self.ask(Some(*i), |c| c.replies(&p.team, &p.channel, root, None)) {
                Ok(page) => {
                    read += 1;
                    let times: Vec<i64> =
                        page.items.iter().filter_map(|m| m.get("createdDateTime").and_then(Value::as_str).and_then(ms_of)).collect();
                    let pairs = || times.windows(2).filter_map(|w| w.first().zip(w.get(1)));
                    if times.len() >= 2 && pairs().all(|(a, b)| a >= b) {
                        newest += 1;
                    } else if times.len() >= 2 && pairs().all(|(a, b)| a <= b) {
                        oldest += 1;
                    } else {
                        mixed += 1;
                    }
                }
                Err(e) => {
                    refused += 1;
                    why_not.add(why_id(&e));
                }
            }
        }
        let why = if refused > 0 { format!("; why: {}", why_not.said()) } else { String::new() };
        self.say(format!(
            "the replies route: newest first {newest}, oldest first {oldest}, mixed {mixed} (threads read {read}, refused {refused}{why})"
        ));
    }

    /// Line 13: each channel's members: of the organisation, guests, of another organisation, of other
    /// kinds (counted per channel), and the largest channel. Settles: "External" by the members,
    /// and the cap on a channel's members.
    fn members(&mut self, picked: &[Picked]) {
        let mine = self.opts.tenant.clone();
        let (mut read, mut refused, mut not_asked) = (0usize, 0usize, 0usize);
        let mut why_not = ByName::default();
        let [mut org, mut guests, mut foreign, mut other] = [0usize; 4];
        let (mut largest, mut largest_capped) = (0usize, false);
        for (i, p) in picked.iter().enumerate() {
            if self.stopped.is_some() {
                not_asked += 1;
                continue;
            }
            let (mut next, mut count, mut capped): (Option<String>, usize, bool) = (None, 0, true);
            let mut failed = None;
            for page_no in 0..MEMBER_PAGES {
                match self.ask(Some(i), |c| c.channel_members(&p.team, &p.channel, next.as_deref())) {
                    Ok(page) => {
                        for m in &page.items {
                            count += 1;
                            let kind = m.get("@odata.type").and_then(Value::as_str);
                            let guest = m.get("roles").and_then(Value::as_array).is_some_and(|r| r.iter().any(|x| x.as_str() == Some("guest")));
                            let elsewhere = mine.as_deref().is_some_and(|t| text_of(m, "tenantId").is_some_and(|x| !x.eq_ignore_ascii_case(t)));
                            match () {
                                _ if kind != Some("#microsoft.graph.aadUserConversationMember") => other += 1,
                                _ if guest => guests += 1,
                                _ if elsewhere => foreign += 1,
                                _ => org += 1,
                            }
                        }
                        next = page.next;
                        if next.is_none() {
                            capped = false;
                            break;
                        }
                    }
                    Err(e) => {
                        if page_no == 0 {
                            failed = Some(e);
                        }
                        break;
                    }
                }
            }
            match failed {
                Some(e) => {
                    refused += 1;
                    why_not.add(why_id(&e));
                }
                None => {
                    read += 1;
                    if count > largest || (count == largest && capped) {
                        (largest, largest_capped) = (count, capped);
                    }
                }
            }
        }
        let not_asked = if not_asked > 0 { format!("; not asked {not_asked}") } else { String::new() };
        let foreign = if mine.is_some() { foreign.to_string() } else { "not known".into() };
        let least = if largest_capped { "at least " } else { "" };
        self.say(format!(
            "channel members: read {read} of {} (refused {refused}; why: {}{not_asked}); organisation {org}, guests {guests}, \
             another organisation {foreign}, other kinds {other}; largest channel {least}{largest}",
            picked.len(),
            said(&why_not, refused)
        ));
    }

    /// Line 14: the old delta of a channel's messages, on the first channel read: its status only.
    /// Settles nothing by itself: a 200 is noted as a lead, never used.
    fn delta(&mut self, picked: &[Picked], first: Option<usize>) {
        let said = match first.and_then(|i| picked.get(i).map(|p| (i, p))) {
            None => "not tried (no channel read)".into(),
            Some(_) if self.stopped.is_some() => "not asked".into(),
            Some((i, p)) => match self.ask(Some(i), |c| c.channel_delta_status(&p.team, &p.channel)) {
                Ok((status, linked)) => format!("{status} (a link to go on: {})", yes(linked)),
                Err(e) => why_id(&e),
            },
        };
        self.say(format!("the old delta: {said}"));
    }
}

/// The channels whose messages are read: [`READ_CHANNELS`] at most, taken in turn from the
/// teams (the first of each, then the second of each…).
fn pick(lists: &[(String, Vec<String>)]) -> Vec<Picked> {
    let mut out = Vec::new();
    let longest = lists.iter().map(|(_, c)| c.len()).max().unwrap_or(0);
    for i in 0..longest {
        for (team, channels) in lists {
            if out.len() >= READ_CHANNELS {
                return out;
            }
            if let Some(c) = channels.get(i) {
                out.push(Picked { team: team.clone(), channel: c.clone() });
            }
        }
    }
    out
}

/// What the messages read hold, roots and replies alike unless said.
#[derive(Default)]
struct Tally {
    roots: usize,
    replies: usize,
    system: usize,
    apps: usize,
    subject: usize,
    high: usize,
    urgent: usize,
    links: usize,
    /// Senders who are people: their organisation said or not, the user's or another, their kind.
    present: usize,
    absent: usize,
    mine: usize,
    another: usize,
    aad: usize,
    other_kind: usize,
    /// Roots with replies, and those whose change time is at or after their newest reply's.
    with_replies: usize,
    moved: usize,
    /// Two roots one after the other, the second changed later than the first.
    out_of_order: usize,
    most_expanded: usize,
    cut: usize,
    /// Pictures by the form of their address ([`picture_form`]).
    pictures: [usize; 5],
    /// Mentions: of a person (of the user), a channel, a team, a tag, another.
    person: usize,
    the_user: usize,
    channel: usize,
    team: usize,
    tag: usize,
    other_mention: usize,
}

impl Tally {
    /// A page of root messages, each with its expanded replies.
    fn page(&mut self, roots: &[Value], me: &str, tenant: Option<&str>) {
        let mut before: Option<i64> = None;
        for root in roots {
            self.roots += 1;
            self.message(root, false, me, tenant);
            let at = root.get("lastModifiedDateTime").and_then(Value::as_str).and_then(ms_of);
            if let (Some(prev), Some(now)) = (before, at)
                && now > prev
            {
                self.out_of_order += 1;
            }
            before = at.or(before);
            let replies = root.get("replies").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
            self.most_expanded = self.most_expanded.max(replies.len());
            self.cut += usize::from(root.get("replies@odata.nextLink").is_some_and(Value::is_string));
            for r in replies {
                self.replies += 1;
                self.message(r, true, me, tenant);
            }
            if !replies.is_empty() {
                self.with_replies += 1;
                let newest = replies.iter().filter_map(|r| r.get("createdDateTime").and_then(Value::as_str).and_then(ms_of)).max();
                self.moved += usize::from(at.zip(newest).is_some_and(|(a, n)| a >= n));
            }
        }
    }

    fn message(&mut self, m: &Value, reply: bool, me: &str, tenant: Option<&str>) {
        let text = |k: &str| m.get(k).and_then(Value::as_str);
        let set = |p: &str| m.pointer(p).filter(|v| v.is_object());
        self.system += usize::from(text("messageType") == Some("systemEventMessage"));
        self.apps += usize::from(set("/from/application").is_some());
        if let Some(user) = set("/from/user") {
            match user.get("tenantId").and_then(Value::as_str).filter(|t| !t.is_empty()) {
                Some(t) => {
                    self.present += 1;
                    match tenant {
                        Some(mine) if t.eq_ignore_ascii_case(mine) => self.mine += 1,
                        Some(_) => self.another += 1,
                        None => {}
                    }
                }
                None => self.absent += 1,
            }
            if user.get("userIdentityType").and_then(Value::as_str) == Some("aadUser") {
                self.aad += 1;
            } else {
                self.other_kind += 1;
            }
        }
        self.subject += usize::from(text("subject").is_some_and(|s| !s.trim().is_empty()));
        match text("importance") {
            Some("high") => self.high += 1,
            Some("urgent") => self.urgent += 1,
            _ => {}
        }
        self.links += usize::from(teams_link(text("webUrl"), "l/message/"));
        let html = m.pointer("/body/content").and_then(Value::as_str).unwrap_or_default();
        for src in html.split("src=").skip(1).take(MAX_PICTURES) {
            let Some(quote) = src.chars().next().filter(|c| matches!(c, '"' | '\'')) else { continue };
            let address = src.get(1..).unwrap_or_default().split(quote).next().unwrap_or_default();
            if address.contains("/hostedContents/")
                && let Some(n) = self.pictures.get_mut(picture_form(address, reply))
            {
                *n += 1;
            }
        }
        for x in m.get("mentions").and_then(Value::as_array).into_iter().flatten() {
            let of = |k: &str| x.get("mentioned").and_then(|v| v.get(k)).filter(|v| v.is_object());
            if let Some(user) = of("user") {
                self.person += 1;
                self.the_user += usize::from(user.get("id").and_then(Value::as_str) == Some(me));
            } else if let Some(c) = of("conversation") {
                match c.get("conversationIdentityType").and_then(Value::as_str) {
                    Some("channel") => self.channel += 1,
                    Some("team") => self.team += 1,
                    _ => self.other_mention += 1,
                }
            } else if of("tag").is_some() {
                self.tag += 1;
            } else {
                self.other_mention += 1;
            }
        }
    }

    /// Line 5: what the messages are: roots, replies expanded, system messages, from apps; with a
    /// subject, by importance; with Teams' own link to the message.
    fn messages_line(&self) -> String {
        format!(
            "messages: roots {}, replies expanded {}, system {}, from apps {}; subject {}, importance high {}, urgent {}; webUrl l/message {}",
            self.roots, self.replies, self.system, self.apps, self.subject, self.high, self.urgent, self.links
        )
    }

    /// Line 6: who wrote them, of the people: their organisation said or not, the user's or not, their
    /// kind of identity.
    fn senders_line(&self, tenant_known: bool) -> String {
        let org = if tenant_known {
            format!("the user's organisation {}, another {}", self.mine, self.another)
        } else {
            "the user's organisation not known".into()
        };
        format!(
            "from.user: tenantId present {}, absent {}; {org}; userIdentityType aadUser {}, other {}",
            self.present, self.absent, self.aad, self.other_kind
        )
    }

    /// Line 7: whether a reply moves its root's change time.
    fn moved_line(&self) -> String {
        format!("roots with replies: {}; whose lastModifiedDateTime is at or after their newest reply: {}", self.with_replies, self.moved)
    }

    /// Line 8: whether the listing follows the roots' change times, the newest first.
    fn order_line(&self) -> String {
        format!("the listing is in lastModifiedDateTime order: {} (out of order {})", yes(self.out_of_order == 0), self.out_of_order)
    }

    /// Line 11: the pictures kept with the messages, by the form of their address. Settles which forms
    /// the rule for a picture's address takes.
    fn pictures_line(&self) -> String {
        let [channel, reply, as_message, chat, other] = self.pictures;
        format!("hosted images: channel form {channel}, channel reply form {reply}, reply as a message {as_message}, chat form {chat}, other {other}")
    }

    /// Line 12: the mentions, by what they name. Settles how a mention of a channel or a team is told
    /// from a mention of the user.
    fn mentions_line(&self) -> String {
        format!(
            "mentions: person {} (the user {}), channel {}, team {}, tag {}, other {}",
            self.person, self.the_user, self.channel, self.team, self.tag, self.other_mention
        )
    }
}

/// The form of a picture's address: 0 a channel message's, 1 a reply's under its root
/// (`…/messages/{root}/replies/{reply}/…`), 2 a reply's as a message of its own, 3 a chat
/// message's, 4 another (another host, or another path).
fn picture_form(address: &str, reply: bool) -> usize {
    let Some(rest) = address.strip_prefix("https://graph.microsoft.com/") else { return 4 };
    let path: Vec<&str> = rest.split(['?', '#']).next().unwrap_or_default().split('/').skip(1).take(MAX_SEGMENTS).collect();
    match path.as_slice() {
        ["teams", _, "channels", _, "messages", _, "replies", _, "hostedContents", ..] => 1,
        ["teams", _, "channels", _, "messages", _, "hostedContents", ..] => usize::from(reply) * 2,
        ["chats", _, "messages", _, "hostedContents", ..] => 3,
        _ => 4,
    }
}

/// Whether an address is Teams' own link of this kind (`l/channel/`, `l/message/`): `https`, on
/// one of Teams' web hosts exactly, without a port.
fn teams_link(address: Option<&str>, kind: &str) -> bool {
    address.is_some_and(|a| TEAMS_HOSTS.iter().any(|h| a.strip_prefix(h).is_some_and(|rest| rest.starts_with(kind))))
}

/// Whether Graph asks to wait: a 429 or a 503.
fn busy(e: &NetError) -> bool {
    matches!(e, NetError::Closed(m) if m.starts_with("HTTP 429") || m.starts_with("HTTP 503"))
}

/// [`why`], an id refused before any request said as such.
fn why_id(e: &NetError) -> String {
    match e {
        NetError::Refused(_) => "unusable id".into(),
        e => why(e),
    }
}

/// Reasons by name, `-` when there were none.
fn said(why: &ByName, refused: usize) -> String {
    if refused == 0 { "-".into() } else { why.said() }
}

fn yes(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

/// Bytes as KiB, rounded up.
fn kib(bytes: usize) -> usize {
    bytes.div_ceil(1024)
}
