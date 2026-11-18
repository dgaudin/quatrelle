// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The Teams worker's outbox: the messages the user sends, and the changes to messages already
//! there (edits, deletions, reactions: `changes.rs`), kept in the copy's outbox
//! (`store/outbox.rs`) until Teams has them, and the chats to mark read. Changes go in their
//! chat's order with the sends and count in the caps, but not among the messages said not sent
//! yet ([`Event::Unsent`]).
//!
//! * A message is taken when it can go: its chat in the copy and not left, its HTML within
//!   [`SEND_MAX`](quatrelle_model::pim::teams::SEND_MAX), under the caps ([`OUTBOX_CHAT_MAX`] of
//!   a chat, [`OUTBOX_MAX`] in all, counted on the records kept, read back or not). Else it is
//!   answered "Not sent" (`seq` 0) and nothing of it is kept. Either way the engine hears the
//!   chat's outbox, naming the message it took.
//! * Each chat's messages go in their order, a second apart at least ([`Route::Send`]): a message
//!   "Not sent" doesn't hold those after it, and one chat never holds another's. A message is
//!   written "Sending" before it's posted, so that a run cut off meanwhile ([`opened`]) checks it
//!   rather than posting it again.
//! * A post without an answer, or answered 5xx, may have gone ("Unknown"): after a wait (its
//!   `Retry-After`, else the worker's pause), the chat's messages changed since a little before
//!   the newest one known when it was taken (never past Graph's clock and an hour) are read
//!   ([`Route::Messages`]), a page a step, four pages at most, for the user's own made after it
//!   whose text, as Graph's reader reads it, is the same. Found, it was sent; the range read to
//!   its end without it, it's posted again, three posts at most, then "Not sent" (couldn't be
//!   confirmed). More changed since than four pages, a time past Graph's, or five checks failing
//!   running (30, 60, 120 then 240 s apart): "Not sent" at once, never posted again.
//!   Once one goes, the chat's others are judged against what came after it: two messages of the
//!   same text are two. A message "Not sent" goes again only when the user asks.
//! * Graph asking to wait (429, 503) holds every request for its `Retry-After`
//!   ([`Worker::busy_wait`]): a 429 is a post Graph didn't take, a 503 one it may have.
//! * Marking a chat read is kept in memory, once a chat, tried three times while Graph is busy or
//!   the network away; its row goes at once.
//!
//! What is hostile here: Graph's answers (whether a message was made, the messages a check reads
//! back and its next pages' links, error codes), and the outbox as read back from the copy (its
//! times too). What it protects: the user's messages (never lost without being said, never posted
//! twice for an answer lost), and Teams' limits (a request a step, the limiter's pace, a
//! `Retry-After` whole, a check's pages bounded, a failing check slowed down then given up). The
//! outbox is read from the copy only when it changes. The log says counts, keyed hashes and HTTP
//! statuses: never a body, a chat's id or a local id.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use quatrelle_mime::{TEAMS_RUNS, teams_html, teams_write_mentions};
use quatrelle_model::pim::teams::{
    CLOCK_TOLERANCE_MS, ChatMessage, ChatText, Event, Mention, OUTBOX_CHAT_MAX, OUTBOX_MAX, Op, OutKind, Outgoing, Part, Sender, Unsent,
};

use super::graph::{Range, message_of};
use super::limits::Route;
use super::store::{Copy, OpRecord, OpState, id_ok, local_ok};
use super::sync::{dirty, flush, http_code, rfc3339, say_messages, server_ms};
use super::worker::{Worker, busy, kind, of_the_network};
use crate::NetError;
use crate::worker::connect::fenced;

mod changes;
mod held;
mod parts;

pub(crate) use changes::gone;
use held::Held;
pub(crate) use parts::{offline_changed, uploaded};

/// The most chats waiting to be marked read.
const MARKS_MAX: usize = 1_000;
/// The tries of a mark while Graph is busy or the network away.
const MARK_TRIES: u8 = 3;
/// The posts of a message that got no answer saying Teams has it; the tries of a change too.
const POSTS: u8 = 3;
/// Graph's clocks may differ by as much: a check reads from this long before its time.
const OVERLAP_MS: i64 = 2_000;
/// The most messages of a check's page looked at (Graph sends the 50 asked).
const CHECK_ITEMS: usize = 200;
/// The most pages a check reads, one a step: past them, more changed in the chat since than a
/// check looks through, and the message is "Not sent" (couldn't be confirmed), not posted again.
const CHECK_PAGES: u32 = 4;
/// A check that failed otherwise than by the network, Graph asking to wait or the sign-in: the
/// next one waits this long, doubled at each failure (30, 60, 120 then 240 s), and the
/// [`CHECK_FAILS_MAX`]th failure running makes the message "Not sent" (couldn't be confirmed).
const CHECK_AGAIN: Duration = Duration::from_secs(30);
const CHECK_FAILS_MAX: u32 = 5;
/// The longest code of Teams' answer a "Not sent" keeps (the model's).
const CODE_MAX: usize = 64;

