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
use std::time::Instant;

use quatrelle_model::pim::teams::{CHAT_PAGES, CLOCK_TOLERANCE_MS, Chat, MAX_CHATS, Sender};

use super::graph::{Page, chat_of};
use super::limits::{Cadence, Route};
use super::store::{ChatRecord, chat_hash, id_ok};
use super::worker::{Worker, kind};
use crate::NetError;

mod reads;
mod rows;

// The history (next) and the outbox read a chat on with it; the cadences go through `read_step`.
#[cfg_attr(not(test), allow(unused_imports))]
pub(crate) use reads::incremental;
use reads::{Read, ReadStep, read_members, read_step};
use rows::{dirty, flush, say_messages};

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
}

impl SyncState {
    /// What is under way given up (a step panicked); what the engine was sent stays known.
    pub(crate) fn give_up(&mut self) {
        self.pass = None;
        self.active = None;
        self.open = None;
        self.reads.clear();
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
}

/// What a step of a chat's work did.
enum Progress {
    /// A request went, or the limiter holds the next: the turn is used.
    Asked,
    /// This chat's work is over and nothing was asked: the next one's can go.
    Next,
}

/// One step of whatever cadence is due, in their order: the open chat, the list (the hourly pass
/// when that's due), the week's chats. One request at most.
pub(crate) fn tick(w: &mut Worker, now: Instant) {
    // Presence comes with the pictures (next): passed until then.
    if w.limiter.due(Cadence::Presence, now) {
        w.limiter.ran(Cadence::Presence, now);
    }
    if open_step(w, now) || list_step(w, now) {
        return;
    }
    active_step(w, now);
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

/// The list: a page a step, then the chats it found that need reading.
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
        Ok(page) => listed(w, page),
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
    }
}

fn round(w: &mut Worker, which: Which) -> Option<&mut Round> {
    match which {
        Which::Open => w.sync.open.as_mut(),
        Which::List => w.sync.pass.as_mut().map(|p| &mut p.round),
        Which::Active => w.sync.active.as_mut(),
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
        match task_step(w, which, &task, now) {
            Progress::Next => continue,
            Progress::Asked => {
                if round(w, which).is_some_and(|r| r.todo.is_empty()) {
                    finish(w, which, now);
                }
                return true;
            }
        }
    }
}

/// A cadence's chats are all done: it ran, and the rows that changed go.
fn finish(w: &mut Worker, which: Which, now: Instant) {
    match which {
        Which::Open => w.sync.open = None,
        Which::Active => w.sync.active = None,
        Which::List => {
            if w.sync.pass.take().is_some_and(|p| p.whole) {
                w.limiter.ran(Cadence::Hourly, now);
            }
        }
    }
    w.limiter.ran(cadence_of(which), now);
    flush(w);
}

/// The first chat's work is over: the next one's turn; its row may have changed.
fn done(w: &mut Worker, which: Which, chat: &str) {
    if let Some(r) = round(w, which) {
        r.todo.pop_front();
    }
    dirty(w, chat);
}

/// One step of a chat's work: its members, then its messages.
fn task_step(w: &mut Worker, which: Which, task: &Task, now: Instant) -> Progress {
    let cadence = cadence_of(which);
    if task.guarded {
        let closed = w.copy.as_ref().and_then(|c| c.chat(&task.chat)).is_none_or(|r| r.chat.left || r.chat.unreadable.is_some());
        if closed {
            done(w, which, &task.chat);
            return Progress::Next;
        }
    }
    if task.members {
        if !w.may(Route::Members, now) {
            return Progress::Asked;
        }
        match read_members(w, &task.chat, task.updated, now) {
            Ok(()) => match round(w, which).and_then(|r| r.todo.front_mut()) {
                Some(t) if task.read => t.members = false,
                _ => done(w, which, &task.chat),
            },
            Err(e) => {
                w.failed(cadence, "the members of a conversation", &e, now);
                done(w, which, &task.chat);
            }
        }
        return Progress::Asked;
    }
    if !task.read {
        done(w, which, &task.chat);
        return Progress::Next;
    }
    match read_step(w, &task.chat, now) {
        Ok(ReadStep::Asked) => Progress::Asked,
        Ok(ReadStep::Over { asked }) => {
            done(w, which, &task.chat);
            if asked { Progress::Asked } else { Progress::Next }
        }
        Err(e) => {
            w.failed(cadence, "the messages of a conversation", &e, now);
            done(w, which, &task.chat);
            Progress::Asked
        }
    }
}

