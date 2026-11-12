// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The Teams worker's limiter: Microsoft's Teams service limits, the approved polling cadences
//! and the slow-down after a 429. Pure: every call is given the time, nothing here reads a clock
//! or sleeps.
//!
//! * Requests: `GET /me/chats` at most once a second; each chat's message reads, and its hosted
//!   contents, at most once a second per chat; all Teams requests together at most
//!   [`CEILING`] in any second.
//! * Cadences: the chat list every 10 s, the open chat every 3 s, the chats active in the last
//!   7 days every minute, the hourly pass, presence every minute. A cadence is due once its
//!   interval has passed since it last ran, however long ago that was: a computer waking after a
//!   night gives one request per cadence, never the missed ones.
//! * A 429 or 503 on a cadence: nothing of it before `Retry-After` has passed, then its interval
//!   doubled, up to [`MAX_SLOWDOWN`] times; an hour without one brings it back to its base.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use quatrelle_model::pim::teams::MAX_CHATS;

use crate::graph::backoff_secs;

/// All Teams requests together in any second: about 83 % of the 20 per app per tenant Microsoft
/// evaluates.
pub(crate) const CEILING: usize = 16;
/// The most a 429 slows a cadence down: 16 times its base interval.
pub(crate) const MAX_SLOWDOWN: u32 = 16;
/// The longest `Retry-After` waited (the client already caps it there).
const MAX_RETRY_AFTER: u64 = 3600;
/// The `backoff_secs` round whose fallback is the 10 s waited when a 429 says no `Retry-After`.
const DEFAULT_ROUND: u32 = 5;
/// How long a cadence must go without a 429 to come back to its base interval.
const CLEAN: Duration = Duration::from_secs(3600);
/// The longest the worker waits before looking again, so that a stop is heard within a second.
const MAX_DEADLINE: Duration = Duration::from_secs(1);
const SECOND: Duration = Duration::from_secs(1);

/// What a request is for, as far as the limits go. The `u64` is the keyed hash of the chat's id
/// (never the id itself).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Route {
    /// `GET /me/chats`: once a second (the per-user limit).
    ChatList,
    /// A chat's messages: once a second per chat.
    Messages(u64),
    /// A chat's hosted contents (images): once a second per chat.
    Hosted(u64),
    /// A chat's members.
    Members,
    /// Presence of the people shown.
    Presence,
    /// A user's photo.
    Photo,
    /// A file shared in a chat.
    Drive,
}

impl Route {
    /// Whether this route has a limit of its own (one a second), besides the ceiling.
    fn spaced(self) -> bool {
        matches!(self, Route::ChatList | Route::Messages(_) | Route::Hosted(_))
    }
}

/// The approved polling cadences.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cadence {
    /// The chat list, every 10 s.
    List,
    /// The chat on screen, every 3 s.
    Open,
    /// The chats active in the last 7 days, every minute.
    Active,
    /// The whole list, left chats, the month's chats: every hour.
    Hourly,
    /// Presence of the people shown, every minute.
    Presence,
}

impl Cadence {
    const ALL: [Cadence; 5] = [Cadence::List, Cadence::Open, Cadence::Active, Cadence::Hourly, Cadence::Presence];

    /// The approved interval.
    fn base(self) -> Duration {
        Duration::from_secs(match self {
            Cadence::List => 10,
            Cadence::Open => 3,
            Cadence::Active | Cadence::Presence => 60,
            Cadence::Hourly => 3600,
        })
    }

    fn index(self) -> usize {
        match self {
            Cadence::List => 0,
            Cadence::Open => 1,
            Cadence::Active => 2,
            Cadence::Hourly => 3,
            Cadence::Presence => 4,
        }
    }
}

/// Where one cadence stands.
#[derive(Clone, Copy, Debug, Default)]
struct Pace {
    /// When it last ran; `None` until it first does, or when its last run was refused with a 429
    /// (that run is still owed, once the wait is over).
    ran: Option<Instant>,
    /// Nothing of it before then (`Retry-After`).
    not_before: Option<Instant>,
    /// How many times its base interval it runs at now: 1, 2, 4, 8 or 16 (0 reads as 1).
    slowdown: u32,
    /// Its last 429.
    throttled: Option<Instant>,
}

impl Pace {
    /// The slow-down at `now`: back to 1 once an hour has gone without a 429.
    fn slowdown_at(&self, now: Instant) -> u32 {
        match self.throttled {
            Some(t) if now.saturating_duration_since(t) < CLEAN => self.slowdown.max(1),
            _ => 1,
        }
    }

    /// When it is due, judged at `now`.
    fn due_at(&self, c: Cadence, now: Instant) -> Option<Instant> {
        let interval = c.base().saturating_mul(self.slowdown_at(now));
        let after_run = match self.ran {
            Some(r) => r.checked_add(interval),
            None => Some(now),
        };
        match (after_run, self.not_before) {
            (Some(a), Some(n)) => Some(a.max(n)),
            (a, None) => a,
            // Past what an `Instant` holds: never, as far as this process lives.
            (None, Some(_)) => None,
        }
    }

