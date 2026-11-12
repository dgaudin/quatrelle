// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The synchronisation: the approved cadences, run by the Teams worker's turns a request at a
//! time, in this order: the chat on screen (every 3 s), the chat list (every 10 s, all of it
//! every hour), the chats active in the last 7 days (every minute).
//!
//! * The list is read until a chat older than the newest activity known (to its end at the hourly
//!   pass). Each chat is kept in the copy with its read state; one new to the copy, or whose
//!   `lastUpdatedDateTime` moved (a rename, a member change), has its members read; one whose last
//!   message is newer than its watermark is read. At the hourly pass the chats active in the last
//!   30 days have their members and messages read, older ones once a week, and a chat of the copy
//!   the whole list no longer has is left.
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

use quatrelle_model::pim::teams::{CHAT_PAGES, CLOCK_TOLERANCE_MS, Chat, MAX_CHATS, Sender, Status};

use super::backfill::count_panic;
use super::graph::{Page, chat_of};
use super::limits::{Cadence, Route};
use super::store::{ChatRecord, chat_hash, id_ok};
use super::worker::{Worker, busy, kind, of_the_network};
use crate::NetError;
use crate::worker::connect::fenced;

mod reads;
mod rows;

// The cadences go through `read_step` (the tests through `incremental`). The history
// (`backfill.rs`) keeps what it reads as the reads do.
#[cfg(test)]
pub(crate) use reads::incremental;
use reads::{Read, ReadStep, read_members, read_step};
pub(crate) use reads::{count_unread, http_code, refusal, refused, rfc3339};
pub(crate) use rows::{dirty, flush, say_members, say_messages};

const DAY_MS: i64 = 86_400_000;
/// The chats read every minute: active in the last 7 days.
const ACTIVE_MS: i64 = 7 * DAY_MS;
/// The chats refreshed at the hourly pass: active in the last 30 days; the others once a week.
const MONTH_MS: i64 = 30 * DAY_MS;
const WEEK_MS: i64 = 7 * DAY_MS;
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
/// The chat list refused (403, an account without Teams): asked again an hour later.
const LIST_RETRY: Duration = Duration::from_secs(3600);
const NO_CHATS: &str = "Teams didn't give this account's chats (403): sign in with the account you use Teams with";

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
    /// The chats of the copy the last whole list lacked (their hashes in the copy): lacked by the
    /// next one too, they are left.
    absent: HashSet<String>,
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

/// A list pass: its pages, then the chats it found that need reading.
struct Pass {
    /// The hourly pass: the whole list, the month's chats refreshed, the absent ones left.
    whole: bool,
    /// The newest activity known before it: a page that reaches it is the last.
    known: Option<i64>,
    /// Its pages are read: the chats' work goes on.
    listed: bool,
    next: Option<String>,
    pages: usize,
    /// The links followed and the chats seen, by keyed hashes: a link met twice, or a page with
    /// nobody new, ends the listing.
    links: HashSet<u64>,
    seen: HashSet<u64>,
    /// The hash each chat listed is kept under in the copy (the hourly pass only).
    hashes: HashSet<String>,
    /// The list came to its end by itself: a chat the copy has and it hasn't is no longer the
    /// user's.
    ended: bool,
    round: Round,
    /// The hourly refresh found by the whole list's pages.
    refresh: VecDeque<Task>,
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
    if let Some(t) = w.sync.open.take().and_then(|r| r.todo.front().cloned()) {
        w.sync.reads.remove(&t.chat);
    }
    w.open = chat.filter(|c| id_ok(c));
    w.sync.opening = w.open.is_some();
    let Some(chat) = w.open.clone() else { return };
    let messages = w.copy.as_ref().map(|c| c.last_pages(&chat, 2)).unwrap_or_default();
    say_messages(w, &chat, &messages, false, false);
    w.sync.members_said = None;
    say_members(w);
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
fn server_ms(w: &Worker) -> Option<i64> {
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
        w.sync.open = Some(Round { todo: VecDeque::from([Task { chat, members, updated: None, read: true, guarded: true }]) });
    }
    run_round(w, Which::Open, now)
}

/// Whether the chat list is refused (an account-wide 403) at `now`. Meanwhile no chat is read,
/// the history neither: a refusal then is the account's, and says nothing of a chat (none is
/// marked for it). The cadences are passed, so that they don't hold the loop's deadline.
fn list_refused(w: &mut Worker, now: Instant) -> bool {
    let Some(until) = w.sync.list_refused else { return false };
    if now >= until {
        w.sync.list_refused = None;
        return false;
    }
    for c in [Cadence::Open, Cadence::List, Cadence::Active, Cadence::Hourly] {
        if w.limiter.due(c, now) {
            w.limiter.ran(c, now);
        }
    }
    let s = &mut w.sync;
    (s.open, s.active, s.hourly) = (None, None, None);
    s.reads.clear();
    true
}

