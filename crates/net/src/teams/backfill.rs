// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The first copy and the history. The synchronisation's list pass lists every chat first, newest
//! activity first; then each chat's history is read backwards, a page a step and only when no
//! cadence has anything due: `lastModifiedDateTime lt S`, `S` being Graph's time when the chat's
//! copy began (its `history_from`, the start of its incremental reads too). Recent chats first,
//! meeting chats last; one history read in flight.
//!
//! * Where a chat's history stopped (Graph's `nextLink`) is kept in its record, so a restart goes
//!   on from there. Read back from the file, the link is checked as any ([`TeamsClient::own`]):
//!   one elsewhere is dropped, and so is one Graph refuses; the read then starts again just after
//!   the oldest change already copied (`history_oldest`).
//! * At most [`MESSAGE_PAGES`] pages of a chat in a row, then the others' turn; a link met twice
//!   in the read, or a page that brings nothing new, ends its history.
//! * While the first copy runs, its progress is said ([`Status::Copying`]) after each chat. A
//!   chat whose history keeps failing doesn't hold it: once every chat left waits after failing,
//!   the first copy is over, and they go on in the background.
//! * Short of disk ([`enough_room`] on the data folder), the history pauses, told once and the disk
//!   looked at again ten minutes on; pictures and photos ask [`room`] too. Incremental reads go on.
//! * A 429 or a 503: nothing at all is asked during its `Retry-After`, then the history goes at
//!   half its pace, slower at each new one ([`MAX_SLOWDOWN`] times at most), back to its own
//!   after an hour without one.
//! * A chat whose history read panics three times running is set aside for a day (server time),
//!   its row saying it couldn't be read; the others go on.
//!
//! What is hostile here: Graph's answers (links elsewhere or going round, pages that bring nothing
//! new, refusals) and what the file holds (a link kept, times). What it protects: the token, sent
//! nowhere but Graph's own address; the work a server can cause (pages per chat, one request a
//! step after the cadences, none during a `Retry-After`); the disk.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use quatrelle_model::pim::teams::{ChatKind, MAX_CHATS, MESSAGE_PAGES, Status};

use super::graph::{Page, Range, TeamsClient, message_of};
use super::limits::{MAX_SLOWDOWN, Route, retry_wait};
use super::store::ChatRecord;
use super::sync::{count_unread, dirty, flush, http_code, refusal, refused, rfc3339, say_messages};
use super::worker::{Worker, busy, kind, of_the_network, with_ids};
use crate::NetError;
use crate::worker::ahead::enough_room;
use crate::worker::connect::fenced;

const DAY_MS: i64 = 86_400_000;
/// How long a pause for room lasts before the disk is looked at again.
const ROOM_AGAIN: Duration = Duration::from_secs(10 * 60);
/// With nothing left to read, the copy is looked at again this long after for chats come since
/// (or set aside until a day that is over).
const RESCAN: Duration = Duration::from_secs(3600);
/// A chat's history failing (not refused): not asked again for this long, doubled at each failure
/// up to [`BACK_OFF_MAX`]; the other chats go on meanwhile.
const BACK_OFF: Duration = Duration::from_secs(30);
const BACK_OFF_MAX: Duration = Duration::from_secs(15 * 60);
/// The panics running that set a chat aside.
const PANICS: u8 = 3;
/// The pace of history after a 429: a page every this long, times the slow-down.
const PAGE_EVERY: Duration = Duration::from_secs(1);
/// How long the history must go without a 429 to come back to its own pace.
const CLEAN: Duration = Duration::from_secs(3600);
/// The longest link to a next page followed, and the most items of a page taken (as the
/// synchronisation's).
const MAX_LINK: usize = 8 << 10;
const MAX_PAGE_ITEMS: usize = 200;

