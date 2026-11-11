// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The step 0 check: what a user runs on their own Microsoft 365 account to learn whether
//! Graph serves their Teams chats, before anything else is built on it.
//!
//! It reads the chats and the messages of the twenty most recently active (a page each,
//! every page of the newest one read), counts what it finds (chats by kind and by organisation,
//! the chats Graph refuses to read and why, messages, reactions by form, shared files), tries
//! each write Quatrelle will need on a chat the user names, and, if asked, runs the approved
//! polling cadences for an hour to measure the 429 rate. It says only counts, kinds of chat as
//! Graph names them, yes or no, HTTP statuses with Graph's error codes, and durations: never a
//! name, an address, a topic, a message's text, a chat or message id or a user's object id
//! (an error is said by its status and code alone, never its text, which carries the address
//! asked).
//! Every message it posts is soft-deleted before it returns, whatever failed in between.
//!
//! The hour outlives an access token (60 to 90 minutes): a request refused for its token is
//! made once more with a new one from [`CheckOptions::renew`]; refused again, or with no new
//! token to be had, the hour stops there, without asking anything more.
//!
//! Ages are measured on Graph's own clock (its `Date` header, or failing that the newest time
//! Graph wrote in what was read), never on this computer's.

use std::collections::HashSet;
use std::time::Duration;

use serde_json::Value;

use super::{Me, Range, TeamsClient};
use crate::graph::utc;
use crate::{NetError, Result};

/// 6. The approved cadences, for an hour.
mod hour;
/// How the check names what it counts.
mod names;

pub(super) use names::{ByName, author, id_shape, kind_name};

/// Pages followed at most per listing.
const PAGES: usize = 10;
/// The chats whose messages are read, the most recently active first.
const READ_CHATS: usize = 20;
/// Longest Graph error code said.
const MAX_CODE: usize = 64;
/// A shared file's bytes counted at most (none kept).
const MAX_DOWNLOAD: usize = 50 << 20;
/// Longest chat or message id, or shared file address, taken from an answer.
const MAX_ID: usize = 8 << 10;
/// The reactions Graph names rather than writes as an emoji.
const NAMED: [&str; 6] = ["like", "heart", "laugh", "surprised", "sad", "angry"];
/// The test messages.
const FIRST: &str = "Quatrelle check, please ignore (1/2)";
const EDITED: &str = "Quatrelle check, please ignore (1/2, edited)";
const SECOND: &str = "Quatrelle check, please ignore (2/2)";
const THUMBS_UP: &str = "\u{1f44d}";
/// How far before the writes began the sweep looks (ms): Graph's front ends can disagree by a
/// few seconds, and a lost send's message may carry a time just before the `Date` read.
const SWEEP_SLACK: i64 = 10_000;
/// Between two writes in the chat (Graph takes one a second per chat).
const PACE: Duration = Duration::from_secs(1);

#[cfg(test)]
thread_local! {
    /// Tests: how often the hour says how it is going, in place of the hour's `REPORT_EVERY`.
    pub(super) static REPORT_EVERY_IN_TESTS: std::cell::Cell<Option<Duration>> = const { std::cell::Cell::new(None) };
}

/// What the check does besides reading.
pub struct CheckOptions {
    /// The tenant to mark a chat read in (else the one Graph gives for the signed-in user).
    pub tenant: Option<String>,
    /// The chat (its id) where test messages may be posted, reacted to and deleted.
    pub write_chat: Option<String>,
    /// Run the approved cadences this long (the CLI: an hour).
    pub hour: Option<Duration>,
    /// How to wait: `std::thread::sleep`, or something shorter in tests.
    pub sleep: fn(Duration),
    /// A new access token when Graph refuses the one in use during the hour (it lasts 60 to 90
    /// minutes): `None` when there is none to be had. Called at most once per refused request.
    pub renew: Option<Box<dyn Fn() -> Option<String>>>,
}

/// Run the check, saying what it finds one line at a time. Refusals are said and the check goes
/// on; it fails only when the token is refused from the start.
pub fn run_check(client: &TeamsClient, me: &Me, opts: &CheckOptions, say: &mut dyn FnMut(&str)) -> Result<()> {
    let mut ck = Check { client, me, opts, say, seen: None, sent: Vec::new() };
    let listed = ck.chats()?;
    let read = ck.read_recent(&listed.recent);
    ck.reactions_and_files(&read.tally);
    if let Some(chat) = &opts.write_chat {
        ck.write(chat, listed.write_tenant);
    }
    if let Some(d) = opts.hour {
        ck.hour(d, read.newest, read.watermark, &read.refused);
    }
    let s = client.stats();
    ck.say(format!("in all: requests {}, throttled {}", s.requests, s.throttled));
    Ok(())
}

