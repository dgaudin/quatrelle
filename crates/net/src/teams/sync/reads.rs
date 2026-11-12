// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A chat's incremental read: `lastModifiedDateTime gt` its watermark less 2 s (Graph's clocks
//! may differ by as much), a page a step; what changed goes in the copy, its unread messages
//! counted, and to the engine, `fresh` once the chat's first read is done. Once the pages are
//! read, the watermark moves to the newest change read, never past Graph's own clock and an hour.
//!
//! * A read stops at [`MESSAGE_PAGES`] pages, on a link already followed, or on a page that brings
//!   nothing new.
//! * A sender the chat's members don't have makes them read again before their messages go, once
//!   for that sender: who they are is the members' to say, not the message's.
//! * Refused (403, 404) from a chat's first read: Graph doesn't serve it here (tried again a day
//!   later); refused once read: the user is no longer in it.

use super::*;
use crate::Result;
use crate::teams::graph::{Range, member_of, message_of};
use crate::teams::store::MAX_UNREAD;
use crate::teams::worker::with_ids;
use quatrelle_model::pim::teams::{ChatMessage, Event, MAX_MEMBERS, MESSAGE_PAGES, MessageKind};

/// The overlap of a read with the one before.
const OVERLAP_MS: i64 = 2_000;
/// The latest time a range is asked from (the end of the year 9999).
const MAX_MS: i64 = 253_402_300_799_999;
/// The most reads held under way at once (a few, one per cadence, in practice).
const MAX_READS: usize = 64;

/// A chat's read under way.
#[derive(Default)]
pub(super) struct Read {
    /// The chat's first read: what it brings is the copy's start, not news.
    first: bool,
    /// Where its range starts: the watermark (held to Graph's clock), or the copy's start.
    start: i64,
    next: Option<String>,
    pages: usize,
    /// The links followed and the messages read, by keyed hashes.
    links: HashSet<u64>,
    ids: HashSet<u64>,
    /// The newest change read.
    newest: Option<i64>,
    changed: usize,
    /// Its pages are read.
    ended: bool,
    /// Messages from senders the members don't have, and those senders: sent once the members
    /// are read again.
    waiting: Option<(Vec<ChatMessage>, Vec<String>)>,
}

/// What a step of a read did.
pub(super) enum ReadStep {
    /// A request went, or the limiter holds the next: more to come.
    Asked,
    /// The read is over (`asked`: a request went in this step).
    Over { asked: bool },
}

/// A step of `chat`'s incremental read, one request at most: `Ok(true)` once the read is over
/// (the watermark moved, what changed sent), `Ok(false)` while a page is left (or the limiter
/// holds it: called again at a later turn). A refusal is dealt with here (the chat marked as
/// Graph doesn't serve it, or as left) and ends the read; any other failure is the caller's.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn incremental(w: &mut Worker, chat: &str, now: Instant) -> Result<bool> {
    Ok(matches!(read_step(w, chat, now)?, ReadStep::Over { .. }))
}

pub(super) fn read_step(w: &mut Worker, chat: &str, now: Instant) -> Result<ReadStep> {
    if !w.sync.reads.contains_key(chat) {
        let Some(read) = begin(w, chat) else { return Ok(ReadStep::Over { asked: false }) };
        if w.sync.reads.len() >= MAX_READS {
            w.sync.reads.clear();
        }
        w.sync.reads.insert(chat.to_string(), read);
    }
    let waiting = w.sync.reads.get(chat).is_some_and(|r| r.waiting.is_some());
    let step = if waiting { strangers_step(w, chat, now) } else { page_step(w, chat, now) };
    if step.is_err() {
        w.sync.reads.remove(chat);
    }
    step
}

/// A read begun, from the chat's watermark, or from the copy's start (`S`, the server's time
/// when it began) for its first. Nothing without Graph's clock to hold times to.
fn begin(w: &Worker, chat: &str) -> Option<Read> {
    let server = server_ms(w)?;
    let cap = server.saturating_add(CLOCK_TOLERANCE_MS);
    let r = w.copy.as_ref()?.chat(chat)?;
    let start = r.watermark.or(r.history_from).unwrap_or(server).min(cap);
    Some(Read { first: r.watermark.is_none(), start, ..Read::default() })
}