/// Where the history stands between two steps, in memory only: what has to last (where each
/// chat's history stopped, what it copied, its panics) is in the copy. Every collection is bounded
/// by [`MAX_CHATS`].
#[derive(Default)]
pub(crate) struct Backfill {
    /// The chats whose history is left to read, in their order: the first is the one read.
    todo: VecDeque<String>,
    /// The copy was looked at for them in this run.
    built: bool,
    /// The copy is looked at again then, nothing being left.
    rescan: Option<Instant>,
    /// The first copy runs: its progress is said, `of` the chats in the copy.
    first: bool,
    of: usize,
    /// The read under way, of the first chat.
    read: Option<Read>,
    /// Chats whose history failed: not asked again before then.
    waits: HashMap<String, Wait>,
    /// The disk was found short: looked at again then.
    room_again: Option<Instant>,
    /// After a 429: how many times slower the history goes, since when, and its next page not
    /// before then.
    slowdown: u32,
    throttled: Option<Instant>,
    not_before: Option<Instant>,
    /// The free space a test makes the disk say: (available, total) bytes.
    #[cfg(test)]
    pub(crate) probe: Option<(u64, u64)>,
}

impl Backfill {
    /// Nothing under way any more (switched off, signed out, a new start); a test's probe stays.
    pub(crate) fn reset(&mut self) {
        *self = Backfill {
            #[cfg(test)]
            probe: self.probe,
            ..Backfill::default()
        };
    }
}

/// A chat's history read under way.
struct Read {
    chat: String,
    /// The next page's link; `None`: from just after the oldest change copied (or from `S`).
    link: Option<String>,
    /// Pages read in a row, and the links followed and the messages read (keyed hashes).
    pages: usize,
    links: HashSet<u64>,
    ids: HashSet<u64>,
}

/// A chat's history that failed: not asked again before `until`; `fails` in a row.
#[derive(Clone, Copy)]
struct Wait {
    until: Instant,
    fails: u32,
}

/// What a step of a chat's history did.
enum Step {
    /// A request went, or the limiter holds the next: the turn is used.
    Asked,
    /// This chat is done with for now, nothing asked: the next one may go.
    Next,
    /// Nothing can be asked yet (Graph's clock unknown).
    Idle,
}

/// One history page at most, once no cadence has anything due: of the first chat whose history is
/// left that isn't waiting. Whether there is history left to read (asked, or held for a while).
pub(crate) fn step(w: &mut Worker, now: Instant) -> bool {
    if w.backfill.not_before.is_some_and(|at| now < at) {
        return true;
    }
    if !room(w, now) {
        return false;
    }
    if !w.backfill.built {
        build(w);
    }
    if w.backfill.todo.is_empty() {
        match w.backfill.rescan {
            None => {
                w.backfill.rescan = now.checked_add(RESCAN);
                return false;
            }
            Some(at) if now < at => return false,
            Some(_) => {
                w.backfill.rescan = None;
                build(w);
                if w.backfill.todo.is_empty() {
                    w.backfill.rescan = now.checked_add(RESCAN);
                    return false;
                }
            }
        }
    }
    for _ in 0..w.backfill.todo.len() {
        let Some(chat) = w.backfill.todo.front().cloned() else { return false };
        if w.backfill.waits.get(&chat).is_some_and(|x| now < x.until) {
            rotate(w);
            continue;
        }
        match fenced(|| page(w, &chat, now)) {
            Some(Step::Asked) => return true,
            Some(Step::Next) => continue,
            Some(Step::Idle) => return false,
            None => {
                panicked(w, &chat, now);
                return true;
            }
        }
    }
    all_waiting(w, now);
    false
}

/// Every chat whose history is left waits after failing: the first copy is over all the same
/// (said up to date); their history goes on in the background as they can be read.
fn all_waiting(w: &mut Worker, now: Instant) {
    let b = &w.backfill;
    if !b.first || b.todo.is_empty() || !b.todo.iter().all(|c| b.waits.get(c).is_some_and(|x| now < x.until)) {
        return;
    }
    log::info!("teams: the first copy is over; the history of {} conversations goes on once they can be read", b.todo.len());
    w.backfill.first = false;
    w.up_to_date();
}

