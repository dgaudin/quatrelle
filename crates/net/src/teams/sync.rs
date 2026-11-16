// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The synchronisation: the approved cadences, run by the Teams worker's turns a request at a
//! time, in this order: the chat on screen (every 3 s), the chat list (every 10 s, all of it
//! every hour), the chats active in the last 7 days (every minute).
//!
//! * The list (`list.rs`): read a page a step, each chat kept with what it needs; at the hourly
//!   pass, the month's chats refreshed and those the whole list no longer has left.
//! * A chat's read (`reads.rs`): its messages changed since its watermark, page after page; then
//!   the watermark moves, never past Graph's own clock and an hour.
//! * The rows (`rows.rs`): what the list shows of a chat, sent when it changed, at most
//!   [`EVENT_ITEMS`] an event, once the user's own id is known.
//!
//! What is hostile here: Graph's answers (paging links that go round, pages that bring nothing
//! new, times ahead of its clock, answers that refuse), what the copy holds (a watermark far
//! ahead), and the chats' members and senders. What it protects: the work a server can cause
//! (pages per pass, links seen, one request a step under the limiter, none during a
//! `Retry-After`), and who said it (a sender the members don't have makes them read again before
//! the message or the row says who). Ages are measured on Graph's clock only, never this
//! computer's.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use quatrelle_model::pim::teams::{CLOCK_TOLERANCE_MS, Chat, Event, MAX_CHATS, Sender, Status};

use super::backfill::count_panic;
use super::limits::{Cadence, Route};
use super::store::{ChatRecord, id_ok};
use super::worker::{Worker, busy, kind, of_the_network};
use crate::NetError;
use crate::worker::connect::fenced;

mod links;
mod list;
mod news;
mod reads;
mod rows;

// The cadences go through `read_step` (the tests through `incremental`). The history
// (`backfill.rs`) keeps what it reads as the reads do.
#[cfg(test)]
pub(crate) use links::built_chat_link;
use list::{Pass, list_refused, list_step};
#[cfg(test)]
pub(crate) use reads::incremental;
use reads::{Read, ReadStep, read_members, read_step};
pub(crate) use reads::{count_unread, http_code, refusal, refused, rfc3339};
pub(crate) use rows::{dirty, flush, say_members, say_messages};

const DAY_MS: i64 = 86_400_000;
/// The chats read every minute: active in the last 7 days.
const ACTIVE_MS: i64 = 7 * DAY_MS;
/// The chats refreshed at the hourly pass: active in the last 30 days.
const MONTH_MS: i64 = 30 * DAY_MS;
/// The longest link to a listing's next page followed.
const MAX_LINK: usize = 8 << 10;
/// The most items of a page taken (chats, messages): Graph sends the 50 asked; past this many,
/// what a server sends is passed over, so that no step grows with it.
const MAX_PAGE_ITEMS: usize = 200;
/// The most (chat, sender) pairs remembered as looked for among the members.
const MAX_LOOKED: usize = 10_000;
/// A chat's members or messages failing (not refused): not asked again for this long, doubled at
/// each failure up to [`BACK_OFF_MAX`]; the other chats go on meanwhile.
const BACK_OFF: Duration = Duration::from_secs(30);
const BACK_OFF_MAX: Duration = Duration::from_secs(3600);