/// The next page of a read.
fn page_step(w: &mut Worker, chat: &str, now: Instant) -> Result<ReadStep> {
    let route = Route::Messages(w.key_of(chat));
    if !w.may(route, now) {
        return Ok(ReadStep::Asked);
    }
    let Some((start, next, first)) = w.sync.reads.get(chat).map(|r| (r.start, r.next.clone(), r.first)) else {
        return Ok(ReadStep::Over { asked: false });
    };
    let from = rfc3339(start.saturating_sub(OVERLAP_MS));
    let page = match w.request(route, now, |c| c.messages(chat, Range::After(&from), next.as_deref())) {
        Ok(page) => page,
        Err(e) if refusal(&e) => {
            w.sync.reads.remove(chat);
            refused(w, chat, &e);
            return Ok(ReadStep::Over { asked: true });
        }
        Err(e) => return Err(e),
    };
    let messages = with_ids(page.items.iter().take(MAX_PAGE_ITEMS).map(|v| message_of(chat, v)).collect());
    let keys: Vec<u64> = messages.iter().map(|m| w.key_of(&m.id)).collect();
    let next_key = page.next.as_deref().map(|n| w.key_of(n));
    let changed = match &w.copy {
        Some(copy) => copy.upsert(chat, &messages)?,
        None => Vec::new(),
    };
    count_unread(w, chat, &changed);
    let strangers = strangers(w, chat, &changed);
    let Some(read) = w.sync.reads.get_mut(chat) else { return Ok(ReadStep::Asked) };
    read.pages = read.pages.saturating_add(1);
    let mut news = false;
    for k in keys {
        news |= read.ids.insert(k);
    }
    read.newest = read.newest.max(messages.iter().map(|m| m.modified).max());
    read.changed = read.changed.saturating_add(changed.len());
    let more = news && read.pages < MESSAGE_PAGES;
    read.next = page.next.filter(|n| more && n.len() <= MAX_LINK && next_key.is_some_and(|k| read.links.insert(k)));
    read.ended = read.next.is_none();
    if !strangers.is_empty() {
        read.waiting = Some((changed, strangers));
        return Ok(ReadStep::Asked);
    }
    let ended = read.ended;
    say_messages(w, chat, &changed, !first, false);
    if ended {
        end(w, chat)?;
        return Ok(ReadStep::Over { asked: true });
    }
    Ok(ReadStep::Asked)
}

/// Messages from senders the members don't have: the members read again, then the messages go.
/// Should the members fail to come, the messages go all the same (in the copy already, they'd
/// never be read as news again), their senders judged by what they say, and the failure is said.
fn strangers_step(w: &mut Worker, chat: &str, now: Instant) -> Result<ReadStep> {
    if !w.may(Route::Members, now) {
        return Ok(ReadStep::Asked);
    }
    let got = read_members(w, chat, None, now);
    let Some(read) = w.sync.reads.get_mut(chat) else { return got.map(|()| ReadStep::Asked) };
    let Some((messages, senders)) = read.waiting.take() else { return got.map(|()| ReadStep::Asked) };
    let (first, ended) = (read.first, read.ended);
    if got.is_ok() {
        look(w, chat, &senders);
    }
    say_messages(w, chat, &messages, !first, false);
    got?;
    if ended {
        end(w, chat)?;
        return Ok(ReadStep::Over { asked: true });
    }
    Ok(ReadStep::Asked)
}

/// A read's pages are read: the watermark to the newest change read (never past Graph's clock and
/// an hour, nor back), the time it was read, and a chat that was refused or left is served again.
fn end(w: &mut Worker, chat: &str) -> Result<()> {
    let Some(read) = w.sync.reads.remove(chat) else { return Ok(()) };
    let Some(server) = server_ms(w) else { return Ok(()) };
    let cap = server.saturating_add(CLOCK_TOLERANCE_MS);
    let Some(copy) = &w.copy else { return Ok(()) };
    let Some(mut r) = copy.chat(chat) else { return Ok(()) };
    r.watermark = Some(read.newest.map_or(read.start, |n| n.max(read.start)).min(cap));
    if read.first {
        r.history_from.get_or_insert(read.start);
    }
    r.read_at = Some(server);
    let back = r.chat.unreadable.is_some() || r.chat.left;
    r.chat.unreadable = None;
    r.chat.left = false;
    copy.put_chat(&r)?;
    if read.changed > 0 || read.pages > 1 || back {
        let again = if back { "; served again" } else { "" };
        log::info!("teams: conversation {}: {} messages changed, {} pages read{again}", w.hashed(chat), read.changed, read.pages);
    }
    Ok(())
}

/// Whether a failure is Graph refusing the chat itself: 403 or 404.
fn refusal(e: &NetError) -> bool {
    matches!(http_code(e), Some(403 | 404))
}

/// The HTTP status of a refusal as the client writes it (`HTTP 403 Forbidden`).
pub(super) fn http_code(e: &NetError) -> Option<u16> {
    let NetError::Protocol(m) = e else { return None };
    let rest = m.strip_prefix("HTTP ")?;
    let code = rest.get(..3)?;
    let after = rest.get(3..)?;
    (code.bytes().all(|b| b.is_ascii_digit()) && (after.is_empty() || after.starts_with(' '))).then(|| code.parse().ok()).flatten()
}