/// The first copy's progress, said at the end of each listing: `Copying` while it runs (`None`
/// once it's over). The copy's first listing over, every chat it found has its `S`: from then on,
/// a chat new to the copy is news (its first read starts from its last message).
pub(crate) fn progress(w: &mut Worker) -> Option<Status> {
    first_listed(w);
    if !w.backfill.built {
        build(w);
    }
    let b = &w.backfill;
    b.first.then(|| copying(b.of.saturating_sub(b.todo.len()), b.of))
}

/// `Copying { done, of }`, `of` at most [`MAX_CHATS`] (the copy keeps no more: the module says
/// so when it's reached).
pub(crate) fn copying(done: usize, of: usize) -> Status {
    let of = of.min(MAX_CHATS);
    let n = |x: usize| u32::try_from(x).unwrap_or(u32::MAX);
    Status::Copying { done: n(done.min(of)), of: n(of) }
}

/// The list as the copy has it, sent at once (a start, online or not): every chat's row.
pub(crate) fn from_copy(w: &mut Worker) {
    let ids: Vec<String> = w.copy.as_ref().map(|c| c.chats().into_iter().map(|r| r.chat.id).collect()).unwrap_or_default();
    for id in &ids {
        dirty(w, id);
    }
    flush(w);
    if !ids.is_empty() {
        log::info!("teams: {} conversations shown from the copy", ids.len());
    }
}

/// Whether there's room on the disk the copy is on for more history, pictures and photos
/// ([`enough_room`]). Short of it, they pause — told once — and the disk is looked at again ten
/// minutes on; new messages still come.
pub(crate) fn room(w: &mut Worker, now: Instant) -> bool {
    if w.backfill.room_again.is_some_and(|at| now < at) {
        return false;
    }
    let Some((available, total)) = free_space(w) else { return true };
    if enough_room(available, total) {
        if w.backfill.room_again.take().is_some() {
            log::info!("teams: room on the disk again: the history goes on");
        }
        return true;
    }
    if w.backfill.room_again.is_none() {
        log::warn!("teams: the disk is almost full ({} GB left): the history, pictures and photos pause", available >> 30);
    }
    w.backfill.room_again = now.checked_add(ROOM_AGAIN).or(Some(now));
    false
}

/// The free space of the disk the copy is on: (available, total) bytes; `None` for a copy in
/// memory.
fn free_space(w: &Worker) -> Option<(u64, u64)> {
    #[cfg(test)]
    if let Some(probe) = w.backfill.probe {
        return Some(probe);
    }
    let stats = fs4::statvfs(w.store.data_dir()?).ok()?;
    Some((stats.available_space(), stats.total_space()))
}

/// The server's time (milliseconds since 1970), by the newest `Date` Graph sent.
fn server_ms(w: &Worker) -> Option<i64> {
    w.client.as_ref()?.server_time()?.checked_mul(1000)
}

/// Whether a link may be followed: Graph's own address, as [`TeamsClient::own`] says.
fn owned(w: &Worker, link: &str) -> bool {
    link.len() <= MAX_LINK && w.client.as_ref().is_some_and(|c: &TeamsClient| c.own(link).is_ok())
}

/// Whether a chat's history is left to read: not all of it copied, and Graph serves it (not left,
/// not refused, not set aside until a day that isn't over).
fn wants_history(r: &ChatRecord, server: Option<i64>) -> bool {
    let aside = r.set_aside_until.is_some_and(|until| server.is_none_or(|now| now < until));
    !r.done && !r.chat.left && r.chat.unreadable.is_none() && !aside
}

/// The copy's first listing is over: every chat it holds without its `S` gets it, Graph's time
/// now, and the copy notes it (once).
fn first_listed(w: &mut Worker) {
    let server = server_ms(w);
    let Some(copy) = &w.copy else { return };
    if copy.listed() {
        return;
    }
    let Some(server) = server else { return };
    let begun: Vec<ChatRecord> =
        copy.chats().into_iter().filter(|r| r.history_from.is_none()).map(|r| ChatRecord { history_from: Some(server), ..r }).collect();
    if let Err(e) = copy.put_chats(&begun).and_then(|()| copy.set_listed()) {
        log::warn!("teams: where the copy began couldn't be kept: {}", kind(&NetError::Store(e)));
    }
}