/// What the worker holds of the outbox between two reads of it, in memory only.
#[derive(Default)]
pub(crate) struct Outbox {
    /// Each chat's first message still to go (not "Not sent"), by number: what a step looks at.
    /// Kept from the last read of the outbox, which is read again whenever it changes.
    heads: Vec<Head>,
    /// The chats to mark read, oldest first, each once, at most [`MARKS_MAX`].
    marks: VecDeque<Mark>,
    /// The checks under way or held back, by message number (one a chat's first message, so
    /// [`OUTBOX_MAX`] at most).
    checks: HashMap<u64, Checking>,
    /// The heads whose last request got no answer at all, held back a while (`held.rs`).
    held: Held,
    /// The messages whose files are being made ready, by number (`parts.rs`).
    uploads: HashMap<u64, parts::Upload>,
}

impl Outbox {
    /// When a head may go again, if it's held back: by its check's wait, or after a request of it
    /// that got no answer.
    fn until(&self, seq: u64) -> Option<Instant> {
        self.checks.get(&seq).and_then(|c| c.until).into_iter().chain(self.held.until(seq)).max()
    }
}

/// A chat's first message still to go.
#[derive(Clone)]
struct Head {
    seq: u64,
    chat: String,
    /// The chat's keyed hash, as the limiter knows it.
    key: u64,
    /// It may have gone ("Unknown", or "Sending" left by a run cut off): checked, not posted.
    check: bool,
}

impl Head {
    fn route(&self) -> Route {
        if self.check { Route::Messages(self.key) } else { Route::Send(self.key) }
    }
}

/// A chat to mark read, and the tries made.
struct Mark {
    chat: String,
    key: u64,
    tries: u8,
}

/// A check between two steps: the next page of its read and the pages read so far; the checks
/// failed in a row, none tried again before `until`.
#[derive(Clone, Default)]
struct Checking {
    next: Option<String>,
    pages: u32,
    fails: u32,
    until: Option<Instant>,
}

/// [`Job::Op`](quatrelle_model::pim::teams::Job::Op): a message to send or a change to one taken
/// into the outbox (or answered "Not sent"), or a chat to mark read.
pub(crate) fn queue(w: &mut Worker, op: Op) {
    match op {
        Op::Send { chat, local, body, mentions, parts } => take(w, chat, local, body, mentions, parts),
        Op::Edit { chat, local, message, created, body, mentions } => {
            changes::take(w, chat, local, OutKind::Edit { message, created }, body, mentions)
        }
        // A deletion or a reaction carries no words.
        Op::Delete { chat, local, message, created } => {
            changes::take(w, chat, local, OutKind::Delete { message, created }, ChatText::default(), Vec::new())
        }
        Op::React { chat, local, message, created, emoji, on } => {
            changes::take(w, chat, local, OutKind::React { message, created, emoji, on }, ChatText::default(), Vec::new())
        }
        Op::MarkRead { chat } => mark(w, chat),
    }
}