/// What the synchronisation holds between two steps, in memory only: what has to last is in the
/// copy. Every collection is bounded (by [`MAX_CHATS`], [`MAX_LOOKED`], or a few reads).
#[derive(Default)]
pub(crate) struct SyncState {
    /// The list pass under way.
    pass: Option<Pass>,
    /// The minute's pass under way: the week's chats.
    active: Option<Round>,
    /// The chat on screen's read under way.
    open: Option<Round>,
    /// The hourly refresh under way: the month's chats, and older ones once a week, their members
    /// and messages read again; after the other cadences, never holding them.
    hourly: Option<Round>,
    /// Chats' members or messages that failed, waiting on their own (keyed hashes).
    backoff: HashMap<u64, Backoff>,
    /// The chat list was refused: not asked again before then.
    list_refused: Option<Instant>,
    /// A chat was just opened: an older one's members are read again with its first read.
    opening: bool,
    /// The chats' reads under way, by chat.
    reads: HashMap<String, Read>,
    /// Each chat listed and its last activity: the minute's pass reads the week's.
    activity: HashMap<String, Option<i64>>,
    /// The newest activity listed: the next list pass reads no further back.
    newest: Option<i64>,
    /// What the engine was last sent of each chat's row (its hash), by the chat's keyed hash.
    shown: HashMap<u64, u64>,
    /// The chats whose row may have changed, to be sent, each once.
    dirty: Vec<String>,
    dirtied: HashSet<String>,
    /// The (chat, sender) pairs already looked for among the chat's members (keyed hashes).
    looked: HashSet<u64>,
    /// The open chat's members and link as last said to the engine (their hash).
    members_said: Option<u64>,
    /// The chats of the copy the last whole list lacked, when it came to its end by itself (their
    /// hashes in the copy): the next whole list takes them, and leaves those it lacks too.
    absent: HashSet<String>,
    /// The kind and link of each chat listed, for the log's figures.
    links: links::Links,
    /// A listing's count (conversations, pages) as the log last said it: said again only once
    /// it changes (the list is read every ten seconds).
    listed_said: Option<(usize, usize)>,
}

impl SyncState {
    /// What is under way given up (a step panicked); what the engine was sent stays known.
    pub(crate) fn give_up(&mut self) {
        self.pass = None;
        self.active = None;
        self.open = None;
        self.hourly = None;
        self.reads.clear();
    }
}

/// A chat's work that failed: not asked again before `until`; `fails` in a row.
#[derive(Clone, Copy)]
struct Backoff {
    until: Instant,
    fails: u32,
}

/// What of a chat failed.
#[derive(Clone, Copy)]
enum Work {
    Members,
    Messages,
}

impl Work {
    fn what(self) -> &'static str {
        match self {
            Work::Members => "its members",
            Work::Messages => "its messages",
        }
    }
}

/// A cadence's chats to go through, in order.
#[derive(Default)]
struct Round {
    todo: VecDeque<Task>,
}

/// What a chat needs.
#[derive(Clone)]
struct Task {
    chat: String,
    /// Its members read (again).
    members: bool,
    /// The list's `lastUpdatedDateTime`, kept once the members are read.
    updated: Option<Option<i64>>,
    /// Its messages read.
    read: bool,
    /// Passed over when Graph doesn't serve it or the user left it (the open chat, the week's).
    guarded: bool,
    /// Its new last message as the list gives it, said once its members are read again (a chat
    /// Graph won't serve, from a sender they didn't have).
    preview: bool,
}

/// Which cadence's work.
#[derive(Clone, Copy)]
enum Which {
    Open,
    List,
    Active,
    Hourly,
}

/// What a step of a chat's work did.
enum Progress {
    /// A request went, or the limiter holds the next: the turn is used.
    Asked,
    /// This chat's work is over and nothing was asked: the next one's can go.
    Next,
}

/// One step of whatever cadence is due, in their order: the open chat, the list (all of it when
/// the hour has come), the week's chats, then the hour's refresh. One request at most; whether the
/// turn is used (else the history may go).
pub(crate) fn tick(w: &mut Worker, now: Instant) -> bool {
    if list_refused(w, now) {
        return true;
    }
    if open_step(w, now) || list_step(w, now) || active_step(w, now) {
        return true;
    }
    w.sync.hourly.is_some() && run_round(w, Which::Hourly, now)
}

/// [`Job::Open`](quatrelle_model::pim::teams::Job::Open): the chat now on screen, none for the
/// list only. Its last two pages go to the engine at once, from the copy (online or not); then it
/// is read every three seconds.
pub(crate) fn open(w: &mut Worker, chat: Option<String>) {
    let Some(chat) = on_screen(w, chat) else { return };
    let messages = w.copy.as_ref().map(|c| c.last_pages(&chat, 2)).unwrap_or_default();
    say_messages(w, &chat, &messages, false, false);
    say_opened(w, &chat);
}