/// The chats whose history is left, looked for in the copy: recent ones first, meetings last. The
/// first time in a run, the first copy is under way when there are some.
fn build(w: &mut Worker) {
    let server = server_ms(w);
    let chats = w.copy.as_ref().map(|c| c.chats()).unwrap_or_default();
    let mut todo: Vec<(bool, Reverse<Option<i64>>, String)> = chats
        .iter()
        .filter(|r| wants_history(r, server))
        .map(|r| (r.chat.kind == ChatKind::Meeting, Reverse(r.chat.last_activity), r.chat.id.clone()))
        .collect();
    todo.sort();
    let b = &mut w.backfill;
    if !b.built {
        b.first = !todo.is_empty();
        b.of = chats.len();
    }
    b.built = true;
    b.todo = todo.into_iter().map(|(_, _, chat)| chat).collect();
    b.read = None;
    if b.first {
        log::info!("teams: the history of {} conversations of {} to copy", b.todo.len(), b.of);
    }
}

/// The first chat's turn is over for now: the next one's.
fn rotate(w: &mut Worker) {
    let b = &mut w.backfill;
    if let Some(chat) = b.todo.pop_front() {
        b.todo.push_back(chat);
    }
    b.read = None;
}

/// A chat's history is done with (all of it read, or Graph doesn't serve it, or set aside): the
/// progress said, and its row, which may have changed.
fn finish(w: &mut Worker, chat: &str) {
    let b = &mut w.backfill;
    if b.todo.front().is_some_and(|c| c == chat) {
        b.todo.pop_front();
    } else {
        b.todo.retain(|c| c != chat);
    }
    b.read = None;
    b.waits.remove(chat);
    dirty(w, chat);
    flush(w);
    let b = &mut w.backfill;
    if b.first {
        if b.todo.is_empty() {
            b.first = false;
            log::info!("teams: the first copy is over");
        }
        w.up_to_date();
    }
}

/// A step of a chat's history: its read begun (its `S` kept, a link kept checked), then its next
/// page.
fn page(w: &mut Worker, chat: &str, now: Instant) -> Step {
    let Some(server) = server_ms(w) else { return Step::Idle };
    let Some(r) = w.copy.as_ref().and_then(|c| c.chat(chat)) else {
        finish(w, chat);
        return Step::Next;
    };
    if !wants_history(&r, Some(server)) {
        finish(w, chat);
        return Step::Next;
    }
    let s = r.history_from.unwrap_or(server);
    if w.backfill.read.as_ref().is_none_or(|x| x.chat != chat) {
        let link = r.next_link.clone().filter(|l| owned(w, l));
        let dropped = r.next_link.is_some() && link.is_none();
        if dropped {
            log::warn!("teams: conversation {}: the history's kept link isn't Graph's own; read again from the oldest change copied", w.hashed(chat));
        }
        if r.history_from.is_none() || dropped {
            let begun = ChatRecord { history_from: Some(s), next_link: link.clone(), ..r.clone() };
            if let Err(e) = w.copy.as_ref().map_or(Ok(()), |c| c.put_chat(&begun)) {
                return failed(w, chat, false, &NetError::Store(e), now);
            }
        }
        w.backfill.read = Some(Read { chat: chat.to_string(), link, pages: 0, links: HashSet::new(), ids: HashSet::new() });
    }
    let route = Route::Messages(w.key_of(chat));
    if !w.may(route, now) {
        return Step::Asked;
    }
    let link = w.backfill.read.as_ref().and_then(|x| x.link.clone());
    // Without a link: from before `S`, or just after the oldest change copied (one changed in the
    // same millisecond isn't missed; read again, it's the same).
    let before = rfc3339(r.history_oldest.map_or(s, |o| o.saturating_add(1)).min(s));
    match w.request(route, now, |c| c.messages(chat, Range::Before(&before), link.as_deref())) {
        Ok(p) => match taken(w, chat, p, s, now) {
            Ok(()) => Step::Asked,
            Err(e) => failed(w, chat, false, &e, now),
        },
        Err(e) => failed(w, chat, link.is_some(), &e, now),
    }
}