/// A message to send: kept as the next of the outbox, waiting, with its parts ([`parts::taken`]),
/// when it can go; else answered "Not sent", nothing kept. Ids no engine makes: dropped unanswered.
fn take(w: &mut Worker, chat: String, local: String, body: ChatText, mentions: Vec<Mention>, parts: Vec<Part>) {
    if !id_ok(&chat) || !local_ok(&local) {
        return log::warn!("teams: a message to send without usable ids was dropped");
    }
    let op = OpRecord { seq: 0, local, chat, body, mentions, what: OutKind::Send, state: OpState::Waiting, tries: 0, after: None, parts: Vec::new() };
    // Switched off or signed out meanwhile: nothing can keep it.
    let Some(copy) = &w.copy else { return refuse(w, &[], op, Unsent::Refused) };
    let ops = copy.ops();
    let runs: usize = op.body.blocks.iter().map(|b| b.runs.len()).sum();
    let full = copy.op_count().is_none_or(|n| n >= OUTBOX_MAX) || ops.iter().filter(|o| o.chat == op.chat).count() >= OUTBOX_CHAT_MAX;
    let why = match () {
        _ if copy.chat(&op.chat).is_none_or(|r| r.chat.left) => Some(Unsent::Left),
        _ if runs > TEAMS_RUNS || !parts::fits(&op.body, &op.mentions, &parts) => Some(Unsent::TooLong),
        _ if full => Some(Unsent::TooMany),
        _ => None,
    };
    if let Some(why) = why {
        return refuse(w, &ops, op, why);
    }
    let (records, pictures) = match parts::taken(w, &op.chat, parts) {
        Ok(taken) => taken,
        Err(why) => return refuse(w, &ops, op, why),
    };
    // What a check of it looks after: the newest message the copy knows in the chat.
    let newest = copy.last_pages(&op.chat, 1).iter().map(|m| m.created).max();
    let after = newest.or_else(|| server_ms(w).map(|t| t.saturating_sub(OVERLAP_MS)));
    let op = OpRecord { seq: copy.next_seq(), after, parts: records, ..op };
    if let Err(e) = copy.put_op(&op).and_then(|()| parts::keep(copy, &op, pictures)) {
        log::error!("teams: a message to send couldn't be kept: {}", kind(&NetError::Store(e)));
        return refuse(w, &ops, op, Unsent::Refused);
    }
    let mut ops = ops;
    ops.push(op.clone());
    log::info!("teams: a message to conversation {} was taken; {} in the outbox", w.hashed(&op.chat), ops.len());
    said(w, &ops, &op.chat, Some(op.local), Vec::new());
}

/// What goes out, not taken: answered as "Not sent" for `why` (`seq` 0), after the chat's own.
fn refuse(w: &mut Worker, ops: &[OpRecord], op: OpRecord, why: Unsent) {
    log::warn!("teams: {} conversation {} wasn't taken: {why:?}", changes::named(&op.what), w.hashed(&op.chat));
    let (chat, local) = (op.chat.clone(), op.local.clone());
    let item = OpRecord { seq: 0, state: OpState::NotSent { reason: why, code: None }, ..op }.outgoing();
    said(w, ops, &chat, Some(local), vec![item]);
}

/// A chat to mark read: noted once; past [`MARKS_MAX`], the oldest is let go.
fn mark(w: &mut Worker, chat: String) {
    if !id_ok(&chat) {
        return log::warn!("teams: a conversation to mark read without a usable id was dropped");
    }
    if w.outbox.marks.iter().any(|m| m.chat == chat) {
        return;
    }
    if w.outbox.marks.len() >= MARKS_MAX {
        w.outbox.marks.pop_front();
        log::warn!("teams: {MARKS_MAX} conversations wait to be marked read; the oldest is let go");
    }
    let key = w.key_of(&chat);
    w.outbox.marks.push_back(Mark { chat, key, tries: 0 });
}

/// [`Job::Retry`](quatrelle_model::pim::teams::Job::Retry): a message "Not sent" waits to go
/// again, its posts counted from none. Any other is left as it is.
pub(crate) fn retry(w: &mut Worker, seq: u64) {
    let Some(mut op) = w.copy.as_ref().and_then(|c| c.op_at(seq)) else {
        return log::info!("teams: no message there to send again");
    };
    if !matches!(op.state, OpState::NotSent { .. }) {
        log::info!("teams: a message asked to go again isn't one that wasn't sent: left as it is");
        return changed(w, &op.chat);
    }
    log::info!("teams: {} conversation {} goes again, as asked", changes::named(&op.what), w.hashed(&op.chat));
    op.state = OpState::Waiting;
    op.tries = 0;
    parts::again(&mut op);
    settle(w, &op);
}

/// [`Job::Discard`](quatrelle_model::pim::teams::Job::Discard): a message waiting (its upload
/// cancelled), or "Not sent", deleted from the outbox. One being sent or checked stays.
pub(crate) fn discard(w: &mut Worker, seq: u64) {
    let Some(op) = w.copy.as_ref().and_then(|c| c.op_at(seq)) else {
        return log::info!("teams: no message there to delete");
    };
    if matches!(op.state, OpState::Waiting | OpState::NotSent { .. }) {
        if let Some(copy) = &w.copy
            && let Err(e) = copy.remove_op(seq)
        {
            log::error!("teams: a message couldn't be deleted from the outbox: {}", kind(&NetError::Store(e)));
        }
        parts::let_go(w, seq);
        log::info!("teams: a message to conversation {} was deleted from the outbox, as asked", w.hashed(&op.chat));
    } else {
        log::info!("teams: a message being sent isn't deleted: it may be in Teams already");
    }
    changed(w, &op.chat);
}