/// [`Job::Find`](quatrelle_model::pim::teams::Job::Find): `chat` on screen at its message
/// `message` (a search's hit), as [`open`] puts it there, but its messages from that message's
/// page on ([`Copy::messages_from`](super::store::Copy::messages_from)), then
/// [`Event::Placed`] saying whether they reach its newest. A message the copy doesn't have opens
/// it as usual: its last two pages, the newest reached.
pub(crate) fn find(w: &mut Worker, chat: String, message: String) {
    let Some(chat) = on_screen(w, Some(chat)) else { return };
    let copy = w.copy.as_ref();
    let found = copy.and_then(|c| c.messages_from(&chat, &message));
    let (messages, newest) = found.unwrap_or_else(|| (copy.map(|c| c.last_pages(&chat, 2)).unwrap_or_default(), true));
    say_messages(w, &chat, &messages, false, false);
    w.say(Event::Placed { chat: chat.clone(), message, newest });
    say_opened(w, &chat);
}

/// `chat` (none: the list only) now the one on screen: the last one's read under way dropped. A
/// chat with a usable id is returned, what it waits after failing forgiven (the user asks for
/// it), and its kind, whether it can be read and its link said in the log.
fn on_screen(w: &mut Worker, chat: Option<String>) -> Option<String> {
    if let Some(t) = w.sync.open.take().and_then(|r| r.todo.front().cloned()) {
        w.sync.reads.remove(&t.chat);
    }
    w.open = chat.filter(|c| id_ok(c));
    w.sync.opening = w.open.is_some();
    let chat = w.open.clone()?;
    forgive(w, Work::Members, &chat);
    forgive(w, Work::Messages, &chat);
    // Its link: built whenever it can be, else Graph's as the window will judge it.
    let r = w.copy.as_ref().and_then(|c| c.chat(&chat)).map(|r| r.chat);
    let url = r.as_ref().and_then(|c| c.web_url.as_deref());
    let built = links::built_chat_link(&chat, &w.settings.tenant_id).is_some();
    log::info!("{}", links::opened(r.as_ref().map(|c| &c.kind), r.as_ref().is_some_and(|c| c.unreadable.is_some()), url, built));
    Some(chat)
}

/// The chat just put on screen, once its messages are said: its members and link, then its
/// messages going out (none as well); then it's read at once, and every three seconds.
fn say_opened(w: &mut Worker, chat: &str) {
    w.sync.members_said = None;
    say_members(w);
    super::outbox::say_chat(w, chat);
    w.limiter.prompt(Cadence::Open);
}

/// [`Job::Page`](quatrelle_model::pim::teams::Job::Page): the copy's page before `before`, for
/// the chat on screen scrolled up.
pub(crate) fn page(w: &mut Worker, chat: &str, before: i64) {
    if !id_ok(chat) {
        return;
    }
    let messages = w.copy.as_ref().map(|c| c.before(chat, before)).unwrap_or_default();
    say_messages(w, chat, &messages, false, true);
}

/// The server's time (milliseconds since 1970), by the newest `Date` Graph sent.
pub(crate) fn server_ms(w: &Worker) -> Option<i64> {
    w.client.as_ref()?.server_time()?.checked_mul(1000)
}

/// Whether a chat's last activity is within `span` of the server's time.
fn recent(at: Option<i64>, server: Option<i64>, span: i64) -> bool {
    matches!((at, server), (Some(at), Some(now)) if at >= now.saturating_sub(span))
}

/// The open chat's read, every three seconds: not a chat Graph doesn't serve or the user left.
/// An older chat just opened has its members read again too.
fn open_step(w: &mut Worker, now: Instant) -> bool {
    if w.sync.open.is_none() {
        if !w.limiter.due(Cadence::Open, now) {
            return false;
        }
        let Some(chat) = w.open.clone() else {
            w.limiter.ran(Cadence::Open, now);
            return false;
        };
        let opening = std::mem::take(&mut w.sync.opening);
        let at = w.copy.as_ref().and_then(|c| c.chat(&chat)).and_then(|r| r.chat.last_activity);
        let members = opening && !recent(at, server_ms(w), MONTH_MS);
        w.sync.open = Some(Round { todo: VecDeque::from([Task { chat, members, updated: None, read: true, guarded: true, preview: false }]) });
    }
    run_round(w, Which::Open, now)
}

