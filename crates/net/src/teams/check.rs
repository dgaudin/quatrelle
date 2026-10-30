//! The step 0 check: what the owner runs on his own Microsoft 365 account to learn whether
//! Graph serves his Teams chats, before anything else is built on it.
//!
//! It reads the chats and the newest chat's messages, counts what it finds (chats by kind,
//! messages, reactions by form, shared files), tries each write Quatrelle will need on a chat
//! the owner names, and, if asked, runs the approved polling cadences for an hour to measure
//! the 429 rate. It says only counts, yes or no, HTTP status numbers and durations: never a
//! name, an address, a topic, a message's text, a chat or message id or a user's object id
//! (an error is said by its status alone, never its text, which carries the address asked).
//! Every message it posts is soft-deleted before it returns, whatever failed in between.
//!
//! Ages are measured on Graph's own clock (its `Date` header, or failing that the newest time
//! Graph wrote in what was read), never on this computer's.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{Me, Range, TeamsClient};
use crate::graph::utc;
use crate::{NetError, Result};

/// Pages followed at most per listing.
const PAGES: usize = 10;
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
/// Between two writes in the chat (Graph takes one a second per chat).
const PACE: Duration = Duration::from_secs(1);
/// The approved cadences: the chat list, the open chat, the chats active lately.
const LIST_EVERY: Duration = Duration::from_secs(10);
const OPEN_EVERY: Duration = Duration::from_secs(3);
const ACTIVE_EVERY: Duration = Duration::from_secs(60);
/// What "active lately" is (milliseconds), and how many such chats are read at most.
const ACTIVE_FOR: i64 = 7 * 86_400_000;
const ACTIVE_MAX: usize = 20;
/// How often the hour says how it is going.
const REPORT_EVERY: Duration = Duration::from_secs(600);

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
}