/// A page of history: its messages in the copy (and to the engine when the chat is on screen),
/// counted unread as they come; then where the read stands kept: the next link, or the history
/// done (no next page, a link met before, or nothing new).
fn taken(w: &mut Worker, chat: &str, page: Page, s: i64, now: Instant) -> crate::Result<()> {
    #[cfg(test)]
    if w.hooks.trip_history.as_deref() == Some(chat) {
        panic!("a panic the test asked for");
    }
    let messages = with_ids(page.items.iter().take(MAX_PAGE_ITEMS).map(|v| message_of(chat, v)).collect());
    let changed = match &w.copy {
        Some(copy) => copy.upsert(chat, &messages)?,
        None => return Ok(()),
    };
    count_unread(w, chat, &changed);
    if w.open.as_deref() == Some(chat) {
        say_messages(w, chat, &changed, false, true);
    }
    let keys: Vec<u64> = messages.iter().map(|m| w.key_of(&m.id)).collect();
    let has_next = page.next.is_some();
    let next = page.next.filter(|n| owned(w, n));
    let next_key = next.as_deref().map(|n| w.key_of(n));
    let Some(read) = w.backfill.read.as_mut().filter(|x| x.chat == chat) else { return Ok(()) };
    read.pages = read.pages.saturating_add(1);
    let mut news = false;
    for k in keys {
        news |= read.ids.insert(k);
    }
    let again = next_key.is_some_and(|k| !read.links.insert(k));
    // A link not Graph's own is dropped: the read goes on from the oldest change copied.
    let done = page_ended(has_next, news, again);
    let paused = !done && read.pages >= MESSAGE_PAGES;
    let pages = read.pages;
    read.link = next.clone();
    let Some(copy) = &w.copy else { return Ok(()) };
    let Some(mut r) = copy.chat(chat) else { return Ok(()) };
    r.history_from = Some(s);
    r.watermark.get_or_insert(s);
    r.history_oldest = r.history_oldest.into_iter().chain(messages.iter().map(|m| m.modified)).min();
    r.next_link = if done { None } else { next };
    r.done = done;
    r.panics = 0;
    copy.put_chat(&r)?;
    w.backfill.waits.remove(chat);
    pace(w, now);
    if done {
        if pages > 1 {
            log::info!("teams: conversation {}: its history is copied ({pages} pages)", w.hashed(chat));
        }
        finish(w, chat);
    } else if paused {
        log::info!("teams: conversation {}: {pages} pages of history read; the others' turn", w.hashed(chat));
        rotate(w);
    }
    Ok(())
}

/// Whether a page ends a chat's history: Graph has no page after it, it brought nothing new, or its
/// link was met before in the read (a server going round).
fn page_ended(has_next: bool, news: bool, again: bool) -> bool {
    !has_next || !news || again
}

/// After a page, the next one at the history's pace: as fast as the limiter lets it, or slower
/// after a 429 (back to its own after an hour without one).
fn pace(w: &mut Worker, now: Instant) {
    let b = &mut w.backfill;
    if b.throttled.is_some_and(|t| now.saturating_duration_since(t) >= CLEAN) {
        b.slowdown = 1;
        b.throttled = None;
    }
    if b.slowdown > 1 {
        b.not_before = now.checked_add(PAGE_EVERY.saturating_mul(b.slowdown));
    }
}