/// The minute's pass: the chats active in the last 7 days, newest first, read for the edits,
/// deletions and reactions on their older messages (the chat on screen is read anyway).
fn active_step(w: &mut Worker, now: Instant) -> bool {
    if w.sync.active.is_none() {
        if !w.limiter.due(Cadence::Active, now) {
            return false;
        }
        let server = server_ms(w);
        let open = w.open.clone();
        let mut week: Vec<(i64, String)> = w
            .sync
            .activity
            .iter()
            .filter(|(id, at)| recent(**at, server, ACTIVE_MS) && open.as_ref() != Some(*id))
            .map(|(id, at)| (at.unwrap_or_default(), id.clone()))
            .collect();
        week.sort_by(|a, b| b.cmp(a));
        let todo =
            week.into_iter().map(|(_, chat)| Task { chat, members: false, updated: None, read: true, guarded: true, preview: false }).collect();
        w.sync.active = Some(Round { todo });
    }
    run_round(w, Which::Active, now)
}

fn cadence_of(which: Which) -> Cadence {
    match which {
        Which::Open => Cadence::Open,
        Which::List => Cadence::List,
        Which::Active => Cadence::Active,
        Which::Hourly => Cadence::Hourly,
    }
}

fn round(w: &mut Worker, which: Which) -> Option<&mut Round> {
    match which {
        Which::Open => w.sync.open.as_mut(),
        Which::List => w.sync.pass.as_mut().map(|p| &mut p.round),
        Which::Active => w.sync.active.as_mut(),
        Which::Hourly => w.sync.hourly.as_mut(),
    }
}

/// A step of a cadence's chats: the first one's next request, or, once they're all done, the
/// cadence ran. Whether the turn is used.
fn run_round(w: &mut Worker, which: Which, now: Instant) -> bool {
    loop {
        let Some(task) = round(w, which).and_then(|r| r.todo.front().cloned()) else {
            if round(w, which).is_some() {
                finish(w, which, now);
            }
            return false;
        };
        // A bug met in a chat's work (a panic) is that chat's alone.
        match fenced(|| task_step(w, which, &task, now)) {
            Some(Progress::Next) => continue,
            Some(Progress::Asked) => {
                if round(w, which).is_some_and(|r| r.todo.is_empty()) {
                    finish(w, which, now);
                }
                return true;
            }
            None => {
                panicked(w, which, &task.chat, now);
                return true;
            }
        }
    }
}

/// A chat's work panicked: its read dropped, the panic counted in its record (the third running
/// sets it aside for a day, its row saying it couldn't be read), and it waits as after a failure.
/// The other chats go on.
fn panicked(w: &mut Worker, which: Which, chat: &str, now: Instant) {
    w.sync.reads.remove(chat);
    let aside = count_panic(w, chat).unwrap_or(false);
    done(w, which, chat);
    let wait = back_off(w, Work::Messages, chat, now);
    let then = if aside { "set aside for a day".to_string() } else { format!("asked again in {} s", wait.as_secs()) };
    log::error!("teams: conversation {}: an internal error reading it; {then}", w.hashed(chat));
}

/// A cadence's chats are all done: it ran, and the rows that changed go.
fn finish(w: &mut Worker, which: Which, now: Instant) {
    match which {
        Which::Open => w.sync.open = None,
        Which::Active => w.sync.active = None,
        Which::List => w.sync.pass = None,
        // The hour's cadence ran when its list was read.
        Which::Hourly => w.sync.hourly = None,
    }
    if !matches!(which, Which::Hourly) {
        w.limiter.ran(cadence_of(which), now);
    }
    flush(w);
}

/// The first chat's work is over: the next one's turn; its row may have changed. Only its own
/// task leaves (by id): never the next one's, when a step that took it off panics after.
fn done(w: &mut Worker, which: Which, chat: &str) {
    if let Some(r) = round(w, which)
        && r.todo.front().is_some_and(|t| t.chat == chat)
    {
        r.todo.pop_front();
    }
    #[cfg(test)]
    w.hooks.done(chat);
    dirty(w, chat);
}