/// Run the check, saying what it finds one line at a time. Refusals are said and the check goes
/// on; it fails only when the token is refused from the start.
pub fn run_check(client: &TeamsClient, me: &Me, opts: &CheckOptions, say: &mut dyn FnMut(&str)) -> Result<()> {
    let started = client.server_time().and_then(|s| s.checked_mul(1000));
    let mut ck = Check { client, me, opts, say, seen: None, started, sent: Vec::new() };
    let listed = ck.chats()?;
    let tally = ck.newest_messages(listed.newest.as_deref());
    ck.reactions_and_files(&tally);
    if let Some(chat) = &opts.write_chat {
        ck.write(chat, listed.write_tenant);
    }
    if let Some(d) = opts.hour {
        ck.hour(d, listed.newest, tally.newest);
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
    /// Graph's clock when the check began (milliseconds), by its `Date`.
    started: Option<i64>,
    /// The test messages posted (their ids).
    sent: Vec<String>,
}

/// What the chat list gave.
#[derive(Default)]
struct Listed {
    /// The chat with the most recent message.
    newest: Option<String>,
    /// The write chat's tenant, as the list gives it.
    write_tenant: Option<String>,
}

/// What the newest chat's messages hold.
#[derive(Default)]
struct Tally {
    messages: usize,
    capped: bool,
    reacted: usize,
    named: usize,
    emoji: usize,
    other: usize,
    files: usize,
    first_file: Option<String>,
    /// The newest `lastModifiedDateTime` (milliseconds): the chat's watermark.
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

    /// 1. The chats, by kind.
    fn chats(&mut self) -> Result<Listed> {
        let mut listed = Listed::default();
        let (mut kinds, mut total, mut newest_at) = ([0usize; 4], 0usize, None);
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
                let kind = match chat.get("chatType").and_then(Value::as_str) {
                    Some("oneOnOne") => 0,
                    Some("group") => 1,
                    Some("meeting") => 2,
                    _ => 3,
                };
                if let Some(k) = kinds.get_mut(kind) {
                    *k += 1;
                }
                let Some(id) = id_of(chat) else { continue };
                let at = chat.pointer("/lastMessagePreview/createdDateTime").and_then(Value::as_str).and_then(ms_of);
                self.saw(at);
                if listed.newest.is_none() || at > newest_at {
                    (listed.newest, newest_at) = (Some(id.clone()), at);
                }
                if self.opts.write_chat.as_deref() == Some(id.as_str()) {
                    listed.write_tenant = text_of(chat, "tenantId");
                }
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
        Ok(listed)
    }

    /// 2. The newest chat's messages, read backwards from now.
    fn newest_messages(&mut self, chat: Option<&str>) -> Tally {
        let mut t = Tally::default();
        let Some(chat) = chat else {
            self.say("messages in the newest chat: 0 (no chat)".into());
            return t;
        };
        let Some(before) = self.now().map(rfc3339) else {
            self.say("messages in the newest chat: not read (no server time)".into());
            return t;
        };
        let mut next: Option<String> = None;
        t.capped = true;
        let mut end = String::new();
        for _ in 0..PAGES {
            let page = match self.client.messages(chat, Range::Before(&before), next.as_deref()) {
                Ok(p) => p,
                Err(e) => {
                    end = format!(", then no ({})", why(&e));
                    t.capped = t.messages > 0;
                    break;
                }
            };
            for m in &page.items {
                t.count(m);
            }
            next = page.next;
            if next.is_none() {
                t.capped = false;
                break;
            }
        }
        self.saw(t.newest);
        if t.messages == 0 && !end.is_empty() {
            self.say(format!("messages in the newest chat: no{}", end.trim_start_matches(", then no")));
        } else {
            let least = if t.capped { "at least " } else { "" };
            self.say(format!("messages in the newest chat: {least}{}{end}", t.messages));
        }
        t
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
        self.messages += 1;
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

/// 5. The writes, on the chat the owner named.
impl Check<'_> {
    fn write(&mut self, chat: &str, listed_tenant: Option<String>) {
        // Sent and not yet deleted; and sends whose outcome isn't known (made or not, no id).
        let mut left: Vec<String> = Vec::new();
        let mut lost = 0usize;
        let since = self.started.or(self.seen);
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
    /// own (from the signed-in user, with one of its texts, made since the check began), deleted.
    /// How many were, and how many couldn't be.
    fn sweep(&mut self, chat: &str, since: Option<i64>, known: &[String]) -> std::result::Result<(usize, usize), String> {
        let since = since.ok_or_else(|| "no server time".to_string())?;
        let page = self.client.messages(chat, Range::After(&rfc3339(since.saturating_sub(1000))), None).map_err(|e| why(&e))?;
        let ours = |m: &&Value| {
            let text = m.pointer("/body/content").and_then(Value::as_str).unwrap_or_default().trim();
            let text = text.strip_prefix("<p>").and_then(|t| t.strip_suffix("</p>")).unwrap_or(text);
            m.pointer("/from/user/id").and_then(Value::as_str) == Some(self.me.id.as_str())
                && [FIRST, EDITED, SECOND].contains(&text)
                && m.get("deletedDateTime").is_none_or(Value::is_null)
                && m.get("createdDateTime").and_then(Value::as_str).and_then(ms_of).is_some_and(|t| t >= since)
        };
        let stray: Vec<String> = page.items.iter().filter(ours).filter_map(id_of).filter(|id| !known.contains(id)).collect();
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

/// 6. The approved cadences, for `d`: the 429 rate they meet.
impl Check<'_> {
    fn hour(&mut self, d: Duration, newest: Option<String>, watermark: Option<i64>) {
        let before = self.client.stats();
        let start = Instant::now();
        let from = self.now();
        // Each chat's watermark (milliseconds): what its incremental reads start after.
        let mut marks: HashMap<String, i64> = HashMap::new();
        if let (Some(chat), Some(at)) = (&newest, watermark.or(from)) {
            marks.insert(chat.clone(), at);
        }
        // The list's chats, with when their last message came.
        let mut list: Vec<(String, Option<i64>)> = Vec::new();
        let (mut at_list, mut at_open, mut at_active, mut at_report) = (Duration::ZERO, Duration::ZERO, Duration::ZERO, REPORT_EVERY);
        let mut failed = 0u32;
        let mut stopped = String::new();
        loop {
            let t = start.elapsed();
            if t >= d {
                break;
            }
            let mut outcome: Vec<Result<()>> = Vec::new();
            if t >= at_list {
                at_list = t.saturating_add(LIST_EVERY);
                outcome.push(self.client.chats(None).map(|p| {
                    list = p
                        .items
                        .iter()
                        .filter_map(|c| Some((id_of(c)?, c.pointer("/lastMessagePreview/createdDateTime")?.as_str().and_then(ms_of))))
                        .collect();
                }));
                let newest_at = list.iter().filter_map(|(_, at)| *at).max();
                self.saw(newest_at);
            }
            if t >= at_open {
                at_open = t.saturating_add(OPEN_EVERY);
                if let Some(chat) = &newest {
                    outcome.push(self.incremental(chat, &mut marks, from));
                }
            }
            if t >= at_active {
                at_active = t.saturating_add(ACTIVE_EVERY);
                let since = self.now().map(|n| n.saturating_sub(ACTIVE_FOR));
                let active: Vec<String> = list
                    .iter()
                    .filter(|(id, at)| Some(id) != newest.as_ref() && since.is_some_and(|s| at.is_some_and(|a| a >= s)))
                    .take(ACTIVE_MAX)
                    .map(|(id, _)| id.clone())
                    .collect();
                for chat in &active {
                    outcome.push(self.incremental(chat, &mut marks, from));
                }
            }
            for r in outcome {
                if let Err(e) = r {
                    failed = failed.saturating_add(1);
                    if matches!(e, NetError::Auth(_)) {
                        stopped = format!(", stopped ({})", why(&e));
                    }
                }
            }
            if !stopped.is_empty() {
                break;
            }
            if t >= at_report {
                at_report = at_report.saturating_add(REPORT_EVERY);
                let s = self.client.stats();
                let (requests, throttled) = (s.requests.saturating_sub(before.requests), s.throttled.saturating_sub(before.throttled));
                self.say(format!("hour, after {} min: requests {requests}, throttled {throttled}, failed {failed}", t.as_secs() / 60));
            }
            let wake = at_list.min(at_open).min(at_active).min(at_report).min(d);
            let wait = wake.saturating_sub(start.elapsed());
            if !wait.is_zero() {
                (self.opts.sleep)(wait);
            }
        }
        let s = self.client.stats();
        let (requests, throttled) = (s.requests.saturating_sub(before.requests), s.throttled.saturating_sub(before.throttled));
        let rate = if requests == 0 { 0.0 } else { f64::from(throttled) * 100.0 / f64::from(requests) };
        self.say(format!(
            "hour, {} s: requests {requests}, throttled {throttled}, failed {failed}, 429 rate {rate:.1} %{stopped}",
            start.elapsed().as_secs()
        ));
    }

    /// A chat's messages changed after its watermark, every page; the watermark moved to the
    /// newest change seen.
    fn incremental(&mut self, chat: &str, marks: &mut HashMap<String, i64>, from: Option<i64>) -> Result<()> {
        let Some(mark) = marks.get(chat).copied().or(from) else { return Ok(()) };
        let after = rfc3339(mark);
        let mut newest = mark;
        let mut next: Option<String> = None;
        for _ in 0..PAGES {
            let page = self.client.messages(chat, Range::After(&after), next.as_deref())?;
            for m in &page.items {
                newest = newest.max(m.get("lastModifiedDateTime").and_then(Value::as_str).and_then(ms_of).unwrap_or(newest));
            }
            match page.next {
                Some(n) => next = Some(n),
                None => break,
            }
        }
        marks.insert(chat.to_string(), newest);
        Ok(())
    }
}

/// An error said without its text (which can carry the address asked): its HTTP status, or
/// what kind of failure it was.
fn why(e: &NetError) -> String {
    if let NetError::Protocol(m) = e
        && m.starts_with("the shared file is larger")
    {
        return format!("over {} MiB", MAX_DOWNLOAD >> 20);
    }
    let text = e.to_string();
    let status = text.split("HTTP ").skip(1).map(|rest| rest.chars().take_while(char::is_ascii_digit).collect::<String>()).find(|s| s.len() == 3);
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

/// Whether a post that failed may have been made all the same: anything but a refusal said
/// before acting (a 4xx, a 429 after its waits, a refused sign-in, a refusal before sending).
fn uncertain(e: &NetError) -> bool {
    match e {
        NetError::Auth(_) | NetError::Refused(_) => false,
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