/// One step of the outbox at `now`, a request at most: a chat to mark read, else the first
/// chat's message (by number) that may go; whether a request went. Nothing while offline or
/// signed out.
pub(crate) fn step(w: &mut Worker, now: Instant) -> bool {
    if !w.polling() || w.offline() || w.copy.is_none() {
        return false;
    }
    if mark_step(w, now) {
        return true;
    }
    let mut chosen = None;
    for i in 0..w.outbox.heads.len() {
        let Some((seq, route)) = w.outbox.heads.get(i).map(|h| (h.seq, h.route())) else { break };
        if w.outbox.until(seq).is_some_and(|until| now < until) || w.transfers.uploading(seq) || !w.may(route, now) {
            continue;
        }
        chosen = w.outbox.heads.get(i).cloned();
        break;
    }
    let Some(head) = chosen else { return false };
    if head.check {
        check(w, &head, now)
    } else {
        post(w, &head, now)
    }
    true
}

/// When the next mark or message may go, by the limiter (a head held back by its own wait); none
/// when there's nothing to send, or nothing is asked of Teams.
pub(crate) fn due(w: &Worker, now: Instant) -> Option<Instant> {
    if !w.polling() || w.offline() {
        return None;
    }
    let at = |route: Route| now.checked_add(w.limiter.wait(route, now));
    let marks = w.outbox.marks.iter().filter_map(|m| at(Route::Send(m.key)));
    let heads = w.outbox.heads.iter().filter(|h| !w.transfers.uploading(h.seq)).filter_map(|h| {
        let may = at(h.route())?;
        Some(w.outbox.until(h.seq).map_or(may, |until| may.max(until)))
    });
    marks.chain(heads).min()
}

/// The chat was left (or deleted): its messages and changes still to go won't, said "Not sent",
/// their uploads let go ([`parts::let_go`]); its reactions leave the outbox, each said once.
pub(crate) fn left(w: &mut Worker, chat: &str) {
    let Some(copy) = &w.copy else { return };
    let (mut moved, mut gone, mut parted) = (0usize, Vec::new(), Vec::new());
    for op in copy.ops().into_iter().filter(|o| o.chat == chat && !matches!(o.state, OpState::NotSent { .. })) {
        parted.extend((!op.parts.is_empty()).then_some(op.seq));
        let not_made = OpRecord { state: OpState::NotSent { reason: Unsent::Left, code: None }, ..op };
        let reaction = matches!(not_made.what, OutKind::React { .. });
        match if reaction { copy.remove_op(not_made.seq) } else { copy.put_op(&not_made) } {
            Ok(()) if reaction => gone.push(OpRecord { seq: 0, ..not_made }.outgoing()),
            Ok(()) => moved = moved.saturating_add(1),
            Err(e) => log::error!("teams: a message's state couldn't be kept: {}", kind(&NetError::Store(e))),
        }
    }
    if moved > 0 || !gone.is_empty() {
        log::info!(
            "teams: {moved} messages and changes to conversation {} won't be made, {} reactions let go: it was left",
            w.hashed(chat),
            gone.len()
        );
        let ops = copy.ops();
        parted.into_iter().for_each(|seq| parts::let_go(w, seq));
        said(w, &ops, chat, None, gone);
    }
}

/// The copy was opened: what the steps look at begun again (the marks of before let go), a
/// message left "Sending" (a run cut off during its request) checked rather than posted again, a
/// change left so made again ([`changes::opened`]), a message with files or pictures the copy
/// alone vouches for let go, its files attached again ([`parts::opened`]), and the outbox's count
/// said.
pub(crate) fn opened(w: &mut Worker) {
    w.outbox = Outbox::default();
    let Some(copy) = &w.copy else { return };
    let mut ops = copy.ops();
    let mut cut = 0usize;
    for op in ops.iter_mut().filter(|o| o.what == OutKind::Send && o.state == OpState::Sending) {
        op.state = OpState::Unknown;
        match copy.put_op(op) {
            Ok(()) => cut = cut.saturating_add(1),
            Err(e) => log::error!("teams: a message's state couldn't be kept: {}", kind(&NetError::Store(e))),
        }
    }
    if cut > 0 {
        log::info!("teams: {cut} messages were being sent when the last run stopped: whether Teams has them is checked");
    }
    for item in changes::opened(w, &mut ops).into_iter().chain(parts::opened(w, &mut ops)) {
        let chat = item.chat.clone();
        snapshot(w, &ops, &chat, None, vec![item]);
    }
    w.say(Event::Unsent { count: count(&ops) });
    keep(w, &ops);
}