/// A history read failed. A followed link refused (any 4xx, a 404 too: it says nothing of the
/// chat): dropped, the read goes on from the oldest change copied. Else refused (403, 404): the
/// chat is marked as the reads do (Graph doesn't serve it, or the user left) and done with. Busy
/// (429, 503): nothing at all is asked during its `Retry-After`, and the history slows. The
/// network's failure or the sign-in's: everything waits ("Offline"). Anything else is that chat's
/// alone: it waits, the others go on.
fn failed(w: &mut Worker, chat: &str, followed: bool, e: &NetError, now: Instant) -> Step {
    let id = w.hashed(chat);
    // A link refused (a 404 too) says nothing of the chat: the link may be one kept too long.
    if followed && http_code(e).is_some_and(|c| (400..500).contains(&c)) {
        log::info!("teams: conversation {id}: Graph refused the history's link ({}); read again from the oldest change copied", kind(e));
        if let Some(read) = w.backfill.read.as_mut() {
            read.link = None;
        }
        if let Some(copy) = &w.copy
            && let Some(r) = copy.chat(chat)
            && let Err(e) = copy.put_chat(&ChatRecord { next_link: None, ..r })
        {
            log::warn!("teams: a conversation couldn't be kept: {}", kind(&NetError::Store(e)));
        }
        return Step::Asked;
    }
    if refusal(e) {
        refused(w, chat, e);
        finish(w, chat);
        return Step::Asked;
    }
    if busy(e) {
        let wait = retry_wait(w.client.as_ref().and_then(TeamsClient::retry_after));
        log::warn!("teams: the history of conversation {id}: {}; nothing asked for {} s", kind(e), wait.as_secs());
        if let Some(until) = now.checked_add(wait) {
            w.limiter.pause_all(until);
        }
        let b = &mut w.backfill;
        b.slowdown = b.slowdown.max(1).saturating_mul(2).min(MAX_SLOWDOWN);
        b.throttled = Some(now);
        b.not_before = now.checked_add(wait);
        return Step::Asked;
    }
    if of_the_network(e) || matches!(e, NetError::Auth(_)) {
        log::warn!("teams: the history of a conversation failed: {}", kind(e));
        w.pause(e, now);
        return Step::Asked;
    }
    let wait = back_off(w, chat, now);
    log::warn!("teams: the history of conversation {id} failed ({}); asked again in {} s", kind(e), wait.as_secs());
    rotate(w);
    Step::Asked
}

/// A chat's history failed once more: how long it waits.
fn back_off(w: &mut Worker, chat: &str, now: Instant) -> Duration {
    let waits = &mut w.backfill.waits;
    let fails = waits.get(chat).map_or(0, |x| x.fails).saturating_add(1);
    let wait = BACK_OFF.saturating_mul(1u32.checked_shl(fails.saturating_sub(1)).unwrap_or(u32::MAX)).min(BACK_OFF_MAX);
    if !waits.contains_key(chat) && waits.len() >= MAX_CHATS {
        waits.retain(|_, x| now < x.until);
        if waits.len() >= MAX_CHATS {
            waits.clear();
        }
    }
    waits.insert(chat.to_string(), Wait { until: now.checked_add(wait).unwrap_or(now), fails });
    wait
}

/// A panic reading `chat` (its history, or its news): counted in its record; the third running,
/// it is set aside for a day (server time) and its row says it couldn't be read. Whether it was
/// set aside; `None` when the copy has no record of it.
pub(crate) fn count_panic(w: &mut Worker, chat: &str) -> Option<bool> {
    let server = server_ms(w);
    let mut r = w.copy.as_ref().and_then(|c| c.chat(chat))?;
    r.panics = r.panics.saturating_add(1);
    let aside = r.panics >= PANICS && server.is_some();
    if let (true, Some(server)) = (aside, server) {
        r.panics = 0;
        r.set_aside_until = Some(server.saturating_add(DAY_MS));
        r.chat.unreadable = Some(server);
    }
    if let Some(Err(e)) = w.copy.as_ref().map(|c| c.put_chat(&r)) {
        log::warn!("teams: a conversation couldn't be kept: {}", kind(&NetError::Store(e)));
    }
    Some(aside)
}

/// A chat's history read panicked: counted ([`count_panic`]); set aside, its history is over for
/// now; before that it waits, as after a failure. The others go on.
fn panicked(w: &mut Worker, chat: &str, now: Instant) {
    let id = w.hashed(chat);
    w.backfill.read = None;
    let Some(aside) = count_panic(w, chat) else {
        finish(w, chat);
        return;
    };
    if aside {
        log::error!("teams: conversation {id}: its history couldn't be read ({PANICS} internal errors running); set aside for a day");
        finish(w, chat);
    } else {
        let wait = back_off(w, chat, now);
        log::error!("teams: conversation {id}: an internal error reading its history; asked again in {} s", wait.as_secs());
        rotate(w);
    }
}