/// The list: a page a step, then the chats it found that need reading. Refused (403: an account
/// without Teams), it is said, and asked again an hour later. A page that follows a link refused
/// otherwise (a 400 on a link Graph no longer takes) ends the listing there, as a link elsewhere
/// does: what it found is read, and nothing is taken for gone.
fn list_step(w: &mut Worker, now: Instant) -> bool {
    if w.sync.pass.is_none() {
        let hourly = w.limiter.due(Cadence::Hourly, now);
        if !hourly && !w.limiter.due(Cadence::List, now) {
            return false;
        }
        w.sync.pass = Some(Pass {
            whole: hourly,
            known: w.sync.newest,
            listed: false,
            next: None,
            pages: 0,
            links: HashSet::new(),
            seen: HashSet::new(),
            hashes: HashSet::new(),
            ended: false,
            round: Round::default(),
            refresh: VecDeque::new(),
        });
    }
    if w.sync.pass.as_ref().is_some_and(|p| p.listed) {
        return run_round(w, Which::List, now);
    }
    if !w.may(Route::ChatList, now) {
        return true;
    }
    #[cfg(test)]
    w.trip_list();
    let next = w.sync.pass.as_ref().and_then(|p| p.next.clone());
    match w.request(Route::ChatList, now, |c| c.chats(next.as_deref())) {
        Ok(page) => listed(w, page, now),
        Err(e) if http_code(&e) == Some(403) => {
            log::warn!("teams: the list of conversations was refused ({}); asked again in an hour", kind(&e));
            w.sync.pass = None;
            w.sync.list_refused = now.checked_add(LIST_RETRY);
            w.limiter.ran(Cadence::List, now);
            w.limiter.ran(Cadence::Hourly, now);
            w.status(Status::Failed { why: NO_CHATS.into() });
        }
        Err(e) if next.is_some() && matches!(e, NetError::Protocol(_)) => {
            log::warn!("teams: a page of the list of conversations was refused ({}); the listing ends there", kind(&e));
            list_over(w, now);
        }
        Err(e) => w.failed(Cadence::List, "the list of conversations", &e, now),
    }
    true
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
        let todo = week.into_iter().map(|(_, chat)| Task { chat, members: false, updated: None, read: true, guarded: true }).collect();
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

/// The first chat's work is over: the next one's turn; its row may have changed.
fn done(w: &mut Worker, which: Which, chat: &str) {
    if let Some(r) = round(w, which) {
        r.todo.pop_front();
    }
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
            match round(w, which).and_then(|r| r.todo.front_mut()) {
                Some(t) if task.read && goes_on => t.members = false,
                _ => done(w, which, &task.chat),
            }
            return Progress::Asked;
        }
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

/// A page of the list read: each chat kept, what it needs noted; the next page, or the listing's
/// end (the list is up to date, and at the hourly pass the chats it lacks are left).
fn listed(w: &mut Worker, page: Page, now: Instant) {
    let chats: Vec<Chat> = page.items.iter().take(MAX_PAGE_ITEMS).filter_map(chat_of).filter(|c| id_ok(&c.id)).collect();
    let (had_next, next_key) = (page.next.is_some(), page.next.as_deref().map(|n| w.key_of(n)));
    // Graph's own address only: a link elsewhere ends the listing (never followed with the token).
    let owned = page.next.as_deref().is_some_and(|n| w.client.as_ref().is_some_and(|c| c.own(n).is_ok()));
    if had_next && !owned {
        log::warn!("teams: a page of the list of conversations links outside Graph's address; the listing ends there");
    }
    // A page cut at the most items taken: what it had past them isn't known, so not its end.
    let (mut fresh, mut reached, mut capped) = (false, false, page.items.len() > MAX_PAGE_ITEMS);
    let (mut writes, mut rows) = (Vec::new(), Vec::new());
    for chat in chats {
        let key = w.key_of(&chat.id);
        let Some(pass) = w.sync.pass.as_mut() else { return };
        if pass.seen.len() >= MAX_CHATS {
            capped = true;
            break;
        }
        if !pass.seen.insert(key) {
            continue;
        }
        fresh = true;
        if pass.whole {
            pass.hashes.insert(chat_hash(&chat.id));
        }
        reached |= pass.known.is_some_and(|k| chat.last_activity.is_none_or(|at| at <= k));
        let whole = pass.whole;
        let s = &mut w.sync;
        if s.activity.len() < MAX_CHATS || s.activity.contains_key(&chat.id) {
            s.activity.insert(chat.id.clone(), chat.last_activity);
        }
        s.newest = s.newest.max(chat.last_activity);
        let (id, write, task, refresh) = take_chat(w, chat, whole);
        writes.extend(write);
        match (task, w.sync.pass.as_mut()) {
            // New activity: read now, refreshed with it when the hour asks.
            (Some(mut task), Some(pass)) => {
                if refresh {
                    task.members = true;
                    task.read = true;
                }
                pass.round.todo.push_back(task);
            }
            (None, Some(pass)) => {
                if refresh {
                    pass.refresh.push_back(Task { chat: id.clone(), members: true, updated: None, read: true, guarded: true });
                }
                rows.push(id);
            }
            (_, None) => rows.push(id),
        }
    }
    // The page's chats kept in one go; then their rows (read back from the copy).
    if let Some(copy) = &w.copy
        && let Err(e) = copy.put_chats(&writes)
    {
        log::warn!("teams: a page of conversations couldn't be kept: {}", kind(&NetError::Store(e)));
    }
    for id in rows {
        dirty(w, &id);
    }
    let Some(pass) = w.sync.pass.as_mut() else { return };
    pass.pages = pass.pages.saturating_add(1);
    let more = fresh && !capped && pass.pages < CHAT_PAGES && (pass.whole || !reached);
    pass.next = page.next.filter(|n| more && owned && n.len() <= MAX_LINK && next_key.is_some_and(|k| pass.links.insert(k)));
    if pass.next.is_some() {
        return;
    }
    pass.ended = !had_next && !capped && !pass.seen.is_empty();
    list_over(w, now);
}

/// A listing is over (by its end, or cut short): the list is up to date, the hour's refresh is
/// noted, and when the whole list came to its end by itself, the chats it lacks are looked at.
fn list_over(w: &mut Worker, now: Instant) {
    let Some(pass) = w.sync.pass.as_mut() else { return };
    pass.next = None;
    pass.listed = true;
    let (whole, ended, count, pages) = (pass.whole, pass.ended, pass.seen.len(), pass.pages);
    let refresh = std::mem::take(&mut pass.refresh);
    log::info!("teams: {count} conversations listed in {pages} pages");
    w.up_to_date();
    if whole {
        // The hour's pass: its refresh goes after the other cadences, a request a step.
        w.limiter.ran(Cadence::Hourly, now);
        match &mut w.sync.hourly {
            Some(_) => log::info!("teams: the last hour's refresh isn't over; {} conversations wait for the next", refresh.len()),
            None if !refresh.is_empty() => w.sync.hourly = Some(Round { todo: refresh }),
            None => {}
        }
        if ended {
            left_behind(w);
        }
    }
    flush(w);
}

/// A chat as the list gives it: the list's fields (read state, hidden, topic, preview) over what
/// the copy has, whose members, `lastUpdatedDateTime` (until the members are read again) and
/// refused or left marks stay. Its id, its record when that changed (to be written), what it
/// needs now (members, a read; none: only its row), and whether the hour's refresh is its (the
/// month's chats, older ones once a week).
fn take_chat(w: &Worker, listed: Chat, whole: bool) -> (String, Option<ChatRecord>, Option<Task>, bool) {
    let server = server_ms(w);
    let stored = w.copy.as_ref().and_then(|c| c.chat(&listed.id));
    let updated = listed.updated;
    let mut r = match &stored {
        Some(s) => ChatRecord { chat: merged(&s.chat, listed), ..s.clone() },
        None => ChatRecord::new(Chat { members: Vec::new(), left: false, unreadable: None, updated: None, ..listed }),
    };
    if let Some(s) = &stored
        && s.chat.last_read != r.chat.last_read
    {
        // Read elsewhere (or marked unread): what came before it no longer counts.
        let after = r.chat.last_read.unwrap_or(i64::MIN);
        r.unread.retain(|(_, created)| *created > after);
    }
    let before = stored.as_ref().map(|s| &s.chat);
    let moved = before.is_none_or(|s| s.updated != updated);
    let stranger = match r.chat.preview.as_ref().map(|p| &p.sender) {
        Some(Sender::User { id, .. }) => !r.chat.members.iter().any(|m| m.user_id.eq_ignore_ascii_case(id)) && !looked(w, &r.chat.id, id),
        _ => false,
    };
    let members = moved || stranger;
    // One marked left that the whole list has again is read: served, it is back.
    let read = wants_read(&r, before, server) || (whole && r.chat.left && server.is_some());
    let refresh = whole
        && !r.chat.left
        && r.chat.unreadable.is_none()
        && server.is_some()
        && (recent(r.chat.last_activity, server, MONTH_MS) || r.read_at.is_none_or(|t| !recent(Some(t), server, WEEK_MS)));
    let task = (members || read).then(|| Task { chat: r.chat.id.clone(), members, updated: moved.then_some(updated), read, guarded: false });
    let write = (stored.as_ref() != Some(&r)).then(|| r.clone());
    (r.chat.id, write, task, refresh)
}

/// A listed chat over the copy's: the copy's members, refused or left marks and
/// `lastUpdatedDateTime` stay; a preview, a time or an address the list doesn't give stays too.
fn merged(stored: &Chat, listed: Chat) -> Chat {
    Chat {
        members: stored.members.clone(),
        left: stored.left,
        unreadable: stored.unreadable,
        updated: stored.updated,
        preview: listed.preview.or_else(|| stored.preview.clone()),
        last_activity: listed.last_activity.or(stored.last_activity),
        web_url: listed.web_url.or_else(|| stored.web_url.clone()),
        tenant_id: listed.tenant_id.or_else(|| stored.tenant_id.clone()),
        ..listed
    }
}

/// Whether the list says a chat is to be read: its last message newer than its watermark (held
/// to Graph's clock and an hour, whatever the copy says); one Graph doesn't serve, once a day by
/// the server's clock; one the user left, only on new activity (they may be back).
fn wants_read(r: &ChatRecord, before: Option<&Chat>, server: Option<i64>) -> bool {
    let Some(server) = server else { return false };
    let at = r.chat.preview.as_ref().map(|p| p.at);
    if r.chat.left {
        let was = before.and_then(|b| b.preview.as_ref()).map(|p| p.at);
        return at.is_some_and(|at| was.is_none_or(|was| at > was));
    }
    if let Some(tried) = r.chat.unreadable {
        return server.saturating_sub(tried) >= DAY_MS;
    }
    let cap = server.saturating_add(CLOCK_TOLERANCE_MS);
    at.is_some_and(|at| r.watermark.is_none_or(|w| at > w.min(cap)))
}

/// Whether `sender` was looked for among `chat`'s members already.
fn looked(w: &Worker, chat: &str, sender: &str) -> bool {
    w.sync.looked.contains(&w.key_of(&format!("{chat}\0{sender}")))
}

/// Senders looked for among a chat's members: not again for them.
fn look(w: &mut Worker, chat: &str, senders: &[String]) {
    let keys: Vec<u64> = senders.iter().map(|s| w.key_of(&format!("{chat}\0{s}"))).collect();
    let looked = &mut w.sync.looked;
    if looked.len().saturating_add(keys.len()) > MAX_LOOKED {
        looked.clear();
    }
    looked.extend(keys);
}

/// The hourly pass, the whole list read to its end: the chats of the copy it lacks, and the one
/// before lacked too, are no longer the user's (lacked once, one may have moved in the list while
/// it was read). Kept, read-only, and said.
fn left_behind(w: &mut Worker) {
    let Some(hashes) = w.sync.pass.as_mut().map(|p| std::mem::take(&mut p.hashes)) else { return };
    let gone: Vec<ChatRecord> = match &w.copy {
        Some(c) => c.absent(&hashes).into_iter().filter(|r| !r.chat.left).collect(),
        None => return,
    };
    let before = std::mem::take(&mut w.sync.absent);
    w.sync.absent = gone.iter().map(|r| chat_hash(&r.chat.id)).take(MAX_CHATS).collect();
    let now_left: Vec<ChatRecord> = gone
        .iter()
        .filter(|r| before.contains(&chat_hash(&r.chat.id)))
        .map(|r| ChatRecord { chat: Chat { left: true, ..r.chat.clone() }, ..r.clone() })
        .collect();
    if let Some(copy) = &w.copy
        && let Err(e) = copy.put_chats(&now_left)
    {
        log::warn!("teams: the conversations no longer listed couldn't be kept: {}", kind(&NetError::Store(e)));
        return;
    }
    for r in &now_left {
        log::info!("teams: conversation {} is no longer in the list; kept to read", w.hashed(&r.chat.id));
        w.say(quatrelle_model::pim::teams::Event::Removed { chat: r.chat.id.clone() });
        dirty(w, &r.chat.id);
    }
}