/// A chat opened: its messages going out said, none as well.
pub(crate) fn say_chat(w: &Worker, chat: &str) {
    let ops = w.copy.as_ref().map(Copy::ops).unwrap_or_default();
    snapshot(w, &ops, chat, None, Vec::new());
}

/// Signing out: every message going out forgotten, in the copy (open or not) and here, and said.
pub(crate) fn signed_out(w: &mut Worker) {
    let cleared = match &w.copy {
        Some(copy) => copy.clear_ops(),
        None => Copy::clear_outbox(&w.store),
    };
    if let Err(e) = cleared {
        log::error!("teams: the outbox couldn't be emptied: {}", kind(&NetError::Store(e)));
    }
    w.outbox = Outbox::default();
    w.say(Event::Unsent { count: 0 });
}

/// Teams' answer as a "Not sent" keeps it: the status and Graph's code as the client writes them
/// (`HTTP 403 Forbidden`), the status alone past [`CODE_MAX`] characters; none without a status.
pub(super) fn answer_code(e: &NetError) -> Option<String> {
    let status = http_code(e)?;
    let NetError::Protocol(m) = e else { return None };
    let fits = m.len() <= CODE_MAX && m.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b' ' | b'_' | b'.' | b'-'));
    Some(if fits { m.clone() } else { format!("HTTP {status}") })
}

/// A chat to mark read, the first whose turn it is: done, its row goes at once; Graph busy or the
/// network away, tried again ([`MARK_TRIES`] in all); refused, let go. Whether a request went.
fn mark_step(w: &mut Worker, now: Instant) -> bool {
    let mut chosen = None;
    for i in 0..w.outbox.marks.len() {
        let Some(key) = w.outbox.marks.get(i).map(|m| m.key) else { break };
        if w.may(Route::Send(key), now) {
            chosen = Some(i);
            break;
        }
    }
    let Some(mark) = chosen.and_then(|i| w.outbox.marks.remove(i)) else { return false };
    let (me, tenant) = (w.settings.object_id.clone(), w.settings.tenant_id.clone());
    let hash = w.hashed(&mark.chat);
    match w.request(Route::Send(mark.key), now, |c| c.mark_read(&mark.chat, &me, &tenant)) {
        Ok(()) => {
            log::info!("teams: conversation {hash} marked read");
            marked(w, &mark.chat);
        }
        Err(e) if busy(&e) || of_the_network(&e) => {
            let tries = mark.tries.saturating_add(1);
            if tries < MARK_TRIES {
                log::warn!("teams: conversation {hash} wasn't marked read ({}); tried again", kind(&e));
                w.outbox.marks.push_front(Mark { tries, ..mark });
            } else {
                log::warn!("teams: conversation {hash} wasn't marked read ({}); given up after {MARK_TRIES} tries", kind(&e));
            }
            if busy(&e) {
                w.busy_wait(now);
            } else {
                w.pause(&e, now);
            }
        }
        Err(e) => log::warn!("teams: conversation {hash} wasn't marked read ({}); given up", kind(&e)),
    }
    true
}

/// Teams has the chat read: the copy's too, as of Graph's clock, and its row goes at once.
fn marked(w: &mut Worker, chat: &str) {
    let server = server_ms(w);
    if let Some(copy) = &w.copy
        && let Some(mut r) = copy.chat(chat)
    {
        r.chat.last_read = r.chat.last_read.max(server);
        r.unread.clear();
        if let Err(e) = copy.put_chat(&r) {
            log::warn!("teams: a conversation's read state couldn't be kept: {}", kind(&NetError::Store(e)));
        }
    }
    dirty(w, chat);
    flush(w);
}

/// Whether a record is checked rather than posted: a message that may have gone ("Unknown", or
/// "Sending" left by a run cut off). A change never is: it is made again.
fn checked(o: &OpRecord) -> bool {
    o.what == OutKind::Send && matches!(o.state, OpState::Unknown | OpState::Sending)
}