struct Check<'a> {
    client: &'a TeamsClient,
    me: &'a Me,
    opts: &'a CheckOptions,
    say: &'a mut dyn FnMut(&str),
    /// The newest time Graph wrote in what was read (milliseconds), for want of its `Date`.
    seen: Option<i64>,
    /// The test messages posted (their ids).
    sent: Vec<String>,
}

/// What the chat list gave.
#[derive(Default)]
struct Listed {
    /// The [`READ_CHATS`] chats with the most recent messages, the newest first.
    recent: Vec<Listing>,
    /// The write chat's tenant, as the list gives it.
    write_tenant: Option<String>,
}

/// A chat as the list gives it.
struct Listing {
    id: String,
    /// One-to-one, group, meeting, other: 0 to 3.
    kind: usize,
    /// Made in another organisation than the signed-in user's.
    foreign: bool,
    /// When its last message came (milliseconds).
    at: Option<i64>,
}

/// What reading the newest chats gave.
#[derive(Default)]
struct Read {
    /// Reactions and files, over every chat read.
    tally: Tally,
    /// The newest chat read: its id, its messages (every page), and its watermark (the newest
    /// `lastModifiedDateTime`, in milliseconds).
    newest: Option<String>,
    messages: usize,
    capped: bool,
    end: String,
    watermark: Option<i64>,
    /// The chats Graph refused for good (an answer such as 403, not a failure that may pass):
    /// the hour leaves them out.
    refused: HashSet<String>,
}

/// What the messages read hold.
#[derive(Default)]
struct Tally {
    reacted: usize,
    named: usize,
    emoji: usize,
    other: usize,
    files: usize,
    first_file: Option<String>,
    /// The newest `lastModifiedDateTime` (milliseconds).
    newest: Option<i64>,
}