/// One step of a chat's work: its members, then its messages. A part that failed (not refused)
/// waits on its own ([`BACK_OFF`]) and the rest goes on: its messages are read though its members
/// failed, and the other chats aren't held for it.
fn task_step(w: &mut Worker, which: Which, task: &Task, now: Instant) -> Progress {
    if task.guarded {
        let closed = w.copy.as_ref().and_then(|c| c.chat(&task.chat)).is_none_or(|r| r.chat.left || r.chat.unreadable.is_some());
        if closed {
            done(w, which, &task.chat);
            return Progress::Next;
        }
    }
    if task.members {
        if !backed_off(w, Work::Members, &task.chat, now) {
            if !w.may(Route::Members, now) {
                return Progress::Asked;
            }
            let goes_on = match read_members(w, &task.chat, task.updated, now) {
                Ok(()) => {
                    forgive(w, Work::Members, &task.chat);
                    true
                }
                Err(e) => failed(w, which, Work::Members, &task.chat, &e, now),
            };
            news::after_members(w, task);
            match round(w, which).and_then(|r| r.todo.front_mut()) {
                Some(t) if task.read && goes_on => t.members = false,
                _ => done(w, which, &task.chat),
            }
            return Progress::Asked;
        }
        news::after_members(w, task);
        if let Some(t) = round(w, which).and_then(|r| r.todo.front_mut()) {
            t.members = false;
        }
    }
    if !task.read || backed_off(w, Work::Messages, &task.chat, now) {
        done(w, which, &task.chat);
        return Progress::Next;
    }
    match read_step(w, &task.chat, now) {
        Ok(ReadStep::Asked) => Progress::Asked,
        Ok(ReadStep::Over { asked }) => {
            forgive(w, Work::Messages, &task.chat);
            done(w, which, &task.chat);
            if asked { Progress::Asked } else { Progress::Next }
        }
        Err(e) => {
            failed(w, which, Work::Messages, &task.chat, &e, now);
            done(w, which, &task.chat);
            Progress::Asked
        }
    }
}

/// A chat's members or messages failed (not refused). The network's failure holds everything (it
/// is said, and tried again in a while); Graph asking to wait holds everything for its
/// `Retry-After` and slows the cadence; anything else is that chat's alone. The chat's part waits
/// on its own, but for the network's. Whether its work goes on (the messages after the members).
fn failed(w: &mut Worker, which: Which, work: Work, chat: &str, e: &NetError, now: Instant) -> bool {
    let what = format!("{} of a conversation", work.what().trim_start_matches("its "));
    if of_the_network(e) || matches!(e, NetError::Auth(_)) {
        w.failed(cadence_of(which), &what, e, now);
        return false;
    }
    if busy(e) {
        w.failed(cadence_of(which), &what, e, now);
    }
    let wait = back_off(w, work, chat, now);
    log::warn!("teams: conversation {}: {} failed ({}); asked again in {} s", w.hashed(chat), work.what(), kind(e), wait.as_secs());
    true
}

fn backoff_key(w: &Worker, work: Work, chat: &str) -> u64 {
    w.key_of(&format!("{}\0{chat}", work.what()))
}

/// Whether a chat's part waits after failing.
fn backed_off(w: &Worker, work: Work, chat: &str, now: Instant) -> bool {
    w.sync.backoff.get(&backoff_key(w, work, chat)).is_some_and(|b| now < b.until)
}

/// A chat's part failed once more: how long it waits.
fn back_off(w: &mut Worker, work: Work, chat: &str, now: Instant) -> Duration {
    let key = backoff_key(w, work, chat);
    let s = &mut w.sync.backoff;
    let fails = s.get(&key).map_or(0, |b| b.fails).saturating_add(1);
    let wait = BACK_OFF.saturating_mul(1u32.checked_shl(fails.saturating_sub(1)).unwrap_or(u32::MAX)).min(BACK_OFF_MAX);
    if !s.contains_key(&key) && s.len() >= MAX_CHATS {
        s.retain(|_, b| now < b.until);
        if s.len() >= MAX_CHATS {
            s.clear();
        }
    }
    s.insert(key, Backoff { until: now.checked_add(wait).unwrap_or(now), fails });
    wait
}

/// A chat's part went through: no more waiting for it.
fn forgive(w: &mut Worker, work: Work, chat: &str) {
    let key = backoff_key(w, work, chat);
    w.sync.backoff.remove(&key);
}