/// A step's message as the copy has it now; none (the outbox read again) when it's gone or no
/// longer where the step thought.
fn current(w: &mut Worker, head: &Head) -> Option<OpRecord> {
    let op = w
        .copy
        .as_ref()
        .and_then(|c| c.op_at(head.seq))
        .filter(|o| o.chat == head.chat && checked(o) == head.check && !matches!(o.state, OpState::NotSent { .. }));
    if op.is_none() {
        changed(w, &head.chat);
    }
    op
}

/// A message posted, or a change made ([`changes::make`]): written "Sending" first; then sent,
/// waiting again (Graph didn't take it), "Unknown" (it may have gone: checked after a wait), or
/// "Not sent" (refused, too long).
fn post(w: &mut Worker, head: &Head, now: Instant) {
    let Some(mut op) = current(w, head) else { return };
    let send = op.what == OutKind::Send;
    // Its files made ready first, a step each ([`parts::ready`]).
    if send && !op.parts.is_empty() && !matches!(parts::ready(w, head, &mut op, now), parts::Ready::Post) {
        return;
    }
    // A change's words are written as it's made.
    let Ok(html) = (if send { teams_write_mentions(&op.body, &op.mentions) } else { Ok(String::new()) }) else {
        op.state = OpState::NotSent { reason: Unsent::TooLong, code: None };
        return settle(w, &op);
    };
    if send {
        op.after = op.after.or_else(|| server_ms(w).map(|t| t.saturating_sub(OVERLAP_MS)));
    }
    op.state = OpState::Sending;
    if let Some(copy) = &w.copy
        && let Err(e) = copy.put_op(&op)
    {
        let e = NetError::Store(e);
        log::error!(
            "teams: {} conversation {} wasn't made: its state couldn't be kept ({})",
            changes::named(&op.what),
            w.hashed(&head.chat),
            kind(&e)
        );
        return w.pause(&e, now);
    }
    if !send {
        return changes::make(w, head, op, now);
    }
    let chat = head.chat.clone();
    let made = match op.parts.is_empty() {
        true => w.request(Route::Send(head.key), now, |c| c.send_html(&chat, &html, &op.mentions)),
        false => parts::post(w, head, &op, now),
    };
    let e = match made {
        Ok(v) => return sent(w, &op, message_of(&chat, &v)),
        Err(e) => e,
    };
    op.state = after_post(&e);
    let unknown = op.state == OpState::Unknown;
    let then = match &op.state {
        OpState::Unknown => "it is checked after a wait",
        OpState::Waiting => "it waits",
        _ => "not sent",
    };
    if unknown {
        op.tries = op.tries.saturating_add(1);
    }
    log::warn!("teams: a message to conversation {} failed ({}); {then}", w.hashed(&chat), kind(&e));
    settle(w, &op);
    // A message that may have been made is checked once Graph had time to list it: after its
    // `Retry-After` when busy, else after the worker's pause (a 5xx, an answer lost or unread).
    if busy(&e) {
        w.busy_wait(now);
    } else if unknown {
        w.pause(&e, now);
    }
}

/// Where a post that failed with `e` leaves its message.
fn after_post(e: &NetError) -> OpState {
    match e {
        // Not taken: refused for its token (renewed already, or "sign in again"), or Graph asking
        // to wait first.
        NetError::Auth(_) => OpState::Waiting,
        NetError::Closed(m) if m.starts_with("HTTP 429") => OpState::Waiting,
        // Refused before any request.
        NetError::Refused(_) => OpState::NotSent { reason: Unsent::Refused, code: None },
        _ => match http_code(e) {
            Some(413) => OpState::NotSent { reason: Unsent::TooLong, code: answer_code(e) },
            Some(_) => OpState::NotSent { reason: Unsent::Refused, code: answer_code(e) },
            // A 5xx, or no answer at all: it may have gone.
            None => OpState::Unknown,
        },
    }
}