impl Check<'_> {
    fn say(&mut self, line: String) {
        (self.say)(&line);
    }

    /// Graph's clock now (milliseconds): the end of the second its last `Date` named, or else
    /// just after the newest time it wrote.
    fn now(&self) -> Option<i64> {
        match self.client.server_time() {
            Some(s) => s.checked_mul(1000)?.checked_add(1000),
            None => self.seen.map(|t| t.saturating_add(1)),
        }
    }

    fn saw(&mut self, t: Option<i64>) {
        self.seen = self.seen.max(t);
    }

    /// 1. The chats, by kind and by organisation.
    fn chats(&mut self) -> Result<Listed> {
        let mut listed = Listed::default();
        let mine = self.opts.tenant.clone();
        let (mut kinds, mut total, mut foreign) = ([0usize; 4], 0usize, 0usize);
        let (mut others, mut other_ids, mut other_authors) = (ByName::default(), ByName::default(), ByName::default());
        let mut all: Vec<Listing> = Vec::new();
        let mut next: Option<String> = None;
        let mut end = String::new();
        let mut capped = true;
        for i in 0..PAGES {
            let page = match self.client.chats(next.as_deref()) {
                Ok(p) => p,
                Err(e @ NetError::Auth(_)) if i == 0 => return Err(e),
                Err(e) if i == 0 => {
                    self.say(format!("chats: no ({})", why(&e)));
                    return Ok(listed);
                }
                Err(e) => {
                    end = format!(", the next page: no ({})", why(&e));
                    break;
                }
            };
            for chat in &page.items {
                total += 1;
                let named = chat.get("chatType").and_then(Value::as_str);
                let kind = match named {
                    Some("oneOnOne") => 0,
                    Some("group") => 1,
                    Some("meeting") => 2,
                    _ => {
                        others.add(kind_name(named));
                        other_ids.add(id_shape(chat.get("id").and_then(Value::as_str)));
                        other_authors.add(author(chat));
                        3
                    }
                };
                if let Some(k) = kinds.get_mut(kind) {
                    *k += 1;
                }
                let made_in = text_of(chat, "tenantId");
                let is_foreign = mine.as_deref().is_some_and(|m| made_in.as_deref().is_some_and(|t| !t.eq_ignore_ascii_case(m)));
                foreign += usize::from(is_foreign);
                let Some(id) = id_of(chat) else { continue };
                let at = chat.pointer("/lastMessagePreview/createdDateTime").and_then(Value::as_str).and_then(ms_of);
                self.saw(at);
                if self.opts.write_chat.as_deref() == Some(id.as_str()) {
                    listed.write_tenant = made_in;
                }
                all.push(Listing { id, kind, foreign: is_foreign, at });
            }
            next = page.next;
            if next.is_none() {
                capped = false;
                break;
            }
        }
        let [one, group, meeting, other] = kinds;
        let least = if capped { "at least " } else { "" };
        self.say(format!("chats: {least}{total} (one-to-one {one}, group {group}, meeting {meeting}, other {other}){end}"));
        if other > 0 {
            self.say(format!("chats of other kinds: {}", others.said()));
            self.say(format!("chats of other kinds, by id: {}", other_ids.said()));
            self.say(format!("chats of other kinds, last message from: {}", other_authors.said()));
        }
        match mine {
            Some(_) => self.say(format!("chats made in another organisation: {foreign}")),
            None => self.say("chats made in another organisation: not known (no tenant)".into()),
        }
        // The most recently active first (a chat without a message last), whatever order the
        // pages came in.
        all.sort_by_key(|c| std::cmp::Reverse(c.at));
        all.truncate(READ_CHATS);
        listed.recent = all;
        Ok(listed)
    }

    /// 2. The newest chats' messages, read backwards from now.
    ///
    /// A page of each, every page of the newest one Graph lets be read. What Graph refuses is
    /// counted by kind of chat, by organisation and by why.
    fn read_recent(&mut self, recent: &[Listing]) -> Read {
        let mut read = Read::default();
        let Some(before) = self.now().map(rfc3339) else {
            self.say(format!("messages, the {} newest chats: not read (no server time)", recent.len()));
            self.say("messages in the newest chat read: 0 (no chat read)".into());
            return read;
        };
        let (mut ok, mut kinds, mut foreign) = (0usize, [0usize; 4], 0usize);
        let mut why_not = ByName::default();
        for chat in recent {
            let all = read.newest.is_none();
            match self.read_chat(&chat.id, &before, all, &mut read) {
                Ok(()) => {
                    ok += 1;
                    if all {
                        read.newest = Some(chat.id.clone());
                    }
                }
                Err(e) => {
                    if let Some(k) = kinds.get_mut(chat.kind) {
                        *k += 1;
                    }
                    foreign += usize::from(chat.foreign);
                    why_not.add(why(&e));
                    if matches!(e, NetError::Protocol(_)) {
                        read.refused.insert(chat.id.clone());
                    }
                }
            }
        }
        self.saw(read.tally.newest);
        let refused = recent.len().saturating_sub(ok);
        self.say(format!("messages, the {} newest chats: read {ok}, refused {refused}", recent.len()));
        if refused > 0 {
            let [one, group, meeting, other] = kinds;
            self.say(format!("messages refused, by kind: one-to-one {one}, group {group}, meeting {meeting}, other {other}"));
            if self.opts.tenant.is_some() {
                self.say(format!("messages refused, made in another organisation: {foreign}"));
            }
            self.say(format!("messages refused, why: {}", why_not.said()));
        }
        if read.newest.is_none() {
            self.say("messages in the newest chat read: 0 (no chat read)".into());
        } else {
            let least = if read.capped { "at least " } else { "" };
            self.say(format!("messages in the newest chat read: {least}{}{}", read.messages, read.end));
        }
        read
    }

    /// A chat's messages before `before`: its first page, or with `all` every page (the newest
    /// chat read: its messages counted, its watermark kept). An error on the first page is
    /// the chat refused; on a later one, the count stops there.
    fn read_chat(&mut self, chat: &str, before: &str, all: bool, read: &mut Read) -> Result<()> {
        let mut next: Option<String> = None;
        for i in 0..if all { PAGES } else { 1 } {
            let page = match self.client.messages(chat, Range::Before(before), next.as_deref()) {
                Ok(p) => p,
                Err(e) if i == 0 => return Err(e),
                Err(e) => {
                    read.end = format!(", then no ({})", why(&e));
                    read.capped = true;
                    return Ok(());
                }
            };
            for m in &page.items {
                read.tally.count(m);
                if all {
                    read.messages += 1;
                    read.watermark = read.watermark.max(m.get("lastModifiedDateTime").and_then(Value::as_str).and_then(ms_of));
                }
            }
            next = page.next;
            if next.is_none() {
                return Ok(());
            }
        }
        read.capped = all;
        Ok(())
    }

    /// 3 and 4. Reactions by form, and shared files (the first one downloaded, counted only).
    fn reactions_and_files(&mut self, t: &Tally) {
        self.say(format!(
            "reactions: {} on {} messages (by name {}, as emoji {}, other {})",
            t.named + t.emoji + t.other,
            t.reacted,
            t.named,
            t.emoji,
            t.other
        ));
        let first = match &t.first_file {
            None => "none to try".to_string(),
            Some(url) => match self.client.download_shared(url, MAX_DOWNLOAD) {
                Ok(n) => format!("yes, {n} bytes"),
                Err(e) => format!("no ({})", why(&e)),
            },
        };
        self.say(format!("file attachments: {}, the first downloads: {first}", t.files));
    }
}