/// A read refused. Never read before: Graph doesn't serve this chat here; it is listed all the
/// same and tried again a day later (server time). Read before: the user is no longer in it; it
/// is kept, read-only, and said.
fn refused(w: &mut Worker, chat: &str, e: &NetError) {
    let server = server_ms(w);
    let Some(copy) = &w.copy else { return };
    let Some(mut r) = copy.chat(chat) else { return };
    let gone = r.watermark.is_some() || r.chat.left;
    let said = r.chat.left;
    if gone {
        r.chat.left = true;
    } else if let Some(t) = server {
        r.chat.unreadable = Some(t);
    }
    if let Err(e) = copy.put_chat(&r) {
        log::warn!("teams: a conversation couldn't be kept: {}", kind(&NetError::Store(e)));
    }
    if gone {
        log::info!("teams: conversation {} is no longer the user's ({}); kept to read", w.hashed(chat), kind(e));
        if !said {
            w.say(Event::Removed { chat: chat.to_string() });
        }
    } else {
        log::info!("teams: conversation {} isn't served here ({}); tried again in a day", w.hashed(chat), kind(e));
    }
}

/// A chat's members read again (`updated`: the list's `lastUpdatedDateTime`, kept with them).
/// Refused (403, 404): kept as they were, the name falling back on the topic. The preview's
/// sender, still not among them, isn't looked for again.
pub(super) fn read_members(w: &mut Worker, chat: &str, updated: Option<Option<i64>>, now: Instant) -> Result<()> {
    let members = match w.request(Route::Members, now, |c| c.members_all(chat)) {
        Ok(values) => Some(values.iter().filter_map(member_of).take(MAX_MEMBERS).collect::<Vec<_>>()),
        Err(e) if refusal(&e) => {
            log::info!("teams: the members of conversation {} were refused ({}); kept as they were", w.hashed(chat), kind(&e));
            None
        }
        Err(e) => return Err(e),
    };
    let Some(copy) = &w.copy else { return Ok(()) };
    let Some(mut r) = copy.chat(chat) else { return Ok(()) };
    if let Some(m) = members {
        r.chat.members = m;
    }
    if let Some(u) = updated {
        r.chat.updated = u;
    }
    copy.put_chat(&r)?;
    if let Some(Sender::User { id, .. }) = r.chat.preview.as_ref().map(|p| &p.sender)
        && !r.chat.members.iter().any(|m| m.user_id == *id)
    {
        let id = id.clone();
        look(w, chat, &[id]);
    }
    Ok(())
}

/// The senders of `messages` the chat's members don't have, and not looked for yet.
fn strangers(w: &Worker, chat: &str, messages: &[ChatMessage]) -> Vec<String> {
    let Some(r) = w.copy.as_ref().and_then(|c| c.chat(chat)) else { return Vec::new() };
    let mut out: Vec<String> = Vec::new();
    for m in messages {
        if let Sender::User { id, .. } = &m.sender
            && !r.chat.members.iter().any(|x| x.user_id == *id)
            && !out.contains(id)
            && !looked(w, chat, id)
        {
            out.push(id.clone());
        }
    }
    out
}

/// A chat's unread messages as `changed` come: one from someone else, made after the user last
/// read the chat and not deleted, counts (once, edited or not); one deleted no longer does.
fn count_unread(w: &Worker, chat: &str, changed: &[ChatMessage]) {
    if changed.is_empty() {
        return;
    }
    let Some(copy) = &w.copy else { return };
    let Some(mut r) = copy.chat(chat) else { return };
    let before = r.unread.clone();
    let after = r.chat.last_read.unwrap_or(i64::MIN);
    for m in changed {
        r.unread.retain(|(id, _)| *id != m.id);
        let counts = theirs(&m.sender, &w.settings.object_id) && m.deleted.is_none() && m.kind == MessageKind::Message && m.created > after;
        if counts && r.unread.len() < MAX_UNREAD {
            r.unread.push((m.id.clone(), m.created));
        }
    }
    if r.unread != before
        && let Err(e) = copy.put_chat(&r)
    {
        log::warn!("teams: a conversation's unread messages couldn't be kept: {}", kind(&NetError::Store(e)));
    }
}

/// Whether a message is someone else's than the user's (`me`, their object id): a person's or an
/// application's; a system line is no one's.
pub(super) fn theirs(s: &Sender, me: &str) -> bool {
    match s {
        Sender::User { id, .. } => !me.is_empty() && !id.eq_ignore_ascii_case(me),
        Sender::App { .. } => true,
        Sender::Nobody => false,
    }
}

/// Milliseconds since 1970 as a range's RFC 3339 time (`2026-10-30T09:12:34.567Z`), held to
/// the years 1970 to 9999.
fn rfc3339(ms: i64) -> String {
    let ms = ms.clamp(0, MAX_MS);
    let day = crate::graph::utc(ms.div_euclid(1000));
    format!("{}.{:03}Z", day.trim_end_matches('Z'), ms.rem_euclid(1000))
}