    /// Commits the return to the base interval once the hour is clean.
    fn settle(&mut self, now: Instant) {
        self.slowdown = self.slowdown_at(now);
        if self.slowdown == 1 {
            self.throttled = None;
        }
    }
}

/// The limiter and the cadences' clock, owned by the Teams worker.
#[derive(Debug)]
pub(crate) struct Limiter {
    /// The last [`CEILING`] requests, oldest first.
    window: VecDeque<Instant>,
    /// The last request of each spaced route, at most [`MAX_CHATS`] of them.
    last: HashMap<Route, Instant>,
    paces: [Pace; 5],
}

impl Limiter {
    /// A limiter with no request made yet: every cadence is due at once, so the worker's first
    /// pass runs as soon as it starts.
    pub(crate) fn new(_now: Instant) -> Limiter {
        Limiter { window: VecDeque::with_capacity(CEILING), last: HashMap::new(), paces: [Pace::default(); 5] }
    }

    /// How long to wait before a request of `route` (zero when it may go now).
    pub(crate) fn wait(&self, route: Route, now: Instant) -> Duration {
        let own = match self.last.get(&route) {
            Some(t) if route.spaced() => until(*t, now),
            _ => Duration::ZERO,
        };
        let all = match self.window.front() {
            Some(t) if self.window.len() >= CEILING => until(*t, now),
            _ => Duration::ZERO,
        };
        own.max(all)
    }

    /// A request of `route` was made at `now`.
    pub(crate) fn took(&mut self, route: Route, now: Instant) {
        while self.window.len() >= CEILING {
            self.window.pop_front();
        }
        self.window.push_back(now);
        if !route.spaced() {
            return;
        }
        if !self.last.contains_key(&route) && self.last.len() >= MAX_CHATS {
            // Only the last second's requests still hold anything back.
            self.last.retain(|_, t| now.saturating_duration_since(*t) < SECOND);
            if self.last.len() >= MAX_CHATS
                && let Some(oldest) = self.last.iter().min_by_key(|(_, t)| **t).map(|(r, _)| *r)
            {
                self.last.remove(&oldest);
            }
        }
        self.last.insert(route, now);
    }

    /// Whether cadence `c` should run now.
    pub(crate) fn due(&self, c: Cadence, now: Instant) -> bool {
        self.pace(c).due_at(c, now).is_some_and(|at| at <= now)
    }

    /// Cadence `c` ran at `now`.
    pub(crate) fn ran(&mut self, c: Cadence, now: Instant) {
        if let Some(p) = self.paces.get_mut(c.index()) {
            p.settle(now);
            p.ran = Some(now);
        }
    }

    /// Cadence `c` was answered 429 or 503 at `now`, with this `Retry-After` in seconds (10 s
    /// when none, an hour at most): nothing of it before that has passed, then its interval
    /// doubled, up to [`MAX_SLOWDOWN`] times its base.
    pub(crate) fn throttled(&mut self, c: Cadence, retry_after: Option<u64>, now: Instant) {
        // Not `backoff_secs(Some(hint))`: it holds a hint to 20 s, and Graph's Retry-After is obeyed whole.
        let secs = retry_after.unwrap_or_else(|| backoff_secs(None, DEFAULT_ROUND)).min(MAX_RETRY_AFTER);
        if let Some(p) = self.paces.get_mut(c.index()) {
            p.settle(now);
            p.slowdown = p.slowdown.max(1).saturating_mul(2).min(MAX_SLOWDOWN);
            p.throttled = Some(now);
            p.ran = None;
            p.not_before = now.checked_add(Duration::from_secs(secs)).or(Some(now));
        }
    }

    /// Cadence `c`'s interval now: its base, times its slow-down.
    pub(crate) fn interval(&self, c: Cadence) -> Duration {
        c.base().saturating_mul(self.pace(c).slowdown.max(1))
    }

    /// When the worker should look again: the next cadence due, never more than a second away
    /// so that a stop is heard, and never before `now`.
    pub(crate) fn next_deadline(&self, now: Instant) -> Instant {
        let cap = now.checked_add(MAX_DEADLINE).unwrap_or(now);
        Cadence::ALL.iter().filter_map(|c| self.pace(*c).due_at(*c, now)).fold(cap, Instant::min).max(now)
    }

    fn pace(&self, c: Cadence) -> Pace {
        self.paces.get(c.index()).copied().unwrap_or_default()
    }
}

/// How long from `now` until a second after `then` (zero once it has passed).
fn until(then: Instant, now: Instant) -> Duration {
    SECOND.saturating_sub(now.saturating_duration_since(then))
}