/// A message that may have gone, checked: the chat's messages changed since a little before its
/// time, a page a step ([`CHECK_PAGES`] at most), for the user's own of the same text made after
/// it. Found, it was sent; the range read to its end without it, posted again ([`POSTS`] posts at
/// most) or "Not sent". Pages left past the last, or a time past Graph's clock and its tolerance
/// (nothing sent now can be told from it), it can't be told: "Not sent" (couldn't be confirmed).
fn check(w: &mut Worker, head: &Head, now: Instant) {
    let Some(mut op) = current(w, head) else { return };
    // Never past Graph's clock and its tolerance, as a read's watermark: a time read back from the
    // copy, or chained from one Graph gave, can't put the range ahead.
    let cap = server_ms(w).map(|t| t.saturating_add(CLOCK_TOLERANCE_MS));
    let after = op.after.map(|a| cap.map_or(a, |cap| a.min(cap)));
    let from = rfc3339(after.unwrap_or(0).saturating_sub(OVERLAP_MS));
    let chat = head.chat.clone();
    let checking = w.outbox.checks.get(&op.seq).cloned().unwrap_or_default();
    let page = match w.request(Route::Messages(head.key), now, |c| c.messages(&chat, Range::After(&from), checking.next.as_deref())) {
        Ok(page) => page,
        Err(e) => return check_failed(w, &mut op, &e, now),
    };
    let (me, text) = (w.settings.object_id.clone(), as_read(&op.body, &op.mentions, &chat));
    let found = page.items.iter().take(CHECK_ITEMS).map(|v| message_of(&chat, v)).find(|m| {
        let mine = matches!(&m.sender, Sender::User { id, .. } if !me.is_empty() && id.eq_ignore_ascii_case(&me));
        let same = mine && !m.id.is_empty() && m.deleted.is_none() && after.is_none_or(|a| m.created > a) && flat(&m.body.plain()) == text;
        same && parts::same_files(&op, m)
    });
    if let Some(m) = found {
        w.outbox.checks.remove(&op.seq);
        log::info!("teams: a message to conversation {} was found sent", w.hashed(&chat));
        return sent(w, &op, m);
    }
    let pages = checking.pages.saturating_add(1);
    let ended = page.next.is_none();
    if let Some(next) = page.next.filter(|_| pages < CHECK_PAGES) {
        w.outbox.checks.insert(op.seq, Checking { next: Some(next), pages, ..checking });
        return;
    }
    w.outbox.checks.remove(&op.seq);
    let again = ended && after == op.after && op.tries < POSTS;
    let then = match () {
        _ if again => "posted again",
        _ if !ended => "more changed since than a check reads: not sent",
        _ if after != op.after => "its time is past Graph's: not sent",
        _ => "not sent",
    };
    log::warn!("teams: a message to conversation {} wasn't found sent; {then}", w.hashed(&chat));
    op.state = if again { OpState::Waiting } else { OpState::NotSent { reason: Unsent::Unconfirmed, code: None } };
    settle(w, &op);
}

/// A check that failed. Graph asking to wait, the network away or the sign-in refused: it waits
/// for them, and goes on from the page it was at. Anything else: the next waits, longer each time,
/// and reads the range from its start again; the [`CHECK_FAILS_MAX`]th running, the message is
/// "Not sent" (couldn't be confirmed).
fn check_failed(w: &mut Worker, op: &mut OpRecord, e: &NetError, now: Instant) {
    let hash = w.hashed(&op.chat);
    if busy(e) || of_the_network(e) || matches!(e, NetError::Auth(_)) {
        log::warn!("teams: a message to conversation {hash} couldn't be checked ({}); checked again later", kind(e));
        if busy(e) {
            w.busy_wait(now);
        } else if of_the_network(e) {
            w.pause(e, now);
        }
        return;
    }
    let fails = w.outbox.checks.get(&op.seq).map_or(0, |c| c.fails).saturating_add(1);
    if fails >= CHECK_FAILS_MAX {
        log::warn!("teams: a message to conversation {hash} couldn't be checked ({}), {fails} times running; not sent", kind(e));
        w.outbox.checks.remove(&op.seq);
        op.state = OpState::NotSent { reason: Unsent::Unconfirmed, code: None };
        return settle(w, op);
    }
    log::warn!("teams: a message to conversation {hash} couldn't be checked ({}); checked again later", kind(e));
    let wait = CHECK_AGAIN.saturating_mul(1u32.checked_shl(fails.saturating_sub(1)).unwrap_or(u32::MAX));
    w.outbox.checks.insert(op.seq, Checking { fails, until: now.checked_add(wait), ..Checking::default() });
}

/// A text as a check compares it: its runs of white space one space, trimmed.
fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A message's body as a check compares it with what Graph lists: its HTML as it went (its
/// mentions with it), read back by Graph's own reader (as every message read is: entities, spaces
/// and wrappers as Graph's rewriting leaves them), then [`flat`]. Should the reader fail, its own
/// text.
fn as_read(body: &ChatText, mentions: &[Mention], chat: &str) -> String {
    let read = teams_write_mentions(body, mentions).ok().and_then(|html| fenced(|| teams_html(&html, chat, "")));
    flat(&read.as_ref().unwrap_or(body).plain())
}