impl Tally {
    fn count(&mut self, m: &Value) {
        self.newest = self.newest.max(m.get("lastModifiedDateTime").and_then(Value::as_str).and_then(ms_of));
        let reactions = m.get("reactions").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
        if !reactions.is_empty() {
            self.reacted += 1;
        }
        for r in reactions {
            match r.get("reactionType").and_then(Value::as_str) {
                Some(k) if NAMED.contains(&k) => self.named += 1,
                Some(k) if !k.is_ascii() => self.emoji += 1,
                _ => self.other += 1,
            }
        }
        let files = m.get("attachments").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
        for a in files.iter().filter(|a| a.get("contentType").and_then(Value::as_str) == Some("reference")) {
            self.files += 1;
            if self.first_file.is_none() {
                self.first_file = text_of(a, "contentUrl");
            }
        }
    }
}

/// 5. The writes, on the chat the user named.
impl Check<'_> {
    fn write(&mut self, chat: &str, listed_tenant: Option<String>) {
        // Sent and not yet deleted; and sends whose outcome isn't known (made or not, no id).
        let mut left: Vec<String> = Vec::new();
        let mut lost = 0usize;
        // When the writes began, by Graph's clock: what a lost send's message is made after. Its
        // `Date` is needed: a time Graph wrote in what was read can be days old, and a window
        // that wide could take in a real message. Without it there is no sweep at all (matching
        // on ids instead adds nothing: the ids known are deleted by the clean-up already).
        let since = self.client.server_time().and_then(|s| s.checked_mul(1000)).map(|d| self.seen.map_or(d, |t| t.max(d)));
        let first = self.send(chat, FIRST, "(1/2)", &mut left, &mut lost);
        match &first {
            Some((id, at)) => {
                self.pace();
                let reacted = self.step("write, react:", self.client.set_reaction(chat, id, THUMBS_UP));
                if reacted {
                    self.pace();
                    let moved = self.moved(chat, id, *at);
                    self.say(format!("write, the reaction moves lastModifiedDateTime: {moved}"));
                    self.pace();
                    self.step("write, unset the reaction:", self.client.unset_reaction(chat, id, THUMBS_UP));
                } else {
                    self.not_tried(&["write, the reaction moves lastModifiedDateTime:", "write, unset the reaction:"]);
                }
                self.pace();
                self.step("write, edit:", self.client.edit_text(chat, id, EDITED));
                self.pace();
                if self.step("write, soft delete by /users:", self.client.soft_delete(chat, id, Some(&self.me.id))) {
                    left.retain(|l| l != id);
                }
            }
            None => self.not_tried(&[
                "write, react:",
                "write, the reaction moves lastModifiedDateTime:",
                "write, unset the reaction:",
                "write, edit:",
                "write, soft delete by /users:",
            ]),
        }
        self.pace();
        match self.send(chat, SECOND, "(2/2)", &mut left, &mut lost) {
            Some((id, _)) => {
                self.pace();
                if self.step("write, soft delete by /me:", self.client.soft_delete(chat, &id, None)) {
                    left.retain(|l| *l != id);
                }
            }
            None => self.not_tried(&["write, soft delete by /me:"]),
        }
        self.pace();
        self.mark_read(chat, listed_tenant);
        // Whatever failed above, nothing of the check stays in the chat.
        let known = std::mem::take(&mut self.sent);
        left.retain(|id| !self.delete(chat, id));
        let mut undeleted = left.len();
        if lost > 0 {
            match self.sweep(chat, since, &known) {
                Ok((swept, failed)) => {
                    self.say(format!("write, stray test messages swept: {swept}"));
                    undeleted += failed;
                }
                Err(why) => {
                    self.say(format!("write, stray test messages swept: no ({why})"));
                    undeleted += lost;
                }
            }
        }
        self.say(format!("write, test messages left undeleted: {undeleted}"));
    }

    /// Soft-delete a message of the check's, by `/users/{oid}` and else by `/me`: whether it is.
    fn delete(&self, chat: &str, id: &str) -> bool {
        self.client.soft_delete(chat, id, Some(&self.me.id)).is_ok() || self.client.soft_delete(chat, id, None).is_ok()
    }

    /// After a send whose outcome isn't known: the chat's newest messages that are the check's
    /// own (from the signed-in user, with one of its texts, made since the writes began), deleted.
    /// How many were, and how many couldn't be.
    fn sweep(&mut self, chat: &str, since: Option<i64>, known: &[String]) -> std::result::Result<(usize, usize), String> {
        let since = since.ok_or_else(|| "no server time".to_string())?;
        let floor = since.saturating_sub(SWEEP_SLACK).saturating_sub(1000);
        let page = self.client.messages(chat, Range::After(&rfc3339(floor)), None).map_err(|e| why(&e))?;
        let me = self.me.id.as_str();
        let stray: Vec<String> = page.items.iter().filter(|m| stray(m, me, since)).filter_map(id_of).filter(|id| !known.contains(id)).collect();
        let swept = stray.iter().filter(|id| self.delete(chat, id)).count();
        Ok((swept, stray.len().saturating_sub(swept)))
    }

    /// Post a test message: its id and change time, noted in `left` until it is deleted.
    fn send(&mut self, chat: &str, text: &str, which: &str, left: &mut Vec<String>, lost: &mut usize) -> Option<(String, Option<i64>)> {
        match self.client.send_text(chat, text) {
            Ok(m) => {
                self.say(format!("write, send {which}: yes"));
                let Some(id) = id_of(&m) else {
                    *lost += 1;
                    return None;
                };
                left.push(id.clone());
                self.sent.push(id.clone());
                let at = ["lastModifiedDateTime", "createdDateTime"].iter().find_map(|k| m.get(*k).and_then(Value::as_str).and_then(ms_of));
                Some((id, at))
            }
            Err(e) => {
                self.say(format!("write, send {which}: no ({})", why(&e)));
                if uncertain(&e) {
                    *lost += 1;
                }
                None
            }
        }
    }

    /// Whether the reaction moved the message's `lastModifiedDateTime` past `sent`.
    fn moved(&mut self, chat: &str, id: &str, sent: Option<i64>) -> String {
        let Some(sent) = sent else { return "not checked (no time)".into() };
        let after = rfc3339(sent.saturating_sub(1000));
        let mut next: Option<String> = None;
        for _ in 0..PAGES {
            let page = match self.client.messages(chat, Range::After(&after), next.as_deref()) {
                Ok(p) => p,
                Err(e) => return format!("not checked ({})", why(&e)),
            };
            if let Some(m) = page.items.iter().find(|m| id_of(m).as_deref() == Some(id)) {
                let now = m.get("lastModifiedDateTime").and_then(Value::as_str).and_then(ms_of);
                return if now.is_some_and(|t| t > sent) { "yes" } else { "no" }.into();
            }
            match page.next {
                Some(n) => next = Some(n),
                None => break,
            }
        }
        "not checked (not found)".into()
    }

    /// Mark the chat read, in the tenant given, else the signed-in user's as the chat says it.
    fn mark_read(&mut self, chat: &str, listed_tenant: Option<String>) {
        const SAID: &str = "write, mark the chat read:";
        let tenant = match &self.opts.tenant {
            Some(t) => Some(t.clone()),
            None => {
                let mine = self.client.members(chat).ok().and_then(|ms| {
                    ms.iter().find(|m| m.get("userId").and_then(Value::as_str) == Some(self.me.id.as_str())).and_then(|m| text_of(m, "tenantId"))
                });
                mine.or(listed_tenant)
            }
        };
        match tenant {
            Some(t) => {
                self.step(SAID, self.client.mark_read(chat, &self.me.id, &t));
            }
            None => self.say(format!("{SAID} no (no tenant)")),
        }
    }

    /// Say how a write went: whether it worked.
    fn step(&mut self, said: &str, r: Result<()>) -> bool {
        match r {
            Ok(()) => {
                self.say(format!("{said} yes"));
                true
            }
            Err(e) => {
                self.say(format!("{said} no ({})", why(&e)));
                false
            }
        }
    }

    fn not_tried(&mut self, said: &[&str]) {
        for s in said {
            self.say(format!("{s} not tried"));
        }
    }

    fn pace(&self) {
        (self.opts.sleep)(PACE);
    }
}