/// A page of the list read: each chat kept, what it needs noted; the next page, or the listing's
/// end (the list is up to date, and at the hourly pass the chats it lacks are left).
fn listed(w: &mut Worker, page: Page) {
    let chats: Vec<Chat> = page.items.iter().take(MAX_PAGE_ITEMS).filter_map(chat_of).filter(|c| id_ok(&c.id)).collect();
    let (had_next, next_key) = (page.next.is_some(), page.next.as_deref().map(|n| w.key_of(n)));
    let (mut fresh, mut reached, mut capped) = (false, false, false);
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
        let (id, write, task) = take_chat(w, chat, whole);
        writes.extend(write);
        match (task, w.sync.pass.as_mut()) {
            (Some(task), Some(pass)) => pass.round.todo.push_back(task),
            _ => rows.push(id),
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
    pass.next = page.next.filter(|n| more && n.len() <= MAX_LINK && next_key.is_some_and(|k| pass.links.insert(k)));
    if pass.next.is_some() {
        return;
    }
    pass.listed = true;
    pass.ended = !had_next && !capped && !pass.seen.is_empty();
    let (whole, ended, count, pages) = (pass.whole, pass.ended, pass.seen.len(), pass.pages);
    log::info!("teams: {count} conversations listed in {pages} pages");
    w.up_to_date();
    if whole && ended {
        left_behind(w);
    }
    flush(w);
}

/// A chat as the list gives it: the list's fields (read state, hidden, topic, preview) over what
/// the copy has, whose members, `lastUpdatedDateTime` (until the members are read again) and
/// refused or left marks stay. Its id, its record when that changed (to be written), and what it
/// needs (members, a read; none: only its row).
fn take_chat(w: &Worker, listed: Chat, whole: bool) -> (String, Option<ChatRecord>, Option<Task>) {
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
        Some(Sender::User { id, .. }) => !r.chat.members.iter().any(|m| m.user_id == *id) && !looked(w, &r.chat.id, id),
        _ => false,
    };
    let mut members = moved || stranger;
    let mut read = wants_read(&r, before, server);
    if whole && !r.chat.left && r.chat.unreadable.is_none() && server.is_some() {
        let month = recent(r.chat.last_activity, server, MONTH_MS);
        let week = r.read_at.is_none_or(|t| !recent(Some(t), server, WEEK_MS));
        if month || week {
            members = true;
            read = true;
        }
    }
    let task = (members || read).then(|| Task { chat: r.chat.id.clone(), members, updated: moved.then_some(updated), read, guarded: false });
    let write = (stored.as_ref() != Some(&r)).then(|| r.clone());
    (r.chat.id, write, task)
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

/// The hourly pass, the whole list read to its end: the chats of the copy it lacks are no longer
/// the user's. Kept, read-only, and said.
fn left_behind(w: &mut Worker) {
    let Some(hashes) = w.sync.pass.as_mut().map(|p| std::mem::take(&mut p.hashes)) else { return };
    let gone = match &w.copy {
        Some(c) => c.absent(&hashes),
        None => return,
    };
    let now_left: Vec<ChatRecord> =
        gone.iter().filter(|r| !r.chat.left).map(|r| ChatRecord { chat: Chat { left: true, ..r.chat.clone() }, ..r.clone() }).collect();
    if let Some(copy) = &w.copy
        && let Err(e) = copy.put_chats(&now_left)
    {
        log::warn!("teams: the conversations no longer listed couldn't be kept: {}", kind(&NetError::Store(e)));
        return;
    }
    for r in &now_left {
        log::info!("teams: conversation {} is no longer in the list; kept to read", w.hashed(&r.chat.id));
        w.say(quatrelle_model::pim::teams::Event::Removed { chat: r.chat.id.clone() });
    }
    for r in &gone {
        dirty(w, &r.chat.id);
    }
}