/// Teams has the message `op` was: `made`, as Graph gave it (the post's answer, or found by a
/// check). Its record goes; the chat's others are judged against what came after it; it goes in
/// the copy and to the engine.
fn sent(w: &mut Worker, op: &OpRecord, made: ChatMessage) {
    let made = (!made.id.is_empty()).then_some(made);
    let mut ops = Vec::new();
    if let Some(copy) = &w.copy {
        if let Err(e) = copy.remove_op(op.seq) {
            log::error!("teams: a message sent couldn't leave the outbox: {}", kind(&NetError::Store(e)));
        }
        ops = copy.ops();
        if let Some(m) = &made {
            // Not itself, should it still be there (its removal failed): its check must find it.
            // Changes are never checked.
            let later = |o: &OpRecord| o.seq != op.seq && o.chat == op.chat && o.what == OutKind::Send && o.after.is_none_or(|a| a < m.created);
            for other in ops.iter_mut().filter(|o| later(o)) {
                other.after = Some(m.created);
                if let Err(e) = copy.put_op(other) {
                    log::error!("teams: a message's state couldn't be kept: {}", kind(&NetError::Store(e)));
                }
            }
            if let Err(e) = copy.upsert(&op.chat, std::slice::from_ref(m)) {
                log::warn!("teams: a message sent couldn't be kept in the copy: {}", kind(&NetError::Store(e)));
            }
        }
    }
    match &made {
        Some(m) => say_messages(w, &op.chat, std::slice::from_ref(m), false, false),
        None => log::warn!("teams: Graph's answer to a message sent doesn't say which it is; it comes with the next read"),
    }
    log::info!("teams: a message to conversation {} was sent", w.hashed(&op.chat));
    said(w, &ops, &op.chat, None, Vec::new());
}

/// A message's record kept as it now stands, and the outbox said.
fn settle(w: &mut Worker, op: &OpRecord) {
    if let Some(copy) = &w.copy
        && let Err(e) = copy.put_op(op)
    {
        log::error!("teams: a message's state couldn't be kept: {}", kind(&NetError::Store(e)));
    }
    changed(w, &op.chat);
}

/// The outbox read again after a change to `chat`'s messages, and said.
fn changed(w: &mut Worker, chat: &str) {
    let ops = w.copy.as_ref().map(Copy::ops).unwrap_or_default();
    said(w, &ops, chat, None, Vec::new());
}

/// What the outbox holds now (`ops`): `chat`'s outbox said ([`snapshot`]); the count in all; and
/// what the steps look at.
fn said(w: &mut Worker, ops: &[OpRecord], chat: &str, took: Option<String>, extra: Vec<Outgoing>) {
    snapshot(w, ops, chat, took, extra);
    w.say(Event::Unsent { count: count(ops) });
    keep(w, ops);
}

/// `chat`'s messages and changes going out, of `ops`, said; `extra` after them (not kept: not
/// taken, or a reaction let go), `took` naming what was just answered for.
fn snapshot(w: &Worker, ops: &[OpRecord], chat: &str, took: Option<String>, extra: Vec<Outgoing>) {
    let mut items: Vec<Outgoing> = ops.iter().filter(|o| o.chat == chat).map(|o| parts::shown(w, o.outgoing())).collect();
    items.extend(extra);
    w.say(Event::Outbox { chat: chat.to_string(), items, took });
}

/// The messages to send the outbox holds: its changes to messages already there aren't counted.
fn count(ops: &[OpRecord]) -> u32 {
    u32::try_from(ops.iter().filter(|o| o.what == OutKind::Send).count()).unwrap_or(u32::MAX)
}

/// What the steps look at, from `ops` (by number): each chat's first message or change still to
/// go. The checks of messages no longer checked, and the holds of records no longer heads, are
/// forgotten.
fn keep(w: &mut Worker, ops: &[OpRecord]) {
    let mut seen = HashSet::new();
    let heads: Vec<Head> = ops
        .iter()
        .filter(|o| !matches!(o.state, OpState::NotSent { .. }) && seen.insert(o.chat.as_str()))
        .map(|o| Head { seq: o.seq, chat: o.chat.clone(), key: w.key_of(&o.chat), check: checked(o) })
        .collect();
    w.outbox.checks.retain(|seq, _| heads.iter().any(|h| h.check && h.seq == *seq));
    w.outbox.held.retain(|seq| heads.iter().any(|h| h.seq == seq));
    w.outbox.uploads.retain(|seq, _| heads.iter().any(|h| h.seq == *seq));
    w.outbox.heads = heads;
}