/// An error said without its text (which can carry the address asked): its HTTP status and
/// Graph's error code (`403 Forbidden`), or what kind of failure it was.
fn why(e: &NetError) -> String {
    if let NetError::Protocol(m) = e
        && m.starts_with("the shared file is larger")
    {
        return format!("over {} MiB", MAX_DOWNLOAD >> 20);
    }
    let text = e.to_string();
    let status = text.split("HTTP ").skip(1).find_map(|rest| {
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if digits.len() != 3 {
            return None;
        }
        let code: String = rest
            .get(3..)
            .and_then(|r| r.strip_prefix(' '))
            .map(|r| r.chars().take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')).take(MAX_CODE).collect())
            .unwrap_or_default();
        Some(if code.is_empty() { digits } else { format!("{digits} {code}") })
    });
    if let Some(s) = status {
        return s;
    }
    match e {
        NetError::Io(_) => "network",
        NetError::Tls(_) => "secure connection",
        NetError::Auth(_) => "sign-in",
        NetError::Refused(_) => "refused",
        _ => "unexpected answer",
    }
    .into()
}

/// Whether a message is one the check may have posted without knowing it: from the signed-in
/// user (`me`), not deleted, made after `since` or at most [`SWEEP_SLACK`] before it, with one of
/// the two texts it posts exactly (never the edited one: only a message the check knows the id
/// of is edited).
pub(super) fn stray(m: &Value, me: &str, since: i64) -> bool {
    let text = m.pointer("/body/content").and_then(Value::as_str).unwrap_or_default().trim();
    let text = text.strip_prefix("<p>").and_then(|t| t.strip_suffix("</p>")).unwrap_or(text);
    m.pointer("/from/user/id").and_then(Value::as_str) == Some(me)
        && [FIRST, SECOND].contains(&text)
        && m.get("deletedDateTime").is_none_or(Value::is_null)
        && m.get("createdDateTime").and_then(Value::as_str).and_then(ms_of).is_some_and(|t| t >= since.saturating_sub(SWEEP_SLACK))
}

/// Whether a post that failed may have been made all the same: anything but a refusal said
/// before acting (a 4xx, a 429 after its waits, a refused sign-in, a refusal before sending).
fn uncertain(e: &NetError) -> bool {
    match e {
        NetError::Auth(_) | NetError::Refused(_) | NetError::Untrusted { .. } => false,
        NetError::Protocol(m) => !m.starts_with("HTTP 4"),
        NetError::Closed(m) => !m.starts_with("HTTP 429"),
        _ => true,
    }
}

/// An item's `id`, when it is a plausible one.
fn id_of(v: &Value) -> Option<String> {
    text_of(v, "id")
}

/// A short, non-empty text field.
fn text_of(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty() && s.len() <= MAX_ID).map(str::to_string)
}

/// An RFC 3339 time in milliseconds since 1970.
fn ms_of(t: &str) -> Option<i64> {
    let secs = crate::jmap::utc_date(t)?;
    let frac =
        t.trim().get(19..).and_then(|r| r.strip_prefix('.')).map_or("", |r| r.get(..r.bytes().take_while(u8::is_ascii_digit).count()).unwrap_or(""));
    let ms = format!("{:0<3}", frac.get(..frac.len().min(3)).unwrap_or("")).parse::<i64>().ok()?;
    secs.checked_mul(1000)?.checked_add(ms)
}

/// Milliseconds since 1970 as an RFC 3339 UTC time, to the millisecond.
fn rfc3339(ms: i64) -> String {
    let day = utc(ms.div_euclid(1000));
    format!("{}.{:03}Z", day.trim_end_matches('Z'), ms.rem_euclid(1000))
}
